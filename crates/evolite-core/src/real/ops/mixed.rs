//! Payments from «Mixed» (FEAT-0030 phase A): coins of the CoinJoin account mixed elsewhere
//! (DashPay, Dash Core), spent only on their own, to a Dash address, with no change. What the
//! coins hold over the amount and the fee goes with the fee (арх-30-1), within АРХ-9а's bounds.

use crate::real::*;
use evolite_chain::mixed_pay::{self, MixedPlan, MixedShort};

/// To a Platform or private address (FEAT-0048 A15, BUG-261 A3): the screen's own line. Mixed
/// coins reach Platform from the identity's creation, its top-up and «Shield» (FEAT-0048 step 2).
const NOT_TO_PLATFORM: &str =
    tr!("From mixed coins you can pay to a Core address or to a DashPay contact.");

/// BUG-261 A2: `@name` that is not an established DashPay contact.
fn not_a_contact(label: &str) -> CoreError {
    CoreError::NotAvailable(format!(
        tr!(
            "From mixed coins you can pay @{} only as a DashPay contact. Add them to contacts or enter their Core address."
        ),
        label
    ))
}

/// Who a payment from «Mixed» goes to (BUG-261): a Core address, or an established DashPay
/// contact — paid to the DIP-15 Core address of theirs the last look showed (r1, BUG-212's way:
/// the same `PaymentPreview`, never a reservation).
pub(in crate::real) enum MixedPayee {
    Address(key_wallet::dashcore::Address),
    Contact(Identifier),
}

impl Real {
    /// BUG-260: the denominations' rounds to the journal, as counts by bucket, when they change.
    pub(in crate::real) fn say_mixed_rounds(&self, coins: &[convert::MixedCoin]) {
        let buckets = convert::rounds_buckets(coins);
        let mut said = self
            .mixed_rounds_said
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if *said != Some(buckets) {
            *said = Some(buckets);
            let [none, some, full, unknown] = buckets;
            crate::diag::record(crate::diag::Record::MixedRounds {
                none,
                some,
                full,
                unknown,
            });
        }
    }

    /// Who a payment from «Mixed» goes to: a Core address, or an established DashPay contact
    /// (BUG-261 A1, R-1); never Platform or the pool (A3), never `@name` that is not a contact
    /// (A2). Nothing is reserved here: this is also the preview's.
    pub(in crate::real) fn mixed_payee(&self, to: &str) -> Result<MixedPayee, CoreError> {
        if PlatformAddress::from_bech32m_string(to.trim()).is_ok() {
            return Err(CoreError::NotAvailable(NOT_TO_PLATFORM.into()));
        }
        // The names still loading, or the contact's key disabled, say so as for a Core payment.
        let loading = tr!("Contacts' names are still loading — try again in a few seconds");
        match self.core_payee(to) {
            Ok(CorePayee::Address(address)) => Ok(MixedPayee::Address(address)),
            Ok(CorePayee::Contact(contact)) => Ok(MixedPayee::Contact(contact)),
            Err(CoreError::NotAvailable(why))
                if to.trim().starts_with('@')
                    && why != loading
                    && !why.contains("disabled key") =>
            {
                Err(not_a_contact(&dpns::label(to).unwrap_or_else(|_| {
                    to.trim().trim_start_matches('@').to_owned()
                })))
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
        let spendable: Vec<key_wallet::Utxo> =
            convert::mixed_coins(&state.core_wallet, &self.stored_history, |txid| {
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
        let payee = self.mixed_payee(to)?;
        // BUG-261 r1: a contact only to the address the last look showed, as a Core payment.
        let shown = match &payee {
            MixedPayee::Address(_) => None,
            MixedPayee::Contact(contact) => Some(
                self.previews
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&contact.to_string(Encoding::Base58))
                    .ok_or(CoreError::NotAvailable(
                        tr!("The contact's address was not shown before signing — check the payment again")
                            .into(),
                    ))?,
            ),
        };
        let amount = positive_duffs(amount)?;
        let address = match &payee {
            MixedPayee::Address(address) => Some(address),
            // A contact's DIP-15 address is P2PKH: the same dust as any such one.
            MixedPayee::Contact(_) => None,
        };
        if convert::is_dust(address, amount) {
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
        let contact_paid = self.contact_paid.clone();
        // BUG-206: the identity «Acting as» shows when the command comes (BUG-261 A5).
        let pinned = self.shown_now(id);
        tokio::spawn(async move {
            let paid = match (payee, shown) {
                (MixedPayee::Address(address), _) => {
                    chain
                        .pay_mixed(&wallet, &keys, &address, amount, &plan)
                        .await
                }
                // Never without the address shown (taken above): refused, not an empty address.
                (MixedPayee::Contact(_), None) => Err(tr!(
                    "The contact's address was not shown before signing — check the payment again"
                )
                .to_string()),
                (MixedPayee::Contact(to), Some(shown)) => {
                    let contact = to.to_string(Encoding::Base58);
                    let before = contact_paid.paid(&contact);
                    let pay = async {
                        let (_, me) = pinned_acting(&wallet, pinned)
                            .await
                            .ok_or_else(|| tr!("The wallet has no identity").to_string())?;
                        // Still the address shown, after the password: the check a Core payment
                        // to a contact makes (`pay_contact`).
                        let now = chain
                            .contact_address(&wallet, &keys, &me, to, &before)
                            .await?;
                        if now.as_deref() != Some(shown.as_str()) {
                            return Err(tr!("The contact's address changed after the check — nothing was sent, check the payment again").to_string());
                        }
                        let address = evolite_chain::convert::payee(&shown)
                            .map_err(|e| format!("{e:?}"))?
                            .require_network(crate::network::dash())
                            .map_err(|e| e.to_string())?;
                        chain
                            .pay_mixed(&wallet, &keys, &address, amount, &plan)
                            .await
                    };
                    pay_then_mark(&contact_paid, &contact, &shown, pay).await
                }
            };
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

/// BUG-261 r1 (as BUG-212): pays, and only on the network's answer — sent, or unclear (BUG-243:
/// it may have gone) — writes `shown` as paid for `contact`, so the next payment takes the next
/// address. A refusal before the network leaves it unpaid: the next look shows it again, and the
/// contact's wallet, looking a few addresses ahead, never misses a payment.
pub(in crate::real) async fn pay_then_mark(
    contact_paid: &contact_addresses::ContactAddresses,
    contact: &str,
    shown: &str,
    pay: impl std::future::Future<Output = Result<evolite_chain::chain::Paid, String>>,
) -> Result<evolite_chain::chain::Paid, String> {
    let paid = pay.await;
    if paid.is_ok() {
        contact_paid.add(contact, shown);
    }
    paid
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
