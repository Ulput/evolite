//! FEAT-0048: an identity created privately, from the pool (`IdentityCreateFromShieldedPool`).
//! Nothing moves on Core: the pool's notes pay a fixed exit, the identity gets it minus the fee,
//! the change stays in the pool. Before anything is sent, a record is written in the wallet's
//! folder (slot, exit, refusal address, name): a creation whose result is not proved is never
//! sent again (BUG-243), and the record says what to look for.

use super::super::*;
use evolite_chain::identity;
use evolite_chain::pool::{CreateNotes, PendingCreate};
use serde::{Deserialize, Serialize};

/// r1 Must 1: what an unproved creation left armed, as the record keeps it (see
/// [`PendingCreate`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::real) struct PendingRecord {
    pub identity_id: [u8; 32],
    pub activity: [u8; 32],
    pub anchor: [u8; 32],
    pub nullifiers: Vec<[u8; 32]>,
}

impl From<PendingCreate> for PendingRecord {
    fn from(p: PendingCreate) -> Self {
        Self {
            identity_id: p.identity_id,
            activity: p.activity,
            anchor: p.anchor,
            nullifiers: p.nullifiers,
        }
    }
}

impl From<&PendingRecord> for PendingCreate {
    fn from(p: &PendingRecord) -> Self {
        Self {
            identity_id: p.identity_id,
            activity: p.activity,
            anchor: p.anchor,
            nullifiers: p.nullifiers.clone(),
        }
    }
}

/// r1 Must 1: the wallets whose pending creation this run has seen reserved in the pool. Only a
/// reservation seen held and then gone, with the notes unspent, is platform-wallet's proof
/// that the transition can never execute; a restart drops reservations without one.
static SEEN_ARMED: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

fn seen_armed(wallet: &str) -> bool {
    SEEN_ARMED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(wallet)
}

fn set_seen_armed(wallet: &str, seen: bool) {
    let mut set = SEEN_ARMED.lock().unwrap_or_else(|e| e.into_inner());
    if seen {
        set.insert(wallet.to_owned());
    } else {
        set.remove(wallet);
    }
}

/// The record of a creation from the pool handed to the network, kept until its outcome is known.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::real) struct PoolIdentityRecord {
    /// The DIP-9 slot the identity takes; its keys and its refusal address are that slot's.
    pub slot: u32,
    pub exit: Duffs,
    /// The Platform address (bech32m) the money goes to if Platform refuses the creation.
    pub refusal: String,
    pub name: Option<String>,
    /// Unix seconds, when it was sent.
    pub sent_at: i64,
    /// The credits already on the refusal address before sending: a slot left empty by a
    /// refusal is taken again by the next creation, with the same refusal address.
    #[serde(default)]
    pub before: u64,
    /// Platform refused it: the credits seen on the refusal address. Once seen, final: an
    /// identity found in the slot later is another one.
    #[serde(default)]
    pub refused: Option<u64>,
    /// r1 Must 1: what the unproved send left armed in the pool, read right after it.
    #[serde(default)]
    pub pending: Option<PendingRecord>,
    /// r1 Must 1: proved never to execute (its anchor gone, its notes free again): the money is
    /// in the pool, and a new creation may start. Kept for the plate until then.
    #[serde(default)]
    pub not_delivered: bool,
}

/// What became of a creation from the pool, as this computer sees it now.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(in crate::real) enum Outcome {
    /// The identity is in its slot: the record goes; a missing name is finished as any new
    /// identity's.
    Made,
    /// Platform paid the money back to the refusal address: these credits.
    Refused(u64),
    /// Neither yet: not «failed» (BUG-243), nothing is sent again.
    Waiting,
    /// r1 Must 1: proved never to execute: the reservation of its notes, seen held in this run,
    /// was freed with the notes unspent — platform-wallet frees it so only once Platform has
    /// pruned the transition's anchor, after which it can never execute.
    NotDelivered,
}

/// `made`: an identity is in the record's slot. `refund`: the credits on its refusal address
/// now, when the address is known. `notes`: where the transition's notes stand in the pool
/// store, `seen_armed`: their reservation was seen held in this run.
pub(in crate::real) fn outcome(
    record: &PoolIdentityRecord,
    made: bool,
    refund: Option<u64>,
    notes: Option<CreateNotes>,
    seen_armed: bool,
) -> Outcome {
    if let Some(seen) = record.refused {
        // A sync that has not read the address again yet shows less: the last seen stays.
        return Outcome::Refused(refund.filter(|c| *c > record.before).unwrap_or(seen));
    }
    match refund {
        Some(credits) if credits > record.before => Outcome::Refused(credits),
        _ if made => Outcome::Made,
        _ if record.pending.is_some() && seen_armed && notes == Some(CreateNotes::Released) => {
            Outcome::NotDelivered
        }
        _ => Outcome::Waiting,
    }
}

fn shown(record: &PoolIdentityRecord) -> UnfinishedPoolIdentity {
    UnfinishedPoolIdentity {
        exit: record.exit,
        name: record.name.clone(),
        sent_at: record.sent_at,
        refusal_address: record.refusal.clone(),
        refused: record.refused.map(identity::credits_as_duffs),
        not_delivered: record.not_delivered,
    }
}

pub(in crate::real) fn read_record(path: &std::path::Path) -> Option<PoolIdentityRecord> {
    let bytes = evolite_local::json_file::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// r1 Must 2: the refusal address's credits before sending, from a proved read; a read that
/// failed stops the creation (never `0`, which would turn a remainder into a false refusal).
fn refusal_before(read: Result<u64, String>) -> Result<u64, CoreError> {
    read.map_err(read_failure)
}

fn write_record(path: &std::path::Path, record: &PoolIdentityRecord) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(record).map_err(|e| e.to_string())?;
    evolite_local::json_file::write(path, &bytes).map_err(|e| e.to_string())
}

/// Whether `exit` is one the network creates identities from now (FEAT-0048 R-2).
pub(in crate::real) fn exit_offered(fees: &Fees, exit: Duffs) -> bool {
    fees.pool_identity_exits.contains(&exit)
}

impl Real {
    /// r1 Should 2: whether an identity can be created from the pool now — the build has the
    /// pool, the keys are open, Platform is ready, the pool is connected for the open wallet and
    /// the network takes an exit. Otherwise (UC-6) the creation from Core is the fallback even
    /// without «Features for advanced users».
    pub(in crate::real) fn pool_creates_identities(&self, fees: &Fees) -> bool {
        let Some(open) = self.unlocked.as_ref() else {
            return false;
        };
        let connected = *self.pool_wallet.lock().unwrap_or_else(|e| e.into_inner());
        cfg!(feature = "pool")
            && open.keys.is_some()
            && self.chain.platform_ready()
            && connected == Some(open.id)
            && !fees.pool_identity_exits.is_empty()
    }

    /// A creation from the pool is waiting for its outcome: its slot may hold the identity
    /// Platform has not answered for, so no other creation takes a slot meanwhile.
    pub(in crate::real) fn pool_identity_waiting(&self) -> bool {
        read_record(&self.paths.pool_identity.clone())
            .is_some_and(|r| r.refused.is_none() && !r.not_delivered)
    }

    /// FEAT-0048 UC-3, UC-4: the creation from the pool whose outcome is not known yet, or which
    /// Platform refused; the record goes once the identity is in its slot. Nothing while one is
    /// being made: that job answers for itself.
    pub(in crate::real) async fn unfinished_pool_identity(
        &self,
        wallet: &platform_wallet::PlatformWallet,
    ) -> Option<UnfinishedPoolIdentity> {
        if self.identity_busy.is_busy() {
            return None;
        }
        let path = self.paths.pool_identity.clone();
        let mut record = read_record(&path)?;
        if record.not_delivered {
            return Some(shown(&record));
        }
        // r1 Must 1: the transition's notes. Held: platform-wallet judges them after each scan.
        // Free again but not seen held in this run (a restart dropped the reservation, proving
        // nothing): held again, to be judged the same way.
        let notes = match &record.pending {
            Some(pending) => {
                let pending = PendingCreate::from(pending);
                let notes = self.chain.pending_create_notes(wallet, &pending).await;
                match notes {
                    Some(CreateNotes::Armed) => set_seen_armed(&self.wallet, true),
                    Some(CreateNotes::Released) if !seen_armed(&self.wallet) => {
                        if let Err(e) = self.chain.rearm_pending_create(wallet, &pending).await {
                            crate::diag::platform_failed("pool_identity_rearm", &e);
                        }
                    }
                    _ => {}
                }
                notes
            }
            None => None,
        };
        let made = self
            .chain
            .identity_in_slot(wallet, record.slot)
            .await
            .is_some();
        let refund = self.chain.refusal_credits(wallet, record.slot).await;
        match outcome(&record, made, refund, notes, seen_armed(&self.wallet)) {
            Outcome::Made => {
                let _ = std::fs::remove_file(&path);
                set_seen_armed(&self.wallet, false);
                None
            }
            Outcome::NotDelivered => {
                record.not_delivered = true;
                if let Err(e) = write_record(&path, &record) {
                    crate::diag::platform_failed("pool_identity_record", &e);
                }
                set_seen_armed(&self.wallet, false);
                Some(shown(&record))
            }
            Outcome::Refused(credits) => {
                if record.refused != Some(credits) {
                    record.refused = Some(credits);
                    if let Err(e) = write_record(&path, &record) {
                        crate::diag::platform_failed("pool_identity_record", &e);
                    }
                }
                Some(shown(&record))
            }
            Outcome::Waiting => Some(shown(&record)),
        }
    }

    /// FEAT-0048 UC-4 «Shield again»: what Platform returned for a refused creation goes back
    /// into the pool; then the record goes, and with it the plate. Refused when nothing was
    /// returned, the wallet locked, or the pool not connected or busy.
    pub(in crate::real) fn shield_refund(&mut self) -> Result<(), CoreError> {
        let path = self.paths.pool_identity.clone();
        if read_record(&path).and_then(|r| r.refused).is_none() {
            return Err(CoreError::NotAvailable(
                tr!("Nothing was returned to put back into the pool").into(),
            ));
        }
        let (id, keys, _) = self.pool_move()?;
        let chain = self.chain.clone();
        self.pool_job(
            "shield-refund",
            async move {
                let wallet = loaded(&chain, &id).await?;
                chain
                    .shield_refund(&wallet, &keys)
                    .await
                    .map_err(super::pool::pool_error)?;
                let _ = std::fs::remove_file(&path);
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

    /// FEAT-0048 UC-1: creates an identity from the pool with `exit` and, when given, registers
    /// `name` from its credits. Refused before anything moves: an exit the network does not
    /// take, a locked wallet, the pool not connected or busy, an identity being made or one
    /// cut short, a creation from the pool whose outcome is not known yet. The answer comes
    /// later (`Done { what: Identity }`).
    pub(in crate::real) async fn create_identity_from_pool(
        &mut self,
        name: Option<&str>,
        exit: Duffs,
    ) -> Result<(), CoreError> {
        let label = identity::new_identity_label(name)?;
        let fees = self.fees_now(0).await;
        if !exit_offered(&fees, exit) {
            return Err(CoreError::NotAvailable(
                tr!("The network does not create an identity from the pool with this amount")
                    .into(),
            ));
        }
        let path = self.paths.pool_identity.clone();
        // r1 Must 1: one proved not delivered gives way to a new creation.
        if read_record(&path).is_some_and(|r| !r.not_delivered) {
            return Err(CoreError::NotAvailable(
                tr!("The identity from the pool is still being checked — wait for its outcome")
                    .into(),
            ));
        }
        // r1 Must 1: what a former creation's reservation showed says nothing of this one.
        set_seen_armed(&self.wallet, false);
        let needed = fees.name_credits_needed();
        let exit_credits = super::pool::positive_credits(exit)?;
        let (id, keys, seed) = self.pool_move()?;
        let release = self.pool_busy.clone();
        let refuse = |e: CoreError| {
            release.store(false, Ordering::Relaxed);
            Err(e)
        };
        let wallet = match loaded(&self.chain, &id).await {
            Ok(wallet) => wallet,
            Err(e) => return refuse(e),
        };
        if self.unfinished_identity(&wallet).await.is_some() {
            return refuse(CoreError::NotAvailable(
                tr!("First finish the stopped identity — its money is already locked").into(),
            ));
        }
        let Some(busy) = self.identity_busy.take() else {
            return refuse(CoreError::NotAvailable(
                tr!("An identity is already being made").into(),
            ));
        };
        let chain = self.chain.clone();
        let pending = self.pending_names();
        let unclear = self.unclear.clone();
        let pool_fees = self.pool_fees.clone();
        let pool_history = self.pool_history.clone();
        self.pool_job(
            "identity-from-pool",
            async move {
                let made = async {
                    if let Some(label) = label.clone() {
                        let asked = label.clone();
                        let available = super::names::within(&chain, |c| async move {
                            c.name_available(&asked).await
                        })
                        .await;
                        match available {
                            Ok(true) => {}
                            Ok(false) => {
                                return Err(CoreError::NotAvailable(format!(
                                    tr!(
                                        "The name @{label} is already taken, the money is untouched"
                                    ),
                                    label = label
                                )));
                            }
                            Err(e) => return Err(read_failure(e)),
                        }
                    }
                    let slot = chain
                        .next_identity_index(&wallet, &keys)
                        .await
                        .map_err(core_failure)?;
                    let refusal = chain
                        .refusal_address(&wallet, &keys, slot)
                        .await
                        .map_err(core_failure)?;
                    // r1 Must 2: what is on the refusal address now, proved, before anything
                    // is written. A refused creation leaves the slot free, so the next one takes
                    // the same slot and address; what «Shield again» could not take stays there,
                    // and only credits above it are this creation's refusal. Not read: nothing
                    // starts on a guess.
                    let before = refusal_before(chain.address_credits_proved(refusal).await)?;
                    // Kept before anything is sent: a creation cut short is looked for by its
                    // slot, never sent again (BUG-243); the name is finished as any new
                    // identity's (ResumeIdentity, `nameless_slot`).
                    identity::remember_name(&pending, slot, label.as_deref())
                        .map_err(core_failure)?;
                    write_record(
                        &path,
                        &PoolIdentityRecord {
                            slot,
                            exit,
                            refusal: refusal.to_bech32m_string(evolite_chain::network::dash()),
                            name: label.clone(),
                            sent_at: now(),
                            before,
                            refused: None,
                            pending: None,
                            not_delivered: false,
                        },
                    )
                    .map_err(core_failure)?;
                    let created = chain
                        .create_identity_from_pool(
                            &wallet,
                            super::super::seed_bytes(&seed),
                            &keys,
                            slot,
                            exit_credits,
                            refusal,
                        )
                        .await;
                    let identity_id = match created {
                        Ok((identity_id, credits)) => {
                            // BUG-319 p. 3: what it cost, on its line in History and in the
                            // price record; credits not read — not known, never 0.
                            pool_history.creation_cost(
                                &identity_id,
                                credits.map(|c| exit.saturating_sub(identity::credits_as_duffs(c))),
                                now(),
                            );
                            // r1 Should 1: the fee really taken, to tune the one shown.
                            if let Some(credits) = credits {
                                let fee = exit.saturating_sub(identity::credits_as_duffs(credits));
                                crate::diag::record(crate::diag::Record::IdentityFromPoolFee {
                                    exit,
                                    fee,
                                });
                                // BUG-319: measured, for the next fee shown.
                                if let Err(e) =
                                    pool_fees.record(fee, fees.pool_identity_create_floor)
                                {
                                    crate::diag::platform_failed("pool_fees", &e);
                                }
                            }
                            identity_id
                        }
                        // Not proved: the record stays, nothing is sent again. One search of
                        // the phrase's identities now: one made puts the outcome in the next
                        // snapshot; otherwise the next unlock's search (or «Find») finds it.
                        Err(e @ pool::PoolError::Unconfirmed) => {
                            // r1 Must 1: what it left armed, while platform-wallet holds it.
                            if let (Some(pending), Some(mut record)) = (
                                chain.pending_identity_create(&wallet).await,
                                read_record(&path),
                            ) {
                                record.pending = Some(pending.into());
                                if let Err(e) = write_record(&path, &record) {
                                    crate::diag::platform_failed("pool_identity_record", &e);
                                }
                            }
                            let _ = chain.discover_identities(&wallet, &keys).await;
                            return Err(super::pool::pool_error(e));
                        }
                        // Refused before it went: nothing moved, the record goes.
                        Err(e) => {
                            let _ = std::fs::remove_file(&path);
                            return Err(super::pool::pool_error(e));
                        }
                    };
                    let _ = std::fs::remove_file(&path);
                    if let Some(label) = label.as_deref()
                        && let Err(e) = chain
                            .register_new_identity_name(
                                &wallet,
                                &keys,
                                slot,
                                &identity_id,
                                label,
                                needed,
                            )
                            .await
                    {
                        // BUG-243 (4): the identity is made; the name may have gone out.
                        super::identity::name_not_seen(
                            &chain,
                            &wallet,
                            slot,
                            Some(label),
                            &e,
                            &unclear,
                        )
                        .await;
                        return Err(core_failure(e));
                    }
                    pending.remove(slot).map_err(core_failure)?;
                    Ok(slot)
                }
                .await;
                let result = identity::choose_created(id, made);
                drop(busy);
                result
            },
            |result| Event::Done {
                what: DoneWhat::Identity,
                result,
                ask: None,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fees(exits: &[Duffs]) -> Fees {
        let mut fees = evolite_chain::convert::fees(0);
        fees.pool_identity_exits = exits.to_vec();
        fees
    }

    /// R-2: only an exit the network takes; none at all where it takes none.
    #[test]
    fn only_the_networks_exits_are_taken() {
        let set = fees(&[3_000_000, 10_000_000, 25_000_000]);
        assert!(exit_offered(&set, 3_000_000));
        assert!(exit_offered(&set, 25_000_000));
        assert!(!exit_offered(&set, 3_000_001));
        assert!(!exit_offered(&set, 30_000_000));
        assert!(!exit_offered(&fees(&[]), 3_000_000));
    }

    /// The record round-trips and lives in the wallet's own folder.
    #[test]
    fn the_record_is_kept_in_the_wallets_folder() {
        let root = std::env::temp_dir().join(format!("evolite-pool-id-{}", std::process::id()));
        let path = Paths::new(root.clone(), "aaaa0001").pool_identity;
        assert!(path.starts_with(evolite_local::wallets::wallet_dir(&root, "aaaa0001")));
        assert_eq!(read_record(&path), None);
        let record = PoolIdentityRecord {
            slot: 3,
            exit: 3_000_000,
            refusal: "tdash1example".into(),
            name: Some("kestrel".into()),
            sent_at: 1_795_000_000,
            before: 0,
            refused: None,
            pending: None,
            not_delivered: false,
        };
        write_record(&path, &record).unwrap();
        assert_eq!(read_record(&path), Some(record));
        let _ = std::fs::remove_dir_all(root);
    }

    fn record(before: u64, refused: Option<u64>) -> PoolIdentityRecord {
        PoolIdentityRecord {
            slot: 2,
            exit: 3_000_000,
            refusal: "tdash1example".into(),
            name: None,
            sent_at: 1_795_000_000,
            before,
            refused,
            pending: None,
            not_delivered: false,
        }
    }

    /// R-9, BUG-243: not proved is not «failed»: with neither the identity nor the money back,
    /// it waits (and is never sent again); the identity in its slot ends it.
    #[test]
    fn an_unproved_creation_waits_until_its_slot_answers() {
        assert_eq!(
            outcome(&record(0, None), false, None, None, false),
            Outcome::Waiting
        );
        assert_eq!(
            outcome(&record(0, None), false, Some(0), None, false),
            Outcome::Waiting
        );
        assert_eq!(
            outcome(&record(0, None), true, Some(0), None, false),
            Outcome::Made
        );
        assert_eq!(
            outcome(&record(0, None), true, None, None, false),
            Outcome::Made
        );
    }

    /// UC-4: the money back on the refusal address is a refusal; what was there before (an
    /// earlier refusal in the same slot) is not; once seen, it stays, even if a sync reads
    /// less for a while or another identity takes the slot later.
    #[test]
    fn a_refusal_is_the_money_back_on_its_own_address() {
        assert_eq!(
            outcome(&record(0, None), false, Some(2_900_000_000), None, false),
            Outcome::Refused(2_900_000_000)
        );
        assert_eq!(
            outcome(&record(5_000, None), false, Some(5_000), None, false),
            Outcome::Waiting
        );
        assert_eq!(
            outcome(&record(5_000, None), false, Some(9_000), None, false),
            Outcome::Refused(9_000)
        );
        let seen = record(0, Some(2_900_000_000));
        assert_eq!(
            outcome(&seen, false, Some(0), None, false),
            Outcome::Refused(2_900_000_000)
        );
        assert_eq!(
            outcome(&seen, true, None, None, false),
            Outcome::Refused(2_900_000_000)
        );
    }

    /// An older record, written before `before` and `refused`, still reads.
    #[test]
    fn an_older_record_still_reads() {
        let old = br#"{"slot":1,"exit":3000000,"refusal":"tdash1x","name":null,"sent_at":5}"#;
        let read: PoolIdentityRecord = serde_json::from_slice(old).unwrap();
        assert_eq!((read.before, read.refused), (0, None));
    }

    fn sent(pending: bool) -> PoolIdentityRecord {
        PoolIdentityRecord {
            pending: pending.then(|| PendingRecord {
                identity_id: [3; 32],
                activity: [9; 32],
                anchor: [7; 32],
                nullifiers: vec![[21; 32]],
            }),
            ..record(0, None)
        }
    }

    /// r1 Must 1, «lost»: the reservation seen held in this run, then freed with the notes
    /// unspent (platform-wallet frees it so only once the anchor is pruned): not delivered.
    #[test]
    fn a_creation_that_never_reached_the_network_is_proved_not_delivered() {
        let released = Some(CreateNotes::Released);
        assert_eq!(
            outcome(&sent(true), false, Some(0), released, true),
            Outcome::NotDelivered
        );
    }

    /// r1 Must 1: everything short of that proof waits — the notes spent and no identity yet
    /// (it landed, the identity follows), still held, freed by a restart (not seen held in this
    /// run), not in the store, or no reservation was read at the send.
    #[test]
    fn anything_short_of_the_proof_waits() {
        let wait =
            |record: &PoolIdentityRecord, notes, seen| outcome(record, false, Some(0), notes, seen);
        assert_eq!(
            wait(&sent(true), Some(CreateNotes::Spent), true),
            Outcome::Waiting
        );
        assert_eq!(
            wait(&sent(true), Some(CreateNotes::Armed), true),
            Outcome::Waiting
        );
        assert_eq!(
            wait(&sent(true), Some(CreateNotes::Released), false),
            Outcome::Waiting
        );
        assert_eq!(
            wait(&sent(true), Some(CreateNotes::Unknown), true),
            Outcome::Waiting
        );
        assert_eq!(wait(&sent(true), None, true), Outcome::Waiting);
        assert_eq!(
            wait(&sent(false), Some(CreateNotes::Released), true),
            Outcome::Waiting
        );
    }

    /// r1 Must 1: the identity found, or the money back, still decides first.
    #[test]
    fn the_identity_or_a_refusal_comes_before_not_delivered() {
        let released = Some(CreateNotes::Released);
        assert_eq!(
            outcome(&sent(true), true, Some(0), released, true),
            Outcome::Made
        );
        assert_eq!(
            outcome(&sent(true), false, Some(10), released, true),
            Outcome::Refused(10)
        );
    }

    /// r1 Must 2: a refusal address that could not be read stops the creation; it is never
    /// taken as empty.
    #[test]
    fn an_unread_refusal_address_starts_nothing() {
        assert!(refusal_before(Err("timeout".into())).is_err());
        assert_eq!(refusal_before(Ok(12_345)).unwrap(), 12_345);
    }

    /// r1 Must 1: a record from before r1 reads, with nothing pending and not «not delivered».
    #[test]
    fn a_record_from_before_r1_reads() {
        let old = br#"{"slot":1,"exit":3000000,"refusal":"tdash1x","name":null,"sent_at":5,"before":0,"refused":null}"#;
        let read: PoolIdentityRecord = serde_json::from_slice(old).unwrap();
        assert_eq!((read.pending, read.not_delivered), (None, false));
    }
}
