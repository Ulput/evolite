//! DashPay: contact requests, the profile, contacts' accounts and names.

use crate::real::*;

impl Real {
    /// What every DashPay write needs: the open wallet's id and keys, and proofs that can be
    /// checked.
    pub(in crate::real) fn dashpay_ready(
        &self,
    ) -> Result<(WalletId, key_wallet::Wallet), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let keys = open
            .keys
            .clone()
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        if !self.chain.platform_ready() {
            return Err(CoreError::NotSynced);
        }
        Ok((open.id, keys))
    }

    /// Sends our contact request to the owner of `name`, in the background; the answer is
    /// `Done { what: DoneWhat::Contact }`. Not again to someone we already asked or who is a contact:
    /// platform-wallet would write a new request (a key rotation), paid in credits, for nothing.
    pub(in crate::real) fn add_contact(&self, name: &str) -> Result<(), CoreError> {
        let label = dpns::label(name)?;
        let (id, keys) = self.dashpay_ready()?;
        let chain = self.chain.clone();
        let refresh = self.refresher();
        let journal = self.journal.clone();
        self.answer_later("task:add_contact", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let to = match chain.resolve_name(&label).await {
                    Ok(Some(to)) => to,
                    Ok(None) => return Err(CoreError::NameNotFound),
                    Err(e) => return Err(read_failure(e)),
                };
                let to = Identifier::from_string(&to, Encoding::Base58)
                    .map_err(|e| CoreError::Failed(e.to_string()))?;
                // By platform-wallet's state of the identity the request goes from, now: the
                // snapshot may be of the one shown before "Act as", and a repeat is paid (BUG-13).
                let (from, standing) = Chain::contact_standing(&wallet, &to)
                    .await
                    .map_err(CoreError::Failed)?;
                if let Some(why) = repeat_request(standing, &label) {
                    return Err(CoreError::NotAvailable(why));
                }
                // Checked, signed, paid and keyed as that one identity, whatever "Act as"
                // shows meanwhile (BUG-13 r1 should 1).
                let before = chain.identity_credits(&wallet, &from, false).await;
                chain
                    .add_contact(&wallet, &keys, &from, to)
                    .await
                    .map_err(dashpay_failure)?;
                // Keyed as the read of Platform keys it (BUG-58): one line, not two.
                journal.add(platform_history::sent_request(
                    &from,
                    &to,
                    &label,
                    now(),
                    spent_credits(before, chain.identity_credits(&wallet, &from, true).await)
                        .unwrap_or(0),
                ));
                Ok(())
            }
            .await;
            refresh();
            Event::Done {
                what: DoneWhat::Contact,
                result,
            }
        });
        Ok(())
    }

    /// Accepts or hides the request from `identity_id`, in the background.
    pub(in crate::real) fn contact_job(
        &self,
        identity_id: &str,
        job: ContactJob,
    ) -> Result<(), CoreError> {
        let from = Identifier::from_string(identity_id, Encoding::Base58)
            .map_err(|e| CoreError::Failed(e.to_string()))?;
        let (id, keys) = self.dashpay_ready()?;
        let chain = self.chain.clone();
        let refresh = self.refresher();
        let journal = self.journal.clone();
        let hidden = self.hidden.clone();
        let sender = identity_id.to_owned();
        // Accepting writes our request back: the history names whom.
        let who = self
            .contacts
            .iter()
            .find(|c| c.identity_id == identity_id)
            .map(Contact::handle)
            .unwrap_or_else(|| identity_id.to_owned());
        self.answer_later("task:contact_job", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                match job {
                    ContactJob::Accept => {
                        let before = chain.first_identity_credits(&wallet, false).await;
                        chain
                            .accept_contact(&wallet, &keys, from)
                            .await
                            .map_err(dashpay_failure)?;
                        let after = chain.first_identity_credits(&wallet, true).await;
                        journal.add(journal::Entry {
                            at: now(),
                            what: journal::What::ContactRequest { to: who },
                            spent: spent_credits(before, after).unwrap_or(0),
                            moved: 0,
                            ..Default::default()
                        });
                        Ok(())
                    }
                    ContactJob::Hide => {
                        // Kept outside the store too: a resync removes the store.
                        hidden
                            .add(&sender)
                            .map_err(|e| CoreError::Failed(e.to_string()))?;
                        chain
                            .hide_request(&wallet, &keys, from)
                            .await
                            .map_err(dashpay_failure)
                    }
                }
            }
            .await;
            refresh();
            Event::Done {
                what: DoneWhat::Contact,
                result,
            }
        });
        Ok(())
    }

    /// Writes the profile in the background. A picture's link is downloaded first: its hash
    /// goes into the profile, so contacts can check they see the same picture.
    pub(in crate::real) fn save_profile(
        &self,
        display: String,
        about: String,
        avatar_url: Option<String>,
    ) -> Result<(), CoreError> {
        if display.chars().count() > 25 || about.chars().count() > 140 {
            return Err(CoreError::NotAvailable(
                tr!("Name up to 25 characters, “About” up to 140").into(),
            ));
        }
        if avatar_url
            .as_deref()
            .is_some_and(|u| !u.trim().starts_with("https://"))
        {
            return Err(CoreError::NotAvailable(
                tr!("The picture link must start with https://").into(),
            ));
        }
        let (id, keys) = self.dashpay_ready()?;
        let chain = self.chain.clone();
        let refresh = self.refresher();
        let journal = self.journal.clone();
        self.answer_later("task:save_profile", async move {
            let result = async {
                let avatar = match avatar_url {
                    Some(url) => {
                        let fetch = url.clone();
                        let bytes = tokio::task::spawn_blocking(move || avatars::download(&fetch))
                            .await
                            .map_err(|e| CoreError::Failed(e.to_string()))?
                            .map_err(|e| {
                                CoreError::NotAvailable(format!(
                                    tr!("The picture did not download: {e}"),
                                    e = e
                                ))
                            })?;
                        Some((url, bytes))
                    }
                    None => None,
                };
                let wallet = loaded(&chain, &id).await?;
                let before = chain.first_identity_credits(&wallet, false).await;
                chain
                    .save_profile(&wallet, &keys, display, about, avatar)
                    .await
                    .map_err(dashpay_failure)?;
                let after = chain.first_identity_credits(&wallet, true).await;
                journal.add(journal::Entry {
                    at: now(),
                    what: journal::What::Profile,
                    spent: spent_credits(before, after).unwrap_or(0),
                    moved: 0,
                    ..Default::default()
                });
                Ok(())
            }
            .await;
            refresh();
            Event::Done {
                what: DoneWhat::Profile,
                result,
            }
        });
        Ok(())
    }

    /// Builds the key accounts of the established contacts in the background once the DashPay
    /// sync has found them: a payment to a contact is named by its @name, and counted in
    /// its payments, also in a wallet restored from its phrase and before any payment in this
    /// run. Again when more contacts are established; a failure is tried on a later round.
    pub(in crate::real) fn build_contact_accounts(&self) {
        let Some((id, Some(keys))) = self.unlocked.as_ref().map(|o| (o.id, o.keys.clone())) else {
            return;
        };
        let established = self
            .contacts
            .iter()
            .filter(|c| c.state == ContactState::Established)
            .count();
        if established <= self.contact_accounts_for.load(Ordering::Relaxed)
            || self.contact_accounts_busy.swap(true, Ordering::Relaxed)
        {
            return;
        }
        let chain = self.chain.clone();
        let built = self.contact_accounts_for.clone();
        let busy = self.contact_accounts_busy.clone();
        let dirty = self.dirty.clone();
        let wake = self.wake.clone();
        tokio::spawn(async move {
            let accounts = match chain.wallet(&id).await {
                Some(wallet) => chain.build_contact_accounts(&wallet, &keys).await.ok(),
                None => None,
            };
            if accounts.is_some() {
                // The history names the contacts' payments with what was built.
                dirty.store(true, Ordering::Relaxed);
                wake();
            }
            if contact_accounts_done(established, accounts) {
                built.store(established, Ordering::Relaxed);
            } else {
                // Not every contact has its account yet (the sweep or a contact's key was not
                // there): tried again, not before a pause.
                tokio::time::sleep(CONTACT_ACCOUNTS_RETRY).await;
            }
            busy.store(false, Ordering::Relaxed);
        });
    }

    /// Asks Platform for the DPNS names of contacts not asked about yet. An ask that failed (no
    /// proofs yet, the network) is asked again half a minute later: without its name a contact
    /// cannot be paid or found by name.
    ///
    /// Nothing before the masternode lists are in (BUG-78): the read would fail its proof and
    /// get DAPI nodes banned, every half minute for each contact. None is marked asked, so the
    /// snapshot that comes once they are in (the sync status changes) asks them all.
    pub(in crate::real) fn ask_names(&mut self, ids: Vec<String>) {
        if !self.chain.platform_ready() {
            return;
        }
        for id in ids {
            if !self
                .names_asked
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id.clone())
            {
                continue;
            }
            let chain = self.chain.clone();
            let names = self.names.clone();
            let asked = self.names_asked.clone();
            let refresh = self.refresher();
            tokio::spawn(async move {
                let Ok(identifier) = Identifier::from_string(&id, Encoding::Base58) else {
                    return;
                };
                match turn::timed("contact_name_read", chain.name_of(identifier)).await {
                    Ok(name) => {
                        names
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(id, name);
                    }
                    Err(_) => {
                        tokio::time::sleep(NAME_RETRY).await;
                        asked.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                    }
                }
                refresh();
            });
        }
    }
}

/// Why a contact request to `@label` would only repeat one already made, by where its owner
/// stands among our contacts; `None`: send it. A request from them is answered with ours, so it
/// is no reason to hold back.
pub(in crate::real) fn repeat_request(state: Option<ContactState>, label: &str) -> Option<String> {
    match state? {
        ContactState::Established => Some(format!(
            tr!("@{label} is already in your contacts"),
            label = label
        )),
        ContactState::Outgoing => Some(format!(
            tr!(
                "The request to @{label} is already sent — waiting for the answer, no new one is needed"
            ),
            label = label
        )),
        ContactState::Incoming => None,
    }
}

/// Whether the key accounts of `established` contacts are all there (`accounts` built, `None`
/// when the try failed): only then is the count kept and no more tries made. A try
/// that read no contact's key (the sweep had not queued them) is not success.
pub(in crate::real) fn contact_accounts_done(established: usize, accounts: Option<usize>) -> bool {
    accounts.is_some_and(|built| built >= established)
}

/// What O5 shows for a phrase imported from DashPay: its identity, credits, keys, name
/// and contacts, read from Platform with proofs. `None` — and the onboarding goes straight on —
/// when the phrase has no identity, or Platform could not be read in time.
pub(in crate::real) async fn scan_dashpay(
    chain: &Chain,
    keys: &key_wallet::Wallet,
) -> Option<DashPayScan> {
    let ready = turn::timed("gate_dashpay_scan", chain.until_platform_ready());
    tokio::time::timeout(DASHPAY_SCAN_WAIT, ready).await.ok()?;
    let found = chain.scan_dashpay(keys).await.ok()?;
    dashpay_scan_of(found)
}

/// O5's rows from what [`Chain::scan_dashpay`] found; nothing without an identity. Core
/// balances are not known before the wallet's own sync.
pub(in crate::real) fn dashpay_scan_of(found: chain::DashPayFound) -> Option<DashPayScan> {
    let (id, credits, keys) = found.identity?;
    Some(DashPayScan {
        core: 0,
        core_addresses_used: 0,
        identity_id: Some(id),
        identity_credits: identity::credits_as_duffs(credits),
        identity_keys: keys,
        name: found.name,
        contacts: found.contacts,
        coinjoin: None,
        core_known: false,
    })
}

/// A failed DashPay write, in the interface's terms.
pub(in crate::real) fn dashpay_failure(text: String) -> CoreError {
    let lower = text.to_lowercase();
    if lower.contains("insufficient") || lower.contains("balance") {
        CoreError::NotEnough
    } else if text == tr!("The wallet has no identity") {
        CoreError::NotAvailable(text)
    } else {
        CoreError::Failed(text)
    }
}
