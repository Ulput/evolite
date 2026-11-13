//! The wallet's own search of the network for its tokens (FEAT-0019 UC-1а, R-11): one task at a
//! time, off the core's loop (R-12). "Find my tokens" starts a pass, and so does a day passing
//! since the last one; a lock or another wallet stops it, and the next pass goes on from where it
//! stopped (`token-search.json`, shared by the network's wallets). After a whole pass, the
//! tokens the wallet's identities hold are followed as "found". Testnet only (R-9).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;

use evolite_api::tokens::{SearchState, TokenSearch};
use evolite_chain::tokens::crawl::{Cache, Limits};
use evolite_chain::tokens::{pace, run, store};
use evolite_local::wallet_tokens::{Origin, Tracked, WalletTokens};

use crate::real::*;

/// A page of ids and a batch of contracts or versions: Platform's most (card, АРХ-7).
const LIMITS: Limits = Limits {
    ids_page: 100,
    batch: 100,
};

/// A new pass in the background once a day while the wallet is open (card, «Обновление»).
const EVERY_SECS: Timestamp = 24 * 60 * 60;

/// The search: where it stands for the snapshot, and which run is the live one.
pub(in crate::real) struct Search {
    pub(in crate::real) state: Arc<Mutex<TokenSearch>>,
    run: Arc<AtomicU64>,
    path: PathBuf,
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
}

fn set(state: &Mutex<TokenSearch>, change: impl FnOnce(&mut TokenSearch)) {
    change(&mut state.lock().unwrap_or_else(|e| e.into_inner()));
}

/// A wait of the pass, cut short by a stop.
async fn wait_live(wait: Duration, live: impl Fn() -> bool) {
    let until = tokio::time::Instant::now() + wait;
    while live() && tokio::time::Instant::now() < until {
        let left = until - tokio::time::Instant::now();
        tokio::time::sleep(left.min(Duration::from_secs(1))).await;
    }
}

/// The tokens the wallet's `identities` hold, not followed yet and not hidden on the identity
/// that holds them, followed as "found" with their proved contracts.
async fn follow_found(
    chain: &Chain,
    file: &WalletTokens,
    identities: &[String],
    tokens: &[String],
    live: &(impl Fn() -> bool + Sync),
) -> Result<(), String> {
    let gap = Duration::from_millis(pace::GAP_MS);
    for identity in identities {
        if !live() {
            return Ok(());
        }
        let held = chain.token_balances(identity, tokens, gap).await?;
        for token in held.keys() {
            if file.get(token).is_some() || file.is_hidden(token, identity) {
                continue;
            }
            tokio::time::sleep(gap).await;
            let found = chain.find_token(token).await?;
            let Some(info) = found.tokens.iter().find(|t| t.token_id == *token) else {
                continue;
            };
            file.track(Tracked {
                token_id: info.token_id.clone(),
                contract_id: info.contract_id.clone(),
                position: info.position,
                origin: Origin::Found,
                contract: None,
            })
            .map_err(|e| e.to_string())?;
            if let Some((_, bytes)) = found.contracts.iter().find(|(c, _)| *c == info.contract_id) {
                file.set_contract(token, bytes).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
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
            state.checked = 0;
            state.failed = false;
        }
        let run_id = search.run.fetch_add(1, Ordering::Relaxed) + 1;
        let current = search.run.clone();
        let live = move || current.load(Ordering::Relaxed) == run_id;
        let (chain, state, path) = (
            self.chain.clone(),
            search.state.clone(),
            search.path.clone(),
        );
        let identities = self.tokens.identities();
        let refresh = self.refresher();
        tokio::spawn(async move {
            while !chain.platform_ready() {
                if !live() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            let mut cache: Cache<chain::CrawlTokens> = store::load(&path);
            let started = std::time::Instant::now();
            let now_ms = move || started.elapsed().as_millis() as u64;
            let ended = run::pass(
                &mut cache,
                LIMITS,
                pace::GAP_MS,
                |ask| {
                    let chain = chain.clone();
                    async move {
                        chain.crawl_step(&ask).await.map_err(|e| match e {
                            chain::CrawlFailure::Slow(wait) => run::Failure::Slow(wait),
                            chain::CrawlFailure::Failed(e) => run::Failure::Failed(e),
                        })
                    }
                },
                now_ms,
                |wait| wait_live(wait, live.clone()),
                live.clone(),
                |p| {
                    set(&state, |s| {
                        s.checked = p.ids;
                        s.state = match p.paused_until {
                            Some(until) => SearchState::Paused {
                                until: now() + (until.saturating_sub(now_ms()) / 1000) as Timestamp,
                            },
                            None => SearchState::Running,
                        };
                        // About one request a second for what is left to read.
                        s.left_secs = Some(p.left.div_ceil(LIMITS.batch as u32));
                    });
                    refresh();
                },
                |c| {
                    let _ = store::save(&path, c);
                },
            )
            .await;
            let tokens: Vec<String> = cache
                .tokens
                .values()
                .flat_map(|k| k.tokens.clone())
                .collect();
            let total = cache.ids.len() as u32;
            let result = match &ended {
                run::Ended::Done => follow_found(&chain, &file, &identities, &tokens, &live).await,
                run::Ended::Stopped => Ok(()),
                run::Ended::Failed(e) => Err(e.clone()),
            };
            set(&state, |s| {
                s.left_secs = None;
                s.total = Some(total);
                match (&ended, result) {
                    (run::Ended::Done, Ok(())) => {
                        s.state = SearchState::Idle;
                        s.done = Some((now(), total));
                        s.checked = total;
                    }
                    (run::Ended::Stopped, _) => {
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
}
