//! What the gateway says to the Brain and what it makes of the answers, with no socket and no ROS
//! in sight. `main.rs` owns the socket and the robot and feeds this.
//!
//! ENVELOPE.md is the contract and the Brain's code is what it actually does; where they differ
//! the difference is noted at the line that follows the Brain. Where this file and a conformance
//! vector disagree, the vector is the bug report.

use crate::{decode, known_topic};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::Duration;

pub const PROTOCOL_VERSION: u64 = 1;
/// Both directions, every 5 s (ENVELOPE §8). The Brain drops a Heart it has not had a valid
/// heartbeat from for 15 s.
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);
/// Three missed heartbeats and the link is treated as dead.
pub const DEAD_AFTER: Duration = Duration::from_secs(15);
/// The capability manifest is re-published on a timer, and at once when it changes.
pub const MANIFEST_EVERY: Duration = Duration::from_secs(10);
/// How long an executed command key is remembered, so a retry replays its answer (ENVELOPE §4).
pub const IDEM_WINDOW: Duration = Duration::from_secs(600);
/// How long Nav2 has to say yes or no to a goal. The Brain stops waiting for the ACK after 5 s;
/// past that the goal is given up on, so it cannot hold the one slot for ever.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(5);
/// Only coarse, infrequent messages cross (ENVELOPE §1): Nav2 reports progress at ~50 Hz, the
/// Brain hears it once a second.
pub const FEEDBACK_EVERY: Duration = Duration::from_secs(1);
/// ponytail: facts that happen while the link is down wait here, in memory, so a goal that ends
/// during an outage does not leave the Brain believing it is still running. Lost on a restart,
/// and capped. The durable on-disk outbox (H3.2) replaces this.
pub const HELD_MAX: usize = 100;

/// The two clocks every envelope carries (ENVELOPE §7), read by the caller so this stays testable.
#[derive(Clone, Copy, Debug)]
pub struct Now {
    /// Monotonic: for durations and timeouts, immune to clock adjustments.
    pub mono: Duration,
    /// Wall clock, milliseconds since the epoch: for human time, and for `expires_at`.
    pub wall_ms: u64,
}

/// A place and a heading in the map frame: metres, metres, radians.
pub type Pose = (f64, f64, f64);

/// What the gateway asks the robot to do. The only two things in v1.
#[derive(Clone, Debug, PartialEq)]
pub enum Robot {
    Goto { corr_id: String, pose: Pose },
    Cancel { corr_id: String },
}

/// What a step produces.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// An envelope to put on the wire.
    Send(Value),
    /// Something for the robot to do.
    Robot(Robot),
    /// Give up on this connection, and why. The caller closes and dials again.
    Close(String),
}

/// How a navigation goal ended, as the robot reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Succeeded,
    Cancelled,
    Aborted,
}

struct Goal {
    corr_id: String, // of the nav.goto: feedback and the result travel on it
    key: String,     // the idem_key of that nav.goto
    goal_id: String, // the Brain's name for it: a cancel names the goal by this
    answered: bool,  // Nav2 has taken it
    started: Duration,
    feedback_at: Option<Duration>,
}

enum Seen {
    /// Handed to the robot; its answer has not come back yet.
    Pending,
    /// Answered: the payload of the ACK, to be replayed on a retry.
    Answered(Value),
}

/// A place name as it is looked up: the Brain's model writes "Living Room", "living_room" and
/// "living-room" for the same room.
pub fn place(name: &str) -> String {
    name.trim().to_lowercase().replace(['_', '-'], " ")
}

/// The payload of an ACK that says no. A refusal is an answer, not an error (ENVELOPE §3).
fn refusal(reason: &str) -> Value {
    json!({ "accepted": false, "reason": reason, "exec_status": "rejected" })
}

pub struct Session {
    device_id: String,
    token: String,
    /// Where a named place is. The Brain usually sends a name and no coordinates.
    regions: HashMap<String, Pose>,

    // This connection.
    seq: u32,
    hello: String, // the corr_id of our sys.hello
    welcomed: bool,
    heard_at: Duration,
    heartbeat_at: Duration,
    manifest_at: Duration,
    manifest: Value, // the last one sent on this connection

    // The robot, across connections.
    goal: Option<Goal>,
    fault: Option<String>, // a fault standing on the robot (`TIP_OVER`, ...)
    nav_ready: bool,       // Nav2 is there to take a goal
    pose: Option<Pose>,    // the last place the robot reported being
    seen: HashMap<String, (Seen, Duration)>,
    held: Vec<Value>,
}

impl Session {
    pub fn new(device_id: &str, token: &str, regions: HashMap<String, Pose>) -> Self {
        Self {
            device_id: device_id.into(),
            token: token.into(),
            regions: regions
                .into_iter()
                .map(|(name, pose)| (place(&name), pose))
                .collect(),
            seq: 0,
            hello: String::new(),
            welcomed: false,
            heard_at: Duration::ZERO,
            heartbeat_at: Duration::ZERO,
            manifest_at: Duration::ZERO,
            manifest: Value::Null,
            goal: None,
            fault: None,
            nav_ready: false,
            pose: None,
            seen: HashMap::new(),
            held: Vec::new(),
        }
    }

    /// A socket has just opened: the `sys.hello` to send first. `seq` starts again with every
    /// connection; what is known about the robot does not.
    pub fn connect(&mut self, now: Now) -> Value {
        (self.seq, self.welcomed, self.manifest) = (0, false, Value::Null);
        (self.heard_at, self.heartbeat_at) = (now.mono, now.mono);
        let payload = json!({
            "device_id": self.device_id,
            "token": self.token,
            "protocol_version": PROTOCOL_VERSION,
            "role": "heart",
            "client_wall_ms": now.wall_ms,
            "capabilities": ["nav.goto", "nav.cancel"],
        });
        let mut hello = self.envelope("CMD", "sys.hello", None, payload, now);
        self.hello = hello["corr_id"].as_str().unwrap_or_default().into();
        hello["idem_key"] = json!(format!("hello-{}", self.hello));
        hello
    }

    /// The socket is gone. Until the next welcome, facts are held and everything else is dropped.
    pub fn disconnected(&mut self) {
        self.welcomed = false;
    }

    pub fn welcomed(&self) -> bool {
        self.welcomed
    }

    /// Builds an envelope (ENVELOPE §11). `corr_id` ties a reply to what it answers; a message
    /// that starts an interaction is its own correlation.
    fn envelope(
        &mut self,
        kind: &str,
        topic: &str,
        corr_id: Option<&str>,
        payload: Value,
        now: Now,
    ) -> Value {
        self.seq = self.seq.wrapping_add(1);
        let msg_id = ulid::Ulid::new().to_string();
        json!({
            "msg_id": msg_id,
            "corr_id": corr_id.unwrap_or(&msg_id),
            "t_mono_ns": now.mono.as_nanos().to_string(),
            "t_wall_ms": now.wall_ms,
            "kind": kind,
            "topic": topic,
            "seq": self.seq,
            "payload": payload,
        })
    }

    /// Something that describes now — an ACK, progress, a heartbeat. With no link it is worth
    /// nothing later, so it is dropped (ENVELOPE §5).
    fn say(
        &mut self,
        kind: &str,
        topic: &str,
        corr_id: Option<&str>,
        payload: Value,
        now: Now,
    ) -> Vec<Out> {
        if !self.welcomed {
            return Vec::new();
        }
        vec![Out::Send(self.envelope(kind, topic, corr_id, payload, now))]
    }

    /// A fact — it happened, and it is still true later. With no link it waits, keeping its
    /// `msg_id` and both clocks, and is given a fresh `seq` when it is finally sent (ENVELOPE §8).
    fn fact(&mut self, topic: &str, corr_id: Option<&str>, payload: Value, now: Now) -> Vec<Out> {
        let seq = self.seq;
        let envelope = self.envelope("EVT", topic, corr_id, payload, now);
        if self.welcomed {
            return vec![Out::Send(envelope)];
        }
        // Not sent, so not numbered: a `seq` used up here would be a gap on the wire, and a gap
        // means a lost message to whoever is counting (ENVELOPE §6).
        self.seq = seq;
        if self.held.len() == HELD_MAX {
            self.held.remove(0);
        }
        self.held.push(envelope);
        Vec::new()
    }

    /// Sends an ACK and remembers it against the command's key, so a retry gets the same answer.
    fn answer(
        &mut self,
        key: &str,
        topic: &str,
        corr_id: &str,
        payload: Value,
        now: Now,
    ) -> Vec<Out> {
        self.seen
            .insert(key.into(), (Seen::Answered(payload.clone()), now.mono));
        self.say("ACK", topic, Some(corr_id), payload, now)
    }

    fn error(
        &mut self,
        reason: &str,
        field: Option<&str>,
        about: Option<&Value>,
        now: Now,
    ) -> Vec<Out> {
        let mut payload = Map::new();
        payload.insert("reason".into(), json!(reason));
        if let Some(field) = field {
            payload.insert("field".into(), json!(field));
        }
        if let Some(msg_id) = about.and_then(|m| m["msg_id"].as_str()) {
            payload.insert("msg_id".into(), json!(msg_id));
        }
        // Nothing here disconnects the peer (ENVELOPE §9): say what was wrong and carry on.
        self.say("ERR", "sys.error", None, Value::Object(payload), now)
    }

    /// One message off the wire.
    pub fn inbound(&mut self, raw: &[u8], now: Now) -> Vec<Out> {
        self.heard_at = now.mono;
        let message = match decode(raw) {
            Ok(message) => message,
            // Expired by the sender's own two clocks. That is a refusal, not a bad message.
            Err(e) if e.reason == "expired" => {
                let message: Value = serde_json::from_slice(raw).unwrap_or_default();
                let field = |name: &str| message[name].as_str().unwrap_or_default().to_owned();
                let (topic, corr_id, key) = (field("topic"), field("corr_id"), field("idem_key"));
                return self.answer(&key, &topic, &corr_id, refusal("expired"), now);
            }
            Err(e) => return self.error(e.reason, e.field, None, now),
        };
        let text = |name: &str| message[name].as_str().unwrap_or_default().to_owned();
        let (kind, topic, corr_id) = (text("kind"), text("topic"), text("corr_id"));
        let payload = &message["payload"];

        if !self.welcomed {
            // Until the Brain has let us in, the only message that means anything is its answer.
            if kind != "ACK" || topic != "sys.welcome" || corr_id != self.hello {
                return Vec::new();
            }
            if payload["accepted"] != json!(true) {
                let reason = payload["reason"].as_str().unwrap_or("no reason given");
                return vec![Out::Close(format!("the Brain refused sys.hello: {reason}"))];
            }
            self.welcomed = true;
            // First what the robot can do now, then whatever happened while nobody was listening.
            let mut out = self.publish_manifest(now);
            for mut fact in std::mem::take(&mut self.held) {
                self.seq = self.seq.wrapping_add(1);
                fact["seq"] = json!(self.seq);
                out.push(Out::Send(fact));
            }
            return out;
        }

        if !known_topic(&topic) {
            return self.error("unknown_topic", Some("topic"), Some(&message), now);
        }
        if kind != "CMD" {
            // Heartbeats and receipts: being heard from is all they are for, until the outbox.
            return Vec::new();
        }

        // A command. Never execute the same key twice: a retry gets the original answer (§4).
        let key = text("idem_key");
        self.seen
            .retain(|_, (_, at)| now.mono.saturating_sub(*at) < IDEM_WINDOW);
        match self.seen.get(&key) {
            Some((Seen::Answered(answer), _)) => {
                let answer = answer.clone();
                return self.say("ACK", &topic, Some(&corr_id), answer, now);
            }
            Some((Seen::Pending, _)) => return Vec::new(),
            None => {}
        }
        // Expired by this side's clock: `decode` can only compare the sender's two with each other.
        if message["expires_at"]
            .as_f64()
            .is_some_and(|at| at <= now.wall_ms as f64)
        {
            return self.answer(&key, &topic, &corr_id, refusal("expired"), now);
        }

        let goal_id = payload["goal_id"].as_str().filter(|id| !id.is_empty());
        let answer = match (topic.as_str(), goal_id) {
            ("nav.goto", Some(goal_id)) => match self.goto(payload) {
                Err(answer) => answer,
                Ok(pose) => {
                    self.goal = Some(Goal {
                        corr_id: corr_id.clone(),
                        key: key.clone(),
                        goal_id: goal_id.into(),
                        answered: false,
                        started: now.mono,
                        feedback_at: None,
                    });
                    self.seen.insert(key, (Seen::Pending, now.mono));
                    // No ACK yet: it says what Nav2 said, and Nav2 has not been asked.
                    return vec![Out::Robot(Robot::Goto { corr_id, pose })];
                }
            },
            // The Brain cancels on a corr_id of its own and names the goal in the payload, so
            // that — not ENVELOPE §3's shared corr_id — is what a cancel is matched by.
            ("nav.cancel", Some(goal_id)) => {
                match self.goal.as_ref().filter(|g| g.goal_id == goal_id) {
                    None => refusal("goal_not_found"),
                    Some(goal) => {
                        let cancel = Out::Robot(Robot::Cancel {
                            corr_id: goal.corr_id.clone(),
                        });
                        let done = json!({ "accepted": true, "exec_status": "completed" });
                        let mut out = self.answer(&key, &topic, &corr_id, done, now);
                        out.push(cancel);
                        return out;
                    }
                }
            }
            ("nav.goto" | "nav.cancel", None) => refusal("invalid_navigation_payload"),
            // On the list and addressed to a Heart, but not on this robot yet. An ACK that says no
            // is answered at once; an error would leave the Brain waiting out its timeout.
            ("voice.speak", _) => refusal("local_voice_unavailable"),
            _ => refusal("unsupported"),
        };
        self.answer(&key, &topic, &corr_id, answer, now)
    }

    /// Where a `nav.goto` wants the robot, or the refusal it gets instead.
    fn goto(&self, payload: &Value) -> Result<Pose, Value> {
        let number = |name: &str| payload[name].as_f64().filter(|n| n.is_finite());
        let pose = match (number("x"), number("y"), payload["region"].as_str()) {
            (Some(x), Some(y), _) => (x, y, number("yaw").unwrap_or(0.0)),
            (_, _, Some(region)) => match self.regions.get(&place(region)) {
                Some(pose) => *pose,
                None => {
                    let mut answer = refusal("unknown_region");
                    answer["details"] = json!({ "region": region });
                    return Err(answer);
                }
            },
            _ => return Err(refusal("invalid_navigation_payload")),
        };
        if let Some(code) = &self.fault {
            let mut answer = refusal("active_fault");
            answer["details"] = json!({ "code": code });
            return Err(answer);
        }
        if !self.nav_ready {
            return Err(refusal("navigation_unavailable"));
        }
        if self.goal.is_some() {
            return Err(refusal("navigation_goal_active"));
        }
        Ok(pose)
    }

    /// Nav2 took the goal, or would not.
    pub fn goal_answered(&mut self, corr_id: &str, accepted: bool, now: Now) -> Vec<Out> {
        let Some(goal) = self.goal.as_mut().filter(|g| g.corr_id == corr_id) else {
            return Vec::new();
        };
        goal.answered = accepted;
        let key = goal.key.clone();
        let answer = if accepted {
            json!({ "accepted": true, "exec_status": "executing", "goal_id": goal.goal_id })
        } else {
            self.goal = None;
            refusal("navigation_rejected")
        };
        self.answer(&key, "nav.goto", corr_id, answer, now)
    }

    /// Progress from Nav2. Sent at most once a second.
    pub fn progress(
        &mut self,
        corr_id: &str,
        remaining_m: f64,
        remaining_s: f64,
        now: Now,
    ) -> Vec<Out> {
        let Some(goal) = self.goal.as_mut().filter(|g| g.corr_id == corr_id) else {
            return Vec::new();
        };
        // Nav2 reports 0 m to go until it has planned a path. Passed on, that reads as "arrived"
        // to whoever is watching, a second after the robot was asked to leave.
        if remaining_m <= 0.0 {
            return Vec::new();
        }
        if goal
            .feedback_at
            .is_some_and(|at| now.mono.saturating_sub(at) < FEEDBACK_EVERY)
        {
            return Vec::new();
        }
        goal.feedback_at = Some(now.mono);
        let payload = json!({
            "distance_remaining_m": remaining_m,
            "estimated_time_remaining_s": remaining_s,
        });
        self.say("EVT", "nav.feedback", Some(corr_id), payload, now)
    }

    /// The goal is over, one way or another. Always said: the Brain takes no new goal while it
    /// believes one is running, so a result that goes missing wedges it.
    pub fn goal_ended(&mut self, corr_id: &str, ended: Ended, now: Now) -> Vec<Out> {
        let Some(goal) = self.goal.take_if(|g| g.corr_id == corr_id) else {
            return Vec::new();
        };
        // The Brain's check wants a pose with numbers in it; before the robot has ever said where
        // it is, that is the map's origin.
        let (x, y, yaw) = self.pose.unwrap_or_default();
        let mut payload = json!({
            "success": ended == Ended::Succeeded,
            "total_time_s": now.mono.saturating_sub(goal.started).as_secs_f64(),
            "final_pose": { "x": x, "y": y, "yaw": yaw },
        });
        match ended {
            Ended::Succeeded => {}
            Ended::Cancelled => payload["reason"] = json!("cancelled"),
            Ended::Aborted => payload["reason"] = json!("aborted"),
        }
        self.fact("nav.result", Some(&goal.corr_id), payload, now)
    }

    /// Where the robot says it is.
    pub fn pose(&mut self, pose: Pose) {
        self.pose = Some(pose);
    }

    /// The robot's fault state changed (`None` = healthy). A new fault is reported upward.
    pub fn fault(&mut self, fault: Option<String>, now: Now) -> Vec<Out> {
        if self.fault == fault {
            return Vec::new();
        }
        self.fault = fault;
        let mut out = Vec::new();
        if let Some(code) = self.fault.clone() {
            let payload =
                json!({ "subsystem": "safety_supervisor", "code": code, "severity": "error" });
            out = self.fact("diag.fault", None, payload, now);
        }
        out.extend(self.publish_manifest(now));
        out
    }

    /// Nav2 came up, or went away.
    pub fn nav_ready(&mut self, ready: bool, now: Now) -> Vec<Out> {
        self.nav_ready = ready;
        self.publish_manifest(now)
    }

    /// What the robot can do right now, and why not: only what the robot itself can verify
    /// (HEART_DECISIONS 19). Sent when it differs from the last one sent on this connection.
    fn publish_manifest(&mut self, now: Now) -> Vec<Out> {
        let unavailable = |reason: String| json!({ "state": "unavailable", "reason": reason });
        let fault = self
            .fault
            .as_ref()
            .map(|code| format!("the robot has a fault: {code}"));
        let driving = match (&fault, self.nav_ready) {
            (Some(why), _) => unavailable(why.clone()),
            (None, false) => unavailable("navigation is not running".into()),
            (None, true) => json!({ "state": "available" }),
        };
        let manifest = json!({ "capabilities": {
            "driving": driving,
            "docking": unavailable("docking is not built yet".into()),
            "recording": unavailable("recording is not built yet".into()),
            "local-voice": unavailable("the offline voice is not on the robot yet".into()),
            "robot-health": fault.map_or(json!({ "state": "available" }), unavailable),
        }});
        if !self.welcomed || manifest == self.manifest {
            return Vec::new();
        }
        (self.manifest, self.manifest_at) = (manifest.clone(), now.mono);
        self.say("EVT", "cap.manifest", None, manifest, now)
    }

    /// Call about once a second: heartbeat and manifest when they are due, and gives up on a link
    /// that has been silent for three heartbeats.
    pub fn tick(&mut self, now: Now) -> Vec<Out> {
        if now.mono.saturating_sub(self.heard_at) >= DEAD_AFTER {
            return vec![Out::Close("nothing heard from the Brain for 15 s".into())];
        }
        let mut out = Vec::new();
        if let Some(goal) = self
            .goal
            .take_if(|g| !g.answered && now.mono.saturating_sub(g.started) >= ANSWER_WITHIN)
        {
            out.push(Out::Robot(Robot::Cancel {
                corr_id: goal.corr_id.clone(),
            }));
            let unanswered = refusal("navigation_unavailable");
            out.extend(self.answer(&goal.key, "nav.goto", &goal.corr_id, unanswered, now));
        }
        if self.welcomed && now.mono.saturating_sub(self.heartbeat_at) >= HEARTBEAT_EVERY {
            self.heartbeat_at = now.mono;
            // The Brain's watchdog only counts a heartbeat whose status is one of ok, degraded
            // and fault, and that carries a wall-clock time.
            let status = if self.fault.is_some() { "fault" } else { "ok" };
            let payload = json!({ "status": status, "t_wall_ms": now.wall_ms });
            out.extend(self.say("EVT", "sys.heartbeat", None, payload, now));
        }
        if self.welcomed && now.mono.saturating_sub(self.manifest_at) >= MANIFEST_EVERY {
            self.manifest = Value::Null; // due again, changed or not
            out.extend(self.publish_manifest(now));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALL: u64 = 1_786_412_400_000;

    fn at(ms: u64) -> Now {
        Now {
            mono: Duration::from_millis(ms),
            wall_ms: WALL + ms,
        }
    }

    /// A command as the Brain sends it: a UUID for the correlation, another for the key.
    fn cmd(topic: &str, corr_id: &str, key: &str, payload: Value) -> Vec<u8> {
        json!({
            "msg_id": format!("01BRAIN{corr_id}"), "corr_id": corr_id, "t_mono_ns": "1",
            "t_wall_ms": WALL, "kind": "CMD", "topic": topic, "seq": 1, "idem_key": key,
            "payload": payload,
        })
        .to_string()
        .into_bytes()
    }

    fn goto(corr_id: &str, key: &str, goal_id: &str) -> Vec<u8> {
        let payload = json!({ "x": 3.2, "y": -1.1, "yaw": 1.57, "goal_id": goal_id });
        cmd("nav.goto", corr_id, key, payload)
    }

    /// The envelopes in `outs`, each checked against the shared contract on the way out: what
    /// this gateway sends has to be something the conformance decoder accepts.
    fn sent(outs: &[Out]) -> Vec<Value> {
        outs.iter()
            .filter_map(|out| match out {
                Out::Send(envelope) => {
                    decode(envelope.to_string().as_bytes())
                        .unwrap_or_else(|e| panic!("sent an invalid envelope: {e:?}\n{envelope}"));
                    Some(envelope.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn robot(outs: &[Out]) -> Vec<Robot> {
        outs.iter()
            .filter_map(|out| match out {
                Out::Robot(robot) => Some(robot.clone()),
                _ => None,
            })
            .collect()
    }

    fn on(topic: &str, envelopes: &[Value]) -> Vec<Value> {
        envelopes
            .iter()
            .filter(|e| e["topic"] == topic)
            .cloned()
            .collect()
    }

    fn welcome(session: &mut Session, hello: &Value, now: Now) -> Vec<Out> {
        let welcome = json!({
            "msg_id": "01BRAINWELCOME", "corr_id": hello["corr_id"], "t_mono_ns": "1",
            "t_wall_ms": WALL, "kind": "ACK", "topic": "sys.welcome", "seq": 1,
            "payload": { "accepted": true, "session_id": "sess-1", "server_wall_ms": WALL },
        });
        session.inbound(welcome.to_string().as_bytes(), now)
    }

    fn beat() -> Vec<u8> {
        json!({
            "msg_id": "01BRAINBEAT", "corr_id": "01BRAINBEAT", "t_mono_ns": "1",
            "t_wall_ms": WALL, "kind": "EVT", "topic": "sys.heartbeat", "seq": 9,
            "payload": { "status": "ok", "t_wall_ms": WALL },
        })
        .to_string()
        .into_bytes()
    }

    fn regions() -> HashMap<String, Pose> {
        HashMap::from([
            ("kitchen".to_owned(), (2.5, -0.3, 0.0)),
            ("Living Room".to_owned(), (-1.6, 0.0, 3.0)),
        ])
    }

    /// A session the Brain has let in, on a robot with Nav2 running.
    fn welcomed() -> Session {
        let mut session = Session::new("heart-test", "token", regions());
        session.nav_ready(true, at(0));
        let hello = session.connect(at(0));
        welcome(&mut session, &hello, at(10));
        assert!(session.welcomed());
        session
    }

    #[test]
    fn hello_is_a_valid_command_that_names_a_heart() {
        let hello = Session::new("heart-test", "secret", regions()).connect(at(0));
        decode(hello.to_string().as_bytes()).expect("sys.hello must pass the shared decoder");
        assert_eq!(hello["kind"], "CMD");
        assert_eq!(hello["topic"], "sys.hello");
        assert_eq!(hello["corr_id"], hello["msg_id"]);
        assert_eq!(hello["seq"], 1);
        assert!(hello["idem_key"].as_str().is_some_and(|k| !k.is_empty()));
        let payload = &hello["payload"];
        assert_eq!(payload["role"], "heart");
        assert_eq!(payload["device_id"], "heart-test");
        assert_eq!(payload["token"], "secret");
        assert_eq!(payload["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(hello["t_mono_ns"], "0", "a string of digits, not a number");
    }

    #[test]
    fn nothing_is_acted_on_before_the_welcome() {
        let mut session = Session::new("heart-test", "token", regions());
        session.nav_ready(true, at(0));
        session.connect(at(0));
        assert!(session.inbound(&goto("C1", "k1", "g-1"), at(5)).is_empty());
        assert!(!session.welcomed());
    }

    #[test]
    fn a_refused_hello_closes_the_connection() {
        let mut session = Session::new("heart-test", "wrong", regions());
        let hello = session.connect(at(0));
        let refused = json!({
            "msg_id": "01BRAINWELCOME", "corr_id": hello["corr_id"], "t_mono_ns": "1",
            "t_wall_ms": WALL, "kind": "ACK", "topic": "sys.welcome", "seq": 1,
            "payload": { "accepted": false, "session_id": "", "reason": "Unauthenticated token" },
        });
        let outs = session.inbound(refused.to_string().as_bytes(), at(10));
        assert!(matches!(&outs[..], [Out::Close(why)] if why.contains("Unauthenticated token")));
    }

    #[test]
    fn the_welcome_is_answered_with_what_the_robot_can_do() {
        let mut session = Session::new("heart-test", "token", regions());
        let hello = session.connect(at(0));
        let manifests = on(
            "cap.manifest",
            &sent(&welcome(&mut session, &hello, at(10))),
        );
        assert_eq!(manifests[0]["kind"], "EVT");
        let capabilities = &manifests[0]["payload"]["capabilities"];
        // Nav2 has not come up yet, and the robot says so rather than claiming it can drive.
        assert_eq!(capabilities["driving"]["state"], "unavailable");
        assert_eq!(
            capabilities["driving"]["reason"],
            "navigation is not running"
        );
        assert_eq!(capabilities["robot-health"]["state"], "available");
        for id in ["docking", "recording", "local-voice"] {
            assert_eq!(capabilities[id]["state"], "unavailable", "{id}");
            assert!(capabilities[id]["reason"].is_string(), "{id}");
        }

        // It changes the moment the robot does, and is repeated every ten seconds regardless.
        let changed = sent(&session.nav_ready(true, at(500)));
        assert_eq!(
            changed[0]["payload"]["capabilities"]["driving"]["state"],
            "available"
        );
        assert!(on("cap.manifest", &sent(&session.tick(at(5_000)))).is_empty());
        assert_eq!(
            on("cap.manifest", &sent(&session.tick(at(10_500)))).len(),
            1
        );
    }

    #[test]
    fn a_goal_goes_to_the_robot_and_is_acked_when_nav2_answers() {
        let mut session = welcomed();
        let outs = session.inbound(&goto("C7", "key-C7", "g-88"), at(100));
        let expected = Robot::Goto {
            corr_id: "C7".into(),
            pose: (3.2, -1.1, 1.57),
        };
        assert_eq!(robot(&outs), [expected]);
        assert!(
            sent(&outs).is_empty(),
            "no ACK until Nav2 has said yes or no"
        );

        let acks = sent(&session.goal_answered("C7", true, at(130)));
        assert_eq!(acks[0]["kind"], "ACK");
        assert_eq!(acks[0]["topic"], "nav.goto");
        assert_eq!(acks[0]["corr_id"], "C7");
        assert_eq!(
            acks[0]["payload"],
            json!({ "accepted": true, "exec_status": "executing", "goal_id": "g-88" })
        );
    }

    #[test]
    fn a_named_place_is_looked_up_and_an_unknown_one_refused() {
        let mut session = welcomed();
        let named = |region: &str, n: u32| {
            let payload = json!({ "region": region, "goal_id": format!("g-{n}") });
            cmd("nav.goto", &format!("C{n}"), &format!("k{n}"), payload)
        };
        let outs = session.inbound(&named("attic", 1), at(100));
        assert!(robot(&outs).is_empty());
        let ack = &sent(&outs)[0]["payload"];
        assert_eq!(ack["accepted"], false);
        assert_eq!(ack["reason"], "unknown_region");
        assert_eq!(ack["details"]["region"], "attic");

        let outs = session.inbound(&named("kitchen", 2), at(200));
        let expected = Robot::Goto {
            corr_id: "C2".into(),
            pose: (2.5, -0.3, 0.0),
        };
        assert_eq!(robot(&outs), [expected]);
        session.goal_ended("C2", Ended::Succeeded, at(300));

        // However the model spells it.
        for (n, spelling) in ["living_room", " Living-Room ", "LIVING ROOM"]
            .iter()
            .enumerate()
        {
            let n = n as u32 + 3;
            let outs = session.inbound(&named(spelling, n), at(400));
            assert_eq!(robot(&outs).len(), 1, "{spelling}");
            session.goal_ended(&format!("C{n}"), Ended::Succeeded, at(500));
        }
    }

    #[test]
    fn a_goal_nav2_never_answers_is_given_up_on() {
        let mut session = welcomed();
        session.inbound(&goto("C7", "key-C7", "g-88"), at(0));
        assert!(robot(&session.tick(at(4_000))).is_empty());
        let outs = session.tick(at(5_000));
        assert_eq!(
            robot(&outs),
            [Robot::Cancel {
                corr_id: "C7".into()
            }]
        );
        let acks = on("nav.goto", &sent(&outs));
        assert_eq!(acks[0]["payload"]["reason"], "navigation_unavailable");
        // The slot is free again, and a late answer from Nav2 changes nothing.
        assert!(session.goal_answered("C7", true, at(5_500)).is_empty());
        assert_eq!(
            robot(&session.inbound(&goto("C8", "key-C8", "g-89"), at(6_000))).len(),
            1
        );
        // One that was answered is left alone however long it runs.
        session.goal_answered("C8", true, at(6_100));
        assert!(robot(&session.tick(at(60_000 - 45_500))).is_empty());
    }

    #[test]
    fn a_goal_is_refused_with_the_reason_it_cannot_be_taken() {
        fn refused(session: &mut Session, payload: Value) -> Value {
            let key = ulid::Ulid::new().to_string();
            let outs = session.inbound(&cmd("nav.goto", "C1", &key, payload), at(100));
            assert!(robot(&outs).is_empty());
            sent(&outs)[0]["payload"].clone()
        }
        let good = || json!({ "x": 1.0, "y": 0.0, "goal_id": "g-1" });
        let invalid = "invalid_navigation_payload";

        let mut session = welcomed();
        assert_eq!(
            refused(&mut session, json!({ "x": 1.0, "y": 0.0 }))["reason"],
            invalid
        );
        assert_eq!(
            refused(&mut session, json!({ "goal_id": "g-1" }))["reason"],
            invalid
        );
        assert_eq!(
            refused(&mut session, json!({ "x": 1.0, "goal_id": "g-1" }))["reason"],
            invalid
        );

        session.nav_ready(false, at(50));
        assert_eq!(
            refused(&mut session, good())["reason"],
            "navigation_unavailable"
        );
        session.nav_ready(true, at(60));

        session.fault(Some("TIP_OVER".into()), at(70));
        let ack = refused(&mut session, good());
        assert_eq!(ack["reason"], "active_fault");
        assert_eq!(ack["details"]["code"], "TIP_OVER");
        assert_eq!(ack["exec_status"], "rejected");
        session.fault(None, at(80));

        assert_eq!(
            robot(&session.inbound(&goto("C7", "key-C7", "g-88"), at(90))).len(),
            1
        );
        assert_eq!(
            refused(&mut session, good())["reason"],
            "navigation_goal_active"
        );
    }

    #[test]
    fn a_goal_nav2_will_not_take_is_refused_and_forgotten() {
        let mut session = welcomed();
        session.inbound(&goto("C7", "key-C7", "g-88"), at(100));
        let acks = sent(&session.goal_answered("C7", false, at(130)));
        assert_eq!(acks[0]["payload"]["accepted"], false);
        assert_eq!(acks[0]["payload"]["reason"], "navigation_rejected");
        assert!(session.goal_ended("C7", Ended::Aborted, at(200)).is_empty());
        // And the next goal is free to run.
        assert_eq!(
            robot(&session.inbound(&goto("C8", "key-C8", "g-89"), at(300))).len(),
            1
        );
    }

    #[test]
    fn a_retried_command_gets_its_original_answer_and_the_robot_moves_once() {
        let mut session = welcomed();
        assert_eq!(
            robot(&session.inbound(&goto("C7", "key-C7", "g-88"), at(100))).len(),
            1
        );
        // The retry arrives before Nav2 has answered: nothing new happens.
        assert!(session
            .inbound(&goto("C7", "key-C7", "g-88"), at(110))
            .is_empty());
        let first = sent(&session.goal_answered("C7", true, at(130)));
        // And again afterwards: the same answer, and still no second trip.
        let outs = session.inbound(&goto("C7", "key-C7", "g-88"), at(2_000));
        assert!(robot(&outs).is_empty());
        assert_eq!(sent(&outs)[0]["payload"], first[0]["payload"]);
        // The key outlives the connection it arrived on.
        session.disconnected();
        let hello = session.connect(at(3_000));
        welcome(&mut session, &hello, at(3_010));
        let outs = session.inbound(&goto("C7", "key-C7", "g-88"), at(3_100));
        assert!(robot(&outs).is_empty());
        assert_eq!(sent(&outs)[0]["payload"], first[0]["payload"]);
    }

    #[test]
    fn a_key_is_forgotten_ten_minutes_after_its_answer() {
        let mut session = welcomed();
        let speak = cmd(
            "voice.speak",
            "S1",
            "speak-1",
            json!({ "text": "hi", "lang": "en" }),
        );
        session.inbound(&speak, at(1_000));
        session.inbound(&beat(), at(600_000));
        assert!(
            session.seen.contains_key("speak-1"),
            "599 s on, still remembered"
        );
        // The cache is swept when the next command arrives.
        let other = cmd("nav.cancel", "X", "other-key", json!({ "goal_id": "g-0" }));
        session.inbound(&other, at(601_600));
        assert!(
            !session.seen.contains_key("speak-1"),
            "forgotten after the window"
        );
    }

    #[test]
    fn an_expired_command_is_refused_by_either_clock() {
        let mut session = welcomed();
        let mut late: Value = serde_json::from_slice(&goto("C7", "key-C7", "g-88")).unwrap();
        // Already past by the sender's own clock: the decoder says so.
        late["expires_at"] = json!(WALL - 1);
        let outs = session.inbound(late.to_string().as_bytes(), at(100));
        assert!(robot(&outs).is_empty());
        assert_eq!(sent(&outs)[0]["payload"]["reason"], "expired");
        assert_eq!(sent(&outs)[0]["corr_id"], "C7");
        // Fine when it was sent, past by the time it arrived: only this side can know.
        let mut late: Value = serde_json::from_slice(&goto("C9", "key-C9", "g-90")).unwrap();
        late["expires_at"] = json!(WALL + 1_000);
        let outs = session.inbound(late.to_string().as_bytes(), at(5_000));
        assert!(robot(&outs).is_empty());
        assert_eq!(sent(&outs)[0]["payload"]["reason"], "expired");
        // And one with time left goes through.
        let mut fresh: Value = serde_json::from_slice(&goto("C10", "key-C10", "g-91")).unwrap();
        fresh["expires_at"] = json!(WALL + 30_000);
        assert_eq!(
            robot(&session.inbound(fresh.to_string().as_bytes(), at(5_100))).len(),
            1
        );
    }

    #[test]
    fn a_fault_is_reported_once_and_changes_what_the_robot_says_it_can_do() {
        let mut session = welcomed();
        let out = sent(&session.fault(Some("TIP_OVER".into()), at(50)));
        let faults = on("diag.fault", &out);
        assert_eq!(faults[0]["kind"], "EVT");
        assert_eq!(
            faults[0]["payload"],
            json!({ "subsystem": "safety_supervisor", "code": "TIP_OVER", "severity": "error" })
        );
        let capabilities = &on("cap.manifest", &out)[0]["payload"]["capabilities"];
        assert_eq!(capabilities["driving"]["state"], "unavailable");
        assert_eq!(capabilities["robot-health"]["state"], "unavailable");
        assert!(
            session.fault(Some("TIP_OVER".into()), at(60)).is_empty(),
            "said once"
        );

        // A fault that arrives between the hello and the welcome waits, and leaves no gap in seq.
        let mut connecting = Session::new("heart-test", "token", regions());
        let hello = connecting.connect(at(0));
        assert!(connecting.fault(Some("TIP_OVER".into()), at(5)).is_empty());
        let out = sent(&welcome(&mut connecting, &hello, at(10)));
        let seqs: Vec<_> = out.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
        assert_eq!((hello["seq"].as_u64(), seqs), (Some(1), vec![2, 3]));
        assert_eq!(out[1]["topic"], "diag.fault");

        let beats = on("sys.heartbeat", &sent(&session.tick(at(5_000))));
        assert_eq!(beats[0]["payload"]["status"], "fault");

        let out = sent(&session.fault(None, at(6_000)));
        assert!(
            on("diag.fault", &out).is_empty(),
            "getting better is not a fault"
        );
        assert_eq!(
            out[0]["payload"]["capabilities"]["driving"]["state"],
            "available"
        );
    }

    #[test]
    fn progress_crosses_once_a_second_however_often_nav2_reports_it() {
        let mut session = welcomed();
        session.inbound(&goto("C7", "key-C7", "g-88"), at(0));
        session.goal_answered("C7", true, at(10));
        let mut feedback = Vec::new();
        for tick in 0..150 {
            // 50 Hz for three seconds.
            feedback.extend(sent(&session.progress("C7", 2.1, 8.5, at(20 + tick * 20))));
        }
        assert_eq!(feedback.len(), 3);
        assert_eq!(feedback[0]["kind"], "EVT");
        assert_eq!(feedback[0]["topic"], "nav.feedback");
        assert_eq!(feedback[0]["corr_id"], "C7");
        assert_eq!(
            feedback[0]["payload"],
            json!({ "distance_remaining_m": 2.1, "estimated_time_remaining_s": 8.5 })
        );
        assert!(session.progress("other", 1.0, 1.0, at(9_000)).is_empty());
        // Before Nav2 has a path it says 0 m to go; that is not progress, and is not passed on.
        assert!(session.progress("C7", 0.0, 0.0, at(10_000)).is_empty());
        assert_eq!(sent(&session.progress("C7", 1.9, 7.0, at(10_020))).len(), 1);
    }

    #[test]
    fn the_result_says_how_it_ended_and_where_and_is_said_once() {
        let mut session = welcomed();
        session.inbound(&goto("C7", "key-C7", "g-88"), at(0));
        session.goal_answered("C7", true, at(10));
        session.pose((3.19, -1.08, 1.56));
        let results = sent(&session.goal_ended("C7", Ended::Succeeded, at(8_879)));
        assert_eq!(results[0]["kind"], "EVT");
        assert_eq!(results[0]["topic"], "nav.result");
        assert_eq!(results[0]["corr_id"], "C7");
        assert_eq!(
            results[0]["payload"],
            json!({ "success": true, "total_time_s": 8.879,
                    "final_pose": { "x": 3.19, "y": -1.08, "yaw": 1.56 } })
        );
        assert!(session
            .goal_ended("C7", Ended::Succeeded, at(9_000))
            .is_empty());

        session.inbound(&goto("C8", "key-C8", "g-89"), at(10_000));
        let results = sent(&session.goal_ended("C8", Ended::Aborted, at(11_000)));
        assert_eq!(results[0]["payload"]["success"], false);
        assert_eq!(results[0]["payload"]["reason"], "aborted");
        assert!(
            results[0]["payload"]["final_pose"]["x"].is_number(),
            "the Brain's check needs it"
        );
    }

    #[test]
    fn a_cancel_names_its_goal_in_the_payload_not_by_correlation() {
        let mut session = welcomed();
        session.inbound(&goto("C7", "key-C7", "g-88"), at(0));
        session.goal_answered("C7", true, at(10));

        let other = cmd("nav.cancel", "X1", "c-1", json!({ "goal_id": "g-other" }));
        let outs = session.inbound(&other, at(100));
        assert!(robot(&outs).is_empty());
        assert_eq!(sent(&outs)[0]["payload"]["reason"], "goal_not_found");

        // A fresh corr_id, as the Brain sends it; the goal is the one named.
        let cancel = cmd("nav.cancel", "X2", "c-2", json!({ "goal_id": "g-88" }));
        let outs = session.inbound(&cancel, at(200));
        assert_eq!(
            robot(&outs),
            [Robot::Cancel {
                corr_id: "C7".into()
            }]
        );
        let ack = &sent(&outs)[0];
        assert_eq!(ack["topic"], "nav.cancel");
        assert_eq!(ack["corr_id"], "X2");
        assert_eq!(
            ack["payload"],
            json!({ "accepted": true, "exec_status": "completed" })
        );

        // Then Nav2 reports it, and the result travels on the goal's own correlation.
        let results = sent(&session.goal_ended("C7", Ended::Cancelled, at(400)));
        assert_eq!(results[0]["corr_id"], "C7");
        assert_eq!(results[0]["payload"]["success"], false);
        assert_eq!(results[0]["payload"]["reason"], "cancelled");
    }

    #[test]
    fn a_result_that_happens_with_no_link_waits_for_the_next_one() {
        let mut session = welcomed();
        session.inbound(&goto("C7", "key-C7", "g-88"), at(0));
        session.goal_answered("C7", true, at(10));
        session.disconnected();
        // The robot keeps driving and arrives; progress is worth nothing later, the result is.
        assert!(session.progress("C7", 1.0, 2.0, at(1_000)).is_empty());
        session.pose((3.2, -1.1, 1.57));
        assert!(session
            .goal_ended("C7", Ended::Succeeded, at(4_000))
            .is_empty());

        let hello = session.connect(at(60_000));
        assert_eq!(hello["seq"], 1, "seq starts again with the connection");
        let out = sent(&welcome(&mut session, &hello, at(60_010)));
        let results = on("nav.result", &out);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["corr_id"], "C7");
        assert_eq!(
            results[0]["t_wall_ms"],
            WALL + 4_000,
            "when it happened, not when it was sent"
        );
        assert_eq!(results[0]["payload"]["total_time_s"], 4.0);
        let seqs: Vec<_> = out.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
        assert_eq!(
            seqs,
            [2, 3],
            "the manifest, then what was held, numbered for this connection"
        );
        // Said once: the next connection has nothing left to deliver.
        session.disconnected();
        let hello = session.connect(at(120_000));
        assert!(on(
            "nav.result",
            &sent(&welcome(&mut session, &hello, at(120_010)))
        )
        .is_empty());
    }

    #[test]
    fn what_is_not_built_is_refused_and_what_is_not_known_is_an_error() {
        let mut session = welcomed();
        let speak = cmd(
            "voice.speak",
            "S1",
            "speak-1",
            json!({ "text": "salam", "lang": "ur" }),
        );
        let outs = sent(&session.inbound(&speak, at(100)));
        assert_eq!(outs[0]["kind"], "ACK");
        assert_eq!(outs[0]["corr_id"], "S1");
        assert_eq!(outs[0]["payload"]["reason"], "local_voice_unavailable");

        let window = cmd("sched.window", "W1", "w-1", json!({ "occurrences": [] }));
        assert_eq!(
            sent(&session.inbound(&window, at(150)))[0]["payload"]["reason"],
            "unsupported"
        );

        let unknown = cmd("nav.teleport", "T1", "t-1", json!({}));
        let outs = sent(&session.inbound(&unknown, at(200)));
        assert_eq!(outs[0]["kind"], "ERR");
        assert_eq!(outs[0]["payload"]["reason"], "unknown_topic");
        assert_eq!(outs[0]["payload"]["msg_id"], "01BRAINT1");

        let outs = sent(&session.inbound(b"{ not json", at(300)));
        assert_eq!(outs[0]["payload"]["reason"], "malformed");
    }

    #[test]
    fn a_heartbeat_every_five_seconds_and_a_dead_link_after_fifteen() {
        let mut session = welcomed();
        assert!(on("sys.heartbeat", &sent(&session.tick(at(1_000)))).is_empty());
        let beats = on("sys.heartbeat", &sent(&session.tick(at(5_000))));
        assert_eq!(beats[0]["kind"], "EVT");
        assert_eq!(
            beats[0]["payload"],
            json!({ "status": "ok", "t_wall_ms": WALL + 5_000 })
        );
        assert!(on("sys.heartbeat", &sent(&session.tick(at(6_000)))).is_empty());
        // The Brain's own heartbeat keeps the link alive...
        assert!(session.inbound(&beat(), at(12_000)).is_empty());
        assert!(!matches!(&session.tick(at(20_000))[..], [Out::Close(_)]));
        // ...and fifteen seconds of nothing ends it.
        assert!(matches!(&session.tick(at(27_000))[..], [Out::Close(_)]));
        // A Brain that never answers the hello is given up on the same way.
        let mut session = Session::new("heart-test", "token", regions());
        session.connect(at(0));
        assert!(session.tick(at(14_000)).is_empty());
        assert!(matches!(&session.tick(at(15_000))[..], [Out::Close(_)]));
    }
}
