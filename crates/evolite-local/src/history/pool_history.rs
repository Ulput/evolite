//! The pool's operations as they were when made: amounts and fees. platform-wallet keeps its
//! shielded history in memory and leaves keeping it to the host; the store evolite uses keeps
//! only the viewing keys. After a restart the history is derived again from the scan, and a
//! spend with no outside output (a top-up of an identity from the pool) comes back as a spend
//! of 0 with no fee. This file keeps what was known and puts it back over what the scan gives:
//! an operation's id comes from its outputs, the same before and after. Public data only
//! (amounts, times, blocks). A missing or unreadable file is no operations.

use evolite_api::{Activity, ActivityKind, Chain, Duffs, Proof, Timestamp};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

/// One operation as kept: what the history line needs. Opaque outside: the backup file
/// (FEAT-0045) carries it as it is.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Saved {
    id: u64,
    kind: Kind,
    amount: Duffs,
    fee: Option<Duffs>,
    at: Timestamp,
    block: Option<u32>,
    /// BUG-320 p. 2: the tags of the notes it made ([`tag`]), never the notes themselves: what
    /// ties a line the scan gives under another id to this one. None in a file from before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<Tag>,
}

/// A note of the pool as this file may name it (BUG-320 p. 2): the first 8 bytes of a keyed
/// BLAKE2b of its commitment (cmx). A plain cmx would tie the file — and a backup of it — to the
/// wallet's notes in the pool; the tag cannot be checked against a cmx without the wallet's key
/// ([`tag_key`]), and 8 bytes tell one wallet's notes apart.
pub type Tag = [u8; 8];

/// The key of the note tags of the wallet whose `seed` it is: the same on every copy of the
/// phrase, so a backup's lines meet the scan's. One way (a BLAKE2b of the seed under its own
/// personalization: nothing of the seed comes back from it); wiped when dropped, kept in memory
/// only — never in a file or the journal (the architect's conditions, BUG-320 p. 2).
pub fn tag_key(seed: &[u8]) -> zeroize::Zeroizing<[u8; 32]> {
    let hash = blake2b_simd::Params::new()
        .hash_length(32)
        .personal(b"evolite-pooltagk")
        .hash(seed);
    let mut key = zeroize::Zeroizing::new([0u8; 32]);
    key.copy_from_slice(hash.as_bytes());
    key
}

/// The tag of the note whose commitment is `cmx`, under `key` ([`tag_key`]).
pub fn tag(key: &[u8; 32], cmx: &[u8; 32]) -> Tag {
    let hash = blake2b_simd::Params::new()
        .hash_length(8)
        .key(key)
        .personal(b"evolite-pooltag1")
        .hash(cmx);
    let mut tag = [0u8; 8];
    tag.copy_from_slice(hash.as_bytes());
    tag
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
enum Kind {
    Shielded {
        from: Option<String>,
    },
    Received,
    Sent,
    /// FEAT-0049 r1: out of the pool to one of the wallet's own Core addresses, which the
    /// Core payment it becomes is matched by. Kept: after a restart the scan no longer knows
    /// where a withdrawal went.
    ToCore {
        address: Option<String>,
    },
    /// Between the pool's own addresses (BUG-109).
    SelfTransfer {
        moved: Duffs,
    },
    /// BUG-320: into an identity of ours, by its id — its creation or a top-up. Kept: after a
    /// restart the scan gives it as a payment out with no recipient. A creation's `fee` is
    /// what it cost the network (BUG-319 p. 3), `None` when not known.
    ToIdentity {
        identity: String,
        creates: bool,
    },
}

pub struct PoolHistory {
    path: PathBuf,
    /// By operation id; read once, kept up to date.
    saved: Mutex<Option<BTreeMap<u64, Saved>>>,
    /// BUG-319 p. 3: what each creation of this run cost, by the identity's id, from its own
    /// answer — put on its line when the scan gives it ([`Self::creation_cost`]).
    costs: Mutex<Vec<Cost>>,
}

/// What creating `identity` cost the network, noted `at`; `None`: its credits were not read.
#[derive(Clone, Debug, PartialEq)]
struct Cost {
    identity: String,
    cost: Option<Duffs>,
    at: Timestamp,
}

impl PoolHistory {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            saved: Mutex::new(None),
            costs: Mutex::new(Vec::new()),
        }
    }

    /// BUG-319 p. 3: what creating `identity` cost the network — the exit less the credits that
    /// came in, `None` when they were not read. One truth for its line in History (its fee) and
    /// for the price record ([`Self::cost_since`]).
    pub fn creation_cost(&self, identity: &str, cost: Option<Duffs>, at: Timestamp) {
        let mut costs = self.costs.lock().unwrap_or_else(|e| e.into_inner());
        costs.retain(|c| c.identity != identity);
        costs.push(Cost {
            identity: identity.to_owned(),
            cost,
            at,
        });
    }

    /// The cost of the newest creation noted since `since`: `None` when none was, `Some(None)`
    /// when its credits were not read.
    pub fn cost_since(&self, since: Timestamp) -> Option<Option<Duffs>> {
        let costs = self.costs.lock().unwrap_or_else(|e| e.into_inner());
        costs
            .iter()
            .filter(|c| c.at >= since)
            .max_by_key(|c| c.at)
            .map(|c| c.cost)
    }

    /// A creation's line with its cost as noted this run: what the network took, or none known.
    /// What left the pool (the line's amount) stays: the exit, exact.
    fn costed(&self, mut op: Activity) -> Activity {
        if let ActivityKind::PoolToIdentity {
            identity,
            creates: true,
        } = &op.kind
        {
            // The pool's own fee for it is what platform-wallet predicted, not what was taken:
            // none known until the creation's answer says.
            let costs = self.costs.lock().unwrap_or_else(|e| e.into_inner());
            op.fee = costs
                .iter()
                .find(|c| &c.identity == identity)
                .and_then(|c| c.cost);
        }
        op
    }

    /// The pool's operations from the scan, with what was kept put back: an operation that
    /// comes back poorer (no amount, no fee) is shown as it was, one the scan no longer gives
    /// is shown from the file, and one known better now is kept for next time. One the scan gives
    /// empty with nothing better known is kept as it is: it is still the wallet's line, and it is
    /// there at the next start before the scan is (BUG-103). Only what Platform took is kept: a move
    /// seen pending may fail and leave the scan (review r1 should 2). With no note tags: by id only.
    pub fn merge(&self, operations: Vec<Activity>) -> Vec<Activity> {
        self.merge_scan(operations, &HashMap::new(), true, false)
    }

    /// [`Self::merge`] with each operation's note tags ([`tag`]) and whether the scan gave every
    /// operation of the pool (`complete`). BUG-320 p. 2: after a restart the scan groups notes by
    /// fetch batch, not by operation — a batch with our change and another note is a spend under
    /// a new id. By id first; then a scanned operation whose tags meet a kept one's is not new:
    /// with exactly one, that one takes what the scan knows better and its tags; with several,
    /// nothing changes. Once a complete scan is in, a kept empty line (nothing moved, no fee, no
    /// time) with no tags that the scan did not give is one of those and goes.
    /// Without the key (`keyed` false: the wallet locked) by id only, and a new empty undated
    /// line is shown, not kept: it may be one of those batches, and with the key it is checked.
    pub fn merge_scan(
        &self,
        operations: Vec<Activity>,
        tags: &HashMap<u64, Vec<Tag>>,
        keyed: bool,
        complete: bool,
    ) -> Vec<Activity> {
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        let all = saved.get_or_insert_with(|| self.load());
        let mut changed = false;
        let scanned: std::collections::HashSet<u64> = operations.iter().map(|op| op.id).collect();
        let mut shown: Vec<Activity> = Vec::new();
        // Kept lines met by tags, as the scan sees them now; shown after the loop unless the scan
        // gave the kept id itself (then that one is the line).
        let mut met_by_tags: Vec<Activity> = Vec::new();
        for op in operations.into_iter().map(|op| self.costed(op)) {
            let op_tags = tags.get(&op.id).map(Vec::as_slice).unwrap_or(&[]);
            // By tags, for one the file does not know by its id: the kept lines it meets.
            if !all.contains_key(&op.id) && !op_tags.is_empty() {
                let met: Vec<u64> = all
                    .values()
                    .filter(|kept| kept.tags.iter().any(|t| op_tags.contains(t)))
                    .map(|kept| kept.id)
                    .collect();
                match met.as_slice() {
                    [] => {}
                    // The same operation under the batch's id: the kept line stays, with what
                    // the scan knows better and the tags.
                    [one] => {
                        if let Some(kept) = all.get_mut(one) {
                            let before = kept.clone();
                            if kept.poor()
                                && !poor(&op)
                                && op.proof == Proof::Proved
                                && let Some(better) = Saved::of(&op)
                            {
                                *kept = Saved {
                                    id: kept.id,
                                    tags: kept.tags.clone(),
                                    ..better
                                };
                            }
                            kept.add_tags(op_tags);
                            changed |= *kept != before;
                            // r1: the scan's proof and block, as for a kept line met by id.
                            met_by_tags.push(Activity {
                                proof: op.proof,
                                block: op.block.or(kept.block),
                                ..kept.to_activity()
                            });
                        }
                        continue;
                    }
                    // A batch over several kept operations: none of them, nothing new.
                    _ => continue,
                }
            }
            // The scan lost what the operation moved, or took the wallet's own shield for a note
            // that came in (after a restart it cannot tell them apart, and gives no time): the
            // kept line, with the scan's proof.
            let kept_wins = all.get(&op.id).is_some_and(|kept| {
                poor(&op)
                    || own_shield(kept, &op)
                    || own_withdrawal(kept, &op)
                    || own_identity(kept, &op)
            });
            let line = match all.get_mut(&op.id) {
                Some(kept) if kept_wins => {
                    let before = kept.tags.len();
                    kept.add_tags(op_tags);
                    changed |= kept.tags.len() != before;
                    Activity {
                        proof: op.proof,
                        block: op.block.or(kept.block),
                        // The scan's layers; a withdrawal to Core is the pool's and Core's, one
                        // into an identity the pool's and the identity's.
                        layers: if own_withdrawal(kept, &op) || own_identity(kept, &op) {
                            kept.to_activity().layers
                        } else {
                            op.layers
                        },
                        ..kept.to_activity()
                    }
                }
                _ => {
                    let unchecked = !keyed && poor(&op) && op.at == 0;
                    if op.proof == Proof::Proved
                        && !unchecked
                        && let Some(mut new) = Saved::of(&op)
                    {
                        if let Some(kept) = all.get(&op.id) {
                            new.tags = kept.tags.clone();
                        }
                        new.add_tags(op_tags);
                        if all.get(&op.id) != Some(&new) {
                            all.insert(op.id, new);
                            changed = true;
                        }
                    }
                    op
                }
            };
            shown.push(line);
        }
        // BUG-320 p. 2: the empty lines kept before tags that a complete scan no longer gives.
        // A real one (BUG-103) the scan gives again, and it is kept with its tags above.
        if complete && keyed {
            let before = all.len();
            all.retain(|id, kept| {
                scanned.contains(id) || !(kept.tags.is_empty() && kept.poor() && kept.at == 0)
            });
            changed |= all.len() != before;
        }
        for line in met_by_tags {
            if !shown.iter().any(|op| op.id == line.id) {
                shown.push(line);
            }
        }
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

    /// FEAT-0045: every operation kept (only proved ones are), for the backup file.
    pub fn records(&self) -> Vec<Saved> {
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        saved
            .get_or_insert_with(|| self.load())
            .values()
            .cloned()
            .collect()
    }

    /// FEAT-0045 R-9: a backup's operations added — one kept here stays. Written once, only
    /// when one was added. How many were added.
    pub fn merge_backup(&self, from: &[Saved]) -> std::io::Result<usize> {
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = saved.get_or_insert_with(|| self.load()).clone();
        let before = all.len();
        for s in from {
            all.entry(s.id).or_insert_with(|| s.clone());
        }
        let added = all.len() - before;
        if added > 0 {
            crate::json_file::save(&self.path, &all.values().collect::<Vec<_>>())
                .map_err(std::io::Error::other)?;
            *saved = Some(all);
        }
        Ok(added)
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

/// FEAT-0049 r1: the wallet's own withdrawal to Core, kept as made, that the scan now gives as
/// a payment out with no address (after a restart). The kept kind and address stay.
fn own_withdrawal(kept: &Saved, op: &Activity) -> bool {
    matches!(kept.kind, Kind::ToCore { .. }) && op.kind == ActivityKind::PoolSent
}

/// BUG-320: the wallet's own exit into an identity, kept as made, that the scan now gives as a
/// payment out with no recipient (after a restart). The kept kind, identity and cost stay.
fn own_identity(kept: &Saved, op: &Activity) -> bool {
    matches!(kept.kind, Kind::ToIdentity { .. }) && op.kind == ActivityKind::PoolSent
}

/// A line the scan could not fill: nothing moved and no fee known.
fn poor(op: &Activity) -> bool {
    op.amount == 0 && op.fee.is_none()
}

impl Saved {
    /// Nothing moved and no fee known ([`poor`]).
    fn poor(&self) -> bool {
        self.amount == 0 && self.fee.is_none()
    }

    fn add_tags(&mut self, tags: &[Tag]) {
        for t in tags {
            if !self.tags.contains(t) {
                self.tags.push(*t);
            }
        }
    }

    fn of(op: &Activity) -> Option<Self> {
        let kind = match &op.kind {
            ActivityKind::Shielded { from } => Kind::Shielded { from: from.clone() },
            ActivityKind::PoolReceived => Kind::Received,
            ActivityKind::PoolSent => Kind::Sent,
            ActivityKind::PoolToCore { .. } => Kind::ToCore {
                address: op.address.clone(),
            },
            ActivityKind::SelfTransfer { moved } => Kind::SelfTransfer { moved: *moved },
            ActivityKind::PoolToIdentity { identity, creates } => Kind::ToIdentity {
                identity: identity.clone(),
                creates: *creates,
            },
            _ => return None,
        };
        Some(Self {
            id: op.id,
            kind,
            amount: op.amount,
            fee: op.fee,
            at: op.at,
            block: op.block,
            tags: Vec::new(),
        })
    }

    fn to_activity(&self) -> Activity {
        Activity {
            id: self.id,
            kind: match &self.kind {
                Kind::Shielded { from } => ActivityKind::Shielded { from: from.clone() },
                Kind::Received => ActivityKind::PoolReceived,
                Kind::Sent => ActivityKind::PoolSent,
                // Whether it arrived the core finds again, from the Core payments.
                Kind::ToCore { .. } => ActivityKind::PoolToCore { arrived: false },
                Kind::SelfTransfer { moved } => ActivityKind::SelfTransfer { moved: *moved },
                Kind::ToIdentity { identity, creates } => ActivityKind::PoolToIdentity {
                    identity: identity.clone(),
                    creates: *creates,
                },
            },
            chain: Chain::Platform,
            at: self.at,
            amount: self.amount,
            // Kept once Platform took it; the scan says otherwise when it knows better.
            proof: Proof::Proved,
            txid: None,
            fee: self.fee,
            block: self.block,
            address: match &self.kind {
                Kind::ToCore { address } => address.clone(),
                _ => None,
            },
            layers: match self.kind {
                Kind::ToCore { .. } => evolite_api::layer::POOL | evolite_api::layer::CORE,
                Kind::ToIdentity { .. } => evolite_api::layer::POOL | evolite_api::layer::IDENTITY,
                _ => evolite_api::layer::POOL,
            },
            flow: match self.kind {
                Kind::Shielded { .. } => evolite_api::Flow::Between,
                Kind::Received => evolite_api::Flow::In,
                Kind::Sent => evolite_api::Flow::Out,
                Kind::ToCore { .. } => evolite_api::Flow::Between,
                Kind::SelfTransfer { .. } => evolite_api::Flow::Between,
                Kind::ToIdentity { .. } => evolite_api::Flow::Between,
            },
            key: format!("pool:{:016x}", self.id),
            detail: None,
            note: None,
            rate: None,
            tax_transfer: false,
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
            rate: None,
            tax_transfer: false,
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

    /// FEAT-0049 r1 Must: a withdrawal to the wallet's own Core stays one over a restart — the
    /// scan then gives it as a payment out with no address; the kept kind and address come back,
    /// so the Core payment it became can still be matched.
    #[test]
    fn an_own_withdrawal_to_core_stays_one_over_a_restart() {
        let dir =
            std::env::temp_dir().join(format!("evolite-pool-history-core-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let made = Activity {
            address: Some("yChange".into()),
            ..op(
                11,
                ActivityKind::PoolToCore { arrived: false },
                -150_000,
                Some(50_000),
            )
        };
        assert_eq!(
            PoolHistory::new(path.clone()).merge(vec![made.clone()]),
            vec![made.clone()]
        );
        let scanned = op(11, ActivityKind::PoolSent, -150_000, Some(50_000));
        let again = PoolHistory::new(path.clone()).merge(vec![scanned.clone()]);
        assert_eq!(again, vec![made.clone()]);
        assert_eq!(again[0].layers, made.layers, "pool and Core");
        // Before any scan, from the file alone.
        assert_eq!(PoolHistory::new(path).kept(), vec![made]);
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

    /// BUG-320, BUG-319 p. 3: an identity created from the pool is its own line — by its id,
    /// with what the network took as its fee (not platform-wallet's prediction) — and stays so
    /// after a restart, when the scan gives it as a payment out with no recipient.
    #[test]
    fn a_creation_from_the_pool_keeps_its_identity_and_cost_after_a_restart() {
        let dir = std::env::temp_dir().join(format!(
            "evolite-pool-history-create-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let created = ActivityKind::PoolToIdentity {
            identity: "G1cAid".into(),
            creates: true,
        };
        // As platform-wallet records it: the exit out, its own predicted fee.
        let live = op(21, created.clone(), -3_000_000, Some(229_280));
        let pool = PoolHistory::new(path.clone());
        let before = pool.merge(vec![live.clone()]);
        assert_eq!(before[0].fee, None, "a prediction is not what was taken");
        assert_eq!(pool.cost_since(0), None, "nothing noted yet");
        pool.creation_cost("G1cAid", Some(336_000), 1_000);
        let shown = pool.merge(vec![live]);
        assert_eq!(shown[0].kind, created);
        assert_eq!((shown[0].amount, shown[0].fee), (-3_000_000, Some(336_000)));
        assert_eq!(pool.cost_since(1_000), Some(Some(336_000)), "the same cost");
        assert_eq!(pool.cost_since(1_001), None, "only since the spend began");
        // After a restart: the scan's line has no kind of its own.
        let scanned = Activity {
            at: 0,
            block: None,
            ..op(21, ActivityKind::PoolSent, -2_663_860, None)
        };
        let again = PoolHistory::new(path).merge(vec![scanned]);
        assert_eq!(again[0].kind, created, "named, not «Sent privately»");
        assert_eq!((again[0].amount, again[0].fee), (-3_000_000, Some(336_000)));
        assert_eq!(
            again[0].layers,
            evolite_api::layer::POOL | evolite_api::layer::IDENTITY
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-319 p. 3: credits not read — the cost is not known: no fee, never 0, and the price
    /// record gets none either.
    #[test]
    fn a_creation_whose_credits_were_not_read_has_no_cost() {
        let dir = std::env::temp_dir().join(format!(
            "evolite-pool-history-create-u-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let pool = PoolHistory::new(dir.join("pool-history.json"));
        pool.creation_cost("G1cAid", None, 1_000);
        let live = op(
            22,
            ActivityKind::PoolToIdentity {
                identity: "G1cAid".into(),
                creates: true,
            },
            -3_000_000,
            Some(229_280),
        );
        assert_eq!(pool.merge(vec![live])[0].fee, None);
        assert_eq!(pool.cost_since(1_000), Some(None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-320: a top-up of an identity from the pool keeps its own fee and is named after a
    /// restart too.
    #[test]
    fn a_top_up_from_the_pool_is_named_after_a_restart() {
        let dir =
            std::env::temp_dir().join(format!("evolite-pool-history-topup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        let topped = ActivityKind::PoolToIdentity {
            identity: "kestrelid".into(),
            creates: false,
        };
        let live = op(23, topped.clone(), -214_359, Some(114_359));
        assert_eq!(
            PoolHistory::new(path.clone()).merge(vec![live.clone()]),
            vec![live]
        );
        let scanned = Activity {
            at: 0,
            block: None,
            ..op(23, ActivityKind::PoolSent, 0, None)
        };
        let again = PoolHistory::new(path).merge(vec![scanned]);
        assert_eq!(again[0].kind, topped);
        assert_eq!((again[0].amount, again[0].fee), (-214_359, Some(114_359)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-320 p. 2: a fresh directory and its history file for one test.
    fn fresh(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "evolite-pool-history-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pool-history.json");
        (dir, path)
    }

    const KEY: [u8; 32] = [9; 32];

    /// The tags of notes `cmxs` (one byte each, as a cmx of 32).
    fn tags_of(cmxs: &[u8]) -> Vec<Tag> {
        cmxs.iter().map(|&c| tag(&KEY, &[c; 32])).collect()
    }

    fn by(id: u64, cmxs: &[u8]) -> HashMap<u64, Vec<Tag>> {
        HashMap::from([(id, tags_of(cmxs))])
    }

    /// An operation as the scan gives it after a restart: no time, no block.
    fn restored(id: u64, kind: ActivityKind, amount: Duffs) -> Activity {
        Activity {
            at: 0,
            block: None,
            ..op(id, kind, amount, None)
        }
    }

    /// BUG-320 p. 2 (QA: one more «Sent privately · time unknown · —» after a restart): after a
    /// restart the scan groups by fetch batch — our change and another note in one batch is a
    /// spend of 0 under a new id. Its tags meet the kept spend's: no new line, now or later. The
    /// mutant «by id only» shows it.
    #[test]
    fn a_batch_with_our_change_and_another_note_is_not_a_new_line() {
        let (dir, path) = fresh("batch");
        let top_up = ActivityKind::PoolToIdentity {
            identity: "kestrelid".into(),
            creates: false,
        };
        let made = op(1, top_up.clone(), -214_359, Some(114_359));
        let pool = PoolHistory::new(path.clone());
        pool.merge_scan(vec![made.clone()], &by(1, &[0xA1]), true, true);
        // After a restart: the change (A1) and a note shielded later (B2) in one batch.
        let batch = restored(77, ActivityKind::PoolSent, 0);
        let shown = PoolHistory::new(path.clone()).merge_scan(
            vec![batch],
            &by(77, &[0xA1, 0xB2]),
            true,
            true,
        );
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!(shown[0].kind, top_up);
        assert_eq!(shown[0].amount, -214_359);
        // The next restart: the same.
        let again = PoolHistory::new(path).merge_scan(
            vec![restored(78, ActivityKind::PoolSent, 0)],
            &by(78, &[0xA1, 0xC3]),
            true,
            true,
        );
        assert_eq!(again.len(), 1, "{again:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-320 p. 2: met with exactly one kept line that the scan knows better — that line takes
    /// it and the batch's tags, under its own id.
    #[test]
    fn met_with_one_poorer_line_it_is_updated() {
        let (dir, path) = fresh("richer");
        let pool = PoolHistory::new(path.clone());
        pool.merge_scan(
            vec![restored(5, ActivityKind::PoolSent, 0)],
            &by(5, &[0x11]),
            true,
            false,
        );
        let richer = restored(6, ActivityKind::PoolSent, -150_000);
        let shown = pool.merge_scan(vec![richer], &by(6, &[0x11, 0x12]), true, false);
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!((shown[0].id, shown[0].amount), (5, -150_000));
        let kept = PoolHistory::new(path).records();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].tags, tags_of(&[0x11, 0x12]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-320 r1: a kept line met by tags is shown with the scan's proof and block, as one met
    /// by id is — not with the «Proved» it was kept with.
    #[test]
    fn met_by_tags_the_line_has_the_scans_proof() {
        let (dir, path) = fresh("proof");
        let pool = PoolHistory::new(path.clone());
        let spent = op(1, ActivityKind::PoolSent, -100_000, Some(10_000));
        pool.merge_scan(vec![spent], &by(1, &[0x41]), true, false);
        let pending = Activity {
            proof: Proof::Mempool,
            block: Some(4_242),
            ..restored(8, ActivityKind::PoolSent, 0)
        };
        let shown = pool.merge_scan(vec![pending], &by(8, &[0x41, 0x42]), true, false);
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!(
            (shown[0].id, shown[0].proof, shown[0].block),
            (1, Proof::Mempool, Some(4_242))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-320 p. 2: a batch over several kept operations adds nothing and changes none of them.
    #[test]
    fn met_with_several_lines_nothing_changes() {
        let (dir, path) = fresh("several");
        let pool = PoolHistory::new(path.clone());
        let a = op(1, ActivityKind::PoolSent, -100_000, Some(10_000));
        let b = op(2, ActivityKind::PoolSent, -200_000, Some(10_000));
        let tags = HashMap::from([(1, tags_of(&[0x21])), (2, tags_of(&[0x22]))]);
        pool.merge_scan(vec![a.clone(), b.clone()], &tags, true, false);
        let before = pool.records();
        let shown = pool.merge_scan(
            vec![restored(9, ActivityKind::PoolSent, -50_000)],
            &by(9, &[0x21, 0x22]),
            true,
            false,
        );
        assert_eq!(shown.len(), 2, "{shown:?}");
        assert_eq!(pool.records(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-320 p. 2: tags that meet nothing kept — a new line, as before.
    #[test]
    fn tags_that_meet_nothing_are_a_new_line() {
        let (dir, path) = fresh("new");
        let pool = PoolHistory::new(path.clone());
        pool.merge_scan(
            vec![op(1, ActivityKind::PoolReceived, 1_000_000, None)],
            &by(1, &[0x31]),
            true,
            false,
        );
        let shown = pool.merge_scan(
            vec![op(2, ActivityKind::PoolSent, -100_000, Some(10_000))],
            &by(2, &[0x32]),
            true,
            false,
        );
        assert_eq!(shown.len(), 2);
        assert_eq!(PoolHistory::new(path).records().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-320 p. 2: the empty lines kept before tags (three on QA's copy) go once a complete scan
    /// does not give them — never on a scan that may not have given everything yet.
    #[test]
    fn old_empty_lines_go_only_after_a_complete_scan() {
        let (dir, path) = fresh("cleanup");
        let empty = restored(3, ActivityKind::PoolSent, 0);
        let dated = op(4, ActivityKind::PoolSent, -214_359, Some(114_359));
        PoolHistory::new(path.clone()).merge(vec![empty.clone(), dated.clone()]);
        let partial =
            PoolHistory::new(path.clone()).merge_scan(Vec::new(), &HashMap::new(), true, false);
        assert_eq!(
            partial.len(),
            2,
            "not on a scan that may not have given all"
        );
        let complete =
            PoolHistory::new(path.clone()).merge_scan(Vec::new(), &HashMap::new(), true, true);
        assert_eq!(
            complete,
            vec![dated.clone()],
            "the empty one went, the dated stayed"
        );
        assert_eq!(PoolHistory::new(path).kept(), vec![dated]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-103 with BUG-320 p. 2: a restored wallet's real spend the scan gives as 0 stays —
    /// the complete scan gives it, and it is kept with its tags now.
    #[test]
    fn the_scans_own_empty_line_stays() {
        let (dir, path) = fresh("bug103");
        let empty = restored(5, ActivityKind::PoolSent, 0);
        PoolHistory::new(path.clone()).merge(vec![empty.clone()]);
        let shown = PoolHistory::new(path.clone()).merge_scan(
            vec![empty.clone()],
            &by(5, &[0x51]),
            true,
            true,
        );
        assert_eq!(shown, vec![empty]);
        let kept = PoolHistory::new(path).records();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].tags, tags_of(&[0x51]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-320 p. 2: a file from before the tags reads as it did; the field is left out while
    /// there are none, so an older build reads the new file too.
    #[test]
    fn a_file_from_before_the_tags_is_read() {
        let (dir, path) = fresh("old");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            r#"[{"id":1,"kind":"Sent","amount":-214359,"fee":114359,"at":1795607576,"block":739336}]"#,
        )
        .unwrap();
        let kept = PoolHistory::new(path).records();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].tags.is_empty());
        let text = serde_json::to_string(&kept[0]).unwrap();
        assert!(!text.contains("tags"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-320 p. 2 (the architect): the file names the wallet's notes by their tags only —
    /// no commitment in it, as bytes, hex or base64.
    #[test]
    fn no_note_commitment_is_in_the_file() {
        let (dir, path) = fresh("plain");
        let cmx = [0xC7u8; 32];
        PoolHistory::new(path.clone()).merge_scan(
            vec![op(1, ActivityKind::PoolSent, -100_000, Some(10_000))],
            &HashMap::from([(1, vec![tag(&KEY, &cmx)])]),
            true,
            false,
        );
        let text = std::fs::read_to_string(&path).unwrap();
        let bytes: Vec<String> = cmx.iter().map(u8::to_string).collect();
        assert!(!text.contains(&bytes[..8].join(",")), "{text}");
        assert!(!text.to_lowercase().contains("c7c7c7c7"), "{text}");
        assert!(text.contains("tags"), "the tag is there: {text}");
        assert_ne!(
            tag(&KEY, &cmx),
            tag(&[8; 32], &cmx),
            "another key, another tag"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-320 p. 2 (the architect's condition): without the key — the wallet locked — lines are
    /// tied by id only; a new empty undated line is shown but not kept (it may be a batch the key
    /// would have tied), and nothing is cleaned up.
    #[test]
    fn without_the_key_by_id_only_and_nothing_kept_or_dropped() {
        let (dir, path) = fresh("locked");
        let empty = restored(3, ActivityKind::PoolSent, 0);
        PoolHistory::new(path.clone()).merge(vec![empty.clone()]);
        let batch = restored(88, ActivityKind::PoolSent, 0);
        let shown = PoolHistory::new(path.clone()).merge_scan(
            vec![batch.clone()],
            &HashMap::new(),
            false,
            true,
        );
        assert_eq!(shown.len(), 2, "shown: {shown:?}");
        let kept = PoolHistory::new(path).records();
        assert_eq!(
            kept.len(),
            1,
            "the batch not kept, the old line not dropped"
        );
        assert_eq!(kept[0].id, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
