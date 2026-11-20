//! «Workshop» series (FEAT-0038 item 1): one action over many instances, one write each, run
//! here on the core's rule for a write sent but not seen (BUG-243, `sent`). At most
//! [`AT_ONCE`] writes at a time (AC-T-M3); before each, the credits (R-19) and, for an instance
//! on sale, its owner and price read again; a node asking to slow down pauses the series,
//! doubling (M3); a lock stops it before its next write while the ones sent finish (SEC-6);
//! «Continue» first reads again every write sent without an answer and repeats nothing done
//! (M2, R-13). The network is a [`SeriesNet`], so all of it is tested without one.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicU64;

use evolite_chain::nft_series::plan::Instance;
use evolite_local::nft_series::{Item, Kind, NftSeries, State};

use crate::real::sent::{self, NO_GAP, Outcome, QUIET, SEND_WAIT};
use crate::real::*;

/// Writes of a series in flight at once (AC-T-M3).
pub(in crate::real) const AT_ONCE: usize = 4;

/// The longest pause a node's «too often» doubles to.
const MAX_PAUSE: Duration = Duration::from_secs(64);

pub(in crate::real) type Fut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// One write of a series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::real) enum Write {
    Mint,
    Price(u64),
    /// To another (a gift) or to oneself (off sale).
    Transfer(Identifier),
    Delete,
}

/// The network of one series: its identity, contract and model.
pub(in crate::real) trait SeriesNet: Send + Sync + 'static {
    /// The identity's credits, read fresh (credits, not duffs).
    fn credits(&self) -> Fut<Result<u64, String>>;
    /// The instance read again with proof: by its document, or (a mint) by its serial.
    fn read(&self, serial: u32, doc: Option<Identifier>) -> Fut<Result<Option<Instance>, String>>;
    /// The write; a mint's answer is its document.
    fn write(
        &self,
        write: Write,
        serial: u32,
        doc: Option<Identifier>,
    ) -> Fut<Result<Option<Identifier>, String>>;
    /// The identity writing.
    fn me(&self) -> Identifier;
}

/// Where the running series stand, and the lock's mark.
#[derive(Default)]
pub(in crate::real) struct SeriesRun {
    /// Bumped by the lock: a series started before it sends nothing more.
    run: AtomicU64,
    /// «Stop after the current ones», by series.
    stopping: std::sync::Mutex<HashSet<u64>>,
    /// The series running now.
    running: std::sync::Mutex<HashSet<u64>>,
}

impl SeriesRun {
    pub(in crate::real) fn lock(&self) {
        self.run.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::real) fn current(&self) -> u64 {
        self.run.load(Ordering::Relaxed)
    }

    pub(in crate::real) fn stop(&self, id: u64) {
        self.stopping
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id);
    }

    fn stops(&self, id: u64) -> bool {
        self.stopping
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&id)
    }

    pub(in crate::real) fn is_running(&self, id: u64) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&id)
    }

    fn set_running(&self, id: u64, on: bool) {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if on {
            running.insert(id);
        } else {
            running.remove(&id);
            self.stopping
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
        }
    }
}

/// The write of `kind` for an instance.
fn write_of(kind: &Kind, me: Identifier, to: Option<Identifier>) -> Write {
    match kind {
        Kind::Mint => Write::Mint,
        Kind::Price { price } | Kind::Reprice { price, .. } => Write::Price(*price),
        Kind::Gift { .. } => Write::Transfer(to.unwrap_or(me)),
        Kind::Unprice => Write::Transfer(me),
        Kind::Delete => Write::Delete,
    }
}

/// Whether a read of the instance shows its write done.
fn shows(kind: &Kind, me: &Identifier, to: Option<&Identifier>, read: &Option<Instance>) -> bool {
    match (kind, read) {
        (Kind::Mint, Some(i)) => i.owner == *me,
        (Kind::Price { price } | Kind::Reprice { price, .. }, Some(i)) => {
            i.owner == *me && i.price == Some(*price)
        }
        (Kind::Gift { .. }, Some(i)) => Some(&i.owner) == to,
        (Kind::Unprice, Some(i)) => i.owner == *me && i.price.is_none(),
        (Kind::Delete, None) => true,
        _ => false,
    }
}

/// How a series run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::real) enum Ended {
    /// Every write done or refused.
    Finished,
    /// Stopped with writes left: by the person, a lock, too few credits, or writes not seen
    /// yet; «Continue» goes on.
    Partial(&'static str),
}

/// Runs series `id` of `store` on `net` until it is finished or stopped. `to`: a gift's
/// recipient. `per_write`: one write's estimate (credits), checked against the credits before
/// each. `now`: unix seconds (the quiet's clock).
pub(in crate::real) async fn run<N: SeriesNet>(
    net: Arc<N>,
    store: Arc<NftSeries>,
    runs: Arc<SeriesRun>,
    id: u64,
    to: Option<Identifier>,
    per_write: u64,
    now: impl Fn() -> u64 + Send + Sync + 'static,
) -> Ended {
    let started = runs.current();
    runs.set_running(id, true);
    let ended = run_inner(&net, &store, &runs, id, to, per_write, &now, started).await;
    runs.set_running(id, false);
    ended
}

#[allow(clippy::too_many_arguments)]
async fn run_inner<N: SeriesNet>(
    net: &Arc<N>,
    store: &Arc<NftSeries>,
    runs: &Arc<SeriesRun>,
    id: u64,
    to: Option<Identifier>,
    per_write: u64,
    now: &(impl Fn() -> u64 + Send + Sync),
    started: u64,
) -> Ended {
    let Some(series) = store.get(id) else {
        return Ended::Finished;
    };
    let me = net.me();
    let kind = series.kind.clone();
    // M2 / R-13: what was sent without an answer is read first; done is not sent again.
    for item in series
        .items
        .iter()
        .filter(|i| matches!(i.state, State::Sending | State::Unclear))
    {
        settle(net, store, id, &kind, &me, to.as_ref(), item, now()).await;
    }
    let mut pause = Duration::ZERO;
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        let series = store.get(id).expect("the series");
        let waiting: Vec<Item> = series
            .items
            .iter()
            .filter(|i| i.state == State::Waiting)
            .cloned()
            .collect();
        let live = runs.current() == started && !runs.stops(id);
        let mut next = waiting.into_iter();
        // Launch up to AT_ONCE.
        while live && tasks.len() < AT_ONCE {
            let Some(item) = next.next() else { break };
            // R-19: not sent without the credits for it; the series stops partial.
            match net.credits().await {
                Ok(c) if c >= per_write => {}
                Ok(_) => {
                    while tasks.join_next().await.is_some() {}
                    return Ended::Partial("credits");
                }
                Err(_) => {
                    while tasks.join_next().await.is_some() {}
                    return Ended::Partial("network");
                }
            }
            let (net, store, kind) = (net.clone(), store.clone(), kind.clone());
            let at = now();
            // r1 Must: its send time on disk before it goes out, so a «Continue» after evolite
            // closed mid-write waits the quiet from here (BUG-243), not from 0.
            store
                .edit(id, item.serial, |i| {
                    i.state = State::Sending;
                    i.sent_at = Some(at);
                    i.no_since = None;
                })
                .ok();
            tasks.spawn(async move { one(&net, &store, id, &kind, me, to, item, at).await });
        }
        match tasks.join_next().await {
            Some(Ok(Slow::Yes)) => {
                pause = (pause * 2).max(Duration::from_secs(1)).min(MAX_PAUSE);
                tokio::time::sleep(pause).await;
            }
            Some(_) => {}
            None => break,
        }
    }
    let series = store.get(id).expect("the series");
    if series.finished() {
        Ended::Finished
    } else if runs.current() != started {
        Ended::Partial("lock")
    } else if runs.stops(id) {
        Ended::Partial("stopped")
    } else {
        Ended::Partial("unclear")
    }
}

/// Whether a node asked to slow down (the write goes back to waiting).
enum Slow {
    Yes,
    No,
}

/// One write: the instance on sale read again first (R-19), then sent by `sent::run`.
#[allow(clippy::too_many_arguments)]
async fn one<N: SeriesNet>(
    net: &Arc<N>,
    store: &Arc<NftSeries>,
    id: u64,
    kind: &Kind,
    me: Identifier,
    to: Option<Identifier>,
    item: Item,
    at: u64,
) -> Slow {
    let doc = item
        .doc
        .as_deref()
        .and_then(|d| Identifier::from_string(d, Encoding::Base58).ok());
    // An instance on sale may have been bought meanwhile: no write if it is not ours, or its
    // price is not the one the series changes.
    if let Kind::Unprice | Kind::Reprice { .. } = kind {
        match net.read(item.serial, doc).await {
            Ok(Some(i)) if i.owner != me => {
                store
                    .set(
                        id,
                        item.serial,
                        State::Refused,
                        None,
                        Some("not yours".into()),
                    )
                    .ok();
                return Slow::No;
            }
            Ok(Some(i)) if matches!(kind, Kind::Reprice { from, .. } if i.price != Some(*from)) => {
                store
                    .set(
                        id,
                        item.serial,
                        State::Refused,
                        None,
                        Some("price changed".into()),
                    )
                    .ok();
                return Slow::No;
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                store
                    .set(
                        id,
                        item.serial,
                        State::Refused,
                        None,
                        Some("not found".into()),
                    )
                    .ok();
                return Slow::No;
            }
            Err(_) => {
                store.set(id, item.serial, State::Waiting, None, None).ok();
                return Slow::No;
            }
        }
    }
    let write = write_of(kind, me, to);
    let (send_net, reread_net) = (net.clone(), net.clone());
    let serial = item.serial;
    let outcome = sent::run(
        async move { send_net.write(write, serial, doc).await },
        SEND_WAIT,
        |e: &String| sent::refused_before(e) || slow(e),
        move || async move { reread_net.read(serial, doc).await.ok() },
        |read| shows(kind, &me, to.as_ref(), read),
    )
    .await;
    match outcome {
        Outcome::Done(written) => {
            let doc = written.or(doc).map(|d| d.to_string(Encoding::Base58));
            store.set(id, serial, State::Done, doc, None).ok();
            Slow::No
        }
        Outcome::Shown(read) => {
            let doc = read.map(|i| i.id.to_string(Encoding::Base58));
            store.set(id, serial, State::Done, doc, None).ok();
            Slow::No
        }
        Outcome::Refused(e) if slow(&e) => {
            store.set(id, serial, State::Waiting, None, None).ok();
            Slow::Yes
        }
        Outcome::Refused(e) => {
            store
                .set(id, serial, State::Refused, None, Some(short(&e)))
                .ok();
            Slow::No
        }
        Outcome::Unclear { .. } => {
            store
                .edit(id, serial, |i| {
                    i.state = State::Unclear;
                    i.sent_at = Some(at);
                    i.no_since = None;
                })
                .ok();
            Slow::No
        }
    }
}

/// A write sent without an answer, read again (BUG-243): shown — done; not shown past the
/// quiet twice, [`NO_GAP`] apart — not landed, to be sent again; otherwise left unclear.
#[allow(clippy::too_many_arguments)]
async fn settle<N: SeriesNet>(
    net: &Arc<N>,
    store: &Arc<NftSeries>,
    id: u64,
    kind: &Kind,
    me: &Identifier,
    to: Option<&Identifier>,
    item: &Item,
    now: u64,
) {
    let doc = item
        .doc
        .as_deref()
        .and_then(|d| Identifier::from_string(d, Encoding::Base58).ok());
    let Ok(read) = net.read(item.serial, doc).await else {
        return;
    };
    if shows(kind, me, to, &read) {
        let doc = read
            .map(|i| i.id.to_string(Encoding::Base58))
            .or(item.doc.clone());
        store.set(id, item.serial, State::Done, doc, None).ok();
        return;
    }
    // An entry of an older file without its send time: its quiet runs from this «Continue».
    let Some(sent_at) = item.sent_at else {
        store
            .edit(id, item.serial, |i| {
                i.state = State::Unclear;
                i.sent_at = Some(now);
            })
            .ok();
        return;
    };
    let quiet = now.saturating_sub(sent_at) >= QUIET.as_secs();
    store
        .edit(id, item.serial, |i| {
            i.state = State::Unclear;
            i.sent_at.get_or_insert(now);
            if !quiet {
                return;
            }
            match i.no_since {
                Some(first) if now.saturating_sub(first) >= NO_GAP.as_secs() => {
                    i.state = State::Waiting;
                    i.sent_at = None;
                    i.no_since = None;
                }
                Some(_) => {}
                None => i.no_since = Some(now),
            }
        })
        .ok();
}

/// A node asking to slow down: not a refusal of the write.
fn slow(e: &str) -> bool {
    let e = e.to_lowercase();
    e.contains("429") || e.contains("resourceexhausted") || e.contains("resource exhausted")
}

/// A refusal in a few words, for the History card.
fn short(e: &str) -> String {
    e.chars().take(120).collect()
}

#[cfg(test)]
mod tests;
