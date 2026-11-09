//! The slow-step probe: which step of the core took longer than [`SLOW`], or has not come back.
//!
//! A manual pass saw name lookups wait minutes and answer in the same second (BUG-87, BUG-88). The
//! journal could not say whether the core's loop stood or the tasks off it did. Here every step
//! of the loop, of the snapshot and every task answering later is timed by name, and a thread
//! outside the runtime, which a starved runtime cannot hold up, notes the ones still running.
//!
//! What goes into the journal is a name fixed in the code and whole milliseconds: `&'static str`
//! and `u64` have no room for anything of the person's. A command is named by
//! [`crate::diag::command_name`], never by its `Debug`.
//!
//! The table of running steps is locked only to add or take out an entry, or by the watchdog to
//! look over it; never across an await and with no other lock inside, so it cannot park anyone.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::diag::{self, Record};

/// A step shorter than this says nothing.
const SLOW: Duration = Duration::from_secs(5);
/// How often the watchdog looks.
const WATCH_EVERY: Duration = Duration::from_secs(1);
/// At most one record of a kind for one step in this time.
const PER_STEP: Duration = Duration::from_secs(60);
/// At most this many records in one [`WINDOW`]. A window, not a total for the run: what happened
/// early must not leave a later stall, the one being looked for, without a line.
const PER_WINDOW: u32 = 100;
const WINDOW: Duration = Duration::from_secs(10 * 60);
/// The one line that says the window is spent, so that silence is not read as «no slow step».
/// Its `ms` is how long at most the journal may stay silent about the steps.
const BUDGET_SPENT: &str = "budget_spent";

struct Limits {
    slow: Duration,
    per_step: Duration,
    per_window: u32,
    window: Duration,
}

const LIMITS: Limits = Limits {
    slow: SLOW,
    per_step: PER_STEP,
    per_window: PER_WINDOW,
    window: WINDOW,
};

/// Which of the two records: they are capped apart, so that the total of a step that was noted
/// as still running at 5 s is not lost when it ends a few seconds later.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum Kind {
    Ended,
    StillRunning,
}

/// A step under way.
struct Entry {
    step: &'static str,
    started: Instant,
    /// How long it has to run before the watchdog notes it (again).
    note_at: Duration,
}

struct State {
    next_id: u64,
    running: HashMap<u64, Entry>,
    written: HashMap<(Kind, &'static str), Instant>,
    /// When the records of the window were written, oldest first.
    recent: VecDeque<Instant>,
    /// The window is spent and [`BUDGET_SPENT`] has been written for it.
    spent: bool,
}

struct Probe {
    limits: Limits,
    sink: Box<dyn Fn(Record) + Send + Sync>,
    state: Mutex<State>,
}

/// Over whole milliseconds; an impossible length saturates.
fn millis(length: Duration) -> u64 {
    u64::try_from(length.as_millis()).unwrap_or(u64::MAX)
}

impl Probe {
    fn new(limits: Limits, sink: Box<dyn Fn(Record) + Send + Sync>) -> Self {
        Self {
            limits,
            sink,
            state: Mutex::new(State {
                next_id: 0,
                running: HashMap::new(),
                written: HashMap::new(),
                recent: VecDeque::new(),
                spent: false,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn open(&self, step: &'static str, now: Instant) -> u64 {
        let mut state = self.state();
        let id = state.next_id;
        state.next_id += 1;
        let entry = Entry {
            step,
            started: now,
            note_at: self.limits.slow,
        };
        state.running.insert(id, entry);
        id
    }

    /// May a record of this kind for this step be written now? Counts it if so. The first
    /// refusal of a spent window puts [`BUDGET_SPENT`] in `out`.
    fn admit(
        &self,
        state: &mut State,
        kind: Kind,
        step: &'static str,
        now: Instant,
        out: &mut Vec<Record>,
    ) -> bool {
        let key = (kind, step);
        if state
            .written
            .get(&key)
            .is_some_and(|at| now.saturating_duration_since(*at) < self.limits.per_step)
        {
            return false;
        }
        let window = self.limits.window;
        while state
            .recent
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= window)
        {
            state.recent.pop_front();
        }
        if state.recent.len() >= self.limits.per_window as usize {
            if !state.spent {
                state.spent = true;
                let oldest = state.recent.front().copied().unwrap_or(now);
                out.push(Record::SlowStep {
                    step: BUDGET_SPENT,
                    ms: millis(window.saturating_sub(now.saturating_duration_since(oldest))),
                });
            }
            return false;
        }
        state.spent = false;
        state.written.insert(key, now);
        state.recent.push_back(now);
        true
    }

    /// The step has ended (or its future was dropped).
    fn close(&self, id: u64, now: Instant) {
        let mut records = Vec::new();
        {
            let mut state = self.state();
            let Some(entry) = state.running.remove(&id) else {
                return;
            };
            let took = now.saturating_duration_since(entry.started);
            if took >= self.limits.slow
                && self.admit(&mut state, Kind::Ended, entry.step, now, &mut records)
            {
                records.push(Record::SlowStep {
                    step: entry.step,
                    ms: millis(took),
                });
            }
        }
        for record in records {
            (self.sink)(record);
        }
    }

    /// One look of the watchdog: notes the steps past the limit that have not come back. One
    /// that stays is noted again at doubling lengths, so a stall of hours is a few lines. A note
    /// that could not be written is tried again at the next look, not at the next doubling.
    fn sweep(&self, now: Instant) {
        let mut records = Vec::new();
        {
            let mut guard = self.state();
            let state = &mut *guard;
            let mut due = Vec::new();
            for (id, entry) in &state.running {
                let so_far = now.saturating_duration_since(entry.started);
                if so_far >= entry.note_at {
                    due.push((*id, entry.step, so_far));
                }
            }
            for (id, step, so_far) in due {
                if self.admit(state, Kind::StillRunning, step, now, &mut records) {
                    records.push(Record::StuckStep {
                        step,
                        ms: millis(so_far),
                    });
                    if let Some(entry) = state.running.get_mut(&id) {
                        entry.note_at = (so_far * 2).max(so_far + self.limits.per_step);
                    }
                }
            }
        }
        for record in records {
            (self.sink)(record);
        }
    }

    /// Starts the watchdog. If the thread cannot start, steps that end are still recorded.
    fn watch(&'static self, every: Duration) {
        let _ = std::thread::Builder::new()
            .name("evolite-slow-steps".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(every);
                    self.sweep(Instant::now());
                }
            });
    }

    fn begin(&'static self, step: &'static str) -> Running {
        Running {
            probe: self,
            id: self.open(step, Instant::now()),
        }
    }

    async fn timed<F: Future>(&'static self, step: &'static str, work: F) -> F::Output {
        let _running = self.begin(step);
        work.await
    }

    /// Timed from now, not from its first poll: the wait for a free worker is part of it.
    fn task<F: Future>(
        &'static self,
        step: &'static str,
        work: F,
    ) -> impl Future<Output = F::Output> {
        let running = self.begin(step);
        async move {
            let _running = running;
            work.await
        }
    }
}

/// While this lives its step is running; dropped, the step has ended.
#[must_use]
pub struct Running {
    probe: &'static Probe,
    id: u64,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.probe.close(self.id, Instant::now());
    }
}

static PROBE: LazyLock<&'static Probe> = LazyLock::new(|| {
    let probe: &'static Probe = Box::leak(Box::new(Probe::new(LIMITS, Box::new(diag::record))));
    probe.watch(WATCH_EVERY);
    probe
});

/// Times the section until the result is dropped.
pub fn begin(step: &'static str) -> Running {
    PROBE.begin(step)
}

/// Awaits `work`, timed as `step`.
pub async fn timed<F: Future>(step: &'static str, work: F) -> F::Output {
    PROBE.timed(step, work).await
}

/// Runs `work`, which has no await in it, timed as `step`.
pub fn during<T>(step: &'static str, work: impl FnOnce() -> T) -> T {
    let _running = begin(step);
    work()
}

/// `work`, to be spawned, timed as `step` from this moment on.
pub fn task<F: Future>(step: &'static str, work: F) -> impl Future<Output = F::Output> {
    PROBE.task(step, work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    type Written = Arc<Mutex<Vec<Record>>>;

    /// A probe of its own, with the real limits unless `limits` says otherwise.
    fn probe_with(limits: Limits) -> (&'static Probe, Written) {
        let written: Written = Arc::default();
        let into = written.clone();
        let sink = Box::new(move |r: Record| into.lock().unwrap().push(r));
        (Box::leak(Box::new(Probe::new(limits, sink))), written)
    }

    fn probe() -> (&'static Probe, Written) {
        probe_with(LIMITS)
    }

    fn taken(written: &Written) -> Vec<Record> {
        written.lock().unwrap().clone()
    }

    const SEC: Duration = Duration::from_secs(1);

    #[test]
    fn a_step_over_the_limit_is_recorded_with_its_length() {
        let (probe, written) = probe();
        let t = Instant::now();
        let id = probe.open("core_peers", t);
        probe.close(id, t + 6 * SEC);
        assert_eq!(
            taken(&written),
            [Record::SlowStep {
                step: "core_peers",
                ms: 6000
            }]
        );
    }

    #[test]
    fn a_step_under_the_limit_says_nothing() {
        let (probe, written) = probe();
        let t = Instant::now();
        let id = probe.open("core_peers", t);
        probe.close(id, t + SLOW - Duration::from_millis(1));
        probe.sweep(t + 100 * SEC);
        assert_eq!(taken(&written), []);
    }

    #[test]
    fn a_step_that_never_ends_is_noted_by_a_look_at_the_table() {
        let (probe, written) = probe();
        let t = Instant::now();
        probe.open("snapshot", t);
        probe.sweep(t + 4 * SEC);
        assert_eq!(taken(&written), [], "under the limit");
        probe.sweep(t + 5 * SEC);
        assert_eq!(
            taken(&written),
            [Record::StuckStep {
                step: "snapshot",
                ms: 5000
            }]
        );
    }

    #[test]
    fn a_step_that_ended_is_not_noted_as_running() {
        let (probe, written) = probe();
        let t = Instant::now();
        let id = probe.open("snapshot", t);
        probe.close(id, t + SEC);
        probe.sweep(t + 100 * SEC);
        assert_eq!(taken(&written), []);
    }

    /// Waits on the real clock for what a thread has to do: a slow machine only takes longer.
    fn wait_for(written: &Written, what: impl Fn(&Record) -> bool) -> bool {
        for _ in 0..1000 {
            if written.lock().unwrap().iter().any(&what) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    const QUICK: Limits = Limits {
        slow: Duration::from_millis(20),
        per_step: PER_STEP,
        per_window: PER_WINDOW,
        window: WINDOW,
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn the_watchdog_thread_records_a_step_that_never_returns() {
        let (probe, written) = probe_with(QUICK);
        probe.watch(Duration::from_millis(5));
        let hung = tokio::spawn(probe.timed("hung", std::future::pending::<()>()));
        assert!(
            wait_for(&written, |r| matches!(
                r,
                Record::StuckStep { step: "hung", ms } if *ms >= 20
            )),
            "{:?}",
            taken(&written)
        );
        hung.abort();
    }

    #[tokio::test]
    async fn timed_records_its_step_and_gives_back_the_answer() {
        let (probe, written) = probe_with(QUICK);
        let answer = probe
            .timed("handle", async {
                tokio::time::sleep(Duration::from_millis(40)).await;
                7
            })
            .await;
        assert_eq!(answer, 7);
        assert!(matches!(
            taken(&written)[..],
            [Record::SlowStep { step: "handle", ms }] if ms >= 20
        ));
    }

    #[test]
    fn a_task_counts_from_its_making_not_from_its_first_poll() {
        let (probe, written) = probe();
        // Spawned on a runtime with no free worker: never polled yet.
        let waiting = probe.task("task:resolve_name", async {});
        probe.sweep(Instant::now() + 6 * SEC);
        assert!(matches!(
            taken(&written)[..],
            [Record::StuckStep {
                step: "task:resolve_name",
                ..
            }]
        ));
        drop(waiting);
    }

    #[test]
    fn one_step_is_written_once_a_minute() {
        let (probe, written) = probe();
        let t = Instant::now();
        for i in 0..1000 {
            let start = t + Duration::from_millis(i);
            let id = probe.open("wallet_state", start);
            probe.close(id, start + 6 * SEC);
        }
        assert_eq!(taken(&written).len(), 1, "within one minute");
        let id = probe.open("wallet_state", t + 100 * SEC);
        probe.close(id, t + 106 * SEC);
        assert_eq!(taken(&written).len(), 2, "a minute later");
    }

    #[test]
    fn a_stall_that_goes_on_does_not_flood_the_journal() {
        let (probe, written) = probe();
        let t = Instant::now();
        probe.open("quorum_key", t);
        let day = 24 * 60 * 60;
        for second in 0..day {
            probe.sweep(t + second * SEC);
        }
        let notes = taken(&written);
        assert!(
            (2..=16).contains(&notes.len()),
            "a day of one stall: {notes:?}"
        );
        // The length so far keeps growing.
        assert!(matches!(
            notes[1],
            Record::StuckStep { ms, .. } if ms >= 65_000
        ));
    }

    #[test]
    fn the_total_of_a_noted_step_is_not_lost_to_the_cap() {
        let (probe, written) = probe();
        let t = Instant::now();
        let id = probe.open("task:add_contact", t);
        probe.sweep(t + 5 * SEC);
        probe.close(id, t + 9 * SEC);
        assert_eq!(
            taken(&written),
            [
                Record::StuckStep {
                    step: "task:add_contact",
                    ms: 5000
                },
                Record::SlowStep {
                    step: "task:add_contact",
                    ms: 9000
                }
            ]
        );
    }

    const MINUTE: Duration = Duration::from_secs(60);

    /// Spends the whole window at `at`: different steps, each past the limit.
    fn spend_the_window(probe: &'static Probe, at: Instant, tag: &str) {
        for i in 0..PER_WINDOW {
            let step: &'static str = Box::leak(format!("{tag}{i}").into_boxed_str());
            let id = probe.open(step, at);
            probe.close(id, at + 6 * SEC);
        }
    }

    fn marks(written: &Written) -> Vec<u64> {
        taken(written)
            .iter()
            .filter_map(|r| match r {
                Record::SlowStep {
                    step: BUDGET_SPENT,
                    ms,
                } => Some(*ms),
                _ => None,
            })
            .collect()
    }

    fn stuck(written: &Written, name: &str) -> Vec<u64> {
        taken(written)
            .iter()
            .filter_map(|r| match r {
                Record::StuckStep { step, ms } if *step == name => Some(*ms),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_window_writes_at_most_its_budget_and_one_mark() {
        let (probe, written) = probe();
        let t = Instant::now();
        // More different steps than the budget, all slow, each also noted as running.
        for i in 0..2 * PER_WINDOW {
            let step: &'static str = Box::leak(format!("step{i}").into_boxed_str());
            let id = probe.open(step, t);
            probe.sweep(t + 6 * SEC);
            probe.close(id, t + 7 * SEC);
        }
        assert_eq!(taken(&written).len(), PER_WINDOW as usize + 1);
        assert_eq!(marks(&written).len(), 1);
    }

    #[test]
    fn a_spent_window_is_marked_once_and_says_how_long_it_lasts() {
        let (probe, written) = probe();
        let t = Instant::now();
        spend_the_window(probe, t, "noise");
        assert_eq!(
            marks(&written),
            [0u64; 0],
            "just spent, nothing refused yet"
        );
        spend_the_window(probe, t, "more");
        assert_eq!(marks(&written), [WINDOW.as_millis() as u64], "one mark");
        // The window slides on: lines go through again, and it is marked when spent again.
        let later = t + WINDOW;
        spend_the_window(probe, later, "again");
        spend_the_window(probe, later, "again_more");
        assert_eq!(marks(&written).len(), 2);
    }

    #[test]
    fn noise_early_in_the_run_leaves_a_later_stall_its_lines() {
        let (probe, written) = probe();
        let t = Instant::now();
        spend_the_window(probe, t, "noise");
        let stalled = t + WINDOW + MINUTE;
        probe.open("quorum_key", stalled);
        probe.sweep(stalled + 5 * SEC);
        assert_eq!(stuck(&written, "quorum_key"), [5000]);
    }

    #[test]
    fn a_stall_in_a_spent_window_is_noted_as_soon_as_the_window_frees() {
        let (probe, written) = probe();
        let t = Instant::now();
        // All written at t + 6 s, so the window frees at t + 606 s.
        spend_the_window(probe, t, "noise");
        probe.open("task:resolve_name", t + MINUTE);
        for second in 60..=606 {
            probe.sweep(t + second * SEC);
        }
        assert_eq!(marks(&written).len(), 1, "the stall was seen and refused");
        assert_eq!(
            stuck(&written, "task:resolve_name"),
            [546_000],
            "written at the first look after the window, not at a later doubling"
        );
    }
}
