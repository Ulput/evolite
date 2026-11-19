//! One pass of the token search (FEAT-0019 UC-1а, R-11): the [`crawl`] state asks, the network
//! answers, the [`pace`] keeps it to one request a second. The network, the clock and the sleep
//! are the caller's, so the pass is tested without either. The SDK picks the node of each request
//! itself, so the pool is paced as one node (HANDOFF part 3); a node that asks to slow down
//! pauses the pass, never bans the node (BUG-78).

use std::future::Future;
use std::time::Duration;

use super::crawl::{Ask, Cache, Id, Limits, Pass, Reply};
use super::pace::{Pacer, Turn};

/// How a step failed (see `chain::CrawlFailure`, which the real core maps to this).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Wait this long, then go on.
    Slow(Duration),
    /// No complete answer in time: the same place is asked again with a smaller page; only at
    /// the smallest page is it a failure (architect's decision on batch 39).
    TimedOut(String),
    /// The network did not answer, or the proof did not check.
    Failed(String),
}

/// How a pass ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// Nothing left to ask: a whole pass.
    Done,
    /// `live` said stop (a lock, another wallet, the app closing); the cache says where.
    Stopped,
    /// Failed [`FAILURES`] times in a row; the cache says where, and the next pass goes on.
    Failed(String),
}

/// Failures in a row that end a pass.
pub const FAILURES: u32 = 3;

/// The cache is written after this many answers, and when the pass ends.
pub const SAVE_EVERY: u32 = 10;

/// The sizes a request with contracts' bodies (a first pass's page, a batch of contracts) takes,
/// smallest first: a timeout goes one down, [`GROW_AFTER`] answers in a row one up.
const SIZES: [u32; 5] = [5, 10, 25, 50, 100];

/// Where each run starts in [`SIZES`]: 25. The size lives in memory only.
const START: usize = 2;

/// Answers in a row at one size before the next larger.
const GROW_AFTER: u32 = 3;

/// The size of the requests that carry contracts' bodies, for this run.
struct Sizer {
    at: usize,
    streak: u32,
}

impl Sizer {
    fn new() -> Self {
        Sizer {
            at: START,
            streak: 0,
        }
    }

    /// Whether `pass` asks for bodies: those requests are sized.
    fn sizes(pass: &Pass) -> bool {
        matches!(pass, Pass::Pages { .. } | Pass::Contracts)
    }

    /// The limits for the request `pass` makes now: `base` with its sized part no larger than
    /// the size now.
    fn limits(&self, pass: &Pass, base: Limits) -> Limits {
        let size = SIZES[self.at];
        match pass {
            Pass::Pages { .. } => Limits {
                ids_page: base.ids_page.min(size),
                ..base
            },
            Pass::Contracts => Limits {
                batch: base.batch.min(size as usize),
                ..base
            },
            _ => base,
        }
    }

    /// An answer to a sized request: the new size when it grew.
    fn answered(&mut self) -> Option<u32> {
        self.streak += 1;
        if self.streak < GROW_AFTER || self.at + 1 == SIZES.len() {
            return None;
        }
        self.streak = 0;
        self.at += 1;
        Some(SIZES[self.at])
    }

    /// A timeout of a request for `asked` items: the new size, or `None` when it asked for the
    /// smallest size or less already (then the timeout is a failure).
    fn timed_out(&mut self, asked: u32) -> Option<u32> {
        self.streak = 0;
        if asked <= SIZES[0] {
            return None;
        }
        // One below what was asked: a size above the base limit does not count.
        self.at = SIZES.iter().rposition(|s| *s < asked).unwrap_or(0);
        Some(SIZES[self.at])
    }
}

/// How many items `ask` asks for.
fn asked_items(ask: &Ask) -> u32 {
    match ask {
        Ask::Pages { limit, .. } | Ask::Ids { limit, .. } => *limit,
        Ask::Versions(ids) | Ask::Contracts(ids) => ids.len() as u32,
    }
}

/// The stage of a pass (F): what its requests read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// A first pass's pages of contracts with their bodies.
    Pages,
    /// The ids of the contracts.
    Ids,
    /// The versions of the contracts read.
    Versions,
    /// The contracts new or grown since.
    Contracts,
}

impl Stage {
    fn of(pass: &Pass) -> Option<Stage> {
        match pass {
            Pass::Idle => None,
            Pass::Pages { .. } => Some(Stage::Pages),
            Pass::Ids { .. } => Some(Stage::Ids),
            Pass::Versions { .. } => Some(Stage::Versions),
            Pass::Contracts => Some(Stage::Contracts),
        }
    }

    /// Its name in the journal.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Pages => "pages",
            Stage::Ids => "ids",
            Stage::Versions => "versions",
            Stage::Contracts => "contracts",
        }
    }
}

/// What the screen is told while the pass runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub stage: Stage,
    /// Contracts (ids, versions) done in this stage.
    pub done: u32,
    /// How many the stage has, once known; never less than `done`. `None` for a first pass's
    /// pages: the network's count is unknown until the last page.
    pub of: Option<u32>,
    /// About how many requests the pass still makes, once they can be counted.
    pub requests_left: Option<u32>,
    /// Requests answered in this pass, and the ms since it began: its pace.
    pub requests: u32,
    pub ms: u64,
    /// Requests answered in this stage, and the ms since it began (the journal's stage line).
    pub stage_requests: u32,
    pub stage_ms: u64,
    /// Paused by a node until this time (ms of the caller's clock).
    pub paused_until: Option<u64>,
    /// The size of the requests with bodies changed: the new size and the ms of the request
    /// that changed it (the journal's `crawl_page`, counts only).
    pub resized: Option<(u32, u64)>,
}

/// One stage's count, from where it began.
struct Tally {
    stage: Stage,
    done: u32,
    of: Option<u32>,
    requests: u32,
    since: u64,
}

impl Tally {
    /// The stage `cache` is in now, begun at `now`.
    fn begin<P>(cache: &Cache<P>, now: u64) -> Option<Tally> {
        let (stage, done, of) = so_far(cache)?;
        Some(Tally {
            stage,
            done,
            of,
            requests: 0,
            since: now,
        })
    }
}

/// The stage of the pass under way in `cache`, what it has done of it already (in this run or
/// in one cut short before: the count does not go back after a cut and a retry, BUG-238) and
/// of how many, once known. A stage of contracts to read counts from where this run takes it:
/// what was read before a cut is not told apart from what is left.
pub fn so_far<P>(cache: &Cache<P>) -> Option<(Stage, u32, Option<u32>)> {
    let stage = Stage::of(&cache.pass)?;
    let up_to = |after: Option<Id>, id: &Id| after.is_some_and(|a| *id <= a);
    let read = cache.ids.values().filter(|v| v.is_some()).count() as u32;
    Some(match cache.pass {
        // A first pass's pages read every id they have.
        Pass::Pages { .. } => (stage, cache.ids.len() as u32, None),
        Pass::Ids { after } => (
            stage,
            cache.ids.keys().filter(|id| up_to(after, id)).count() as u32,
            Some(cache.ids.len() as u32),
        ),
        Pass::Versions { after } => (
            stage,
            cache
                .ids
                .iter()
                .filter(|(id, v)| v.is_some() && up_to(after, id))
                .count() as u32,
            Some(read),
        ),
        Pass::Contracts => (stage, 0, Some(cache.to_read.len() as u32)),
        Pass::Idle => return None,
    })
}

/// How many `ask` covers: what it counts towards its stage's `done`.
fn counted<P>(ask: &Ask, reply: &Reply<P>) -> u32 {
    let n = match (ask, reply) {
        (_, Reply::Pages(page)) => page.len(),
        (_, Reply::Ids(page)) => page.len(),
        (Ask::Versions(ids) | Ask::Contracts(ids), _) => ids.len(),
        _ => 0,
    };
    n as u32
}

/// The requests the pass still makes, as far as they can be counted now: none for pages.
fn requests_left<P>(cache: &Cache<P>, tally: &Tally, limits: Limits) -> Option<u32> {
    let per = |n: u32, by: usize| n.div_ceil(by.max(1) as u32);
    let to_read = per(cache.to_read.len() as u32, limits.batch);
    let read = cache.ids.values().filter(|v| v.is_some()).count() as u32;
    let left = tally.of.unwrap_or(0).saturating_sub(tally.done);
    match Stage::of(&cache.pass) {
        None => Some(0),
        Some(Stage::Pages) => None,
        Some(Stage::Ids) => {
            Some(per(left, limits.ids_page as usize) + per(read, limits.batch) + to_read)
        }
        Some(Stage::Versions) => Some(per(left, limits.batch) + to_read),
        Some(Stage::Contracts) => Some(to_read),
    }
}

/// Run the pass under way in `cache` (or begin one) until it is done, stopped or failed.
#[allow(clippy::too_many_arguments)]
pub async fn pass<P, S, SF, W, WF>(
    cache: &mut Cache<P>,
    limits: Limits,
    gap_ms: u64,
    mut step: S,
    now_ms: impl Fn() -> u64,
    mut wait: W,
    live: impl Fn() -> bool,
    mut progress: impl FnMut(Progress),
    mut save: impl FnMut(&Cache<P>),
) -> Ended
where
    S: FnMut(Ask) -> SF,
    SF: Future<Output = Result<Reply<P>, Failure>>,
    W: FnMut(Duration) -> WF,
    WF: Future<Output = ()>,
{
    cache.begin();
    let mut pacer = Pacer::new(vec![()], gap_ms);
    let mut failures = 0;
    let mut since_save = 0;
    let began = now_ms();
    let mut requests = 0u32;
    let mut tally = Tally::begin(cache, began);
    let mut sizer = Sizer::new();
    let ended = loop {
        let sized = Sizer::sizes(&cache.pass);
        let limits = sizer.limits(&cache.pass, limits);
        let Some(ask) = cache.ask(limits) else {
            break Ended::Done;
        };
        if !live() {
            break Ended::Stopped;
        }
        match pacer.turn(now_ms()) {
            Turn::Ask(()) => {}
            Turn::Wait(ms) => {
                wait(Duration::from_millis(ms)).await;
                continue;
            }
            Turn::NoNodes => break Ended::Failed("no nodes".into()),
        }
        let asked_at = now_ms();
        match step(ask.clone()).await {
            Ok(reply) => {
                let n = counted(&ask, &reply);
                let mut resized = None;
                if cache.take(&ask, reply, limits).is_err() {
                    failures += 1;
                } else {
                    failures = 0;
                    if sized && let Some(size) = sizer.answered() {
                        resized = Some((size, now_ms().saturating_sub(asked_at)));
                    }
                    requests += 1;
                    since_save += 1;
                    if since_save >= SAVE_EVERY {
                        save(cache);
                        since_save = 0;
                    }
                    if let Some(t) = tally.as_mut() {
                        t.done = t.done.saturating_add(n);
                        t.requests += 1;
                        if t.stage == Stage::Contracts {
                            // What was read, plus what is left to read.
                            t.of = Some(t.done + cache.to_read.len() as u32);
                        }
                    }
                }
                if let Some(t) = &tally {
                    let now = now_ms();
                    let mut said = told(t, cache, limits, (began, now, requests), None);
                    said.resized = resized;
                    progress(said);
                }
                // A new stage begins its own count, told from its first answer on.
                if let Some(stage) = Stage::of(&cache.pass)
                    && tally.as_ref().is_none_or(|t| t.stage != stage)
                {
                    tally = Tally::begin(cache, now_ms());
                }
            }
            Err(Failure::Slow(wait_for)) => {
                let until = now_ms().saturating_add(wait_for.as_millis() as u64);
                pacer.slow_down(&(), until);
                if let Some(t) = &tally {
                    let now = now_ms();
                    let paused = pacer.all_paused_until(now);
                    progress(told(t, cache, limits, (began, now, requests), paused));
                }
            }
            Err(Failure::TimedOut(e)) => {
                // Too large for the network now: the same place again at once, smaller. Only
                // at the smallest size is it a failure.
                let ms = now_ms().saturating_sub(asked_at);
                match sized.then(|| sizer.timed_out(asked_items(&ask))).flatten() {
                    Some(size) => {
                        if let Some(t) = &tally {
                            let mut said =
                                told(t, cache, limits, (began, now_ms(), requests), None);
                            said.resized = Some((size, ms));
                            progress(said);
                        }
                    }
                    None => {
                        failures += 1;
                        if failures >= FAILURES {
                            break Ended::Failed(e);
                        }
                        wait(Duration::from_millis(gap_ms.saturating_mul(5))).await;
                    }
                }
            }
            Err(Failure::Failed(e)) => {
                failures += 1;
                if failures >= FAILURES {
                    break Ended::Failed(e);
                }
                wait(Duration::from_millis(gap_ms.saturating_mul(5))).await;
            }
        }
        if failures >= FAILURES {
            break Ended::Failed("answers to another question".into());
        }
    };
    save(cache);
    ended
}

/// What the screen is told: the stage's count and the pass's pace. `clock`: when the pass
/// began, now, and the requests answered.
fn told<P>(
    tally: &Tally,
    cache: &Cache<P>,
    limits: Limits,
    (began, now, requests): (u64, u64, u32),
    paused_until: Option<u64>,
) -> Progress {
    Progress {
        stage: tally.stage,
        done: tally.done,
        of: tally.of.map(|of| of.max(tally.done)),
        requests_left: requests_left(cache, tally, limits),
        requests,
        ms: now.saturating_sub(began),
        stage_requests: tally.requests,
        stage_ms: now.saturating_sub(tally.since),
        paused_until,
        resized: None,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    use super::super::crawl::{Id, Read};
    use super::*;

    const LIMITS: Limits = Limits {
        ids_page: 3,
        batch: 2,
    };

    fn id(n: u8) -> Id {
        let mut id = [0; 32];
        id[31] = n;
        id
    }

    /// A network of `n` contracts, every third with a token; it fails as `script` says, one
    /// entry per request (`None`: answer).
    struct Net {
        contracts: BTreeMap<Id, u32>,
        script: RefCell<Vec<Option<Failure>>>,
        asked: Cell<u32>,
        /// A page of more contracts than this times out (a slow Platform, qa-f41).
        times_out_over: Option<u32>,
        /// The size of each page asked, in order.
        pages: RefCell<Vec<u32>>,
    }

    impl Net {
        fn new(n: u8, script: Vec<Option<Failure>>) -> Self {
            Net {
                contracts: (1..=n).map(|i| (id(i), 1)).collect(),
                script: RefCell::new(script),
                asked: Cell::new(0),
                times_out_over: None,
                pages: RefCell::default(),
            }
        }

        fn answer(&self, ask: Ask) -> Result<Reply<String>, Failure> {
            self.asked.set(self.asked.get() + 1);
            if let Ask::Pages { limit, .. } = ask {
                self.pages.borrow_mut().push(limit);
                if self.times_out_over.is_some_and(|over| limit > over) {
                    return Err(Failure::TimedOut("deadline".into()));
                }
            }
            let mut script = self.script.borrow_mut();
            if !script.is_empty()
                && let Some(failure) = script.remove(0)
            {
                return Err(failure);
            }
            Ok(match ask {
                Ask::Pages { after, limit } => Reply::Pages(
                    self.contracts
                        .iter()
                        .filter(|(k, _)| after.is_none_or(|a| **k > a))
                        .take(limit as usize)
                        .map(|(k, v)| Read {
                            id: *k,
                            version: *v,
                            tokens: (k[31] % 3 == 0).then(|| format!("T{}", k[31])),
                        })
                        .collect(),
                ),
                Ask::Ids { after, limit } => Reply::Ids(
                    self.contracts
                        .keys()
                        .filter(|k| after.is_none_or(|a| **k > a))
                        .take(limit as usize)
                        .copied()
                        .collect(),
                ),
                Ask::Versions(ids) => {
                    Reply::Versions(ids.iter().map(|i| (*i, self.contracts[i])).collect())
                }
                Ask::Contracts(ids) => Reply::Contracts(
                    ids.iter()
                        .map(|i| Read {
                            id: *i,
                            version: self.contracts[i],
                            tokens: (i[31] % 3 == 0).then(|| format!("T{}", i[31])),
                        })
                        .collect(),
                ),
            })
        }
    }

    /// Runs a pass on a clock that only the waits move; returns how it ended, the waits, the
    /// saves and the last progress.
    fn run(
        cache: &mut Cache<String>,
        net: &Net,
        live: impl Fn() -> bool,
    ) -> (Ended, Vec<Duration>, u32, Vec<Progress>) {
        run_with(LIMITS, cache, net, live)
    }

    /// [`run`] with `limits` of its own.
    fn run_with(
        limits: Limits,
        cache: &mut Cache<String>,
        net: &Net,
        live: impl Fn() -> bool,
    ) -> (Ended, Vec<Duration>, u32, Vec<Progress>) {
        let clock = Cell::new(0u64);
        let waits = RefCell::new(Vec::new());
        let saves = Cell::new(0);
        let seen = RefCell::new(Vec::new());
        let ended = futures_lite_block_on(pass(
            cache,
            limits,
            1_000,
            |ask| std::future::ready(net.answer(ask)),
            || clock.get(),
            |d: Duration| {
                clock.set(clock.get() + d.as_millis() as u64);
                waits.borrow_mut().push(d);
                std::future::ready(())
            },
            live,
            |p| seen.borrow_mut().push(p),
            |_| saves.set(saves.get() + 1),
        ));
        (ended, waits.into_inner(), saves.get(), seen.into_inner())
    }

    /// The futures here are ready at once: one poll each.
    fn futures_lite_block_on<F: Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        loop {
            if let std::task::Poll::Ready(out) = f.as_mut().poll(&mut cx) {
                return out;
            }
        }
    }

    #[test]
    fn a_whole_pass_at_one_request_a_second() {
        let net = Net::new(7, Vec::new());
        let mut cache = Cache::default();
        let (ended, waits, saves, seen) = run(&mut cache, &net, || true);
        assert_eq!(ended, Ended::Done);
        assert_eq!(cache.passes, 1);
        assert_eq!(cache.tokens.len(), 2, "contracts 3 and 6");
        // A first pass: 3 pages with their contracts (dev2-3), nothing else.
        assert_eq!(net.asked.get(), 3);
        assert!(waits.iter().all(|w| *w <= Duration::from_secs(1)));
        let waited: Duration = waits.iter().sum();
        assert_eq!(
            waited,
            Duration::from_secs(2),
            "one second between requests"
        );
        assert!(saves >= 1, "saved at the end");
        let last = seen.last().expect("progress");
        assert_eq!((last.stage, last.done, last.of), (Stage::Pages, 7, None));
    }

    #[test]
    fn a_rate_limit_pauses_the_pass_and_it_goes_on() {
        let slow = Some(Failure::Slow(Duration::from_secs(30)));
        let net = Net::new(7, vec![None, slow]);
        let mut cache = Cache::default();
        let (ended, waits, _, _) = run(&mut cache, &net, || true);
        assert_eq!(ended, Ended::Done, "a pause is not a failure");
        assert!(waits.contains(&Duration::from_secs(30)), "waited the reset");
        assert_eq!(cache.tokens.len(), 2);
    }

    #[test]
    fn three_failures_in_a_row_end_the_pass_where_it_was() {
        let failed = || Some(Failure::Failed("no answer".into()));
        let net = Net::new(7, vec![None, failed(), failed(), failed()]);
        let mut cache = Cache::default();
        let (ended, _, saves, _) = run(&mut cache, &net, || true);
        assert_eq!(ended, Ended::Failed("no answer".into()));
        assert_eq!(saves, 1, "saved where it stopped");
        assert_eq!(cache.passes, 0);
        // The next pass goes on from the second page, not from the start.
        let net = Net::new(7, Vec::new());
        let (ended, _, _, _) = run(&mut cache, &net, || true);
        assert_eq!(ended, Ended::Done);
        assert_eq!(net.asked.get(), 2, "the first page is not read again");
    }

    #[test]
    fn a_lock_stops_the_pass_and_keeps_where_it_was() {
        let net = Net::new(7, Vec::new());
        let mut cache = Cache::default();
        let asked = Cell::new(0);
        let (ended, _, saves, _) = run(&mut cache, &net, || {
            asked.set(asked.get() + 1);
            asked.get() <= 2
        });
        assert_eq!(ended, Ended::Stopped);
        assert_eq!(saves, 1);
        // Asked once; `live` is asked again during the wait before the second request, which
        // a lock cuts short as well.
        assert_eq!(net.asked.get(), 1);
        assert_ne!(
            cache.pass,
            super::super::crawl::Pass::Idle,
            "a pass under way"
        );
    }

    /// F: the progress says the stage, how many it has done and of how many, once that is
    /// known; never more done than in all (QA: «4 190 из 4 189»); and the requests left, once
    /// they can be counted. A first pass's pages have no «of»: the network's count is unknown.
    #[test]
    fn the_progress_says_the_stage_and_its_count() {
        let net = Net::new(7, Vec::new());
        let mut cache = Cache::default();
        let (_, _, _, seen) = run(&mut cache, &net, || true);
        let pages: Vec<(Stage, u32, Option<u32>, Option<u32>)> = seen
            .iter()
            .map(|p| (p.stage, p.done, p.of, p.requests_left))
            .collect();
        assert_eq!(
            pages,
            [
                (Stage::Pages, 3, None, None),
                (Stage::Pages, 6, None, None),
                (Stage::Pages, 7, None, Some(0)),
            ]
        );
        assert_eq!(seen.last().map(|p| p.requests), Some(3));
        // A later pass, two new contracts: ids, then versions, then the new ones.
        let mut net = Net::new(9, Vec::new());
        net.contracts.insert(id(3), 2);
        let (ended, _, _, seen) = run(&mut cache, &net, || true);
        assert_eq!(ended, Ended::Done);
        for p in &seen {
            assert!(p.of.is_some_and(|of| p.done <= of), "{p:?}");
        }
        let stages: Vec<Stage> = seen.iter().map(|p| p.stage).collect();
        assert!(stages.contains(&Stage::Ids) && stages.contains(&Stage::Contracts));
        let ids = seen.iter().rfind(|p| p.stage == Stage::Ids).unwrap();
        assert_eq!((ids.done, ids.of), (9, Some(9)), "9 ids of 7 known before");
        let read = seen.last().unwrap();
        assert_eq!(read.stage, Stage::Contracts);
        assert_eq!((read.done, read.of), (3, Some(3)), "8, 9 and the grown 3");
        assert_eq!(read.requests_left, Some(0));
        // Each stage says how many requests and how long it took, for the journal.
        let versions = seen.iter().rfind(|p| p.stage == Stage::Versions).unwrap();
        assert_eq!(versions.stage_requests, 4, "7 versions by 2");
        assert!(versions.stage_ms >= 3_000, "a second between them");
    }

    /// The search's real limits: pages and batches of 100.
    const FULL: Limits = Limits {
        ids_page: 100,
        batch: 100,
    };

    /// Batch 39 (architect's decision, (a)): pages over 10 time out. A run starts at 25, the
    /// timeout asks the same place again at once at 10 (not a failure, no wait), and after 3
    /// answers in a row it tries 25 again. The pass ends whole.
    #[test]
    fn a_timeout_asks_the_same_place_smaller_and_grows_back() {
        let mut net = Net::new(80, Vec::new());
        net.times_out_over = Some(10);
        let mut cache = Cache::default();
        let (ended, waits, _, seen) = run_with(FULL, &mut cache, &net, || true);
        assert_eq!(ended, Ended::Done);
        assert_eq!(cache.ids.len(), 80, "every contract read");
        let pages = net.pages.borrow().clone();
        assert_eq!(&pages[..6], [25, 10, 10, 10, 25, 10], "{pages:?}");
        assert!(
            waits.iter().all(|w| *w <= Duration::from_secs(1)),
            "a timeout is not waited out as a failure: {waits:?}"
        );
        let resized: Vec<u32> = seen.iter().filter_map(|p| p.resized).map(|r| r.0).collect();
        assert_eq!(&resized[..3], [10, 25, 10], "{resized:?}");
    }

    /// Pages over 5 time out: 25, then 10, then 5, where it goes on.
    #[test]
    fn a_slower_network_goes_on_at_five() {
        let mut net = Net::new(30, Vec::new());
        net.times_out_over = Some(5);
        let mut cache = Cache::default();
        let (ended, _, _, _) = run_with(FULL, &mut cache, &net, || true);
        assert_eq!(ended, Ended::Done);
        assert_eq!(cache.ids.len(), 30);
        let pages = net.pages.borrow().clone();
        assert_eq!(&pages[..6], [25, 10, 5, 5, 5, 10], "{pages:?}");
    }

    /// A timeout at the smallest size is a failure: three of them end the pass, where it was.
    #[test]
    fn a_timeout_at_five_is_a_failure() {
        let mut net = Net::new(30, Vec::new());
        net.times_out_over = Some(0);
        let mut cache = Cache::default();
        let (ended, _, saves, _) = run_with(FULL, &mut cache, &net, || true);
        assert_eq!(ended, Ended::Failed("deadline".into()));
        assert_eq!(net.pages.borrow().as_slice(), [25, 10, 5, 5, 5]);
        assert_eq!(saves, 1);
        assert_eq!(cache.pass, Pass::Pages { after: None });
    }

    /// Batch 39, (c): after a cut and a retry the count goes on from what the pass read, not
    /// from 0 («0 of 800» after «Retry»).
    #[test]
    fn the_count_does_not_go_back_after_a_cut_and_a_retry() {
        let failed = || Some(Failure::Failed("no answer".into()));
        let net = Net::new(7, vec![None, failed(), failed(), failed()]);
        let mut cache = Cache::default();
        let (_, _, _, seen) = run(&mut cache, &net, || true);
        assert_eq!(seen.last().map(|p| p.done), Some(3));
        assert_eq!(
            so_far(&cache),
            Some((Stage::Pages, 3, None)),
            "what the retry shows first"
        );
        let net = Net::new(7, Vec::new());
        let (ended, _, _, seen) = run(&mut cache, &net, || true);
        assert_eq!(ended, Ended::Done);
        let done: Vec<u32> = seen.iter().map(|p| p.done).collect();
        assert_eq!(done, [6, 7], "on from 3, never back");
    }

    /// A later pass cut in its ids: the retry counts the ids already paged.
    #[test]
    fn a_cut_in_the_ids_counts_on_from_there() {
        let mut cache = Cache::default();
        run(&mut cache, &Net::new(7, Vec::new()), || true);
        let failed = || Some(Failure::Failed("no answer".into()));
        let net = Net::new(9, vec![None, failed(), failed(), failed()]);
        run(&mut cache, &net, || true);
        assert_eq!(
            so_far(&cache),
            Some((Stage::Ids, 3, Some(7))),
            "of the 7 ids known when it began"
        );
    }
}
