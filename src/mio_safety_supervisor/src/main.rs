//! The veto layer between everything that wants to drive and the wheels (H2.3, H2.4).
//!
//! Anything that drives publishes `/cmd_vel`. Only this node publishes `/cmd_vel_safe`, and
//! `/cmd_vel_safe` is the only velocity topic the motors (in simulation, the Gazebo bridge) listen
//! to. So a command reaches the wheels clamped, and when commands stop arriving the wheels are told
//! to stop — a crashed planner, a dead link and a closed app all end the same way.
//!
//! It also watches `/imu`. A tilt that lasts stops the wheels and latches until a person calls
//! `~/clear`; a silent IMU stops them until it is back. The order never changes: halt the motors,
//! and only then say so on `/diagnostics`, which is where the voice and the gateway hear of it.

use mio_safety_supervisor::{tilt_from_quaternion, Config, Supervisor, TICK};
use rclrs::*;
use ros_env::diagnostic_msgs::msg::{DiagnosticArray, DiagnosticStatus, KeyValue};
use ros_env::geometry_msgs::msg::{Twist, TwistStamped};
use ros_env::sensor_msgs::msg::Imu;
use ros_env::std_srvs::srv::{Trigger, Trigger_Request, Trigger_Response};
use std::time::{Duration, Instant};

fn main() -> Result<(), RclrsError> {
    let mut executor = Context::default_from_env()?.create_basic_executor();
    let node = executor.create_node("mio_safety_supervisor")?;

    // Read once at start-up and read-only after: a limit that `ros2 param set` can raise mid-drive
    // is not a limit. The defaults, and what each number means, are `Config` in lib.rs.
    //
    // `brake_ms`, the two speed limits and the drive's own acceleration limits are ONE measurement
    // (HEART_DECISIONS 47): a faster limit or a weaker brake makes `brake_ms` a lie.
    let defaults = Config::default();
    let float = |name: &'static str, default: f64| -> Result<f64, RclrsError> {
        Ok(node
            .declare_parameter(name)
            .default(default)
            .read_only()?
            .get())
    };
    let millis = |name: &'static str, default: Duration| -> Result<Duration, RclrsError> {
        let ms: i64 = node
            .declare_parameter(name)
            .default(default.as_millis() as i64)
            .read_only()?
            .get();
        Ok(Duration::from_millis(ms.try_into().expect(name)))
    };
    let config = Config {
        max_linear: float("max_linear", defaults.max_linear)?,
        max_angular: float("max_angular", defaults.max_angular)?,
        stop_within: millis("stop_within_ms", defaults.stop_within)?,
        brake: millis("brake_ms", defaults.brake)?,
        tip_angle: float("tip_angle_deg", defaults.tip_angle.to_degrees())?.to_radians(),
        hold: millis("hold_ms", defaults.hold)?,
        imu_timeout: millis("imu_timeout_ms", defaults.imu_timeout)?,
    };
    let supervisor = Supervisor::new(config.clone());
    log_info!(
        node.logger(),
        "cmd_vel -> cmd_vel_safe, clamped to {} m/s and {} rad/s; standing still within {} ms of the last command: {} ms of silence, then {} ms to brake; locked past {:.0} deg of tilt",
        config.max_linear,
        config.max_angular,
        config.stop_within.as_millis(),
        supervisor.window().as_millis(),
        config.brake.as_millis(),
        config.tip_angle.to_degrees()
    );

    let safe = node.create_publisher::<TwistStamped>("cmd_vel_safe")?;
    let diagnostics = node.create_publisher::<DiagnosticArray>("/diagnostics")?;
    let worker = node.create_worker(supervisor);

    // One way to say stop, used by the watchdog and by the tilt latch.
    let (publisher, logger, clock) = (safe.clone(), node.logger().clone(), node.get_clock());
    let stop = move || {
        let mut stop = TwistStamped::default();
        stop.header.frame_id = "base_link".into();
        if let Ok(stamp) = clock.now().to_ros_msg() {
            stop.header.stamp = stamp;
        }
        if let Err(e) = publisher.publish(&stop) {
            log_error!(&logger, "could not publish a stop: {e}");
        }
    };

    let (publisher, logger) = (safe, node.logger().clone());
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

    let (halt, logger) = (stop.clone(), node.logger().clone());
    let _imu =
        worker.create_subscription("imu", move |supervisor: &mut Supervisor, imu: Imu| {
            // REP 145: a first covariance element of -1 means "this IMU does not know its
            // orientation". That is a sensor with nothing to say about tilt, not a level robot.
            let q = &imu.orientation;
            let tilt = (imu.orientation_covariance[0] >= 0.0)
                .then(|| tilt_from_quaternion(q.x, q.y, q.z, q.w))
                .flatten();
            if supervisor.imu(tilt, Instant::now()) {
                // Halt first. The report below goes out on the watchdog's next tick, after this.
                halt();
                log_error!(&logger, "tipping: wheels stopped and locked until ~/clear");
            }
        })?;

    let _clear = worker.create_service::<Trigger, _>(
        "~/clear",
        |supervisor: &mut Supervisor, _: Trigger_Request| match supervisor.clear(Instant::now()) {
            Ok(()) => Trigger_Response {
                success: true,
                message: "unlocked".into(),
            },
            Err(why) => Trigger_Response {
                success: false,
                message: why.into(),
            },
        },
    )?;

    // The timer runs on the steady clock (rclrs's default), and silence is measured with
    // `Instant`: neither a jump in wall time nor a stalled simulation clock can hold it open.
    let (logger, clock) = (node.logger().clone(), node.get_clock());
    let _watchdog = worker.create_timer_repeating(TICK, move |supervisor: &mut Supervisor| {
        let now = Instant::now();
        if supervisor.must_stop(now) {
            stop();
        }
        // Always after the stop: halt, then report.
        let Some(health) = supervisor.report(now) else {
            return;
        };
        let mut report = DiagnosticArray::default();
        if let Ok(stamp) = clock.now().to_ros_msg() {
            report.header.stamp = stamp;
        }
        report.status.push(DiagnosticStatus {
            level: if health.fault.is_some() {
                DiagnosticStatus::ERROR
            } else {
                DiagnosticStatus::OK
            },
            name: "mio_safety_supervisor".into(),
            message: health.fault.map_or("OK", |fault| fault.code()).into(),
            hardware_id: String::new(),
            values: vec![KeyValue {
                key: "tilt_deg".into(),
                value: format!("{:.1}", health.tilt.to_degrees()),
            }],
        });
        if let Err(e) = diagnostics.publish(&report) {
            log_error!(&logger, "could not report health: {e}");
        }
    })?;

    executor.spin(SpinOptions::default()).first_error()?;
    Ok(())
}
