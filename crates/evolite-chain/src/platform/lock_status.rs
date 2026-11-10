//! Asks Platform, with a proof, whether an asset lock was used (FEAT-0016): the outpoint in
//! Drive's tree of spent locks. Read only: nothing is sent or signed here, and an answer is never
//! guessed. A key absent from a proved answer is proved absent; an error says nothing.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dash_sdk::drive::grovedb::Element;
use dash_sdk::platform::{Fetch, FetchMany};
use dash_sdk::query_types::KeysInPath;
use dpp::asset_lock::reduced_asset_lock_value::{AssetLockValue, AssetLockValueGettersV0};
use dpp::identity::Identity;
use dpp::prelude::Identifier;
use dpp::serialization::PlatformDeserializableTrusted;
use evolite_api::{Credits, Duffs};
use key_wallet::dashcore::{OutPoint, Txid};

use crate::chain::Chain;

/// `RootTree::SpentAssetLockTransactions`.
const SPENT_LOCKS_TREE: u8 = 72;
/// Outpoints in one request: 10 were tried on testnet, Drive's real limit is not known.
pub const BATCH: usize = 16;
/// How soon a request that failed may be made again (R-5).
pub const RETRY: Duration = Duration::from_secs(10 * 60);

/// What Platform proved about a lock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Answer {
    Spent,
    /// Part of it was used; `left` is still in the lock.
    Partly {
        left: Duffs,
    },
    /// Proved absent: Platform has not seen it.
    NotSeen,
}

/// The 36 bytes Drive keys a lock by: the txid in its raw order, then the output's index,
/// little-endian (R-8). A lock's credits are always its output 0.
pub fn outpoint_key(txid: &str) -> Option<[u8; 36]> {
    let txid: Txid = txid.parse().ok()?;
    Some(OutPoint::new(txid, 0).into())
}

/// What a proved element says (AC-T1): none — not seen; an empty item — spent; a value — what is
/// left of it. Anything else is an error, not an answer (R-5).
pub fn read_element(element: Option<&Element>) -> Result<Answer, String> {
    let Some(element) = element else {
        return Ok(Answer::NotSeen);
    };
    let bytes = element.as_item_bytes().map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Ok(Answer::Spent);
    }
    let value = AssetLockValue::deserialize_from_bytes_trusted(bytes).map_err(|e| e.to_string())?;
    Ok(answer_of_remaining(value.remaining_credit_value()))
}

fn answer_of_remaining(credits: u64) -> Answer {
    match credits {
        0 => Answer::Spent,
        credits => Answer::Partly {
            left: crate::identity::credits_as_duffs(credits),
        },
    }
}

/// The table of R-3. `identity`: an identity with the id from this outpoint exists (proved).
/// `unfinished`: Identity offers «Continue» for this lock. A top-up is never `Added` from the
/// network (R-4): Platform does not record who got its credits.
pub fn credits_of(answer: Answer, creates: bool, identity: bool, unfinished: bool) -> Credits {
    match answer {
        Answer::Spent if creates && identity => Credits::Added,
        Answer::Spent => Credits::Used,
        Answer::Partly { left } => Credits::UsedPartly { left },
        Answer::NotSeen if unfinished => Credits::NotAdded,
        Answer::NotSeen => Credits::NotReceived,
    }
}

/// One lock to ask about: its txid and whether it registers an identity.
pub type Lock = (String, bool);

/// One answer: whether an identity with the id from the outpoint exists, for a registration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Told {
    pub answer: Answer,
    pub identity: bool,
}

/// The answers of a request, by lock; a lock that cannot be read has an error of its own.
pub async fn read(
    chain: &Chain,
    locks: &[Lock],
) -> Result<Vec<(Lock, Result<Told, String>)>, String> {
    chain.ready()?;
    let sdk = chain.sdk().await?;
    let keyed: Vec<(&Lock, [u8; 36])> = locks
        .iter()
        .filter_map(|lock| Some((lock, outpoint_key(&lock.0)?)))
        .collect();
    let query = KeysInPath {
        path: vec![vec![SPENT_LOCKS_TREE]],
        keys: keyed.iter().map(|(_, key)| key.to_vec()).collect(),
    };
    let found = Element::fetch_many(&sdk, query)
        .await
        .map_err(|e| e.to_string())?;
    let mut told = Vec::with_capacity(keyed.len());
    for (lock, key) in keyed {
        let element = found.get(key.as_slice()).cloned().flatten();
        let answer = read_element(element.as_ref());
        let result = match answer {
            Ok(Answer::Spent) if lock.1 => {
                let id = Identifier::new(dpp::util::hash::hash_double(key));
                Identity::fetch(&sdk, id)
                    .await
                    .map(|identity| Told {
                        answer: Answer::Spent,
                        identity: identity.is_some(),
                    })
                    .map_err(|e| e.to_string())
            }
            Ok(answer) => Ok(Told {
                answer,
                identity: false,
            }),
            Err(e) => Err(e),
        };
        told.push((lock.clone(), result));
    }
    Ok(told)
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct File {
    /// The txids Platform proved spent; true: an identity from its outpoint exists.
    #[serde(default)]
    spent: HashMap<String, bool>,
}

#[derive(Default)]
struct State {
    /// `None` until the file is read.
    spent: Option<HashMap<String, bool>>,
    /// This run's answers that are not for good (R-7): not seen, or used only in part.
    this_run: HashMap<String, Answer>,
    /// Asked in this run, or being asked (R-1).
    asked: HashSet<String>,
    /// The locks the last snapshot had no answer for.
    wanted: Vec<Lock>,
    /// A request failed: not again before this (R-5).
    not_before: Option<Instant>,
}

/// What Platform answered about the wallet's locks. «Spent» does not change and is kept in
/// `lock-status.json` in the wallet's folder; the rest is asked again next run (R-7).
pub struct LockStatuses {
    path: PathBuf,
    state: Mutex<State>,
}

impl LockStatuses {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn load(&self, state: &mut State) {
        if state.spent.is_none() {
            let file: File = crate::json_file::read(&self.path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            state.spent = Some(file.spent);
        }
    }

    /// What the line says, when Platform has answered about it.
    pub fn credits(&self, txid: &str, creates: bool, unfinished: bool) -> Option<Credits> {
        let mut state = self.state();
        self.load(&mut state);
        if let Some(identity) = state.spent.as_ref().and_then(|s| s.get(txid)) {
            return Some(credits_of(Answer::Spent, creates, *identity, unfinished));
        }
        state
            .this_run
            .get(txid)
            .map(|answer| credits_of(*answer, creates, false, unfinished))
    }

    /// The locks still without an answer, as the snapshot found them.
    pub fn want(&self, locks: Vec<Lock>) {
        self.state().wanted = locks;
    }

    /// The next request to make at `now`, if one is due: locks not asked in this run, marked as
    /// asked.
    pub fn next_batch(&self, now: Instant) -> Vec<Lock> {
        let mut state = self.state();
        if state.not_before.is_some_and(|at| now < at) {
            return Vec::new();
        }
        let batch: Vec<Lock> = state
            .wanted
            .iter()
            .filter(|(txid, _)| !state.asked.contains(txid))
            .take(BATCH)
            .cloned()
            .collect();
        for (txid, _) in &batch {
            state.asked.insert(txid.clone());
        }
        batch
    }

    /// Keeps what Platform proved.
    pub fn answered(&self, txid: &str, told: Told) {
        let mut state = self.state();
        self.load(&mut state);
        match told.answer {
            Answer::Spent => {
                let spent = state.spent.get_or_insert_default();
                spent.insert(txid.to_owned(), told.identity);
                let file = File {
                    spent: spent.clone(),
                };
                // A file that cannot be written only asks again next run.
                let _ = crate::json_file::save(&self.path, &file);
            }
            answer => {
                state.this_run.insert(txid.to_owned(), answer);
            }
        }
    }

    /// A request, or one lock of it, failed at `now` (R-5): nothing is concluded; these locks
    /// are asked again after [`RETRY`].
    pub fn failed(&self, locks: &[Lock], now: Instant) {
        let mut state = self.state();
        for (txid, _) in locks {
            state.asked.remove(txid);
        }
        state.not_before = Some(now + RETRY);
    }
}

/// Whether Platform is asked about a lock with these credits (R-1 and the review's Should 1):
/// one nothing is known of, and one not added, which may have been used from another device
/// since. Its «Continue» then goes; a proved «not seen» keeps it.
pub fn worth_asking(credits: Credits) -> bool {
    matches!(credits, Credits::Unknown | Credits::NotAdded)
}

/// Whether a lock has a Core proof to ask about (R-1): an InstantLock or a ChainLock, or a block
/// under the last ChainLock, which the snapshot's later step turns into one.
pub fn core_proved(
    proof: evolite_api::Proof,
    block: Option<u32>,
    chainlocked: Option<u32>,
) -> bool {
    use evolite_api::Proof;
    match proof {
        Proof::InstantLock | Proof::ChainLock => true,
        Proof::InBlock => matches!((block, chainlocked), (Some(b), Some(l)) if b <= l),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::Proof;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evolite-lock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("lock-status.json")
    }

    fn lock(id: &str) -> Lock {
        (id.to_owned(), false)
    }

    fn txid(n: usize) -> String {
        format!("{n:064x}")
    }

    /// R-8: a real lock Platform proved spent, in the order the chain and Drive use. A reversed
    /// txid would give a proved «not seen».
    #[test]
    fn the_key_is_the_raw_txid_then_the_output() {
        let shown = "7bc7dc8603948607f4a085569f0b0352fe982aceb4aa3a0f056cfe220c01ffc7";
        let key = outpoint_key(shown).expect("a txid");
        let raw: Vec<u8> = (0..64)
            .step_by(2)
            .map(|i| u8::from_str_radix(&shown[i..i + 2], 16).unwrap())
            .rev()
            .collect();
        assert_eq!(&key[..32], raw.as_slice());
        assert_eq!(&key[32..], &[0, 0, 0, 0]);
        assert_eq!(outpoint_key("not a txid"), None);
    }

    /// AC-T1: what the proof's element means; what is not an item is no answer.
    #[test]
    fn an_element_says_spent_or_nothing() {
        assert_eq!(read_element(None), Ok(Answer::NotSeen));
        let empty = Element::new_item(Vec::new());
        assert_eq!(read_element(Some(&empty)), Ok(Answer::Spent));
        let tree = Element::empty_tree();
        assert!(read_element(Some(&tree)).is_err(), "not an item");
        let junk = Element::new_item(vec![9, 9, 9]);
        assert!(read_element(Some(&junk)).is_err(), "cannot be read");
    }

    /// AC-T1: what is left of a lock; nothing left is spent.
    #[test]
    fn nothing_left_is_spent() {
        assert_eq!(answer_of_remaining(0), Answer::Spent);
        assert_eq!(
            answer_of_remaining(5_000_000),
            Answer::Partly {
                left: crate::identity::credits_as_duffs(5_000_000)
            }
        );
    }

    /// R-3, AC-T6: the table. Only a registration with its identity proved says «added».
    #[test]
    fn the_answers_follow_the_table() {
        assert_eq!(credits_of(Answer::Spent, true, true, false), Credits::Added);
        assert_eq!(credits_of(Answer::Spent, true, false, false), Credits::Used);
        assert_eq!(
            credits_of(Answer::Spent, false, true, false),
            Credits::Used,
            "a top-up is never added by the network"
        );
        for (creates, identity) in [(false, false), (false, true), (true, true)] {
            assert_eq!(
                credits_of(Answer::Partly { left: 7 }, creates, identity, false),
                Credits::UsedPartly { left: 7 }
            );
        }
        assert_eq!(
            credits_of(Answer::NotSeen, false, false, false),
            Credits::NotReceived
        );
        assert_eq!(
            credits_of(Answer::NotSeen, false, false, true),
            Credits::NotAdded,
            "Identity offers to continue it"
        );
    }

    /// Should 1 (review r1): a lock not added is asked too; one added, or already answered,
    /// is not. Spent elsewhere, it is used; not seen, Identity still offers to continue it.
    #[test]
    fn a_lock_not_added_is_asked_too() {
        assert!(worth_asking(Credits::Unknown));
        assert!(worth_asking(Credits::NotAdded));
        for done in [
            Credits::Added,
            Credits::Used,
            Credits::UsedPartly { left: 1 },
            Credits::NotReceived,
        ] {
            assert!(!worth_asking(done), "{done:?}");
        }
        assert_eq!(credits_of(Answer::Spent, false, false, true), Credits::Used);
        assert_eq!(
            credits_of(Answer::NotSeen, false, false, true),
            Credits::NotAdded
        );
    }

    /// R-1: only a lock with a Core proof.
    #[test]
    fn a_lock_without_a_core_proof_is_not_asked() {
        assert!(core_proved(Proof::InstantLock, None, None));
        assert!(core_proved(Proof::ChainLock, Some(5), None));
        assert!(core_proved(Proof::InBlock, Some(5), Some(5)));
        assert!(!core_proved(Proof::InBlock, Some(6), Some(5)));
        assert!(!core_proved(Proof::InBlock, Some(6), None));
        assert!(!core_proved(Proof::Mempool, None, Some(5)));
    }

    /// R-1: at most a batch at a time, and a lock once a run.
    #[test]
    fn locks_are_asked_once_in_batches() {
        let status = LockStatuses::new(folder("batches"));
        let now = Instant::now();
        status.want((0..BATCH + 2).map(|n| lock(&txid(n))).collect());
        assert_eq!(status.next_batch(now).len(), BATCH);
        assert_eq!(status.next_batch(now).len(), 2);
        assert!(status.next_batch(now).is_empty(), "all were asked");
    }

    /// AC-T3: an error is no answer, and the next try waits ten minutes.
    #[test]
    fn a_failure_says_nothing_and_waits() {
        let status = LockStatuses::new(folder("failure"));
        let now = Instant::now();
        status.want(vec![lock("a")]);
        let batch = status.next_batch(now);
        status.failed(&batch, now);
        assert_eq!(status.credits("a", false, false), None, "still unknown");
        assert!(
            status
                .next_batch(now + RETRY - Duration::from_secs(1))
                .is_empty()
        );
        assert_eq!(status.next_batch(now + RETRY), batch);
    }

    /// AC-T5, R-7: «spent» outlives the run; «not seen» and a partly used lock do not.
    #[test]
    fn only_spent_is_kept() {
        let path = folder("kept");
        let status = LockStatuses::new(path.clone());
        let told = |answer| Told {
            answer,
            identity: false,
        };
        status.answered("spent", told(Answer::Spent));
        status.answered("unseen", told(Answer::NotSeen));
        status.answered("part", told(Answer::Partly { left: 3 }));
        assert_eq!(
            status.credits("unseen", false, false),
            Some(Credits::NotReceived)
        );
        assert_eq!(
            status.credits("part", false, false),
            Some(Credits::UsedPartly { left: 3 })
        );
        let next = LockStatuses::new(path.clone());
        assert_eq!(next.credits("spent", false, false), Some(Credits::Used));
        assert_eq!(next.credits("unseen", false, false), None);
        assert_eq!(next.credits("part", false, false), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A registration's proved identity is kept with «spent»: it stays «added» after a restart.
    #[test]
    fn a_registrations_identity_is_kept() {
        let path = folder("identity");
        LockStatuses::new(path.clone()).answered(
            "made",
            Told {
                answer: Answer::Spent,
                identity: true,
            },
        );
        let next = LockStatuses::new(path.clone());
        assert_eq!(next.credits("made", true, false), Some(Credits::Added));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// AC-T4, R-6: this module only reads. Names built in parts, so this test does not match.
    #[test]
    fn the_module_sends_nothing() {
        let source = include_str!("lock_status.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        for word in [
            ["broad", "cast"].concat(),
            ["put_to", "_platform"].concat(),
            ["resume", "_"].concat(),
            ["top_", "up"].concat(),
            ["sign", "er"].concat(),
        ] {
            assert!(!code.contains(&word), "{word}");
        }
    }
}
