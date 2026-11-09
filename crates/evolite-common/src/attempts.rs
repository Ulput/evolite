//! Wrong passwords in a row and the pause after them (FEAT-0002): the scale, and the count of
//! each wallet in memory. Both cores keep it — the real one also in the wallet's folder
//! (`real/attempts.rs`), so a restart does not end a pause.
//!
//! The clocks are parameters: `now` is the wall clock in Unix seconds, `at` the process's
//! monotonic one. On the disk only the wall clock means anything (it outlives the process); a
//! pause read from it is never longer than its step, so a clock set back does not stretch it,
//! and one set forward ends it early (accepted, UC-8). The end cut down to one step is written
//! back (R-31 item 4): read again later it does not move on with the clock, so a pause only
//! ever gets shorter. Within one process the monotonic clock also ends a pause after its step
//! when the wall clock was set back, and keeps it when the wall clock was set forward (R-31).
//!
//! Not tied to `Unlock`: a password check of another kind (FEAT-0004, the payment password of
//! BUG-42's `spend_gate`) can count by the same scale in a [`Ledger`] of its own.

use std::collections::HashMap;
use std::time::Instant;

/// Wrong passwords in a row without a pause; the next one starts it.
pub const FREE_TRIES: u32 = 4;

/// The pauses after the 5th, 6th… wrong password, in seconds; the last is the ceiling.
const STEPS: [u64; 6] = [30, 60, 120, 300, 600, 900];

/// The longest pause: no wallet is ever locked for longer, nor deleted.
pub const CEILING: u64 = 900;

/// The pause after the `failures`-th wrong password in a row, in seconds; 0: none.
pub fn pause_after(failures: u32) -> u64 {
    match failures.checked_sub(FREE_TRIES + 1) {
        None => 0,
        Some(past) => STEPS[(past as usize).min(STEPS.len() - 1)],
    }
}

/// One wallet's count: wrong passwords in a row and when the pause after the last ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tries {
    pub failures: u32,
    /// Unix seconds; `None`: no pause was started.
    pub wait_until: Option<u64>,
}

impl Tries {
    /// The end of the pause at `now`, if one lasts: `wait_until`, but never further away than
    /// the step of `failures` — a clock set back, or a file from elsewhere, cannot make it longer.
    /// An end further away is cut down in `self` (R-31 item 4), so it stays where it was cut
    /// and does not move on with `now`; the caller writes the cut count down.
    pub fn until(&mut self, now: u64) -> Option<u64> {
        let step_end = now.saturating_add(pause_after(self.failures));
        let end = self.wait_until?.min(step_end);
        self.wait_until = Some(end);
        (end > now).then_some(end)
    }

    /// The count after one more wrong password at `now`.
    pub fn failed(self, now: u64) -> Self {
        let failures = self.failures.saturating_add(1);
        let pause = pause_after(failures);
        Self {
            failures,
            wait_until: (pause > 0).then(|| now.saturating_add(pause)),
        }
    }
}

/// How a counted password check ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The password was right: the count starts again.
    Right,
    /// The password was wrong: it stays counted.
    Wrong,
    /// Something else went wrong (no vault, another network, the store): not the password's
    /// fault, so the try is not counted.
    Other,
}

/// The counts of the wallets of this process, by wallet id.
#[derive(Default)]
pub struct Ledger {
    tries: HashMap<String, Tries>,
    /// When each wallet's last wrong password was taken, by the monotonic clock: the pause
    /// after it lasts its step within this process even if the wall clock jumps forward.
    last_wrong: HashMap<String, Instant>,
}

impl Ledger {
    pub fn get(&self, id: &str) -> Tries {
        self.tries.get(id).copied().unwrap_or_default()
    }

    /// The count as read from somewhere else (the wallet's file).
    pub fn set(&mut self, id: &str, tries: Tries) {
        self.tries.insert(id.to_owned(), tries);
    }

    /// When the pause of wallet `id` ends, if one lasts. By the wall clock (bounded by the step
    /// and cut down for good, see [`Tries::until`]); within this process the monotonic clock
    /// since the last wrong password here has its say too:
    /// - both see a pause: the earlier end. Normally that is the wall clock's own (the
    ///   monotonic end is never earlier then), so the end does not wobble by a second with how
    ///   the seconds of the two clocks fall (review r1, must 2);
    /// - only the monotonic clock: the wall clock was set forward, the pause lasts its step;
    /// - only the wall clock: it was set back and the step has passed; the pause is over for
    ///   good.
    ///
    /// The kept count changes only to an earlier end; the caller writes it down if it did.
    pub fn until(&mut self, id: &str, now: u64, at: Instant) -> Option<u64> {
        let mut tries = self.get(id);
        let by_wall = tries.until(now);
        let by_instant = self.last_wrong.get(id).map(|last| {
            let gone = at.saturating_duration_since(*last).as_secs();
            let left = pause_after(tries.failures).saturating_sub(gone);
            (left > 0).then(|| now.saturating_add(left))
        });
        let end = match (by_wall, by_instant) {
            (wall, None) => wall,
            (Some(wall), Some(instant)) => {
                tries.wait_until = instant.map(|i| i.min(wall));
                tries.wait_until
            }
            (None, Some(instant)) => instant,
        };
        if tries != self.get(id) {
            self.set(id, tries);
        }
        end
    }

    /// Before the password is checked: `Err(until)` while a pause lasts (the password is not
    /// to be checked at all); else the try is counted as wrong at once — a process killed
    /// during the check cannot take it back — and the count before it is returned for
    /// [`Self::end`].
    pub fn begin(&mut self, id: &str, now: u64, at: Instant) -> Result<Tries, u64> {
        if let Some(until) = self.until(id, now, at) {
            return Err(until);
        }
        let before = self.get(id);
        self.set(id, before.failed(now));
        Ok(before)
    }

    /// After the check: a right password clears the count, a wrong one keeps it (and starts
    /// its pause on the monotonic clock too), anything else puts `before` back.
    pub fn end(&mut self, id: &str, before: Tries, outcome: Outcome, at: Instant) {
        match outcome {
            Outcome::Right => self.forget(id),
            Outcome::Wrong => {
                self.last_wrong.insert(id.to_owned(), at);
            }
            Outcome::Other => self.set(id, before),
        }
    }

    /// The wallet is gone (or its password was right): nothing of its count is kept.
    pub fn forget(&mut self, id: &str) {
        self.tries.remove(id);
        self.last_wrong.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 24 * 60 * 60;

    #[test]
    fn four_wrong_passwords_are_free_then_the_pause_grows_to_its_ceiling() {
        let pauses: Vec<u64> = (1..=12).map(pause_after).collect();
        assert_eq!(
            pauses,
            [0, 0, 0, 0, 30, 60, 120, 300, 600, 900, 900, 900],
            "FR-2"
        );
        assert_eq!(pause_after(0), 0);
        assert_eq!(pause_after(u32::MAX), CEILING);
    }

    #[test]
    fn the_fifth_wrong_password_starts_a_pause_of_thirty_seconds() {
        let mut t = Tries::default();
        for _ in 0..4 {
            t = t.failed(NOW);
            assert_eq!(t.until(NOW), None);
        }
        t = t.failed(NOW);
        assert_eq!(t.failures, 5);
        assert_eq!(t.until(NOW), Some(NOW + 30));
        assert_eq!(t.until(NOW + 29), Some(NOW + 30));
        assert_eq!(t.until(NOW + 30), None, "over at its second");
    }

    #[test]
    fn a_clock_set_back_a_day_waits_no_longer_than_the_step() {
        let mut t = Tries {
            failures: 8,
            wait_until: Some(NOW + 300),
        };
        // Back a day: the stored end is a day and five minutes away, the step is five minutes.
        assert_eq!(t.until(NOW - DAY), Some(NOW - DAY + 300));
    }

    #[test]
    fn a_clock_set_back_before_the_last_wrong_password_waits_one_step() {
        let t = Tries::default().failed(NOW).failed(NOW).failed(NOW);
        let mut t = t.failed(NOW).failed(NOW).failed(NOW); // the 6th: one minute
        assert_eq!(t.until(NOW - 3600), Some(NOW - 3600 + 60));
        assert_eq!(t.failures, 6, "the count is kept");
    }

    #[test]
    fn a_clock_set_forward_a_day_ends_the_pause_but_keeps_the_count() {
        let mut t = Tries {
            failures: 9,
            wait_until: Some(NOW + 600),
        };
        assert_eq!(t.until(NOW + DAY), None);
        assert_eq!(t.failed(NOW + DAY).until(NOW + DAY), Some(NOW + DAY + 900));
    }

    #[test]
    fn a_pause_far_in_the_future_is_cut_to_the_ceiling() {
        let mut t = Tries {
            failures: u32::MAX,
            wait_until: Some(u64::MAX),
        };
        assert_eq!(t.until(NOW), Some(NOW + CEILING));
        // A pause with fewer than five wrong passwords is none.
        let mut t = Tries {
            failures: 2,
            wait_until: Some(u64::MAX),
        };
        assert_eq!(t.until(NOW), None);
        assert_eq!(t.until(u64::MAX), None, "no overflow");
    }

    #[test]
    fn a_pause_refuses_before_the_check_and_a_right_password_clears_the_count() {
        let at = Instant::now();
        let mut ledger = Ledger::default();
        for _ in 0..5 {
            let before = ledger.begin("a", NOW, at).expect("no pause yet");
            ledger.end("a", before, Outcome::Wrong, at);
        }
        let later = at + Duration::from_secs(10);
        assert_eq!(ledger.begin("a", NOW + 10, later), Err(NOW + 30));
        assert_eq!(ledger.get("a").failures, 5, "a refused try is not counted");
        let before = ledger.begin("a", NOW + 30, at + Duration::from_secs(30));
        let before = before.expect("the pause is over");
        assert_eq!(ledger.get("a").failures, 6, "counted before the check");
        ledger.end("a", before, Outcome::Right, at);
        assert_eq!(ledger.get("a"), Tries::default());
    }

    #[test]
    fn an_error_that_is_not_the_password_does_not_count() {
        let at = Instant::now();
        let mut ledger = Ledger::default();
        let before = ledger.begin("a", NOW, at).unwrap();
        ledger.end("a", before, Outcome::Wrong, at);
        let before = ledger.begin("a", NOW, at).unwrap();
        ledger.end("a", before, Outcome::Other, at);
        assert_eq!(ledger.get("a").failures, 1);
    }

    #[test]
    fn two_wallets_count_apart() {
        let at = Instant::now();
        let mut ledger = Ledger::default();
        for _ in 0..5 {
            let before = ledger.begin("a", NOW, at).unwrap();
            ledger.end("a", before, Outcome::Wrong, at);
        }
        assert!(ledger.until("a", NOW, at).is_some());
        assert_eq!(ledger.until("b", NOW, at), None);
        assert!(ledger.begin("b", NOW, at).is_ok());
    }

    #[test]
    fn within_the_process_a_clock_set_forward_does_not_end_the_pause() {
        let at = Instant::now();
        let mut ledger = Ledger::default();
        for _ in 0..5 {
            let before = ledger.begin("a", NOW, at).unwrap();
            ledger.end("a", before, Outcome::Wrong, at);
        }
        // A day later by the wall clock, ten seconds by the process's own.
        let later = at + Duration::from_secs(10);
        assert_eq!(ledger.until("a", NOW + DAY, later), Some(NOW + DAY + 20));
        assert_eq!(
            ledger.until("a", NOW + DAY, at + Duration::from_secs(30)),
            None
        );
    }

    /// Review r1, must 1 (the architect's test): the clock goes back a day in a 5-minute pause;
    /// once those 5 minutes (and a second) have passed by the new clock, the pause is over.
    #[test]
    fn arch_clock_back_pause_ends_after_one_step_of_waiting() {
        let mut t = Tries {
            failures: 8,
            wait_until: Some(NOW + 300),
        };
        let back = NOW - DAY;
        assert_eq!(t.until(back), Some(back + 300), "as the r1 test says");
        assert_eq!(t.until(back + 301), None, "one step waited: over");
    }

    /// The same through the Ledger with the monotonic clock of the process (the architect's).
    #[test]
    fn arch_clock_back_ledger_ends_after_one_step() {
        let at = Instant::now();
        let mut ledger = Ledger::default();
        ledger.set(
            "a",
            Tries {
                failures: 8,
                wait_until: Some(NOW + 300),
            },
        );
        ledger.last_wrong.insert("a".into(), at);
        let later = at + Duration::from_secs(301);
        assert_eq!(ledger.until("a", NOW - DAY + 301, later), None);
    }

    /// Must 1: the cut end is kept, so later reads count down to it instead of moving with the
    /// clock, and it is never moved further away again.
    #[test]
    fn a_pause_cut_by_a_clock_set_back_counts_down_and_never_grows() {
        let mut t = Tries {
            failures: 8,
            wait_until: Some(NOW + 300),
        };
        let back = NOW - DAY;
        assert_eq!(t.until(back), Some(back + 300));
        assert_eq!(t.wait_until, Some(back + 300), "written back");
        assert_eq!(t.until(back + 100), Some(back + 300), "counts down");
        // Back again by a minute: still no longer than one step from then.
        assert_eq!(t.until(back - 60), Some(back + 240));
        assert_eq!(t.until(back + 240), None);
        assert_eq!(t.failures, 8, "the count is kept");
    }

    /// Must 2: within the process, with the wall second turned over but less than a second
    /// gone by the monotonic clock, the end is still the wall clock's own (not a second later).
    #[test]
    fn the_end_of_a_pause_does_not_wobble_with_the_seconds_of_the_two_clocks() {
        let at = Instant::now();
        let mut ledger = Ledger::default();
        for _ in 0..5 {
            let before = ledger.begin("a", NOW, at).unwrap();
            ledger.end("a", before, Outcome::Wrong, at);
        }
        let soon = at + Duration::from_millis(500);
        assert_eq!(ledger.until("a", NOW + 1, soon), Some(NOW + 30));
        assert_eq!(ledger.get("a").wait_until, Some(NOW + 30), "unchanged");
        // A clock set back within the process: the monotonic clock ends it after its step.
        assert_eq!(
            ledger.until("a", NOW - DAY, at + Duration::from_secs(10)),
            Some(NOW - DAY + 20)
        );
        assert_eq!(
            ledger.until("a", NOW - DAY + 21, at + Duration::from_secs(31)),
            None
        );
        assert_eq!(ledger.get("a").failures, 5, "the count is kept");
    }
}
