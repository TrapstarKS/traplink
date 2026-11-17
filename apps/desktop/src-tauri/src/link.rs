use std::time::{Duration, Instant, SystemTime};

pub const SCOPED_FLOW: u32 = 4;
pub const PROTOCOL: u32 = SCOPED_FLOW;
pub const PROBING: u32 = 2;
pub const PROBE_INTERVAL: Duration = Duration::from_secs(1);
pub const TIE_BREAK: Duration = Duration::from_millis(1500);
pub const RESUME_GAP: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    pub base: Duration,
    pub cap: Duration,
    pub max_attempts: u32,
}
impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            cap: Duration::from_secs(15),
            max_attempts: 10,
        }
    }
}
impl Backoff {
    pub fn delay(&self, attempt: u32, jitter: f64) -> Duration {
        let exponent = attempt.saturating_sub(1).min(20);
        let raw = self
            .base
            .saturating_mul(1u32 << exponent)
            .min(self.cap)
            .as_secs_f64();
        let jitter = if jitter.is_finite() {
            jitter.clamp(0.0, 1.0)
        } else {
            0.5
        };
        Duration::from_secs_f64(raw * (0.8 + 0.4 * jitter))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reconnect {
    pub attempt: u32,
    pub next_at: Option<Instant>,
    pub in_flight: bool,
    pub gave_up: bool,
    pub last_error: Option<String>,
}
impl Reconnect {
    pub fn idle(&self) -> bool {
        self.next_at.is_none() && !self.in_flight && !self.gave_up
    }
    pub fn lost(&mut self, backoff: &Backoff, now: Instant, jitter: f64, extra: Duration) {
        if self.idle() {
            self.next_at = Some(now + backoff.delay(1, jitter) + extra);
        }
    }
    pub fn kick(&mut self, now: Instant, extra: Duration) {
        if !self.in_flight {
            self.attempt = 0;
            self.gave_up = false;
            self.next_at = Some(now + extra);
        }
    }
    pub fn take_due(&mut self, now: Instant) -> bool {
        if self.in_flight || self.gave_up || self.next_at.is_none_or(|at| now < at) {
            return false;
        }
        self.attempt += 1;
        self.in_flight = true;
        self.next_at = None;
        true
    }
    pub fn failed(&mut self, backoff: &Backoff, now: Instant, jitter: f64, error: String) {
        self.in_flight = false;
        self.last_error = Some(error);
        if self.attempt >= backoff.max_attempts {
            self.gave_up = true;
            self.next_at = None;
        } else {
            self.next_at = Some(now + backoff.delay(self.attempt + 1, jitter));
        }
    }
    pub fn connected(&mut self) {
        *self = Self::default();
    }
    pub fn retry_in(&self, now: Instant) -> Option<Duration> {
        self.next_at.map(|at| at.saturating_duration_since(now))
    }
}

pub fn resumed(
    previous: SystemTime,
    now: SystemTime,
    monotonic_elapsed: Duration,
    gap: Duration,
) -> bool {
    now.duration_since(previous)
        .is_ok_and(|wall| wall.saturating_sub(monotonic_elapsed) >= gap)
}

pub fn liveness_timeout(protocol: u32, input_active: bool) -> Duration {
    match (protocol >= PROBING, input_active) {
        (true, true) => Duration::from_secs(3),
        (true, false) => Duration::from_secs(20),
        (false, _) => Duration::from_secs(60),
    }
}

pub fn smooth_rtt(previous: Option<u32>, sample: Duration) -> u32 {
    let sample = sample.as_millis().min(60_000) as u32;
    match previous {
        None => sample.max(1),
        Some(previous) => ((previous * 3 + sample) / 4).max(1),
    }
}

pub fn jitter() -> f64 {
    use rand::Rng;
    rand::thread_rng().gen_range(0.0..1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_schedule_is_exponential_capped_and_jittered() {
        let backoff = Backoff::default();
        let mid: Vec<u64> = (1..=8)
            .map(|n| backoff.delay(n, 0.5).as_millis() as u64)
            .collect();
        assert_eq!(mid, [500, 1000, 2000, 4000, 8000, 15000, 15000, 15000]);
        assert_eq!(backoff.delay(1, 0.0), Duration::from_millis(400));
        assert_eq!(backoff.delay(1, 1.0), Duration::from_millis(600));
        assert_eq!(backoff.delay(40, 0.5), Duration::from_secs(15));
        assert_eq!(backoff.delay(0, 0.5), Duration::from_millis(500));
        assert_eq!(backoff.delay(2, f64::NAN), Duration::from_millis(1000));
        for n in 1..=12 {
            for j in [0.0, 0.3, 0.99] {
                let d = backoff.delay(n, j);
                assert!(d >= Duration::from_millis(400) && d <= Duration::from_secs(18));
            }
        }
    }

    #[test]
    fn reconnect_is_bounded_and_resets_on_success() {
        let backoff = Backoff {
            max_attempts: 3,
            ..Backoff::default()
        };
        let t0 = Instant::now();
        let mut link = Reconnect::default();
        link.lost(&backoff, t0, 0.5, Duration::ZERO);
        assert!(!link.take_due(t0), "first attempt waits for the backoff");
        let mut now = t0 + Duration::from_millis(500);
        link.lost(&backoff, now, 0.5, Duration::from_secs(9));
        assert!(link.take_due(now));
        assert!(!link.take_due(now), "only one attempt in flight");
        for attempt in 1..=3 {
            assert_eq!(link.attempt, attempt);
            link.failed(&backoff, now, 0.5, "x".into());
            if attempt < 3 {
                let wait = link.retry_in(now).unwrap();
                assert_eq!(wait, backoff.delay(attempt + 1, 0.5));
                now += wait;
                assert!(link.take_due(now));
            }
        }
        assert!(link.gave_up && link.next_at.is_none());
        assert!(!link.take_due(now + Duration::from_secs(600)));
        link.kick(now, Duration::ZERO);
        assert!(link.take_due(now) && link.attempt == 1);
        link.connected();
        assert_eq!(link, Reconnect::default());
    }

    #[test]
    fn tie_break_delays_the_first_attempt() {
        let backoff = Backoff::default();
        let now = Instant::now();
        let mut link = Reconnect::default();
        link.lost(&backoff, now, 0.0, TIE_BREAK);
        assert_eq!(
            link.retry_in(now),
            Some(Duration::from_millis(400) + TIE_BREAK)
        );
    }

    #[test]
    fn resume_detection_uses_wall_clock_gap() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let tick = Duration::from_millis(250);
        assert!(!resumed(t, t + tick, tick, RESUME_GAP));
        assert!(resumed(t, t + Duration::from_secs(60), tick, RESUME_GAP));
        assert!(!resumed(t, t - Duration::from_secs(60), tick, RESUME_GAP));
        assert!(!resumed(
            t,
            t + Duration::from_secs(7),
            Duration::from_secs(7),
            RESUME_GAP
        ));
    }

    #[test]
    fn liveness_and_rtt() {
        assert_eq!(liveness_timeout(2, true), Duration::from_secs(3));
        assert_eq!(liveness_timeout(PROTOCOL, true), Duration::from_secs(3));
        assert_eq!(liveness_timeout(2, false), Duration::from_secs(20));
        assert_eq!(liveness_timeout(0, true), Duration::from_secs(60));
        assert_eq!(smooth_rtt(None, Duration::from_millis(8)), 8);
        assert_eq!(smooth_rtt(Some(8), Duration::from_millis(40)), 16);
        assert_eq!(smooth_rtt(None, Duration::ZERO), 1);
        assert_eq!(smooth_rtt(None, Duration::from_secs(600)), 60_000);
    }
}
