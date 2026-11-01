//! When `--screenshot` takes its picture with `--open` (the real wallet): after the sync and
//! `EVOLITE_SHOT_SETTLE` seconds more, at the latest 300 s past that, and asked again if the
//! picture never comes back.

use std::time::{Duration, Instant};

/// How long a screenshot asked for may take to come back before it is asked again.
const ASK_AGAIN: Duration = Duration::from_secs(10);
/// The latest a picture is taken, past `settle`: a sync that never ends is worth one too.
const LATEST: Duration = Duration::from_secs(300);

#[derive(Default)]
pub struct ShotWait {
    /// When the wallet was first open and synced. Kept from then on: a new block takes the sync
    /// out of "done" for a moment every 2,5 minutes, and a longer settle never ended.
    ready_since: Option<Instant>,
    /// When the picture was asked for, while it has not come back.
    asked_at: Option<Instant>,
}

impl ShotWait {
    /// `settled`: the wallet is open and synced now.
    pub fn ready(
        &mut self,
        settled: bool,
        started: Instant,
        settle: Duration,
        now: Instant,
    ) -> bool {
        if settled && self.ready_since.is_none() {
            self.ready_since = Some(now);
        }
        self.ready_since
            .is_some_and(|at| now.duration_since(at) >= settle)
            || now.duration_since(started) >= LATEST + settle
    }

    /// Since when pictures may still be drawing: the sync's end, else the start.
    pub fn since(&self, started: Instant) -> Instant {
        self.ready_since.unwrap_or(started)
    }

    /// Whether to ask for the picture now: not asked yet, or asked long ago with no answer (the
    /// answer can come in a frame that went another way, and was lost).
    pub fn ask(&mut self, now: Instant) -> bool {
        let due = self
            .asked_at
            .is_none_or(|at| now.duration_since(at) >= ASK_AGAIN);
        if due {
            self.asked_at = Some(now);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_block_does_not_restart_the_settle() {
        let start = Instant::now();
        let mut w = ShotWait::default();
        let settle = Duration::from_secs(240);
        let at = |s| start + Duration::from_secs(s);
        assert!(!w.ready(true, start, settle, at(60)));
        // A block: the sync is not "done" for a moment.
        assert!(!w.ready(false, start, settle, at(200)));
        assert!(
            w.ready(true, start, settle, at(300)),
            "240 s after the first sync"
        );
    }

    #[test]
    fn at_the_latest_without_a_sync() {
        let start = Instant::now();
        let mut w = ShotWait::default();
        let settle = Duration::from_secs(15);
        assert!(!w.ready(false, start, settle, start + Duration::from_secs(314)));
        assert!(w.ready(false, start, settle, start + Duration::from_secs(315)));
    }

    #[test]
    fn a_lost_picture_is_asked_again() {
        let start = Instant::now();
        let mut w = ShotWait::default();
        assert!(w.ask(start));
        assert!(
            !w.ask(start + Duration::from_secs(5)),
            "still waiting for it"
        );
        assert!(w.ask(start + Duration::from_secs(10)), "never came: again");
    }
}
