//! The safety supervisor's decisions, with no ROS in sight: what speed may pass, and when the
//! command stream counts as dead. `main.rs` is the glue that feeds this from `/cmd_vel`.

use std::time::{Duration, Instant};

/// How often the watchdog looks at the clock.
pub const TICK: Duration = Duration::from_millis(20);

/// The shortest silence that may count as a dead command stream. Phone teleop sends a frame every
/// 100 ms and one lost frame must never be a stop (TELEOP.md §4), so the window has to outlast two
/// frame periods, with two ticks to spare for the watchdog's own tick and a late frame.
pub const MIN_WINDOW: Duration = Duration::from_millis(240);

pub struct Supervisor {
    max_linear: f64,  // m/s
    max_angular: f64, // rad/s
    window: Duration, // how long silence may last before the stop goes out
    last_command: Option<Instant>,
}

impl Supervisor {
    /// `stop_within` is the promise: the robot is standing still this long after the last command.
    /// `brake` is how long the drive takes from the stop command to standstill at the speed limits
    /// — measured on the drive, not computed. What is left over is how long silence may last.
    ///
    /// Panics on a limit that is negative or not a number, or when braking leaves less silence
    /// than `MIN_WINDOW`: a robot too fast for its brakes must not start. Brake harder or drive
    /// slower, then measure again.
    pub fn new(max_linear: f64, max_angular: f64, stop_within: Duration, brake: Duration) -> Self {
        assert!(max_linear.is_finite() && max_linear >= 0.0, "max_linear");
        assert!(max_angular.is_finite() && max_angular >= 0.0, "max_angular");
        let window = stop_within.saturating_sub(brake);
        assert!(
            window >= MIN_WINDOW,
            "window: {stop_within:?} less {brake:?} of braking leaves {window:?} of silence, under {MIN_WINDOW:?}"
        );
        Self {
            max_linear,
            max_angular,
            window,
            last_command: None,
        }
    }

    /// How long silence may last before the stop goes out.
    pub fn window(&self) -> Duration {
        self.window
    }

    /// A drive command arrived at `now`. Returns the (linear, angular) speed that may reach the
    /// wheels. A command that is not a number is a stop, and does not count as a sign of life.
    pub fn command(&mut self, linear: f64, angular: f64, now: Instant) -> (f64, f64) {
        if !linear.is_finite() || !angular.is_finite() {
            return (0.0, 0.0);
        }
        self.last_command = Some(now);
        (
            linear.clamp(-self.max_linear, self.max_linear),
            angular.clamp(-self.max_angular, self.max_angular),
        )
    }

    /// Watchdog tick at `now`: true when zero must be published. It fires on the last tick that
    /// still lands inside the window, not the first one after it, so the stop is out no later
    /// than `window` after the last command. Before any command has arrived it is always true.
    pub fn stalled(&self, now: Instant) -> bool {
        match self.last_command {
            None => true,
            Some(last) => now.duration_since(last) + TICK > self.window,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// The defaults in main.rs: stopped within 400 ms, 150 ms of it for braking.
    fn supervisor() -> Supervisor {
        Supervisor::new(0.5, 2.0, ms(400), ms(150))
    }

    #[test]
    fn silence_is_the_promise_less_the_braking() {
        assert_eq!(supervisor().window(), ms(250));
    }

    #[test]
    fn passes_a_command_inside_the_limits() {
        assert_eq!(supervisor().command(0.2, -0.5, Instant::now()), (0.2, -0.5));
    }

    #[test]
    fn clamps_both_directions() {
        let mut s = supervisor();
        let now = Instant::now();
        assert_eq!(s.command(5.0, 9.0, now), (0.5, 2.0));
        assert_eq!(s.command(-5.0, -9.0, now), (-0.5, -2.0));
    }

    #[test]
    fn a_command_that_is_not_a_number_is_a_stop_and_not_a_sign_of_life() {
        let mut s = supervisor();
        let now = Instant::now();
        assert_eq!(s.command(f64::NAN, 0.0, now), (0.0, 0.0));
        assert_eq!(s.command(0.2, f64::INFINITY, now), (0.0, 0.0));
        assert!(s.stalled(now));
    }

    #[test]
    fn stalled_until_the_first_command() {
        assert!(supervisor().stalled(Instant::now()));
    }

    #[test]
    fn a_steady_stream_never_stalls() {
        let mut s = supervisor();
        let start = Instant::now();
        // Nav2's controller runs at 20 Hz: a command every 50 ms, a tick every 20 ms.
        for tick in 0..500 {
            let now = start + TICK * tick;
            if tick % 3 == 0 {
                s.command(0.2, 0.0, now);
            }
            assert!(!s.stalled(now), "stalled at tick {tick}");
        }
    }

    #[test]
    fn the_stop_is_out_within_the_window() {
        let mut s = supervisor();
        let last = Instant::now();
        s.command(0.2, 0.0, last);
        // Ticks are not aligned with commands, so try every phase a tick could have.
        for phase in 0..20 {
            let first_stalled = (0..)
                .map(|n| ms(phase) + TICK * n)
                .find(|&age| s.stalled(last + age))
                .unwrap();
            assert!(first_stalled <= ms(250), "phase {phase}: {first_stalled:?}");
            assert!(first_stalled > ms(210), "phase {phase}: {first_stalled:?}");
        }
    }

    #[test]
    fn one_lost_teleop_frame_is_not_a_stop_and_two_are() {
        let mut s = supervisor();
        let last = Instant::now();
        s.command(0.2, 0.0, last);
        // Frames every 100 ms: one lost is a 200 ms gap, and the next may still be 20 ms late.
        for age in (0..=220).step_by(TICK.as_millis() as usize) {
            assert!(!s.stalled(last + ms(age)), "stalled {age} ms after a frame");
        }
        assert!(s.stalled(last + ms(300)));
    }

    #[test]
    fn a_new_command_ends_the_stall() {
        let mut s = supervisor();
        let start = Instant::now();
        s.command(0.2, 0.0, start);
        assert!(s.stalled(start + ms(500)));
        s.command(0.2, 0.0, start + ms(500));
        assert!(!s.stalled(start + ms(520)));
    }

    #[test]
    #[should_panic(expected = "max_linear")]
    fn refuses_a_limit_that_is_not_a_number() {
        Supervisor::new(f64::NAN, 2.0, ms(400), ms(150));
    }

    #[test]
    #[should_panic(expected = "window")]
    fn refuses_a_robot_too_fast_for_its_brakes() {
        Supervisor::new(0.5, 2.0, ms(400), ms(200));
    }
}
