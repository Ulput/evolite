//! FEAT-0046: «Broadcast again» — the same signed bytes of a stuck payment of this wallet sent to
//! the nodes once more. Nothing is signed or spent anew: the txid is the same, so no second
//! payment can come of it. dash-spv already does this every 10 minutes while evolite runs; the
//! button is mostly for after a restart.

use key_wallet::managed_account::transaction_record::TransactionRecord;

use crate::convert::resendable;

use super::Chain;

/// Why a payment was not broadcast again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebroadcastError {
    /// No Core node is connected: nothing is tried (AC-T-T5).
    NoPeers,
    /// The wallet keeps no such transaction (its store was cleared, or it was abandoned — an
    /// abandoned one has no record left).
    NotFound,
    /// Not one that is broadcast again: incoming, an asset lock, a withdrawal, a mixing round
    /// (R-5, AC-T-T3).
    NotOurs,
    /// Another transaction of the wallet spends one of its coins: it was replaced (AC-T-T4).
    Replaced,
    /// It is in a block already (review r1, Could 1): a frame with an older snapshot asked.
    Confirmed,
    /// The nodes refused it; the error's text, for the journal's kind only.
    Failed(String),
}

/// AC-T-T4: whether another transaction of `all` spends one of `record`'s coins — then `record`
/// was replaced, and broadcasting it again would be a double spend the network refuses.
pub fn replaced(record: &TransactionRecord, all: &[&TransactionRecord]) -> bool {
    let txid = record.transaction.txid();
    let inputs: std::collections::HashSet<_> = record
        .transaction
        .input
        .iter()
        .map(|i| i.previous_output)
        .collect();
    all.iter()
        .filter(|r| r.transaction.txid() != txid)
        .flat_map(|r| r.transaction.input.iter())
        .any(|i| inputs.contains(&i.previous_output))
}

impl Chain {
    /// FEAT-0046: sends the transaction `txid` of `wallet` to the nodes again, as it was signed.
    /// Accepted and «not sure» are one for the person: broadcast (the architect, answer 4).
    pub async fn rebroadcast(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        txid: &str,
    ) -> Result<(), RebroadcastError> {
        use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
        if self.status().peers == 0 {
            return Err(RebroadcastError::NoPeers);
        }
        let transaction = {
            let state = wallet.state().await;
            let accounts = state.core_wallet.accounts();
            let all: Vec<&TransactionRecord> = accounts
                .all_accounts()
                .into_iter()
                .flat_map(|a| a.transactions().values())
                .collect();
            let record = all
                .iter()
                .find(|r| r.transaction.txid().to_string() == txid)
                .ok_or(RebroadcastError::NotFound)?;
            // Confirmed first: it is said as such, not as «not one to broadcast again».
            if crate::convert::confirmed(record) {
                return Err(RebroadcastError::Confirmed);
            }
            if !resendable(record) {
                return Err(RebroadcastError::NotOurs);
            }
            if replaced(record, &all) {
                return Err(RebroadcastError::Replaced);
            }
            record.transaction.clone()
        };
        match wallet.core().broadcast_transaction(&transaction).await {
            Ok(_)
            | Err(platform_wallet::PlatformWalletError::TransactionBroadcastUnconfirmed(_)) => {
                Ok(())
            }
            Err(e) => Err(RebroadcastError::Failed(e.to_string())),
        }
    }
}
