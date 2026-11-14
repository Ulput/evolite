//! Funding the wallet's own Platform address from Core (FEAT-0015 stage 2): the core checks
//! all the form sees before anything is locked (AC-T4) — the network, the threshold, the
//! address being the wallet's own, Core covering the lock and its fee, the sync — then locks and
//! credits in the background. A funding cut short after the lock is finished with
//! `ResumeFundAddress`, from the same lock, without new money (AC-T5).

use crate::real::*;
use evolite_api::address_fund::{FundRefusal, fund_plan};
use evolite_chain::chain::address_fund::FundLock;

/// The words for a funding the plan refuses.
fn refusal(why: FundRefusal) -> CoreError {
    match why {
        FundRefusal::BelowLeast { .. } => CoreError::BadAmount,
        FundRefusal::NotEnough { .. } => CoreError::NotEnough,
    }
}

/// Whether the funding waiting for «Continue» holds a new one back (AC-T5): it does until its
/// «Continue» fails in this run (BUG-160) — then a new lock may be made, and the old one is
/// still offered.
fn holds_back(
    waiting: Option<&FundLock>,
    resume_failed: &std::collections::HashSet<String>,
) -> bool {
    waiting.is_some_and(|l| !resume_failed.contains(&l.txid))
}

/// AC-T11 (the customer, 2026-11-14): not on mainnet until it is measured there.
fn on_this_network(network: evolite_api::Network) -> Result<(), CoreError> {
    if network == evolite_api::Network::Mainnet {
        return Err(CoreError::NotAvailable(
            tr!("Topping up the Platform address from Core is not available on mainnet yet").into(),
        ));
    }
    Ok(())
}

impl Real {
    /// The wallet, its keys, and the address to credit — the one the wallet shows, its own
    /// (AC-T3: the funding goes nowhere else) — once the network, the lock and the sync allow.
    async fn fund_ready(
        &self,
    ) -> Result<
        (
            Arc<platform_wallet::PlatformWallet>,
            key_wallet::Wallet,
            PlatformAddress,
        ),
        CoreError,
    > {
        on_this_network(crate::network::current())?;
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let status = self.chain.status();
        if !coins_known(&status, true)
            || status.phase == Phase::Error
            || !self.chain.platform_ready()
        {
            return Err(CoreError::NotSynced);
        }
        let keys = open
            .keys
            .clone()
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        let wallet = loaded(&self.chain, &open.id).await?;
        let (address, _) = self
            .chain
            .platform_address(&wallet)
            .await
            .map_err(CoreError::Failed)?;
        let to =
            PlatformAddress::from_bech32m_string(&address).map_err(|_| CoreError::BadAddress)?;
        Ok((wallet, keys, to))
    }

    /// Checks everything the form sees and funds the wallet's Platform address in the
    /// background so that it gets at least `amount` duffs. The answer is
    /// `Done { what: DoneWhat::FundAddress }`.
    pub(in crate::real) async fn fund_address(&mut self, amount: Duffs) -> Result<(), CoreError> {
        let amount = positive_duffs(amount)? as Duffs;
        let (wallet, keys, to) = self.fund_ready().await?;
        // A cut-short funding is finished first, with the money already locked for it — unless
        // its «Continue» failed in this run (BUG-160: never held back for good).
        let waiting = self.waiting_fund(&wallet).await;
        let failed = self
            .fund_resume_failed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if holds_back(waiting.as_ref(), &failed) {
            return Err(CoreError::NotAvailable(
                tr!("First finish the stopped top-up of the Platform address — its money is already locked").into(),
            ));
        }
        let fees = self.fees_now(0).await;
        let spendable = {
            let state = wallet.state().await;
            convert::core_balance(&state.core_wallet) - convert::unsignable(&state.core_wallet)
        };
        let plan = fund_plan(amount, false, spendable, &fees).map_err(refusal)?;
        self.fund_in_background(wallet, keys, to, plan.lock as u64, None)
    }

    /// Finishes the cut-short funding in the background with the lock already made: no new
    /// money. Refused when nothing waits (an earlier one went through: no second transition).
    pub(in crate::real) async fn resume_fund_address(&mut self) -> Result<(), CoreError> {
        let (wallet, keys, to) = self.fund_ready().await?;
        let Some(waiting) = self.waiting_fund(&wallet).await else {
            return Err(CoreError::NotAvailable(
                tr!("Nothing to continue: the top-up of the Platform address is already credited")
                    .into(),
            ));
        };
        self.fund_in_background(wallet, keys, to, 0, Some(waiting))
    }

    /// The funding of the address that waits for «Continue» (the chain's `waiting_lock`):
    /// made here and cut short, or found on chain after a restore (AC-T6); never one Platform
    /// proved it took (BUG-160).
    pub(in crate::real) async fn waiting_fund(
        &self,
        wallet: &platform_wallet::PlatformWallet,
    ) -> Option<FundLock> {
        let open = self.chain.open_fund_locks(wallet).await;
        evolite_chain::chain::address_fund::waiting_lock(&open, |txid| {
            self.lock_status.answer(txid)
        })
        .cloned()
    }

    /// One funding at a time, as one asset lock at a time (an identity's too: they draw on the
    /// same coins).
    fn fund_in_background(
        &self,
        wallet: Arc<platform_wallet::PlatformWallet>,
        keys: key_wallet::Wallet,
        to: PlatformAddress,
        lock: u64,
        resumed: Option<FundLock>,
    ) -> Result<(), CoreError> {
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made or topped up").into(),
            ));
        }
        let chain = self.chain.clone();
        let busy = self.identity_busy.clone();
        let refresh = self.refresher();
        let failed = self.fund_resume_failed.clone();
        let lock_status = self.lock_status.clone();
        self.answer_later("task:fund_address", async move {
            // QA's `--stop-after asset-lock` (dev-tools, BUG-160): the lock alone, then the
            // process ends before Platform is asked.
            #[cfg(feature = "dev-stops")]
            if resumed.is_none() && crate::dev_stop::wanted_after_asset_lock() {
                let locked = chain.lock_for_address_only(&wallet, &keys, lock).await;
                crate::diag::record_now(crate::diag::Record::Answer {
                    what: "stop_after_asset_lock",
                    outcome: if locked.is_ok() { "exit" } else { "failed" },
                });
                if let Err(e) = locked {
                    busy.store(false, Ordering::Relaxed);
                    refresh();
                    return Event::Done {
                        what: DoneWhat::FundAddress,
                        result: Err(identity_failure(e)),
                    };
                }
                std::process::exit(0);
            }
            let existing = resumed.as_ref().map(|l| l.out_point);
            let result = chain
                .fund_address(&wallet, &keys, to, lock, existing)
                .await
                .map_err(identity_failure);
            // BUG-160: a «Continue» that failed holds no new funding back, and Platform is
            // asked again whether it took the lock (then «Continue» goes).
            if let (Some(resumed), Err(_)) = (&resumed, &result) {
                failed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(resumed.txid.clone());
                lock_status.ask_again(&resumed.txid);
            }
            busy.store(false, Ordering::Relaxed);
            refresh();
            Event::Done {
                what: DoneWhat::FundAddress,
                result,
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-160: a stopped funding holds a new one back until its «Continue» fails in this run.
    #[test]
    fn a_failed_continue_does_not_hold_a_new_funding_back() {
        use key_wallet::dashcore::hashes::Hash;
        let lock = FundLock {
            txid: "ab".repeat(32),
            out_point: key_wallet::dashcore::OutPoint::new(
                key_wallet::dashcore::Txid::from_byte_array([0xab; 32]),
                0,
            ),
            amount: 10_012_000,
            made_here: true,
        };
        let mut failed = std::collections::HashSet::new();
        assert!(!holds_back(None, &failed), "nothing stopped");
        assert!(
            holds_back(Some(&lock), &failed),
            "finish the stopped one first"
        );
        failed.insert(lock.txid.clone());
        assert!(!holds_back(Some(&lock), &failed), "its «Continue» failed");
    }

    /// AC-T11: mainnet answers «not available yet»; testnet goes on.
    #[test]
    fn not_on_mainnet_yet() {
        assert!(matches!(
            on_this_network(evolite_api::Network::Mainnet),
            Err(CoreError::NotAvailable(_))
        ));
        assert_eq!(on_this_network(evolite_api::Network::Testnet), Ok(()));
    }

    /// The plan's refusals in the interface's words: under the threshold is a wrong amount,
    /// more than Core holds is «not enough».
    #[test]
    fn a_refused_plan_reads_as_the_form_expects() {
        assert_eq!(
            refusal(FundRefusal::BelowLeast { least: 36_000 }),
            CoreError::BadAmount
        );
        assert_eq!(
            refusal(FundRefusal::NotEnough { most: 1 }),
            CoreError::NotEnough
        );
    }
}
