//! One pass of the token search (FEAT-0019 UC-1а, R-11): the [`crawl`] state asks, the network
//! answers, the [`pace`] keeps it to one request a second. The network, the clock and the sleep
//! are the caller's, so the pass is tested without either. The SDK picks the node of each request
//! itself, so the pool is paced as one node (HANDOFF part 3); a node that asks to slow down
//! pauses the pass, never bans the node (BUG-78).

use std::future::Future;
use std::time::Duration;

use super::crawl::{Ask, Cache, Limits, Reply};
use super::pace::{Pacer, Turn};

/// How a step failed (see `chain::CrawlFailure`, which the real core maps to this).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Wait this long, then go on.
    Slow(Duration),
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

/// What the screen is told while the pass runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Ids read in this pass.
    pub ids: u32,
    /// Contracts still to read.
    pub left: u32,
    /// Paused by a node until this time (ms of the caller's clock).
    pub paused_until: Option<u64>,
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
    let mut ids = 0u32;
    let ended = loop {
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
        match step(ask.clone()).await {
            Ok(reply) => {
                if let Reply::Ids(page) = &reply {
                    ids = ids.saturating_add(page.len() as u32);
                }
                if cache.take(&ask, reply, limits).is_err() {
                    failures += 1;
                } else {
                    failures = 0;
                    since_save += 1;
                    if since_save >= SAVE_EVERY {
                        save(cache);
                        since_save = 0;
                    }
                }
                progress(Progress {
                    ids,
                    left: cache.to_read.len() as u32,
                    paused_until: None,
                });
            }
            Err(Failure::Slow(wait_for)) => {
                let until = now_ms().saturating_add(wait_for.as_millis() as u64);
                pacer.slow_down(&(), until);
                progress(Progress {
                    ids,
                    left: cache.to_read.len() as u32,
                    paused_until: pacer.all_paused_until(now_ms()),
                });
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
    }

    impl Net {
        fn new(n: u8, script: Vec<Option<Failure>>) -> Self {
            Net {
                contracts: (1..=n).map(|i| (id(i), 1)).collect(),
                script: RefCell::new(script),
                asked: Cell::new(0),
            }
        }

        fn answer(&self, ask: Ask) -> Result<Reply<String>, Failure> {
            self.asked.set(self.asked.get() + 1);
            let mut script = self.script.borrow_mut();
            if !script.is_empty()
                && let Some(failure) = script.remove(0)
            {
                return Err(failure);
            }
            Ok(match ask {
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
    ) -> (Ended, Vec<Duration>, u32, Option<Progress>) {
        let clock = Cell::new(0u64);
        let waits = RefCell::new(Vec::new());
        let saves = Cell::new(0);
        let last = Cell::new(None);
        let ended = futures_lite_block_on(pass(
            cache,
            LIMITS,
            1_000,
            |ask| std::future::ready(net.answer(ask)),
            || clock.get(),
            |d: Duration| {
                clock.set(clock.get() + d.as_millis() as u64);
                waits.borrow_mut().push(d);
                std::future::ready(())
            },
            live,
            |p| last.set(Some(p)),
            |_| saves.set(saves.get() + 1),
        ));
        (ended, waits.into_inner(), saves.get(), last.get())
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
        let (ended, waits, saves, last) = run(&mut cache, &net, || true);
        assert_eq!(ended, Ended::Done);
        assert_eq!(cache.passes, 1);
        assert_eq!(cache.tokens.len(), 2, "contracts 3 and 6");
        // 3 pages of ids, 0 versions (nothing read before), 4 batches of contracts.
        assert_eq!(net.asked.get(), 7);
        assert!(waits.iter().all(|w| *w <= Duration::from_secs(1)));
        let waited: Duration = waits.iter().sum();
        assert_eq!(
            waited,
            Duration::from_secs(6),
            "one second between requests"
        );
        assert!(saves >= 1, "saved at the end");
        assert_eq!(last.map(|p| (p.ids, p.left)), Some((7, 0)));
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
        assert_eq!(net.asked.get(), 6, "the first page is not read again");
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
}
