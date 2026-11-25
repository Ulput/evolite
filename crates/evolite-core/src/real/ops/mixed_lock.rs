//! FEAT-0048 step 2: fully mixed coins to Platform — a new identity (path M), the shown
//! identity's top-up (UC-7), the pool (UC-8) — through an asset lock of the chosen denominations
//! (`Chain::mixed_lock`). The coins: only fully mixed final denominations, no change, no other
//! coin (R-M3); the exact set for the amount first, else the least excess (R-M1); that excess
//! goes into the credits or the pool, with the person's yes above 0.01 DASH (И7).

use crate::real::*;
use evolite_chain::chain::mixed_lock::MixedLockFor;
use evolite_chain::coinjoin::{DENOMINATIONS, largest_exact_at_most, pick_denominations};

/// И7: an excess above this needs the person's yes (0.01 DASH, as a payment from «Mixed»).
const EXCESS_ASKED: Duffs = MIXED_EXCESS_MAX;

/// The quote of `pick` for `amount`, from coins worth `values`, with `coins` chosen.
fn lock_quote(
    amount: Duffs,
    pick: &evolite_chain::coinjoin::Pick,
    values: &[u64],
) -> MixedLockQuote {
    let nominal = pick.nominal as Duffs;
    let excess = nominal.saturating_sub(amount);
    MixedLockQuote {
        amount,
        nominal,
        exact: pick.exact,
        excess,
        confirm: excess > EXCESS_ASKED,
        coins: pick.coins(),
        fee: evolite_chain::mixed_pay::usual_fee(pick.coins() as usize) as Duffs,
        exactly: (!pick.exact)
            .then(|| largest_exact_at_most(amount as u64, values).map(|v| v as Duffs))
            .flatten(),
    }
}

/// И7: an excess over the bound only when `accepted` is that very excess; the refusal names
/// it and, when there is one, the exact amount below.
fn excess_agreed(quote: &MixedLockQuote, accepted: Duffs) -> Result<(), CoreError> {
    if quote.confirm && accepted != quote.excess {
        return Err(CoreError::MixedExcess {
            excess: quote.excess,
            suggested: quote.exactly,
        });
    }
    Ok(())
}

impl Real {
    /// The fully mixed coins that pay `amount`, and their quote: read from the wallet,
    /// nothing reserved. `NotEnough` when the fully mixed ones do not reach it.
    async fn mixed_lock_choice(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        amount: Duffs,
    ) -> Result<(Vec<key_wallet::Utxo>, MixedLockQuote), CoreError> {
        let positive = positive_duffs(amount)?;
        let spendable: Vec<key_wallet::Utxo> = {
            let state = wallet.state().await;
            convert::mixed_coins(&state.core_wallet, &self.stored_history, |txid| {
                self.chain.instant_lock_verified(txid)
            })
            .into_iter()
            .filter(|c| c.spendable)
            .map(|c| c.utxo)
            .collect()
        };
        let values: Vec<u64> = spendable.iter().map(|u| u.txout.value).collect();
        let pick = pick_denominations(positive, &values).ok_or(CoreError::NotEnough)?;
        let mut need = pick.counts;
        let chosen = spendable
            .into_iter()
            .filter(
                |u| match DENOMINATIONS.iter().position(|d| *d == u.txout.value) {
                    Some(k) if need[k] > 0 => {
                        need[k] -= 1;
                        true
                    }
                    _ => false,
                },
            )
            .collect();
        Ok((chosen, lock_quote(amount, &pick, &values)))
    }

    /// What `PreviewMixedLock` answers.
    pub(in crate::real) async fn preview_mixed_lock(
        &self,
        amount: Duffs,
    ) -> Result<MixedLockQuote, CoreError> {
        let id = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?
            .id;
        let status = self.chain.status();
        if !coins_known(&status, true) || status.phase == Phase::Error {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        Ok(self.mixed_lock_choice(&wallet, amount).await?.1)
    }

    /// What every mixed-coin lock needs before anything moves: the wallet open with its keys,
    /// Core and Platform in step, the coins chosen and their excess agreed.
    async fn mixed_lock_start(
        &self,
        amount: Duffs,
        accept_excess: Duffs,
    ) -> Result<
        (
            WalletId,
            Arc<platform_wallet::PlatformWallet>,
            key_wallet::Wallet,
            Vec<key_wallet::Utxo>,
        ),
        CoreError,
    > {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let id = open.id;
        let keys = open
            .keys
            .clone()
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        let status = self.chain.status();
        if !coins_known(&status, true)
            || status.phase == Phase::Error
            || !self.chain.platform_ready()
        {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        let (coins, quote) = self.mixed_lock_choice(&wallet, amount).await?;
        excess_agreed(&quote, accept_excess)?;
        Ok((id, wallet, keys, coins))
    }

    /// FEAT-0048 step 2, path M: a new identity from fully mixed coins, then `name` as from
    /// Core. Refused before anything moves as a creation from Core is, and when the coins do not
    /// reach `amount` or its excess is not agreed.
    pub(in crate::real) async fn create_identity_from_mixed(
        &mut self,
        name: Option<&str>,
        amount: Duffs,
        accept_excess: Duffs,
    ) -> Result<(), CoreError> {
        let label = identity::new_identity_label(name)?;
        let (id, wallet, keys, coins) = self.mixed_lock_start(amount, accept_excess).await?;
        if self.unfinished_identity(&wallet).await.is_some() {
            return Err(CoreError::NotAvailable(
                tr!("First finish the stopped identity — its money is already locked").into(),
            ));
        }
        if self.pool_identity_waiting() {
            return Err(CoreError::NotAvailable(
                tr!("The identity from the pool is still being checked — wait for its outcome")
                    .into(),
            ));
        }
        let Some(busy) = self.identity_busy.take() else {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made").into(),
            ));
        };
        let needed = self.fees_now(0).await.name_credits_needed();
        let chain = self.chain.clone();
        let dirty = self.dirty.clone();
        let pending = self.pending_names();
        let (out, wake) = (self.out.clone(), self.wake.clone());
        let unclear = self.unclear.clone();
        self.answer_later("task:create_identity_from_mixed", async move {
            let available = match label.clone() {
                Some(label) => {
                    super::names::within(&chain, |c| async move { c.name_available(&label).await })
                        .await
                }
                None => Ok(true),
            };
            let result = match available {
                Ok(false) => Err(CoreError::NotAvailable(format!(
                    tr!("The name @{label} is already taken, the money is untouched"),
                    // default-ok: error text only: no label is empty
                    label = label.as_deref().unwrap_or_default()
                ))),
                Err(e) => Err(read_failure(e)),
                Ok(true) => async {
                    let index = chain.next_identity_index(&wallet, &keys).await?;
                    super::identity::watch_creation(
                        chain.clone(),
                        wallet.clone(),
                        index,
                        busy.flag(),
                        out,
                        wake,
                    );
                    identity::remember_name(&pending, index, label.as_deref())?;
                    let lock = chain
                        .mixed_lock(
                            &wallet,
                            &keys,
                            &coins,
                            MixedLockFor::Identity { slot: index },
                        )
                        .await?;
                    if let Err(e) = chain
                        .create_identity_from_lock(
                            &wallet,
                            &keys,
                            index,
                            label.as_deref(),
                            lock.out_point,
                            needed,
                        )
                        .await
                    {
                        super::identity::name_not_seen(
                            &chain,
                            &wallet,
                            index,
                            label.as_deref(),
                            &e,
                            &unclear,
                        )
                        .await;
                        return Err(e);
                    }
                    pending.remove(index)?;
                    Ok(index)
                }
                .await
                .map_err(core_failure),
            };
            let result = identity::choose_created(id, result);
            drop(busy);
            dirty.store(true, Ordering::Relaxed);
            Event::Done {
                what: DoneWhat::Identity,
                result,
                ask: None,
            }
        });
        Ok(())
    }

    /// FEAT-0048 step 2, UC-7: the shown identity topped up from fully mixed coins, through its
    /// slot's bound top-up account. Refused as a top-up from Core is.
    pub(in crate::real) async fn top_up_from_mixed(
        &mut self,
        amount: Duffs,
        accept_excess: Duffs,
    ) -> Result<(), CoreError> {
        let (id, wallet, keys, coins) = self.mixed_lock_start(amount, accept_excess).await?;
        if self.top_up_to_finish(&wallet).await {
            return Err(CoreError::NotAvailable(
                tr!("First finish the stopped top-up — its money is already locked").into(),
            ));
        }
        let Some(busy) = self.identity_busy.take() else {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made or topped up").into(),
            ));
        };
        let chain = self.chain.clone();
        let refresh = self.refresher();
        let pinned = self.shown_now(id);
        self.answer_later("task:top_up_from_mixed", async move {
            let result = match pinned_acting(&wallet, pinned).await {
                Some((shown, me)) => {
                    async {
                        let slot = wallet_identity_ids(&wallet)
                            .await
                            .into_iter()
                            .find(|(id, _)| *id == shown)
                            .map(|(_, slot)| slot)
                            .ok_or(tr!("The wallet has no identity"))?;
                        let lock = chain
                            .mixed_lock(&wallet, &keys, &coins, MixedLockFor::TopUp { slot })
                            .await?;
                        chain
                            .top_up_from_lock(&wallet, &keys, &me, lock.out_point)
                            .await
                    }
                    .await
                }
                None => Err(tr!("The wallet has no identity").to_string()),
            };
            let result = result
                .map(|_| ())
                .map_err(super::identity::identity_failure);
            drop(busy);
            refresh();
            Event::Done {
                what: DoneWhat::TopUp,
                result,
                ask: None,
            }
        });
        Ok(())
    }

    /// FEAT-0048 step 2, UC-8: fully mixed coins into the pool. Refused as a pool move is (the
    /// pool connected and free), and while a transfer to the pool cut short waits for its own
    /// lock.
    pub(in crate::real) async fn shield_from_mixed(
        &mut self,
        amount: Duffs,
        accept_excess: Duffs,
    ) -> Result<(), CoreError> {
        let (_, wallet, _, coins) = self.mixed_lock_start(amount, accept_excess).await?;
        if self.chain.unfinished_pool_shield(&wallet).await.is_some() {
            return Err(CoreError::NotAvailable(
                tr!("First finish the stopped transfer to the pool — its money is already locked")
                    .into(),
            ));
        }
        let (_, keys, _) = self.pool_move()?;
        let chain = self.chain.clone();
        self.pool_job(
            "shield-from-mixed",
            async move {
                let lock = chain
                    .mixed_lock(&wallet, &keys, &coins, MixedLockFor::Pool)
                    .await
                    .map_err(core_failure)?;
                chain
                    .shield_from_lock(&wallet, &keys, lock.out_point)
                    .await
                    .map_err(super::pool::pool_error)
            },
            |result| Event::Done {
                what: DoneWhat::Shield,
                result,
                ask: None,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote_of(amount: Duffs, values: &[u64]) -> MixedLockQuote {
        let pick = pick_denominations(amount as u64, values).unwrap();
        lock_quote(amount, &pick, values)
    }

    const D01: u64 = DENOMINATIONS[2];
    const D001: u64 = DENOMINATIONS[3];

    /// A9: 0.03 from 3 × 0.01 is exact: no excess, nothing to agree to.
    #[test]
    fn an_exact_set_asks_nothing() {
        let q = quote_of(3_000_000, &[D001, D001, D001, D01]);
        assert!(q.exact && q.excess == 0 && !q.confirm && q.exactly.is_none());
        assert_eq!(q.coins, 3);
        assert!(excess_agreed(&q, 0).is_ok());
    }

    /// A10, A13: only 0.1 for 0.03 → an excess of 0.07 into the credits, asked; the exact amount
    /// below offered; agreed only with that very excess.
    #[test]
    fn an_excess_over_a_hundredth_needs_that_very_yes() {
        let q = quote_of(3_000_000, &[D01, D001]);
        assert!(!q.exact);
        assert_eq!(q.excess, 7_000_000);
        assert!(q.confirm);
        assert_eq!(
            q.exactly,
            Some(1_000_000),
            "«Exactly from denominations: 0.01»"
        );
        assert!(matches!(
            excess_agreed(&q, 0),
            Err(CoreError::MixedExcess {
                excess: 7_000_000,
                suggested: Some(1_000_000)
            })
        ));
        assert!(excess_agreed(&q, 6_999_999).is_err());
        assert!(excess_agreed(&q, 7_000_000).is_ok());
    }

    /// И7: an excess up to 0.01 goes into the credits without asking.
    #[test]
    fn a_small_excess_is_not_asked() {
        let q = quote_of(2_500_000, &[D001, D001, D001]);
        assert_eq!(q.excess, 500_000);
        assert!(!q.confirm);
        assert!(excess_agreed(&q, 0).is_ok());
    }
}
