//! Starting and stopping the chain, and the data folder: what a wallet's opening and closing leave behind.

use super::*;

impl Real {
    /// Inside a tokio runtime: the idle timer is a tokio instant.
    pub(super) fn new(
        data_dir: PathBuf,
        wallet: String,
        out: Sender<Event>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let paths = Paths::new(data_dir, &wallet);
        let index = wallets::load(&paths.root);
        // A vault that cannot be read still exists: "Locked", and unlocking says why.
        let saved = match vault::load(&paths.vault) {
            Ok(file) => file.map(|f| f.meta),
            Err(_) => Some(VaultMeta {
                network: network().into(),
                wallet_id: String::new(),
                birth_height: 0,
                word_count: 0,
                created_at: 0,
            }),
        };
        let journal = Arc::new(journal::Journal::new(paths.journal.clone()));
        let mempool = mempool::MempoolSeen::new(paths.mempool.clone());
        let mined = mined::Mined::new(paths.mined.clone());
        let core_balance = core_balance::CoreBalance::new(paths.core_balance.clone());
        let contact_paid = Arc::new(contact_addresses::ContactAddresses::new(
            paths.contact_addresses.clone(),
        ));
        let hidden = Arc::new(hidden_requests::HiddenRequests::new(
            paths.hidden_requests.clone(),
        ));
        let pool_history = pool_history::PoolHistory::new(paths.pool_history.clone());
        let paths_apps = paths.connected_apps.clone();
        let password_each_send = security::load(&paths.root, &wallet);
        let attempts =
            attempts::Attempts::read(&paths.root, index.wallets.iter().map(|r| r.id.as_str()));
        Self {
            wallet,
            index,
            notes: Arc::default(),
            attempts,
            pending_origin: wallets::Origin::Created,
            switch_to: None,
            switch_opened: None,
            adding_from: None,
            parked: None,
            password_each_send,
            starting: None,
            stopped: None,
            start_tried: false,
            last_start: tokio::time::Instant::now(),
            attach: None,
            attach_failed: None,
            birth_hint: None,
            journal,
            history_busy: Arc::new(AtomicBool::new(false)),
            history_next: None,
            contact_paid,
            hidden,
            pool_history,
            address_book: address_book::AddressBook::new(paths.address_book.clone()),
            paths,
            chain: Arc::new(Chain::default()),
            out,
            dirty: Arc::new(AtomicBool::new(true)),
            wake,
            pending: None,
            saved,
            unlocked: None,
            restart: false,
            leftovers: false,
            receive_address: None,
            mempool,
            fresh_receive: true,
            fiat: Default::default(),
            fiat_task: None,
            disk: None,
            nodes: None,
            // The settings' default until the interface sends its own.
            auto_lock: Some(Duration::from_secs(15 * 60)),
            last_touch: tokio::time::Instant::now(),
            avatars: Default::default(),
            nfts: Default::default(),
            artist: None,
            art_base: None,
            names: Default::default(),
            names_asked: Default::default(),
            contacts: Vec::new(),
            identity_busy: Arc::new(AtomicBool::new(false)),
            pool_wallet: Arc::new(std::sync::Mutex::new(None)),
            pool_busy: Arc::new(AtomicBool::new(false)),
            contact_accounts_for: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            contact_accounts_busy: Arc::new(AtomicBool::new(false)),
            identities_found: Arc::new(AtomicBool::new(false)),
            identities_complete: Arc::new(AtomicBool::new(false)),
            identity_search: Default::default(),
            discovery_run: Default::default(),
            balances_stale: Default::default(),
            phrase_check: None,
            stored_history: Vec::new(),
            consumed_locks: std::collections::HashSet::new(),
            top_ups: Default::default(),
            history_read: None,
            mined,
            core_balance,
            rescanned_from: None,
            rescan_owed: None,
            outgoing: Default::default(),
            platform_sending: Default::default(),
            previews: Default::default(),
            apps: Arc::new(connected_apps::ConnectedApps::new(paths_apps)),
            app_requests: Default::default(),
            next_app_request: Default::default(),
        }
    }

    /// Starts this core's chain off the loop (F3, BUG-24): first `old` — the chain of the core
    /// before — is stopped, after the start before it (`prev`) has ended, since both use the
    /// same SPV files; then what deleted wallets left is removed (`others`: the other wallets'
    /// folders too), and the chain starts over a clear folder. The loop meanwhile takes commands
    /// and sends snapshots; a start of the core's own that failed is tried again by the loop
    /// (BUG-8 waited for it inside the unlock).
    ///
    /// The store is this wallet's own (only the SPV files are shared): once every chain before
    /// `old` has let its store go (`before`, said by the start of `old`), it is opened while
    /// `prev` still runs, so what it has is shown at once (BUG-65: going back from "Add a
    /// wallet" waited for the adding chain's whole start, ≈50 s, with nothing to show). The loop
    /// passes `before` only when `old` is another wallet's chain: no store is open twice.
    pub(super) fn start_chain(
        &mut self,
        old: Option<Arc<Chain>>,
        prev: Option<tokio::task::JoinHandle<bool>>,
        before: Option<watch::Receiver<bool>>,
        others: bool,
    ) {
        self.chain.mark_starting();
        self.last_start = tokio::time::Instant::now();
        let (chain, paths, wallet) = (self.chain.clone(), self.paths.clone(), self.wallet.clone());
        let publish = self.publisher();
        let refresh = self.refresher();
        let (stopped, after) = watch::channel(false);
        self.stopped = Some(after);
        // A saved wallet's folder is no deleted one's leftovers: nothing removes it below.
        let ahead = before.filter(|_| paths.vault.exists());
        self.starting = Some(tokio::spawn(async move {
            let open_ahead = async {
                if let Some(mut before) = ahead
                    && before.wait_for(|stopped| *stopped).await.is_ok()
                    && chain.open(&paths.chain, publish.clone()).await.is_ok()
                    && chain.store_open()
                {
                    refresh();
                }
            };
            let stop_old = async {
                if let Some(prev) = prev {
                    let _ = prev.await;
                }
                if let Some(old) = old {
                    old.stop().await;
                }
            };
            tokio::join!(open_ahead, stop_old);
            // Every chain before this one has let its store go: the next one may open its own.
            let _ = stopped.send(true);
            if others {
                clear_other_leftovers(&paths.root, &wallet);
            }
            // A replaced chain starts nothing: the next core's start clears its folder after
            // stopping it. A store opened ahead holds the folder: a wallet deleted meanwhile
            // cannot be removed under it (that was tried for 10 s, holding up the next start,
            // BUG-65 r1) — it is not started either, and the next start clears it.
            let clear = if chain.retired() {
                false
            } else if chain.store_open() {
                paths.vault.exists()
            } else {
                clear_leftovers(&paths).await
            };
            // The SPV files may still be let go by the stopped chain's last tasks (BUG-8): a
            // failed start is tried again at once for a while, then by the loop.
            let deadline = tokio::time::Instant::now() + START_AGAIN;
            while clear
                && !chain.retired()
                && chain.start(&paths.chain, publish.clone()).await.is_err()
                && tokio::time::Instant::now() < deadline
            {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            refresh();
            clear
        }));
    }

    /// The start under way has ended: what it found is taken (true then).
    pub(super) async fn start_ended(&mut self) -> bool {
        if !self.starting.as_ref().is_some_and(|t| t.is_finished()) {
            return false;
        }
        if let Some(task) = self.starting.take() {
            self.start_tried = task.await.unwrap_or(false);
            self.leftovers = !self.start_tried;
        }
        true
    }

    /// Waits up to `wait` for a start under way, and takes what it found.
    pub(super) async fn start_settled(&mut self, wait: Duration) {
        let Some(task) = self.starting.as_mut() else {
            return;
        };
        // A handle that has answered must not be polled again: its answer is taken here.
        if let Ok(answer) = tokio::time::timeout(wait, task).await {
            self.starting = None;
            self.start_tried = answer.unwrap_or(false);
            self.leftovers = !self.start_tried;
        }
    }

    /// No start under way and the last one failed (offline at launch, say, or a deleted
    /// wallet's files still held): tried again every [`RETRY_START`].
    pub(super) fn retry_start(&mut self) {
        // An onboarding over a parked chain starts its own once its wallet is written; the
        // parked one is the chain to keep trying meanwhile.
        if self.parked.is_some() {
            self.retry_parked_start();
            return;
        }
        let failed = !self.start_tried || self.chain.status().phase == Phase::Error;
        if self.starting.is_none()
            && !self.chain.running()
            && failed
            && self.last_start.elapsed() >= RETRY_START
        {
            self.start_chain(None, None, None, false);
        }
    }

    /// During an onboarding the parked chain is the one that reaches the network: one not
    /// running ("Add a wallet" while offline, say, or its start failed) is started again every
    /// [`RETRY_START`], with its own callbacks — else a new wallet would be born at 0 and a
    /// DashPay import would find nothing (BUG-76 r1, should 1). Its folder is a saved wallet's:
    /// nothing is cleared. A start under way is not doubled; one retired by
    /// [`Self::start_after`] is not made.
    fn retry_parked_start(&mut self) {
        let wake = self.wake.clone();
        let root = self.paths.root.clone();
        let Some(parked) = self.parked.as_mut() else {
            return;
        };
        if parked.chain.running()
            || parked.starting.as_ref().is_some_and(|t| !t.is_finished())
            || parked.last_start.elapsed() < RETRY_START
        {
            return;
        }
        let chain = parked.chain.clone();
        let paths = Paths::new(root, &parked.wallet);
        let dirty = parked.dirty.clone();
        let publish: Arc<dyn Fn(&SyncStatus) + Send + Sync> = {
            let (dirty, wake) = (dirty.clone(), wake.clone());
            Arc::new(move |_: &SyncStatus| {
                dirty.store(true, Ordering::Relaxed);
                wake();
            })
        };
        chain.mark_starting();
        parked.last_start = tokio::time::Instant::now();
        // A finished start is replaced. Its answer, as [`Self::start_chain`]'s, is whether the
        // folder is clear (taken over on "Back" as `start_tried`), not whether it runs.
        parked.starting = Some(tokio::spawn(async move {
            let clear = !chain.retired() && paths.vault.exists();
            if clear {
                let _ = chain.start(&paths.chain, publish).await;
            }
            dirty.store(true, Ordering::Relaxed);
            wake();
            clear
        }));
    }

    /// The chain that reaches the network now: during an onboarding over a parked chain,
    /// that one (headers for a new wallet's birth, Platform for a DashPay import); else this
    /// core's own.
    pub(super) fn network_chain(&self) -> &Arc<Chain> {
        self.parked.as_ref().map_or(&self.chain, |p| &p.chain)
    }

    /// "Add a wallet": this core's chain and what goes with it, for the onboarding to keep
    /// running ([`Parked`]). Its keys are gone already (the loop locks the core first).
    fn park(&mut self) -> Parked {
        Parked {
            wallet: self.wallet.clone(),
            chain: std::mem::take(&mut self.chain),
            starting: self.starting.take(),
            stopped: self.stopped.take(),
            start_tried: self.start_tried,
            leftovers: self.leftovers,
            last_start: self.last_start,
            dirty: self.dirty.clone(),
            rescanned_from: self.rescanned_from,
            rescan_owed: self.rescan_owed.take(),
            pool_wallet: self.pool_wallet.clone(),
            contact_accounts_for: self.contact_accounts_for.clone(),
            contact_accounts_busy: self.contact_accounts_busy.clone(),
        }
    }

    /// "Back" (or the same wallet chosen again): the parked chain is this core's, running as
    /// it was — nothing stopped, nothing started, the store open all along (BUG-76).
    fn take_over(&mut self, parked: Parked) {
        debug_assert_eq!(parked.wallet, self.wallet);
        self.chain = parked.chain;
        self.starting = parked.starting;
        self.stopped = parked.stopped;
        self.start_tried = parked.start_tried;
        self.leftovers = parked.leftovers;
        self.last_start = parked.last_start;
        self.dirty = parked.dirty;
        self.dirty.store(true, Ordering::Relaxed);
        // Not asked again from the birth: the same run of the same chain (BUG-64).
        self.rescanned_from = parked.rescanned_from;
        // And the rescan asked before "Add a wallet", noted once it has run to its end.
        self.rescan_owed = parked.rescan_owed;
        self.pool_wallet = parked.pool_wallet;
        self.contact_accounts_for = parked.contact_accounts_for;
        self.contact_accounts_busy = parked.contact_accounts_busy;
    }

    /// Any other way out of the onboarding: the parked chain is stopped — its SPV client shut
    /// down, then its wallets removed (R-29) — and only then does this core's own start, as
    /// after a switch (BUG-25/65). `ahead`: this core's store may open while the parked chain
    /// stops, once the chains before it have let theirs go (another wallet's, never the
    /// parked one's).
    pub(super) fn start_after(&mut self, parked: Parked, ahead: bool) {
        // A start of the parked chain still waiting in line is not made.
        parked.chain.retire();
        let before = if ahead {
            open_ahead_after(&parked.wallet, &self.wallet, parked.stopped)
        } else {
            None
        };
        self.start_chain(Some(parked.chain), parked.starting, before, true);
    }

    /// The app closes: every chain of this core stops — its own and a parked one — after any
    /// start under way.
    pub(super) async fn shut_down(&mut self) {
        if let Some(task) = self.starting.take() {
            let _ = task.await;
        }
        self.chain.stop().await;
        if let Some(parked) = self.parked.take() {
            parked.chain.retire();
            if let Some(task) = parked.starting {
                let _ = task.await;
            }
            parked.chain.stop().await;
        }
    }

    /// The open wallet into platform-wallet once the store is open: the one saved there, so
    /// its identities, balance and history are shown at once, before the SPV client has
    /// started (BUG-25), or a new watch-only entry. The phrase is kept until the client runs:
    /// a client that does not start lets its store go, and the wallet goes into the next one.
    pub(super) async fn attach_wallet(&mut self) -> Result<(), String> {
        if !self.chain.store_open()
            || self
                .attach_failed
                .is_some_and(|at| at.elapsed() < RETRY_START)
        {
            return Ok(());
        }
        let (Some(phrase), Some(open)) = (self.attach.as_ref(), self.unlocked.as_ref()) else {
            return Ok(());
        };
        // Read before the wallet is opened: once it runs, the client syncs this manager's.
        let running = self.chain.running();
        if !running && self.chain.wallet(&open.id).await.is_some() {
            // In already; the client is still starting.
            return Ok(());
        }
        match self.chain.open_wallet(phrase, open.meta.birth_height).await {
            Ok(_) => {
                if running {
                    self.attach = None;
                }
                self.attach_failed = None;
                self.history_read = None;
                self.dirty.store(true, Ordering::Relaxed);
                Ok(())
            }
            Err(e) => {
                self.attach_failed = Some(tokio::time::Instant::now());
                Err(e)
            }
        }
    }
}

/// The resync asked for last run, before anything opens: the headers, filters and
/// masternode lists (`spv`) and the wallet's store (with SQLite's `-wal` and `-shm`) go, so
/// sync starts from scratch and the wallet is scanned again from its birth at the unlock. The
/// vault, the names of identities being made, the Platform journal, contacts' addresses and
/// the payment notes stay — none of them comes back from the chain. Returns whether it cleared.
pub fn resync_on_start(paths: &Paths) -> std::io::Result<bool> {
    if !paths.resync.exists() {
        return Ok(false);
    }
    let gone = |r: std::io::Result<()>| match r {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    };
    gone(std::fs::remove_dir_all(&paths.chain.spv))?;
    for suffix in ["", "-wal", "-shm"] {
        let mut file = paths.chain.store.clone().into_os_string();
        file.push(suffix);
        gone(std::fs::remove_file(file))?;
        // The pool's notes are read again from Platform with the rest.
        let mut file = paths.pool.clone().into_os_string();
        file.push(suffix);
        gone(std::fs::remove_file(file))?;
    }
    // Platform is read back from the start; what the journal holds stays (BUG-58).
    gone(std::fs::remove_file(&paths.platform_sync))?;
    // The DashPay backfill record vouches for a scan that is gone with the store (BUG-77); the
    // spent-coin claims are read from the store itself and go with it.
    gone(std::fs::remove_file(store::backfill_file(
        &paths.chain.store,
    )))?;
    std::fs::remove_file(&paths.resync)?;
    Ok(true)
}

/// The open wallet `id` in platform-wallet, or why not, in the interface's terms.
pub(super) async fn loaded(
    chain: &Chain,
    id: &WalletId,
) -> Result<Arc<platform_wallet::PlatformWallet>, CoreError> {
    // Still starting (F3): the wallet is open, its sync not yet — "wait for the sync".
    if !chain.running() {
        return Err(CoreError::NotSynced);
    }
    chain
        .wallet(id)
        .await
        .ok_or_else(|| CoreError::NotAvailable(NOT_LOADED.into()))
}

/// The data folder for this process alone: an exclusive lock on `evolite.lock` in it, held
/// while the returned file lives (the OS lets go if the process dies). `Ok(None)`: another
/// evolite has it — two writers on one store and one vault would corrupt them.
pub fn lock_data_dir(data_dir: &std::path::Path) -> std::io::Result<Option<std::fs::File>> {
    std::fs::create_dir_all(data_dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data_dir.join("evolite.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// A wallet folder without its vault is what a deletion left: the store, names,
/// journal and contacts' addresses of a wallet no longer here. Removed before a store opens
/// over them — tried for a while, since tasks of the deleted wallet may still be letting go.
/// Returns whether nothing is left.
pub(super) async fn clear_leftovers(paths: &Paths) -> bool {
    let Some(folder) = paths.vault.parent() else {
        return true;
    };
    if paths.vault.exists() || !folder.exists() {
        return true;
    }
    for _ in 0..40 {
        match std::fs::remove_dir_all(folder) {
            Ok(()) => return true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
    false
}

/// When the next core may open its store ahead of its SPV client ([`Real::start_chain`],
/// BUG-65): only when the core before had another wallet. The same wallet again (a restart
/// after a delete that failed, say) waits for the chain before to let its store go: one
/// store is never open twice.
pub(super) fn open_ahead_after(
    from: &str,
    to: &str,
    before: Option<watch::Receiver<bool>>,
) -> Option<watch::Receiver<bool>> {
    before.filter(|_| from != to)
}

/// The replaced core's chain, for the core replacing it (`new`, made by the loop; `old` is
/// locked): with the tip it saw, a new wallet's birth.
/// - `adding` ("Add a wallet" asked in `old`): parked in the onboarding, running (BUG-76);
/// - an onboarding over a parked chain, back to that chain's wallet: `new` takes it as it is;
///   to any other wallet: it stops first, then `new` starts its own (its store opened ahead
///   when the chains before have let theirs go). The onboarding's own chain never started;
/// - else, as before: `old`'s chain stops off the loop, then `new`'s starts (BUG-21/22/24),
///   its store opened ahead only for another wallet (BUG-65).
pub(super) fn hand_on_chain(old: &mut Real, new: &mut Real, adding: bool) {
    new.birth_hint = old
        .network_chain()
        .status()
        .headers
        .map(|h| h.height)
        .max(old.birth_hint);
    if adding {
        new.parked = Some(old.park());
        return;
    }
    if let Some(parked) = old.parked.take() {
        old.chain.retire();
        if parked.wallet == new.wallet {
            new.take_over(parked);
            // The onboarding's empty folder, which a start would have cleared.
            clear_other_leftovers(&new.paths.root, &new.wallet);
        } else {
            new.start_after(parked, true);
        }
        return;
    }
    let (chain, prev) = (old.chain.clone(), old.starting.take());
    // A start of this chain still waiting in line is not made: the next core's start follows
    // it (switching on before one ended).
    chain.retire();
    let before = open_ahead_after(&old.wallet, &new.wallet, old.stopped.take());
    new.start_chain(Some(chain), prev, before, true);
}

/// The wallet to open: the one open last (or the first) of the index, else a new folder for
/// the onboarding — it becomes a wallet once its vault is written there.
pub(super) fn opening_wallet(data_dir: &std::path::Path) -> String {
    wallets::load(data_dir)
        .current()
        .map(|r| r.id.clone())
        .unwrap_or_else(|| wallets::new_id(data_dir))
}

/// What deleted wallets (or onboardings left halfway) left in `wallets/<network>/`: folders
/// without a vault, other than `keep` — the one about to open. Best effort: a folder still held
/// is tried again at the next start or switch.
pub(super) fn clear_other_leftovers(data_dir: &std::path::Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(wallets::network_dir(data_dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir()
            && entry.file_name().to_str() != Some(keep)
            && !path.join("wallet.json").exists()
        {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}
