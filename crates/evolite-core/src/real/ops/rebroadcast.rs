//! FEAT-0046: «Broadcast again» a stuck payment — the same signed bytes, no password, nothing
//! new spent. Behind the advanced switch (AC-A9); once a minute per payment (R-6, AC-T-T7). The
//! journal says only that it was asked (R-4): no txid, no address.

use crate::real::*;
use evolite_chain::chain::rebroadcast::RebroadcastError;

/// R-6: how long after a broadcast again the next one waits, seconds.
pub(in crate::real) const AGAIN_AFTER: Timestamp = 60;

impl Real {
    /// `Command::Rebroadcast`.
    pub(in crate::real) async fn rebroadcast(&self, txid: String) -> Result<(), CoreError> {
        if !self.advanced_features {
            return Err(CoreError::NotAvailable(evolite_common::ADVANCED_OFF.into()));
        }
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let now = now();
        if let Some(at) = self.mempool.resent_at(&txid)
            && now.saturating_sub(at) < AGAIN_AFTER
        {
            return Err(CoreError::NotAvailable(format!(
                tr!("You can again in {} s"),
                AGAIN_AFTER - now.saturating_sub(at)
            )));
        }
        let wallet = loaded(&self.chain, &open.id).await?;
        self.chain
            .rebroadcast(&wallet, &txid)
            .await
            .map_err(|e| match e {
                RebroadcastError::NoPeers => CoreError::NotAvailable(
                    tr!(
                        "Could not broadcast: no connection to nodes. Try when the network is back"
                    )
                    .into(),
                ),
                RebroadcastError::Replaced => CoreError::NotAvailable(
                    tr!("This payment has already been replaced by another").into(),
                ),
                RebroadcastError::NotFound | RebroadcastError::NotOurs => {
                    CoreError::NotAvailable(tr!("This payment cannot be broadcast again").into())
                }
                RebroadcastError::Confirmed => {
                    CoreError::NotAvailable(tr!("The payment is already confirmed").into())
                }
                // Review r1, Could 2: words of its own; the error's text stays out (the journal
                // has the command's outcome by its kind).
                RebroadcastError::Failed(_) => {
                    CoreError::NotAvailable(tr!("The nodes did not take the payment").into())
                }
            })?;
        let tip = self.chain.status().headers.map_or(0, |h| h.height);
        self.mempool.resent(&txid, tip, now);
        Ok(())
    }
}
