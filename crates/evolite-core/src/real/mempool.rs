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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Seen {
    at: Timestamp,
    height: u32,
}

pub struct MempoolSeen {
    path: PathBuf,
    seen: Mutex<Option<HashMap<String, Seen>>>,
}

impl MempoolSeen {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            seen: Mutex::new(None),
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
        seen.insert(
            txid.to_owned(),
            Seen {
                at: now,
                height: tip,
            },
        );
        let _ = super::json_file::save(&self.path, seen);
        now
    }

    /// Marks what has waited out of a block for [`STALE_AFTER`] blocks, and forgets what is
    /// in a block by now. An InstantSend payment is out of a block too until a block takes it:
    /// its time stays when it was seen (BUG-12: forgotten at its lock, it was "now" again at
    /// every snapshot and stood above newer payments).
    pub fn mark_stale(&self, history: &mut [Activity], tip: u32) {
        let mut guard = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let seen = guard.get_or_insert_with(|| self.load());
        let waiting: std::collections::HashSet<&str> = history
            .iter()
            .filter(|a| a.block.is_none())
            .filter_map(|a| a.txid.as_deref())
            .collect();
        let before = seen.len();
        seen.retain(|txid, _| waiting.contains(txid.as_str()));
        if seen.len() != before {
            let _ = super::json_file::save(&self.path, seen);
        }
        for a in history.iter_mut().filter(|a| a.proof == Proof::Mempool) {
            let old = a
                .txid
                .as_deref()
                .and_then(|t| seen.get(t))
                .is_some_and(|s| tip >= s.height + STALE_AFTER);
            if old {
                a.proof = Proof::Unconfirmed;
            }
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
        again.mark_stale(&mut h, 500 + STALE_AFTER - 1);
        assert_eq!(h[0].proof, Proof::Mempool, "not long enough yet");
        again.mark_stale(&mut h, 500 + STALE_AFTER);
        assert_eq!(h[0].proof, Proof::Unconfirmed);
        assert_eq!(h[1].proof, Proof::InstantLock, "only what waits");
        // In a block by now: forgotten.
        let mut h = vec![waiting("aa", Proof::ChainLock)];
        h[0].block = Some(650);
        again.mark_stale(&mut h, 700);
        assert_eq!(MempoolSeen::new(path).first_seen("aa", 5, 1), 5);
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
        seen.mark_stale(&mut h, 501);
        assert_eq!(seen.first_seen("is", 1_030, 501), 1_000, "not now");
        seen.mark_stale(&mut h, 502);
        assert_eq!(
            MempoolSeen::new(path.clone()).first_seen("is", 1_060, 502),
            1_000,
            "after a restart too"
        );
        assert_eq!(h[0].proof, Proof::InstantLock, "never marked stale");
        h[0].block = Some(503);
        h[0].proof = Proof::ChainLock;
        seen.mark_stale(&mut h, 503);
        assert_eq!(
            MempoolSeen::new(path).first_seen("is", 9, 1),
            9,
            "forgotten"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
