//! The core's loop: commands in, snapshots and answers out.

use super::*;

impl Real {
    /// Something arrived in the background: the next pass sends a fresh snapshot.
    pub(super) fn refresher(&self) -> impl Fn() + Send + Sync + Clone + 'static {
        let dirty = self.dirty.clone();
        let wake = self.wake.clone();
        move || {
            dirty.store(true, Ordering::Relaxed);
            wake();
        }
    }

    /// Asks the network off the loop and sends the answer when it comes. A name asked before
    /// proofs can be checked gets `NotSynced` at once and the real answer once the masternode
    /// lists are in; the interface keeps the last answer for the name it shows.
    pub(super) fn answer_later(
        &self,
        step: &'static str,
        question: impl Future<Output = Event> + Send + 'static,
    ) {
        let out = self.out.clone();
        let wake = self.wake.clone();
        let stale = self.balances_stale.clone();
        let question = turn::task(step, question);
        tokio::spawn(async move {
            let event = question.await;
            // A write (or a try at one) may have moved credits: read the balances again.
            if matches!(event, Event::Done { .. } | Event::Sent(_)) {
                stale.notify_one();
            }
            let _ = out.send(event);
            wake();
        });
    }

    /// The auto-lock time has passed since the last sign of the person.
    pub(super) fn idle_too_long(&self) -> bool {
        self.unlocked.is_some()
            && self
                .auto_lock
                .is_some_and(|after| self.last_touch.elapsed() >= after)
    }

    pub(super) fn publisher(&self) -> Arc<dyn Fn(&SyncStatus) + Send + Sync> {
        let dirty = self.dirty.clone();
        let wake = self.wake.clone();
        Arc::new(move |_: &SyncStatus| {
            dirty.store(true, Ordering::Relaxed);
            wake();
        })
    }
}

/// The core's thread: a tokio runtime, the sync client, and the command loop.
pub fn run(
    inbox: Receiver<Command>,
    out: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
    data_dir: PathBuf,
    art: crate::ArtOptions,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("evolite-core-io")
        // GroveDB proof checks recurse deeply (Dash Evo Tool reserves 8 MiB too).
        .thread_stack_size(8 << 20)
        .build()
    {
        Ok(r) => r,
        Err(_) => return,
    };
    runtime.block_on(async move {
        // Another evolite on this folder: say so and touch nothing until it closes.
        let mut said = false;
        let _lock = loop {
            match lock_data_dir(&data_dir) {
                Ok(Some(file)) => break Some(file),
                // A folder that cannot be locked at all (a read-only disk, a network share
                // without locks) is used as before: refusing would only hide the wallet.
                Err(_) => break None,
                Ok(None) => {}
            }
            if !said {
                said = true;
                let _ = out.send(Event::Snapshot(Box::new(Snapshot {
                    network: crate::network::current(),
                    sync: sync_info(&SyncStatus::default(), false),
                    vault: VaultState::NoWallet,
                    wallet: None,
                    demo: false,
                    elsewhere: true,
                    fiat: None,
                    wallets: Vec::new(),
                    adding_wallet: false,
                })));
                wake();
            }
            loop {
                match inbox.try_recv() {
                    Ok(_) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            }
            tokio::time::sleep(LOCK_RETRY).await;
        };
        // A data folder from before wallet folders is moved into a wallet folder of its own first (renames
        // only; one cut short is finished here), then the wallet open last is opened.
        let _ = wallets::migrate(&data_dir);
        let first = opening_wallet(&data_dir);
        clear_other_leftovers(&data_dir, &first);
        let mut core = Real::new(data_dir.clone(), first, out.clone(), wake.clone());
        if said {
            // The first window has closed: this one starts as usual.
            core.dirty.store(true, Ordering::Relaxed);
        }
        // Each wallet's own picture cache: emptied with a deleted wallet.
        let art_base = art.cache.clone();
        let art_for = |wallet: &str| crate::ArtOptions {
            cache: art_base.as_ref().map(|base| base.join(wallet)),
            worker: art.worker.clone(),
            off: art.off,
        };
        core.art_base = art_base.clone();
        // Profile pictures are drawn by the sandboxed renderer, never in this process.
        core.artist = Some(crate::art::Artist::start(
            out.clone(),
            wake.clone(),
            art_for(&core.wallet),
        ));
        // Headers, masternode lists and ChainLocks start at once, with any wallet saved in the
        // store. A failed start is in the status (and so on screen) and is tried again later.
        // What a deleted wallet left goes first: a store never opens over it. So does what a
        // resync asked last run to start over. A resync that cannot clear (a file held by
        // something else) is not lost: the mark stays and it is tried at the next start.
        let _ = resync_on_start(&core.paths);
        // The window needs no SPV client to ask for the password: it gets what is known now.
        // Starting the client opens the stored headers and filters — 12 s in a release build
        // with a synced store, half a minute in a debug one — and runs off the loop, so the
        // password typed meanwhile opens the wallet at once (F3, BUG-24).
        let snapshot = turn::timed("snapshot", core.snapshot()).await;
        let _ = out.send(Event::Snapshot(Box::new(snapshot)));
        wake();
        core.start_chain(None, None, None, false);
        // When the last DashPay pass the snapshot knows of finished.
        let mut dashpay_pass = None;
        loop {
            let _turn = turn::begin("loop_turn");
            if turn::timed("start_ended", core.start_ended()).await {
                core.dirty.store(true, Ordering::Relaxed);
            }
            core.retry_start();
            // The open wallet into platform-wallet as soon as the store is open, settled there
            // once the chain runs (BUG-25; a failure is tried again later, and the sync status
            // says what is wrong with the chain itself). The
            // identity lookup begun at the unlock starts again on the loaded wallet, without
            // its pause after "the wallet is not loaded yet".
            if core.attach.is_some()
                && turn::timed("attach_wallet", core.attach_wallet())
                    .await
                    .is_ok()
                && core.attach.is_none()
            {
                core.discover_identities();
            }
            // Platform address balances come with proofs: from the masternode lists on.
            if core.chain.platform_ready() {
                turn::timed("start_platform_sync", core.chain.start_platform_sync()).await;
                core.connect_pool();
                core.build_contact_accounts();
                core.read_platform_history();
                // A DashPay pass brings requests and contacts without any other event: the
                // snapshot is made again after each one (BUG-13).
                let synced = turn::timed("dashpay_synced_at", core.chain.dashpay_synced_at()).await;
                if new_pass(&mut dashpay_pass, synced) {
                    core.dirty.store(true, Ordering::Relaxed);
                }
            }
            loop {
                match inbox.try_recv() {
                    Ok(command) => {
                        let name = crate::diag::command_name(&command);
                        let (event, changed) = turn::timed(name, core.handle(command)).await;
                        answer(&out, &*wake, event);
                        if changed {
                            core.dirty.store(true, Ordering::Relaxed);
                        }
                        if core.restart || core.switch_to.is_some() {
                            // A deleted wallet, or another one asked for: this one is
                            // locked (its keys gone with this core); a new core opens the next
                            // wallet, as at start, and is shown at once. This one's chain is
                            // stopped and its store let go off the loop, before the new chain
                            // starts (F3, BUG-21/22/24): that took 11–18 s here. "Add a
                            // wallet" parks it instead, and "Back" takes it on (BUG-76,
                            // [`hand_on_chain`]).
                            let deleted = core.restart.then(|| core.wallet.clone());
                            let next = core.switch_to.take();
                            let opened = core.switch_opened.take();
                            // "Cancel" while adding goes back to this one.
                            let adding_from = core.adding_from.take();
                            // "Add a wallet" asked here: this chain is parked (BUG-76).
                            let adding =
                                !core.restart && adding_from.as_deref() == Some(&*core.wallet);
                            let _ = turn::timed("switch_lock", core.lock()).await;
                            // A deleted wallet's pictures go too.
                            if let (Some(base), Some(id)) = (&art_base, &deleted) {
                                let _ = std::fs::remove_dir_all(base.join(id));
                            }
                            drop(core.artist.take());
                            let wallet = next.unwrap_or_else(|| opening_wallet(&data_dir));
                            let mut old = core;
                            core = Real::new(data_dir.clone(), wallet, out.clone(), wake.clone());
                            core.art_base = art_base.clone();
                            // Back at the wallet the adding started from: adding is over.
                            core.adding_from = adding_from.filter(|from| *from != core.wallet);
                            core.artist = Some(crate::art::Artist::start(
                                out.clone(),
                                wake.clone(),
                                art_for(&core.wallet),
                            ));
                            hand_on_chain(&mut old, &mut core, adding);
                            drop(old);
                            // Figma 29: the vault opened before the switch opens the wallet now
                            // (no second Argon2, F2).
                            if let Some((file, phrase)) = opened {
                                let result =
                                    turn::timed("switch_unlock", core.unlock_opened(file, phrase))
                                        .await;
                                let _ = out.send(Event::Done {
                                    what: "switch",
                                    result,
                                });
                            }
                            core.dirty.store(false, Ordering::Relaxed);
                            let snapshot = turn::timed("snapshot", core.snapshot()).await;
                            let _ = out.send(Event::Snapshot(Box::new(snapshot)));
                            wake();
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        turn::timed("shut_down", core.shut_down()).await;
                        return;
                    }
                }
            }
            if core.idle_too_long() {
                let result = turn::timed("idle_lock", core.lock()).await;
                let _ = out.send(Event::Done {
                    what: "lock",
                    result,
                });
                core.dirty.store(true, Ordering::Relaxed);
            }
            if core.dirty.swap(false, Ordering::Relaxed) {
                let snapshot = turn::timed("snapshot", core.snapshot()).await;
                let _ = out.send(Event::Snapshot(Box::new(snapshot)));
                wake();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });
}

/// After a command: its answer, if any, goes to the interface, which is woken either way. An
/// answer without a new snapshot ("Wrong password") would otherwise wait for the next mouse move
/// to be drawn; and a command may have sent its answer itself, from inside `handle`.
pub(super) fn answer(out: &Sender<Event>, wake: &(dyn Fn() + Send + Sync), event: Option<Event>) {
    if let Some(event) = event {
        let _ = out.send(event);
    }
    wake();
}

/// Whether a sync pass finished since `last` (and notes it): `at` is when the latest did.
pub(super) fn new_pass(last: &mut Option<u64>, at: Option<u64>) -> bool {
    if at.is_none() || at == *last {
        return false;
    }
    *last = at;
    true
}
