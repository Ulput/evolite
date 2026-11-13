//! Payments from «Mixed» (FEAT-0030 phase A): coins of the CoinJoin account mixed elsewhere
//! (DashPay, Dash Core), spent only on their own, to a Dash address, with no change. What the
//! coins hold over the amount and the fee goes with the fee (арх-30-1), within АРХ-9а's bounds.

use crate::real::*;
use evolite_chain::mixed_pay::{self, MixedPlan, MixedShort};

/// To a contact or by `@name` (AC-T7, AC-A5): platform-wallet builds those payments from its own
/// coins. The analyst's words.
const NOT_TO_CONTACT: &str = tr!("From mixed coins you can pay only to a Core address for now.");

/// To a Platform address, an identity or the pool (AC-T7, AC-A5): an asset lock would join the
/// coins. The analyst's words.
const NOT_TO_PLATFORM: &str = tr!(
    "Mixed coins do not go to Platform: a top-up would put them into one transaction and show they are yours."
);

impl Real {
    /// The Dash address a payment from «Mixed» goes to: never a contact, a name or Platform.
    fn mixed_payee(&self, to: &str) -> Result<key_wallet::dashcore::Address, CoreError> {
        if PlatformAddress::from_bech32m_string(to.trim()).is_ok() {
            return Err(CoreError::NotAvailable(NOT_TO_PLATFORM.into()));
        }
        match self.core_payee(to) {
            Ok(CorePayee::Address(address)) => Ok(address),
            Ok(CorePayee::Contact(_)) => Err(CoreError::NotAvailable(NOT_TO_CONTACT.into())),
            Err(_) if to.trim().starts_with('@') => {
                Err(CoreError::NotAvailable(NOT_TO_CONTACT.into()))
            }
            Err(CoreError::BadAddress) => Err(not_an_address(to).unwrap_or(CoreError::BadAddress)),
            Err(e) => Err(e),
        }
    }

    /// The plan and the quote of a payment of `amount` from «Mixed», read from the wallet:
    /// nothing reserved, nothing sent.
    async fn mixed_plan(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        amount: u64,
    ) -> Result<(MixedPlan, MixedQuote), CoreError> {
        let state = wallet.state().await;
        let spendable: Vec<key_wallet::Utxo> = convert::mixed_coins(&state.core_wallet, |txid| {
            self.chain.instant_lock_verified(txid)
        })
        .into_iter()
        .filter(|c| c.spendable)
        .map(|c| c.utxo)
        .collect();
        let plan = mixed_pay::plan(&spendable, amount).map_err(|e| match e {
            MixedShort::NotEnough { .. } => CoreError::NotEnough,
        })?;
        Ok((plan.clone(), quote(&plan, amount)))
    }

    /// What `PreviewMixed` answers: the cost of a payment of `amount` to `to` from «Mixed».
    pub(in crate::real) async fn preview_mixed(
        &self,
        to: &str,
        amount: Duffs,
    ) -> Result<MixedQuote, CoreError> {
        let id = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?
            .id;
        self.mixed_payee(to)?;
        let amount = positive_duffs(amount)?;
        let status = self.chain.status();
        if !coins_known(&status, true) || status.phase == Phase::Error {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        Ok(self.mixed_plan(&wallet, amount).await?.1)
    }

    /// Signs a payment from «Mixed» and hands it to the network in the background, as
    /// [`Real::send`] does a Core payment. Refused before signing when the excess is over the
    /// bound, or not the one the person agreed to (`accept_excess`, АРХ-9а).
    pub(in crate::real) async fn send_mixed(
        &mut self,
        payment: u64,
        to: &str,
        amount: Duffs,
        (note, rate, accept_excess): (Option<NoteText>, Option<FiatRate>, Duffs),
    ) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let id = open.id;
        let kept = Kept {
            note: send_note(note)?,
            rate: self.payment_rate(rate),
        };
        let address = self.mixed_payee(to)?;
        let amount = positive_duffs(amount)?;
        if convert::is_dust(Some(&address), amount) {
            return Err(CoreError::NotAvailable(
                tr!("This amount is too small: the network does not relay so small a payment")
                    .into(),
            ));
        }
        let status = self.chain.status();
        if !coins_known(&status, true) || status.phase == Phase::Error {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        let (plan, quote) = self.mixed_plan(&wallet, amount).await?;
        excess_agreed(&quote, accept_excess)?;
        let keys = self
            .unlocked
            .as_ref()
            .and_then(|open| open.keys.clone())
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        // In the history from now on, as "sending", until the wallet sees it.
        let key = self
            .outgoing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start(to, amount as Duffs, quote.fee + quote.excess, now());
        let outgoing = self.outgoing.clone();
        let chain = self.chain.clone();
        let out = self.out.clone();
        let wake = self.wake.clone();
        let notes = self.notes.clone();
        let prices = self.prices.clone();
        let dirty = self.dirty.clone();
        tokio::spawn(async move {
            let paid = chain
                .pay_mixed(&wallet, &keys, &address, amount, &plan)
                .await;
            outgoing.lock().unwrap_or_else(|e| e.into_inner()).answered(
                key,
                match &paid {
                    Ok(paid) => outgoing::Answer::Sent {
                        txid: Some(paid.txid.clone()),
                        fee: paid.fee as Duffs,
                    },
                    Err(_) => outgoing::Answer::Refused,
                },
            );
            if let (Ok(paid), false) = (&paid, kept.is_empty()) {
                let key = format!("core:{}", paid.txid);
                keep_note(notes, move |n| kept.into_notes(n, &key)).await;
            }
            let result = paid
                .map(|paid| SentPayment {
                    txid: paid.txid,
                    amount: paid.sent as Duffs,
                    fee: paid.fee as Duffs,
                    relayed: paid.relayed,
                })
                .map_err(core_failure);
            let event = Event::Sent {
                id: payment,
                result,
            };
            prices.finish(&event);
            let _ = out.send(event);
            dirty.store(true, Ordering::Relaxed);
            wake();
        });
        Ok(())
    }
}

/// The quote of `plan` for `amount` (АРХ-9а): the excess's rule, and with a refusal the amount
/// the same coins pay without excess.
pub(in crate::real) fn quote(plan: &MixedPlan, amount: u64) -> MixedQuote {
    let rule = match evolite_chain::coinjoin::excess_rule(amount, plan.excess) {
        evolite_chain::coinjoin::Excess::Free => MixedExcess::Free,
        evolite_chain::coinjoin::Excess::Confirm => MixedExcess::Confirm,
        evolite_chain::coinjoin::Excess::Refuse => MixedExcess::Refuse,
    };
    MixedQuote {
        amount: amount as Duffs,
        fee: plan.fee as Duffs,
        excess: plan.excess as Duffs,
        rule,
        coins: plan.coins.len() as u32,
        suggested: (rule == MixedExcess::Refuse).then(|| (plan.total() - plan.fee) as Duffs),
    }
}

/// Whether the person agreed to the excess `quote` burns (АРХ-9а): never over the bound; a
/// larger one only when `accepted` is that very excess; a small one when agreed or not asked.
pub(in crate::real) fn excess_agreed(quote: &MixedQuote, accepted: Duffs) -> Result<(), CoreError> {
    let changed = CoreError::MixedExcess {
        excess: quote.excess,
        suggested: None,
    };
    match quote.rule {
        MixedExcess::Refuse => Err(CoreError::MixedExcess {
            excess: quote.excess,
            suggested: quote.suggested,
        }),
        MixedExcess::Confirm if accepted != quote.excess => Err(changed),
        MixedExcess::Free if accepted != 0 && accepted != quote.excess => Err(changed),
        _ => Ok(()),
    }
}
