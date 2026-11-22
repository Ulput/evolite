//! BUG-275 (the architect's decision, 2026-11-22): a Core payout from Platform says where it
//! came from on any copy — proved from the network, not from the copy that withdrew
//! ([`evolite_chain::pool_payout`]). Each asset unlock to an address of the wallet has its
//! withdrawal document read once, by `transactionIndex`. Only the document is kept (by the
//! unlock's txid, in `pool-payouts.json`: ids, amount, time, script — public data); where it
//! came from is decided again on every snapshot by the notes and identities known then (r1
//! Must 1: on a fresh copy the pool's notes come after the first read — a verdict kept then
//! would say «from Platform» for good). A read that failed, or found no document yet, is not
//! kept and goes again by the shared rule (`evolite_api::retry`), the pass with a deadline; its
//! kind goes to the journal. In the History: a payout out of the pool is «withdrawal from the
//! pool», and the pool's restored row it came from (when the matched note is one its entry
//! links) is «From the pool to your Core balance» with the document's amount and time. Nothing
//! is labelled by amount or time.

use std::collections::{BTreeMap, HashMap};

use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use evolite_api::retry::Retry;
use evolite_api::{Activity, ActivityKind, Timestamp};
use evolite_chain::pool_payout::{Origin, WithdrawalDoc, classify};
use serde::{Deserialize, Serialize};

use crate::real::*;

/// The pass: at most this long, as the other reads of Platform in the background.
const PAYOUTS_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// A withdrawal document as kept: what the network said, nothing decided from it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::real) struct StoredDoc {
    /// Base58.
    pub id: String,
    pub owner: String,
    /// Net, in credits.
    pub amount: u64,
    pub created_ms: Option<u64>,
    /// Hex.
    pub script: String,
}

impl StoredDoc {
    fn of(doc: &WithdrawalDoc) -> Self {
        Self {
            id: doc.id.to_string(Encoding::Base58),
            owner: doc.owner.to_string(Encoding::Base58),
            amount: doc.amount,
            created_ms: doc.created_ms,
            script: hex(&doc.script),
        }
    }

    fn doc(&self) -> Option<WithdrawalDoc> {
        Some(WithdrawalDoc {
            id: Identifier::from_string(&self.id, Encoding::Base58).ok()?,
            owner: Identifier::from_string(&self.owner, Encoding::Base58).ok()?,
            amount: self.amount,
            created_ms: self.created_ms,
            script: unhex(&self.script)?,
        })
    }
}

/// An asset unlock to one of the wallet's addresses, waiting for its document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::real) struct Candidate {
    pub txid: String,
    pub index: u64,
    pub address: String,
}

/// The payouts' state: documents as kept, and this run's reads.
#[derive(Default)]
pub(in crate::real) struct Payouts {
    docs: Option<BTreeMap<String, StoredDoc>>,
    tries: HashMap<String, Retry>,
    /// What the last snapshot found to read.
    candidates: Vec<Candidate>,
    running: bool,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The unlocks of `history` paid to the wallet: the payouts' lines («received · withdrawal from
/// Platform», or «from the pool» already) with their index.
pub(in crate::real) fn candidates(
    history: &[Activity],
    unlocks: &HashMap<String, u64>,
) -> Vec<Candidate> {
    history
        .iter()
        .filter(|a| {
            matches!(&a.kind, ActivityKind::Received { from: Some(f) }
                if f == tr!("withdrawal from Platform") || f == tr!("withdrawal from the pool"))
        })
        .filter_map(|a| {
            let txid = a.txid.clone()?;
            Some(Candidate {
                index: *unlocks.get(&txid)?,
                address: a.address.clone()?,
                txid,
            })
        })
        .collect()
}

/// The History with the documents known, each one's origin decided now by `spent` (the pool's
/// spent notes as this snapshot reads them) and `identities`: a payout out of the pool says so,
/// and the restored pool row whose spend links the matched note becomes «to your Core balance»
/// with the document's amount and time. Any other row stays as it is.
pub(in crate::real) fn apply(
    history: &mut [Activity],
    docs: &BTreeMap<String, StoredDoc>,
    spent: &[[u8; 32]],
    identities: &[Identifier],
    spent_by_operation: &HashMap<u64, Vec<[u8; 32]>>,
) {
    let mut rows: Vec<(u64, Duffs, Option<Timestamp>, Option<String>)> = Vec::new();
    for act in history.iter_mut() {
        let Some(doc) = act
            .txid
            .as_ref()
            .and_then(|t| docs.get(t))
            .and_then(StoredDoc::doc)
        else {
            continue;
        };
        let Origin::Pool { nullifier } = classify(&doc, identities, spent) else {
            continue;
        };
        if let ActivityKind::Received { from } = &mut act.kind {
            *from = Some(tr!("withdrawal from the pool").into());
        }
        if let Some((op, _)) = spent_by_operation
            .iter()
            .find(|(_, nfs)| nfs.contains(&nullifier))
        {
            rows.push((
                *op,
                (doc.amount / 1000) as Duffs,
                doc.created_ms.map(|ms| (ms / 1000) as Timestamp),
                act.address.clone(),
            ));
        }
    }
    for (op, amount, at, address) in rows {
        if let Some(row) = history
            .iter_mut()
            .find(|a| a.id == op && a.chain == evolite_api::Chain::Platform)
        {
            row.kind = ActivityKind::PoolToCore { arrived: true };
            row.amount = -amount;
            if let Some(at) = at {
                row.at = at;
            }
            row.address = address;
            (row.layers, row.flow) = crate::layers::of(&row.kind, row.chain);
        }
    }
}

fn now_secs() -> f64 {
    evolite_common::wall_secs() as f64
}

/// The journal's word for a read that gave no document: not there yet, out of time, or failed.
fn outcome(read: &Result<Option<WithdrawalDoc>, String>) -> &'static str {
    match read {
        Ok(Some(_)) => "found",
        Ok(None) => "not_found",
        Err(e) if e.contains("timed out") || e.contains("no answer") => "timeout",
        Err(_) => "failed",
    }
}

impl Payouts {
    fn docs(&mut self, path: &std::path::Path) -> &mut BTreeMap<String, StoredDoc> {
        self.docs.get_or_insert_with(|| {
            evolite_local::json_file::read_cache(path, |b| serde_json::from_slice(b).ok())
                .unwrap_or_default()
        })
    }

    /// The unlocks to read now: no document yet, and their read due by the shared rule.
    fn due(&mut self, path: &std::path::Path, now: f64) -> Vec<Candidate> {
        let known: Vec<String> = self.docs(path).keys().cloned().collect();
        let candidates = self.candidates.clone();
        candidates
            .into_iter()
            .filter(|c| !known.contains(&c.txid))
            .filter(|c| self.tries.entry(c.txid.clone()).or_default().ask(now))
            .collect()
    }
}

impl Real {
    /// The snapshot's part: what to read (its unlocks) and the History with the documents
    /// known, their origin decided by this snapshot's notes and identities.
    pub(in crate::real) fn payouts_seen(
        &self,
        history: &mut [Activity],
        unlocks: &HashMap<String, u64>,
        spent: &[[u8; 32]],
        spent_by_operation: &HashMap<u64, Vec<[u8; 32]>>,
        identities: &[Identifier],
    ) {
        let path = self.paths.pool_payouts.clone();
        let mut payouts = self.payouts.lock().unwrap_or_else(|e| e.into_inner());
        payouts.candidates = candidates(history, unlocks);
        let docs = payouts.docs(&path).clone();
        apply(history, &docs, spent, identities, spent_by_operation);
    }

    /// Run with each turn of the core: the unlocks with no document yet are read in one pass
    /// in the background, when the wallet is open and Platform can be asked.
    pub(in crate::real) fn payouts_tick(&mut self) {
        if self.unlocked.is_none() || !self.chain.platform_ready() {
            return;
        }
        let path = self.paths.pool_payouts.clone();
        let now = now_secs();
        let due = {
            let mut payouts = self.payouts.lock().unwrap_or_else(|e| e.into_inner());
            if payouts.running {
                return;
            }
            let due = payouts.due(&path, now);
            if due.is_empty() {
                return;
            }
            payouts.running = true;
            due
        };
        let (chain, payouts, dirty) =
            (self.chain.clone(), self.payouts.clone(), self.dirty.clone());
        self.answer_later("task:pool_payouts", async move {
            let read = {
                let chain = chain.clone();
                let due = due.clone();
                async move {
                    let mut out = Vec::new();
                    for c in &due {
                        let found = chain.payout_document(c.index, &c.address).await;
                        out.push((c.txid.clone(), found));
                    }
                    Ok::<_, String>(out)
                }
            };
            let read = crate::real::ops::names::read_within(PAYOUTS_WAIT, read).await;
            let now = now_secs();
            let mut payouts = payouts.lock().unwrap_or_else(|e| e.into_inner());
            payouts.running = false;
            match read {
                Ok(results) => {
                    for (txid, found) in results {
                        evolite_common::diag::record(evolite_common::diag::Record::Answer {
                            what: "pool_payout",
                            outcome: outcome(&found),
                        });
                        match found {
                            Ok(Some(doc)) => {
                                payouts.tries.entry(txid.clone()).or_default().answered();
                                payouts.docs(&path).insert(txid, StoredDoc::of(&doc));
                            }
                            // Not there yet, or not read: asked again after the pause.
                            Ok(None) | Err(_) => {
                                payouts.tries.entry(txid).or_default().failed(now);
                            }
                        }
                    }
                    let kept = payouts.docs(&path).clone();
                    let _ = evolite_local::json_file::save(&path, &kept);
                }
                Err(_) => {
                    evolite_common::diag::record(evolite_common::diag::Record::Answer {
                        what: "pool_payout",
                        outcome: "timeout",
                    });
                    for c in &due {
                        payouts.tries.entry(c.txid.clone()).or_default().failed(now);
                    }
                }
            }
            dirty.store(true, std::sync::atomic::Ordering::Relaxed);
            Event::Progress {
                what: "pool-payouts",
                fraction: 1.0,
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpp::data_contracts::withdrawals_contract;
    use evolite_api::{Chain, Flow, Proof};
    use evolite_chain::pool_payout::pool_withdrawal_id;

    fn line(
        id: u64,
        kind: ActivityKind,
        chain: Chain,
        txid: Option<&str>,
        address: Option<&str>,
    ) -> Activity {
        Activity {
            id,
            kind,
            chain,
            at: 0,
            amount: 0,
            proof: Proof::Proved,
            txid: txid.map(str::to_owned),
            fee: None,
            block: None,
            address: address.map(str::to_owned),
            layers: 0,
            flow: Flow::In,
            key: String::new(),
            detail: None,
            note: None,
            rate: None,
            tax_transfer: false,
        }
    }

    fn payout(id: u64, txid: &str, address: &str) -> Activity {
        line(
            id,
            ActivityKind::Received {
                from: Some(tr!("withdrawal from Platform").into()),
            },
            Chain::Core,
            Some(txid),
            Some(address),
        )
    }

    /// BUG-275 (TEAM.md §8d В1, «read back»): a fresh copy's History — the pool's restored
    /// spend and the payout «withdrawal from Platform» — with the documents the network gave:
    /// the payout says «from the pool», the spend whose entry links the matched note becomes «to
    /// your Core balance» with the document's amount and time. An identity's withdrawal stays as
    /// it was; a row not linked stays bare.
    #[test]
    fn a_fresh_copy_shows_the_withdrawal_to_its_own_core() {
        let nf = [9u8; 32];
        let script = vec![0x76, 0xa9, 0x14, 1];
        let me = Identifier::from([5u8; 32]);
        let mut history = vec![
            line(1, ActivityKind::PoolSent, Chain::Platform, None, None),
            payout(2, "unlock-pool", "yChange"),
            payout(3, "unlock-identity", "yReceive"),
            line(4, ActivityKind::PoolSent, Chain::Platform, None, None),
        ];
        let unlocks: HashMap<String, u64> =
            [("unlock-pool".into(), 41), ("unlock-identity".into(), 42)].into();
        assert_eq!(
            candidates(&history, &unlocks).len(),
            2,
            "both payouts are read"
        );
        let docs: BTreeMap<String, StoredDoc> = [
            (
                "unlock-pool".to_owned(),
                StoredDoc::of(&WithdrawalDoc {
                    id: pool_withdrawal_id(&nf, &script),
                    owner: withdrawals_contract::OWNER_ID,
                    amount: 200_000_000,
                    created_ms: Some(1_763_800_000_000),
                    script: script.clone(),
                }),
            ),
            (
                "unlock-identity".to_owned(),
                StoredDoc::of(&WithdrawalDoc {
                    id: Identifier::from([6u8; 32]),
                    owner: me,
                    amount: 100_000_000,
                    created_ms: None,
                    script: script.clone(),
                }),
            ),
        ]
        .into();
        let spent_by_operation: HashMap<u64, Vec<[u8; 32]>> = [(1, vec![[1u8; 32], nf])].into();
        apply(
            &mut history,
            &docs,
            &[[1u8; 32], nf],
            &[me],
            &spent_by_operation,
        );
        assert_eq!(history[0].kind, ActivityKind::PoolToCore { arrived: true });
        assert_eq!(history[0].amount, -200_000);
        assert_eq!(history[0].at, 1_763_800_000);
        assert_eq!(history[0].address.as_deref(), Some("yChange"));
        assert_eq!(
            history[1].kind,
            ActivityKind::Received {
                from: Some(tr!("withdrawal from the pool").into())
            }
        );
        assert_eq!(
            history[2].kind,
            ActivityKind::Received {
                from: Some(tr!("withdrawal from Platform").into())
            },
            "an identity's withdrawal is not the pool's"
        );
        assert_eq!(
            history[3].kind,
            ActivityKind::PoolSent,
            "not linked: left bare"
        );
    }

    /// r1 Must 1: the pool's notes not scanned yet — the payout is «from Platform»; the note
    /// comes with a later scan — the same kept document says «from the pool», with no new read.
    #[test]
    fn a_note_found_later_names_the_pool_without_a_new_read() {
        let dir =
            std::env::temp_dir().join(format!("evolite-payouts-later-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pool-payouts.json");
        let nf = [3u8; 32];
        let script = vec![0x76, 0xa9, 0x14, 7];
        let mut p = Payouts {
            candidates: vec![Candidate {
                txid: "u".into(),
                index: 9,
                address: "yA".into(),
            }],
            ..Payouts::default()
        };
        p.docs(&path).insert(
            "u".into(),
            StoredDoc::of(&WithdrawalDoc {
                id: pool_withdrawal_id(&nf, &script),
                owner: withdrawals_contract::OWNER_ID,
                amount: 200_000_000,
                created_ms: None,
                script,
            }),
        );
        let docs = p.docs(&path).clone();
        let mut before = vec![payout(1, "u", "yA")];
        apply(&mut before, &docs, &[], &[], &HashMap::new());
        assert_eq!(
            before[0].kind,
            ActivityKind::Received {
                from: Some(tr!("withdrawal from Platform").into())
            },
            "no note yet: not named the pool's"
        );
        let mut after = vec![payout(1, "u", "yA")];
        apply(&mut after, &docs, &[nf], &[], &HashMap::new());
        assert_eq!(
            after[0].kind,
            ActivityKind::Received {
                from: Some(tr!("withdrawal from the pool").into())
            }
        );
        assert!(p.due(&path, 0.0).is_empty(), "known: no new read");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A kept document is not read again; an unlock with none yet is asked again after the
    /// shared pause, never on every turn.
    #[test]
    fn a_known_payout_is_not_read_again_and_a_failed_one_waits() {
        let dir = std::env::temp_dir().join(format!("evolite-payouts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pool-payouts.json");
        let mut p = Payouts {
            candidates: vec![
                Candidate {
                    txid: "a".into(),
                    index: 1,
                    address: "y1".into(),
                },
                Candidate {
                    txid: "b".into(),
                    index: 2,
                    address: "y2".into(),
                },
            ],
            ..Payouts::default()
        };
        p.docs(&path).insert(
            "a".into(),
            StoredDoc {
                id: "x".into(),
                owner: "y".into(),
                amount: 1,
                created_ms: None,
                script: String::new(),
            },
        );
        let due = p.due(&path, 0.0);
        assert_eq!(
            due.iter().map(|c| c.txid.as_str()).collect::<Vec<_>>(),
            ["b"],
            "a is known"
        );
        p.tries.get_mut("b").unwrap().failed(1.0);
        let asked = (0..1_000)
            .filter(|i| !p.due(&path, 1.0 + *i as f64 * 0.02).is_empty())
            .count();
        assert_eq!(asked, 0, "a thousand turns within the pause");
        assert_eq!(p.due(&path, 31.0).len(), 1, "after 30 s");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The kept form reads back as it was written.
    #[test]
    fn a_kept_document_reads_back() {
        let doc = WithdrawalDoc {
            id: Identifier::from([1u8; 32]),
            owner: withdrawals_contract::OWNER_ID,
            amount: 5,
            created_ms: Some(7),
            script: vec![0, 0xab, 0xff],
        };
        assert_eq!(StoredDoc::of(&doc).doc(), Some(doc));
    }
}
