//! The private pool: connecting it, moving DASH in and out, paying from it.

use crate::real::*;

impl Real {
    /// Connects the open wallet's shielded pool once per run, in the background. The
    /// first time needs the seed (the viewing keys are saved then), so only an unlocked
    /// wallet; a failure is tried again a minute later.
    pub(in crate::real) fn connect_pool(&self) {
        if !cfg!(feature = "pool") {
            return;
        }
        let Some((id, Some(seed))) = self.unlocked.as_ref().map(|o| (o.id, o.seed.clone())) else {
            return;
        };
        {
            let mut connected = self.pool_wallet.lock().unwrap_or_else(|e| e.into_inner());
            if *connected == Some(id) {
                return;
            }
            *connected = Some(id);
        }
        let chain = self.chain.clone();
        let slot = self.pool_wallet.clone();
        let db = self.paths.pool.clone();
        let dirty = self.dirty.clone();
        let wake = self.wake.clone();
        let working = self.work(PlatformWork::Pool);
        let task = tokio::spawn(async move {
            let result = match chain.wallet(&id).await {
                Some(wallet) => {
                    chain
                        .connect_pool(&wallet, super::super::seed_bytes(&seed), &db)
                        .await
                }
                None => Err(tr!("the wallet is not loaded").to_owned()),
            };
            drop(working);
            match result {
                Ok(()) => {
                    dirty.store(true, Ordering::Relaxed);
                    wake();
                }
                Err(e) => {
                    eprintln!("evolite: the shielded pool did not connect: {e}");
                    tokio::time::sleep(POOL_RETRY).await;
                    let mut connected = slot.lock().unwrap_or_else(|e| e.into_inner());
                    if *connected == Some(id) {
                        *connected = None;
                    }
                }
            }
        });
        // SEC-6: the seed goes with the lock, not when the connection ends.
        self.keyed_tasks
            .add(super::keyed::POOL, task.abort_handle());
    }

    /// What every pool move needs: the open wallet with its keys and seed, proofs, a connected
    /// pool, and no other move under way (it is marked under way here).
    pub(in crate::real) fn pool_move(
        &self,
    ) -> Result<(WalletId, key_wallet::Wallet, Arc<super::super::Seed>), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let (Some(keys), Some(seed)) = (open.keys.clone(), open.seed.clone()) else {
            return Err(CoreError::NotAvailable(tr!("The keys are not open").into()));
        };
        if !cfg!(feature = "pool") {
            return Err(CoreError::NotAvailable(
                tr!("This build of evolite has no private pool").into(),
            ));
        }
        if !self.chain.platform_ready() {
            return Err(CoreError::NotSynced);
        }
        let connected = *self.pool_wallet.lock().unwrap_or_else(|e| e.into_inner());
        if connected != Some(open.id) {
            return Err(CoreError::NotAvailable(
                tr!("The private pool is still connecting — try again in a minute").into(),
            ));
        }
        if self.pool_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("A pool operation is already running — wait for it").into(),
            ));
        }
        Ok((open.id, keys, seed))
    }

    /// Runs a pool move off the loop: `Progress { what }` about every second while the proof
    /// is built (an estimate: the prover says nothing), then what `answer` makes of the result.
    pub(in crate::real) fn pool_job<T: Send + 'static>(
        &self,
        what: &'static str,
        job: impl Future<Output = Result<T, CoreError>> + Send + 'static,
        answer: impl FnOnce(Result<T, CoreError>) -> Event + Send + 'static,
    ) {
        let busy = self.pool_busy.clone();
        let out = self.out.clone();
        let wake = self.wake.clone();
        let ticking = busy.clone();
        tokio::spawn(async move {
            let started = tokio::time::Instant::now();
            while ticking.load(Ordering::Relaxed) {
                let fraction = (started.elapsed().as_secs_f32() / POOL_PROOF_SECS).min(0.95);
                let _ = out.send(Event::Progress { what, fraction });
                wake();
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        let dirty = self.dirty.clone();
        self.answer_later("task:pool_job", async move {
            let result = job.await;
            busy.store(false, Ordering::Relaxed);
            dirty.store(true, Ordering::Relaxed);
            answer(result)
        });
    }

    /// "Hide": credits of the identity the interface shows, the wallet's Platform
    /// addresses, or Core funds (an asset lock) into the pool, to the wallet's own private
    /// address.
    pub(in crate::real) fn shield(
        &self,
        from: ShieldSource,
        amount: Duffs,
    ) -> Result<(), CoreError> {
        let credits = positive_credits(amount)?;
        // Core coins are known only after the filters have been checked up to the tip.
        if from == ShieldSource::Core {
            let status = self.chain.status();
            if !coins_known(&status, true) || status.phase == Phase::Error {
                return Err(CoreError::NotSynced);
            }
        }
        let (id, keys, _) = self.pool_move()?;
        let chain = self.chain.clone();
        let journal = self.journal.clone();
        // BUG-206: the identity the screen shows when the command comes: the task checks, pays
        // and signs as it even if the choice moves while it runs, and refuses if it is gone.
        let pinned = self.shown_now(id);
        self.pool_job(
            "shield",
            async move {
                let wallet = loaded(&chain, &id).await?;
                // From the credits: the identity shown at the start, for all of it.
                let me = match from {
                    ShieldSource::Credits => Some(pinned_acting(&wallet, pinned).await.ok_or(
                        CoreError::NotAvailable(tr!("The wallet has no identity").into()),
                    )?),
                    _ => None,
                };
                // Credits and Platform addresses leave a line in the history (Core has its own,
                // the asset lock); what left them is read before and after.
                let (label, before) = match (from, &me) {
                    // The labels the layers read the source from (FEAT-0009).
                    (ShieldSource::Credits, Some((_, me))) => (
                        crate::layers::FROM_CREDITS,
                        chain.identity_credits(&wallet, me, false).await,
                    ),
                    (ShieldSource::Credits, None) => unreachable!("read above"),
                    (ShieldSource::PlatformAddress, _) => (
                        crate::layers::FROM_PLATFORM_ADDRESS,
                        chain.platform_address(&wallet).await.ok().map(|(_, c)| c),
                    ),
                    (ShieldSource::Core, _) => (crate::layers::FROM_CORE, None),
                };
                match (from, &me) {
                    (ShieldSource::Credits, Some((_, me))) => {
                        chain
                            .shield_from_identity(&wallet, &keys, me, credits)
                            .await
                    }
                    (ShieldSource::Credits, None) => unreachable!("read above"),
                    (ShieldSource::PlatformAddress, _) => {
                        chain
                            .shield_from_platform_address(&wallet, &keys, credits)
                            .await
                    }
                    // Duffs locked on Core; an unfinished lock is finished instead — only when
                    // that is what was asked (BUG-248 r1).
                    (ShieldSource::Core, _) => {
                        let duffs = credits / 1000;
                        stopped_shield_first(chain.unfinished_pool_shield(&wallet).await, duffs)?;
                        chain.shield_from_core(&wallet, &keys, duffs).await
                    }
                }
                .map_err(pool_error)?;
                if from != ShieldSource::Core {
                    let after = match &me {
                        Some((_, me)) => chain.identity_credits(&wallet, me, true).await,
                        None => chain.platform_address(&wallet).await.ok().map(|(_, c)| c),
                    };
                    let moved = amount;
                    let acting = me.map(|(acting, _)| acting);
                    journal.add(journal::Entry {
                        at: now(),
                        what: journal::What::Shield {
                            from: label.to_owned(),
                        },
                        // Platform addresses are read again only by their sync: until then the
                        // fee is not known, and the line shows the amount alone.
                        spent: spent_credits(before, after)
                            .filter(|s| *s > moved)
                            .unwrap_or(moved),
                        moved,
                        identity: acting,
                        ..Default::default()
                    });
                }
                Ok(())
            },
            |result| Event::Done {
                what: DoneWhat::Shield,
                result,
                ask: None,
            },
        );
        Ok(())
    }

    /// "Unshield": out of the pool to the wallet's own Platform address.
    pub(in crate::real) fn unshield(&self, amount: Duffs) -> Result<(), CoreError> {
        let credits = positive_credits(amount)?;
        let (id, _, seed) = self.pool_move()?;
        let chain = self.chain.clone();
        self.pool_job(
            "unshield",
            async move {
                let wallet = loaded(&chain, &id).await?;
                let (to, _) = chain
                    .platform_address(&wallet)
                    .await
                    .map_err(CoreError::Failed)?;
                chain
                    .unshield(&wallet, super::super::seed_bytes(&seed), &to, credits)
                    .await
                    .map_err(pool_error)
            },
            |result| Event::Done {
                what: DoneWhat::Unshield,
                result,
                ask: None,
            },
        );
        Ok(())
    }

    /// "Unshield" into the identity's credits: out of the pool into the identity the interface
    /// shows, which gets `amount`; Platform's fee comes from the notes on top.
    pub(in crate::real) fn top_up_from_pool(&self, amount: Duffs) -> Result<(), CoreError> {
        let credits = positive_credits(amount)?;
        let (id, _, seed) = self.pool_move()?;
        let chain = self.chain.clone();
        // BUG-206: the identity the screen shows when the command comes: the task checks, pays
        // and signs as it even if the choice moves while it runs, and refuses if it is gone.
        let pinned = self.shown_now(id);
        self.pool_job(
            "pool-top-up",
            async move {
                let wallet = loaded(&chain, &id).await?;
                // The recipient is the identity shown at the command (BUG-206 r1 Must 2).
                let (_, me) =
                    pinned_acting(&wallet, pinned)
                        .await
                        .ok_or(CoreError::NotAvailable(
                            tr!("The wallet has no identity").into(),
                        ))?;
                chain
                    .top_up_from_pool(&wallet, super::super::seed_bytes(&seed), &me, credits)
                    .await
                    .map_err(pool_error)
            },
            |result| Event::Done {
                what: DoneWhat::PoolTopUp,
                result,
                ask: None,
            },
        );
        Ok(())
    }

    /// "Send" from the pool: to a private address it stays private; to a Platform address
    /// it leaves the pool (and is seen); to a Core address it is withdrawn, paid out by Platform
    /// minutes later. Answered with `Sent`.
    pub(in crate::real) fn send_from_pool(
        &self,
        payment: u64,
        to: &str,
        amount: Duffs,
        kept: Kept,
    ) -> Result<(), CoreError> {
        enum To {
            Private,
            Platform,
            Core,
        }
        let credits = positive_credits(amount)?;
        let to = to.trim().to_owned();
        // Only this network's addresses: the same keys on the other one are another wallet.
        let kind = if to.to_ascii_lowercase().starts_with(platform_hrp()) {
            if pool::is_private_address(&to) {
                To::Private
            } else if PlatformAddress::from_bech32m_string(&to).is_ok() {
                To::Platform
            } else {
                return Err(CoreError::BadAddress);
            }
        } else {
            payee(&to)?
                .require_network(crate::network::dash())
                .map_err(|_| CoreError::BadAddress)?;
            To::Core
        };
        let (id, _, seed) = self.pool_move()?;
        let chain = self.chain.clone();
        // In the history from now on, as "being sent", until the pool's store has its line:
        // the proof takes tens of seconds before Platform is even asked (BUG-12).
        let sending = self.platform_sending.clone();
        let key = sending.lock().unwrap_or_else(|e| e.into_inner()).start(
            ActivityKind::PoolSent,
            amount,
            now(),
        );
        let notes = self.notes.clone();
        let signed = now();
        self.pool_job(
            "send",
            async move {
                let wallet = loaded(&chain, &id).await?;
                let sent = match kind {
                    To::Private => {
                        chain
                            .pool_transfer(&wallet, super::super::seed_bytes(&seed), &to, credits)
                            .await
                    }
                    To::Platform => {
                        chain
                            .unshield(&wallet, super::super::seed_bytes(&seed), &to, credits)
                            .await
                    }
                    To::Core => {
                        chain
                            .pool_withdraw(&wallet, super::super::seed_bytes(&seed), &to, credits)
                            .await
                    }
                }
                // Handed over but not proved: maybe gone, so "sent, no answer", never resent.
                .map(|()| true)
                .or_else(|e| match e {
                    pool::PoolError::Unconfirmed => Ok(false),
                    other => Err(pool_error(other)),
                });
                // Its line (and so its key) comes with the pool's store: the note waits for it,
                // encrypted, after a restart too (FEAT-0003).
                if sent.is_ok() && !kept.is_empty() {
                    keep_note(notes, move |n| {
                        n.set_pending(
                            notes::Pending::Pool,
                            (amount, signed),
                            kept.note,
                            kept.rate.as_ref(),
                        )
                    })
                    .await;
                }
                sent
            },
            move |result| {
                // "No answer" may still be taken: shown until the pool has it, or a while.
                sending.lock().unwrap_or_else(|e| e.into_inner()).answered(
                    key,
                    result.is_ok(),
                    now(),
                );
                Event::Sent {
                    id: payment,
                    result: result.map(|relayed| SentPayment {
                        txid: String::new(),
                        amount,
                        fee: 0,
                        relayed,
                    }),
                }
            },
        );
        Ok(())
    }
}

/// A pool move that failed, for the screens; one that may have gone says so and is not retried.
/// BUG-248 r1 Must: a shield from Core with a stopped one's lock still there (`locked`, duffs)
/// finishes that lock — so only when the amount asked is that lock's, as «Continue» asks.
/// Another amount is refused: the person would see one amount and the other lock would go,
/// past the threshold's tick for its size.
pub(in crate::real) fn stopped_shield_first(
    locked: Option<u64>,
    duffs: u64,
) -> Result<(), CoreError> {
    match locked {
        Some(locked) if locked != duffs => Err(CoreError::NotAvailable(
            tr!("First finish the stopped transfer to the pool — its money is already locked")
                .into(),
        )),
        _ => Ok(()),
    }
}

pub(in crate::real) fn pool_error(error: pool::PoolError) -> CoreError {
    match error {
        pool::PoolError::Unconfirmed => CoreError::Failed(
            tr!("Handed to the network, but not confirmed yet — do not repeat it: the next pool check shows the outcome").into(),
        ),
        // TECH-platform-errors: a key's refusal, a lack of credits — as for every write.
        pool::PoolError::Failed(text) => super::super::platform_failure(text),
    }
}

/// A positive amount in credits (1 duff = 1000 credits): a pool move, a credit transfer.
pub(in crate::real) fn positive_credits(amount: Duffs) -> Result<u64, CoreError> {
    positive_duffs(amount)?
        .checked_mul(1000)
        .ok_or(CoreError::BadAmount)
}

#[cfg(test)]
mod shield_tests {
    use super::*;

    /// BUG-248 r1 Must 1: a shield from Core waits for the sync to know the coins — before the
    /// wallet, the keys or the pool are even looked at.
    #[tokio::test]
    async fn a_shield_from_core_waits_for_the_sync() {
        let dir = std::env::temp_dir().join(format!("evolite-test-shield-{}", std::process::id()));
        let (out, _answers) = std::sync::mpsc::channel();
        let core = Real::new(dir.clone(), "0000beef".into(), out, Arc::new(|| {}));
        assert_eq!(
            core.shield(ShieldSource::Core, 100_000),
            Err(CoreError::NotSynced)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// BUG-248 r1 Must 2: with a stopped shield's lock there, only its own amount goes on (that
    /// lock, as «Continue» asks); another amount is refused with what to do first, and with no
    /// stopped one any amount goes.
    #[test]
    fn a_stopped_shield_is_finished_only_for_its_own_amount() {
        assert_eq!(stopped_shield_first(Some(100_000_000), 100_000_000), Ok(()));
        match stopped_shield_first(Some(100_000_000), 10_000_000) {
            Err(CoreError::NotAvailable(why)) => {
                assert!(
                    why.contains("First finish the stopped transfer to the pool"),
                    "{why}"
                )
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(stopped_shield_first(None, 10_000_000), Ok(()));
    }
}
