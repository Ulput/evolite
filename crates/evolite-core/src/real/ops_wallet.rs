//! A wallet and its password: making, opening, locking and deleting it, the payment password.

use super::*;

impl Real {
    /// Asks the wallet to scan its compact filters again from `from` (BUG-64): a block it
    /// took a transaction out of is matched and applied again, and the store gets it. Once
    /// the sync is done, and only below the lowest height asked in this run, so it never
    /// repeats itself; the journal says why (`lost_block` or `stale`). What it is for — the
    /// `stale` lines of the mempool file, the `lost` ones of `mined.json` — is owed until the
    /// wallet's filters are back at `synced`, where they stand now ([`OwedRescan`]).
    pub(super) async fn rescan_from(
        &mut self,
        id: WalletId,
        from: u32,
        why: &'static str,
        synced: u32,
        stale: Vec<String>,
        lost: Vec<String>,
    ) {
        if self.chain.status().phase != sync::Phase::Synced
            || self.rescanned_from.is_some_and(|asked| asked <= from)
        {
            return;
        }
        let Ok(manager) = self.chain.manager().await else {
            return;
        };
        self.rescanned_from = Some(from);
        crate::diag::record(crate::diag::Record::Answer {
            what: "rescan",
            outcome: why,
        });
        let asked = matches!(
            tokio::task::spawn_blocking(move || manager.spv_rescan_filters_blocking(&id, from))
                .await,
            Ok(true)
        );
        if asked {
            OwedRescan::add(&mut self.rescan_owed, id, synced, stale, lost);
        }
    }

    /// Decrypts the vault with `password`; Argon2 runs off the async threads.
    pub(super) async fn open_vault(
        &self,
        password: &Password,
    ) -> Result<(vault::VaultFile, Zeroizing<String>), CoreError> {
        open_vault_at(&self.paths.vault, password).await
    }

    /// The vault of wallet `id` of this computer: this one's, or another's by its own
    /// password; a wallet not in the index has none.
    pub(super) async fn open_wallet_vault(
        &self,
        id: &str,
        password: &Password,
    ) -> Result<(vault::VaultFile, Zeroizing<String>), CoreError> {
        if self.index.get(id).is_none() {
            return Err(CoreError::NotAvailable(
                tr!("There is no such wallet").into(),
            ));
        }
        open_vault_at(
            &wallets::wallet_dir(&self.paths.root, id).join("wallet.json"),
            password,
        )
        .await
    }

    /// The same phrase under a new password: fresh salt and nonce, written in place atomically.
    pub(super) async fn change_password(
        &mut self,
        old: &Password,
        new: &Password,
    ) -> Result<(), CoreError> {
        if new.expose().is_empty() {
            return Err(CoreError::NotAvailable(
                tr!("The new password is empty").into(),
            ));
        }
        too_short(new)?;
        let (file, phrase) = self.open_vault(old).await?;
        let new = Zeroizing::new(new.expose().to_owned());
        let sealed = tokio::task::spawn_blocking(move || {
            vault::seal(&phrase, &new, file.meta, KdfParams::default())
        })
        .await
        .map_err(|_| CoreError::NotAvailable(tr!("The vault was not written").into()))?
        .map_err(vault_error)?;
        vault::save(&self.paths.vault, &sealed).map_err(vault_error)
    }

    /// Removes the wallet from this computer: with `password` it is checked first; then
    /// the keys go and the vault is deleted — without it the wallet cannot be opened again. The
    /// rest of the wallet's folder (store, names, journal, contacts' addresses) goes once
    /// nothing holds it: the loop makes the core anew, stops this chain off the loop and
    /// [`clear_other_leftovers`] removes it before the next chain starts. Headers and filters
    /// (`spv`) stay: they hold nothing of the wallet.
    pub(super) async fn delete_wallet(
        &mut self,
        password: Option<&Password>,
    ) -> Result<(), CoreError> {
        if self.saved.is_none() {
            return Err(CoreError::NotAvailable(NO_VAULT.into()));
        }
        if let Some(password) = password {
            self.open_vault(password).await?;
        }
        self.lock().await?;
        // Whatever happens next, the core starts again: the loop stops this chain off the
        // loop (seconds, while a sync pass ends) and the next core shows its wallet — or the
        // onboarding — at once (F4, BUG-21).
        self.restart = true;
        let removed = match std::fs::remove_file(&self.paths.vault) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
        match removed {
            Ok(()) => {
                self.saved = None;
                // Its wrong passwords go with it: the same phrase restored starts afresh.
                let id = self.wallet.clone();
                self.attempts.forget(&id);
                Ok(())
            }
            Err(e) => Err(CoreError::Failed(format!(
                tr!("The wallet was not deleted: {e}"),
                e = e
            ))),
        }
    }

    /// Removes another wallet of this computer (its "…" in Settings; R-28, BUG-44), by its
    /// own password — none is refused. Its store is never opened here: only its files go,
    /// the vault first (after it the folder is a leftover that [`clear_other_leftovers`] and
    /// the index's loading take), then the folder (best effort), its record and its pictures.
    /// The open wallet stays as it is: not locked, its chain not stopped.
    pub(super) async fn delete_other(
        &mut self,
        id: &str,
        password: Option<&Password>,
    ) -> Result<(), CoreError> {
        let password = password.ok_or(CoreError::WrongPassword)?;
        if self.switch_to.as_deref() == Some(id) || self.adding_from.as_deref() == Some(id) {
            return Err(CoreError::NotAvailable(
                tr!("This wallet cannot be deleted now").into(),
            ));
        }
        self.open_wallet_vault(id, password).await?;
        let folder = wallets::wallet_dir(&self.paths.root, id);
        match std::fs::remove_file(folder.join("wallet.json")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(CoreError::Failed(format!(
                    tr!("The wallet was not deleted: {e}"),
                    e = e
                )));
            }
        }
        self.attempts.forget(id);
        let _ = std::fs::remove_dir_all(&folder);
        self.index.wallets.retain(|r| r.id != id);
        if self.index.last.as_deref() == Some(id) {
            self.index.last = None;
        }
        self.save_index();
        if let Some(base) = &self.art_base {
            let _ = std::fs::remove_dir_all(base.join(id));
        }
        Ok(())
    }

    /// "Sync again": leaves a mark for [`resync_on_start`]; the app starts
    /// again for it. Not while an identity is being made or topped up, nor when one was cut
    /// short: its lock lives in the store, and "Continue" would be lost with it.
    pub(super) async fn ask_resync(&self) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let unfinished = self.identity_busy.load(Ordering::Relaxed)
            || match self.chain.wallet(&open.id).await {
                Some(wallet) => {
                    self.unfinished_identity(&wallet).await.is_some()
                        || self.top_up_to_finish(&wallet).await
                }
                None => false,
            };
        if unfinished {
            return Err(CoreError::NotAvailable(
                tr!("First finish creating or topping up the identity: its money is already locked, and after syncing again it could not be continued from here").into(),
            ));
        }
        std::fs::write(&self.paths.resync, b"")
            .map_err(|e| CoreError::Failed(format!(tr!("It did not work: {e}"), e = e)))
    }

    /// Forgets the keys. platform-wallet keeps following the wallet watch-only, so opening it
    /// again shows the history at once.
    pub(super) async fn lock(&mut self) -> Result<(), CoreError> {
        // An identity lookup still retrying stops and lets go of the keys; the balance
        // reading that follows a lookup stops too.
        self.discovery_run.fetch_add(1, Ordering::Relaxed);
        self.balances_stale.notify_one();
        // The right words of an unanswered phrase check go with the keys.
        self.phrase_check = None;
        // An app's request is not answered after the next unlock with what was shown before.
        self.app_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.unlocked = None;
        self.attach = None;
        self.receive_address = None;
        // The notes' key and every decrypted note go with the keys (AC-T6).
        *self.notes.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }

    /// Writes the index after a change; the folders stay the truth (see [`wallets::load`]).
    fn save_index(&self) {
        let _ = wallets::save(&self.paths.root, &self.index);
    }

    /// [`Self::unlock`] with this wallet's wrong passwords counted (FEAT-0002): in a pause it
    /// is refused before the vault is touched; else the try is written down as wrong before
    /// Argon2, then cleared by a right password or taken back by an error of another kind.
    pub(super) async fn unlock_counted(&mut self, password: &Password) -> Result<(), CoreError> {
        let id = self.wallet.clone();
        let before = self.attempts.begin(&id, wall_secs()).map_err(too_many)?;
        let result = self.unlock(password).await;
        self.attempts.record(&id, before, counted(&result));
        result
    }

    /// [`Self::open_wallet_vault`] for "Open "…"" (`SwitchWallet` with the password), counted
    /// as [`Self::unlock_counted`] for wallet `id`. The other callers of the vault (deleting
    /// another wallet, a payment's password) do not count: FEAT-0004 (R-31).
    async fn open_other_counted(
        &mut self,
        id: &str,
        password: &Password,
    ) -> Result<(vault::VaultFile, Zeroizing<String>), CoreError> {
        let before = self.attempts.begin(id, wall_secs()).map_err(too_many)?;
        let result = self.open_wallet_vault(id, password).await;
        self.attempts.record(id, before, counted(&result));
        result
    }

    /// Opens the vault and restarts sync with the wallet's keys.
    pub(super) async fn unlock(&mut self, password: &Password) -> Result<(), CoreError> {
        let (file, phrase) = self.open_vault(password).await?;
        self.unlock_opened(file, phrase).await
    }

    /// Unlocks with a vault already opened by its password (one Argon2 per switch, F2). The
    /// wallet is open at once; platform-wallet gets it when the chain runs (F3), so a password
    /// typed while the chain starts no longer waits for the start.
    pub(super) async fn unlock_opened(
        &mut self,
        file: vault::VaultFile,
        phrase: Zeroizing<String>,
    ) -> Result<(), CoreError> {
        let meta = file.meta;
        if meta.network != network() {
            return Err(CoreError::NotAvailable(format!(
                tr!("The wallet on this computer is for the {} network"),
                meta.network
            )));
        }
        // Never over a deleted wallet's files (BUG-8): no store opens over them.
        if self.leftovers {
            return Err(CoreError::NotAvailable(LEFTOVERS.into()));
        }
        self.last_touch = tokio::time::Instant::now();
        let keys = keys_of(&phrase)?;
        // The id platform-wallet gives the wallet: from the same root key.
        let id = keys.compute_wallet_id();
        let seed = seed_of(&phrase)?;
        self.open_notes(seed.as_ref(), &meta.wallet_id).await;
        self.unlocked = Some(Unlocked {
            id,
            meta,
            keys: Some(keys),
            seed: Some(seed),
        });
        self.attach = Some(phrase);
        self.attach_failed = None;
        if let Err(e) = self.attach_wallet().await {
            // The chain runs and refused the wallet: as before, it stays locked.
            self.lock().await?;
            return Err(CoreError::Failed(e));
        }
        self.receive_address = None;
        // History from before this run: the store has it, the wallet in memory does not.
        self.history_read = None;
        self.discover_identities();
        // "opened yesterday" in the switcher, and this one opens first next time.
        if let Some(record) = self.index.get_mut(&self.wallet) {
            record.last_opened = Some(now());
        }
        self.index.last = Some(self.wallet.clone());
        self.save_index();
        Ok(())
    }

    /// Seals the pending phrase with the password, saves the vault, starts the wallet's sync.
    /// A restored wallet is scanned from the start (its age is unknown); a new one from the tip.
    /// The phrase stays pending until the vault is written, so a failure can be retried.
    pub(super) async fn create(&mut self, password: &Password) -> Result<(), CoreError> {
        if password.expose().is_empty() {
            return Err(CoreError::WrongPassword);
        }
        too_short(password)?;
        if self.saved.is_some() {
            return Err(CoreError::NotAvailable(
                tr!("There is already a wallet on this computer").into(),
            ));
        }
        let (phrase, restored) = self.pending.clone().ok_or(CoreError::BadPhrase)?;
        // A new wallet is born at the tip of the headers: this chain's, or the one the wallet
        // before saw ("Add a wallet"). The chain may still be starting (the onboarding no
        // longer waits for it, F3); with neither known, its start is waited for — without
        // headers the wallet would be scanned from the genesis.
        let birth_height = if restored {
            0
        } else {
            let known = |core: &Self| {
                core.network_chain()
                    .status()
                    .headers
                    .map(|h| h.height)
                    .max(core.birth_hint)
            };
            match known(self) {
                Some(height) => height,
                None if self.parked.is_some() => {
                    // The parked chain's start, still under way, brings them (BUG-76).
                    let deadline = tokio::time::Instant::now() + START_WAIT;
                    while known(self).is_none()
                        && tokio::time::Instant::now() < deadline
                        && self
                            .parked
                            .as_ref()
                            .and_then(|p| p.starting.as_ref())
                            .is_some_and(|start| !start.is_finished())
                    {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                    known(self).unwrap_or(0)
                }
                None => {
                    self.start_settled(START_WAIT).await;
                    known(self).unwrap_or(0)
                }
            }
        };
        if self.leftovers {
            return Err(CoreError::NotAvailable(LEFTOVERS.into()));
        }
        let keys = keys_of(&phrase)?;
        let meta = VaultMeta {
            network: network().into(),
            wallet_id: hex(&keys.compute_wallet_id()),
            birth_height,
            word_count: phrase.split_whitespace().count() as u8,
            created_at: now() as u64,
        };
        let password = Zeroizing::new(password.expose().to_owned());
        let file = {
            let phrase = phrase.clone();
            let meta = meta.clone();
            tokio::task::spawn_blocking(move || {
                vault::seal(&phrase, &password, meta, KdfParams::default())
            })
            .await
            .map_err(|_| CoreError::NotAvailable(tr!("The vault was not written").into()))?
            .map_err(vault_error)?
        };
        vault::save(&self.paths.vault, &file).map_err(vault_error)?;
        // From here the wallet exists: if sync does not start, the unlock screen retries it.
        self.pending = None;
        self.saved = Some(meta.clone());
        // In the index of this computer's wallets, and the one to open next time; an
        // added wallet has no "Cancel" any more.
        if self.index.get(&self.wallet).is_none() {
            self.index
                .add(&self.wallet, self.pending_origin, meta.created_at as i64);
        }
        if let Some(record) = self.index.get_mut(&self.wallet) {
            record.last_opened = Some(now());
        }
        self.index.last = Some(self.wallet.clone());
        self.save_index();
        self.adding_from = None;
        // Added over a parked chain: that one stops, then this wallet's own starts (BUG-76).
        // Nothing to open ahead: a new wallet's store has nothing to show yet.
        if let Some(parked) = self.parked.take() {
            self.start_after(parked, false);
        }
        let id = keys.compute_wallet_id();
        let seed = seed_of(&phrase)?;
        self.open_notes(seed.as_ref(), &meta.wallet_id).await;
        self.unlocked = Some(Unlocked {
            id,
            meta,
            keys: Some(keys),
            seed: Some(seed),
        });
        self.attach = Some(phrase);
        self.attach_failed = None;
        if let Err(e) = self.attach_wallet().await {
            // The vault is "Locked" now: the interface goes to the unlock screen.
            self.lock().await?;
            return Err(CoreError::NotAvailable(format!(
                tr!(
                    "The wallet is saved, but the sync did not start ({e}). Sign in with your password — evolite starts it again"
                ),
                e = e
            )));
        }
        self.receive_address = None;
        self.discover_identities();
        Ok(())
    }

    /// "Password for every send" (BUG-42, R-27), before anything of a spend is checked, locked
    /// or signed: when the wallet asks for it, no password is a refusal and a wrong one is
    /// [`CoreError::WrongPassword`]. The attempt counter and the pauses are FEAT-0004's, here.
    pub(super) async fn spend_gate(&self, password: Option<&Password>) -> Result<(), CoreError> {
        if !self.password_each_send {
            return Ok(());
        }
        match password {
            None => Err(CoreError::NotAvailable(crate::PASSWORD_NEEDED.into())),
            Some(password) => self.open_vault(password).await.map(|_| ()),
        }
    }

    /// Settings → Security → "Password for every send": on at once; off only with the
    /// wallet's password, which [`Real::spend_gate`] would ask anyway.
    pub(super) async fn set_password_each_send(
        &mut self,
        on: bool,
        password: Option<&Password>,
    ) -> Result<(), CoreError> {
        if self.unlocked.is_none() {
            return Err(CoreError::NotAvailable(tr!("The wallet is locked").into()));
        }
        if !on {
            let password = password.ok_or(CoreError::WrongPassword)?;
            self.open_vault(password).await?;
        }
        security::save(&self.paths.root, &self.wallet, on).map_err(|e| {
            CoreError::NotAvailable(format!(tr!("The setting was not saved: {e}"), e = e))
        })?;
        self.password_each_send = on;
        Ok(())
    }

    /// A new recovery phrase of `words` words, kept until the wallet is made.
    pub(super) fn new_phrase(&mut self, words: usize) -> (Option<Event>, bool) {
        let event = {
            let n = if words == 24 { 24 } else { 12 };
            match Mnemonic::generate(n, Language::English) {
                Ok(mnemonic) => {
                    let phrase = Zeroizing::new(mnemonic.phrase());
                    let words = phrase.split_whitespace().map(str::to_owned).collect();
                    self.pending = Some((phrase, false));
                    self.pending_origin = wallets::Origin::Created;
                    Event::NewPhrase(Phrase::new(words))
                }
                Err(_) => Event::Done {
                    what: "create",
                    result: Err(CoreError::BadPhrase),
                },
            }
        };
        (Some(event), true)
    }

    /// Takes a phrase to restore from; with `dashpay`, reads what it has on Platform first.
    pub(super) async fn use_phrase(
        &mut self,
        phrase: Phrase,
        dashpay: bool,
    ) -> (Option<Event>, bool) {
        let event = {
            let text = Zeroizing::new(phrase.words().join(" "));
            if !Mnemonic::validate(&text) {
                Event::PhraseAccepted(Err(CoreError::BadPhrase))
            } else if let Some(name) = self.same_wallet(&text) {
                // One phrase, one wallet on this computer.
                Event::PhraseAccepted(Err(CoreError::NotAvailable(if name.is_empty() {
                    tr!("This wallet is already on this computer").into()
                } else {
                    format!(
                        tr!("This wallet is already on this computer — “{name}”"),
                        name = name
                    )
                })))
            } else if !dashpay {
                self.pending = Some((text, true));
                self.pending_origin = wallets::Origin::Restored;
                Event::PhraseAccepted(Ok(None))
            } else {
                self.pending_origin = wallets::Origin::DashPay;
                // Import from DashPay: what the phrase has on Platform is read now,
                // before the wallet is made, for O5. Its keys are dropped after the scan.
                let keys = keys_of(&text);
                self.pending = Some((text, true));
                let chain = self.network_chain().clone();
                self.answer_later("task:use_phrase", async move {
                    let found = match keys {
                        Ok(keys) => scan_dashpay(&chain, &keys).await,
                        Err(_) => None,
                    };
                    Event::PhraseAccepted(Ok(found))
                });
                return (None, false);
            }
        };
        (Some(event), true)
    }

    /// Opens another wallet of this computer in place of the open one.
    pub(super) async fn switch_wallet(
        &mut self,
        id: String,
        password: Option<Password>,
    ) -> (Option<Event>, bool) {
        if self.index.get(&id).is_none() {
            return (None, false);
        }
        if id == self.wallet {
            // Figma 28: the wallet chosen on the unlock screen is this one.
            return match password {
                Some(password) if self.unlocked.is_none() => (
                    Some(Event::Done {
                        what: "switch",
                        result: self.unlock_counted(&password).await,
                    }),
                    true,
                ),
                _ => (None, false),
            };
        }
        // The other wallet's password first: a wrong one changes nothing here but its
        // count, which the next snapshot shows (FEAT-0002).
        let opened = match &password {
            Some(password) => match self.open_other_counted(&id, password).await {
                Ok(opened) => Some(opened),
                Err(e) => {
                    return (
                        Some(Event::Done {
                            what: "switch",
                            result: Err(e),
                        }),
                        true,
                    );
                }
            },
            None => None,
        };
        self.index.last = Some(id.clone());
        self.save_index();
        self.switch_to = Some(id);
        // The new core opens it as opened here (the loop answers "switch").
        self.switch_opened = opened;
        (None, true)
    }

    /// Renames a wallet and sets its colour.
    pub(super) fn rename_wallet(
        &mut self,
        id: String,
        name: String,
        color: u8,
    ) -> (Option<Event>, bool) {
        let event = {
            let name = name.trim().to_owned();
            let result = if name.is_empty() || name.chars().count() > 24 {
                Err(CoreError::NotAvailable(
                    tr!("A wallet name is 1 to 24 characters").into(),
                ))
            } else if let Some(record) = self.index.get_mut(&id) {
                record.name = name;
                record.color = color % wallets::COLORS;
                self.save_index();
                Ok(())
            } else {
                Err(CoreError::NotAvailable(
                    tr!("There is no such wallet").into(),
                ))
            };
            Event::Done {
                what: "wallet",
                result,
            }
        };
        (Some(event), true)
    }
}

/// A random number below `n` from the system's generator: which words of the phrase a check
/// asks must not be guessable.
pub(super) fn os_dice(n: usize) -> usize {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the OS random generator is available");
    (u64::from_le_bytes(bytes) % n.max(1) as u64) as usize
}

/// Opens the vault at `path` with `password`, off the async threads (Argon2id takes a while).
async fn open_vault_at(
    path: &std::path::Path,
    password: &Password,
) -> Result<(vault::VaultFile, Zeroizing<String>), CoreError> {
    let file = vault::load(path)
        .map_err(vault_error)?
        .ok_or(CoreError::NotAvailable(NO_VAULT.into()))?;
    let opened = file.clone();
    let password = Zeroizing::new(password.expose().to_owned());
    let phrase = tokio::task::spawn_blocking(move || vault::open(&opened, &password))
        .await
        .map_err(|_| CoreError::NotAvailable(tr!("The vault did not open").into()))?
        .map_err(vault_error)?;
    Ok((file, phrase))
}

/// The answer while a pause lasts (FEAT-0002).
fn too_many(until: u64) -> CoreError {
    CoreError::TooManyTries {
        until: Timestamp::try_from(until).unwrap_or(Timestamp::MAX),
    }
}

/// How a counted password check ended: only `WrongPassword` is the password's fault (UC-7).
fn counted<T>(result: &Result<T, CoreError>) -> crate::attempts::Outcome {
    use crate::attempts::Outcome;
    match result {
        Ok(_) => Outcome::Right,
        Err(CoreError::WrongPassword) => Outcome::Wrong,
        Err(_) => Outcome::Other,
    }
}

fn vault_error(e: VaultError) -> CoreError {
    match e {
        VaultError::WrongPassword => CoreError::WrongPassword,
        other => CoreError::NotAvailable(format!(tr!("Vault: {other}"), other = other)),
    }
}
