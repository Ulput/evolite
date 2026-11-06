//! The block each of the wallet's Core transactions was seen in, kept in `mined.json` beside the
//! store (public data: txid, height, block hash and time).
//!
//! BUG-64: key-wallet can take a mined transaction back out of its block — dash-spv's fork
//! recovery (`truncate_above`) leaves an InstantSend-locked one in the wallet as `Mempool` — and
//! the store then keeps it so; the sync goes on above that block and never puts it back. The
//! history showed a payment of weeks ago as "Pending · today" from then on, at every start.
//! With this file such a record is shown in the block it was seen in, and the wallet is asked to
//! scan its filters again from there, so the store gets the block back too. Only a final block
//! is kept (chain-locked, or at or below the verified ChainLock, review r1 must): a block a fork
//! may still take away is not, so a transaction really reorged out is never shown final.

use key_wallet::managed_account::transaction_record::TransactionRecord;
use key_wallet::transaction_checking::{BlockInfo, TransactionContext};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Block {
    height: u32,
    hash: String,
    time: u32,
    /// A rescan for the record out of this block ran to its end (BUG-64 review r2 should 2):
    /// not asked again for it, across restarts too, until the record is in a block again.
    #[serde(default)]
    rescanned: bool,
}

/// What [`Mined::recall`] put back into its block with no finished rescan for it yet: the
/// filters are to be scanned again from below `from`, then [`Mined::rescanned`] notes `txids`.
#[derive(Clone, Debug, PartialEq)]
pub struct Lost {
    pub from: u32,
    pub txids: Vec<String>,
}

pub struct Mined {
    path: PathBuf,
    blocks: Mutex<Option<HashMap<String, Block>>>,
}

impl Mined {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            blocks: Mutex::new(None),
        }
    }

    fn load(&self) -> HashMap<String, Block> {
        super::json_file::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Notes the block of every record in a final one — chain-locked, or at or below
    /// `chainlocked`, the verified ChainLock's height — and puts a record that came back without
    /// a block into the final block it was seen in. Returns what was put back and not rescanned
    /// for yet ([`Lost`]): the filters are to be scanned again from below its lowest height. A
    /// final block the file does not know yet for its record is written down (one write).
    pub fn recall(
        &self,
        records: &mut [TransactionRecord],
        chainlocked: Option<u32>,
    ) -> Option<Lost> {
        let mut guard = self.blocks.lock().unwrap_or_else(|e| e.into_inner());
        let blocks = guard.get_or_insert_with(|| self.load());
        let mut changed = false;
        let mut lost: Option<Lost> = None;
        for r in records.iter_mut() {
            let txid = r.txid.to_string();
            match r.context.block_info() {
                Some(info) => {
                    let is_final = r.context.is_chain_locked()
                        || chainlocked.is_some_and(|locked| info.height() <= locked);
                    if !is_final {
                        continue;
                    }
                    // In its block again: a later loss is rescanned for once more.
                    let block = Block {
                        height: info.height(),
                        hash: info.block_hash().to_string(),
                        time: info.timestamp(),
                        rescanned: false,
                    };
                    if blocks.get(&txid) != Some(&block) {
                        blocks.insert(txid, block);
                        changed = true;
                    }
                }
                None => {
                    let Some(b) = blocks.get(&txid) else { continue };
                    let Ok(hash) = key_wallet::dashcore::BlockHash::from_str(&b.hash) else {
                        continue;
                    };
                    r.context = TransactionContext::InChainLockedBlock(BlockInfo::new(
                        b.height, hash, b.time,
                    ));
                    if b.rescanned {
                        continue;
                    }
                    let l = lost.get_or_insert(Lost {
                        from: b.height,
                        txids: Vec::new(),
                    });
                    l.from = l.from.min(b.height);
                    l.txids.push(txid);
                }
            }
        }
        if changed {
            let _ = super::json_file::save(&self.path, blocks);
        }
        lost
    }

    /// Notes that a rescan for `txids` ([`Lost::txids`]) ran to its end (kept in the file).
    pub fn rescanned(&self, txids: &[String]) {
        let mut guard = self.blocks.lock().unwrap_or_else(|e| e.into_inner());
        let blocks = guard.get_or_insert_with(|| self.load());
        let mut changed = false;
        for t in txids {
            if let Some(b) = blocks.get_mut(t)
                && !b.rescanned
            {
                b.rescanned = true;
                changed = true;
            }
        }
        if changed {
            let _ = super::json_file::save(&self.path, blocks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use key_wallet::dashcore::hashes::Hash;

    /// A payment in, told apart by its lock time.
    fn record(seed: u8, context: TransactionContext) -> TransactionRecord {
        let transaction = key_wallet::dashcore::Transaction {
            version: 2,
            lock_time: u32::from(seed),
            input: Vec::new(),
            output: Vec::new(),
            special_transaction_payload: None,
        };
        TransactionRecord::new(
            transaction,
            key_wallet::account::AccountType::Standard {
                index: 0,
                standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
            },
            context,
            key_wallet::transaction_checking::TransactionType::Standard,
            key_wallet::managed_account::transaction_record::TransactionDirection::Incoming,
            Vec::new(),
            Vec::new(),
            100_000_000,
        )
    }

    fn block(height: u32) -> TransactionContext {
        TransactionContext::InChainLockedBlock(BlockInfo::new(
            height,
            key_wallet::dashcore::BlockHash::from_byte_array([height as u8; 32]),
            1_792_000_000 + height,
        ))
    }

    /// BUG-64: a record that loses its block — in this run, or before a restart — is shown in
    /// the block it was seen in, with that block's time; one never in a block stays out of one.
    #[test]
    fn a_record_that_lost_its_block_gets_it_back() {
        let dir = std::env::temp_dir().join(format!("evolite-mined-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("mined.json");
        let mined = Mined::new(path.clone());
        let mut seen = vec![record(1, block(1_579_644)), record(2, block(1_580_000))];
        assert_eq!(mined.recall(&mut seen, None), None, "nothing lost");

        // The next start: the store gives the first one back without its block.
        let again = Mined::new(path.clone());
        let mut after = vec![
            record(1, TransactionContext::Mempool),
            record(2, block(1_580_000)),
            record(3, TransactionContext::Mempool),
        ];
        assert_eq!(
            again.recall(&mut after, None).map(|l| l.from),
            Some(1_579_644)
        );
        assert_eq!(after[0].context, seen[0].context);
        assert_eq!(
            after[0].context.block_info().map(|b| b.timestamp()),
            Some(1_793_579_644)
        );
        assert_eq!(
            after[2].context,
            TransactionContext::Mempool,
            "never in a block"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn in_block(height: u32) -> TransactionContext {
        TransactionContext::InBlock(BlockInfo::new(
            height,
            key_wallet::dashcore::BlockHash::from_byte_array([height as u8; 32]),
            1_792_000_000 + height,
        ))
    }

    /// Review r1 (must): a block above the verified ChainLock is not kept — a fork may still take
    /// it away, and a transaction reorged out must not be shown final; one at or below the
    /// ChainLock is final and kept.
    #[test]
    fn only_a_final_block_is_kept() {
        let dir = std::env::temp_dir().join(format!("evolite-mined-cl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("mined.json");
        let mined = Mined::new(path.clone());
        // ChainLock at 1 600 000: the first is above it, the second at it.
        let mut seen = vec![
            record(1, in_block(1_600_001)),
            record(2, in_block(1_600_000)),
        ];
        assert_eq!(mined.recall(&mut seen, Some(1_600_000)), None);
        let mut lost = vec![
            record(1, TransactionContext::Mempool),
            record(2, TransactionContext::Mempool),
        ];
        assert_eq!(
            Mined::new(path)
                .recall(&mut lost, Some(1_600_000))
                .map(|l| l.from),
            Some(1_600_000)
        );
        assert_eq!(
            lost[0].context,
            TransactionContext::Mempool,
            "reorged out: waits"
        );
        assert!(lost[1].context.is_chain_locked(), "final: in its block");
        assert_eq!(
            lost[1].context.block_info().map(|b| b.height()),
            Some(1_600_000)
        );
        // Without a verified ChainLock, only a chain-locked record counts.
        let mut unknown = vec![record(3, in_block(1_500_000))];
        let none = Mined::new(dir.join("other.json"));
        none.recall(&mut unknown, None);
        let mut back = vec![record(3, TransactionContext::Mempool)];
        assert_eq!(none.recall(&mut back, None), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Review r2 should 2: a record whose rescan ran to its end is still shown in its block but
    /// asks for no rescan again — across restarts too — until it is in a block again; one lost
    /// after that asks once more. Two lost: one rescan from the lower, both noted.
    #[test]
    fn a_finished_rescan_is_not_asked_again() {
        let dir = std::env::temp_dir().join(format!("evolite-mined-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("mined.json");
        Mined::new(path.clone()).recall(
            &mut [record(1, block(1_500_010)), record(2, block(1_500_000))],
            None,
        );
        let gone = || {
            vec![
                record(1, TransactionContext::Mempool),
                record(2, TransactionContext::Mempool),
            ]
        };
        let mined = Mined::new(path.clone());
        let lost = mined.recall(&mut gone(), None).expect("both lost");
        assert_eq!(lost.from, 1_500_000, "from the lower");
        assert_eq!(lost.txids.len(), 2);
        mined.rescanned(&lost.txids);
        let again = Mined::new(path.clone());
        let mut after = gone();
        assert_eq!(again.recall(&mut after, None), None, "not asked again");
        assert!(
            after.iter().all(|r| r.context.is_chain_locked()),
            "still in its block"
        );
        // Back in its block, then lost once more: one rescan again.
        again.recall(&mut [record(1, block(1_500_010))], None);
        let lost = Mined::new(path)
            .recall(&mut gone(), None)
            .expect("lost again");
        assert_eq!(
            lost.txids,
            vec![record(1, block(1_500_010)).txid.to_string()]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
