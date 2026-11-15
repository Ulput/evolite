//! The wallet's own InstantLocks across a restart (BUG-173 part 1): keep the bytes of a lock this
//! run verified while its transaction waits for a block; after a restart such a transaction reads
//! «Confirmed · checking», not «Pending». The coin is not final for spending until the lock is
//! verified again (R-1 unchanged; part 2).

use crate::real::*;
use evolite_api::Proof;

/// What the snapshot does with the kept locks, from its Core lines.
#[derive(Debug, Default, PartialEq)]
pub(in crate::real) struct Changes {
    /// Locks verified in this run whose transaction is not in a block: kept (txid, bytes).
    pub keep: Vec<(String, Vec<u8>)>,
    /// Transactions now in a ChainLocked block: their locks go.
    pub drop: Vec<String>,
    /// Lines «Pending» whose lock is kept from before: «Confirmed · checking».
    pub checking: Vec<String>,
}

/// From each Core line's txid and proof: which locks to keep, which to drop, which lines read
/// «Confirmed · checking». `kept`: a lock of the txid is kept; `bytes`: this run's verified
/// lock of the txid, while the chain keeps it. A line «In a block» keeps its lock until the
/// ChainLock and shows the block.
pub(in crate::real) fn changes<'a>(
    lines: impl Iterator<Item = (&'a str, Proof)>,
    kept: impl Fn(&str) -> bool,
    bytes: impl Fn(&str) -> Option<Vec<u8>>,
) -> Changes {
    let mut out = Changes::default();
    for (txid, proof) in lines {
        match proof {
            Proof::InstantLock if !kept(txid) => {
                if let Some(bytes) = bytes(txid) {
                    out.keep.push((txid.to_owned(), bytes));
                }
            }
            Proof::ChainLock if kept(txid) => out.drop.push(txid.to_owned()),
            Proof::Mempool if kept(txid) => out.checking.push(txid.to_owned()),
            _ => {}
        }
    }
    out
}

impl Real {
    /// Keeps the wallet's verified InstantLocks of transactions not in a block, drops those now
    /// ChainLocked, and shows «Confirmed · checking» for a «Pending» line whose lock is kept.
    pub(in crate::real) fn keep_instant_locks(&self, history: &mut [Activity]) {
        let chain = &self.chain;
        let found = changes(
            history
                .iter()
                .filter(|a| a.chain != evolite_api::Chain::Platform)
                .filter_map(|a| Some((a.txid.as_deref()?, a.proof))),
            |txid| self.kept_locks.contains(txid),
            |txid| {
                let txid: key_wallet::dashcore::Txid = txid.parse().ok()?;
                chain.instant_lock_bytes(&txid)
            },
        );
        if !found.keep.is_empty() || !found.drop.is_empty() {
            self.kept_locks.update(
                found.keep,
                &found.drop,
                evolite_chain::fund_intents::unix_now(),
            );
        }
        for act in history.iter_mut() {
            if act.proof == Proof::Mempool
                && act
                    .txid
                    .as_ref()
                    .is_some_and(|t| found.checking.contains(t))
            {
                act.proof = Proof::InstantLockChecking;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-173 AC-T1, AC-T3, AC-T4: a lock verified in this run is kept while its transaction is
    /// out of a block; after a restart that «Pending» line reads «Confirmed · checking»; in a
    /// ChainLocked block its lock goes; «In a block» keeps it; a lock not kept says nothing.
    #[test]
    fn a_verified_lock_is_kept_and_read_back_as_checking() {
        let lines = [
            ("verified", Proof::InstantLock),
            ("after-restart", Proof::Mempool),
            ("locked-block", Proof::ChainLock),
            ("in-block", Proof::InBlock),
            ("stranger", Proof::Mempool),
        ];
        let kept = |t: &str| matches!(t, "after-restart" | "locked-block" | "in-block");
        let found = changes(lines.into_iter(), kept, |t| {
            (t == "verified").then(|| vec![7])
        });
        assert_eq!(
            found,
            Changes {
                keep: vec![("verified".into(), vec![7])],
                drop: vec!["locked-block".into()],
                checking: vec!["after-restart".into()],
            }
        );
    }
}
