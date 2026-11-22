//! The NFTs of the wallet's identities (FEAT-0055), each identity's list asked for in the
//! background and kept; a snapshot shows the last ones found and asks again after [`REFRESH`].
//! FEAT-0054: they are the instances the identity holds in collections of the evolite standard
//! ([`crate::chain::Chain::standard_nfts`]); no partner contract is read any more.

use evolite_api::{Nft, NftsStatus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often the list is asked for again while the wallet is open.
const REFRESH: Duration = Duration::from_secs(60);
/// How long an ask may take: Platform that does not answer is a failure to show with its
/// reason, never an endless "Loading" (BUG-10); the list is asked for again after [`REFRESH`].
pub const ASK_TIMEOUT: Duration = Duration::from_secs(60);
/// An ask still "running" this long after it started is taken as lost (its task ended
/// without an answer): the next snapshot asks again.
const ASK_LOST: Duration = Duration::from_secs(90);
/// FEAT-0054 (TEAM.md §8d-4): «Try again» asks no more often than this after the last ask.
pub const RETRY_MIN: Duration = Duration::from_secs(30);

/// Why the list did not come: Platform gave no answer within [`ASK_TIMEOUT`].
pub fn no_answer() -> &'static str {
    tr!("Dash Platform did not answer within a minute")
}

/// One identity's list and its asks.
#[derive(Default)]
struct List {
    nfts: Vec<Nft>,
    status: NftsStatus,
    asked: Option<Instant>,
    running: bool,
}

#[derive(Default)]
struct State {
    /// FEAT-0055: every identity's own list, by its id (base58).
    lists: HashMap<String, List>,
}

/// The NFTs of the wallet's identities, each identity's list kept between snapshots
/// (FEAT-0055: the Collection shows them all together).
#[derive(Clone, Default)]
pub struct Nfts(Arc<Mutex<State>>);

impl Nfts {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The lists of `owners` (base58, in this order) together, where they stand as one, and each
    /// one's own state (FEAT-0055 r1). As one: a failed list says so (what it found before
    /// stays), else one still on its way is «Loading» (what the others found is shown
    /// meanwhile), else «Ready». No owners: no identity. An owner never asked for is on its way.
    /// The lists of identities not in `owners` are dropped (r1: another or a deleted wallet's
    /// NFTs do not stay in memory); their late answers are dropped too ([`Self::found`]).
    pub fn shown(&self, owners: &[String]) -> (Vec<Nft>, NftsStatus, Vec<(String, NftsStatus)>) {
        let mut state = self.state();
        state.lists.retain(|owner, _| owners.contains(owner));
        if owners.is_empty() {
            return (Vec::new(), NftsStatus::NoIdentity, Vec::new());
        }
        let mut nfts = Vec::new();
        let mut each = Vec::new();
        for owner in owners {
            let status = match state.lists.get(owner) {
                Some(list) => {
                    nfts.extend(list.nfts.iter().cloned());
                    list.status.clone()
                }
                None => NftsStatus::Loading,
            };
            each.push((owner.clone(), status));
        }
        let failed = each.iter().find_map(|(_, s)| match s {
            NftsStatus::Failed(why) => Some(why.clone()),
            _ => None,
        });
        let status = match failed {
            Some(why) => NftsStatus::Failed(why),
            None if each.iter().any(|(_, s)| *s == NftsStatus::Loading) => NftsStatus::Loading,
            None => NftsStatus::Ready,
        };
        (nfts, status, each)
    }

    /// Should the list of `owner` be asked for now? Marks the ask as running when so.
    pub fn due(&self, owner: &str) -> bool {
        let mut state = self.state();
        let list = state.lists.entry(owner.into()).or_insert_with(|| List {
            status: NftsStatus::Loading,
            ..List::default()
        });
        let due = match list.asked {
            None => true,
            Some(at) if list.running => {
                let lost = at.elapsed() >= ASK_LOST;
                if lost && matches!(list.status, NftsStatus::Loading) {
                    list.status = NftsStatus::Failed(no_answer().into());
                }
                lost
            }
            Some(at) => at.elapsed() >= REFRESH,
        };
        if due {
            list.running = true;
            list.asked = Some(Instant::now());
        }
        due
    }

    /// «Try again»: the next [`Self::due`] of every list asks at once, unless its ask runs now.
    /// A list that failed and has nothing saved shows as loading again until the answer.
    pub fn ask_now(&self) {
        let mut state = self.state();
        for list in state.lists.values_mut() {
            if list.running {
                continue;
            }
            // FEAT-0054: at once if the last ask is RETRY_MIN old, else as soon as it is.
            list.asked = list.asked.and_then(|at| {
                let wait = RETRY_MIN.checked_sub(at.elapsed())?;
                Instant::now().checked_sub(REFRESH.checked_sub(wait)?)
            });
            if list.nfts.is_empty() && matches!(list.status, NftsStatus::Failed(_)) {
                list.status = NftsStatus::Loading;
            }
        }
    }

    /// The answer to an ask for `owner`: the NFTs it holds (FEAT-0054: the evolite standard's,
    /// [`crate::chain::Chain::standard_nfts`]).
    pub fn found(&self, owner: &str, result: Result<Vec<Nft>, String>) {
        let mut state = self.state();
        // An answer for a list never asked for (or dropped): nothing to put it in.
        let Some(list) = state.lists.get_mut(owner) else {
            return;
        };
        list.running = false;
        match result {
            Ok(nfts) => {
                list.nfts = nfts;
                list.status = NftsStatus::Ready;
            }
            Err(e) => list.status = NftsStatus::Failed(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nft(id: &str) -> Nft {
        Nft {
            id: id.into(),
            name: "Pin".into(),
            description: String::new(),
            art: "k".into(),
            art_verified: true,
            origin: evolite_api::NftOrigin::Release,
            issuer: "issuer".into(),
            contract: "collection".into(),
            edition: (Some(1), 10),
            owner: "@o".into(),
            owner_id: "o".into(),
            chosen: false,
            price: None,
        }
    }

    /// BUG-10: an ask that never answers does not leave the Collection on "Loading" for good.
    #[test]
    fn a_lost_ask_fails_and_is_asked_again() {
        let nfts = Nfts::default();
        let owner = "owner";
        let shown = || nfts.shown(&[owner.to_string()]).1;
        assert!(nfts.due(owner), "the first ask");
        assert!(!nfts.due(owner), "not twice while it runs");
        assert_eq!(shown(), NftsStatus::Loading);
        let past = |d: Duration| Instant::now().checked_sub(d).unwrap();
        nfts.state().lists.get_mut(owner).unwrap().asked =
            Some(past(ASK_LOST - Duration::from_secs(1)));
        assert!(!nfts.due(owner), "still within its time");
        nfts.state().lists.get_mut(owner).unwrap().asked = Some(past(ASK_LOST));
        assert!(nfts.due(owner), "lost: asked again");
        assert_eq!(
            shown(),
            NftsStatus::Failed(no_answer().into()),
            "and said so"
        );
        // The new ask answers: an empty list is "no NFTs yet", not loading.
        nfts.found(owner, Ok(Vec::new()));
        assert_eq!(shown(), NftsStatus::Ready);
        assert!(!nfts.due(owner), "the next ask after REFRESH");
        nfts.state().lists.get_mut(owner).unwrap().asked = Some(past(REFRESH));
        assert!(nfts.due(owner));
        nfts.found(owner, Err(no_answer().into()));
        assert_eq!(shown(), NftsStatus::Failed(no_answer().into()));
        assert!(!nfts.due(owner));
    }

    /// CS-1a: «Try again» asks at once, not after REFRESH; never a second ask beside a running
    /// one; a failed list with nothing saved shows as loading until the answer.
    #[test]
    fn try_again_asks_at_once_and_only_once() {
        let nfts = Nfts::default();
        let owner = "owner";
        let shown = || nfts.shown(&[owner.to_string()]);
        assert!(nfts.due(owner));
        nfts.ask_now();
        assert!(!nfts.due(owner), "an ask runs: none beside it");
        nfts.found(owner, Err(no_answer().into()));
        assert!(!nfts.due(owner), "the next one after REFRESH");
        // FEAT-0054: not again within RETRY_MIN of the last ask…
        nfts.ask_now();
        assert!(!nfts.due(owner), "not within 30 s of the last ask");
        // …at once when it is that old.
        let past = |d: Duration| Instant::now().checked_sub(d).unwrap();
        nfts.state().lists.get_mut(owner).unwrap().asked = Some(past(RETRY_MIN));
        nfts.ask_now();
        assert_eq!(shown().1, NftsStatus::Loading);
        assert!(nfts.due(owner), "asked again now");
        assert!(!nfts.due(owner), "once");
        // A saved list stays and says it failed until the answer.
        nfts.found(owner, Ok(vec![nft("a")]));
        assert_eq!(shown().0.len(), 1);
        nfts.state().lists.get_mut(owner).unwrap().status = NftsStatus::Failed("timeout".into());
        nfts.state().lists.get_mut(owner).unwrap().asked = Some(past(RETRY_MIN));
        nfts.ask_now();
        assert_eq!(shown().1, NftsStatus::Failed("timeout".into()));
        assert_eq!(shown().0.len(), 1);
    }

    /// FEAT-0055: two identities' asks run side by side; each answer goes into its own list,
    /// the Collection shows both together, and each one's own state (r1).
    #[test]
    fn each_identity_has_its_own_list_and_the_collection_shows_them_together() {
        let nfts = Nfts::default();
        let both = ["alice".to_string(), "bob".to_string()];
        assert_eq!(nfts.shown(&[]).1, NftsStatus::NoIdentity);
        assert!(nfts.due("alice"));
        assert!(nfts.due("bob"), "the second ask beside the first");
        nfts.found("alice", Ok(vec![nft("a")]));
        assert!(!nfts.due("bob"));
        let (list, status, _) = nfts.shown(&both);
        assert_eq!((list.len(), status), (1, NftsStatus::Loading));
        nfts.found("bob", Ok(vec![nft("b")]));
        let (list, status, _) = nfts.shown(&both);
        assert_eq!((list.len(), status), (2, NftsStatus::Ready));
        assert_eq!(list[1].id, "b");
        // A failed list says so for the whole, what it found before stays.
        nfts.state().lists.get_mut("bob").unwrap().asked = None;
        assert!(nfts.due("bob"));
        nfts.found("bob", Err("network".into()));
        let (list, status, each) = nfts.shown(&both);
        assert_eq!(
            (list.len(), status),
            (2, NftsStatus::Failed("network".into()))
        );
        // r1: under the first one's chip its own state — ready, not the other's failure.
        assert_eq!(
            each,
            vec![
                ("alice".to_string(), NftsStatus::Ready),
                ("bob".to_string(), NftsStatus::Failed("network".into())),
            ]
        );
        // An identity never asked for is on its way, not «no NFTs».
        let three = ["alice".to_string(), "carol".to_string()];
        assert_eq!(nfts.shown(&three).1, NftsStatus::Loading);
        // An answer for a list never asked for is dropped.
        nfts.found("dave", Ok(vec![nft("d")]));
        assert!(nfts.shown(&["dave".to_string()]).0.is_empty());
    }

    /// FEAT-0055 r1 (TEAM.md §8d В1): another wallet opens — its identities are other ones; the
    /// first wallet's lists are dropped from memory, and a late answer for them is too.
    #[test]
    fn another_wallets_identities_drop_the_first_ones_lists() {
        let nfts = Nfts::default();
        let first = ["alice".to_string(), "bob".to_string()];
        assert!(nfts.due("alice"));
        assert!(nfts.due("bob"));
        nfts.found("alice", Ok(vec![nft("a")]));
        assert_eq!(nfts.shown(&first).0.len(), 1);
        let other = ["carol".to_string()];
        let (list, status, each) = nfts.shown(&other);
        assert!(list.is_empty(), "none of the first wallet's NFTs");
        assert_eq!(status, NftsStatus::Loading);
        assert_eq!(each, vec![("carol".to_string(), NftsStatus::Loading)]);
        assert!(!nfts.state().lists.contains_key("alice"));
        nfts.found("bob", Ok(vec![nft("b")]));
        assert!(!nfts.state().lists.contains_key("bob"));
        nfts.shown(&[]);
        assert!(nfts.state().lists.is_empty());
    }
}
