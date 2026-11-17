//! The veto layer between everything that wants to drive and the wheels (H2.3).
//!
//! Anything that drives publishes `/cmd_vel`. Only this node publishes `/cmd_vel_safe`, and
//! `/cmd_vel_safe` is the only velocity topic the motors (in simulation, the Gazebo bridge) listen
//! to. So a command reaches the wheels clamped, and when commands stop arriving the wheels are told
//! to stop — a crashed planner, a dead link and a closed app all end the same way.

use mio_safety_supervisor::{Supervisor, TICK};
use rclrs::*;
use ros_env::geometry_msgs::msg::{Twist, TwistStamped};
use std::time::{Duration, Instant};

fn main() -> Result<(), RclrsError> {
    let mut executor = Context::default_from_env()?.create_basic_executor();
    let node = executor.create_node("mio_safety_supervisor")?;

    // Read once at start-up and read-only after: a limit that `ros2 param set` can raise mid-drive
    // is not a limit.
    //
    // `stop_within_ms` is the promise (HEART_SPEC N2, TELEOP.md §4): standing still this long after
    // the last command. `brake_ms` is how long the drive takes from the stop command to standstill
    // at `max_linear` and at `max_angular` — MEASURED, never assumed. The defaults are the
    // simulated drive's (mio_gazebo.xacro); on hardware all three are measured again together,
    // because a faster limit or a weaker brake makes `brake_ms` a lie.
    let max_linear = node
        .declare_parameter("max_linear")
        .default(0.5)
        .read_only()?
        .get();
    let max_angular = node
        .declare_parameter("max_angular")
        .default(2.0)
        .read_only()?
        .get();
    let stop_within_ms = node
        .declare_parameter("stop_within_ms")
        .default(400)
        .read_only()?
        .get();
    let brake_ms = node
        .declare_parameter("brake_ms")
        .default(150)
        .read_only()?
        .get();
    let millis = |n: i64, name| Duration::from_millis(n.try_into().expect(name));
    let supervisor = Supervisor::new(
        max_linear,
        max_angular,
        millis(stop_within_ms, "stop_within_ms"),
        millis(brake_ms, "brake_ms"),
    );
    let silence_ms = supervisor.window().as_millis();

    let safe = node.create_publisher::<TwistStamped>("cmd_vel_safe")?;
    let worker = node.create_worker(supervisor);

    let (publisher, logger) = (safe.clone(), node.logger().clone());
    let _commands = worker.create_subscription(
        "cmd_vel",
        move |supervisor: &mut Supervisor, mut command: TwistStamped| {
            let (linear, angular) = supervisor.command(
                command.twist.linear.x,
                command.twist.angular.z,
                Instant::now(),
            );
            // A differential drive has two degrees of freedom; nothing else is passed on.
            command.twist = Twist::default();
            command.twist.linear.x = linear;
            command.twist.angular.z = angular;
            if let Err(e) = publisher.publish(&command) {
                log_error!(&logger, "could not forward a command: {e}");
            }
        },
    )?;

    // The timer runs on the steady clock (rclrs's default), and the deadman is measured with
    // `Instant`: neither a jump in wall time nor a stalled simulation clock can hold it open.
    let (publisher, logger, clock) = (safe, node.logger().clone(), node.get_clock());
    let _watchdog = worker.create_timer_repeating(TICK, move |supervisor: &mut Supervisor| {
        if !supervisor.stalled(Instant::now()) {
            return;
        }
        let mut stop = TwistStamped::default();
        stop.header.frame_id = "base_link".into();
        if let Ok(stamp) = clock.now().to_ros_msg() {
            stop.header.stamp = stamp;
        }
        if let Err(e) = publisher.publish(&stop) {
            log_error!(&logger, "could not publish a stop: {e}");
        }
    })?;

    log_info!(
        node.logger(),
        "cmd_vel -> cmd_vel_safe, clamped to {max_linear} m/s and {max_angular} rad/s; standing still within {stop_within_ms} ms of the last command: {silence_ms} ms of silence, then {brake_ms} ms to brake"
    );
    executor.spin(SpinOptions::default()).first_error()?;
    Ok(())
}
