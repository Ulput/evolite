//! The shielded pool (Orchard) on platform-wallet: connecting the wallet's pool, what is
//! in it, and the moves in, inside and out. Built only with the `pool` feature; without it every
//! move answers "not in this build" and the snapshot shows no pool.
//!
//! The pool's viewing keys go to the store on the first connection (the phrase is needed then);
//! spending keys are derived from the seed for each move and dropped after it. Proofs are built
//! on this computer: tens of seconds of CPU, on a blocking thread so the loop goes on.

use crate::chain::Chain;
use evolite_api::Activity;
use platform_wallet::PlatformWallet;
use std::path::Path;

/// The pool as the snapshot shows it.
#[derive(Clone)]
pub struct PoolView {
    /// Spendable credits (notes reserved by a move in flight are not counted).
    pub credits: u64,
    pub notes: u32,
    /// The wallet's private address (`tdash1…`, Orchard), account 0.
    pub address: String,
    /// Unix seconds of the last completed read of the pool on Platform.
    pub checked_at: Option<i64>,
    /// Newest first.
    pub operations: Vec<Activity>,
}

/// Why a pool move did not end in a clear yes.
#[derive(Debug)]
pub enum PoolError {
    /// Handed to the network, but its result could not be proved: it may have gone. Never
    /// resent; the next read of the pool shows what happened.
    #[cfg_attr(not(feature = "pool"), allow(dead_code))]
    Unconfirmed,
    Failed(String),
}

impl PoolError {
    #[cfg(feature = "pool")]
    fn of(error: platform_wallet::PlatformWalletError) -> Self {
        match error {
            platform_wallet::PlatformWalletError::ShieldedSpendUnconfirmed { .. } => {
                Self::Unconfirmed
            }
            other => Self::Failed(other.to_string()),
        }
    }
}

impl From<String> for PoolError {
    fn from(text: String) -> Self {
        Self::Failed(text)
    }
}

impl From<&str> for PoolError {
    fn from(text: &str) -> Self {
        Self::Failed(text.to_owned())
    }
}

/// Whether `text` is a private (Orchard) address; never in a build without the pool.
pub fn is_private_address(text: &str) -> bool {
    #[cfg(feature = "pool")]
    return dpp::address_funds::OrchardAddress::from_bech32m_string(text).is_ok();
    #[cfg(not(feature = "pool"))]
    {
        let _ = text;
        false
    }
}

/// A shield from Core that did not finish: the lock platform-wallet tracks for the pool, made
/// here and not yet taken by Platform.
async fn unfinished_lock(wallet: &PlatformWallet) -> Option<platform_wallet::TrackedAssetLock> {
    wallet
        .asset_locks()
        .list_tracked_locks()
        .await
        .into_iter()
        .find(|l| {
            l.funding_type == platform_wallet::AssetLockFundingType::AssetLockShieldedAddressTopUp
                && !l.status.is_consumed()
                // Found on chain, not made here: whether Platform took it is unknown.
                && l.status != platform_wallet::AssetLockStatus::RecoveredFromChain
        })
}

impl Chain {
    /// How many duffs an unfinished shield from Core has locked, if one is waiting.
    pub async fn unfinished_pool_shield(&self, wallet: &PlatformWallet) -> Option<u64> {
        unfinished_lock(wallet).await.map(|l| l.amount)
    }

    /// The asset locks made for the pool, finished or not, by their Core transaction (txid) with
    /// the duffs each locked: the history names them "Shielded from Core", with the locked amount.
    pub async fn pool_locks(
        &self,
        wallet: &PlatformWallet,
    ) -> std::collections::HashMap<String, u64> {
        wallet
            .asset_locks()
            .list_tracked_locks()
            .await
            .into_iter()
            .filter(|l| {
                l.funding_type
                    == platform_wallet::AssetLockFundingType::AssetLockShieldedAddressTopUp
            })
            .map(|l| (l.out_point.txid.to_string(), l.amount))
            .collect()
    }
}

/// What the person sees when this build has no pool.
#[cfg(not(feature = "pool"))]
const NO_POOL: &str = tr!("This build of evolite has no private pool");

/// `read`, noted in `last`; `last` as it is when `read` takes longer than `wait`.
#[cfg(feature = "pool")]
async fn read_or_last<T: Clone>(
    last: &std::sync::Mutex<Option<T>>,
    wait: std::time::Duration,
    read: impl std::future::Future<Output = Option<T>>,
) -> Option<T> {
    match tokio::time::timeout(wait, read).await {
        Ok(value) => {
            *last.lock().unwrap_or_else(|e| e.into_inner()) = value.clone();
            value
        }
        Err(_) => last.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    }
}

/// A read for wallet `id` is only that wallet's: another's last read is not shown (a switch).
#[cfg(feature = "pool")]
fn view_of<K: PartialEq, V>(read: Option<(K, V)>, id: &K) -> Option<V> {
    read.filter(|(wallet, _)| wallet == id)
        .map(|(_, view)| view)
}

/// The notes and operations of a read that found the store busy: this wallet's last ones, not
/// none, which would stay on screen for the whole scan (review r1 should 1). No last read: none.
#[cfg(feature = "pool")]
fn last_lines<K: PartialEq>(
    last: &std::sync::Mutex<Option<(K, PoolView)>>,
    id: &K,
) -> (u32, Vec<Activity>) {
    match &*last.lock().unwrap_or_else(|e| e.into_inner()) {
        Some((wallet, view)) if wallet == id => (view.notes, view.operations.clone()),
        _ => (0, Vec::new()),
    }
}

/// The one Orchard account evolite uses.
#[cfg(feature = "pool")]
const ACCOUNT: u32 = 0;

#[cfg(feature = "pool")]
mod imp {
    use super::*;
    use evolite_api::{ActivityKind, Chain as Layer, Proof};
    use platform_wallet::wallet::shielded::{
        CachedOrchardProver, ShieldedActivityEntry, ShieldedActivityKind, ShieldedActivityStatus,
        ShieldedDirection, ShieldedLocalBalanceState, ShieldedStore, SubwalletId,
        sort_activity_for_display,
    };
    use std::sync::Arc;

    /// History rows read for the screen.
    const SHOWN_OPERATIONS: usize = 50;

    /// How long a snapshot waits for the pool's reads. A scan holds the pool's store for seconds
    /// (the first one for minutes): the snapshot, and with it every line of the history, does not
    /// wait behind it (BUG-105).
    const POOL_READ_WAIT: std::time::Duration = std::time::Duration::from_millis(200);

    /// The account's spendable credits by the store's ledger, and when the pool was read: the
    /// time of the last pass only while the ledger is ready (BUG-84). platform-wallet stamps a
    /// pass that failed too, and a ledger not ready reads as 0 — that 0 is not known, so no
    /// time, and the screens say «not known yet».
    pub fn read_ledger(
        state: Option<&ShieldedLocalBalanceState>,
        last_pass: Option<u64>,
    ) -> (u64, Option<i64>) {
        match state {
            Some(ShieldedLocalBalanceState::Ready(snapshot)) => (
                snapshot
                    .accounts
                    .get(&ACCOUNT)
                    .map_or(0, |a| a.spendable_credits),
                last_pass.map(|s| s as i64),
            ),
            _ => (0, None),
        }
    }

    /// The Core fee rate of a withdrawal from the pool, duffs a byte (the network minimum).
    const CORE_FEE_PER_BYTE: u32 = 1;

    impl Chain {
        /// Connects the wallet's pool: the network-wide note store at `db` (once per run), the
        /// wallet's viewing keys (from the store, else from `seed`, which then saves them),
        /// and the background scan. Needs the masternode lists (proofs). Idempotent.
        pub async fn connect_pool(
            &self,
            wallet: &PlatformWallet,
            seed: &[u8],
            db: &Path,
        ) -> Result<(), String> {
            let manager = self.manager().await?;
            manager
                .configure_shielded(db)
                .await
                .map_err(|e| e.to_string())?;
            let coordinator = manager
                .shielded_coordinator()
                .await
                .ok_or(tr!("the pool is not set up"))?;
            if wallet.shielded_default_address(ACCOUNT).await.is_none()
                && !wallet
                    .bind_shielded_from_persisted(&[ACCOUNT], &coordinator)
                    .await
                    .map_err(|e| e.to_string())?
            {
                wallet
                    .bind_shielded(seed, &[ACCOUNT], &coordinator)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            let sync = manager.shielded_sync_arc();
            if !sync.is_running() {
                sync.set_interval(std::time::Duration::from_secs(60));
                sync.start();
            }
            Ok(())
        }

        /// What the snapshot shows; `None` while the pool is not connected. Local reads only. One
        /// that waits behind a scan holding the store gives up after [`POOL_READ_WAIT`]: the last
        /// read is shown instead (BUG-105).
        pub async fn pool_view(&self, wallet: &PlatformWallet) -> Option<PoolView> {
            let id = wallet.wallet_id();
            let read = async { self.read_pool_view(wallet).await.map(|view| (id, view)) };
            super::view_of(
                super::read_or_last(&self.last_pool, POOL_READ_WAIT, read).await,
                &id,
            )
        }

        async fn read_pool_view(&self, wallet: &PlatformWallet) -> Option<PoolView> {
            let manager = self.manager().await.ok()?;
            let coordinator = manager.shielded_coordinator().await?;
            let raw = wallet.shielded_default_address(ACCOUNT).await?;
            let address = dpp::address_funds::OrchardAddress::from_raw_bytes(&raw)
                .ok()?
                .to_bech32m_string(crate::network::dash());
            let state = coordinator.local_balance_snapshot(wallet.wallet_id()).await;
            let (credits, checked_at) = read_ledger(
                state.as_ref().ok(),
                manager.shielded_sync().last_sync_unix_seconds(),
            );
            let id = SubwalletId::new(wallet.wallet_id(), ACCOUNT);
            let (notes, operations) = match coordinator.store().try_read() {
                Ok(store) => (
                    store.get_unspent_notes(id).map_or(0, |n| n.len() as u32),
                    operations(
                        store
                            .get_activity(id, 0, SHOWN_OPERATIONS)
                            .unwrap_or_default(),
                    ),
                ),
                Err(_) => super::last_lines(&self.last_pool, &wallet.wallet_id()),
            };
            Some(PoolView {
                credits,
                notes,
                address,
                checked_at,
                operations,
            })
        }

        /// Shields `credits` from the identity the interface shows (its TRANSFER key signs);
        /// the note goes to the wallet's own private address.
        pub async fn shield_from_identity(
            &self,
            wallet: &PlatformWallet,
            keys: &key_wallet::Wallet,
            credits: u64,
        ) -> Result<(), PoolError> {
            let coordinator = self.coordinator().await?;
            let (identity, signer) = Self::first_identity(wallet, keys).await?;
            let id = dpp::identity::accessors::IdentityGettersV0::id(&identity);
            prove(async move {
                wallet
                    .shielded_shield_from_identity(
                        &coordinator,
                        ACCOUNT,
                        &id,
                        credits,
                        &signer,
                        &CachedOrchardProver::new(),
                    )
                    .await
                    .map(|_| ())
                    .map_err(PoolError::of)
            })
            .await?;
            let _ = wallet.identity().refresh_identity(&id).await;
            self.read_pool_now().await;
            Ok(())
        }

        /// Shields `credits` from the wallet's Platform addresses (account 0): platform-wallet
        /// picks the inputs; each is signed with its key derived from `keys`.
        pub async fn shield_from_platform_address(
            &self,
            wallet: &PlatformWallet,
            keys: &key_wallet::Wallet,
            credits: u64,
        ) -> Result<(), PoolError> {
            let coordinator = self.coordinator().await?;
            let signer = platform_address_signer(wallet, keys).await?;
            prove(async move {
                wallet
                    .shielded_shield_from_account(
                        &coordinator,
                        ACCOUNT,
                        PAYMENT_ACCOUNT,
                        credits,
                        &signer,
                        &CachedOrchardProver::new(),
                    )
                    .await
                    .map_err(PoolError::of)
            })
            .await?;
            self.read_pool_now().await;
            Ok(())
        }

        /// Takes `credits` out of the pool to `to` (a Platform address, `tdash1…`).
        pub async fn unshield(
            &self,
            wallet: &PlatformWallet,
            seed: &[u8],
            to: &str,
            credits: u64,
        ) -> Result<(), PoolError> {
            let coordinator = self.coordinator().await?;
            prove(async move {
                wallet
                    .shielded_unshield_to(
                        &coordinator,
                        seed,
                        ACCOUNT,
                        to,
                        credits,
                        &CachedOrchardProver::new(),
                    )
                    .await
                    .map_err(PoolError::of)
            })
            .await?;
            self.read_pool_now().await;
            Ok(())
        }

        /// A private transfer of `credits` to another private address (`tdash1…`, Orchard).
        pub async fn pool_transfer(
            &self,
            wallet: &PlatformWallet,
            seed: &[u8],
            to: &str,
            credits: u64,
        ) -> Result<(), PoolError> {
            let coordinator = self.coordinator().await?;
            let to = dpp::address_funds::OrchardAddress::from_bech32m_string(to)
                .map_err(|e| e.to_string())?
                .to_raw_bytes();
            prove(async move {
                wallet
                    .shielded_transfer_to(
                        &coordinator,
                        seed,
                        ACCOUNT,
                        &to,
                        credits,
                        [0u8; 36],
                        &CachedOrchardProver::new(),
                    )
                    .await
                    .map_err(PoolError::of)
            })
            .await?;
            self.read_pool_now().await;
            Ok(())
        }

        /// Takes `credits` out of the pool to a Core address: Platform pays it out with the
        /// next withdrawal batch, minutes later, like a credit withdrawal.
        pub async fn pool_withdraw(
            &self,
            wallet: &PlatformWallet,
            seed: &[u8],
            to: &str,
            credits: u64,
        ) -> Result<(), PoolError> {
            let coordinator = self.coordinator().await?;
            prove(async move {
                wallet
                    .shielded_withdraw_to(
                        &coordinator,
                        seed,
                        ACCOUNT,
                        to,
                        credits,
                        CORE_FEE_PER_BYTE,
                        &CachedOrchardProver::new(),
                    )
                    .await
                    .map_err(PoolError::of)
            })
            .await?;
            self.read_pool_now().await;
            Ok(())
        }

        /// Shields `duffs` of Core funds: an asset lock (proved by InstantSend or ChainLock),
        /// whose value less Platform's fee becomes a note at the wallet's own private address.
        /// A lock that an earlier try left unfinished is finished instead and no new one is made,
        /// whatever `duffs` says: its money is already locked.
        pub async fn shield_from_core(
            &self,
            wallet: &PlatformWallet,
            keys: &key_wallet::Wallet,
            duffs: u64,
        ) -> Result<(), PoolError> {
            let coordinator = self.coordinator().await?;
            let raw = wallet
                .shielded_default_address(ACCOUNT)
                .await
                .ok_or(tr!("The private pool is not connected yet"))?;
            let address = dpp::address_funds::OrchardAddress::from_raw_bytes(&raw)
                .map_err(|e| e.to_string())?;
            let funding = match unfinished_lock(wallet).await {
                Some(lock) => platform_wallet::AssetLockFunding::FromExistingAssetLock {
                    out_point: lock.out_point,
                    consume_invitation_voucher: false,
                },
                None => platform_wallet::AssetLockFunding::FromWalletBalance {
                    amount_duffs: duffs,
                    account_index: 0,
                },
            };
            let signer = crate::identity::AssetLockSigner(keys.clone());
            prove(async move {
                wallet
                    .shielded_fund_from_asset_lock(
                        &coordinator,
                        funding,
                        vec![(address, None)],
                        &signer,
                        &CachedOrchardProver::new(),
                        None,
                        0,
                        None,
                        None,
                    )
                    .await
                    .map_err(PoolError::of)
            })
            .await?;
            self.read_pool_now().await;
            Ok(())
        }

        /// Takes `credits` out of the pool into the credits of `identity`: the identity gets
        /// `credits`, Platform's fee comes from the notes on top.
        pub async fn top_up_from_pool(
            &self,
            wallet: &PlatformWallet,
            seed: &[u8],
            identity: &dpp::prelude::Identifier,
            credits: u64,
        ) -> Result<(), PoolError> {
            let coordinator = self.coordinator().await?;
            prove(async move {
                wallet
                    .shielded_identity_top_up_from_pool(
                        &coordinator,
                        seed,
                        ACCOUNT,
                        identity,
                        credits,
                        &CachedOrchardProver::new(),
                    )
                    .await
                    .map(|_| ())
                    .map_err(PoolError::of)
            })
            .await?;
            let _ = wallet.identity().refresh_identity(identity).await;
            self.read_pool_now().await;
            Ok(())
        }

        async fn coordinator(
            &self,
        ) -> Result<Arc<platform_wallet::wallet::shielded::NetworkShieldedCoordinator>, String>
        {
            self.manager()
                .await?
                .shielded_coordinator()
                .await
                .ok_or_else(|| tr!("The private pool is not connected yet").to_owned())
        }
    }

    /// Runs a move that builds an Orchard proof on a blocking thread of this runtime: the proof
    /// takes the CPU for tens of seconds and must not hold a worker the loop needs.
    async fn prove<F>(work: F) -> Result<(), PoolError>
    where
        F: std::future::Future<Output = Result<(), PoolError>>,
    {
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(move || handle.block_on(work))
    }

    impl Chain {
        /// Reads the pool on Platform now, after a move: the change note and the spent ones
        /// show at once instead of at the next background pass.
        pub async fn read_pool_now(&self) {
            if let Ok(manager) = self.manager().await {
                let _ = manager.shielded_sync().sync_now(true).await;
            }
        }
    }

    /// The Platform payment account evolite receives on (`tdash1…`, DIP-17 account 0).
    const PAYMENT_ACCOUNT: u32 = 0;

    /// A signer of the wallet's Platform addresses: every address of the payment account that
    /// platform-wallet knows, with its private key derived from `keys` (dropped with the signer).
    async fn platform_address_signer(
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<simple_signer::signer::SimpleSigner, String> {
        crate::spv::chain::addresses::address_signer(wallet, keys).await
    }

    /// The rows newest first, as platform-wallet orders them for display: notes found by the
    /// scan have no time (`created_at_ms` 0, "time unknown") and come after the dated rows, in
    /// chain order by `min_note_position` — the same on every device (BUG-58, R-33 p. 8). The
    /// history's sort by time keeps that order among them.
    pub fn operations(mut entries: Vec<ShieldedActivityEntry>) -> Vec<Activity> {
        sort_activity_for_display(&mut entries);
        entries.iter().filter_map(operation).collect()
    }

    /// One history row as the screen shows it; failed and abandoned moves are left out.
    fn operation(entry: &ShieldedActivityEntry) -> Option<Activity> {
        let proof = match entry.status {
            ShieldedActivityStatus::Confirmed => Proof::Proved,
            ShieldedActivityStatus::Pending => Proof::Sending,
            _ => return None,
        };
        let duffs = crate::identity::credits_as_duffs(entry.amount);
        let fee = entry.fee.map(crate::identity::credits_as_duffs);
        let (kind, amount) = match (&entry.kind, entry.direction) {
            (ShieldedActivityKind::Shield | ShieldedActivityKind::ShieldFromAssetLock, _)
            | (ShieldedActivityKind::ShieldFromIdentity { .. }, _) => {
                (ActivityKind::Shielded { from: None }, duffs)
            }
            (_, ShieldedDirection::In) => (ActivityKind::PoolReceived, duffs),
            // To our own private address: only the fee leaves.
            (_, ShieldedDirection::SelfTransfer) => (
                ActivityKind::SelfTransfer { moved: 0 },
                -fee.unwrap_or_default(),
            ),
            (_, ShieldedDirection::Out) => (ActivityKind::PoolSent, -(duffs + fee.unwrap_or(0))),
        };
        let mut id = [0u8; 8];
        id.copy_from_slice(&entry.id[..8]);
        // The source of a shield, which only the scan knows (FEAT-0009 R-1).
        let (mut layers, flow) = crate::layers::of(&kind, Layer::Platform);
        layers |= match entry.kind {
            ShieldedActivityKind::ShieldFromAssetLock => evolite_api::layer::CORE,
            ShieldedActivityKind::ShieldFromIdentity { .. } => evolite_api::layer::IDENTITY,
            ShieldedActivityKind::Shield => evolite_api::layer::PLATFORM_ADDRESS,
            _ => 0,
        };
        Some(Activity {
            id: u64::from_le_bytes(id),
            kind,
            chain: Layer::Platform,
            at: (entry.created_at_ms / 1000) as i64,
            amount,
            proof,
            txid: None,
            fee,
            block: entry.block_height.map(|h| h as u32),
            address: None,
            layers,
            flow,
            key: format!("pool:{:016x}", u64::from_le_bytes(id)),
            detail: None,
            note: None,
        })
    }
}

#[cfg(all(test, feature = "pool"))]
mod display_order_tests {
    use super::imp::operations;
    use platform_wallet::wallet::shielded::{
        ShieldedActivityEntry, ShieldedActivityKind, ShieldedActivityStatus, ShieldedDirection,
    };

    fn found(n: u8, position: Option<u64>) -> ShieldedActivityEntry {
        ShieldedActivityEntry {
            id: [n; 32],
            kind: ShieldedActivityKind::Received,
            direction: ShieldedDirection::In,
            amount: 1_000_000 * u64::from(n),
            fee: None,
            counterparty: None,
            memo: None,
            block_height: None,
            status: ShieldedActivityStatus::Confirmed,
            created_at_ms: 0,
            min_note_position: position,
            note_cmxs: Vec::new(),
            spent_nullifiers: Vec::new(),
        }
    }

    /// BUG-58: notes the scan found have no time: "time unknown" (0, not 1970), after the
    /// dated rows, later in the chain first, the history's sort by time keeping that order.
    #[test]
    fn undated_notes_come_last_in_chain_order() {
        let dated = ShieldedActivityEntry {
            created_at_ms: 1_700_000_000_000,
            ..found(9, None)
        };
        let rows = operations(vec![
            found(1, Some(10)),
            found(2, None),
            dated,
            found(3, Some(30)),
        ]);
        let mut history = rows.clone();
        history.sort_by_key(|a| std::cmp::Reverse(a.at));
        // Credits shown as duffs (1 000 credits each).
        let amounts: Vec<_> = history.iter().map(|a| (a.at, a.amount)).collect();
        assert_eq!(
            amounts,
            vec![(1_700_000_000, 9_000), (0, 3_000), (0, 1_000), (0, 2_000)]
        );
        assert_eq!(rows, history);
    }
}

#[cfg(not(feature = "pool"))]
impl Chain {
    pub async fn connect_pool(&self, _: &PlatformWallet, _: &[u8], _: &Path) -> Result<(), String> {
        Err(NO_POOL.into())
    }

    pub async fn pool_view(&self, _: &PlatformWallet) -> Option<PoolView> {
        None
    }

    pub async fn shield_from_platform_address(
        &self,
        _: &PlatformWallet,
        _: &key_wallet::Wallet,
        _: u64,
    ) -> Result<(), PoolError> {
        Err(NO_POOL.into())
    }

    pub async fn shield_from_identity(
        &self,
        _: &PlatformWallet,
        _: &key_wallet::Wallet,
        _: u64,
    ) -> Result<(), PoolError> {
        Err(NO_POOL.into())
    }

    pub async fn unshield(
        &self,
        _: &PlatformWallet,
        _: &[u8],
        _: &str,
        _: u64,
    ) -> Result<(), PoolError> {
        Err(NO_POOL.into())
    }

    pub async fn pool_transfer(
        &self,
        _: &PlatformWallet,
        _: &[u8],
        _: &str,
        _: u64,
    ) -> Result<(), PoolError> {
        Err(NO_POOL.into())
    }

    pub async fn pool_withdraw(
        &self,
        _: &PlatformWallet,
        _: &[u8],
        _: &str,
        _: u64,
    ) -> Result<(), PoolError> {
        Err(NO_POOL.into())
    }

    pub async fn shield_from_core(
        &self,
        _: &PlatformWallet,
        _: &key_wallet::Wallet,
        _: u64,
    ) -> Result<(), PoolError> {
        Err(NO_POOL.into())
    }

    pub async fn top_up_from_pool(
        &self,
        _: &PlatformWallet,
        _: &[u8],
        _: &dpp::prelude::Identifier,
        _: u64,
    ) -> Result<(), PoolError> {
        Err(NO_POOL.into())
    }
}

/// The pool's fees, in duffs, from Platform's own formulas at `version` — the ones its builders
/// carve from the notes and its consensus checks — for a bundle of two Orchard actions (one or
/// two notes spent). Shielding from addresses is metered by Platform; its formula gives the floor.
#[cfg(feature = "pool")]
pub fn set_fees(fees: &mut evolite_api::Fees, version: &dpp::version::PlatformVersion) {
    use dpp::shielded::{
        compute_minimum_shielded_fee, compute_shielded_identity_balance_write_fee,
        compute_shielded_identity_top_up_fee, compute_shielded_unshield_fee,
        compute_shielded_withdrawal_fee,
    };
    // Rounded up: a fee is never shown smaller than it is.
    let duffs = |credits: Result<u64, dpp::ProtocolError>| {
        credits.ok().map(|c| c.div_ceil(1000) as evolite_api::Duffs)
    };
    const ACTIONS: usize = 2;
    if let Some(fee) = duffs(compute_minimum_shielded_fee(ACTIONS, version)) {
        fees.pool_transfer = fee;
        fees.shield_from_address = fee;
        // What platform-wallet keeps on the first address (`shield_fee_reserve_credits`).
        fees.shield_reserve = 2 * fee;
    }
    if let Some(fee) = duffs(compute_shielded_identity_balance_write_fee(
        ACTIONS, version,
    )) {
        fees.shield = fee;
    }
    if let Some(fee) = duffs(compute_shielded_unshield_fee(ACTIONS, version)) {
        fees.unshield = fee;
    }
    if let Some(fee) = duffs(compute_shielded_withdrawal_fee(ACTIONS, version)) {
        fees.pool_withdrawal = fee;
    }
    if let Some(fee) = duffs(compute_shielded_identity_top_up_fee(ACTIONS, version)) {
        fees.pool_top_up = fee;
    }
    // A shield from Core: the base fee plus the asset lock's processing cost, both taken from
    // the lock (platform-wallet's `shield_from_asset_lock_pool_fee`).
    let lock_cost = version
        .dpp
        .state_transitions
        .identities
        .asset_locks
        .required_asset_lock_duff_balance_for_processing_start_for_address_funding;
    if let Ok(base) = compute_minimum_shielded_fee(ACTIONS, version) {
        fees.shield_from_core =
            base.div_ceil(1000) as evolite_api::Duffs + lock_cost as evolite_api::Duffs;
    }
}

/// The fees at protocol `version`, for the tests.
#[cfg(all(test, feature = "pool"))]
fn fees_at(version: u32) -> evolite_api::Fees {
    let mut fees = crate::convert::fees(0);
    set_fees(
        &mut fees,
        dpp::version::PlatformVersion::get(version).unwrap(),
    );
    fees
}

#[cfg(not(feature = "pool"))]
pub fn set_fees(_: &mut evolite_api::Fees, _: &dpp::version::PlatformVersion) {}

#[cfg(all(test, feature = "pool"))]
mod tests {
    use super::*;
    use evolite_api::{ActivityKind, Chain as Layer, Proof};

    const WAIT: std::time::Duration = std::time::Duration::from_millis(30);

    /// At most this: a read that never ends must not take the test with it.
    async fn bounded<T>(work: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(std::time::Duration::from_secs(5), work)
            .await
            .expect("waited behind the read")
    }

    fn a_line(id: u64) -> Activity {
        Activity {
            id,
            kind: ActivityKind::PoolSent,
            chain: Layer::Platform,
            at: 1_000,
            amount: -1_000,
            proof: Proof::Proved,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers: evolite_api::layer::POOL,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            detail: None,
            note: None,
        }
    }

    fn a_view(notes: u32, operations: Vec<Activity>) -> PoolView {
        PoolView {
            credits: 5,
            notes,
            address: "tdash1".to_owned(),
            checked_at: None,
            operations,
        }
    }

    /// Review r1 should 1: the store busy, a read keeps the wallet's last notes and lines.
    #[test]
    fn a_busy_store_keeps_the_wallets_last_notes_and_lines() {
        let last = std::sync::Mutex::new(Some((1u8, a_view(3, vec![a_line(9)]))));
        assert_eq!(last_lines(&last, &1), (3, vec![a_line(9)]));
    }

    #[test]
    fn a_busy_store_with_no_last_read_of_the_wallet_has_none() {
        let none = std::sync::Mutex::new(None::<(u8, PoolView)>);
        assert_eq!(last_lines(&none, &1), (0, Vec::new()));
        let other = std::sync::Mutex::new(Some((2u8, a_view(3, vec![a_line(9)]))));
        assert_eq!(last_lines(&other, &1), (0, Vec::new()), "another wallet's");
    }

    /// Review r1 P3: what `pool_view` does with the read. `pool_view` itself needs a connected
    /// pool (a [`PlatformWallet`]), so the filter is apart.
    #[test]
    fn another_wallets_last_read_is_not_shown() {
        assert_eq!(view_of(Some((2u8, 7)), &1), None);
        assert_eq!(view_of(Some((1u8, 7)), &1), Some(7));
        assert_eq!(view_of(None::<(u8, u32)>, &1), None);
    }

    /// BUG-105: a scan held the store and the snapshot stood 4 s, or more, behind it.
    #[tokio::test]
    async fn the_snapshot_does_not_wait_behind_a_read_of_the_pool() {
        let last = std::sync::Mutex::new(Some(7));
        let shown = bounded(read_or_last(&last, WAIT, std::future::pending())).await;
        assert_eq!(shown, Some(7), "what was read last");
        assert_eq!(*last.lock().unwrap(), Some(7), "and it is kept");
    }

    #[tokio::test]
    async fn nothing_read_yet_and_a_read_that_waits_is_nothing() {
        let last = std::sync::Mutex::new(None::<u32>);
        assert_eq!(
            bounded(read_or_last(&last, WAIT, std::future::pending())).await,
            None
        );
    }

    #[tokio::test]
    async fn a_read_in_time_is_shown_and_kept() {
        let last = std::sync::Mutex::new(Some(7));
        assert_eq!(read_or_last(&last, WAIT, async { Some(9) }).await, Some(9));
        assert_eq!(*last.lock().unwrap(), Some(9));
        // Not connected any more: none is shown, not the old one.
        assert_eq!(read_or_last(&last, WAIT, async { None }).await, None);
        assert_eq!(*last.lock().unwrap(), None);
    }

    /// BUG-84: a pass stamped after it failed does not make a ledger that is not ready a read
    /// 0; a ready one — an empty pool too — is read at the pass's time.
    #[test]
    fn the_pool_is_read_only_when_its_ledger_is_ready() {
        use platform_wallet::wallet::shielded::{
            ShieldedLocalAccountBalance, ShieldedLocalBalanceSnapshot, ShieldedLocalBalanceState,
        };
        let stamp = Some(1_793_000_000);
        for state in [
            None,
            Some(ShieldedLocalBalanceState::Unbound),
            Some(ShieldedLocalBalanceState::RestoreIncomplete),
        ] {
            assert_eq!(
                imp::read_ledger(state.as_ref(), stamp),
                (0, None),
                "{state:?}"
            );
        }
        let ready = |credits| {
            let mut snapshot = ShieldedLocalBalanceSnapshot::default();
            snapshot.accounts.insert(
                ACCOUNT,
                ShieldedLocalAccountBalance {
                    spendable_credits: credits,
                    last_scanned_index: Some(0),
                    ..Default::default()
                },
            );
            ShieldedLocalBalanceState::Ready(snapshot)
        };
        assert_eq!(
            imp::read_ledger(Some(&ready(0)), stamp),
            (0, Some(1_793_000_000))
        );
        assert_eq!(
            imp::read_ledger(Some(&ready(250_000)), stamp),
            (250_000, Some(1_793_000_000))
        );
        // Ready before the first pass of this run (restored): the amount, no time yet.
        assert_eq!(imp::read_ledger(Some(&ready(7)), None), (7, None));
    }

    /// What testnet charged on 2026-10-31 (protocol 15): an unshield of 0.005 DASH took
    /// 120 223 duffs, a withdrawal to Core 226 480 — the formulas give the same.
    #[test]
    fn pool_fees_are_what_platform_charges() {
        let fees = fees_at(15);
        // The base fee plus the asset lock's 50 000 duffs of processing.
        assert_eq!(fees.shield_from_core, 164_140);
        assert_eq!(fees.pool_top_up, 114_360);
        assert_eq!(fees.unshield, 120_223);
        assert_eq!(fees.pool_withdrawal, 226_480);
        assert_eq!(fees.pool_transfer, 114_140);
        assert_eq!(fees.shield, 149_760);
        assert_eq!(fees.shield_reserve, 2 * fees.shield_from_address);
    }
}
