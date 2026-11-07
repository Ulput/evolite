//! Payments seen in the mempool and not in a block yet: when each was first seen, and
//! at which height, kept in `mempool-seen.json` beside the store. Without it the time came from
//! this run alone, so a payment the network never took stayed "In the mempool · just now" at
//! every start. One still out of a block [`STALE_AFTER`] blocks later is shown as not confirmed
//! by the network: it may have been replaced or dropped, and it is never resent.

use evolite_api::{Activity, Proof, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// About half an hour of blocks: InstantSend takes seconds, a block 2,5 minutes.
pub const STALE_AFTER: u32 = 12;

/// The same by the clock (a block is 2,5 minutes), only before the chain has a height and for
/// a sighting kept with none (BUG-82). Once both heights are known the chain decides, not this
/// machine's clock: half an hour between two blocks is not a dropped payment (review r1
/// should 2).
const STALE_SECS: Timestamp = STALE_AFTER as Timestamp * 150;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Seen {
    at: Timestamp,
    height: u32,
    /// A rescan for its staleness ran to its end (BUG-64 review r1 should 1, r2 should 1):
    /// never again. One cut short (the app closed mid-way) is asked for at the next start.
    #[serde(default)]
    rescanned: bool,
}

pub struct MempoolSeen {
    path: PathBuf,
    seen: Mutex<Option<HashMap<String, Seen>>>,
    /// When this run began: a sighting before it is of an earlier run
    /// ([`Self::leave_out_checking`]).
    started: Timestamp,
}

impl MempoolSeen {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            seen: Mutex::new(None),
            started: super::now(),
        }
    }

    fn load(&self) -> HashMap<String, Seen> {
        super::json_file::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// When `txid` was first seen: kept from an earlier run, or `now` (at block `tip`) now.
    pub fn first_seen(&self, txid: &str, now: Timestamp, tip: u32) -> Timestamp {
        let mut guard = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let seen = guard.get_or_insert_with(|| self.load());
        if let Some(s) = seen.get(txid) {
            return s.at;
        }
        // No height yet (the chain is still connecting): kept once there is one, or it would
        // count as waiting since block 0, long stale.
        if tip == 0 {
            return now;
        }
        seen.insert(
            txid.to_owned(),
            Seen {
                at: now,
                height: tip,
                rescanned: false,
            },
        );
        let _ = super::json_file::save(&self.path, seen);
        now
    }

    /// Marks what has waited out of a block for [`STALE_AFTER`] blocks, and forgets what is
    /// in a block by now. An InstantSend payment is out of a block too until a block takes it:
    /// its time stays when it was seen (BUG-12: forgotten at its lock, it was "now" again at
    /// every snapshot and stood above newer payments). One that left the history is forgotten
    /// too (review r1 should 2: the file does not grow for ever) — but not while the history
    /// has no Core transaction at all: the chain is starting again and has not read them yet
    /// (BUG-64: forgotten then, it was "just now" again after every restart).
    ///
    /// Stale by the clock (`now`) instead while the chain has no height yet, or for a sighting
    /// kept with none by an older build: a payment first seen half an hour ago or more is not
    /// "Pending" at a start (BUG-82: two payments of a block the store had lost were «Ожидает
    /// · вчера» for the first 40 s). With both heights known only the heights count.
    pub fn mark_stale(&self, history: &mut [Activity], tip: u32, now: Timestamp) {
        let mut guard = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let seen = guard.get_or_insert_with(|| self.load());
        let mut mined = std::collections::HashSet::new();
        let mut waiting = std::collections::HashSet::new();
        for a in history.iter() {
            if let Some(txid) = a.txid.as_deref() {
                if a.block.is_some() {
                    mined.insert(txid);
                } else {
                    waiting.insert(txid);
                }
            }
        }
        let read = !mined.is_empty() || !waiting.is_empty();
        let before = seen.len();
        seen.retain(|txid, _| {
            !mined.contains(txid.as_str()) && (!read || waiting.contains(txid.as_str()))
        });
        if seen.len() != before {
            let _ = super::json_file::save(&self.path, seen);
        }
        for a in history.iter_mut().filter(|a| a.proof == Proof::Mempool) {
            let old = a
                .txid
                .as_deref()
                .and_then(|t| seen.get(t))
                .is_some_and(|s| {
                    if tip == 0 || s.height == 0 {
                        now.saturating_sub(s.at) >= STALE_SECS
                    } else {
                        tip >= s.height + STALE_AFTER
                    }
                });
            if old {
                a.proof = Proof::Unconfirmed;
            }
        }
    }

    /// The stale lines of `history` ([`Self::mark_stale`]) no rescan was asked for yet
    /// (BUG-64 review r1 should 1): each one finished only once, across restarts too.
    pub fn not_rescanned(&self, history: &[Activity]) -> Vec<String> {
        let mut guard = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let seen = guard.get_or_insert_with(|| self.load());
        history
            .iter()
            .filter(|a| a.proof == Proof::Unconfirmed)
            .filter_map(|a| a.txid.as_deref())
            .filter(|t| seen.get(*t).is_some_and(|s| !s.rescanned))
            .map(str::to_owned)
            .collect()
    }

    /// Leaves out of `history` the stale lines first seen in an earlier run while their rescan
    /// is still ahead or under way (BUG-82). Such a line is most likely a payment of a block
    /// the store lost (BUG-64, before `mined.json` knew it): the rescan puts it back in its
    /// block, and until then it showed as a payment of yesterday still waiting, ±1 DASH that
    /// was not coming.
    ///
    /// Only while the rescan can still say (review r1 must 1): the sync is not done yet
    /// (`synced` false: none could be asked), or a rescan asked is owed for that line (`owed`,
    /// [`super::OwedRescan`]). Synced with none owed for it — none asked, as for a second
    /// stale line after the first rescan from the birth — it is shown as not confirmed by the
    /// network, as before. So is one that went stale in this run, one whose rescan has run to
    /// its end, and one of an earlier run still pending.
    pub fn leave_out_checking(&self, history: &mut Vec<Activity>, owed: &[String], synced: bool) {
        let mut guard = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let seen = guard.get_or_insert_with(|| self.load());
        history.retain(|a| {
            let checking = a.proof == Proof::Unconfirmed
                && a.txid.as_deref().is_some_and(|t| {
                    seen.get(t)
                        .is_some_and(|s| !s.rescanned && s.at < self.started)
                        && (!synced || owed.iter().any(|o| o == t))
                });
            !checking
        });
    }

    /// Notes that a rescan for `txids` ran to its end (kept in the file): not when it was asked,
    /// or one cut short by closing the app would never be asked again (review r2 should 1).
    pub fn rescanned(&self, txids: &[String]) {
        let mut guard = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let seen = guard.get_or_insert_with(|| self.load());
        let mut changed = false;
        for t in txids {
            if let Some(s) = seen.get_mut(t)
                && !s.rescanned
            {
                s.rescanned = true;
                changed = true;
            }
        }
        if changed {
            let _ = super::json_file::save(&self.path, seen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::{ActivityKind, Chain};

    fn waiting(txid: &str, proof: Proof) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Received { from: None },
            chain: Chain::Core,
            at: 0,
            amount: 100_000_000,
            proof,
            txid: Some(txid.into()),
            fee: None,
            block: None,
            address: None,
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::In,
            key: String::new(),
            note: None,
        }
    }

    #[test]
    fn the_first_sight_survives_a_restart_and_old_ones_are_marked() {
        let dir = std::env::temp_dir().join(format!("evolite-mempool-{}", std::process::id()));
        let path = dir.join("mempool-seen.json");
        let _ = std::fs::remove_dir_all(&dir);
        let seen = MempoolSeen::new(path.clone());
        assert_eq!(seen.first_seen("aa", 1_000, 500), 1_000);
        assert_eq!(seen.first_seen("aa", 2_000, 510), 1_000, "kept, not now");
        // Another run of the app.
        let again = MempoolSeen::new(path.clone());
        assert_eq!(again.first_seen("aa", 9_000, 600), 1_000);
        let mut h = vec![
            waiting("aa", Proof::Mempool),
            waiting("bb", Proof::InstantLock),
        ];
        again.mark_stale(&mut h, 500 + STALE_AFTER - 1, 1_000);
        assert_eq!(h[0].proof, Proof::Mempool, "not long enough yet");
        again.mark_stale(&mut h, 500 + STALE_AFTER, 1_000);
        assert_eq!(h[0].proof, Proof::Unconfirmed);
        assert_eq!(h[1].proof, Proof::InstantLock, "only what waits");
        // In a block by now: forgotten.
        let mut h = vec![waiting("aa", Proof::ChainLock)];
        h[0].block = Some(650);
        again.mark_stale(&mut h, 700, 1_000);
        assert_eq!(MempoolSeen::new(path).first_seen("aa", 5, 1), 5);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// BUG-64: a history without the payment (read while the chain starts again) does not
    /// forget when it was first seen, nor is a sighting kept before the chain has a height.
    #[test]
    fn a_short_history_or_no_height_forgets_nothing() {
        let dir = std::env::temp_dir().join(format!("evolite-mempool-64-{}", std::process::id()));
        let path = dir.join("mempool-seen.json");
        let _ = std::fs::remove_dir_all(&dir);
        let seen = MempoolSeen::new(path.clone());
        assert_eq!(seen.first_seen("aa", 1_000, 500), 1_000);
        seen.mark_stale(&mut [], 501, 1_000);
        assert_eq!(
            MempoolSeen::new(path.clone()).first_seen("aa", 2_000, 502),
            1_000,
            "kept"
        );
        assert_eq!(seen.first_seen("bb", 3_000, 0), 3_000);
        assert_eq!(
            seen.first_seen("bb", 4_000, 600),
            4_000,
            "not kept at height 0"
        );
        let mut h = vec![waiting("bb", Proof::Mempool)];
        seen.mark_stale(&mut h, 600 + STALE_AFTER - 1, 1_000);
        assert_eq!(h[0].proof, Proof::Mempool, "not stale since block 0");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// BUG-64 review r1: a payment that left a read history is forgotten (the file does not grow
    /// for ever); a stale one asks for a rescan once, also after a restart.
    #[test]
    fn a_line_that_left_is_forgotten_and_a_rescan_is_asked_once() {
        let dir = std::env::temp_dir().join(format!("evolite-mempool-r1-{}", std::process::id()));
        let path = dir.join("mempool-seen.json");
        let _ = std::fs::remove_dir_all(&dir);
        let seen = MempoolSeen::new(path.clone());
        seen.first_seen("gone", 1_000, 500);
        seen.first_seen("old", 1_000, 500);
        let mut h = vec![waiting("old", Proof::Mempool)];
        seen.mark_stale(&mut h, 500 + STALE_AFTER, 1_000);
        assert_eq!(h[0].proof, Proof::Unconfirmed);
        let again = MempoolSeen::new(path.clone());
        assert_eq!(again.first_seen("gone", 2_000, 600), 2_000, "forgotten");
        assert_eq!(again.not_rescanned(&h), vec!["old".to_owned()]);
        again.rescanned(&["old".to_owned()]);
        assert!(MempoolSeen::new(path).not_rescanned(&h).is_empty(), "once");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// BUG-82: payments seen out of a block yesterday are not "Pending" at a start, before the
    /// chain has a height — a sighting kept with a height or, by an older build, without one.
    #[test]
    fn a_sighting_of_yesterday_is_stale_before_the_chain_has_a_height() {
        let dir = std::env::temp_dir().join(format!("evolite-mempool-82-{}", std::process::id()));
        let path = dir.join("mempool-seen.json");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // As qa-data's werner555 has it: kept at height 0.
        std::fs::write(
            &path,
            r#"{"legacy":{"at":1000,"height":0},"kept":{"at":1000,"height":500}}"#,
        )
        .unwrap();
        let seen = MempoolSeen::new(path.clone());
        let mut h = vec![
            waiting("legacy", Proof::Mempool),
            waiting("kept", Proof::Mempool),
        ];
        seen.mark_stale(&mut h, 0, 1_000 + STALE_SECS - 1);
        assert_eq!((h[0].proof, h[1].proof), (Proof::Mempool, Proof::Mempool));
        seen.mark_stale(&mut h, 0, 1_000 + STALE_SECS);
        assert_eq!(
            (h[0].proof, h[1].proof),
            (Proof::Unconfirmed, Proof::Unconfirmed),
            "stale by the clock with no height yet"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What [`MempoolSeen::leave_out_checking`] keeps of `h`.
    fn kept(seen: &MempoolSeen, h: &[Activity], owed: &[&str], synced: bool) -> Vec<String> {
        let mut h = h.to_vec();
        let owed: Vec<String> = owed.iter().map(|t| (*t).to_owned()).collect();
        seen.leave_out_checking(&mut h, &owed, synced);
        h.into_iter().filter_map(|a| a.txid).collect()
    }

    /// BUG-82: a stale line of an earlier run is left out while its rescan can still say —
    /// before the sync is done, or while the rescan asked is owed for it — and until that
    /// rescan has run to its end; one that went stale in this run is not, nor one of an earlier
    /// run still pending (review r1 P3 3).
    #[test]
    fn a_stale_line_of_an_earlier_run_waits_for_its_rescan() {
        let dir = std::env::temp_dir().join(format!("evolite-mempool-82c-{}", std::process::id()));
        let path = dir.join("mempool-seen.json");
        let _ = std::fs::remove_dir_all(&dir);
        let seen = MempoolSeen::new(path.clone());
        let now = super::super::now();
        seen.first_seen("earlier", 1_000, 500);
        // Seen a minute before this run, one block short of stale.
        seen.first_seen("pending", seen.started - 60, 501);
        seen.first_seen("this run", now, 500);
        let mut h = vec![
            waiting("earlier", Proof::Mempool),
            waiting("pending", Proof::Mempool),
            waiting("this run", Proof::Mempool),
        ];
        seen.mark_stale(&mut h, 500 + STALE_AFTER, now);
        assert_eq!(
            [h[0].proof, h[1].proof, h[2].proof],
            [Proof::Unconfirmed, Proof::Mempool, Proof::Unconfirmed]
        );
        let all = ["earlier", "pending", "this run"]
            .map(str::to_owned)
            .to_vec();
        let without = ["pending", "this run"].map(str::to_owned).to_vec();
        assert_eq!(kept(&seen, &h, &[], false), without, "no rescan asked yet");
        assert_eq!(kept(&seen, &h, &["earlier"], true), without, "rescan owed");
        // Synced and none owed for it (none asked, or asked before "Add a wallet" and not
        // carried back): shown as not confirmed, not hidden until a restart (review r1 must 1).
        assert_eq!(kept(&seen, &h, &[], true), all, "none owed");
        assert_eq!(
            kept(&seen, &h, &["this run"], true),
            all,
            "owed for another"
        );
        // Another run: still left out until a rescan has run to its end.
        let again = MempoolSeen::new(path.clone());
        assert_eq!(kept(&again, &h, &[], false), without);
        again.rescanned(&["earlier".to_owned(), "this run".to_owned()]);
        assert_eq!(kept(&again, &h, &[], false), all, "shown as not confirmed");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Review r1 should 2: with both heights known only the heights count — a payment of this
    /// run waiting more than half an hour for a slow block is still pending.
    #[test]
    fn with_a_height_the_clock_does_not_count() {
        let dir = std::env::temp_dir().join(format!("evolite-mempool-82s-{}", std::process::id()));
        let path = dir.join("mempool-seen.json");
        let _ = std::fs::remove_dir_all(&dir);
        let seen = MempoolSeen::new(path.clone());
        seen.first_seen("slow", 1_000, 500);
        let mut h = vec![waiting("slow", Proof::Mempool)];
        seen.mark_stale(&mut h, 500 + STALE_AFTER - 1, 1_000 + 10 * STALE_SECS);
        assert_eq!(h[0].proof, Proof::Mempool, "an hour or more, 11 blocks");
        seen.mark_stale(&mut h, 500 + STALE_AFTER, 1_000);
        assert_eq!(
            h[0].proof,
            Proof::Unconfirmed,
            "12 blocks, whatever the clock"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// BUG-12: an InstantSend payment out of a block keeps the time it was first seen, also
    /// after a restart; once in a block it is forgotten.
    #[test]
    fn an_instant_payment_keeps_its_time_until_a_block() {
        let dir = std::env::temp_dir().join(format!("evolite-mempool-is-{}", std::process::id()));
        let path = dir.join("mempool-seen.json");
        let _ = std::fs::remove_dir_all(&dir);
        let seen = MempoolSeen::new(path.clone());
        assert_eq!(seen.first_seen("is", 1_000, 500), 1_000);
        let mut h = vec![waiting("is", Proof::InstantLock)];
        seen.mark_stale(&mut h, 501, 1_000);
        assert_eq!(seen.first_seen("is", 1_030, 501), 1_000, "not now");
        seen.mark_stale(&mut h, 502, 1_000);
        assert_eq!(
            MempoolSeen::new(path.clone()).first_seen("is", 1_060, 502),
            1_000,
            "after a restart too"
        );
        assert_eq!(h[0].proof, Proof::InstantLock, "never marked stale");
        h[0].block = Some(503);
        h[0].proof = Proof::ChainLock;
        seen.mark_stale(&mut h, 503, 1_000);
        assert_eq!(
            MempoolSeen::new(path).first_seen("is", 9, 1),
            9,
            "forgotten"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
