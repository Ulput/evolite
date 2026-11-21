//! A wallet and its password: making, opening, locking and deleting it, the payment password.

use crate::real::*;

impl Real {
    /// Asks the wallet to scan its compact filters again from `from` (BUG-64): a block it
    /// took a transaction out of is matched and applied again, and the store gets it. Once
    /// the sync is done, and only below the lowest height asked in this run, so it never
    /// repeats itself; the journal says why (`lost_block` or `stale`). What it is for — the
    /// `stale` lines of the mempool file, the `lost` ones of `mined.json` — is owed until the
    /// wallet's filters are at `up_to`, the tip when it was asked (`rescan_end`, [`OwedRescan`]).
    pub(in crate::real) async fn rescan_from(
        &mut self,
        id: WalletId,
        from: u32,
        why: &'static str,
        up_to: u32,
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
            OwedRescan::add(&mut self.rescan_owed, id, up_to, stale, lost);
        }
    }

    /// BUG-232: the filters scanned again from `from` after the pools took in addresses past
    /// their gap ([`Self::adopt_past_the_gap`]): the wallet gets their coins (a chain-locked
    /// record's output that became ours, vendor `evolite-finalized-outputs.patch`, review r1 M1)
    /// and reads the payments onward that are new to it. Asked even below
    /// an earlier rescan of this run (that one did not know these addresses), and leaving
    /// `rescanned_from` as it is (a later lost block or stale payment still gets its own).
    /// Taking an address in happens once, so this is asked rarely. Owed until the filters are
    /// synced (review M1); not asked when the wallet is below `from` already (an added account
    /// rewinds it, rust-dashcore#649): that pass reads the blocks with the longer pools.
    pub(in crate::real) async fn rescan_past_the_gap(
        &mut self,
        id: WalletId,
        from: u32,
        synced: u32,
        up_to: u32,
    ) {
        // Asked from `settle_rescans` once synced; the wallet below `from` is reading those
        // blocks already, with the longer pools: owed until it is back, for the coins it gives.
        if synced <= from {
            OwedRescan::add(&mut self.rescan_owed, id, up_to, Vec::new(), Vec::new());
            OwedRescan::for_past_the_gap(&mut self.rescan_owed, &id);
            return;
        }
        // Review r1 S2: not asked, it stays owed (asked again in a later snapshot).
        let Ok(manager) = self.chain.manager().await else {
            note_past_the_gap(&mut self.past_the_gap_from, id, from);
            return;
        };
        crate::diag::record(crate::diag::Record::Answer {
            what: "rescan",
            outcome: "past_the_gap",
        });
        let asked = matches!(
            tokio::task::spawn_blocking(move || manager.spv_rescan_filters_blocking(&id, from))
                .await,
            Ok(true)
        );
        if asked {
            OwedRescan::add(&mut self.rescan_owed, id, up_to, Vec::new(), Vec::new());
            OwedRescan::for_past_the_gap(&mut self.rescan_owed, &id);
        } else {
            note_past_the_gap(&mut self.past_the_gap_from, id, from);
        }
    }

    /// BUG-150: the filters scanned again from `from` (where the contacts begin, SYNC-7) for the contacts'
    /// receiving `accounts` built after the scan went past their coins. Owed until the wallet's
    /// filters are at `up_to`, the tip when it was asked (`rescan_end`). Asked even after another
    /// rescan in this run (that one did not know these accounts), but not while one for them is
    /// under way; noted done in `contact-rescans.json` only when it has run to its end.
    /// Whether it was asked.
    pub(in crate::real) async fn rescan_for_contacts(
        &mut self,
        id: WalletId,
        from: u32,
        up_to: u32,
        accounts: Vec<String>,
        coins: usize,
    ) -> bool {
        if accounts.is_empty() || self.chain.status().phase != sync::Phase::Synced {
            return false;
        }
        let Ok(manager) = self.chain.manager().await else {
            return false;
        };
        self.rescanned_from = Some(self.rescanned_from.map_or(from, |asked| asked.min(from)));
        // BUG-221: only when it is asked — not on every snapshot of the sync before it (QA saw
        // 29 lines in 40 s).
        crate::diag::record(crate::diag::Record::DashpayRescan {
            accounts: accounts.len() as u32,
            from_height: from,
        });
        crate::diag::record(crate::diag::Record::Answer {
            what: "rescan",
            outcome: "contact_account",
        });
        let asked = matches!(
            tokio::task::spawn_blocking(move || manager.spv_rescan_filters_blocking(&id, from))
                .await,
            Ok(true)
        );
        if asked {
            OwedRescan::add_contacts(&mut self.rescan_owed, id, up_to, accounts, coins);
        }
        asked
    }

    /// Decrypts the vault with `password`; Argon2 runs off the async threads.
    pub(in crate::real) async fn open_vault(
        &self,
        password: &Password,
    ) -> Result<(vault::VaultFile, vault::Secret), CoreError> {
        open_vault_at(&self.paths.vault, password).await
    }

    /// The vault of wallet `id` of this computer: this one's, or another's by its own
    /// password; a wallet not in the index has none.
    pub(in crate::real) async fn open_wallet_vault(
        &self,
        id: &str,
        password: &Password,
    ) -> Result<(vault::VaultFile, vault::Secret), CoreError> {
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
    pub(in crate::real) async fn change_password(
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
        // The old password is counted as the unlock's (BUG-203 r1 Must 1).
        let (file, secret) = self.open_vault_counted(old).await?;
        let new = Zeroizing::new(new.expose().to_owned());
        // FEAT-0041 R-15: written as v2, a v1 file too.
        let sealed = tokio::task::spawn_blocking(move || {
            vault::seal(&secret, &new, file.meta, KdfParams::default())
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
    pub(in crate::real) async fn delete_wallet(
        &mut self,
        password: Option<&Password>,
    ) -> Result<(), CoreError> {
        if self.saved.is_none() {
            return Err(CoreError::NotAvailable(NO_VAULT.into()));
        }
        if let Some(password) = password {
            self.open_vault_counted(password).await?;
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
    pub(in crate::real) async fn delete_other(
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
        self.open_other_counted(id, password).await?;
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
    pub(in crate::real) async fn ask_resync(&self) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let unfinished = self.identity_busy.is_busy()
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
    pub(in crate::real) async fn lock(&mut self) -> Result<(), CoreError> {
        // FEAT-0040…0046: off until the app says it again after the unlock.
        self.advanced_features = false;
        // An identity lookup still retrying stops and lets go of the keys; the balance
        // reading that follows a lookup stops too.
        self.discovery_run.fetch_add(1, Ordering::Relaxed);
        self.balances_stale.notify_one();
        // The token search stops; the next pass goes on from where it stopped.
        self.tokens.search.stop();
        // SEC-6: the background tasks holding the keys or the seed end now, not when their pass
        // does; what they marked as under way is set back for the next unlock.
        let cut = self.keyed_tasks.cut();
        if cut.contains(&super::keyed::POOL) {
            *self.pool_wallet.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        if cut.contains(&super::keyed::CONTACT_ACCOUNTS) {
            self.contact_accounts_busy.store(false, Ordering::Relaxed);
        }
        self.credits_checked.store(false, Ordering::Relaxed);
        // The names searched in this session go with the wallet (FEAT-0039, architect §3).
        self.name_search
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        // The right words of an unanswered phrase check go with the keys.
        self.phrase_check = None;
        // A paid write not known to have gone through is forgotten (BUG-184), unless its send
        // still runs: its late line comes after the lock too (BUG-205 S2).
        self.unclear.forget();
        // FEAT-0020-resend R-7: a batch running stops before its next contact; the rest are owed.
        self.resend_run.lock();
        // FEAT-0038 (SEC-6): a series stops before its next write; the ones sent finish.
        self.series_run.lock();
        // BUG-194: the address pools as the lock leaves them, with what is used.
        if let Some(open) = self.unlocked.as_ref()
            && let Some(wallet) = self.chain.wallet(&open.id).await
        {
            self.chain.save_pools(&wallet).await;
        }
        self.pools_saved = None;
        // BUG-232 review r2 S1: rescans owed for coins are owed again at the next start.
        self.gap_pending.clear();
        // BUG-232 review S1 (fbc4326f): what was read of this wallet's store goes with it, so a
        // read of the next wallet's that fails never shows this one's (its own addresses would
        // make a payment between the two wallets read as between own addresses).
        self.stored_history.clear();
        self.stored_own.clear();
        self.consumed_locks.clear();
        self.history_read = None;
        // An app's request is not answered after the next unlock with what was shown before.
        self.app_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.unlocked = None;
        self.attach = None;
        // SEC-5 (П2): the onboarding's phrase and another wallet's opened by its password go
        // with the lock too; a switch takes its own before it locks (`run.rs`).
        self.pending = None;
        self.switch_opened = None;
        self.receive_address = None;
        // The notes' key and every decrypted note go with the keys (AC-T6).
        *self.notes.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.backup_key.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }

    /// Writes the index after a change; the folders stay the truth (see [`wallets::load`]).
    fn save_index(&self) {
        let _ = wallets::save(&self.paths.root, &self.index);
    }

    /// [`Self::unlock`] with this wallet's wrong passwords counted (FEAT-0002): in a pause it
    /// is refused before the vault is touched; else the try is written down as wrong before
    /// Argon2, then cleared by a right password or taken back by an error of another kind.
    pub(in crate::real) async fn unlock_counted(
        &mut self,
        password: &Password,
    ) -> Result<(), CoreError> {
        let id = self.wallet.clone();
        let before = self.attempts.begin(&id, wall_secs()).map_err(too_many)?;
        let result = self.unlock(password).await;
        self.attempts.record(&id, before, counted(&result));
        result
    }

    /// The open wallet's password before a secret is shown that is not the phrase (FEAT-0020
    /// R-10: a private key): always asked, whatever "Password for every payment" says, and its
    /// wrong tries counted with the unlock's — the same count and pause (FEAT-0004).
    pub(in crate::real) async fn checked_vault(
        &mut self,
        password: &Password,
    ) -> Result<(), CoreError> {
        self.open_vault_counted(password).await.map(|_| ())
    }

    /// [`Self::open_vault`] with the open wallet's wrong passwords counted, as
    /// [`Self::unlock_counted`]: "Show the phrase" (BUG-203) and [`Self::checked_vault`].
    pub(in crate::real) async fn open_vault_counted(
        &mut self,
        password: &Password,
    ) -> Result<(vault::VaultFile, vault::Secret), CoreError> {
        let id = self.wallet.clone();
        let before = self.attempts.begin(&id, wall_secs()).map_err(too_many)?;
        let result = self.open_vault(password).await;
        self.attempts.record(&id, before, counted(&result));
        result
    }

    /// [`Self::open_wallet_vault`] for "Open "…"" (`SwitchWallet` with the password) and
    /// "Show the phrase" of another wallet (BUG-203), counted as [`Self::unlock_counted`] for
    /// wallet `id`. A payment's password counts too since FEAT-0004 A (R-31), through
    /// [`Self::open_vault_counted`].
    pub(in crate::real) async fn open_other_counted(
        &mut self,
        id: &str,
        password: &Password,
    ) -> Result<(vault::VaultFile, vault::Secret), CoreError> {
        let before = self.attempts.begin(id, wall_secs()).map_err(too_many)?;
        let result = self.open_wallet_vault(id, password).await;
        self.attempts.record(id, before, counted(&result));
        result
    }

    /// Opens the vault and restarts sync with the wallet's keys.
    pub(in crate::real) async fn unlock(&mut self, password: &Password) -> Result<(), CoreError> {
        let (file, secret) = self.open_vault(password).await?;
        self.unlock_opened(file, secret).await
    }

    /// Unlocks with a vault already opened by its password (one Argon2 per switch, F2). The
    /// wallet is open at once; platform-wallet gets it when the chain runs (F3), so a password
    /// typed while the chain starts no longer waits for the start.
    pub(in crate::real) async fn unlock_opened(
        &mut self,
        file: vault::VaultFile,
        secret: vault::Secret,
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
        // FEAT-0041 R-4: the keys, the id and the notes' key from the one seed.
        let seed = seed_of(&secret)?;
        let passphrase = !secret.passphrase.is_empty();
        drop(secret);
        let keys = keys_from(seed_bytes(&seed))?;
        // The id platform-wallet gives the wallet: from the same root key.
        // Р1 (review r1): the update check may ask from now on, not at a bare start.
        self.wallet_opened
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let id = keys.compute_wallet_id();
        self.open_notes(seed_bytes(&seed), &meta.wallet_id).await;
        // SYNC-0: the sync-speed lines count from here.
        crate::diag::unlocked();
        self.note_unlock_locks();
        self.seed_contact_names();
        self.unlocked = Some(Unlocked {
            id,
            meta,
            keys: Some(keys),
            seed: Some(seed.clone()),
            passphrase,
        });
        self.attach = Some(seed);
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
    /// BUG-252, FEAT-0041 r1: whether «Features for advanced users» may be on now — the wallet
    /// unlocked, no wallet on this computer yet (the first onboarding: nothing is locked), or a
    /// wallet being added from an open one. Locked wallets, or adding from their choice, keep it
    /// off.
    pub(in crate::real) fn advanced_allowed(&self) -> bool {
        self.unlocked.is_some()
            || self.index.wallets.is_empty()
            || (self.adding_from.is_some() && self.adding_open)
    }

    /// The phrase stays pending until the vault is written, so a failure can be retried.
    /// `passphrase`: a new phrase's BIP-39 passphrase (FEAT-0041 R-2), only with the advanced
    /// features on — hidden is off, not only out of sight.
    pub(in crate::real) async fn create(
        &mut self,
        password: &Password,
        passphrase: Option<&passphrase::Passphrase>,
    ) -> Result<(), CoreError> {
        if password.expose().is_empty() {
            return Err(CoreError::WrongPassword);
        }
        too_short(password)?;
        if self.saved.is_some() {
            return Err(CoreError::NotAvailable(
                tr!("There is already a wallet on this computer").into(),
            ));
        }
        let (mut secret, restored) = self.pending.clone().ok_or(CoreError::BadPhrase)?;
        if let Some(passphrase) = passphrase.filter(|p| !p.is_empty()) {
            if restored {
                // A restored phrase brought its own with `UsePhrase`, checked there (r1 Could 2:
                // said as what it is, not «a bad phrase»).
                return Err(CoreError::NotAvailable(
                    tr!("A restored phrase brings its BIP-39 passphrase with its words").into(),
                ));
            }
            if !self.advanced_features {
                return Err(CoreError::NotAvailable(evolite_common::ADVANCED_OFF.into()));
            }
            secret.passphrase = checked_passphrase(passphrase)?;
        }
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
        let seed = seed_of(&secret)?;
        let passphrase = !secret.passphrase.is_empty();
        let keys = keys_from(seed_bytes(&seed))?;
        let meta = VaultMeta {
            network: network().into(),
            wallet_id: hex(&keys.compute_wallet_id()),
            birth_height,
            word_count: secret.phrase.split_whitespace().count() as u8,
            created_at: now() as u64,
        };
        let password = Zeroizing::new(password.expose().to_owned());
        let file = {
            let meta = meta.clone();
            tokio::task::spawn_blocking(move || {
                vault::seal(&secret, &password, meta, KdfParams::default())
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
        self.open_notes(seed_bytes(&seed), &meta.wallet_id).await;
        // SYNC-0: the sync-speed lines count from here.
        crate::diag::unlocked();
        self.note_unlock_locks();
        self.seed_contact_names();
        self.unlocked = Some(Unlocked {
            id,
            meta,
            keys: Some(keys),
            seed: Some(seed.clone()),
            passphrase,
        });
        self.attach = Some(seed);
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
    /// `always`: this spend asks for the password whatever the setting (an App Connect v2
    /// sign-in, which cannot be undone for the identity: FEAT-0023 round 4).
    pub(in crate::real) async fn spend_gate(
        &mut self,
        password: Option<&Password>,
        always: bool,
    ) -> Result<(), CoreError> {
        // AC-T2: a spend that asks no password touches no count.
        if !self.password_each_send && !always {
            return Ok(());
        }
        match password {
            None => Err(CoreError::NotAvailable(crate::PASSWORD_NEEDED.into())),
            // FEAT-0004 A (R-31, the customer agreed 2026-11-20): counted as every password of
            // the wallet — guessed in a spend's window, the right one would sign at once. Five
            // wrong ones pause the spends and the unlock alike; the right one clears the count.
            Some(password) => self.open_vault_counted(password).await.map(|_| ()),
        }
    }

    /// FEAT-0004 FR-7: whether `op` sends more than the wallet's threshold on mainnet (the tick
    /// is then asked). Testnet has no threshold.
    pub(in crate::real) async fn large_unconfirmed(&self, op: &Spend) -> bool {
        if crate::network::current() != evolite_api::Network::Mainnet {
            return false;
        }
        let fees = self.fees_now(0).await;
        large_refused_on(crate::network::current(), op, &fees, self.large_spend)
    }

    /// FEAT-0004 FR-7 (AC-T6): Settings → Security → the threshold of a large spend. Lower or on
    /// at once; higher or off only with the wallet's password, counted as every password.
    pub(in crate::real) async fn set_large_spend(
        &mut self,
        duffs: Option<Duffs>,
        password: Option<&Password>,
    ) -> Result<(), CoreError> {
        if self.unlocked.is_none() {
            return Err(CoreError::NotAvailable(tr!("The wallet is locked").into()));
        }
        if duffs.is_some_and(|d| d <= 0) {
            return Err(CoreError::BadAmount);
        }
        if evolite_api::confirm::weakens(self.large_spend, duffs) {
            let password = password.ok_or(CoreError::WrongPassword)?;
            self.open_vault_counted(password).await?;
        }
        security::save_large(&self.paths.root, &self.wallet, duffs).map_err(|e| {
            CoreError::NotAvailable(format!(tr!("The setting was not saved: {e}"), e = e))
        })?;
        self.large_spend = duffs;
        Ok(())
    }

    /// Settings → Security → "Password for every send": on at once; off only with the
    /// wallet's password, which [`Real::spend_gate`] would ask anyway.
    pub(in crate::real) async fn set_password_each_send(
        &mut self,
        on: bool,
        password: Option<&Password>,
    ) -> Result<(), CoreError> {
        if self.unlocked.is_none() {
            return Err(CoreError::NotAvailable(tr!("The wallet is locked").into()));
        }
        if !on {
            let password = password.ok_or(CoreError::WrongPassword)?;
            self.open_vault_counted(password).await?;
        }
        security::save(&self.paths.root, &self.wallet, on).map_err(|e| {
            CoreError::NotAvailable(format!(tr!("The setting was not saved: {e}"), e = e))
        })?;
        self.password_each_send = on;
        Ok(())
    }

    /// A new recovery phrase of `words` words, kept until the wallet is made.
    pub(in crate::real) fn new_phrase(&mut self, words: usize) -> (Option<Event>, bool) {
        let event = {
            let n = if words == 24 { 24 } else { 12 };
            match Mnemonic::generate(n, Language::English) {
                Ok(mnemonic) => {
                    let phrase = Zeroizing::new(mnemonic.phrase());
                    let words = phrase.split_whitespace().map(str::to_owned).collect();
                    self.pending = Some((vault::Secret::new(&phrase, ""), false));
                    self.pending_origin = wallets::Origin::Created;
                    Event::NewPhrase(Phrase::new(words))
                }
                Err(_) => Event::Done {
                    what: DoneWhat::Create,
                    result: Err(CoreError::BadPhrase),
                    ask: None,
                },
            }
        };
        (Some(event), true)
    }

    /// Takes a phrase to restore from, with its BIP-39 passphrase (FEAT-0041 R-1: always offered,
    /// the switch or not); with `dashpay`, reads what it has on Platform first.
    pub(in crate::real) async fn use_phrase(
        &mut self,
        phrase: Phrase,
        dashpay: bool,
    ) -> (Option<Event>, bool) {
        let event = {
            let text = Zeroizing::new(phrase.words().join(" "));
            let passphrase = checked_passphrase(phrase.passphrase());
            if !Mnemonic::validate(&text) {
                Event::PhraseAccepted(Err(CoreError::BadPhrase))
            } else if let Err(e) = passphrase {
                Event::PhraseAccepted(Err(e))
            } else if let Some(name) = self.same_wallet(&vault::Secret {
                phrase: text.clone(),
                passphrase: passphrase.clone().unwrap_or_default(),
            }) {
                // One phrase, one wallet on this computer.
                Event::PhraseAccepted(Err(CoreError::NotAvailable(if name.is_empty() {
                    evolite_common::SAME_WALLET.into()
                } else {
                    format!(
                        tr!("This wallet is already on this computer — “{name}”"),
                        name = name
                    )
                })))
            } else if !dashpay {
                let secret = vault::Secret {
                    phrase: text,
                    passphrase: passphrase.unwrap_or_default(),
                };
                self.pending = Some((secret, true));
                self.pending_origin = wallets::Origin::Restored;
                Event::PhraseAccepted(Ok(None))
            } else {
                self.pending_origin = wallets::Origin::DashPay;
                // Import from DashPay: what the phrase has on Platform is read now,
                // before the wallet is made, for O5. Its keys are dropped after the scan.
                let secret = vault::Secret {
                    phrase: text,
                    passphrase: passphrase.unwrap_or_default(),
                };
                let keys = seed_of(&secret).and_then(|seed| keys_from(seed_bytes(&seed)));
                self.pending = Some((secret, true));
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
    pub(in crate::real) async fn switch_wallet(
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
                        what: DoneWhat::Switch,
                        result: self.unlock_counted(&password).await,
                        ask: None,
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
                            what: DoneWhat::Switch,
                            result: Err(e),
                            ask: None,
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
    pub(in crate::real) fn rename_wallet(
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
                what: DoneWhat::Wallet,
                result,
                ask: None,
            }
        };
        (Some(event), true)
    }
}

/// A random number below `n` from the system's generator: which words of the phrase a check
/// asks must not be guessable.
pub(in crate::real) fn os_dice(n: usize) -> usize {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the OS random generator is available");
    (u64::from_le_bytes(bytes) % n.max(1) as u64) as usize
}

/// Opens the vault at `path` with `password`, off the async threads (Argon2id takes a while).
async fn open_vault_at(
    path: &std::path::Path,
    password: &Password,
) -> Result<(vault::VaultFile, vault::Secret), CoreError> {
    let file = vault::load(path)
        .map_err(vault_error)?
        .ok_or(CoreError::NotAvailable(NO_VAULT.into()))?;
    let opened = file.clone();
    let password = Zeroizing::new(password.expose().to_owned());
    let secret = tokio::task::spawn_blocking(move || vault::open(&opened, &password))
        .await
        .map_err(|_| CoreError::NotAvailable(tr!("The vault did not open").into()))?
        .map_err(vault_error)?;
    Ok((file, secret))
}

/// FEAT-0041 R-8, R-10: a BIP-39 passphrase as BIP-39 salts it (NFKD), or refused when it is
/// longer than 256 bytes so — never cut: a cut one is another, empty wallet.
pub(in crate::real) fn checked_passphrase(
    passphrase: &passphrase::Passphrase,
) -> Result<Zeroizing<String>, CoreError> {
    if !passphrase::fits(passphrase.expose()) {
        return Err(CoreError::NotAvailable(
            evolite_common::PASSPHRASE_TOO_LONG.into(),
        ));
    }
    Ok(passphrase::normalized(passphrase.expose()))
}

/// The answer to what needs an open wallet when it is locked.
pub(in crate::real) fn locked() -> CoreError {
    CoreError::NotAvailable(tr!("The wallet is locked").into())
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

/// FEAT-0004 FR-7 (AC-T5): whether a spend without the tick is refused — on mainnet only, when
/// it sends more than `threshold` (exactly it goes).
pub(in crate::real) fn large_refused_on(
    network: evolite_api::Network,
    op: &Spend,
    fees: &evolite_api::Fees,
    threshold: Option<Duffs>,
) -> bool {
    network == evolite_api::Network::Mainnet && evolite_api::confirm::asks_tick(op, fees, threshold)
}
