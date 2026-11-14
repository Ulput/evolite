//! The wallet's own search of the network for its tokens (FEAT-0019 UC-1а, R-11): one task at a
//! time, off the core's loop (R-12). "Find my tokens" starts a pass, and so does a day passing
//! since the last one; a lock or another wallet stops it, and the next pass goes on from where it
//! stopped (`token-search.json`, shared by the network's wallets). Testnet only (R-9).
//!
//! The speed-up (BUG-134…136, the architect's `analysis.md` §6):
//! - the tokens found are asked of the wallet's identities while the contracts are still read,
//!   100 at a time or 12 s after the first one not asked (A), up to 3 identities at once (D);
//! - a token held is followed from the contract the pass proved, not read again (dev2-4), and its
//!   row is read at once, as after "Add" (B);
//! - the identities are the wallet's as of each request, and one found after the pass gets a
//!   pass of its own over the tokens known (C);
//! - the contracts wait for the wallet's first lookup of its identities (G), at most 2 minutes;
//! - the screen is told the stage, its count and a time from the pace measured (F), and the
//!   journal a line per stage, counts and ms only.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use evolite_api::tokens::{SearchStage, SearchState, TokenInfo, TokenSearch};
use evolite_chain::tokens::crawl::{Cache, Limits};
use evolite_chain::tokens::{pace, run, store};
use evolite_local::wallet_tokens::{Origin, Tracked, WalletTokens};

use crate::diag::{self, Record};
use crate::real::*;

/// A page of ids and a batch of contracts or versions: Platform's most (card, АРХ-7).
const LIMITS: Limits = Limits {
    ids_page: 100,
    batch: 100,
};

/// A new pass in the background once a day while the wallet is open (card, «Обновление»).
const EVERY_SECS: Timestamp = 24 * 60 * 60;

/// Token ids asked of an identity in one request: Platform's most (A).
const BATCH: usize = 100;

/// A token found waits at most this long for others before its balances are asked (A).
const BATCH_WAIT: Duration = Duration::from_secs(12);

/// Identities asked at once (D): independent requests to a pool of tens of nodes.
const AT_ONCE: usize = 3;

/// How long the contracts wait for the wallet's first lookup of its identities (G).
const IDENTITIES_WAIT: Duration = Duration::from_secs(120);

/// Requests answered before their pace gives a time (design §T1).
const PACE_AFTER: u32 = 10;

/// The search: where it stands for the snapshot, and which run is the live one.
pub(in crate::real) struct Search {
    pub(in crate::real) state: Arc<Mutex<TokenSearch>>,
    run: Arc<AtomicU64>,
    path: PathBuf,
    /// The identities whose balances were asked over every token known (C); `None` until the
    /// first snapshot names the wallet's identities.
    balanced: Arc<Mutex<Option<HashSet<String>>>>,
}

impl Search {
    /// The search of the network in data folder `root`, where the last pass left it; on a network
    /// without tokens (`here` false: mainnet, R-9) nothing is read and the search is off.
    pub(in crate::real) fn on(here: bool, root: &Path) -> Self {
        let path = store::path(root);
        if !here {
            return Search {
                state: Arc::new(Mutex::new(TokenSearch::off())),
                run: Arc::default(),
                path,
                balanced: Arc::default(),
            };
        }
        let cache: Cache<chain::CrawlTokens> = store::load(&path);
        let done = (cache.passes > 0).then(|| {
            let at = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as Timestamp);
            (at, cache.ids.len() as u32)
        });
        let state = TokenSearch {
            state: if done.is_some() {
                SearchState::Idle
            } else {
                SearchState::NotYet
            },
            stage: SearchStage::Contracts,
            checked: 0,
            total: done.map(|(_, n)| n),
            left_secs: None,
            done,
            failed: false,
        };
        Search {
            state: Arc::new(Mutex::new(state)),
            run: Arc::default(),
            path,
            balanced: Arc::default(),
        }
    }

    /// A lock, another wallet or the app closing: the pass under way stops.
    pub(in crate::real) fn stop(&self) {
        self.run.fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::real) fn shown(&self) -> TokenSearch {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// A day has passed since the last whole pass, and none is under way.
    pub(in crate::real) fn due(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.state == SearchState::Idle
            && state.done.is_some_and(|(at, _)| now() - at >= EVERY_SECS)
    }

    /// Of the wallet's `identities` now, those to ask over every token known (C): the ones found
    /// since the first snapshot. Counted once each.
    pub(in crate::real) fn new_identities(&self, identities: &[String]) -> Vec<String> {
        newly_seen(
            &mut self.balanced.lock().unwrap_or_else(|e| e.into_inner()),
            identities,
        )
    }

    /// No pass under way, and one is done: what it found is there to ask of a new identity.
    pub(in crate::real) fn settled(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        !matches!(
            state.state,
            SearchState::Running | SearchState::Paused { .. }
        ) && state.done.is_some()
    }
}

/// Of `now`, the identities not in `balanced`, which takes them in. The first time (`None`)
/// every one is taken as asked already: the passes before this run asked them.
fn newly_seen(balanced: &mut Option<HashSet<String>>, now: &[String]) -> Vec<String> {
    let Some(known) = balanced else {
        *balanced = Some(now.iter().cloned().collect());
        return Vec::new();
    };
    now.iter()
        .filter(|id| known.insert((*id).clone()))
        .cloned()
        .collect()
}

fn set(state: &Mutex<TokenSearch>, change: impl FnOnce(&mut TokenSearch)) {
    change(&mut state.lock().unwrap_or_else(|e| e.into_inner()));
}

/// About how long the pass still takes, from its own pace (F): only after [`PACE_AFTER`]
/// requests, and only when what is left can be counted (not a first pass's pages).
fn left_secs(p: &run::Progress) -> Option<u32> {
    if p.requests < PACE_AFTER {
        return None;
    }
    let left = u64::from(p.requests_left?);
    Some((p.ms * left / u64::from(p.requests) / 1000) as u32)
}

/// What the screen says of a step of the contracts.
fn told(s: &mut TokenSearch, p: &run::Progress, paused_until: Option<Timestamp>) {
    s.stage = SearchStage::Contracts;
    s.checked = p.done;
    s.total = p.of;
    s.left_secs = left_secs(p);
    s.state = match paused_until {
        Some(until) => SearchState::Paused { until },
        None => SearchState::Running,
    };
}

/// The tokens found, waiting to be asked of the identities together (A).
#[derive(Default)]
struct Batch {
    waiting: Vec<String>,
    /// When the first one waiting came.
    since: Option<Instant>,
}

impl Batch {
    fn add(&mut self, tokens: impl IntoIterator<Item = String>, now: Instant) {
        let before = self.waiting.len();
        self.waiting.extend(tokens);
        if before == 0 && !self.waiting.is_empty() {
            self.since = Some(now);
        }
    }

    /// [`BATCH`] are waiting, or the first has waited [`BATCH_WAIT`].
    fn due(&self, now: Instant) -> bool {
        self.waiting.len() >= BATCH
            || (!self.waiting.is_empty()
                && self
                    .since
                    .is_some_and(|s| now.duration_since(s) >= BATCH_WAIT))
    }

    /// How long until the first one waiting is due; `None`: none waits.
    fn wait(&self, now: Instant) -> Option<Duration> {
        let since = self.since.filter(|_| !self.waiting.is_empty())?;
        Some(BATCH_WAIT.saturating_sub(now.duration_since(since)))
    }

    /// Up to [`BATCH`] of them, the first ones.
    fn take(&mut self, now: Instant) -> Vec<String> {
        let n = self.waiting.len().min(BATCH);
        let taken: Vec<String> = self.waiting.drain(..n).collect();
        self.since = (!self.waiting.is_empty()).then_some(now);
        taken
    }
}

/// Of `tokens`, those not asked of `identity` yet.
fn not_asked(
    asked: &HashMap<String, HashSet<String>>,
    identity: &str,
    tokens: &[String],
) -> Vec<String> {
    let done = asked.get(identity);
    tokens
        .iter()
        .filter(|t| done.is_none_or(|d| !d.contains(*t)))
        .cloned()
        .collect()
}

/// A wait of the pass, cut short by a stop.
async fn wait_live(wait: Duration, live: impl Fn() -> bool) {
    let until = tokio::time::Instant::now() + wait;
    while live() && tokio::time::Instant::now() < until {
        let left = until - tokio::time::Instant::now();
        tokio::time::sleep(left.min(Duration::from_secs(1))).await;
    }
}

/// Asks the identities' balances of the tokens found and follows the ones they hold.
struct Balances {
    chain: Arc<Chain>,
    file: Arc<WalletTokens>,
    /// The wallet's identities as of the last snapshot (C): read at each request.
    identities: Arc<Mutex<Vec<String>>>,
    facts: Arc<Mutex<super::tokens::KnownFacts>>,
    refresh: Arc<dyn Fn() + Send + Sync>,
    /// The contracts the pass proved, by token id, until their tokens are asked (dev2-4).
    contracts: HashMap<String, (TokenInfo, Vec<u8>)>,
    /// What was asked of each identity in this pass.
    asked: HashMap<String, HashSet<String>>,
    /// Balance requests made, for the journal.
    requests: u32,
    /// The first failure, said when the pass ends.
    failed: Option<String>,
}

impl Balances {
    /// Keep the proved contracts of a step, for following their tokens without reading them
    /// again; the token ids, to ask.
    fn keep(&mut self, found: chain::FoundTokens) -> Vec<String> {
        let bytes: HashMap<String, Vec<u8>> = found.contracts.into_iter().collect();
        let mut ids = Vec::new();
        for info in found.tokens {
            if let Some(b) = bytes.get(&info.contract_id) {
                self.contracts
                    .insert(info.token_id.clone(), (info.clone(), b.clone()));
            }
            ids.push(info.token_id);
        }
        ids
    }

    fn identities(&self) -> Vec<String> {
        self.identities
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `tokens` asked of `identities`, [`AT_ONCE`] at a time, what each was not asked yet;
    /// `each` after every group, with how many identities are done.
    /// Stops when `live` says so (a lock, another wallet; review r1, Should 1): before each group
    /// and after each answer, so the requests of the group under way are dropped with it.
    async fn ask(
        &mut self,
        tokens: &[String],
        identities: &[String],
        live: &(dyn Fn() -> bool + Sync),
        each: impl FnMut(usize),
    ) {
        let gap = Duration::from_millis(pace::GAP_MS);
        let query = move |chain: Arc<Chain>, identity: String, todo: Vec<String>| async move {
            chain.token_balances(&identity, &todo, gap).await
        };
        self.ask_with(tokens, identities, live, each, query).await;
    }

    /// [`Balances::ask`] with the balances' request given (a test asks no network).
    async fn ask_with<F>(
        &mut self,
        tokens: &[String],
        identities: &[String],
        live: &(dyn Fn() -> bool + Sync),
        mut each: impl FnMut(usize),
        query: impl Fn(Arc<Chain>, String, Vec<String>) -> F,
    ) where
        F: std::future::Future<Output = Result<HashMap<String, u64>, String>> + Send + 'static,
    {
        let mut done = 0;
        for group in identities.chunks(AT_ONCE) {
            if !live() {
                return;
            }
            let mut asking = tokio::task::JoinSet::new();
            for identity in group {
                let todo = not_asked(&self.asked, identity, tokens);
                if todo.is_empty() {
                    continue;
                }
                self.requests += todo.len().div_ceil(BATCH) as u32;
                let asked = query(self.chain.clone(), identity.clone(), todo.clone());
                let identity = identity.clone();
                asking.spawn(async move { (identity, todo, asked.await) });
            }
            while let Some(answer) = asking.join_next().await {
                // Dropped here, the set ends the requests still under way.
                if !live() {
                    return;
                }
                let Ok((identity, todo, held)) = answer else {
                    continue;
                };
                match held {
                    Ok(held) => {
                        self.asked.entry(identity.clone()).or_default().extend(todo);
                        for token in held.keys() {
                            match self.follow(&identity, token).await {
                                Ok(true) => self.show(),
                                Ok(false) => {}
                                Err(e) => {
                                    self.failed.get_or_insert(e);
                                }
                            }
                        }
                    }
                    // Not asked: the end of the pass asks it again.
                    Err(e) => {
                        self.failed.get_or_insert(e);
                    }
                }
            }
            done += group.len();
            each(done);
        }
    }

    /// Follow `token`, which `identity` holds, as "found", unless it is followed already or hidden
    /// on that identity: from the contract the pass proved (dev2-4), or read again when this pass
    /// did not read it. True when it was followed now.
    async fn follow(&self, identity: &str, token: &str) -> Result<bool, String> {
        if self.file.get(token).is_some() || self.file.is_hidden(token, identity) {
            return Ok(false);
        }
        let (info, bytes) = match self.contracts.get(token) {
            Some((info, bytes)) => (info.clone(), Some(bytes.clone())),
            None => {
                tokio::time::sleep(Duration::from_millis(pace::GAP_MS)).await;
                let found = self.chain.find_token(token).await?;
                let Some(info) = found.tokens.iter().find(|t| t.token_id == token).cloned() else {
                    return Ok(false);
                };
                let bytes = found
                    .contracts
                    .into_iter()
                    .find(|(c, _)| *c == info.contract_id)
                    .map(|(_, b)| b);
                (info, bytes)
            }
        };
        self.file
            .track(Tracked {
                token_id: info.token_id.clone(),
                contract_id: info.contract_id.clone(),
                position: info.position,
                origin: Origin::Found,
                contract: None,
            })
            .map_err(|e| e.to_string())?;
        if let Some(bytes) = bytes {
            self.file
                .set_contract(token, &bytes)
                .map_err(|e| e.to_string())?;
            // Its later proofs (balances, a transfer) are checked against it (АРХ-2).
            self.chain
                .register_kept_contracts(std::slice::from_ref(&bytes))
                .await;
        }
        Ok(true)
    }

    /// A token just followed: its row is read with the next snapshot, built now (B), as after
    /// "Add a token".
    fn show(&self) {
        super::tokens::make_due(&self.facts);
        (self.refresh)();
    }
}

impl Real {
    /// "Find my tokens" (`asked`), or the daily pass: one pass in the background, unless one is
    /// under way. Only the snapshots answer.
    pub(in crate::real) fn find_my_tokens(&self, asked: bool) -> (Option<Event>, bool) {
        let Some(file) = self.tokens.file.clone() else {
            return (None, false);
        };
        let search = &self.tokens.search;
        {
            let mut state = search.state.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(
                state.state,
                SearchState::Running | SearchState::Paused { .. }
            ) {
                return (None, false);
            }
            state.state = SearchState::Running;
            state.stage = SearchStage::Contracts;
            state.checked = 0;
            state.left_secs = None;
            state.failed = false;
        }
        let run_id = search.run.fetch_add(1, Ordering::Relaxed) + 1;
        let current = search.run.clone();
        let live = move || current.load(Ordering::Relaxed) == run_id;
        let (chain, state, path, balanced) = (
            self.chain.clone(),
            search.state.clone(),
            search.path.clone(),
            search.balanced.clone(),
        );
        let refresh = self.refresher();
        let mut balances = Balances {
            chain: chain.clone(),
            file,
            identities: self.tokens.identities_handle(),
            facts: self.tokens.facts_handle(),
            refresh: Arc::new(refresh.clone()),
            contracts: HashMap::new(),
            asked: HashMap::new(),
            requests: 0,
            failed: None,
        };
        let identities_found = self.identities_found.clone();
        tokio::spawn(async move {
            while !chain.platform_ready() {
                if !live() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            // G: the wallet's own lookup of its identities first; they share the nodes.
            let waited = Instant::now();
            while !identities_found.load(Ordering::Relaxed) && waited.elapsed() < IDENTITIES_WAIT {
                if !live() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            let mut cache: Cache<chain::CrawlTokens> = store::load(&path);
            let started = std::time::Instant::now();
            let now_ms = move || started.elapsed().as_millis() as u64;
            // A: the tokens of each step go to the balances while the contracts are read.
            let (found_tx, mut found_rx) = tokio::sync::mpsc::unbounded_channel();
            let worker = {
                let live = live.clone();
                tokio::spawn(async move {
                    let mut batch = Batch::default();
                    loop {
                        let wait = batch
                            .wait(Instant::now())
                            .unwrap_or(Duration::from_secs(60));
                        tokio::select! {
                            got = found_rx.recv() => match got {
                                Some(found) => {
                                    let ids = balances.keep(found);
                                    batch.add(ids, Instant::now());
                                }
                                None => break,
                            },
                            () = tokio::time::sleep(wait) => {}
                        }
                        while live() && batch.due(Instant::now()) {
                            let tokens = batch.take(Instant::now());
                            let identities = balances.identities();
                            balances.ask(&tokens, &identities, &live, |_| {}).await;
                            // Their contracts are not needed again in this pass.
                            for t in &tokens {
                                balances.contracts.remove(t);
                            }
                        }
                        if !live() {
                            break;
                        }
                    }
                    balances
                })
            };
            let mut stage: Option<run::Progress> = None;
            let ended = run::pass(
                &mut cache,
                LIMITS,
                pace::GAP_MS,
                |ask| {
                    let (chain, found_tx) = (chain.clone(), found_tx.clone());
                    async move {
                        match chain.crawl_step(&ask).await {
                            Ok((reply, found)) => {
                                if !found.tokens.is_empty() {
                                    let _ = found_tx.send(found);
                                }
                                Ok(reply)
                            }
                            Err(chain::CrawlFailure::Slow(wait)) => Err(run::Failure::Slow(wait)),
                            Err(chain::CrawlFailure::Failed(e)) => Err(run::Failure::Failed(e)),
                        }
                    }
                },
                now_ms,
                |wait| wait_live(wait, live.clone()),
                live.clone(),
                |p| {
                    // The journal: a line for each stage that ended.
                    if let Some(last) = stage.filter(|s| s.stage != p.stage) {
                        stage_line(&last);
                    }
                    stage = Some(p);
                    let paused = p
                        .paused_until
                        .map(|until| now() + (until.saturating_sub(now_ms()) / 1000) as Timestamp);
                    set(&state, |s| told(s, &p, paused));
                    refresh();
                },
                |c| {
                    let _ = store::save(&path, c);
                },
            )
            .await;
            if let Some(last) = &stage {
                stage_line(last);
            }
            drop(found_tx);
            let Ok(mut balances) = worker.await else {
                return;
            };
            let tokens: Vec<String> = cache
                .tokens
                .values()
                .flat_map(|k| k.tokens.clone())
                .collect();
            let total = cache.ids.len() as u32;
            let mut identities = Vec::new();
            let result = match &ended {
                // Every identity of the wallet now, over every token known: what the pass did
                // not ask yet (identities found meanwhile, failures, tokens of earlier passes).
                run::Ended::Done if live() => {
                    identities = balances.identities();
                    let of = identities.len() as u32;
                    let began = (Instant::now(), balances.requests);
                    set(&state, |s| {
                        s.stage = SearchStage::Balances {
                            identity: of.min(1),
                            of,
                        };
                        s.left_secs = None;
                    });
                    refresh();
                    balances
                        .ask(&tokens, &identities, &live, |done| {
                            set(&state, |s| {
                                s.stage = SearchStage::Balances {
                                    identity: (done as u32 + 1).min(of),
                                    of,
                                };
                            });
                            refresh();
                        })
                        .await;
                    diag::record(Record::TokenSearchStage {
                        stage: "balances",
                        requests: balances.requests - began.1,
                        ms: began.0.elapsed().as_millis() as u64,
                    });
                    balanced
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get_or_insert_with(HashSet::new)
                        .extend(identities.iter().cloned());
                    match balances.failed.take() {
                        Some(e) => Err(e),
                        None => Ok(()),
                    }
                }
                run::Ended::Done | run::Ended::Stopped => Ok(()),
                run::Ended::Failed(e) => Err(e.clone()),
            };
            if matches!(ended, run::Ended::Done) {
                diag::record(Record::TokenSearchEnd {
                    contracts: total,
                    tokens: tokens.len() as u32,
                    identities: identities.len() as u32,
                    ms: now_ms(),
                });
            }
            set(&state, |s| {
                s.stage = SearchStage::Contracts;
                s.left_secs = None;
                s.total = Some(total);
                match (&ended, result) {
                    (run::Ended::Done, Ok(())) if live() => {
                        s.state = SearchState::Idle;
                        s.done = Some((now(), total));
                        s.checked = total;
                    }
                    (run::Ended::Stopped, _) | (run::Ended::Done, _) if !live() => {
                        s.state = if s.done.is_some() {
                            SearchState::Idle
                        } else {
                            SearchState::NotYet
                        };
                    }
                    (_, _) => {
                        s.state = SearchState::NoNetwork;
                        // A failed pass in the background says nothing (TokenSearch::failed).
                        s.failed = asked;
                    }
                }
            });
            refresh();
        });
        (None, true)
    }

    /// C: identities of the wallet found after the last pass are asked over every token it knows,
    /// in the background; the snapshot shows what they hold.
    pub(in crate::real) fn balance_new_identities(&self, identities: Vec<String>) {
        let Some(file) = self.tokens.file.clone() else {
            return;
        };
        if identities.is_empty() {
            return;
        }
        let path = self.tokens.search.path.clone();
        let mut balances = Balances {
            chain: self.chain.clone(),
            file,
            identities: self.tokens.identities_handle(),
            facts: self.tokens.facts_handle(),
            refresh: Arc::new(self.refresher()),
            contracts: HashMap::new(),
            asked: HashMap::new(),
            requests: 0,
            failed: None,
        };
        let chain = self.chain.clone();
        // A lock, another wallet or a new pass stops it, as it stops the search (Should 1). It
        // asks beside a pass only rarely (it waits for `settled`): up to 6 requests a second to
        // the pool then, well below a node's 150 a minute, but outside the search's pace (r1 P3 2).
        let run = self.tokens.search.run.load(Ordering::Relaxed);
        let current = self.tokens.search.run.clone();
        let live = move || current.load(Ordering::Relaxed) == run;
        tokio::spawn(async move {
            if !chain.platform_ready() {
                return;
            }
            let cache: Cache<chain::CrawlTokens> = store::load(&path);
            let tokens: Vec<String> = cache
                .tokens
                .values()
                .flat_map(|k| k.tokens.clone())
                .collect();
            let began = Instant::now();
            balances.ask(&tokens, &identities, &live, |_| {}).await;
            diag::record(Record::TokenSearchStage {
                stage: "balances",
                requests: balances.requests,
                ms: began.elapsed().as_millis() as u64,
            });
        });
    }
}

/// The journal's line for a stage of the contracts that ended.
fn stage_line(p: &run::Progress) {
    diag::record(Record::TokenSearchStage {
        stage: p.stage.name(),
        requests: p.stage_requests,
        ms: p.stage_ms,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evolite-search-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_search_starts_where_the_last_pass_left_it() {
        let dir = root("start");
        let fresh = Search::on(true, &dir);
        assert_eq!(
            fresh.shown().state,
            SearchState::NotYet,
            "no pass on this network yet"
        );
        assert!(!fresh.due(), "the first pass waits for the tokens screen");
        let mut cache: Cache<chain::CrawlTokens> = Cache::default();
        cache.ids.insert([1; 32], Some(1));
        cache.ids.insert([2; 32], None);
        cache.passes = 1;
        store::save(&store::path(&dir), &cache).unwrap();
        let after = Search::on(true, &dir);
        let shown = after.shown();
        assert_eq!(shown.state, SearchState::Idle);
        assert_eq!(shown.done.map(|(_, n)| n), Some(2));
        assert!(!after.due(), "the pass was just now");
        set(&after.state, |s| s.done = Some((now() - EVERY_SECS, 2)));
        assert!(after.due(), "a day later");
        set(&after.state, |s| s.state = SearchState::Running);
        assert!(!after.due(), "not while one runs");
    }

    /// A: found tokens are asked together, 100 at once or 12 s after the first one waiting.
    #[test]
    fn found_tokens_wait_for_a_hundred_or_twelve_seconds() {
        let t0 = Instant::now();
        let mut batch = Batch::default();
        assert!(!batch.due(t0) && batch.wait(t0).is_none(), "nothing waits");
        batch.add((0..31).map(|i| format!("T{i}")), t0);
        assert!(!batch.due(t0 + Duration::from_secs(5)));
        assert_eq!(
            batch.wait(t0 + Duration::from_secs(5)),
            Some(Duration::from_secs(7))
        );
        assert!(batch.due(t0 + BATCH_WAIT), "12 s after the first");
        batch.add(
            (31..130).map(|i| format!("T{i}")),
            t0 + Duration::from_secs(6),
        );
        assert!(batch.due(t0 + Duration::from_secs(6)), "a hundred waiting");
        let first = batch.take(t0 + Duration::from_secs(6));
        assert_eq!(first.len(), BATCH);
        assert_eq!(first[0], "T0", "the first come first");
        // The rest wait from now.
        assert!(!batch.due(t0 + Duration::from_secs(7)));
        assert!(batch.due(t0 + Duration::from_secs(6) + BATCH_WAIT));
        assert_eq!(batch.take(t0 + Duration::from_secs(20)).len(), 30);
        assert!(batch.wait(t0 + Duration::from_secs(20)).is_none());
    }

    /// What an identity was asked in the pass is not asked again at its end.
    #[test]
    fn what_was_asked_of_an_identity_is_not_asked_again() {
        let mut asked: HashMap<String, HashSet<String>> = HashMap::new();
        asked
            .entry("I1".into())
            .or_default()
            .extend(["A".to_owned(), "B".to_owned()]);
        let all = ["A".to_owned(), "B".to_owned(), "C".to_owned()];
        assert_eq!(not_asked(&asked, "I1", &all), ["C"]);
        assert_eq!(
            not_asked(&asked, "I2", &all),
            ["A", "B", "C"],
            "found meanwhile: all"
        );
    }

    /// C: an identity found after the first snapshot is asked once; the ones there at the first
    /// snapshot were asked by the passes before.
    #[test]
    fn an_identity_found_later_is_asked_once() {
        let mut balanced = None;
        let first = ["I1".to_owned(), "I2".to_owned()];
        assert!(newly_seen(&mut balanced, &first).is_empty());
        assert!(newly_seen(&mut balanced, &first).is_empty());
        let later = ["I1".to_owned(), "I2".to_owned(), "I3".to_owned()];
        assert_eq!(newly_seen(&mut balanced, &later), ["I3"]);
        assert!(newly_seen(&mut balanced, &later).is_empty(), "once");
    }

    fn progress(requests: u32, ms: u64, left: Option<u32>) -> run::Progress {
        run::Progress {
            stage: run::Stage::Contracts,
            done: 0,
            of: None,
            requests_left: left,
            requests,
            ms,
            stage_requests: 0,
            stage_ms: 0,
            paused_until: None,
        }
    }

    /// F: a time only from the pass's own pace, after ten requests, and only when what is left
    /// can be counted: a first pass's pages promise nothing (BUG-132: «about a minute» held two).
    #[test]
    fn the_time_left_comes_from_the_pace_measured() {
        assert_eq!(left_secs(&progress(9, 9_000, Some(30))), None, "before ten");
        assert_eq!(
            left_secs(&progress(10, 25_000, None)),
            None,
            "pages: unknown"
        );
        // 2.5 s a request, 30 left: 75 s.
        assert_eq!(left_secs(&progress(10, 25_000, Some(30))), Some(75));
        assert_eq!(left_secs(&progress(20, 20_000, Some(0))), Some(0));
    }

    /// F: the screen is told the stage's count, and «of» only once it is known.
    #[test]
    fn the_screen_is_told_the_stage_and_its_count() {
        let mut s = TokenSearch::off();
        let p = run::Progress {
            stage: run::Stage::Pages,
            done: 1_800,
            ..progress(18, 40_000, None)
        };
        told(&mut s, &p, None);
        assert_eq!(
            (s.state.clone(), s.checked, s.total, s.left_secs),
            (SearchState::Running, 1_800, None, None)
        );
        let p = run::Progress {
            done: 4_190,
            of: Some(4_190),
            ..progress(42, 84_000, Some(0))
        };
        told(&mut s, &p, Some(99));
        assert_eq!((s.checked, s.total), (4_190, Some(4_190)));
        assert_eq!(s.state, SearchState::Paused { until: 99 });
        assert_eq!(s.stage, SearchStage::Contracts);
    }

    /// Review r1, Should 1: a lock (or another wallet) stops the balances under way: the group
    /// asked when it came finishes no further, and no other group is asked.
    #[tokio::test]
    async fn a_lock_stops_the_balances_under_way() {
        use std::sync::atomic::AtomicU32;
        let dir = root("balances-live");
        let mut balances = Balances {
            chain: Arc::new(Chain::default()),
            file: Arc::new(WalletTokens::new(dir.join("tokens.json"))),
            identities: Arc::default(),
            facts: Arc::default(),
            refresh: Arc::new(|| {}),
            contracts: HashMap::new(),
            asked: HashMap::new(),
            requests: 0,
            failed: None,
        };
        let asked = Arc::new(AtomicU32::new(0));
        let counted = asked.clone();
        let query = move |_: Arc<Chain>, _: String, _: Vec<String>| {
            counted.fetch_add(1, Ordering::Relaxed);
            async { Ok(HashMap::new()) }
        };
        let identities: Vec<String> = (1..=7).map(|i| format!("I{i}")).collect();
        let tokens = ["T".to_owned()];
        // Live until the first answers come.
        let seen = asked.clone();
        let live = move || seen.load(Ordering::Relaxed) == 0;
        balances
            .ask_with(&tokens, &identities, &live, |_| {}, query)
            .await;
        assert_eq!(
            asked.load(Ordering::Relaxed),
            AT_ONCE as u32,
            "one group, no more"
        );
        assert!(
            balances.asked.len() < AT_ONCE,
            "the answers after the lock are not kept: {:?}",
            balances.asked.keys()
        );
    }
}
