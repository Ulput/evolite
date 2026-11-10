//! Core payments: who is paid, what a payment costs, and sending it.

use crate::real::*;

/// A payment below the dust limit (R-4).
const TOO_SMALL: &str =
    tr!("This amount is too small: the network does not relay so small a payment");

/// Money that is there but not final yet (R-5).
const NOT_SETTLED: &str = tr!("Part of your balance is not confirmed yet: try again in a minute");

impl Real {
    /// Who a Core payment to `to` goes to: a testnet address, or an established contact by name
    /// or — one without a name — by its identity id.
    fn core_payee(&self, to: &str) -> Result<CorePayee, CoreError> {
        let by_id = self
            .contacts
            .iter()
            .find(|c| c.identity_id == to.trim() && c.state == ContactState::Established);
        if let Some(contact) = by_id {
            Ok(CorePayee::Contact(
                Identifier::from_string(&contact.identity_id, Encoding::Base58)
                    .map_err(|e| CoreError::Failed(e.to_string()))?,
            ))
        } else if to.trim().starts_with('@') {
            let label = dpns::label(to)?;
            // By the name in the last snapshot, or one learnt since (a lookup, `ResolveName`).
            let names = self.names.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let name_of = |c: &Contact| {
                c.name
                    .clone()
                    .or_else(|| names.get(&c.identity_id).cloned().flatten())
            };
            let established = || {
                self.contacts
                    .iter()
                    .filter(|c| c.state == ContactState::Established)
            };
            let contact = established()
                .find(|c| name_of(c).as_deref() == Some(label.as_str()))
                .ok_or_else(|| {
                    if established().any(|c| name_of(c).is_none()) {
                        // Some contacts' names are still on their way: it may be one of them.
                        CoreError::NotAvailable(
                            tr!("Contacts' names are still loading — try again in a few seconds")
                                .into(),
                        )
                    } else {
                        CoreError::NotAvailable(format!(
                            tr!("@{label} is not in your contacts: a Core payment by name goes only to a DashPay contact")
                        , label = label))
                    }
                })?;
            Ok(CorePayee::Contact(
                Identifier::from_string(&contact.identity_id, Encoding::Base58)
                    .map_err(|e| CoreError::Failed(e.to_string()))?,
            ))
        } else if PlatformAddress::from_bech32m_string(to.trim()).is_ok() {
            Err(CoreError::NotAvailable(
                tr!("We do not send from Core to a Platform address yet (an asset lock is needed). Send from “Credits”").into(),
            ))
        } else {
            Ok(CorePayee::Address(
                payee(to)?
                    .require_network(crate::network::dash())
                    .map_err(|_| CoreError::BadAddress)?,
            ))
        }
    }

    /// The address a Core payment to `to` goes to, answered as `PaymentPreview`: an address at
    /// once, a contact's next address from its key in the background. The contact's is kept:
    /// the payment is signed only to it.
    pub(in crate::real) fn preview_payment(&self, to: String) -> Result<(), CoreError> {
        // Asked as the address is typed: a typo is said apart from another network's address.
        let payee = self.core_payee(&to).map_err(|e| match e {
            CoreError::BadAddress => not_an_address(&to).unwrap_or(CoreError::BadAddress),
            e => e,
        })?;
        let contact = match payee {
            CorePayee::Address(address) => {
                let _ = self.out.send(Event::PaymentPreview {
                    to,
                    result: Ok(address.to_string()),
                });
                return Ok(());
            }
            CorePayee::Contact(contact) => contact,
        };
        let (id, keys) = self.dashpay_ready()?;
        let chain = self.chain.clone();
        let previews = self.previews.clone();
        let paid = self.contact_paid.paid(&contact.to_string(Encoding::Base58));
        self.answer_later("task:preview_payment", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let address = chain
                    .contact_address(&wallet, &keys, contact, &paid)
                    .await
                    .map_err(CoreError::Failed)?
                    .ok_or(CoreError::NotAvailable(
                        tr!("No address from the contact's key yet — try again in a minute").into(),
                    ))?;
                previews
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(contact.to_string(Encoding::Base58), address.clone());
                Ok(address)
            }
            .await;
            Event::PaymentPreview { to, result }
        });
        Ok(())
    }

    /// Signs a Core payment and hands it to the network in the background: the network's answer
    /// takes up to a minute and the loop must go on sending snapshots. Never resent: a payment
    /// with no answer may still go through. "Password for every send" is checked before, by
    /// [`Real::spend_gate`].
    pub(in crate::real) async fn send(
        &mut self,
        payment: u64,
        from: Source,
        to: &str,
        amount: Duffs,
        note: Option<NoteText>,
    ) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let id = open.id;
        let note = send_note(note)?;
        match from {
            Source::Core => {}
            Source::Credits => return self.send_credits(payment, id, to, amount, note),
            Source::Pool => return self.send_from_pool(payment, to, amount, note),
        }
        let payee = self.core_payee(to)?;
        // A contact is paid only to the address the last look before signing showed.
        let shown = match &payee {
            CorePayee::Address(_) => None,
            CorePayee::Contact(contact) => Some(
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
            CorePayee::Address(address) => Some(address),
            CorePayee::Contact(_) => None,
        };
        if convert::is_dust(address, amount) {
            return Err(CoreError::NotAvailable(TOO_SMALL.into()));
        }
        // Coins are known only after the filters have been checked up to the tip.
        let status = self.chain.status();
        if !coins_known(&status, true) || status.phase == Phase::Error {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        core_pays(&wallet, amount).await?;
        coins_settled(
            &wallet.state().await.core_wallet,
            amount,
            matches!(payee, CorePayee::Contact(_)),
            |txid| self.chain.instant_lock_verified(txid),
        )?;
        let keys = self
            .unlocked
            .as_ref()
            .and_then(|open| open.keys.clone())
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        // In the history from now on, as "sending", until the wallet sees it; its fee as the
        // payment's own coins make it (BUG-55 review, should 2).
        let values: Vec<u64> = convert::coins_once(&wallet.state().await.core_wallet)
            .iter()
            .map(|u| u.value())
            .collect();
        let fee = payment_fee(values, amount as Duffs);
        let key = self
            .outgoing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start(to, amount as Duffs, fee, now());
        let outgoing = self.outgoing.clone();
        let contact_paid = self.contact_paid.clone();
        let chain = self.chain.clone();
        let out = self.out.clone();
        let wake = self.wake.clone();
        let notes = self.notes.clone();
        let dirty = self.dirty.clone();
        // Building, signing and the network's answer happen off the loop: the broadcast waits
        // for peers to relay the payment.
        tokio::spawn(async move {
            let paid = match (&payee, shown) {
                (CorePayee::Address(address), _) => {
                    chain.pay(&wallet, &keys, address, amount).await
                }
                (CorePayee::Contact(to), shown) => {
                    let shown = shown.unwrap_or_default();
                    let contact = to.to_string(Encoding::Base58);
                    let before = contact_paid.paid(&contact);
                    let paid = chain
                        .pay_contact(&wallet, &keys, *to, &shown, &before, amount)
                        .await;
                    // Taken, or maybe taken: never this address again, after a restart too.
                    if paid.is_ok() {
                        contact_paid.add(&contact, &shown);
                    }
                    paid
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
            // Taken (or maybe taken, with its txid): the note goes with the txid (AC-T3).
            if let (Ok(paid), Some(text)) = (&paid, note) {
                let key = format!("core:{}", paid.txid);
                keep_note(notes, move |n| {
                    let _ = n.set(&key, Some(text));
                })
                .await;
            }
            let result = paid
                .map(|paid| SentPayment {
                    txid: paid.txid,
                    amount: amount as Duffs,
                    fee: paid.fee as Duffs,
                    relayed: paid.relayed,
                })
                .map_err(core_failure);
            let _ = out.send(Event::Sent {
                id: payment,
                result,
            });
            dirty.store(true, Ordering::Relaxed);
            wake();
        });
        Ok(())
    }
}

/// The first address of the wallet's receive chain (BIP44 account 0, external, index 0).
pub(in crate::real) fn first_receive_address(
    wallet: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> Option<String> {
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::managed_account::managed_account_type::ManagedAccountType;
    let account = wallet.accounts.standard_bip44_accounts.get(&0)?;
    match account.managed_account_type() {
        ManagedAccountType::Standard {
            external_addresses, ..
        } => external_addresses
            .address_at_index(0)
            .map(|a| a.to_string()),
        _ => None,
    }
}

/// What a payment from Core may spend: the wallet's coins, each once (BUG-77) — not
/// platform-wallet's `balance()`, which counts a coin two accounts hold twice. The callers ask
/// [`coins_known`] first: an unchecked balance is no basis for a payment.
pub(in crate::real) async fn core_pays(
    wallet: &platform_wallet::PlatformWallet,
    amount: u64,
) -> Result<(), CoreError> {
    core_covers(&wallet.state().await.core_wallet, amount)
}

/// [`core_pays`] on the wallet's state: [`CoreError::NotEnough`] when its coins, each once, are
/// less than `amount`.
pub(in crate::real) fn core_covers(
    info: &key_wallet::wallet::ManagedWalletInfo,
    amount: u64,
) -> Result<(), CoreError> {
    if (convert::core_balance(info).max(0) as u64) < amount {
        return Err(CoreError::NotEnough);
    }
    Ok(())
}

/// A failed Core payment or asset lock, in the interface's terms.
pub(in crate::real) fn core_failure(text: String) -> CoreError {
    if text.contains(chain::name_credits_unknown_text()) {
        CoreError::NotAvailable(chain::name_credits_unknown_text().into())
    } else if text.to_lowercase().contains("insufficient") {
        CoreError::NotEnough
    } else {
        CoreError::Failed(text)
    }
}

/// Whether a payment of `amount` can go now without a coin that is not final: neither in a block
/// nor InstantSend-locked by a lock `verified` in this run (R-5, BUG-111). `contact`: such a payment is built by platform-wallet, which takes
/// any coin, largest first: it waits only if that walk reaches someone's coin that is not final.
/// A coin no block ever takes stays in the wallet without end, so one the payment would not take
/// must not stop it.
pub(in crate::real) fn coins_settled(
    info: &key_wallet::wallet::ManagedWalletInfo,
    amount: u64,
    contact: bool,
    verified: impl Fn(&key_wallet::dashcore::Txid) -> bool,
) -> Result<(), CoreError> {
    let waiting = || CoreError::NotAvailable(NOT_SETTLED.into());
    let coins = convert::coins_once(info);
    let is_final = |coin: &&key_wallet::Utxo| convert::coin_is_final(coin, &verified);
    if contact {
        let risky = |coin: &&key_wallet::Utxo| !is_final(coin) && !coin.is_trusted;
        let mut by_value = coins;
        // As `LargestFirst` takes them; of equal ones, the worse first.
        by_value.sort_by_key(|coin| (std::cmp::Reverse(coin.value()), !risky(coin)));
        let steps = convert::core_coins(by_value.iter().map(|c| c.value()).collect());
        let mut taken = 0;
        for (coin, step) in by_value.iter().zip(&steps) {
            if risky(coin) {
                return Err(waiting());
            }
            taken += coin.value();
            if taken >= amount + step.fee as u64 {
                break;
            }
        }
        return Ok(());
    }
    let values: Vec<u64> = coins
        .iter()
        .filter(|c| is_final(c))
        .map(|c| c.value())
        .collect();
    if values.len() == coins.len() {
        return Ok(());
    }
    let settled: u64 = values.iter().sum();
    let fee = payment_fee(values, amount as Duffs) as u64;
    if settled < amount + fee {
        return Err(waiting());
    }
    Ok(())
}

/// A positive amount of duffs, as the chain takes it.
pub(in crate::real) fn positive_duffs(amount: Duffs) -> Result<u64, CoreError> {
    u64::try_from(amount)
        .ok()
        .filter(|a| *a > 0)
        .ok_or(CoreError::BadAmount)
}

/// The fee a Core payment of `amount` is shown with while it is sent: as the coins it takes
/// make it (the same estimate as the Send screen's, `Fees::core_payment_for`), not as spending
/// every coin of the wallet would (BUG-55 review, should 2).
pub(in crate::real) fn payment_fee(coin_values: Vec<u64>, amount: Duffs) -> Duffs {
    let mut estimate = fees(coin_values.len());
    estimate.core_coins = convert::core_coins(coin_values);
    estimate.core_payment_for(amount)
}
