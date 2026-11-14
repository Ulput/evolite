//! Funding the wallet's own Platform address from Core (FEAT-0015 stage 2): the core checks
//! all the form sees before anything is locked (AC-T4) — the network, the threshold, the
//! address being the wallet's own, Core covering the lock and its fee, the sync — then locks and
//! credits in the background. A funding cut short after the lock is finished with
//! `ResumeFundAddress`, from the same lock, without new money (AC-T5).

use crate::real::*;
use evolite_api::address_fund::{FundRefusal, fund_plan};

/// The words for a funding the plan refuses.
fn refusal(why: FundRefusal) -> CoreError {
    match why {
        FundRefusal::BelowLeast { .. } => CoreError::BadAmount,
        FundRefusal::NotEnough { .. } => CoreError::NotEnough,
    }
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
        // A cut-short funding is finished first, with the money already locked for it.
        if self.chain.unfinished_address_fund(&wallet).await.is_some() {
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
        self.fund_in_background(wallet, keys, to, plan.lock as u64)
    }

    /// Finishes the cut-short funding in the background with the lock already made: no new
    /// money. Refused when nothing waits (an earlier one went through: no second transition).
    pub(in crate::real) async fn resume_fund_address(&mut self) -> Result<(), CoreError> {
        let (wallet, keys, to) = self.fund_ready().await?;
        if self.chain.unfinished_address_fund(&wallet).await.is_none() {
            return Err(CoreError::NotAvailable(
                tr!("Nothing to continue: the top-up of the Platform address is already credited")
                    .into(),
            ));
        }
        self.fund_in_background(wallet, keys, to, 0)
    }

    /// One funding at a time, as one asset lock at a time (an identity's too: they draw on the
    /// same coins).
    fn fund_in_background(
        &self,
        wallet: Arc<platform_wallet::PlatformWallet>,
        keys: key_wallet::Wallet,
        to: PlatformAddress,
        lock: u64,
    ) -> Result<(), CoreError> {
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made or topped up").into(),
            ));
        }
        let chain = self.chain.clone();
        let busy = self.identity_busy.clone();
        let refresh = self.refresher();
        self.answer_later("task:fund_address", async move {
            let result = chain
                .fund_address(&wallet, &keys, to, lock)
                .await
                .map_err(identity_failure);
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
