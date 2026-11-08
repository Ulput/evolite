//! Identities and credits: finding them, creating and topping up, names, spending credits.

use super::*;

impl Real {
    fn pending_names(&self) -> pending::PendingNames {
        pending::PendingNames::new(self.paths.pending_names.clone())
    }

    /// What a cut-short "Create an identity" left, if anything: a lock without its identity, or
    /// an identity without the name this computer remembers for it. Nothing while one is being
    /// made: its lock exists before Platform has the identity. Nor before a complete lookup
    /// (BUG-9): the identity may sit in a slot Platform did not answer for.
    pub(super) async fn unfinished_identity(
        &self,
        wallet: &platform_wallet::PlatformWallet,
    ) -> Option<UnfinishedIdentity> {
        if self.identity_busy.load(Ordering::Relaxed)
            || !self.identities_complete.load(Ordering::Relaxed)
        {
            return None;
        }
        let names = self.pending_names().load();
        if let Some(lock) = self.chain.unfinished(wallet).await {
            return Some(UnfinishedIdentity {
                locked: Some(lock.amount as Duffs),
                name: names.get(&lock.identity_index).cloned(),
            });
        }
        let slot = nameless_slot(&self.chain, wallet, &names).await?;
        Some(UnfinishedIdentity {
            locked: None,
            name: names.get(&slot).cloned(),
        })
    }

    /// Finishes the unfinished identity in the background: no new lock, no new money. The
    /// answer is `Done { what: "identity" }`.
    pub(super) fn resume_identity(&self) -> Result<(), CoreError> {
        let (id, keys) = self.dashpay_ready()?;
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made").into(),
            ));
        }
        let chain = self.chain.clone();
        let pending = self.pending_names();
        let busy = self.identity_busy.clone();
        let refresh = self.refresher();
        let (out, wake) = (self.out.clone(), self.wake.clone());
        self.answer_later(async move {
            let result = async {
                let wallet = chain.wallet(&id).await.ok_or(NOT_LOADED)?;
                let names = pending.load();
                let index = match chain.unfinished(&wallet).await {
                    Some(lock) => Some(lock.identity_index),
                    None => nameless_slot(&chain, &wallet, &names).await,
                };
                let index = index.ok_or(tr!("There is no unfinished identity"))?;
                watch_creation(
                    chain.clone(),
                    wallet.clone(),
                    index,
                    busy.clone(),
                    out,
                    wake,
                );
                chain
                    .resume_identity(&wallet, &keys, index, names.get(&index).map(String::as_str))
                    .await?;
                identity::finish_resumed(&pending, id, index)
            }
            .await
            .map_err(core_failure);
            busy.store(false, Ordering::Relaxed);
            refresh();
            Event::Done {
                what: "identity",
                result,
            }
        });
        Ok(())
    }

    /// What a cut-short top-up left locked on Core, if anything. Nothing while a top-up or an
    /// identity is being made: its lock exists before Platform takes it.
    pub(super) async fn unfinished_top_up(
        &self,
        wallet: &platform_wallet::PlatformWallet,
    ) -> Option<UnfinishedTopUp> {
        if self.identity_busy.load(Ordering::Relaxed)
            || !self.identities_found.load(Ordering::Relaxed)
        {
            return None;
        }
        self.chain
            .unfinished_top_up(wallet)
            .await
            .map(|lock| UnfinishedTopUp {
                locked: lock.amount as Duffs,
                awaiting_chain_lock: lock.awaiting_chain_lock,
            })
    }

    /// A cut-short top-up that still has to go to Platform (not one Platform already took and
    /// only its ChainLock is awaited): it holds back a new top-up and syncing again.
    pub(super) async fn top_up_to_finish(&self, wallet: &platform_wallet::PlatformWallet) -> bool {
        self.chain
            .unfinished_top_up(wallet)
            .await
            .is_some_and(|lock| !lock.awaiting_chain_lock)
    }

    /// Checks what can be checked at once and tops up the shown identity in the background
    /// with `amount` duffs from Core (an asset lock, then Platform). The answer is
    /// `Done { what: "top-up" }`.
    pub(super) async fn top_up_identity(&mut self, amount: Duffs) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let id = open.id;
        let amount = positive_duffs(amount)?;
        let status = self.chain.status();
        if !coins_known(&status, true)
            || status.phase == Phase::Error
            || !self.chain.platform_ready()
        {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        core_pays(&wallet, amount).await?;
        // A cut-short top-up is finished first, with the money already locked for it; one that
        // Platform already took (awaiting its ChainLock) does not hold a new one back.
        if self.top_up_to_finish(&wallet).await {
            return Err(CoreError::NotAvailable(
                tr!("First finish the stopped top-up — its money is already locked").into(),
            ));
        }
        let keys = open
            .keys
            .clone()
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made or topped up").into(),
            ));
        }
        let chain = self.chain.clone();
        let busy = self.identity_busy.clone();
        let refresh = self.refresher();
        self.answer_later(async move {
            let result = chain
                .top_up_identity(&wallet, &keys, amount)
                .await
                .map(|_| ())
                .map_err(identity_failure);
            busy.store(false, Ordering::Relaxed);
            refresh();
            Event::Done {
                what: "top-up",
                result,
            }
        });
        Ok(())
    }

    /// Checks the name at once and registers it for the shown identity in the background (one
    /// more DPNS name, paid from its credits). The answer is `Done { what: "name" }`.
    pub(super) fn add_name(&self, name: &str) -> Result<(), CoreError> {
        let label = dpns::label(name)?;
        if dpns::is_contested(&label) {
            return Err(CoreError::NotAvailable(
                tr!("Contested names are decided by a masternode vote, which evolite cannot do yet. Add a digit from 2 to 9 to the name or make it longer than 19 characters").into(),
            ));
        }
        let (id, keys) = self.dashpay_ready()?;
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made or topped up").into(),
            ));
        }
        let chain = self.chain.clone();
        let busy = self.identity_busy.clone();
        let refresh = self.refresher();
        let journal = self.journal.clone();
        self.answer_later(async move {
            let result = match chain.name_available(&label).await {
                Ok(false) => Err(CoreError::NotAvailable(format!(
                    tr!("The name @{label} is already taken"),
                    label = label
                ))),
                Err(e) => Err(CoreError::Failed(e)),
                Ok(true) => async {
                    let wallet = chain.wallet(&id).await.ok_or(NOT_LOADED)?;
                    let before = chain.first_identity_credits(&wallet, false).await;
                    chain.add_name(&wallet, &keys, &label).await?;
                    // In the history, with what it cost: the next name is estimated from it.
                    let after = chain.first_identity_credits(&wallet, true).await;
                    journal.add(journal::Entry {
                        at: now(),
                        what: journal::What::Name {
                            label: label.clone(),
                        },
                        spent: spent_credits(before, after).unwrap_or(0),
                        moved: 0,
                        ..Default::default()
                    });
                    Ok::<(), String>(())
                }
                .await
                .map_err(identity_failure),
            };
            busy.store(false, Ordering::Relaxed);
            refresh();
            Event::Done {
                what: "name",
                result,
            }
        });
        Ok(())
    }

    /// Finishes the cut-short top-up in the background with the lock already made: no new
    /// money. The answer is `Done { what: "top-up" }`.
    pub(super) fn resume_top_up(&self) -> Result<(), CoreError> {
        let (id, keys) = self.dashpay_ready()?;
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made or topped up").into(),
            ));
        }
        let chain = self.chain.clone();
        let busy = self.identity_busy.clone();
        let refresh = self.refresher();
        self.answer_later(async move {
            let result = async {
                let wallet = chain.wallet(&id).await.ok_or(NOT_LOADED)?;
                chain.resume_top_up(&wallet, &keys).await
            }
            .await
            .map(|_| ())
            .map_err(identity_failure);
            busy.store(false, Ordering::Relaxed);
            refresh();
            Event::Done {
                what: "top-up",
                result,
            }
        });
        Ok(())
    }

    /// Credits of the wallet's identity to `to`: `@name` (a transfer to that identity), a
    /// Platform address `tdash1…` (credits stay on Platform) or a Core address (a withdrawal,
    /// paid out on Core later). Signed and sent in the background; the answer is `Event::Sent`
    /// with no txid (Platform proves the transition itself).
    pub(super) fn send_credits(
        &self,
        id: WalletId,
        to: &str,
        amount: Duffs,
        note: Option<String>,
    ) -> Result<(), CoreError> {
        let credits = positive_credits(amount)?;
        let to = to.trim();
        enum To {
            Name(String),
            Ready(CreditPayee),
        }
        // And how the history will say it.
        let (to, what) = if to.starts_with('@') {
            let label = dpns::label(to)?;
            let what = journal::What::Credits {
                to: format!("@{label}"),
            };
            (To::Name(label), what)
        } else if let Ok(address) = PlatformAddress::from_bech32m_string(to) {
            // The same key's address on the other network is not where the person meant.
            if !platform_address_here(to) {
                return Err(CoreError::BadAddress);
            }
            let what = journal::What::Credits { to: to.to_owned() };
            (To::Ready(CreditPayee::Platform(address)), what)
        } else {
            let address = payee(to)?
                .require_network(crate::network::dash())
                .map_err(|_| CoreError::BadAddress)?;
            let what = journal::What::Withdrawal {
                to: address.to_string(),
            };
            (To::Ready(CreditPayee::Core(address)), what)
        };
        if !self.chain.platform_ready() {
            return Err(CoreError::NotSynced);
        }
        let keys = self
            .unlocked
            .as_ref()
            .and_then(|open| open.keys.clone())
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        let chain = self.chain.clone();
        let dirty = self.dirty.clone();
        let journal = self.journal.clone();
        let notes = self.notes.clone();
        // In the history from now on, as "being sent", until the journal has its line (BUG-12).
        let sending = self.platform_sending.clone();
        let key = sending.lock().unwrap_or_else(|e| e.into_inner()).start(
            platform_sending::credits_line(&what),
            amount,
            now(),
        );
        self.answer_later(async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let before = chain.first_identity_credits(&wallet, false).await;
                let payee = match to {
                    To::Ready(payee) => payee,
                    To::Name(label) => match chain.resolve_name(&label).await {
                        Ok(Some(id)) => CreditPayee::Identity(
                            Identifier::from_string(&id, Encoding::Base58)
                                .map_err(|e| CoreError::Failed(e.to_string()))?,
                        ),
                        Ok(None) => return Err(CoreError::NameNotFound),
                        Err(e) => return Err(read_failure(e)),
                    },
                };
                chain
                    .send_credits(&wallet, &keys, payee, credits)
                    .await
                    .map_err(dashpay_failure)?;
                // `send_credits` read the balance back: the difference is the amount and fee.
                let after = chain.first_identity_credits(&wallet, false).await;
                let line = journal.add(journal::Entry {
                    at: now(),
                    what,
                    spent: spent_credits(before, after).unwrap_or(amount),
                    moved: amount,
                    ..Default::default()
                });
                // The journal's line is the transfer's for good: its note goes there (AC-T3).
                if let (Some(line), Some(text)) = (line, note) {
                    let key = format!("platform:{line:016x}");
                    keep_note(notes, move |n| {
                        let _ = n.set(&key, Some(text));
                    })
                    .await;
                }
                Ok::<(), CoreError>(())
            }
            .await
            .map(|()| SentPayment {
                txid: String::new(),
                amount,
                fee: 0,
                relayed: true,
            });
            sending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .answered(key, result.is_ok(), now());
            dirty.store(true, Ordering::Relaxed);
            Event::Sent(result)
        });
        Ok(())
    }

    /// Looks for the phrase's identities in the background once proofs can be checked; what is
    /// found goes to the store and so into the next snapshot. A failed try is made again after
    /// 10 s, then less and less often (up to 5 min), while this unlock lasts; so is a pass with
    /// slots or names missing (BUG-9), though what it found is shown and its balances kept
    /// fresh at once. A newer search (`FindIdentities`) or locking stops this one, and its keys
    /// go with it; a complete pass lets them go too.
    pub(super) fn discover_identities(&self) {
        let Some((id, Some(keys))) = self.unlocked.as_ref().map(|o| (o.id, o.keys.clone())) else {
            return;
        };
        let run = self.discovery_run.fetch_add(1, Ordering::Relaxed) + 1;
        let current = self.discovery_run.clone();
        let chain = self.chain.clone();
        let search = self.identity_search.clone();
        let found = self.identities_found.clone();
        let complete = self.identities_complete.clone();
        let refresh = self.refresher();
        let stale = self.balances_stale.clone();
        found.store(false, Ordering::Relaxed);
        complete.store(false, Ordering::Relaxed);
        set_search(&search, IdentitySearch::Searching);
        refresh();
        tokio::spawn(async move {
            let live = move || current.load(Ordering::Relaxed) == run;
            chain.until_platform_ready().await;
            let mut pause = DISCOVERY_RETRY;
            let mut names_read = std::collections::HashSet::new();
            while live() {
                let result = match chain.wallet(&id).await {
                    Some(wallet) => {
                        chain
                            .discover_identities_again(&wallet, &keys, &mut names_read)
                            .await
                    }
                    None => Err(tr!("the wallet is not loaded yet").into()),
                };
                if !live() {
                    break;
                }
                let was_found = found.load(Ordering::Relaxed);
                // Every slot answered is enough for the unfinished lock (BUG-9 r2 should-1):
                // names still missing are read on the next pass. Once answered, it stays so
                // for this search.
                if result.as_ref().is_ok_and(|d| d.slots_complete) {
                    complete.store(true, Ordering::Relaxed);
                }
                let step = discovery_step(result, was_found);
                set_search(&search, step.status);
                if step.found && !was_found {
                    found.store(true, Ordering::Relaxed);
                    tokio::spawn(keep_balances_fresh(
                        chain.clone(),
                        id,
                        live.clone(),
                        stale.clone(),
                        refresh.clone(),
                    ));
                }
                refresh();
                if !step.again {
                    // The keys are not needed to read balances: let them go now.
                    return;
                }
                // A second at a time, so a lock or a newer search ends the wait at once.
                let until = tokio::time::Instant::now() + pause;
                while live() && tokio::time::Instant::now() < until {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                pause = (pause * 2).min(DISCOVERY_RETRY_MAX);
            }
        });
    }

    /// Checks what can be checked at once and starts the identity in the background: the name
    /// check, the asset lock ([`convert::identity_lock`]: `top_up`, the name and the
    /// creation), the identity and its name take a minute or more. The answer is
    /// `Done { what: "identity" }`.
    pub(super) async fn create_identity(
        &mut self,
        name: Option<&str>,
        top_up: Duffs,
    ) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let id = open.id;
        let label = identity::new_identity_label(name)?;
        let top_up = positive_duffs(top_up)?;
        // The name and the identity's creation at the fees the form showed (BUG-54).
        // Below Platform's least lock it would be refused and its DASH stay locked: not made
        // (review r1 must; the form says how much at least).
        let amount = convert::identity_lock(top_up, &self.fees_now(0).await, label.is_some())
            .ok_or(CoreError::BadAmount)?;
        let status = self.chain.status();
        if !coins_known(&status, true)
            || status.phase == Phase::Error
            || !self.chain.platform_ready()
        {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        core_pays(&wallet, amount).await?;
        // A cut-short identity is finished first, with the money already locked for it.
        if self.unfinished_identity(&wallet).await.is_some() {
            return Err(CoreError::NotAvailable(
                tr!("First finish the stopped identity — its money is already locked").into(),
            ));
        }
        let keys = open
            .keys
            .clone()
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made").into(),
            ));
        }
        let chain = self.chain.clone();
        let dirty = self.dirty.clone();
        let pending = self.pending_names();
        let busy = self.identity_busy.clone();
        let (out, wake) = (self.out.clone(), self.wake.clone());
        self.answer_later(async move {
            let available = match &label {
                Some(label) => chain.name_available(label).await,
                None => Ok(true),
            };
            let result = match available {
                Ok(false) => Err(CoreError::NotAvailable(format!(
                    tr!("The name @{label} is already taken, the money is untouched"),
                    label = label.as_deref().unwrap_or_default()
                ))),
                Err(e) => Err(CoreError::Failed(e)),
                Ok(true) => async {
                    let index = chain.next_identity_index(&wallet, &keys).await?;
                    watch_creation(
                        chain.clone(),
                        wallet.clone(),
                        index,
                        busy.clone(),
                        out,
                        wake,
                    );
                    // The name is kept before any money moves: a run cut short is finished
                    // with it (ResumeIdentity). Without a name nothing is kept, or the
                    // nameless identity would look cut short for ever (R-7, `nameless_slot`).
                    identity::remember_name(&pending, index, label.as_deref())?;
                    chain
                        .create_identity(&wallet, &keys, index, label.as_deref(), amount)
                        .await?;
                    pending.remove(index)?;
                    Ok(index)
                }
                .await
                .map_err(core_failure),
            };
            // The new identity is the one actions go from (analyst-44); on a failure the
            // choice stays.
            let result = identity::choose_created(id, result);
            busy.store(false, Ordering::Relaxed);
            dirty.store(true, Ordering::Relaxed);
            Event::Done {
                what: "identity",
                result,
            }
        });
        Ok(())
    }

    /// Makes `id` the identity the wallet works as.
    pub(super) async fn choose_identity(&mut self, id: String) -> (Option<Event>, bool) {
        if let Some(open) = &self.unlocked
            && let Some(wallet) = self.chain.wallet(&open.id).await
        {
            let slot = {
                let state = wallet.state().await;
                state
                    .identity_manager
                    .managed_identities()
                    .find(|m| {
                        m.wallet_id == Some(open.id)
                            && m.identity.id().to_string(Encoding::Base58) == id
                    })
                    .and_then(|m| m.identity_index)
            };
            if let Some(slot) = slot {
                chain::choose_identity(open.id, slot);
                // The contacts, NFTs and names of the other identity come next, and
                // its credits as Platform has them now.
                self.contacts.clear();
                self.balances_stale.notify_one();
            }
        }
        (None, true)
    }
}

/// The steps of an identity's creation for "What happens": read once a second from the
/// lock platform-wallet tracks for slot `index`, sent as `Progress { what: "identity" }` (the
/// step out of four) whenever it changes, until the creation ends (`busy` goes down).
fn watch_creation(
    chain: Arc<Chain>,
    wallet: Arc<platform_wallet::PlatformWallet>,
    index: u32,
    busy: Arc<AtomicBool>,
    out: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    tokio::spawn(async move {
        let mut last = 0;
        while busy.load(Ordering::Relaxed) {
            let step = chain.creation_step(&wallet, index).await;
            if step != last {
                last = step;
                let _ = out.send(Event::Progress {
                    what: "identity",
                    fraction: step as f32 / 4.0,
                });
                wake();
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

/// What a Platform write took from the identity's credits, in duffs (1 duff = 1000 credits),
/// when both balances are known and it went down.
pub(super) fn spent_credits(before: Option<u64>, after: Option<u64>) -> Option<Duffs> {
    let spent = before?.checked_sub(after?)?;
    (spent > 0).then_some((spent / 1000) as Duffs)
}

/// The first slot this computer remembers a name for (`names`, [`pending::PendingNames`])
/// whose identity exists without a DPNS name: made, its name not written.
async fn nameless_slot(
    chain: &Chain,
    wallet: &platform_wallet::PlatformWallet,
    names: &std::collections::BTreeMap<u32, String>,
) -> Option<u32> {
    for slot in names.keys() {
        if chain.nameless(wallet, *slot).await.is_some() {
            return Some(*slot);
        }
    }
    None
}

/// A failed top-up or name, in the interface's terms. A top-up that fails after its lock loses
/// nothing: the snapshot shows it as unfinished and `ResumeTopUp` finishes it.
pub(super) fn identity_failure(text: String) -> CoreError {
    if text.to_lowercase().contains("insufficient") {
        CoreError::NotEnough
    } else if text == tr!("The wallet has no identity") {
        CoreError::NotAvailable(tr!("Make an identity first").into())
    } else if text == chain::awaiting_chain_lock_text()
        || text == chain::top_up_identity_unknown_text()
        || text == chain::top_up_settled_text()
    {
        // Not a failure of the network: what is going on and what to do, as is.
        CoreError::NotAvailable(text)
    } else {
        CoreError::Failed(text)
    }
}

/// [`DiscoveryStep`] of a pass, `found_before` by an earlier pass of the same search. A pass
/// with slots or names missing shows what it found and goes on searching; only a complete one
/// is done. A failed pass is told only while no identity was found: beside identities the
/// search just goes on.
pub(super) fn discovery_step(
    pass: Result<chain::Discovery, String>,
    found_before: bool,
) -> DiscoveryStep {
    match pass {
        Ok(pass) => DiscoveryStep {
            status: if pass.complete {
                IdentitySearch::Done
            } else {
                IdentitySearch::Searching
            },
            found: true,
            again: !pass.complete,
        },
        Err(why) => DiscoveryStep {
            status: if found_before {
                IdentitySearch::Searching
            } else {
                IdentitySearch::Failed(why)
            },
            found: found_before,
            again: true,
        },
    }
}

pub(super) fn too_short(password: &Password) -> Result<(), CoreError> {
    if password.expose().chars().count() < MIN_PASSWORD {
        return Err(CoreError::NotAvailable(
            tr!("The new password needs at least 8 characters").into(),
        ));
    }
    Ok(())
}

fn set_search(search: &std::sync::Mutex<IdentitySearch>, state: IdentitySearch) {
    if let Ok(mut s) = search.lock() {
        *s = state;
    }
}
