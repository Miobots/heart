//! The safety supervisor's decisions, with no ROS in sight: what speed may pass, when the command
//! stream counts as dead, and when the robot is tipping. `main.rs` is the glue that feeds this
//! from `/cmd_vel` and `/imu`.

use std::time::{Duration, Instant};

/// How often the watchdog looks at the clock.
pub const TICK: Duration = Duration::from_millis(20);

/// The shortest silence that may count as a dead command stream. Phone teleop sends a frame every
/// 100 ms and one lost frame must never be a stop (TELEOP.md §4), so the window has to outlast two
/// frame periods, with two ticks to spare for the watchdog's own tick and a late frame.
pub const MIN_WINDOW: Duration = Duration::from_millis(240);

/// How long the robot must have been upright before a tip-over may be cleared.
pub const UPRIGHT_BEFORE_CLEAR: Duration = Duration::from_secs(1);

/// How often health is reported when nothing changes: a subsystem that stops reporting is treated
/// as failed, not as quiet (HEART_SPEC R9).
pub const REPORT_EVERY: Duration = Duration::from_secs(1);

/// Every number the supervisor enforces. The defaults are the simulated robot's; on hardware they
/// are measured, not assumed.
#[derive(Clone, Debug)]
pub struct Config {
    pub max_linear: f64,  // m/s
    pub max_angular: f64, // rad/s
    /// The promise: the robot is standing still this long after the last command.
    pub stop_within: Duration,
    /// How long the drive takes from the stop command to standstill at the speed limits — measured
    /// on the drive, not computed. What is left of `stop_within` is how long silence may last.
    pub brake: Duration,
    /// Tilt from vertical, in radians, past which the robot counts as tipping.
    pub tip_angle: f64,
    /// How long a reading has to persist before it is believed: a tilt before it latches, a
    /// returning IMU before it is trusted again. One bump is not a fall.
    pub hold: Duration,
    /// How long the IMU may be silent before that is itself a fault.
    pub imu_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_linear: 0.5,
            max_angular: 2.0,
            stop_within: Duration::from_millis(400),
            brake: Duration::from_millis(160),
            tip_angle: 30_f64.to_radians(),
            hold: Duration::from_millis(100),
            imu_timeout: Duration::from_millis(500),
        }
    }
}

/// What stops the wheels whatever the commands say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Latched: stays until a person clears it.
    TipOver,
    /// Clears itself once the IMU has been back for `hold`.
    ImuNoData,
}

impl Fault {
    /// The structured code that goes upward (HEART_SPEC R9, `diag.fault`).
    pub fn code(self) -> &'static str {
        match self {
            Fault::TipOver => "TIP_OVER",
            Fault::ImuNoData => "IMU_NO_DATA",
        }
    }
}

/// One health report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Health {
    pub fault: Option<Fault>,
    /// The last tilt reading, in radians.
    pub tilt: f64,
}

/// The body's tilt from vertical, in radians, from its orientation. None when the quaternion is
/// not one (all zeros, or not a number): a sensor that sends garbage has sent nothing.
pub fn tilt_from_quaternion(x: f64, y: f64, z: f64, w: f64) -> Option<f64> {
    let norm = x * x + y * y + z * z + w * w;
    if !norm.is_finite() || norm < 1e-6 {
        return None;
    }
    // The world-z component of the body's z axis.
    Some((1.0 - 2.0 * (x * x + y * y) / norm).clamp(-1.0, 1.0).acos())
}

pub struct Supervisor {
    config: Config,
    window: Duration, // how long silence may last before the stop goes out
    last_command: Option<Instant>,
    last_imu: Option<Instant>,      // the last usable reading
    imu_since: Option<Instant>,     // when the current unbroken run of readings began
    tilted_since: Option<Instant>,  // when the current unbroken tilt began
    upright_since: Option<Instant>, // when the current unbroken upright spell began
    // ponytail: the latch lives in memory, so a supervisor that restarts comes back unlocked
    // unless the robot is still tilted. Write it to disk if a restart must not clear it.
    tipped: bool,
    tilt: f64,
    reported: Option<(Option<Fault>, Instant)>,
}

impl Supervisor {
    /// Panics on a limit that is negative or not a number, or when braking leaves less silence
    /// than `MIN_WINDOW`: a robot too fast for its brakes must not start. Brake harder or drive
    /// slower, then measure again.
    pub fn new(config: Config) -> Self {
        let limit = |n: f64| n.is_finite() && n >= 0.0;
        assert!(limit(config.max_linear), "max_linear");
        assert!(limit(config.max_angular), "max_angular");
        assert!(limit(config.tip_angle), "tip_angle");
        let window = config.stop_within.saturating_sub(config.brake);
        assert!(
            window >= MIN_WINDOW,
            "window: {:?} less {:?} of braking leaves {window:?} of silence, under {MIN_WINDOW:?}",
            config.stop_within,
            config.brake
        );
        Self {
            config,
            window,
            last_command: None,
            last_imu: None,
            imu_since: None,
            tilted_since: None,
            upright_since: None,
            tipped: false,
            tilt: 0.0,
            reported: None,
        }
    }

    /// How long silence may last before the stop goes out.
    pub fn window(&self) -> Duration {
        self.window
    }

    /// A drive command arrived at `now`. Returns the (linear, angular) speed that may reach the
    /// wheels: nothing while a fault stands, and a command that is not a number is a stop and
    /// does not count as a sign of life.
    pub fn command(&mut self, linear: f64, angular: f64, now: Instant) -> (f64, f64) {
        if !linear.is_finite() || !angular.is_finite() {
            return (0.0, 0.0);
        }
        self.last_command = Some(now);
        if self.fault(now).is_some() {
            return (0.0, 0.0);
        }
        (
            linear.clamp(-self.config.max_linear, self.config.max_linear),
            angular.clamp(-self.config.max_angular, self.config.max_angular),
        )
    }

    /// An IMU reading arrived at `now`: the body's tilt from vertical, or None when the reading is
    /// unusable. Returns true on the one reading that trips the latch, so the caller can stop the
    /// wheels before anything else hears about it.
    pub fn imu(&mut self, tilt: Option<f64>, now: Instant) -> bool {
        let Some(tilt) = tilt.filter(|t| t.is_finite()) else {
            return false;
        };
        if self.imu_silent(now) {
            // A new run of readings: what was known about tilt before the gap is not known now.
            (self.imu_since, self.tilted_since, self.upright_since) = (Some(now), None, None);
        }
        self.last_imu = Some(now);
        self.tilt = tilt;
        if tilt <= self.config.tip_angle {
            self.tilted_since = None;
            self.upright_since.get_or_insert(now);
            return false;
        }
        self.upright_since = None;
        let since = *self.tilted_since.get_or_insert(now);
        let trips = !self.tipped && now.duration_since(since) >= self.config.hold;
        self.tipped |= trips;
        trips
    }

    fn imu_silent(&self, now: Instant) -> bool {
        self.last_imu
            .is_none_or(|last| now.duration_since(last) > self.config.imu_timeout)
    }

    /// The fault standing at `now`, if any. A tip-over outranks a silent IMU: it needs a person.
    pub fn fault(&self, now: Instant) -> Option<Fault> {
        if self.tipped {
            return Some(Fault::TipOver);
        }
        let settled = self
            .imu_since
            .is_some_and(|since| now.duration_since(since) >= self.config.hold);
        (self.imu_silent(now) || !settled).then_some(Fault::ImuNoData)
    }

    /// A person asks to release the tip-over latch. Refused, with the reason, unless the IMU has
    /// shown the robot upright for `UPRIGHT_BEFORE_CLEAR`.
    pub fn clear(&mut self, now: Instant) -> Result<(), &'static str> {
        if !self.tipped {
            return Ok(());
        }
        if self.imu_silent(now) {
            return Err("no IMU data, so the robot cannot be seen to be upright");
        }
        match self.upright_since {
            Some(since) if now.duration_since(since) >= UPRIGHT_BEFORE_CLEAR => {
                self.tipped = false;
                Ok(())
            }
            Some(_) => Err("upright, but not for long enough yet"),
            None => Err("still tipped"),
        }
    }

    /// Watchdog tick at `now`: true when zero must be published — a fault stands, or the command
    /// stream is dead. For silence it fires on the last tick that still lands inside the window,
    /// not the first one after it, so the stop is out no later than `window` after the last
    /// command. Before any command has arrived it is always true.
    pub fn must_stop(&self, now: Instant) -> bool {
        self.fault(now).is_some()
            || self
                .last_command
                .is_none_or(|last| now.duration_since(last) + TICK > self.window)
    }

    /// Some when health is due to be reported at `now`: on any change, and every `REPORT_EVERY`
    /// when nothing changes.
    pub fn report(&mut self, now: Instant) -> Option<Health> {
        let fault = self.fault(now);
        let due = self
            .reported
            .is_none_or(|(last, at)| last != fault || now.duration_since(at) >= REPORT_EVERY);
        due.then(|| {
            self.reported = Some((fault, now));
            Health {
                fault,
                tilt: self.tilt,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    const UPRIGHT: Option<f64> = Some(0.02);
    const ON_ITS_SIDE: Option<f64> = Some(1.5);

    /// A supervisor whose IMU has been upright long enough to be trusted, and the moment that
    /// became true. With `imu_timeout` out of the way, for the tests that are not about the IMU.
    fn supervisor() -> (Supervisor, Instant) {
        let mut s = Supervisor::new(Config {
            imu_timeout: Duration::from_secs(3600),
            ..Config::default()
        });
        let start = Instant::now();
        s.imu(UPRIGHT, start);
        s.imu(UPRIGHT, start + ms(100));
        (s, start + ms(100))
    }

    /// Feeds one reading every 20 ms from `from` for `span`, and returns when the last one was.
    fn feed(s: &mut Supervisor, tilt: Option<f64>, from: Instant, span: Duration) -> Instant {
        let mut now = from;
        while now <= from + span {
            s.imu(tilt, now);
            now += TICK;
        }
        now - TICK
    }

    // ---- speed and silence ----------------------------------------------------------------

    #[test]
    fn silence_is_the_promise_less_the_braking() {
        assert_eq!(supervisor().0.window(), ms(240));
    }

    #[test]
    fn passes_a_command_inside_the_limits() {
        let (mut s, now) = supervisor();
        assert_eq!(s.command(0.2, -0.5, now), (0.2, -0.5));
    }

    #[test]
    fn clamps_both_directions() {
        let (mut s, now) = supervisor();
        assert_eq!(s.command(5.0, 9.0, now), (0.5, 2.0));
        assert_eq!(s.command(-5.0, -9.0, now), (-0.5, -2.0));
    }

    #[test]
    fn a_command_that_is_not_a_number_is_a_stop_and_not_a_sign_of_life() {
        let (mut s, now) = supervisor();
        assert_eq!(s.command(f64::NAN, 0.0, now), (0.0, 0.0));
        assert_eq!(s.command(0.2, f64::INFINITY, now), (0.0, 0.0));
        assert!(s.must_stop(now));
    }

    #[test]
    fn stopped_until_the_first_command() {
        let (s, now) = supervisor();
        assert!(s.must_stop(now));
    }

    #[test]
    fn a_steady_stream_never_stops() {
        let (mut s, start) = supervisor();
        // Nav2's controller runs at 20 Hz: a command every 50 ms, a tick every 20 ms.
        for tick in 0..500 {
            let now = start + TICK * tick;
            if tick % 3 == 0 {
                s.command(0.2, 0.0, now);
            }
            assert!(!s.must_stop(now), "stopped at tick {tick}");
        }
    }

    #[test]
    fn the_stop_is_out_within_the_window() {
        let (mut s, last) = supervisor();
        s.command(0.2, 0.0, last);
        // Ticks are not aligned with commands, so try every phase a tick could have.
        for phase in 0..20 {
            let first_stop = (0..)
                .map(|n| ms(phase) + TICK * n)
                .find(|&age| s.must_stop(last + age))
                .unwrap();
            assert!(first_stop <= ms(240), "phase {phase}: {first_stop:?}");
            assert!(first_stop > ms(200), "phase {phase}: {first_stop:?}");
        }
    }

    #[test]
    fn one_lost_teleop_frame_is_not_a_stop_and_two_are() {
        let (mut s, last) = supervisor();
        s.command(0.2, 0.0, last);
        // Frames every 100 ms: one lost is a 200 ms gap, and the next may still be 20 ms late.
        for age in (0..=220).step_by(TICK.as_millis() as usize) {
            assert!(
                !s.must_stop(last + ms(age)),
                "stopped {age} ms after a frame"
            );
        }
        assert!(s.must_stop(last + ms(300)));
    }

    #[test]
    fn a_new_command_ends_the_silence() {
        let (mut s, start) = supervisor();
        s.command(0.2, 0.0, start);
        assert!(s.must_stop(start + ms(500)));
        s.command(0.2, 0.0, start + ms(500));
        assert!(!s.must_stop(start + ms(520)));
    }

    #[test]
    #[should_panic(expected = "max_linear")]
    fn refuses_a_limit_that_is_not_a_number() {
        Supervisor::new(Config {
            max_linear: f64::NAN,
            ..Config::default()
        });
    }

    #[test]
    #[should_panic(expected = "window")]
    fn refuses_a_robot_too_fast_for_its_brakes() {
        Supervisor::new(Config {
            brake: ms(200),
            ..Config::default()
        });
    }

    // ---- tilt ------------------------------------------------------------------------------

    #[test]
    fn tilt_is_read_off_the_orientation() {
        let half = std::f64::consts::FRAC_PI_4;
        let close = |a: Option<f64>, b: f64| (a.unwrap() - b).abs() < 1e-9;
        assert!(close(tilt_from_quaternion(0.0, 0.0, 0.0, 1.0), 0.0));
        // Turning on the spot is not tilting.
        assert!(close(
            tilt_from_quaternion(0.0, 0.0, half.sin(), half.cos()),
            0.0
        ));
        // On its side, and on its nose.
        assert!(close(
            tilt_from_quaternion(half.sin(), 0.0, 0.0, half.cos()),
            std::f64::consts::FRAC_PI_2
        ));
        assert!(close(
            tilt_from_quaternion(0.0, half.sin(), 0.0, half.cos()),
            std::f64::consts::FRAC_PI_2
        ));
        // Upside down, and a quaternion that was never normalised.
        assert!(close(
            tilt_from_quaternion(1.0, 0.0, 0.0, 0.0),
            std::f64::consts::PI
        ));
        assert!(close(tilt_from_quaternion(0.0, 0.0, 0.0, 7.0), 0.0));
        assert_eq!(tilt_from_quaternion(0.0, 0.0, 0.0, 0.0), None);
        assert_eq!(tilt_from_quaternion(f64::NAN, 0.0, 0.0, 1.0), None);
    }

    #[test]
    fn nothing_moves_until_the_imu_has_reported_for_a_while() {
        let mut s = Supervisor::new(Config::default());
        let start = Instant::now();
        assert_eq!(s.fault(start), Some(Fault::ImuNoData));
        assert_eq!(s.command(0.2, 0.0, start), (0.0, 0.0));
        // One reading is not a sensor that works.
        s.imu(UPRIGHT, start);
        assert_eq!(s.fault(start), Some(Fault::ImuNoData));
        let now = feed(&mut s, UPRIGHT, start, ms(100));
        assert_eq!(s.fault(now), None);
        assert_eq!(s.command(0.2, 0.0, now), (0.2, 0.0));
        assert!(!s.must_stop(now));
    }

    #[test]
    fn a_silent_imu_stops_the_robot_and_a_returning_one_frees_it() {
        let mut s = Supervisor::new(Config::default());
        let start = Instant::now();
        let last = feed(&mut s, UPRIGHT, start, ms(200));
        s.command(0.2, 0.0, last + ms(500));
        assert_eq!(s.fault(last + ms(500)), None);
        // Commands keep coming; the IMU does not.
        s.command(0.2, 0.0, last + ms(520));
        assert_eq!(s.fault(last + ms(520)), Some(Fault::ImuNoData));
        assert!(s.must_stop(last + ms(520)));
        assert_eq!(s.command(0.2, 0.0, last + ms(520)), (0.0, 0.0));
        // Back, but not believed on the first reading.
        let back = last + ms(1000);
        s.imu(UPRIGHT, back);
        assert_eq!(s.fault(back), Some(Fault::ImuNoData));
        let now = feed(&mut s, UPRIGHT, back, ms(100));
        assert_eq!(s.fault(now), None);
    }

    #[test]
    fn a_reading_that_is_garbage_is_silence() {
        let mut s = Supervisor::new(Config::default());
        let start = Instant::now();
        let now = feed(&mut s, None, start, ms(300));
        assert_eq!(s.fault(now), Some(Fault::ImuNoData));
        let now = feed(&mut s, Some(f64::NAN), now, ms(300));
        assert_eq!(s.fault(now), Some(Fault::ImuNoData));
    }

    #[test]
    fn a_bump_is_not_a_fall() {
        let (mut s, start) = supervisor();
        // Tilted for 80 ms, then level again: under `hold`, so nothing latches.
        let now = feed(&mut s, ON_ITS_SIDE, start, ms(80));
        assert!(!s.imu(UPRIGHT, now + TICK));
        // And the next tilt starts counting from zero.
        assert!(!s.imu(ON_ITS_SIDE, now + TICK * 2));
        assert_eq!(s.fault(now + TICK * 2), None);
    }

    #[test]
    fn a_fall_latches_once_and_stays_latched_when_upright_again() {
        let (mut s, start) = supervisor();
        s.command(0.2, 0.0, start);
        let mut trips = 0;
        let mut now = start;
        while now <= start + ms(300) {
            trips += s.imu(ON_ITS_SIDE, now) as u32;
            now += TICK;
        }
        assert_eq!(trips, 1, "the latch trips on exactly one reading");
        assert_eq!(s.fault(now), Some(Fault::TipOver));
        // Set upright again, with commands still arriving: still locked.
        let now = feed(&mut s, UPRIGHT, now, Duration::from_secs(5));
        assert_eq!(s.fault(now), Some(Fault::TipOver));
        assert_eq!(s.command(0.2, 0.0, now), (0.0, 0.0));
        assert!(s.must_stop(now));
    }

    #[test]
    fn the_latch_is_within_hold_and_one_reading_of_the_tilt() {
        let (mut s, start) = supervisor();
        let tripped_after = (0..)
            .map(|n| TICK * n)
            .find(|&age| s.imu(ON_ITS_SIDE, start + age))
            .unwrap();
        assert_eq!(tripped_after, ms(100));
    }

    #[test]
    fn clearing_needs_a_robot_that_has_been_upright_for_a_second() {
        let (mut s, start) = supervisor();
        assert_eq!(s.clear(start), Ok(()), "nothing latched, nothing to refuse");
        let now = feed(&mut s, ON_ITS_SIDE, start, ms(300));
        assert_eq!(s.clear(now), Err("still tipped"));
        let upright = now + TICK;
        let now = feed(&mut s, UPRIGHT, upright, ms(500));
        assert_eq!(s.clear(now), Err("upright, but not for long enough yet"));
        let now = feed(&mut s, UPRIGHT, now + TICK, ms(500));
        assert_eq!(s.clear(now), Ok(()));
        assert_eq!(s.fault(now), None);
        assert_eq!(s.command(0.2, 0.0, now), (0.2, 0.0));
    }

    #[test]
    fn clearing_is_refused_when_the_imu_has_gone_quiet() {
        let mut s = Supervisor::new(Config::default());
        let start = Instant::now();
        let now = feed(&mut s, ON_ITS_SIDE, start, ms(300));
        let now = feed(&mut s, UPRIGHT, now + TICK, Duration::from_secs(2));
        let later = now + Duration::from_secs(2);
        assert_eq!(
            s.clear(later),
            Err("no IMU data, so the robot cannot be seen to be upright")
        );
        // A tip-over outranks the silence that followed it.
        assert_eq!(s.fault(later), Some(Fault::TipOver));
    }

    // ---- reporting -------------------------------------------------------------------------

    #[test]
    fn health_is_reported_on_a_change_and_once_a_second_otherwise() {
        let (mut s, start) = supervisor();
        assert_eq!(s.report(start).unwrap().fault, None);
        assert_eq!(s.report(start + ms(500)), None);
        assert_eq!(s.report(start + ms(1000)).unwrap().fault, None);
        // A fall is reported at once, not at the next second.
        let now = feed(&mut s, ON_ITS_SIDE, start + ms(1020), ms(200));
        let health = s.report(now).unwrap();
        assert_eq!(health.fault.map(Fault::code), Some("TIP_OVER"));
        assert_eq!(health.tilt, 1.5);
        assert_eq!(s.report(now + TICK), None);
    }
}
