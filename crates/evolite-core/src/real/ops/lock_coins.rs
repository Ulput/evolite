//! An asset lock checked for its coins before the password (SPV Б2 and item 3): too many inputs
//! for the network, or final coins short. One rule with the forms (`Fees::lock_coins`), on the
//! wallet's coins as they are now.

use crate::real::*;
use evolite_api::lock_coins::{LockCoins, lock_coins};

/// What the wallet's coins say of an asset lock of `amount` duffs: the coins it may take are the
/// final ones a payment can sign for (`payable_coins`, by `verified`); `max_inputs` is the
/// platform's limit. Refused when it needs more inputs than the network takes, when the final
/// coins fall short (the others are not confirmed yet), and when the wallet has not enough.
pub(in crate::real) fn lock_coins_allow(
    info: &key_wallet::wallet::ManagedWalletInfo,
    amount: u64,
    verified: impl Fn(&key_wallet::dashcore::Txid) -> bool,
    max_inputs: usize,
) -> Result<(), CoreError> {
    let finals = convert::payable_coins(info, &verified);
    let finals = convert::core_coins(finals.iter().map(|u| u.value()).collect());
    let all = convert::core_coins(
        convert::coins_once(info)
            .iter()
            .map(|u| u.value())
            .collect(),
    );
    let coins = lock_coins(&finals, &all, amount as Duffs, max_inputs);
    match crate::lock_words::lock_refusal(coins) {
        Some(words) => Err(CoreError::NotAvailable(words)),
        None if coins == LockCoins::Short => Err(CoreError::NotEnough),
        None => Ok(()),
    }
}

/// The duffs of the new asset lock `op` makes, as the op itself reckons them: an identity's
/// lock with its name and creation, a top-up's with Platform's fee on top, Core into the pool
/// (none when a cut-short lock is finished instead, `pool_lock_waits`), the Platform address's
/// plan (`spendable`: what Core can sign for). `None`: no new lock, or one the op refuses itself.
pub(in crate::real) fn lock_amount(
    op: &Spend,
    fees: &Fees,
    pool_lock_waits: bool,
    spendable: Duffs,
) -> Option<u64> {
    let amount = match op {
        Spend::CreateIdentity { name, top_up } => u64::try_from(*top_up)
            .ok()
            .and_then(|t| convert::identity_lock(t, fees, name.is_some())),
        Spend::TopUpIdentity { amount } => u64::try_from(*amount)
            .ok()
            .and_then(|a| convert::top_up_lock(a, fees)),
        Spend::Shield {
            from: ShieldSource::Core,
            amount,
        } if !pool_lock_waits => u64::try_from(*amount).ok(),
        Spend::FundAddress { amount } => {
            evolite_api::address_fund::fund_plan(*amount, false, spendable, fees)
                .ok()
                .map(|plan| plan.lock as u64)
        }
        _ => None,
    };
    amount.filter(|a| *a > 0)
}

impl Real {
    /// The asset lock a spend would make of new Core money, checked for its coins before the
    /// password (Б2 AC-T2, item 3): creating an identity, topping one up, Core into the pool
    /// (unless a cut-short lock is finished instead) and topping up the Platform address.
    /// Everything else, and whatever the spend refuses itself (a locked wallet, no sync, a bad
    /// amount), passes: the spend says that in its own words.
    pub(in crate::real) async fn lock_coins_before_password(
        &self,
        op: &Spend,
    ) -> Result<(), CoreError> {
        let Some(open) = self.unlocked.as_ref() else {
            return Ok(());
        };
        if !matches!(
            op,
            Spend::CreateIdentity { .. }
                | Spend::TopUpIdentity { .. }
                | Spend::Shield {
                    from: ShieldSource::Core,
                    ..
                }
                | Spend::FundAddress { .. }
        ) {
            return Ok(());
        }
        let Ok(wallet) = loaded(&self.chain, &open.id).await else {
            return Ok(());
        };
        let fees = self.fees_now(0).await;
        let pool_lock_waits = matches!(op, Spend::Shield { .. })
            && self.chain.unfinished_pool_shield(&wallet).await.is_some();
        let spendable = {
            let state = wallet.state().await;
            convert::core_balance(&state.core_wallet) - convert::unsignable(&state.core_wallet)
        };
        let amount = lock_amount(op, &fees, pool_lock_waits, spendable);
        let Some(amount) = amount else {
            return Ok(());
        };
        lock_coins_allow(
            &wallet.state().await.core_wallet,
            amount,
            |txid| self.chain.instant_lock_verified(txid),
            fees.lock_inputs_max,
        )
    }
}
