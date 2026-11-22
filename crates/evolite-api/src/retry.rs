//! One rule for a read that may fail (TEAM.md §8d-4, §8d-5; W7 r1, BUG-271 r1, BUG-266): it is
//! «waiting» until its answer comes; a failure is said — with «Retry» — and asked again by itself
//! only after a pause that grows with each failure in a row, never on every frame or turn of
//! the core. Times are seconds on the caller's clock (the frame's `ui.input(|i| i.time)`, or
//! the core's time since its start).

/// The pause after the first failure (W7, BUG-271): the second waits twice as long, and so on.
pub const FIRST: f64 = 30.0;

/// The longest pause between tries by themselves.
pub const MOST: f64 = 600.0;

/// Where a read stands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Retry {
    /// Asked and not answered yet.
    pub asking: bool,
    /// The failures in a row since the last answer that came.
    pub failures: u32,
    /// When the next try goes by itself, after a failure.
    pub next_at: f64,
    /// How many times it was asked (for the tests: never on every frame).
    pub asked: u32,
    /// The first pause (seconds) after a failure: [`FIRST`] for a read that failed, shorter
    /// for an answer that only says «not yet» (BUG-266: the sync, 3 s).
    pub first: f64,
    /// The longest pause; 0: [`MOST`]. «Not yet» is no failure: its pause stays short (W7 r2:
    /// BUG-266's «not synced» at most 30 s, else it outlived the sync by minutes).
    pub most: f64,
}

impl Retry {
    /// A read whose first pause is `first` seconds.
    pub fn with_first(first: f64) -> Self {
        Self {
            first,
            ..Self::default()
        }
    }

    /// The same, its pause never longer than `most` seconds.
    pub fn capped(mut self, most: f64) -> Self {
        self.most = most;
        self
    }

    fn pause(&self) -> f64 {
        let first = if self.first > 0.0 { self.first } else { FIRST };
        let most = if self.most > 0.0 { self.most } else { MOST };
        let grown = first * 2f64.powi(self.failures.saturating_sub(1).min(16) as i32);
        grown.min(most.max(first))
    }

    /// Whether to ask now: never asked; or failed and its pause is over; never while asking.
    /// Asks it when so (the caller sends the read).
    pub fn ask(&mut self, now: f64) -> bool {
        let due = !self.asking && (self.asked == 0 || (self.failures > 0 && now >= self.next_at));
        if due {
            self.asking = true;
            self.asked += 1;
        }
        due
    }

    /// «Retry»: the next [`Self::ask`] goes at once.
    pub fn again(&mut self, now: f64) {
        if self.failures > 0 {
            self.next_at = now;
        }
    }

    /// The answer came.
    pub fn answered(&mut self) {
        self.asking = false;
        self.failures = 0;
    }

    /// The read failed at `now`: said, and tried again by itself after the pause.
    pub fn failed(&mut self, now: f64) {
        self.asking = false;
        self.failures += 1;
        self.next_at = now + self.pause();
    }

    /// The last try failed and the next one waits.
    pub fn is_failed(&self) -> bool {
        self.failures > 0 && !self.asking
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A read that keeps failing is asked once per pause, the pause growing 30, 60, 120 s… up to
    /// 10 minutes; «Retry» asks at once; an answer starts the count again.
    #[test]
    fn a_failing_read_is_asked_once_a_pause() {
        let mut r = Retry::default();
        assert!(r.ask(0.0), "the first time");
        assert!(!r.ask(0.1), "not while asking");
        r.failed(1.0);
        assert!(r.is_failed());
        let asks = (0..1_000)
            .filter(|i| r.ask(1.0 + *i as f64 * 0.016))
            .count();
        assert_eq!(asks, 0, "a thousand frames within the pause: no ask");
        assert!(r.ask(31.0), "after 30 s");
        r.failed(31.0);
        assert!(!r.ask(80.0));
        assert!(r.ask(91.0), "then after 60 s");
        r.failed(91.0);
        r.again(92.0);
        assert!(r.ask(92.0), "«Retry» at once");
        r.answered();
        assert!(!r.is_failed());
        assert!(!r.ask(10_000.0), "answered: not asked again by itself");
        let mut long = Retry::default();
        long.ask(0.0);
        for _ in 0..20 {
            long.failed(0.0);
        }
        assert_eq!(long.next_at, MOST, "at most 10 minutes");
        let mut sync = Retry::with_first(3.0).capped(30.0);
        sync.ask(0.0);
        sync.failed(0.0);
        assert!(sync.ask(3.0), "«not synced» again after 3 s");
        for _ in 0..10 {
            sync.failed(100.0);
        }
        assert_eq!(sync.next_at, 130.0, "«not yet» waits at most 30 s");
    }
}
