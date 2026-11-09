//! Identities on the chain: finding them, making them, topping them up and naming them, and resuming what a restart cut short.

use super::*;

/// The identity slot each open wallet works with, as chosen on the Identity screen.
/// Kept for the process (not in a file yet): many places pick "the identity" knowing only the
/// wallet, and they all read it here.
static CHOSEN: std::sync::Mutex<BTreeMap<WalletId, u32>> = std::sync::Mutex::new(BTreeMap::new());

/// From now on the wallet acts as its identity of slot `slot`.
pub fn choose_identity(wallet: WalletId, slot: u32) {
    if let Ok(mut chosen) = CHOSEN.lock() {
        chosen.insert(wallet, slot);
    }
}

/// Which of the wallet's identity `slots` it works with: the chosen one while the wallet has
/// it, else the lowest.
pub fn shown_slot(wallet: WalletId, slots: impl Iterator<Item = u32>) -> Option<u32> {
    let slots: Vec<u32> = slots.collect();
    let chosen = CHOSEN.lock().ok().and_then(|c| c.get(&wallet).copied());
    chosen
        .filter(|c| slots.contains(c))
        .or_else(|| slots.into_iter().min())
}

/// The slot a new identity takes (FEAT-0014 R-3): one past the highest of the phrase's
/// identities `taken` and of a lock still waiting for its identity (`locked`, which holds its
/// slot), 0 for the first. Gaps are not filled: a slot left empty below the highest (say, by an
/// identity made in another app past it) stays empty, so discovery, which stops after 5 empty
/// slots in a row, still finds every identity.
pub fn next_slot(taken: impl IntoIterator<Item = u32>, locked: Option<u32>) -> u32 {
    taken.into_iter().max().max(locked).map_or(0, |i| i + 1)
}

/// The identities of `wallet` among all those platform-wallet keeps.
pub(super) fn of_wallet(
    identities: &platform_wallet::IdentityManager,
    wallet: WalletId,
) -> impl Iterator<Item = &platform_wallet::ManagedIdentity> {
    identities
        .managed_identities()
        .filter(move |m| m.wallet_id == Some(wallet))
}

/// The identity of `wallet` in phrase slot `slot`, if the store has it.
fn in_slot(
    identities: &platform_wallet::IdentityManager,
    wallet: WalletId,
    slot: u32,
) -> Option<&platform_wallet::ManagedIdentity> {
    of_wallet(identities, wallet).find(|m| m.identity_index == Some(slot))
}

/// Of the wallet's identities `(slot, id, item)`, identity `id` with its slot — whichever one is
/// shown now.
pub(super) fn with_id<T>(
    mut mine: impl Iterator<Item = (u32, Identifier, T)>,
    id: &Identifier,
) -> Option<(u32, T)> {
    mine.find(|(_, found, _)| found == id)
        .map(|(slot, _, item)| (slot, item))
}

/// Of the wallet's identities `(slot, item)`, the one it works as ([`shown_slot`]), with its
/// slot.
pub(super) fn shown_of<T>(
    wallet: WalletId,
    mine: impl Iterator<Item = (u32, T)>,
) -> Option<(u32, T)> {
    let mine: Vec<(u32, T)> = mine.collect();
    let slot = shown_slot(wallet, mine.iter().map(|(i, _)| *i))?;
    mine.into_iter().find(|(i, _)| *i == slot)
}

/// Core funds locked for an identity that Platform has not made yet.
#[derive(Clone, Copy, Debug)]
pub struct Unfinished {
    pub out_point: key_wallet::dashcore::OutPoint,
    pub identity_index: u32,
    /// Duffs.
    pub amount: u64,
    /// For a top-up: Platform already said it took the lock; only its ChainLock is awaited to
    /// confirm that. Not to be sent as new, and it does not hold back a new top-up.
    pub awaiting_chain_lock: bool,
}

/// What Platform answered about a top-up lock, beyond what the store keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopUpNote {
    /// The identity's credits grew: the lock is done with.
    Credited,
    /// "Already taken", not provable before the lock's block has a ChainLock.
    AwaitingChainLock,
}

/// How a tracked asset lock counts as a cut-short top-up: `None` when it does not; `Some(true)`
/// when it only awaits its ChainLock ([`Unfinished::awaiting_chain_lock`]).
pub fn top_up_state(
    funding: AssetLockFundingType,
    status: &AssetLockStatus,
    note: Option<TopUpNote>,
) -> Option<bool> {
    if funding != AssetLockFundingType::IdentityTopUp
        || status.is_consumed()
        // Found on chain, not made here; or "already taken" with its ChainLock proof kept
        // (consumption unknown): nothing is left to send.
        || *status == AssetLockStatus::RecoveredFromChain
    {
        return None;
    }
    match note {
        Some(TopUpNote::Credited) => None,
        Some(TopUpNote::AwaitingChainLock) => Some(true),
        None => Some(false),
    }
}

/// The lock this top-up attempt made, for when Platform's answer names none (BUG-106): of the
/// wallet's tracked locks `(out point, funding, status, slot)`, the one top-up lock of the
/// identity's `slot` that was not among those `known` before the attempt. No such lock, or more
/// than one, is `None`: another lock is someone's stopped top-up and is never sent in its place.
pub(super) fn lock_of_this_attempt(
    known: &HashSet<key_wallet::dashcore::OutPoint>,
    tracked: &[(
        key_wallet::dashcore::OutPoint,
        AssetLockFundingType,
        AssetLockStatus,
        u32,
    )],
    slot: Option<u32>,
) -> Option<key_wallet::dashcore::OutPoint> {
    let slot = slot?;
    let mut made = tracked
        .iter()
        .filter(|(out_point, funding, status, index)| {
            !known.contains(out_point)
                && *index == slot
                && top_up_state(*funding, status, None).is_some()
        })
        .map(|(out_point, ..)| *out_point);
    let first = made.next()?;
    made.next().is_none().then_some(first)
}

/// What "the lock is already taken" means for the person, from the identity's credits before
/// and after (proved) and the lock's status after the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlreadyTaken {
    /// The credits grew: the top-up went through, only its answer was lost.
    Credited(u64),
    /// platform-wallet proved the lock's finality by ChainLock and keeps it as "taken,
    /// consumption unknown": nothing is left to finish, though the credits did not grow on this
    /// attempt ([`top_up_settled_text`]).
    Settled,
    /// No ChainLock for the lock's block yet: wait and check again.
    AwaitChainLock,
}

pub fn already_taken(
    before: Option<u64>,
    after: Option<u64>,
    status: Option<AssetLockStatus>,
) -> AlreadyTaken {
    if let Some(after) = after
        && after > before.unwrap_or(0)
    {
        return AlreadyTaken::Credited(after);
    }
    match status {
        Some(s) if s == AssetLockStatus::RecoveredFromChain || s.is_consumed() => {
            AlreadyTaken::Settled
        }
        _ => AlreadyTaken::AwaitChainLock,
    }
}

/// Whose DPNS names a discovery reads itself (BUG-9), of the wallet's identities `(id, named)`:
/// platform-wallet read those of the `new` ones only, and a failed read left them nameless.
/// Those already read in this search (`read`) are not read again on its next pass.
pub(super) fn names_to_read(
    known: &[(Identifier, bool)],
    new: &[Identifier],
    read: &HashSet<Identifier>,
) -> Vec<Identifier> {
    known
        .iter()
        .filter(|(id, named)| (!new.contains(id) || !named) && !read.contains(id))
        .map(|(id, _)| *id)
        .collect()
}

/// What one pass of [`Chain::discover_identities`] brought (BUG-9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// Identities new to the store.
    pub found: usize,
    /// Every slot answered and every name read. Otherwise more identities or names may come,
    /// and the caller looks again.
    pub complete: bool,
    /// Every slot answered, names aside: no identity of the phrase is left unknown. Enough to
    /// choose a new slot or use a lock (BUG-9 r2 should-1); names are read on the next pass.
    pub slots_complete: bool,
    /// Why it is not complete, for "Could not ask Platform … ({})". `None` when complete.
    pub missing: Option<String>,
}

/// A pass is complete only with every slot answered and every name read (BUG-9). With both
/// missing, the unanswered slots are named: they may hide whole identities, names included.
pub(super) fn discovery_answer(found: usize, incomplete: bool, names_failed: bool) -> Discovery {
    let missing = if incomplete {
        Some(tr!("Platform did not answer for some of the phrase's identities").into())
    } else if names_failed {
        Some(tr!("the names of an identity did not load").into())
    } else {
        None
    };
    Discovery {
        found,
        complete: missing.is_none(),
        slots_complete: !incomplete,
        missing,
    }
}

/// The step of an identity's creation its lock is at, as "What happens" numbers them: 1 the
/// asset lock is being made (no lock yet, or built), 2 it is sent and waits for its proof, 3 it
/// is proved (InstantLock or ChainLock) and Platform makes the identity, 4 Platform took it and
/// the name is registered.
pub fn creation_step_of(status: Option<&AssetLockStatus>) -> usize {
    match status {
        None | Some(AssetLockStatus::Built) => 1,
        Some(AssetLockStatus::Broadcast) => 2,
        Some(AssetLockStatus::Consumed) => 4,
        Some(_) => 3,
    }
}

impl Chain {
    /// Looks on Platform for the identities of the phrase (slots from 0 until a gap) with their
    /// DPNS names; platform-wallet keeps what it finds in the store. Needs [`Self::platform_ready`].
    ///
    /// BUG-9: what it found even with slots or names missing, and whether it is
    /// [`Discovery::complete`]; an `Err` only when the scan itself failed. platform-wallet
    /// (575d961, `identity/network/discovery.rs`) answers `Ok` for a scan that left slots
    /// unanswered once it saw one identity ("the caller re-scans later"), and reads the names of
    /// new identities only, a failed read logged and dropped: the names of identities the store
    /// already had, or whose read failed, are read here.
    pub async fn discover_identities(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<Discovery, String> {
        self.discover_identities_again(wallet, keys, &mut HashSet::new())
            .await
    }

    /// [`Self::discover_identities`] as the next pass of one search: the names in `names_read`
    /// were read on an earlier pass and are not read again; those read now are added.
    pub async fn discover_identities_again(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        names_read: &mut HashSet<Identifier>,
    ) -> Result<Discovery, String> {
        let master = keys
            .derive_extended_private_key(&DerivationPath::master())
            .map_err(|e| e.to_string())?;
        let found = wallet
            .identity()
            .discover_from_master(
                IdentityDiscoveryOptions {
                    start_index: Some(0),
                    ..Default::default()
                },
                &master,
            )
            .await
            .map_err(|e| e.to_string())?;
        let (incomplete, known) = {
            let state = wallet.state().await;
            let manager = &state.identity_manager;
            let known: Vec<(Identifier, bool)> = of_wallet(manager, wallet.wallet_id())
                .map(|m| {
                    let named = !m.dpns_names.is_empty() || !m.contested_dpns_names.is_empty();
                    (m.identity.id(), named)
                })
                .collect();
            (
                manager.identity_scan_is_incomplete(&wallet.wallet_id()),
                known,
            )
        };
        let new: Vec<Identifier> = found.iter().map(|i| i.id()).collect();
        // platform-wallet named the new ones it could: those are read too.
        names_read.extend(
            known
                .iter()
                .filter(|(id, named)| *named && new.contains(id))
                .map(|(id, _)| *id),
        );
        let mut names_failed = false;
        for id in names_to_read(&known, &new, names_read) {
            match wallet.identity().sync_dpns_names(&id).await {
                Ok(_) => {
                    names_read.insert(id);
                }
                Err(_) => names_failed = true,
            }
        }
        Ok(discovery_answer(found.len(), incomplete, names_failed))
    }

    /// [`Self::discover_identities`] that is an `Err` unless every slot answered: before a slot
    /// is chosen or a lock is used, an identity may sit in an unanswered slot. A name that did
    /// not load does not stop either (BUG-9 r2 should-1): it changes neither.
    async fn discover_every_identity(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<usize, String> {
        let pass = self.discover_identities(wallet, keys).await?;
        if pass.slots_complete {
            Ok(pass.found)
        } else {
            Err(pass.missing.unwrap_or_default())
        }
    }

    /// The slot a new identity takes: one past the highest of the phrase's identities. They are
    /// looked up first, so a slot taken by an identity the store does not know yet is not reused.
    pub async fn next_identity_index(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<u32, String> {
        self.discover_every_identity(wallet, keys).await?;
        let state = wallet.state().await;
        let taken: Vec<u32> = of_wallet(&state.identity_manager, wallet.wallet_id())
            .filter_map(|m| m.identity_index)
            .collect();
        drop(state);
        let locked = self.unfinished(wallet).await.map(|u| u.identity_index);
        Ok(next_slot(taken, locked))
    }

    /// Creates an identity in slot `index` from `amount` duffs of Core funds (an asset lock,
    /// proved by InstantSend or else ChainLock) and registers `label` for it in DPNS; `None`:
    /// no name, the step is skipped (FEAT-0014 R-7) and the answer is `Ok`.
    /// platform-wallet tracks the asset lock in the store until Platform takes it, so a run cut
    /// short is finished by [`Self::resume_identity`]. An identity made without the name it
    /// was to get is an `Err` that says so.
    pub async fn create_identity(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        index: u32,
        label: Option<&str>,
        amount: u64,
        needed: evolite_api::Duffs,
    ) -> Result<String, String> {
        let id = self
            .register_identity(
                wallet,
                keys,
                index,
                AssetLockFunding::FromWalletBalance {
                    amount_duffs: amount,
                    account_index: 0,
                },
            )
            .await?;
        if let Some(label) = label {
            self.register_name(wallet, keys, index, &id, label, (needed, true))
                .await?;
        }
        Ok(id.to_string(Encoding::Base58))
    }

    /// Which of the four steps of an identity's creation slot `index` is on, from the lock
    /// platform-wallet tracks for it: see [`creation_step_of`].
    pub async fn creation_step(&self, wallet: &PlatformWallet, index: u32) -> usize {
        let status = wallet
            .asset_locks()
            .list_tracked_locks()
            .await
            .into_iter()
            .find(|l| {
                l.funding_type == AssetLockFundingType::IdentityRegistration
                    && l.identity_index == index
            })
            .map(|l| l.status);
        creation_step_of(status.as_ref())
    }

    /// An identity registration that did not finish: Core funds locked for slot
    /// `identity_index`, and no identity there yet that the wallet knows of.
    pub async fn unfinished(&self, wallet: &PlatformWallet) -> Option<Unfinished> {
        let have: Vec<u32> = {
            let state = wallet.state().await;
            of_wallet(&state.identity_manager, wallet.wallet_id())
                .filter_map(|m| m.identity_index)
                .collect()
        };
        wallet
            .asset_locks()
            .list_tracked_locks()
            .await
            .into_iter()
            .filter(|l| {
                l.funding_type == AssetLockFundingType::IdentityRegistration
                    && !l.status.is_consumed()
                    // Found on chain by a restore, not made here: platform-wallet cannot tell
                    // whether Platform took it, and puts every such lock in slot 0. The
                    // identities of a restored phrase come from discovery instead.
                    && l.status != AssetLockStatus::RecoveredFromChain
                    && !have.contains(&l.identity_index)
            })
            .min_by_key(|l| l.identity_index)
            .map(|l| Unfinished {
                out_point: l.out_point,
                identity_index: l.identity_index,
                amount: l.amount,
                awaiting_chain_lock: false,
            })
    }

    /// The identity in slot `index` when it has no DPNS name yet: an identity made, its name
    /// not written.
    pub async fn nameless(&self, wallet: &PlatformWallet, index: u32) -> Option<Identifier> {
        let state = wallet.state().await;
        in_slot(&state.identity_manager, wallet.wallet_id(), index)
            .filter(|m| m.dpns_names.is_empty() && m.contested_dpns_names.is_empty())
            .map(|m| m.identity.id())
    }

    /// Finishes what a cut-short [`Self::create_identity`] left: the identity from the tracked
    /// lock (from whatever stage it reached: built, broadcast, InstantSend or ChainLock), then
    /// `label` when the identity has no name. An identity Platform already made from that lock
    /// is found, not made twice.
    pub async fn resume_identity(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        index: u32,
        label: Option<&str>,
        needed: evolite_api::Duffs,
    ) -> Result<String, String> {
        // Platform may have taken the lock before the run was cut: look first.
        self.discover_every_identity(wallet, keys).await?;
        // Made here and now, so the store's balance is Platform's proved answer; an identity
        // from an earlier attempt can have spent since: its credits are read or unknown.
        let (id, just_made) = match self.unfinished(wallet).await {
            Some(lock) if lock.identity_index == index => (
                self.register_identity(
                    wallet,
                    keys,
                    index,
                    AssetLockFunding::FromExistingAssetLock {
                        out_point: lock.out_point,
                        consume_invitation_voucher: false,
                    },
                )
                .await?,
                true,
            ),
            _ => {
                let state = wallet.state().await;
                let id = in_slot(&state.identity_manager, wallet.wallet_id(), index)
                    .map(|m| m.identity.id())
                    .ok_or(tr!("There is no unfinished identity any more"))?;
                (id, false)
            }
        };
        if let Some(label) = label
            && self.nameless(wallet, index).await.is_some()
        {
            self.register_name(wallet, keys, index, &id, label, (needed, just_made))
                .await?;
        }
        Ok(id.to_string(Encoding::Base58))
    }

    /// Reads the credit balance of each of the wallet's identities from Platform (proved) and
    /// keeps it in the store; a snapshot older than the one kept is ignored by platform-wallet.
    /// The store's balance is only what the last run of the app saw, and other apps (or other
    /// runs of this one) spend and top up the same identities. `all`: every identity, else only
    /// the one the interface shows (one query, not one per identity). Answers how many
    /// were read.
    pub async fn refresh_balances(
        &self,
        wallet: &PlatformWallet,
        all: bool,
    ) -> Result<usize, String> {
        let ids: Vec<Identifier> = {
            let state = wallet.state().await;
            let mine: Vec<_> = of_wallet(&state.identity_manager, wallet.wallet_id()).collect();
            let shown = shown_slot(
                wallet.wallet_id(),
                mine.iter().filter_map(|m| m.identity_index),
            );
            mine.into_iter()
                .filter(|m| all || m.identity_index == shown)
                .map(|m| m.identity.id())
                .collect()
        };
        let mut read = 0;
        let mut failed = None;
        for id in &ids {
            match wallet.identity().refresh_identity_balance(id).await {
                Ok(_) => read += 1,
                Err(e) => failed = Some(e.to_string()),
            }
        }
        match failed {
            Some(why) if read == 0 && !ids.is_empty() => Err(why),
            _ => Ok(read),
        }
    }

    /// Registers `label` in DPNS as one more name of the wallet's first identity, signed with
    /// the keys the identity has (made here or elsewhere) and paid from its credits.
    pub async fn add_name(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        label: &str,
        needed: evolite_api::Duffs,
    ) -> Result<(), String> {
        let (identity, signer) = Self::first_identity(wallet, keys).await?;
        let id = identity.id();
        self.write_name(wallet, &id, label, &signer, needed, false)
            .await
    }

    /// The one way a DPNS name reaches Platform (BUG-97): the credits are read and judged first,
    /// and a name they cannot finish is not sent, so nothing is charged. `trust_store`: the
    /// identity was just made, its balance in the store is the proved result of that, and a
    /// node that lags cannot refuse it; anywhere else a failed read means credits unknown.
    async fn write_name(
        &self,
        wallet: &PlatformWallet,
        id: &Identifier,
        label: &str,
        signer: &SimpleSigner,
        needed: evolite_api::Duffs,
        trust_store: bool,
    ) -> Result<(), String> {
        let fresh = wallet.identity().refresh_identity_balance(id).await.ok();
        let credits = match fresh {
            Some(credits) => Some(credits),
            None if trust_store => self.identity_credits(wallet, id, false).await,
            None => None,
        };
        gated_write(credits, needed, async {
            wallet
                .identity()
                .register_name_with_external_signer(id, label, None, signer)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .await?;
        // The name in the store, so the interface shows it at once.
        let _ = wallet.identity().sync_dpns_names(id).await;
        Ok(())
    }

    /// Tops up the wallet's first identity (the one the interface shows) with `amount` duffs
    /// of Core funds: an asset lock, proved by InstantSend or else ChainLock. platform-wallet
    /// tracks the lock until Platform takes it, so a run cut short is finished by
    /// [`Self::resume_top_up`]. Answers the identity's balance in credits.
    pub async fn top_up_identity(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        amount: u64,
    ) -> Result<u64, String> {
        let (identity, _) = Self::first_identity(wallet, keys).await?;
        self.top_up(
            wallet,
            keys,
            &identity.id(),
            AssetLockFunding::FromWalletBalance {
                amount_duffs: amount,
                account_index: 0,
            },
        )
        .await
    }

    /// A top-up that did not finish: Core funds locked for an identity's credits that Platform
    /// has not taken yet, or has taken without a proof yet ([`Unfinished::awaiting_chain_lock`]).
    /// A lock that can still be sent comes first.
    pub async fn unfinished_top_up(&self, wallet: &PlatformWallet) -> Option<Unfinished> {
        self.unfinished_top_ups(wallet)
            .await
            .into_iter()
            .min_by_key(|l| (l.awaiting_chain_lock, l.identity_index))
    }

    /// Every top-up [`Self::unfinished_top_up`] could name, in no order: the History marks
    /// each one's line (BUG-57).
    pub async fn unfinished_top_ups(&self, wallet: &PlatformWallet) -> Vec<Unfinished> {
        let notes = self
            .top_up_notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        wallet
            .asset_locks()
            .list_tracked_locks()
            .await
            .into_iter()
            .filter_map(|l| {
                let awaiting =
                    top_up_state(l.funding_type, &l.status, notes.get(&l.out_point).copied())?;
                Some(Unfinished {
                    out_point: l.out_point,
                    identity_index: l.identity_index,
                    amount: l.amount,
                    awaiting_chain_lock: awaiting,
                })
            })
            .collect()
    }

    /// Finishes what a cut-short [`Self::top_up_identity`] left: the tracked lock goes to
    /// Platform from whatever stage it reached, for the identity of its slot. No new lock.
    /// For a lock Platform already took, this asks again for its ChainLock proof.
    pub async fn resume_top_up(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<u64, String> {
        let lock = self
            .unfinished_top_up(wallet)
            .await
            .ok_or(tr!("There is no stopped top-up any more"))?;
        let id = {
            let state = wallet.state().await;
            in_slot(
                &state.identity_manager,
                wallet.wallet_id(),
                lock.identity_index,
            )
            .map(|m| m.identity.id())
            .ok_or(top_up_identity_unknown_text())?
        };
        self.top_up(
            wallet,
            keys,
            &id,
            AssetLockFunding::FromExistingAssetLock {
                out_point: lock.out_point,
                consume_invitation_voucher: false,
            },
        )
        .await
    }

    /// The credits of the identity `id` as the store has them.
    async fn stored_credits(wallet: &PlatformWallet, id: &Identifier) -> Option<u64> {
        let state = wallet.state().await;
        of_wallet(&state.identity_manager, wallet.wallet_id())
            .find(|m| m.identity.id() == *id)
            .map(|m| m.identity.balance())
    }

    /// The lock `funding` went to Platform with: the one asked for, or the one this attempt made
    /// ([`lock_of_this_attempt`], against the locks `known` before it).
    async fn lock_sent(
        wallet: &PlatformWallet,
        id: &Identifier,
        funding: &AssetLockFunding,
        known: &HashSet<key_wallet::dashcore::OutPoint>,
    ) -> Option<key_wallet::dashcore::OutPoint> {
        if let AssetLockFunding::FromExistingAssetLock { out_point, .. } = funding {
            return Some(*out_point);
        }
        let slot = {
            let state = wallet.state().await;
            of_wallet(&state.identity_manager, wallet.wallet_id())
                .find(|m| m.identity.id() == *id)
                .and_then(|m| m.identity_index)
        };
        let tracked: Vec<_> = wallet
            .asset_locks()
            .list_tracked_locks()
            .await
            .into_iter()
            .map(|l| (l.out_point, l.funding_type, l.status, l.identity_index))
            .collect();
        lock_of_this_attempt(known, &tracked, slot)
    }

    /// Tops up the identity `id` from `funding`; the asset lock is signed with the phrase's keys.
    ///
    /// "The lock is already taken" (an earlier attempt reached Platform, its answer was lost)
    /// is not a failure to repeat: the identity is read again with a proof, and grown credits
    /// answer `Ok`. Without a ChainLock for the lock's block that answer cannot be proved yet;
    /// the lock is then noted as awaiting it ([`TopUpNote`]) and the answer says so.
    ///
    /// "This transition is already in the cache" (BUG-7, seen on testnet): the same lock sent
    /// again makes the very same transition, byte for byte, and the network's cache of seen
    /// transitions answers that without a verdict — on every "Continue". The lock is then sent
    /// once more with a raised `user_fee_increase` (+1 % of the processing fee), a new
    /// transition Platform judges: credited, or "already taken" above.
    ///
    /// "Already taken" once the lock's block has a ChainLock reaches here as platform-wallet's
    /// failure to record it ([`is_taken_with_chain_lock`]): that is settled, not an error, and
    /// answers [`top_up_settled_text`] when the credits did not grow on this attempt.
    async fn top_up(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        id: &Identifier,
        funding: AssetLockFunding,
    ) -> Result<u64, String> {
        let before = Self::stored_credits(wallet, id).await;
        let known: HashSet<_> = wallet
            .asset_locks()
            .list_tracked_locks()
            .await
            .into_iter()
            .map(|l| l.out_point)
            .collect();
        let signer = identity::AssetLockSigner(keys.clone());
        let mut funding = funding;
        let mut fee_increase: u16 = 0;
        let error = loop {
            let settings = (fee_increase > 0).then(|| PutSettings {
                user_fee_increase: Some(fee_increase),
                ..Default::default()
            });
            let error = match wallet
                .identity()
                .top_up_identity_with_funding(id, funding.clone(), &signer, settings)
                .await
            {
                Ok(balance) => return Ok(balance),
                Err(e) => e,
            };
            crate::diag::platform_failed("top_up", &format!("{error:?}"));
            if !is_cached_duplicate(&error) || fee_increase >= SAME_TRANSITION_RETRIES {
                break error;
            }
            // The lock the cached transition spent; if it cannot be named, nothing is sent.
            let Some(out_point) = Self::lock_sent(wallet, id, &funding, &known).await else {
                break error;
            };
            funding = AssetLockFunding::FromExistingAssetLock {
                out_point,
                consume_invitation_voucher: false,
            };
            fee_increase += 1;
        };
        let (out_point, chain_locked) = match error {
            platform_wallet::PlatformWalletError::AssetLockAlreadyConsumed(out_point) => {
                (out_point, false)
            }
            ref e if is_taken_with_chain_lock(e) => {
                let Some(out_point) = Self::lock_sent(wallet, id, &funding, &known).await else {
                    return Err(error.to_string());
                };
                (out_point, true)
            }
            _ => return Err(error.to_string()),
        };
        let _ = wallet.identity().refresh_identity(id).await;
        let after = Self::stored_credits(wallet, id).await;
        // Taken with a ChainLock proof: as settled as platform-wallet's own "recovered from
        // chain", which its store could not record.
        let status = if chain_locked {
            Some(AssetLockStatus::RecoveredFromChain)
        } else {
            wallet
                .asset_locks()
                .list_tracked_locks()
                .await
                .into_iter()
                .find(|l| l.out_point == out_point)
                .map(|l| l.status)
        };
        let note = |n| {
            self.top_up_notes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(out_point, n);
        };
        match already_taken(before, after, status) {
            AlreadyTaken::Credited(balance) => {
                note(TopUpNote::Credited);
                Ok(balance)
            }
            AlreadyTaken::Settled => {
                // The store still has the lock as not taken: not shown again until a restart.
                // Done, but not by this attempt: the answer says so instead of "Credits added".
                note(TopUpNote::Credited);
                Err(top_up_settled_text().into())
            }
            AlreadyTaken::AwaitChainLock => {
                note(TopUpNote::AwaitingChainLock);
                Err(awaiting_chain_lock_text().into())
            }
        }
    }

    /// Registers the identity of slot `index` with its six keys, funded by `funding`.
    async fn register_identity(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        index: u32,
        funding: AssetLockFunding,
    ) -> Result<Identifier, String> {
        let new_keys = identity::new_identity_keys(keys, index)?;
        let mut signer = SimpleSigner::default();
        signer.add_identity_public_keys(new_keys.iter().cloned());
        let public: BTreeMap<u32, IdentityPublicKey> =
            new_keys.into_iter().map(|(k, _)| (k.id(), k)).collect();
        let created = wallet
            .identity()
            .register_identity_with_funding(
                funding,
                index,
                public,
                &signer,
                &identity::AssetLockSigner(keys.clone()),
                None,
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(created.id())
    }

    /// Registers `label` in DPNS for the identity `id` of slot `index`.
    async fn register_name(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        index: u32,
        id: &Identifier,
        label: &str,
        (needed, trust_store): (evolite_api::Duffs, bool),
    ) -> Result<(), String> {
        let mut signer = SimpleSigner::default();
        signer.add_identity_public_keys(identity::new_identity_keys(keys, index)?);
        self.write_name(wallet, id, label, &signer, needed, trust_store)
            .await
            .map_err(|e| {
                format!(
                    tr!("the identity is made, but the name @{label} was not written: {e}"),
                    label = label,
                    e = e
                )
            })
    }

    /// The identity the interface shows (the chosen one, else the lowest slot: see
    /// [`shown_slot`]), with a signer of its keys derived from `keys`. Every write that acts
    /// "as the identity" goes through here, so they all follow the choice.
    pub async fn first_identity(
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<(dpp::identity::Identity, SimpleSigner), String> {
        let (index, identity) = {
            let state = wallet.state().await;
            let mine = of_wallet(&state.identity_manager, wallet.wallet_id())
                .filter_map(|m| Some((m.identity_index?, m.identity.clone())));
            shown_of(wallet.wallet_id(), mine).ok_or(tr!("The wallet has no identity"))?
        };
        let mut signer = SimpleSigner::default();
        signer.add_identity_public_keys(identity::found_identity_keys(keys, index, &identity));
        Ok((identity, signer))
    }

    /// The wallet's identity `id` with a signer of its keys: a write checked as one identity is
    /// signed as that one, even when "Act as" moved on in between (BUG-13 r1 should 1).
    pub async fn identity_as(
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        id: &Identifier,
    ) -> Result<(dpp::identity::Identity, SimpleSigner), String> {
        let (index, identity) = {
            let state = wallet.state().await;
            let mine = of_wallet(&state.identity_manager, wallet.wallet_id())
                .filter_map(|m| Some((m.identity_index?, m.identity.id(), m.identity.clone())));
            with_id(mine, id).ok_or(tr!("The wallet has no identity"))?
        };
        let mut signer = SimpleSigner::default();
        signer.add_identity_public_keys(identity::found_identity_keys(keys, index, &identity));
        Ok((identity, signer))
    }

    /// The credits of the wallet's identity `id`, as [`Self::first_identity_credits`] reads the
    /// shown one's.
    pub async fn identity_credits(
        &self,
        wallet: &PlatformWallet,
        id: &Identifier,
        fresh: bool,
    ) -> Option<u64> {
        if fresh {
            let _ = wallet.identity().refresh_identity(id).await;
        }
        let state = wallet.state().await;
        let mine = of_wallet(&state.identity_manager, wallet.wallet_id())
            .filter_map(|m| Some((m.identity_index?, m.identity.id(), m.identity.balance())));
        with_id(mine, id).map(|(_, balance)| balance)
    }

    /// The shown identity's credits (the one [`Self::first_identity`] signs as): as the store
    /// has them, or with `fresh` read from Platform first (after a write, to see what it cost).
    /// `None` without an identity.
    pub async fn first_identity_credits(
        &self,
        wallet: &PlatformWallet,
        fresh: bool,
    ) -> Option<u64> {
        let read = || async {
            let state = wallet.state().await;
            let mine = of_wallet(&state.identity_manager, wallet.wallet_id())
                .filter_map(|m| Some((m.identity_index?, (m.identity.id(), m.identity.balance()))));
            shown_of(wallet.wallet_id(), mine).map(|(_, found)| found)
        };
        let (id, balance) = read().await?;
        if !fresh {
            return Some(balance);
        }
        let _ = wallet.identity().refresh_identity(&id).await;
        read().await.map(|(_, balance)| balance)
    }
}
