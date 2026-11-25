//! BUG-271 (the architect, 2026-11-22): the identity's own collections found on the network —
//! the contracts its identity nonces made ([`evolite_chain::Chain::find_own_contracts`]) — once
//! after the unlock for the identity shown, in the background, not on every Workshop frame.
//! «Checked up to nonce N» is kept by identity in `own-contracts.json` (public data: a number),
//! so the next opening reads only the new nonces and the last few again: a nonce may belong to
//! a transition not seen yet (BUG-243).
//!
//! r1 (TEAM.md §8d-2, -4, -5): the pass is said in the snapshot — searching, done or failed —
//! and a failed one is tried again by the shared rule (`evolite_api::retry`): after 30 s, then
//! 60, 120… up to 10 minutes, or at once on «Retry»; never on every turn of the core.

use std::collections::{BTreeMap, HashMap, HashSet};

use evolite_api::retry::Retry;
use evolite_api::workshop::{OwnSearch, WorkshopAsk, WorkshopRead};

use crate::real::*;

/// The last nonces read again each time: a registration not visible yet when they were read.
pub(in crate::real) const RECHECK: u64 = 3;

/// BUG-271 r2 (the architect's Should, §8d В3): a pass that neither answers nor fails is given
/// up after this, as the NFTs' read — failed, said with «Retry» — not «Looking…» to the end of
/// the run.
pub(in crate::real) const OWN_SCAN_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// A pass, at most `wait` long: past it, a failure.
pub(in crate::real) async fn within<F>(
    wait: std::time::Duration,
    pass: F,
) -> Result<(usize, u64), String>
where
    F: std::future::Future<Output = Result<(usize, u64), String>> + Send + 'static,
{
    crate::real::ops::names::read_within(wait, pass).await
}

/// This run's passes, by identity: each done once; a failed one waits its pause.
#[derive(Default)]
pub(in crate::real) struct OwnScan {
    tries: HashMap<String, Retry>,
    done: HashSet<String>,
}

impl OwnScan {
    /// Whether a pass for `identity` goes now (`now`: seconds): not done, not running, and
    /// never tried or its pause over. Marks it running when so.
    pub(in crate::real) fn start(&mut self, identity: &str, now: f64) -> bool {
        if self.done.contains(identity) {
            return false;
        }
        self.tries.entry(identity.to_owned()).or_default().ask(now)
    }

    /// The pass ended: read (done for this run), or failed (said, tried again after its pause).
    pub(in crate::real) fn finish(&mut self, identity: &str, read: bool, now: f64) {
        let r = self.tries.entry(identity.to_owned()).or_default();
        if read {
            r.answered();
            self.done.insert(identity.to_owned());
        } else {
            r.failed(now);
        }
    }

    /// «Retry» on the Workshop: a failed pass goes at the next turn.
    pub(in crate::real) fn again(&mut self, identity: &str, now: f64) {
        if let Some(r) = self.tries.get_mut(identity) {
            r.again(now);
        }
    }

    /// What the snapshot says of `identity`'s search.
    pub(in crate::real) fn status(&self, identity: &str) -> OwnSearch {
        if self.done.contains(identity) {
            return OwnSearch::Done;
        }
        match self.tries.get(identity) {
            Some(r) if r.is_failed() => OwnSearch::Failed,
            _ => OwnSearch::Searching,
        }
    }
}

/// Where a pass starts: after what was checked, less [`RECHECK`]; from 1 the first time.
pub(in crate::real) fn scan_from(checked: Option<u64>) -> u64 {
    checked.map_or(1, |n| n.saturating_sub(RECHECK).max(1))
}

/// The core's clock for the passes' pauses: wall seconds.
fn now_secs() -> f64 {
    evolite_common::wall_secs() as f64
}

impl Real {
    /// Run with each turn of the core: a pass for the identity shown, when the wallet is open,
    /// Platform can be asked and [`OwnScan::start`] says so. Its end answers the Workshop's list
    /// again, so a page open on an empty list fills — and a new snapshot says how it went.
    pub(in crate::real) fn own_contracts_tick(&mut self) {
        if self.unlocked.is_none() || !self.chain.platform_ready() {
            return;
        }
        let Some(identity) = self.tokens.shown_identity() else {
            return;
        };
        let started = self
            .own_scan
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start(&identity, now_secs());
        if !started {
            return;
        }
        let path = self.paths.own_contracts.clone();
        let (chain, scan, dirty) = (
            self.chain.clone(),
            self.own_scan.clone(),
            self.dirty.clone(),
        );
        self.answer_later("task:own_contracts", async move {
            let mut checked: BTreeMap<String, u64> =
                evolite_local::json_file::read_cache(&path, |b| serde_json::from_slice(b).ok())
                    // default-ok: a cache (BUG-271): unread, the scan starts over, set aside
                    .unwrap_or_default();
            let from = scan_from(checked.get(&identity).copied());
            let pass = {
                let (chain, identity) = (chain.clone(), identity.clone());
                async move { chain.find_own_contracts(&identity, from).await }
            };
            let found = within(OWN_SCAN_WAIT, pass).await;
            scan.lock().unwrap_or_else(|e| e.into_inner()).finish(
                &identity,
                found.is_ok(),
                now_secs(),
            );
            if let Ok((_, to)) = &found {
                checked.insert(identity.clone(), *to);
                let _ = evolite_local::json_file::save(&path, &checked);
            }
            // The snapshot says how it went.
            dirty.store(true, std::sync::atomic::Ordering::Relaxed);
            let result = chain
                .workshop_collections(&identity)
                .await
                .map(WorkshopRead::Collections)
                .map_err(CoreError::Failed);
            Event::Workshop {
                ask: WorkshopAsk::Collections,
                result,
            }
        });
    }

    /// «Retry» or a new read of the Workshop's list: a failed pass for `identity` goes now.
    pub(in crate::real) fn own_contracts_again(&self, identity: &str) {
        self.own_scan
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .again(identity, now_secs());
    }

    /// The snapshot's word on the shown identity's search.
    pub(in crate::real) fn own_search(&self) -> OwnSearch {
        match self.tokens.shown_identity() {
            Some(identity) => self
                .own_scan
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .status(&identity),
            None => OwnSearch::Done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-271: the first pass reads from nonce 1; the next ones only the new nonces and the
    /// last [`RECHECK`] again.
    #[test]
    fn a_later_pass_reads_only_the_new_nonces() {
        assert_eq!(scan_from(None), 1);
        assert_eq!(scan_from(Some(2)), 1);
        assert_eq!(scan_from(Some(40)), 37);
    }

    /// BUG-271 r1 (TEAM.md §8d-4, the class test): a pass that keeps failing is tried once a
    /// pause — a thousand turns of the core within it start nothing — and the snapshot says
    /// «failed»; «Retry» starts it at once; a pass that read is done for the run.
    #[test]
    fn a_failing_pass_is_tried_once_a_pause() {
        let mut scan = OwnScan::default();
        assert_eq!(scan.status("me"), OwnSearch::Searching);
        assert!(scan.start("me", 0.0));
        assert!(!scan.start("me", 0.2), "running");
        scan.finish("me", false, 1.0);
        assert_eq!(scan.status("me"), OwnSearch::Failed);
        let starts = (0..1_000)
            .filter(|i| scan.start("me", 1.0 + *i as f64 * 0.02))
            .count();
        assert_eq!(starts, 0, "a thousand turns within the pause");
        assert!(scan.start("me", 31.0), "after 30 s");
        scan.finish("me", false, 31.0);
        assert!(!scan.start("me", 80.0), "the next pause is 60 s");
        scan.again("me", 81.0);
        assert!(scan.start("me", 81.0), "«Retry» at once");
        scan.finish("me", true, 82.0);
        assert_eq!(scan.status("me"), OwnSearch::Done);
        assert!(!scan.start("me", 10_000.0), "done for the run");
    }

    /// BUG-271 r2: a pass whose answer never comes fails at its deadline — the snapshot says
    /// «failed», and «Retry» starts it again.
    #[tokio::test]
    async fn a_pass_that_never_answers_fails_at_its_deadline() {
        let mut scan = OwnScan::default();
        assert!(scan.start("me", 0.0));
        let never = std::future::pending::<Result<(usize, u64), String>>();
        let found = within(std::time::Duration::from_millis(50), never).await;
        assert!(found.is_err(), "given up");
        scan.finish("me", found.is_ok(), 1.0);
        assert_eq!(scan.status("me"), OwnSearch::Failed);
        scan.again("me", 2.0);
        assert!(scan.start("me", 2.0), "«Retry» starts it again");
    }
}
