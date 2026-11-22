//! mio_gateway — the only node that speaks to the Brain (H3.1).
//!
//! Two halves on two threads. The robot half lives on the ROS executor: it hands Nav2 the goals
//! the Brain asks for and listens to the safety supervisor. The link half has a thread and a
//! runtime of its own: it holds the WebSocket, and everything it decides is `session.rs`. All the
//! two ever exchange is `Robot` one way and `FromRobot` the other.
//!
//! The Brain's address and the token come from the environment, under the names the Fake Heart
//! uses: `BRAIN_URL`, `DEV_TOKEN`, `DEVICE_ID`. The token is not a ROS parameter because anything
//! on the graph can read those. Events wait on disk for the Brain in the `outbox_dir` parameter,
//! by default `$XDG_STATE_HOME/mio_gateway/outbox` (H3.2).

use futures::{SinkExt, StreamExt};
use mio_gateway::outbox::Outbox;
use mio_gateway::session::{Ended, Now, Out, Pose, Robot, Session};
use rclrs::*;
use ros_env::diagnostic_msgs::msg::{DiagnosticArray, DiagnosticStatus};
use ros_env::geometry_msgs::msg::PoseWithCovarianceStamped;
use ros_env::nav2_msgs::action::{NavigateToPose, NavigateToPose_Goal};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

/// The supervisor reports once a second. Three missed and it is treated as failed, not as quiet
/// (HEART_SPEC R9). It gets the same three seconds to be heard for the first time: a gateway that
/// has only just started has not found a fault, it has not looked yet.
const SUPERVISOR_SILENT_AFTER: Duration = Duration::from_secs(3);
const SUPERVISOR_SILENT: &str = "SAFETY_SUPERVISOR_NO_DATA";

/// What the robot half tells the link half.
enum FromRobot {
    NavReady(bool),
    GoalAnswered {
        corr_id: String,
        accepted: bool,
    },
    Progress {
        corr_id: String,
        remaining_m: f64,
        remaining_s: f64,
    },
    GoalEnded {
        corr_id: String,
        ended: Ended,
    },
    Pose(Pose),
    Fault(Option<String>),
}

/// What the robot half remembers between callbacks.
struct Watch {
    heard: Option<Instant>,        // when the supervisor last reported
    reported: Option<String>,      // the fault it reported, if any
    fault: Option<Option<String>>, // what the link was last told
    nav_ready: Option<bool>,
}

fn now(start: Instant) -> Now {
    Now {
        mono: start.elapsed(),
        wall_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64),
    }
}

/// `{"kitchen": [x, y, yaw], ...}` in the map frame. A file that is missing or malformed means
/// no named places, said once in the log — never a guess.
fn regions(path: &str, logger: &Logger) -> HashMap<String, Pose> {
    if path.is_empty() {
        return HashMap::new();
    }
    let parsed = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|raw| {
            serde_json::from_slice::<HashMap<String, Pose>>(&raw).map_err(|e| e.to_string())
        });
    match parsed {
        Ok(regions) => regions,
        Err(e) => {
            log_error!(logger, "no named places: could not read {path}: {e}");
            HashMap::new()
        }
    }
}

/// Where events wait for the Brain: the robot's state, apart from the source and the build.
fn default_outbox_dir() -> String {
    std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|base| base.join("mio_gateway/outbox").display().to_string())
        .unwrap_or_default()
}

/// Keeps the files in step with the session. A disk that fails costs durability, not the link:
/// the event is still in memory and still sent — less capable, not broken.
/// ponytail: blocking writes on the link's thread, a few ms each on an SD card; the robot half
/// runs on its own thread and never waits for them.
fn keep(outbox: &mut Option<Outbox>, out: &Out, logger: &Logger) {
    match out {
        Out::Store(event) => {
            if let Some(Err(e)) = outbox.as_mut().map(|disk| disk.put(event)) {
                log_error!(
                    logger,
                    "an event is not on disk, so a restart would lose it: {e}"
                );
            }
        }
        Out::Forget { msg_ids, confirmed } => {
            if !confirmed {
                log_error!(
                    logger,
                    "the outbox is full: dropped the oldest event ({}) unconfirmed",
                    msg_ids.join(", ")
                );
            }
            for id in msg_ids {
                if let Some(Err(e)) = outbox.as_mut().map(|disk| disk.remove(id)) {
                    log_warn!(logger, "could not delete confirmed event {id}: {e}");
                }
            }
        }
        _ => {}
    }
}

fn main() -> Result<(), RclrsError> {
    let start = Instant::now();
    let mut executor = Context::default_from_env()?.create_basic_executor();
    let node = executor.create_node("mio_gateway")?;
    let logger = node.logger().clone();

    let url = std::env::var("BRAIN_URL").unwrap_or_else(|_| "ws://127.0.0.1:8080/ws".into());
    let device_id = std::env::var("DEVICE_ID").unwrap_or_else(|_| "heart-01".into());
    let Ok(token) = std::env::var("DEV_TOKEN") else {
        // No default: a token compiled into the robot is not a secret.
        log_fatal!(
            &logger,
            "DEV_TOKEN is not set: the Brain would refuse this Heart"
        );
        std::process::exit(1);
    };
    let regions_file: Arc<str> = node
        .declare_parameter("regions_file")
        .default("".into())
        .read_only()?
        .get();
    let places = regions(&regions_file, &logger);
    let outbox_dir: Arc<str> = node
        .declare_parameter("outbox_dir")
        .default(default_outbox_dir().into())
        .read_only()?
        .get();
    log_info!(
        &logger,
        "dialling {url} as {device_id}, with {} named place(s)",
        places.len()
    );

    let (to_robot, mut for_robot) = unbounded_channel::<Robot>();
    let (to_link, from_robot) = unbounded_channel::<FromRobot>();

    // ---- the link half ------------------------------------------------------------------------
    let mut session = Session::new(&device_id, &token, places);
    let outbox = match Outbox::open(&*outbox_dir) {
        Ok(opened) => {
            for bad in &opened.unreadable {
                log_error!(&logger, "not an event, moved aside: {}", bad.display());
            }
            log_info!(
                &logger,
                "outbox at {outbox_dir}: {} event(s) still waiting for the Brain",
                opened.waiting.len()
            );
            session.restore(opened.waiting);
            Some(opened.outbox)
        }
        Err(e) => {
            log_error!(
                &logger,
                "no outbox at '{outbox_dir}' ({e}): events wait in memory only, and a restart loses them"
            );
            None
        }
    };
    let link_logger = logger.clone();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime for the link")
            .block_on(link(
                url,
                session,
                outbox,
                start,
                to_robot,
                from_robot,
                link_logger,
            ));
    });

    // ---- the robot half -----------------------------------------------------------------------
    let navigate = node.create_action_client::<NavigateToPose>("navigate_to_pose")?;
    let worker = node.create_worker(Watch {
        heard: None,
        reported: None,
        fault: None,
        nav_ready: None,
    });

    let _health = worker.create_subscription(
        "/diagnostics",
        |watch: &mut Watch, report: DiagnosticArray| {
            let supervisor = |s: &&DiagnosticStatus| s.name == "mio_safety_supervisor";
            if let Some(status) = report.status.iter().find(supervisor) {
                watch.heard = Some(Instant::now());
                watch.reported =
                    (status.level != DiagnosticStatus::OK).then(|| status.message.clone());
            }
        },
    )?;

    let (tx, client) = (to_link.clone(), navigate.clone());
    let _watchdog =
        worker.create_timer_repeating(Duration::from_millis(500), move |watch: &mut Watch| {
            let silent = watch.heard.unwrap_or(start).elapsed() > SUPERVISOR_SILENT_AFTER;
            let fault = if silent {
                Some(SUPERVISOR_SILENT.to_owned())
            } else {
                watch.reported.clone()
            };
            if watch.fault.as_ref() != Some(&fault) {
                watch.fault = Some(fault.clone());
                let _ = tx.send(FromRobot::Fault(fault));
            }
            let ready = client.server_is_available().unwrap_or(false);
            if watch.nav_ready != Some(ready) {
                watch.nav_ready = Some(ready);
                let _ = tx.send(FromRobot::NavReady(ready));
            }
        })?;

    // AMCL latches its last estimate, so this is known from the moment the robot is localised.
    let tx = to_link.clone();
    let _pose = node.create_subscription(
        "amcl_pose".qos(QoSProfile::default().transient_local()),
        move |estimate: PoseWithCovarianceStamped| {
            let (p, q) = (
                &estimate.pose.pose.position,
                &estimate.pose.pose.orientation,
            );
            let _ = tx.send(FromRobot::Pose((p.x, p.y, 2.0 * q.z.atan2(q.w))));
        },
    )?;

    let (tx, commands) = (to_link, executor.commands().clone());
    // `run` keeps going whether or not the promise is kept; the names are only to say so.
    let _robot = executor.commands().run(async move {
        // The goal Nav2 is running for us, and the handle that cancels it.
        let mut running = None;
        while let Some(command) = for_robot.recv().await {
            match command {
                Robot::Goto {
                    corr_id,
                    pose: (x, y, yaw),
                } => {
                    let mut goal = NavigateToPose_Goal::default();
                    // No stamp: zero means "wherever the map frame is now". A stamp from this
                    // node would be a time Nav2's transforms have not reached yet whenever the
                    // two disagree about the time, as they do in simulation.
                    goal.pose.header.frame_id = "map".into();
                    goal.pose.pose.position.x = x;
                    goal.pose.pose.position.y = y;
                    goal.pose.pose.orientation.z = (yaw / 2.0).sin();
                    goal.pose.pose.orientation.w = (yaw / 2.0).cos();
                    let accepted = navigate.request_goal(goal).await;
                    let _ = tx.send(FromRobot::GoalAnswered {
                        corr_id: corr_id.clone(),
                        accepted: accepted.is_some(),
                    });
                    let Some(goal) = accepted else { continue };
                    running = Some((corr_id.clone(), goal.cancellation.clone()));
                    let tx = tx.clone();
                    let _goal = commands.run(async move {
                        let mut events = goal.stream();
                        while let Some(event) = events.next().await {
                            let report = match event {
                                GoalEvent::Feedback(f) => {
                                    let p = &f.current_pose.pose;
                                    let yaw = 2.0 * p.orientation.z.atan2(p.orientation.w);
                                    let _ =
                                        tx.send(FromRobot::Pose((p.position.x, p.position.y, yaw)));
                                    let eta = &f.estimated_time_remaining;
                                    FromRobot::Progress {
                                        corr_id: corr_id.clone(),
                                        remaining_m: f.distance_remaining.into(),
                                        remaining_s: eta.sec as f64 + eta.nanosec as f64 / 1e9,
                                    }
                                }
                                GoalEvent::Result((status, _)) => FromRobot::GoalEnded {
                                    corr_id: corr_id.clone(),
                                    ended: match status {
                                        GoalStatusCode::Succeeded => Ended::Succeeded,
                                        GoalStatusCode::Cancelled => Ended::Cancelled,
                                        _ => Ended::Aborted,
                                    },
                                },
                                GoalEvent::Status(_) => continue,
                            };
                            let over = matches!(report, FromRobot::GoalEnded { .. });
                            let _ = tx.send(report);
                            if over {
                                break;
                            }
                        }
                    });
                }
                Robot::Cancel { corr_id } => {
                    if let Some((_, cancellation)) = running.take_if(|(id, _)| *id == corr_id) {
                        let _ = cancellation.cancel().await;
                    }
                }
            }
        }
    });

    executor.spin(SpinOptions::default()).first_error()?;
    Ok(())
}

/// What the robot said, as the session hears it.
fn heard(session: &mut Session, event: FromRobot, now: Now) -> Vec<Out> {
    match event {
        FromRobot::NavReady(ready) => session.nav_ready(ready, now),
        FromRobot::GoalAnswered { corr_id, accepted } => {
            session.goal_answered(&corr_id, accepted, now)
        }
        FromRobot::Progress {
            corr_id,
            remaining_m,
            remaining_s,
        } => session.progress(&corr_id, remaining_m, remaining_s, now),
        FromRobot::GoalEnded { corr_id, ended } => session.goal_ended(&corr_id, ended, now),
        FromRobot::Pose(pose) => {
            session.pose(pose);
            Vec::new()
        }
        FromRobot::Fault(fault) => session.fault(fault, now),
    }
}

/// Dials the Brain, for ever: Heart dials out and the Brain never dials (ENVELOPE §8). Between
/// attempts the wait grows by half each time to 10 s, with jitter, so that everything that dropped
/// when the router rebooted does not come back in the same instant.
async fn link(
    url: String,
    mut session: Session,
    mut outbox: Option<Outbox>,
    start: Instant,
    to_robot: UnboundedSender<Robot>,
    mut from_robot: UnboundedReceiver<FromRobot>,
    logger: Logger,
) {
    let mut wait = Duration::from_secs(1);
    loop {
        match tokio_tungstenite::connect_async(url.as_str()).await {
            Err(e) => log_warn!(&logger, "the Brain is not reachable at {url}: {e}"),
            Ok((socket, _)) => {
                let why = talk(
                    socket,
                    &mut session,
                    &mut outbox,
                    start,
                    &to_robot,
                    &mut from_robot,
                    &logger,
                )
                .await;
                if session.welcomed() {
                    wait = Duration::from_secs(1);
                }
                session.disconnected();
                log_warn!(&logger, "link to the Brain closed: {why}");
            }
        }
        // Somewhere in [0.5, 1.5) of the wait. A ULID's random half is randomness already here.
        let jitter = 0.5 + (ulid::Ulid::new().random() % 1000) as f64 / 1000.0;
        let pause = tokio::time::sleep(wait.mul_f64(jitter));
        tokio::pin!(pause);
        wait = (wait.mul_f64(1.5)).min(Duration::from_secs(10));
        // The robot does not stop for an outage. What it says while the link is down still changes
        // what is known, and an event — a goal that ends, a fault — goes on disk for the next one.
        loop {
            tokio::select! {
                _ = &mut pause => break,
                Some(event) = from_robot.recv() => {
                    for out in heard(&mut session, event, now(start)) {
                        keep(&mut outbox, &out, &logger);
                    }
                }
            }
        }
    }
}

/// One connection, from hello to whatever ends it. Returns why it ended.
async fn talk<S>(
    socket: tokio_tungstenite::WebSocketStream<S>,
    session: &mut Session,
    outbox: &mut Option<Outbox>,
    start: Instant,
    to_robot: &UnboundedSender<Robot>,
    from_robot: &mut UnboundedReceiver<FromRobot>,
    logger: &Logger,
) -> String
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut write, mut read) = socket.split();
    let hello = session.connect(now(start)).to_string();
    if let Err(e) = write.send(Message::Text(hello.into())).await {
        return format!("could not send sys.hello: {e}");
    }
    let mut every_second = tokio::time::interval(Duration::from_secs(1));
    let mut welcomed = false;
    loop {
        let outs = tokio::select! {
            message = read.next() => match message {
                Some(Ok(Message::Text(text))) => session.inbound(text.as_bytes(), now(start)),
                Some(Ok(Message::Binary(bytes))) => session.inbound(&bytes, now(start)),
                Some(Ok(Message::Close(_))) | None => return "the Brain closed it".into(),
                Some(Ok(_)) => continue,
                Some(Err(e)) => return format!("socket error: {e}"),
            },
            _ = every_second.tick() => session.tick(now(start)),
            Some(event) = from_robot.recv() => heard(session, event, now(start)),
        };
        if session.welcomed() && !welcomed {
            welcomed = true;
            log_info!(logger, "the Brain let us in");
        }
        for out in outs {
            match out {
                Out::Send(envelope) => {
                    if let Err(e) = write.send(Message::Text(envelope.to_string().into())).await {
                        return format!("could not send: {e}");
                    }
                }
                Out::Robot(command) => {
                    let _ = to_robot.send(command);
                }
                Out::Close(why) => {
                    let _ = write.close().await;
                    return why;
                }
                // Before the Send of the same event: on disk before it is on the wire.
                out @ (Out::Store(_) | Out::Forget { .. }) => keep(outbox, &out, logger),
            }
        }
    }
}
