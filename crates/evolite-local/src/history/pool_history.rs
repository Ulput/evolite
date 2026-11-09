//! The pool's operations as they were when made: amounts and fees. platform-wallet keeps its
//! shielded history in memory and leaves keeping it to the host; the store evolite uses keeps
//! only the viewing keys. After a restart the history is derived again from the scan, and a
//! spend with no outside output (a top-up of an identity from the pool) comes back as a spend
//! of 0 with no fee. This file keeps what was known and puts it back over what the scan gives:
//! an operation's id comes from its outputs, the same before and after. Public data only
//! (amounts, times, blocks). A missing or unreadable file is no operations.

use evolite_api::{Activity, ActivityKind, Chain, Duffs, Proof, Timestamp};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// One operation as kept: what the history line needs.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Saved {
    id: u64,
    kind: Kind,
    amount: Duffs,
    fee: Option<Duffs>,
    at: Timestamp,
    block: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
enum Kind {
    Shielded {
        from: Option<String>,
    },
    Received,
    Sent,
    /// Between the pool's own addresses (BUG-109).
    SelfTransfer {
        moved: Duffs,
    },
}

pub struct PoolHistory {
    path: PathBuf,
    /// By operation id; read once, kept up to date.
    saved: Mutex<Option<BTreeMap<u64, Saved>>>,
}

impl PoolHistory {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            saved: Mutex::new(None),
        }
    }

    /// The pool's operations from the scan, with what was kept put back: an operation that
    /// comes back poorer (no amount, no fee) is shown as it was, one the scan no longer gives
    /// is shown from the file, and one known better now is kept for next time. One the scan gives
    /// empty with nothing better known is kept as it is: it is still the wallet's line, and it is
    /// there at the next start before the scan is (BUG-103). Only what Platform took is kept: a move
    /// seen pending may fail and leave the scan (review r1 should 2).
    pub fn merge(&self, operations: Vec<Activity>) -> Vec<Activity> {
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        let all = saved.get_or_insert_with(|| self.load());
        let mut changed = false;
        let mut shown: Vec<Activity> = operations
            .into_iter()
            .map(|op| match all.get(&op.id) {
                // The scan lost what the operation moved, or took the wallet's own shield for a
                // note that came in (after a restart it cannot tell them apart, and gives no
                // time): the kept line, with the scan's proof.
                Some(kept) if poor(&op) || own_shield(kept, &op) => Activity {
                    proof: op.proof,
                    block: op.block.or(kept.block),
                    layers: op.layers,
                    ..kept.to_activity()
                },
                _ => {
                    if op.proof == Proof::Proved
                        && let Some(new) = Saved::of(&op)
                        && all.get(&op.id) != Some(&new)
                    {
                        all.insert(op.id, new);
                        changed = true;
                    }
                    op
                }
            })
            .collect();
        for kept in all.values() {
            if !shown.iter().any(|op| op.id == kept.id) {
                shown.push(kept.to_activity());
            }
        }
        if changed {
            let _ = crate::json_file::save(&self.path, &all.values().collect::<Vec<_>>());
        }
        shown
    }

    /// The operations the file keeps, for the history while the pool is not connected yet (a
    /// start, BUG-103): the lines do not wait for the masternode lists and the scan.
    pub fn kept(&self) -> Vec<Activity> {
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        saved
            .get_or_insert_with(|| self.load())
            .values()
            .map(Saved::to_activity)
            .collect()
    }

    fn load(&self) -> BTreeMap<u64, Saved> {
        crate::json_file::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<Saved>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.id, s))
            .collect()
    }
}

/// The wallet's own shield, kept as made, that the scan now gives as a note received: the
/// same outputs, so the same id (BUG-37). The kept kind and time stay.
fn own_shield(kept: &Saved, op: &Activity) -> bool {
    matches!(kept.kind, Kind::Shielded { .. }) && op.kind == ActivityKind::PoolReceived
}

/// A line the scan could not fill: nothing moved and no fee known.
fn poor(op: &Activity) -> bool {
    op.amount == 0 && op.fee.is_none()
}

impl Saved {
    fn of(op: &Activity) -> Option<Self> {
        let kind = match &op.kind {
            ActivityKind::Shielded { from } => Kind::Shielded { from: from.clone() },
            ActivityKind::PoolReceived => Kind::Received,
            ActivityKind::PoolSent => Kind::Sent,
            ActivityKind::SelfTransfer { moved } => Kind::SelfTransfer { moved: *moved },
            _ => return None,
        };
        Some(Self {
            id: op.id,
            kind,
            amount: op.amount,
            fee: op.fee,
            at: op.at,
            block: op.block,
        })
    }

    fn to_activity(&self) -> Activity {
        Activity {
            id: self.id,
            kind: match &self.kind {
                Kind::Shielded { from } => ActivityKind::Shielded { from: from.clone() },
                Kind::Received => ActivityKind::PoolReceived,
                Kind::Sent => ActivityKind::PoolSent,
                Kind::SelfTransfer { moved } => ActivityKind::SelfTransfer { moved: *moved },
            },
            chain: Chain::Platform,
            at: self.at,
            amount: self.amount,
            // Kept once Platform took it; the scan says otherwise when it knows better.
            proof: Proof::Proved,
            txid: None,
            fee: self.fee,
            block: self.block,
            address: None,
            layers: evolite_api::layer::POOL,
            flow: match self.kind {
                Kind::Shielded { .. } => evolite_api::Flow::Between,
                Kind::Received => evolite_api::Flow::In,
                Kind::Sent => evolite_api::Flow::Out,
                Kind::SelfTransfer { .. } => evolite_api::Flow::Between,
            },
            key: format!("pool:{:016x}", self.id),
            detail: None,
            note: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(id: u64, kind: ActivityKind, amount: Duffs, fee: Option<Duffs>) -> Activity {
        let (layers, flow) = evolite_common::layers::of(&kind, Chain::Platform);
        Activity {
            id,
            kind,
            chain: Chain::Platform,
            at: 1_000,
            amount,
            proof: Proof::Proved,
            txid: None,
            fee,
            block: Some(7),
            address: None,
            layers,
            flow,
            // As `pool.rs` keys its lines and a saved one comes back (FEAT-0003).
            key: format!("pool:{id:016x}"),
            detail: None,
            note: None,
        }
    }

    #[test]
    fn a_top_up_from_the_pool_keeps_its_amount_over_a_restart() {
        let dir = std::env::temp_dir().join(format!("evolite-pool-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        // While the app runs: the line as made, with its amount and fee.
        let live = PoolHistory::new(path.clone());
        let made = op(1, ActivityKind::PoolSent, -214_360, Some(114_360));
        let received = op(2, ActivityKind::PoolReceived, 70_000, None);
        assert_eq!(
            live.merge(vec![made.clone(), received.clone()]),
            vec![made.clone(), received.clone()]
        );
        // After a restart: the scan gives the spend as 0 with no fee, and loses nothing else.
        let again = PoolHistory::new(path);
        let shown = again.merge(vec![
            op(1, ActivityKind::PoolSent, 0, None),
            received.clone(),
        ]);
        assert_eq!(shown, vec![made, received]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_operation_the_scan_no_longer_gives_is_still_shown() {
        let dir =
            std::env::temp_dir().join(format!("evolite-pool-history-gone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let shield = op(3, ActivityKind::Shielded { from: None }, 300_000, None);
        PoolHistory::new(path.clone()).merge(vec![shield.clone()]);
        assert_eq!(PoolHistory::new(path).merge(Vec::new()), vec![shield]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-37 r1 must 2: after a restart the scan gives the wallet's own shield as a note that
    /// came in, with no time. It is still the shield made then (same id), kept so in the file,
    /// and not counted again as money received.
    #[test]
    fn the_own_shield_stays_a_shield_over_a_restart() {
        let dir =
            std::env::temp_dir().join(format!("evolite-pool-history-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let shield = op(
            4,
            ActivityKind::Shielded { from: None },
            300_000,
            Some(2_000),
        );
        PoolHistory::new(path.clone()).merge(vec![shield.clone()]);
        let scanned = Activity {
            at: 0,
            fee: None,
            block: Some(9),
            ..op(4, ActivityKind::PoolReceived, 300_000, None)
        };
        let again = PoolHistory::new(path.clone());
        let expected = Activity {
            block: Some(9),
            ..shield.clone()
        };
        assert_eq!(again.merge(vec![scanned.clone()]), vec![expected.clone()]);
        // And the file was not overwritten: the next start still knows it.
        assert_eq!(PoolHistory::new(path).merge(vec![scanned]), vec![expected]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review r1 should 2: a move seen pending and then gone failed: it is no line of the history
    /// (it came back from the file as proved, with its amount, for good).
    #[test]
    fn a_pending_move_that_went_is_not_kept() {
        let dir =
            std::env::temp_dir().join(format!("evolite-pool-history-went-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let sending = Activity {
            proof: Proof::Sending,
            ..op(8, ActivityKind::PoolSent, -214_360, Some(114_360))
        };
        let live = PoolHistory::new(path.clone());
        assert_eq!(live.merge(vec![sending.clone()]), vec![sending]);
        assert_eq!(live.merge(Vec::new()), Vec::new(), "gone from the scan");
        assert_eq!(
            PoolHistory::new(path).kept(),
            Vec::new(),
            "and from the file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// And one Platform took is kept from then on.
    #[test]
    fn a_pending_move_is_kept_once_proved() {
        let dir = std::env::temp_dir().join(format!(
            "evolite-pool-history-proved-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let proved = op(9, ActivityKind::PoolSent, -214_360, Some(114_360));
        let sending = Activity {
            proof: Proof::Sending,
            ..proved.clone()
        };
        let live = PoolHistory::new(path.clone());
        live.merge(vec![sending]);
        live.merge(vec![proved.clone()]);
        assert_eq!(PoolHistory::new(path).kept(), vec![proved]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-109: a transfer between the pool's own addresses is a line of the history too, and
    /// comes back at the start with the rest.
    #[test]
    fn a_proved_self_transfer_is_kept() {
        let dir =
            std::env::temp_dir().join(format!("evolite-pool-history-self-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let moved = op(
            10,
            ActivityKind::SelfTransfer { moved: 0 },
            -114_140,
            Some(114_140),
        );
        PoolHistory::new(path.clone()).merge(vec![moved.clone()]);
        assert_eq!(PoolHistory::new(path).kept(), vec![moved]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The rule stays: pending, it is not kept.
    #[test]
    fn a_pending_self_transfer_is_not_kept() {
        let dir = std::env::temp_dir().join(format!(
            "evolite-pool-history-self-p-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let moved = Activity {
            proof: Proof::Sending,
            ..op(
                11,
                ActivityKind::SelfTransfer { moved: 0 },
                -114_140,
                Some(114_140),
            )
        };
        PoolHistory::new(path.clone()).merge(vec![moved]);
        assert_eq!(PoolHistory::new(path).kept(), Vec::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-109: as the screen had it: the scan gives this line with no fee, 0 moved and no time.
    #[test]
    fn a_self_transfer_the_scan_gives_empty_is_kept() {
        let dir = std::env::temp_dir().join(format!(
            "evolite-pool-history-self-e-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let moved = Activity {
            at: 0,
            block: None,
            ..op(12, ActivityKind::SelfTransfer { moved: 0 }, 0, None)
        };
        PoolHistory::new(path.clone()).merge(vec![moved.clone()]);
        assert_eq!(PoolHistory::new(path).kept(), vec![moved]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-103: the scan gave a spend as 0 with no fee and no time (a restored wallet's own
    /// spend). Nothing better is known of it, and it is still the wallet's operation: the file
    /// keeps it, so it is in the history at the next start before the pool connects.
    #[test]
    fn a_line_the_scan_gives_empty_is_kept_for_the_next_start() {
        let dir =
            std::env::temp_dir().join(format!("evolite-pool-history-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let empty = Activity {
            at: 0,
            ..op(5, ActivityKind::PoolSent, 0, None)
        };
        assert_eq!(
            PoolHistory::new(path.clone()).merge(vec![empty.clone()]),
            vec![empty.clone()]
        );
        assert_eq!(PoolHistory::new(path).kept(), vec![empty]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-103: what the file holds is there with no scan at all, as the merge would show it.
    #[test]
    fn the_kept_lines_are_there_before_any_scan() {
        let dir =
            std::env::temp_dir().join(format!("evolite-pool-history-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let made = op(6, ActivityKind::PoolSent, -214_360, Some(114_360));
        let received = op(7, ActivityKind::PoolReceived, 70_000, None);
        PoolHistory::new(path.clone()).merge(vec![made.clone(), received.clone()]);
        assert_eq!(PoolHistory::new(path).kept(), vec![made, received]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
