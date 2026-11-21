//! The chain side of the core on platform-wallet: one `PlatformWalletManager` with its SPV client
//! (dash-spv) and a SQLite store (platform-wallet-storage). The manager holds the wallet watch-only
//! — no private keys: the core signs with keys it derives from the phrase while the wallet is
//! open. History, coins and the scan height are on disk, so opening the wallet again picks up
//! where it was instead of rescanning.
//!
//! Proofs of Platform data are checked against quorum keys from this SPV client's own verified
//! masternode list ([`SpvContextProvider`]), not a server's.

pub mod address_fund;
pub mod addresses;
mod contacts;
mod contests;
mod contract_write;
mod contracts;
mod identities;
mod nft_series;
pub mod rebroadcast;
mod store_read;
mod token_actions;
mod token_manage;
mod tokens;

pub use contacts::*;
pub use contracts::WriteToken;
pub use identities::*;
pub use store_read::*;
pub use token_actions::{Bought, BuyRefused, ClaimDoc, Claimable, Every, Offer, Perpetual};
pub use token_manage::{IssuerDoc, IssuerDocKind, IssuerDone, IssuerOp};
pub use tokens::{CrawlFailure, CrawlTokens, Facts as TokenFacts, Found as FoundTokens};

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use dash_sdk::SdkBuilder;
use dash_sdk::platform::transition::put_settings::PutSettings;
use dash_spv::ClientConfig;
use dpp::address_funds::PlatformAddress;
use dpp::identity::IdentityPublicKey;
use dpp::identity::accessors::IdentityGettersV0;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use key_wallet::account::account_collection::{DashpayAccountKey, PlatformPaymentAccountKey};
use key_wallet::bip32::DerivationPath;
use key_wallet::dashcore::Address;
use key_wallet::wallet::initialization::WalletAccountCreationOptions;
use key_wallet::wallet::managed_wallet_info::transaction_builder::TransactionBuilder;
use key_wallet_manager::WalletId;
use platform_wallet::events::PlatformEventHandler;
use platform_wallet::wallet::identity::network::IdentityDiscoveryOptions;
use platform_wallet::{
    AssetLockFunding, AssetLockFundingType, AssetLockStatus, AutoAcceptProofSource, ProfileUpdate,
};
use platform_wallet::{PlatformWallet, PlatformWalletManager, SEND_FUNDING_SOURCES};
use platform_wallet_storage::sqlite::{SqlitePersister, SqlitePersisterConfig};

use crate::provider::{NotYet, SpvContextProvider};
use crate::sync::{Phase, Reporter, SyncStatus, VerifiedLocks};
use crate::{dashpay, identity, nfts};

/// The SQLite store with the DashPay backfill record and the spent-coin claims it does not keep
/// (BUG-77, [`crate::store`]).
type Manager = PlatformWalletManager<crate::store::Store<SqlitePersister>>;

/// Where the chain side keeps its files.
#[derive(Clone)]
pub struct ChainPaths {
    /// dash-spv's headers, filters and masternode lists.
    pub spv: PathBuf,
    /// platform-wallet's state: public data only, never keys.
    pub store: PathBuf,
    /// "Own node": Core's only peer, read at the start ([`crate::node`]).
    pub core_node: PathBuf,
}

pub struct Chain {
    manager: tokio::sync::Mutex<Option<Arc<Manager>>>,
    /// Held while [`Chain::start`] builds the manager, and by [`Chain::stop`]: one start at a
    /// time, and a stop waits for a start under way. [`Chain::manager`] waits for neither
    /// (BUG-24: every command stood in line for the whole start).
    starting: tokio::sync::Mutex<()>,
    /// The SPV client runs (the manager in `manager` syncs): read without waiting for its lock.
    running: AtomicBool,
    /// TECH-spv-prune: dash-spv's pruned count last journaled ([`Self::masternodes_pruned_since`]).
    mn_pruned_seen: std::sync::atomic::AtomicUsize,
    /// TECH-spv-prune review r1 S2: the synced quorum counts were journaled.
    mn_quorums_told: AtomicBool,
    /// The store is open and its wallets are loaded: a manager is in `manager`, maybe before
    /// its SPV client starts (BUG-25: that took 12 s in a release build and half a minute or
    /// more in a debug one, and the wallet's identities, balance and history waited for it).
    store_open: AtomicBool,
    /// The pool as last read, with its wallet: shown while a read has to wait (BUG-105).
    #[cfg(feature = "pool")]
    pub(crate) last_pool: Mutex<Option<(WalletId, crate::platform::pool::PoolView)>>,
    /// BUG-240 r4 (R3): told after a pool pass that failed on something passing; the pool's
    /// task makes one early pass (`connect_pool`).
    pub(crate) pool_retry: Arc<tokio::sync::Notify>,
    /// Its core has been replaced: a start not yet made is not made.
    retired: AtomicBool,
    status: Arc<Mutex<SyncStatus>>,
    /// What Platform said of top-up locks that the store cannot keep: "already taken" while
    /// the lock has no ChainLock proof yet (platform-wallet then leaves its status as it was).
    /// Until the app closes.
    top_up_notes: Mutex<HashMap<key_wallet::dashcore::OutPoint, TopUpNote>>,
    /// Transactions whose InstantLock was verified (R-1): the rest are not shown as InstantSend.
    instant_locks: VerifiedLocks,
    /// BUG-260: the wallet's records as the store keeps them, read by the core after each new
    /// record (`set_stored_history`): rounds are counted over them and the ones in memory.
    stored_history: std::sync::Mutex<
        Arc<Vec<key_wallet::managed_account::transaction_record::TransactionRecord>>,
    >,
    /// The store the manager writes through, kept to write the address pools platform-wallet
    /// leaves out (BUG-194, [`Self::save_pools`]).
    persister: Mutex<Option<Arc<crate::store::Store<SqlitePersister>>>>,
    /// SPV item 3: the coins this chain locked for platform-wallet's builders not to take (not
    /// final), with how many builds under way hold each: [`crate::convert::freeze_unfinal`].
    frozen: Arc<Mutex<Frozen>>,
    /// The network's contracts kept on this computer (FEAT-0031 C5), opened with the store.
    contracts: Mutex<Option<Arc<crate::platform::contracts::ContractCache>>>,
    /// BUG-269: the Workshop's model pictures read this run, to draw ([`Self::workshop_picture`]).
    workshop_pictures: Mutex<Vec<WorkshopPicture>>,
    /// Platform reads past [`Self::ready`]: the ones that would go to the network (BUG-78).
    #[cfg(any(test, feature = "test-support"))]
    pub proof_reads: std::sync::atomic::AtomicUsize,
    /// Starts asked ([`Self::mark_starting`]) and stops made: which path restarts the chain
    /// (BUG-76).
    #[cfg(any(test, feature = "test-support"))]
    pub starts: std::sync::atomic::AtomicUsize,
    #[cfg(any(test, feature = "test-support"))]
    pub stops: std::sync::atomic::AtomicUsize,
    /// DashPay scans asked of this chain: an onboarding's import goes through the parked one
    /// (BUG-76 r1).
    #[cfg(any(test, feature = "test-support"))]
    pub dashpay_scans: std::sync::atomic::AtomicUsize,
}

/// BUG-240 r4 (R1): how often the SDK asks another node before a read fails. Its own default is
/// 3; the pool's scan reads its notes in many long chunks, and on testnet one chunk spent all
/// three on nodes a block or two behind and failed the whole pass (QA batch 45). A broadcast
/// tried again elsewhere is the same signed transition with the same nonce: no second write. The
/// waits stay capped by `sent::run` and `names::within`.
const PLATFORM_RETRIES: usize = 8;

/// BUG-240 r4 (R1b): how many blocks a node's answer may be behind the highest height seen
/// before the SDK calls the node stale and asks another. The SDK's own is 1; on testnet nearly
/// every node is 2–3 behind and the pool's scan asked again and again until a chunk failed the
/// pass. With 6, QA's three first scans of a fresh copy all finished (batch 48). The answer is
/// still proved against the quorum that signed it, only up to six blocks (under a minute)
/// older; a read right after a write of ours counts «not seen» as not known (BUG-243). Never
/// `with_time_tolerance(None)`: the signed time within 31 minutes is the absolute guard.
pub(crate) const PLATFORM_HEIGHT_TOLERANCE: u64 = 6;

/// The SDK for `network`, with this app's request rules (BUG-240 r4).
fn sdk_builder(network: evolite_api::Network) -> SdkBuilder {
    let builder = match network {
        evolite_api::Network::Testnet => SdkBuilder::new_testnet(),
        evolite_api::Network::Mainnet => SdkBuilder::new_mainnet(),
    };
    builder
        .with_settings(dash_sdk::RequestSettings {
            retries: Some(PLATFORM_RETRIES),
            ..Default::default()
        })
        .with_height_tolerance(Some(PLATFORM_HEIGHT_TOLERANCE))
}

/// BUG-240 r3 (A): each wallet's pool passes in this run, as [`SyncStatus`] keeps them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PoolCounts {
    pub passed: std::collections::BTreeMap<[u8; 32], u64>,
    pub failed: std::collections::BTreeMap<[u8; 32], (u32, &'static str)>,
    /// BUG-240 r4 (R2): the passes failed on something passing, counted apart.
    pub waited: std::collections::BTreeMap<[u8; 32], (u32, &'static str)>,
}

impl Default for Chain {
    fn default() -> Self {
        Self {
            manager: tokio::sync::Mutex::new(None),
            starting: tokio::sync::Mutex::new(()),
            running: AtomicBool::new(false),
            mn_pruned_seen: std::sync::atomic::AtomicUsize::new(0),
            mn_quorums_told: AtomicBool::new(false),
            store_open: AtomicBool::new(false),
            #[cfg(feature = "pool")]
            last_pool: Mutex::new(None),
            pool_retry: Arc::new(tokio::sync::Notify::new()),
            retired: AtomicBool::new(false),
            status: Arc::new(Mutex::new(SyncStatus {
                network: crate::network::name(),
                ..Default::default()
            })),
            top_up_notes: Mutex::new(HashMap::new()),
            instant_locks: VerifiedLocks::default(),
            stored_history: Default::default(),
            persister: Mutex::new(None),
            frozen: Default::default(),
            contracts: Mutex::new(None),
            workshop_pictures: Mutex::new(Vec::new()),
            #[cfg(any(test, feature = "test-support"))]
            proof_reads: Default::default(),
            #[cfg(any(test, feature = "test-support"))]
            starts: Default::default(),
            #[cfg(any(test, feature = "test-support"))]
            stops: Default::default(),
            #[cfg(any(test, feature = "test-support"))]
            dashpay_scans: Default::default(),
        }
    }
}

/// Where credits go.
pub enum CreditPayee {
    /// Another identity (a resolved DPNS name): a credit transfer.
    Identity(Identifier),
    /// A Platform address (`tdash1…`): credits stay on Platform.
    Platform(PlatformAddress),
    /// A Core address: a withdrawal, paid out by masternodes some blocks later.
    Core(Address),
}

/// How many times a top-up's lock is sent again as a new transition when the network answers
/// "already in the cache" ([`is_cached_duplicate`]).
const SAME_TRANSITION_RETRIES: u16 = 2;

/// BUG-269: how many model pictures the Workshop keeps to draw — a collection page's worth.
const WORKSHOP_PICTURES: usize = 64;

/// BUG-269: a model's picture as stored, by its key.
struct WorkshopPicture {
    key: String,
    bytes: Arc<[u8]>,
    deflated: bool,
}

/// Why a Platform read was not made ([`Chain::ready`]). Not shown as it is: a caller that
/// answers the UI says [`evolite_api::CoreError::NotSynced`] for it.
pub const NOT_READY: &str =
    "the masternode lists are not in yet: Platform's proofs cannot be checked";

/// The network has this very transition in its cache of seen ones (gRPC `AlreadyExists`, "tx
/// already exists in cache"): it said nothing of whether Platform took it.
pub fn is_cached_duplicate(error: &platform_wallet::PlatformWalletError) -> bool {
    matches!(
        error,
        platform_wallet::PlatformWalletError::Sdk(dash_sdk::Error::AlreadyExists(_))
    )
}

/// Platform answered "already taken" for a lock whose block has a ChainLock, and platform-wallet
/// could not keep that as "taken, consumption unknown": its SQLite store does not attest
/// `WALLET_RESTORE`, which that record requires (platform-wallet 575d961,
/// `mark_asset_lock_consumption_unknown` in `asset_lock/tracking.rs:348`, called from
/// `identity/orchestration.rs:379`; BUG-7, seen on testnet). The answer is otherwise lost
/// in a generic "failed to persist state"; Core finality is proved, Platform took the lock.
pub fn is_taken_with_chain_lock(error: &platform_wallet::PlatformWalletError) -> bool {
    matches!(
        error,
        platform_wallet::PlatformWalletError::Persistence(why)
            if why.contains("asset-lock reconciliation requires persistence capabilities")
    )
}

/// The answer when a name was not started because the identity's credits could not be read
/// (BUG-97); the core passes it on as is (`identity_failure`, `core_failure`).
pub fn name_credits_unknown_text() -> &'static str {
    tr!("The identity's credits are not read yet: nothing is charged until they are")
}

/// Whether a name may start (BUG-97): `needed` duffs ([`evolite_api::Fees::name_credits_needed`])
/// against `credits`, in credits as Platform keeps them. Unknown credits refuse as well: the
/// preorder would be charged at the risk of a domain refused. The word "insufficient" makes it
/// `CoreError::NotEnough`, as the same words from Platform do.
pub fn name_credits_gate(credits: Option<u64>, needed: evolite_api::Duffs) -> Result<(), String> {
    let credits = credits.ok_or(name_credits_unknown_text())?;
    let needed = u64::try_from(needed)
        .unwrap_or(u64::MAX)
        .saturating_mul(1000);
    if credits < needed {
        return Err(format!(
            "insufficient credits for a name: {credits} of {needed}"
        ));
    }
    Ok(())
}

/// `write`, only after [`name_credits_gate`] allows it: nothing is sent otherwise.
pub async fn gated_write<W>(
    credits: Option<u64>,
    needed: evolite_api::Duffs,
    write: W,
) -> Result<(), String>
where
    W: std::future::Future<Output = Result<(), String>>,
{
    name_credits_gate(credits, needed)?;
    write.await
}

/// The answer to a top-up whose lock Platform already took but that cannot be confirmed yet;
/// the core passes it on as is (`identity_failure`).
pub fn awaiting_chain_lock_text() -> &'static str {
    tr!(
        "Platform has already taken these DASH, most likely on the earlier attempt, so the credits should already be on the identity. evolite will confirm it once their block gets a ChainLock, usually within a few minutes: then press “Check again”. No new money is needed"
    )
}

/// The answer to a top-up whose lock Platform proved taken on an earlier attempt: done, but the
/// credits did not grow on this one, so the interface does not say "Credits added". The core
/// passes it on as is (`identity_failure`); the interface shows it as done, not as an error
/// (the same text in `evolite-ui` `screens/identity.rs`, `TOP_UP_SETTLED`).
pub fn top_up_settled_text() -> &'static str {
    tr!(
        "Platform says it took these DASH on an earlier attempt, so the credits are already on the identity. No new money is needed"
    )
}

/// The answer when the identity a cut-short top-up was for is not among the found ones yet.
pub fn top_up_identity_unknown_text() -> &'static str {
    tr!(
        "The identity of this top-up is not found yet — wait until the search for the phrase's identities finishes"
    )
}

/// A Core payment handed to the network.
pub struct Paid {
    pub txid: String,
    pub fee: u64,
    /// What the recipient gets: the amount asked, or with «Max» of chosen coins what is left of
    /// them after the fee (FEAT-0013).
    pub sent: u64,
    /// The broadcaster saw the network accept it; `false`: maybe sent, not resent.
    pub relayed: bool,
}

/// A payment of `amount` to `to`, not yet funded: of final coins only (in a block or
/// InstantSend-locked), as an InstantSend lock of this payment needs them too (R-5).
///
/// FEAT-0040: at `fee_rate` duffs per kB when given (a Core payment to an address, the advanced
/// features on); else the normal rate, the builder's own.
pub fn payment_builder(to: &Address, amount: u64, fee_rate: Option<u64>) -> TransactionBuilder {
    let builder = TransactionBuilder::new()
        .require_final_inputs()
        .add_output(to, amount);
    match fee_rate {
        Some(rate) => builder.set_fee_rate(
            key_wallet::wallet::managed_wallet_info::fee::FeeRate::new(rate),
        ),
        None => builder,
    }
}

/// FEAT-0040 R-7: whether a payment signed with `fee` pays more than the `shown` «at most» —
/// with change up to the dust limit that goes to the fee allowed (the architect: else a payment
/// whose change is dust would be refused each time though nothing changed).
pub fn fee_grew(fee: u64, shown: Option<u64>) -> bool {
    shown.is_some_and(|shown| fee > shown.saturating_add(evolite_api::coin_plan::DUST as u64))
}

/// FEAT-0040 R-7: what a payment whose fee grew past the one shown is refused with.
pub fn fee_changed_text() -> &'static str {
    tr!("The fee changed — check the payment again")
}

/// The coins [`Chain::with_final_coins`] froze, with how many builds hold each.
type Frozen = HashMap<key_wallet::dashcore::OutPoint, u32>;

/// The coins one build froze, thawed when it ends: at once on the normal path
/// ([`Thaw::now`]), and from `drop` when the build's future is dropped or panics — a short task
/// on the runtime, as the wallet's lock is async (SPV item 3, review r1 Should 1). Without one
/// they stayed frozen until a restart.
struct Thaw {
    wallet: Option<PlatformWallet>,
    held: Arc<Mutex<Frozen>>,
    frozen: Vec<key_wallet::dashcore::OutPoint>,
}

impl Thaw {
    async fn now(mut self) {
        if let Some(wallet) = self.wallet.take() {
            thaw_in(&wallet, &self.held, &self.frozen).await;
        }
    }
}

impl Drop for Thaw {
    fn drop(&mut self) {
        let Some(wallet) = self.wallet.take() else {
            return;
        };
        if self.frozen.is_empty() {
            return;
        }
        let (held, frozen) = (self.held.clone(), std::mem::take(&mut self.frozen));
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { thaw_in(&wallet, &held, &frozen).await });
        }
    }
}

async fn thaw_in(
    wallet: &PlatformWallet,
    held: &Mutex<Frozen>,
    frozen: &[key_wallet::dashcore::OutPoint],
) {
    if frozen.is_empty() {
        return;
    }
    let mut state = wallet.state_mut().await;
    let mut held = held.lock().unwrap_or_else(|e| e.into_inner());
    crate::convert::thaw(&mut state.core_wallet, frozen, &mut held);
}

impl Chain {
    /// Writes the address pools of `wallet`'s coin-bearing accounts to the store, with which
    /// addresses are used (BUG-194): platform-wallet writes a pool only when it registers the
    /// account, so the store's table ended at the first gap window (index 0–29), every later
    /// change address unknown to the history after a restart and every used flag lost. The
    /// store upserts the rows and keeps `used` once set. A failed write is journaled; the next
    /// one writes the whole pools again.
    pub async fn save_pools(&self, wallet: &PlatformWallet) {
        let Some(store) = self
            .persister
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            return;
        };
        let pools = crate::convert::pool_entries(&wallet.state().await.core_wallet);
        if pools.is_empty() {
            return;
        }
        let id = wallet.wallet_id();
        let changeset = platform_wallet::changeset::PlatformWalletChangeSet {
            account_address_pools: pools,
            ..Default::default()
        };
        let saved = tokio::task::spawn_blocking(move || {
            use platform_wallet::changeset::PlatformWalletPersistence;
            store.store(id, changeset).map_err(|e| format!("{e:?}"))
        })
        .await;
        if let Ok(Err(e)) = saved {
            crate::diag::platform_failed("save_pools", &e);
        }
    }

    /// BUG-232 review r1 M1: coins key-wallet credited outside a new record
    /// ([`crate::convert::coins_of_sent_outputs`]) to the store, so a restart has them; the store
    /// finds their account by the address (the pools first, [`Self::save_pools`]).
    pub async fn save_coins(&self, id: WalletId, coins: Vec<key_wallet::Utxo>) {
        let Some(store) = self
            .persister
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            return;
        };
        if coins.is_empty() {
            return;
        }
        let changeset = platform_wallet::changeset::PlatformWalletChangeSet {
            core: Some(platform_wallet::changeset::CoreChangeSet {
                new_utxos: coins,
                ..Default::default()
            }),
            ..Default::default()
        };
        let saved = tokio::task::spawn_blocking(move || {
            use platform_wallet::changeset::PlatformWalletPersistence;
            store.store(id, changeset).map_err(|e| format!("{e:?}"))
        })
        .await;
        if let Ok(Err(e)) = saved {
            crate::diag::platform_failed("save_coins", &e);
        }
    }

    /// Whether the InstantLock of `txid` was verified in this run (R-1, BUG-26).
    /// BUG-260: the wallet's stored records, as the core last read them.
    pub fn set_stored_history(
        &self,
        records: Arc<Vec<key_wallet::managed_account::transaction_record::TransactionRecord>>,
    ) {
        *self
            .stored_history
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = records;
    }

    /// BUG-260: the wallet's stored records, as the core last read them.
    pub fn stored_history(
        &self,
    ) -> Arc<Vec<key_wallet::managed_account::transaction_record::TransactionRecord>> {
        self.stored_history
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn instant_lock_verified(&self, txid: &key_wallet::dashcore::Txid) -> bool {
        self.instant_locks.contains(txid)
    }

    /// Runs `build` (one of platform-wallet's own builders) with the coins that are not final
    /// frozen (SPV item 3, [`crate::convert::freeze_unfinal`]), and frees them after, whatever it
    /// answered (AC-T3.4). `keep_trusted`: a payment to a contact may take our own change.
    pub async fn with_final_coins<T>(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        keep_trusted: bool,
        build: impl std::future::Future<Output = T>,
    ) -> T {
        let frozen = {
            let mut state = wallet.state_mut().await;
            let (confirmed, is_verified, is_unverified) =
                crate::convert::finality_counts(&state.core_wallet, |txid| {
                    self.instant_lock_verified(txid)
                });
            let mut held = self.frozen.lock().unwrap_or_else(|e| e.into_inner());
            let frozen = crate::convert::freeze_unfinal(
                &mut state.core_wallet,
                |txid| self.instant_lock_verified(txid),
                keep_trusted,
                &mut held,
            );
            // TECH-finality-journal: counts of coins only. Only a payment to a contact keeps our
            // own change; every other build is an asset lock.
            crate::diag::record(crate::diag::Record::FinalCoins {
                build: if keep_trusted {
                    "contact_pay"
                } else {
                    "asset_lock"
                },
                confirmed,
                is_verified,
                is_unverified,
                frozen: frozen.len() as u32,
            });
            frozen
        };
        // Thawed however the build ends: a panic in it or its future dropped (a timeout, a
        // cancelled task) thaws from the guard's drop (review r1 Should 1).
        let guard = Thaw {
            wallet: Some(wallet.clone()),
            held: self.frozen.clone(),
            frozen,
        };
        let answer = build.await;
        guard.now().await;
        answer
    }

    /// The bytes of `txid`'s InstantLock verified in this run, while kept (BUG-173: the wallet
    /// keeps its own across a restart).
    pub fn instant_lock_bytes(&self, txid: &key_wallet::dashcore::Txid) -> Option<Vec<u8>> {
        self.instant_locks.bytes(txid)
    }

    pub fn status(&self) -> SyncStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// BUG-240 r3 (A): what this run knows of each wallet's pool passes — the last success and
    /// the failures in a row since — for the chain that follows a wallet switch. In memory only:
    /// after a restart a pool is «not known» until a pass (r2).
    pub fn pool_counts(&self) -> PoolCounts {
        let status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        PoolCounts {
            passed: status.pool_passed.clone(),
            failed: status.pool_failed.clone(),
            waited: status.pool_waited.clone(),
        }
    }

    /// BUG-240 r3 (A): takes on what the chain before knew of each wallet's pool passes, so a
    /// wallet switch does not start its count over (QA batch 43: three failures, each after a
    /// switch, each counted as the first).
    pub fn carry_pool_counts(&self, counts: PoolCounts) {
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        status.pool_passed.extend(counts.passed);
        status.pool_failed.extend(counts.failed);
        status.pool_waited.extend(counts.waited);
    }

    /// BUG-162: the open wallet's own synced height (none: no wallet open), which says whether
    /// its coins are known ([`crate::convert::coins_known`]).
    pub fn note_wallet_synced(&self, height: Option<u32>) {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .note_wallet_synced(height);
    }

    /// Opens the store at `store` and restores the wallets saved there, before the SPV client
    /// starts: the manager is in [`Self::manager`] from then on.
    async fn open_store(
        &self,
        paths: &ChainPaths,
        handler: Arc<dyn PlatformEventHandler>,
    ) -> Result<Arc<Manager>, String> {
        let store = paths.store.as_path();
        // One persister per file (BUG-65 r1): an open store is taken, never opened again.
        debug_assert!(!self.store_open(), "the store is open already");
        // The context provider needs the manager's SPV client, the manager needs the SDK:
        // the SDK is built with a placeholder and gets ours once the manager exists.
        let sdk = sdk_builder(crate::network::current())
            .with_context_provider(NotYet)
            .build()
            .map_err(|e| e.to_string())?;
        if let Some(dir) = store.parent() {
            crate::folders::create(dir).map_err(|e| e.to_string())?;
        }
        // The record and the claims are read in `load_from_persistor` below: before the SPV
        // client starts (BUG-77).
        let opened = SqlitePersister::open(SqlitePersisterConfig::new(store.to_path_buf()));
        // Kept through the tries to start again, so the words on screen do not blink (BUG-94).
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .folder_refused = opened
            .as_ref()
            .err()
            .and_then(crate::store_refusal::refused);
        let persister = Arc::new(crate::store::Store::new(
            opened.map_err(|e| e.to_string())?,
            store.to_path_buf(),
        ));
        *self.persister.lock().unwrap_or_else(|e| e.into_inner()) = Some(persister.clone());
        let manager = Arc::new(PlatformWalletManager::new(
            Arc::new(sdk),
            persister,
            handler,
        ));
        // FEAT-0031 C5: the network's contracts, shared by its wallets, beside the SPV data.
        let contracts = self
            .contracts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(|| {
                Arc::new(crate::platform::contracts::ContractCache::new(
                    crate::platform::contracts::path_beside_spv(&paths.spv),
                ))
            })
            .clone();
        manager.sdk().set_context_provider(SpvContextProvider::new(
            manager.spv_arc(),
            self.status.clone(),
            Some(contracts),
        ));
        manager
            .load_from_persistor()
            .await
            .map_err(|e| e.to_string())?;
        // What the store has is shown at once: the SPV client opens the stored headers and
        // filters next, which takes far longer (BUG-25). What needs the client waits for
        // [`Self::running`]; what needs proofs, for the masternode lists.
        *self.manager.lock().await = Some(manager.clone());
        self.store_open.store(true, Ordering::Relaxed);
        Ok(manager)
    }

    /// Opens the store (unless [`Self::open`] did) and starts the SPV client; does nothing when
    /// already running. A failure is in the status and can be retried; it lets the store go.
    pub async fn start(
        &self,
        paths: &ChainPaths,
        publish: Arc<dyn Fn(&SyncStatus) + Send + Sync>,
    ) -> Result<(), String> {
        let _one = self.starting.lock().await;
        if self.running() {
            return Ok(());
        }
        let reporter = Reporter::new(self.status.clone(), publish)
            .with_instant_locks(self.instant_locks.clone())
            .with_pool_retry(self.pool_retry.clone());
        reporter.update(|s| {
            s.phase = Phase::Starting;
            s.error = None;
            s.begin_run();
        });
        let result = async {
            // "Own node": that node alone, or no sync at all — never DNS seeds instead.
            let own = crate::node::load(&paths.core_node);
            reporter.update(|s| s.core_node = own.clone());
            let peer = match &own {
                Some(address) => Some(crate::node::resolve(address).await?),
                None => None,
            };
            let handler: Arc<dyn PlatformEventHandler> = Arc::new(reporter.clone());
            // A store opened ahead (BUG-65) is this wallet's: its handler reports to the same
            // status as `reporter` would.
            let opened = self
                .manager
                .lock()
                .await
                .clone()
                .filter(|_| self.store_open());
            let manager = match opened {
                Some(manager) => manager,
                None => self.open_store(paths, handler).await?,
            };
            let config = client_config(crate::network::dash(), paths.spv.clone(), peer);
            manager
                .spv_arc()
                .start_and_spawn(config, || true)
                .await
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(manager)
        }
        .await;
        match result {
            Ok(manager) => {
                *self.manager.lock().await = Some(manager);
                self.running.store(true, Ordering::Relaxed);
                // TECH-spv-prune: how long the start's storage and masternode replay took.
                let stats = dash_spv::masternode_storage_stats();
                crate::diag::record(crate::diag::Record::SpvStart {
                    storage_ms: stats.storage_ms,
                    load_engine_ms: stats.load_engine_ms,
                    messages: stats.messages as u32,
                    verified: stats.quorums_at_start.0 as u32,
                    unverified: stats.quorums_at_start.1 as u32,
                });
                Ok(())
            }
            Err(error) => {
                self.let_store_go().await;
                reporter.update(|s| {
                    s.phase = Phase::Error;
                    s.error = Some(error.clone());
                });
                Err(error)
            }
        }
    }

    /// TECH-spv-prune review r1 S2: the newest list's quorums (verified, not) once the first
    /// masternode sync of the process completed, the first time it is asked after that.
    pub fn masternode_quorums_once_synced(&self) -> Option<(u32, u32)> {
        let (verified, unverified) = dash_spv::masternode_storage_stats().quorums_synced?;
        (!self.mn_quorums_told.swap(true, Ordering::Relaxed))
            .then_some((verified as u32, unverified as u32))
    }

    /// TECH-spv-prune: the stored masternode messages dash-spv pruned since this was last
    /// asked, if any.
    pub fn masternodes_pruned_since(&self) -> Option<u32> {
        let now = dash_spv::masternode_storage_stats().pruned;
        let before = self.mn_pruned_seen.swap(now, Ordering::Relaxed);
        (now > before).then(|| (now - before) as u32)
    }

    /// Started: the store is open and the SPV client runs.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// The store is open and its wallets loaded, the SPV client maybe still starting: what the
    /// store has (identities, balances, history) can be read.
    pub fn store_open(&self) -> bool {
        self.store_open.load(Ordering::Relaxed)
    }

    /// A store opened for a client that did not start is let go as a stop lets it go,
    /// wallets and all: the next try opens it anew (one persister per file), and the open
    /// wallet goes into that one (BUG-25 r1).
    pub async fn let_store_go(&self) {
        if self.store_open.swap(false, Ordering::Relaxed)
            && let Some(manager) = self.manager.lock().await.take()
        {
            self.let_persister_go();
            tear_down(&manager).await;
        }
    }

    /// The store at `paths` opened alone, ahead of the SPV client (BUG-65): what it has is read
    /// at once while the client still waits for the chain before it to let the shared SPV files
    /// go. [`Self::start`] then takes this store. Nothing when open, running or retired; a
    /// failure leaves the opening to [`Self::start`].
    pub async fn open(
        &self,
        paths: &ChainPaths,
        publish: Arc<dyn Fn(&SyncStatus) + Send + Sync>,
    ) -> Result<(), String> {
        let _one = self.starting.lock().await;
        if self.running() || self.store_open() || self.retired() {
            return Ok(());
        }
        let reporter = Reporter::new(self.status.clone(), publish)
            .with_instant_locks(self.instant_locks.clone())
            .with_pool_retry(self.pool_retry.clone());
        self.open_store(paths, Arc::new(reporter)).await.map(|_| ())
    }

    /// The first half of [`Self::start`] alone: the store at `paths` open, no SPV client.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn open_store_only(&self, paths: &ChainPaths) -> Result<(), String> {
        self.open(paths, Arc::new(|_: &SyncStatus| {})).await
    }

    /// Its core is replaced by another: a start of this chain still waiting in line is not
    /// made (the next chain's start waits behind it).
    pub fn retire(&self) {
        self.retired.store(true, Ordering::Relaxed);
    }

    pub fn retired(&self) -> bool {
        self.retired.load(Ordering::Relaxed)
    }

    /// Sets the sync status as dash-spv would report it (tests: there is no SPV client).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_status(&self, change: impl FnOnce(&mut SyncStatus)) {
        change(&mut self.status.lock().unwrap_or_else(|e| e.into_inner()));
    }

    /// A start is coming (off the core's loop): the status says so at once, and no longer
    /// the error of a try before.
    pub fn mark_starting(&self) {
        #[cfg(any(test, feature = "test-support"))]
        self.starts.fetch_add(1, Ordering::Relaxed);
        let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if !self.running() {
            s.phase = Phase::Starting;
            s.error = None;
        }
    }

    pub async fn manager(&self) -> Result<Arc<Manager>, String> {
        self.manager
            .lock()
            .await
            .clone()
            .ok_or_else(|| "the chain is not started".to_owned())
    }

    /// FEAT-0042: the public key of BIP44 account 0 of Core of wallet `id`, as platform-wallet
    /// keeps it (no private key read): `xpub…` on mainnet, `tpub…` on testnet. `None`: not
    /// loaded yet.
    pub async fn account_xpub(&self, id: &WalletId) -> Option<String> {
        let manager = self.manager().await.ok()?;
        let wallets = manager.wallet_manager_arc();
        let wallets = wallets.read().await;
        let wallet = wallets.get_wallet(id)?;
        let account = wallet.accounts.standard_bip44_accounts.get(&0)?;
        Some(account.account_xpub.to_string())
    }

    /// The wallet of `seed` (FEAT-0041: the phrase's with its passphrase): the one saved in the
    /// store if it is there (no rescan), otherwise a new entry scanned from `birth_height`.
    pub async fn open_wallet(
        &self,
        seed: &[u8; 64],
        birth_height: u32,
    ) -> Result<(WalletId, Arc<PlatformWallet>), String> {
        let manager = self.manager().await?;
        let id = wallet_id_of(seed)?;
        if let Some(wallet) = manager.get_wallet(&id).await {
            return Ok((id, wallet));
        }
        let wallet = manager
            .create_wallet_from_seed_bytes(
                crate::network::dash(),
                seed,
                WalletAccountCreationOptions::Default,
                Some(birth_height),
            )
            .await
            .map_err(|e| e.to_string())?;
        // BUG-162: what the running client did before was for no wallet; this one's own pass
        // decides from now on.
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .wallet_attached();
        Ok((wallet.wallet_id(), wallet))
    }

    /// SPV item 2a: the time in the tip's header (seconds since 1970), while the client runs.
    pub async fn tip_block_time(&self) -> Option<u32> {
        let manager = self.manager().await.ok()?;
        manager.spv_arc().tip_block_time().await
    }

    /// The Core nodes connected now, each as the masternode list knows it.
    pub async fn core_peers(&self) -> Vec<evolite_api::NodeInfo> {
        use platform_wallet::spv::SpvPeerNodeType;
        let Ok(manager) = crate::turn::timed("core_peers_manager", self.manager()).await else {
            return Vec::new();
        };
        let spv = manager.spv_arc();
        let mut peers: Vec<_> = crate::turn::timed("core_peers_connected", spv.connected_peers())
            .await
            .into_iter()
            .map(|p| evolite_api::NodeInfo {
                address: p.address.to_string(),
                role: match p.node_type {
                    SpvPeerNodeType::Unknown => evolite_api::NodeRole::Unknown,
                    SpvPeerNodeType::Normal => evolite_api::NodeRole::Regular,
                    SpvPeerNodeType::Masternode => evolite_api::NodeRole::Masternode,
                    SpvPeerNodeType::Evonode => evolite_api::NodeRole::Evonode,
                },
                banned_until: None,
            })
            .collect();
        peers.sort_by(|a, b| a.address.cmp(&b.address));
        peers
    }

    /// The DAPI nodes the SDK may ask, those held off after a bad answer marked with
    /// until when. Evonodes all: Platform runs on them.
    pub async fn dapi_nodes(&self) -> Vec<evolite_api::NodeInfo> {
        let Ok(sdk) = self.sdk().await else {
            return Vec::new();
        };
        let mut nodes: Vec<_> = sdk
            .address_list()
            .ban_info()
            .into_iter()
            .map(|n| evolite_api::NodeInfo {
                address: n.uri,
                role: evolite_api::NodeRole::Evonode,
                banned_until: n
                    .banned
                    .then(|| n.banned_until.map(|t| t.timestamp()))
                    .flatten(),
            })
            .collect();
        nodes.sort_by(|a, b| a.address.cmp(&b.address));
        nodes
    }

    /// The SDK: Platform queries and state transitions, proofs checked with our quorum keys.
    pub async fn sdk(&self) -> Result<Arc<dash_sdk::Sdk>, String> {
        Ok(self.manager().await?.sdk_arc())
    }

    /// BUG-266: the network's protocol as a proved, signed answer showed it — learned now with
    /// one proved read (the current epoch) when this run has none yet. The SDK learns it from any
    /// proved answer, but a check that read nothing proved before (credits from the store) found
    /// it unknown and said «wait for the sync» long after the sync. `None` only when the read
    /// failed.
    pub async fn known_protocol(&self, sdk: &dash_sdk::Sdk) -> Option<u32> {
        if let Some(known) = sdk.protocol_version_info().verified_version {
            return Some(known);
        }
        if self.ready().is_ok() {
            use dash_sdk::platform::fetch_current_no_parameters::FetchCurrent;
            // Failed: still unknown, and the caller says so.
            let _ = dpp::block::extended_epoch_info::ExtendedEpochInfo::fetch_current(sdk).await;
        }
        sdk.protocol_version_info().verified_version
    }

    /// BUG-269: keeps the picture of a model document just read, to draw by its key: the
    /// renderer otherwise had it only in the run that made the model.
    pub fn keep_workshop_picture(&self, doc: &dpp::document::Document) {
        let Some((key, bytes, deflated)) = crate::platform::workshop_read::model_picture(doc)
        else {
            return;
        };
        let mut kept = self
            .workshop_pictures
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        kept.retain(|p| p.key != key);
        kept.push(WorkshopPicture {
            key,
            bytes: bytes.into(),
            deflated,
        });
        if kept.len() > WORKSHOP_PICTURES {
            kept.remove(0);
        }
    }

    /// BUG-269: a model's picture read this run, by its key: its stored bytes and whether they
    /// are raw deflate.
    pub fn workshop_picture(&self, key: &str) -> Option<(Arc<[u8]>, bool)> {
        self.workshop_pictures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|p| p.key == key)
            .map(|p| (p.bytes.clone(), p.deflated))
    }

    /// Proofs of Platform data can be checked: our masternode lists, and so the quorum keys,
    /// are in. Asking before that fails, and the SDK would hold it against the node it asked.
    pub fn platform_ready(&self) -> bool {
        self.status().masternodes.is_some_and(|m| m.done)
    }

    /// A read of Platform data may go out: [`Self::platform_ready`], else [`NOT_READY`] without
    /// asking anyone. Before the masternode lists are in, our provider has no quorum key, the
    /// proof fails, and the SDK bans the node it asked and the next ones it retries on — on
    /// every start, 15–20 DAPI nodes (BUG-78). Every proved read here starts with it, so a
    /// caller that forgets [`Self::until_platform_ready`] costs nothing.
    pub fn ready(&self) -> Result<(), String> {
        if !self.platform_ready() {
            return Err(NOT_READY.to_owned());
        }
        #[cfg(any(test, feature = "test-support"))]
        self.proof_reads.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Returns once [`Self::platform_ready`].
    pub async fn until_platform_ready(&self) {
        while !self.platform_ready() {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    /// Is this DPNS label free (proved)? `label` is already normalized by [`crate::dpns`].
    pub async fn name_available(&self, label: &str) -> Result<bool, String> {
        // TECH-read-deadlines (audit Д9): as every proved read, not before the lists (BUG-78).
        self.ready()?;
        self.sdk()
            .await?
            .is_dpns_name_available(label)
            .await
            .map_err(|e| e.to_string())
    }

    /// The identity that owns this DPNS label (proved), in base58.
    pub async fn resolve_name(&self, label: &str) -> Result<Option<String>, String> {
        self.ready()?;
        Ok(self
            .sdk()
            .await?
            .resolve_dpns_name(label)
            .await
            .map_err(|e| e.to_string())?
            .map(|id| id.to_string(Encoding::Base58)))
    }

    /// Starts platform-wallet's background sync of Platform address balances (its own thread,
    /// every 30 s). Proofs need the masternode lists: call once [`Self::platform_ready`].
    /// Idempotent.
    pub async fn start_platform_sync(&self) {
        if let Ok(manager) = self.manager().await {
            let sync = manager.platform_address_sync_arc();
            if !sync.is_running() {
                sync.set_interval(std::time::Duration::from_secs(30));
                sync.start();
            }
            // DashPay: contact requests both ways, profiles, payments (every 15 s).
            let dashpay = manager.dashpay_sync_arc();
            if !dashpay.is_running() {
                dashpay.start();
            }
        }
    }

    /// Jembe's gifts held by `owner`, and the models among them not in `known` (proved). The
    /// gifts by the contract's `byOwner` index (`$ownerId`, `$createdAt`), the models one by
    /// one by id: few, and never changing, so each is read once.
    pub async fn jembe_gifts(
        &self,
        owner: Identifier,
        known: &[String],
    ) -> Result<nfts::Found, String> {
        use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
        use dash_sdk::platform::{DocumentQuery, Fetch, FetchMany};
        use dpp::data_contract::DataContract;
        use dpp::data_contract::accessors::v0::DataContractV0Getters;
        use dpp::document::{Document, DocumentV0Getters};
        use dpp::platform_value::Value;

        self.ready()?;
        let sdk = self.sdk().await?;
        let id =
            Identifier::from_string(nfts::CONTRACT, Encoding::Base58).map_err(|e| e.to_string())?;
        let contract = Arc::new(
            DataContract::fetch(&sdk, id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or(tr!("The Jembe NFT contract was not found"))?,
        );
        let issuer = contract.owner_id();
        let base58 = |v: &Identifier| v.to_string(Encoding::Base58);
        let text = |d: &Document, k: &str| {
            d.properties()
                .get(k)
                .and_then(Value::as_text)
                .unwrap_or_default()
                .to_owned()
        };
        let integer =
            |d: &Document, k: &str| d.properties().get(k).and_then(|v| v.as_integer::<u64>());

        let query = DocumentQuery::new(contract.clone(), "gift")
            .map_err(|e| e.to_string())?
            .with_where(WhereClause {
                field: "$ownerId".into(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(owner.to_buffer()),
            })
            .with_order_by(OrderClause {
                field: "$createdAt".into(),
                ascending: true,
            })
            .with_limit(100);
        let found = Document::fetch_many(&sdk, query)
            .await
            .map_err(|e| e.to_string())?;
        let mut gifts = Vec::new();
        for doc in found.into_values().flatten() {
            let Some(model_id) = doc
                .properties()
                .get("modelId")
                .and_then(|v| v.to_identifier().ok())
            else {
                continue;
            };
            gifts.push(nfts::GiftDoc {
                id: base58(&doc.id()),
                model_id: base58(&model_id),
                serial: integer(&doc, "serial").unwrap_or(0) as u32,
                held_by_issuer: doc.owner_id() == issuer,
                transferred_at: doc.transferred_at(),
                price: integer(&doc, "$price"),
            });
        }

        let mut models = Vec::new();
        let wanted: std::collections::BTreeSet<&String> = gifts
            .iter()
            .map(|g| &g.model_id)
            .filter(|m| !known.contains(m))
            .collect();
        for model_id in wanted {
            let mid =
                Identifier::from_string(model_id, Encoding::Base58).map_err(|e| e.to_string())?;
            let query = DocumentQuery::new(contract.clone(), "giftModel")
                .map_err(|e| e.to_string())?
                .with_document_id(&mid);
            let Some(doc) = Document::fetch(&sdk, query)
                .await
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            let bytes = |k: &str| {
                doc.properties()
                    .get(k)
                    .and_then(|v| v.to_binary_bytes().ok())
                    .unwrap_or_default()
            };
            models.push(nfts::ModelDoc {
                id: model_id.clone(),
                name: text(&doc, "name"),
                description: text(&doc, "description"),
                image: bytes("image"),
                image_format: integer(&doc, "imageFormat").unwrap_or(0) as u8,
                image_hash: bytes("imageHash"),
                supply: integer(&doc, "supply").map(|s| s as u32),
            });
        }
        Ok((gifts, models))
    }

    /// The wallet's next unused Platform address (`tdash1…`) and the credits on all its
    /// Platform addresses, as of the last sync.
    pub async fn platform_address(&self, wallet: &PlatformWallet) -> Result<(String, u64), String> {
        let address = wallet
            .platform()
            .next_unused_receive_address(PlatformPaymentAccountKey {
                account: 0,
                key_class: 0,
            })
            .await
            .map_err(|e| e.to_string())?;
        Ok((
            address.to_bech32m_string(crate::network::dash()),
            wallet.platform().total_credits().await,
        ))
    }

    /// Sends `credits` from the wallet's first identity (the one the interface shows), signed
    /// with its TRANSFER key, then reads its balance back.
    pub async fn send_credits(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        me: &Identifier,
        to: CreditPayee,
        credits: u64,
    ) -> Result<(), String> {
        let (from, signer) = Self::signing(wallet, keys, me).await?;
        let id = from.id();
        let identities = wallet.identity();
        match to {
            CreditPayee::Identity(to) => identities
                .transfer_credits_with_external_signer(&id, &to, credits, &signer, None)
                .await
                .map_err(|e| e.to_string())?,
            CreditPayee::Platform(address) => identities
                .transfer_credits_to_addresses_with_external_signer(
                    &id,
                    BTreeMap::from([(address, credits)]),
                    &signer,
                    None,
                )
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())?,
            CreditPayee::Core(address) => identities
                .withdraw_credits_with_external_signer(&id, credits, &address, &signer, None)
                .await
                .map_err(|e| e.to_string())?,
        }
        let _ = identities.refresh_identity(&id).await;
        Ok(())
    }

    pub async fn wallet(&self, id: &WalletId) -> Option<Arc<PlatformWallet>> {
        self.manager().await.ok()?.get_wallet(id).await
    }

    /// The payment to `to`, built and signed, not sent. With `chosen` (FEAT-0013) only those
    /// coins are candidates — largest first, or with `all` every one of them and no change — and
    /// the signed transaction is checked against them before anything is sent (R-6, АРХ-2).
    async fn funded_payment(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: &Address,
        amount: u64,
        chosen: Option<&evolite_api::CoinChoice>,
        fee_rate: Option<u64>,
    ) -> Result<platform_wallet::SignedCoreTransaction, String> {
        use key_wallet::wallet::managed_wallet_info::coin_selection::SelectionStrategy;
        // The candidates are ours to give, and the accounts only keep the books (`true`): left to
        // itself the builder takes any coin marked locked, verified or not (BUG-111).
        let coins = crate::convert::payable_coins(&wallet.state().await.core_wallet, |txid| {
            self.instant_lock_verified(txid)
        });
        // Coins of the mixing account not fully mixed go only alone, chosen by hand (FEAT-0030
        // AC-A6); then that account signs them too, and the change goes to the first source.
        let mut sources = SEND_FUNDING_SOURCES.to_vec();
        let builder = match chosen {
            None => payment_builder(to, amount, fee_rate).add_inputs(coins),
            Some(choice) => {
                let unfinished: Vec<_> = {
                    let state = wallet.state().await;
                    let stored = self.stored_history();
                    crate::convert::unfinished_coins(&state.core_wallet, &stored, |txid| {
                        self.instant_lock_verified(txid)
                    })
                    .into_iter()
                    .map(|(u, _)| u)
                    .filter(|u| {
                        !u.is_locked
                            && crate::convert::coin_is_final(
                                u,
                                |txid: &key_wallet::dashcore::Txid| {
                                    self.instant_lock_verified(txid)
                                },
                            )
                    })
                    .collect()
                };
                let (coins, mixing) =
                    crate::coins::within_apart(coins, unfinished, choice.outpoints())
                        .map_err(|e| crate::coins::refusal(&e))?;
                if mixing {
                    sources.push(
                        key_wallet::wallet::managed_wallet_info::transaction_building::AccountTypePreference::CoinJoin,
                    );
                }
                payment_builder(to, amount, fee_rate)
                    .add_inputs(coins)
                    .set_selection_strategy(if choice.all() {
                        SelectionStrategy::All
                    } else {
                        SelectionStrategy::LargestFirst
                    })
            }
        };
        let signed = wallet
            .core()
            .finalize_transaction_with_options(builder, &sources, 0, keys, true)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(choice) = chosen {
            let tx = signed.transaction();
            let inputs: Vec<_> = tx.input.iter().map(|i| i.previous_output).collect();
            // «Max»: `amount` is the set less the fee shown; less than that means a higher fee.
            let kept = crate::coins::inputs_kept(&inputs, choice.outpoints(), choice.all())
                && paid_to(tx, to) >= amount;
            if !kept {
                // Not sent: the coins it reserved go back at once.
                wallet.core().abandon_transaction(&signed).await;
                return Err(crate::coins::refusal(&crate::coins::ChoiceError::Changed));
            }
        }
        Ok(signed)
    }

    /// Builds, signs (with `keys`) and broadcasts a payment to `to`, of final coins only
    /// ([`crate::convert::payable_coins`]). The choice among them and the reservation of the spent
    /// coins are platform-wallet's; a payment with no clear answer is
    /// reported, never resent.
    pub async fn pay(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: &Address,
        amount: u64,
        chosen: Option<&evolite_api::CoinChoice>,
        (fee_rate, fee_shown): (Option<u64>, Option<u64>),
    ) -> Result<Paid, String> {
        let signed = self
            .funded_payment(wallet, keys, to, amount, chosen, fee_rate)
            .await?;
        // FEAT-0040 R-7 (AC-T-F4): the coins moved since the last look and the fee grew past
        // what it showed — not sent, the coins it reserved back at once.
        if fee_grew(signed.fee(), fee_shown) {
            wallet.core().abandon_transaction(&signed).await;
            return Err(fee_changed_text().to_owned());
        }
        let fee = signed.fee();
        let txid = signed.transaction().txid().to_string();
        let sent = paid_to(signed.transaction(), to);
        match wallet.core().broadcast_finalized_transaction(&signed).await {
            Ok(_) => Ok(Paid {
                txid,
                fee,
                sent,
                relayed: true,
            }),
            Err(platform_wallet::PlatformWalletError::TransactionBroadcastUnconfirmed(_)) => {
                Ok(Paid {
                    txid,
                    fee,
                    sent,
                    relayed: false,
                })
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Builds, signs (with `keys`) and broadcasts a payment of exactly `amount` to `to` from the
    /// mixed coins of `plan` (FEAT-0030 phase A): the CoinJoin account alone, its other coins not
    /// offered (`reservation_only`), at the fee rate that burns the excess
    /// ([`crate::mixed_pay::burning_rate`]). The CoinJoin account has no change address, so
    /// the builder cannot give change there. The signed transaction is checked
    /// ([`crate::mixed_pay::check_signed`]) and abandoned, not sent, if it is not exactly that.
    pub async fn pay_mixed(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: &Address,
        amount: u64,
        plan: &crate::mixed_pay::MixedPlan,
    ) -> Result<Paid, String> {
        use key_wallet::wallet::managed_wallet_info::transaction_building::AccountTypePreference;
        let rate = crate::mixed_pay::burning_rate(to, amount, plan)?;
        let builder = crate::mixed_pay::mixed_builder(to, amount, &plan.coins, rate);
        let signed = wallet
            .core()
            .finalize_transaction_with_options(
                builder,
                &[AccountTypePreference::CoinJoin],
                0,
                keys,
                true,
            )
            .await
            .map_err(|e| e.to_string())?;
        if let Err(why) = crate::mixed_pay::check_signed(signed.transaction(), to, amount, plan) {
            // Not sent: the coins it reserved go back at once.
            wallet.core().abandon_transaction(&signed).await;
            return Err(why);
        }
        let fee = signed.fee();
        let txid = signed.transaction().txid().to_string();
        match wallet.core().broadcast_finalized_transaction(&signed).await {
            Ok(_) => Ok(Paid {
                txid,
                fee,
                sent: amount,
                relayed: true,
            }),
            Err(platform_wallet::PlatformWalletError::TransactionBroadcastUnconfirmed(_)) => {
                Ok(Paid {
                    txid,
                    fee,
                    sent: amount,
                    relayed: false,
                })
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Stops the SPV client and the sync workers; the store and the SPV data stay for the next
    /// start.
    pub async fn stop(&self) {
        let _one = self.starting.lock().await;
        #[cfg(any(test, feature = "test-support"))]
        self.stops.fetch_add(1, Ordering::Relaxed);
        let manager = self.manager.lock().await.take();
        self.running.store(false, Ordering::Relaxed);
        self.store_open.store(false, Ordering::Relaxed);
        if let Some(manager) = manager {
            self.let_persister_go();
            tear_down(&manager).await;
        }
    }

    /// Our handle on the store goes with the manager's (BUG-194 `save_pools`): one kept would
    /// hold the file open, and the next open of it would be refused (one persister per file).
    fn let_persister_go(&self) {
        self.persister
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }
}

/// Lets a manager go with everything it holds: its SPV client and sync workers, then its
/// wallets, so its store is closed when the last handle drops.
async fn tear_down(manager: &Manager) {
    // Everything first, in order: SPV, then the sync workers (Platform addresses, DashPay)
    // that block on this runtime and would panic if it went away under them. Their passes
    // are sealed then: none reads a wallet while it is removed below (R-29: a pass running
    // during the removal panicked "wallet exists in guard").
    let _ = manager.shutdown().await;
    // Then the wallets: the sync workers share the manager's map of them and outlive the
    // manager, and a wallet in it keeps the store open — a deleted wallet's files stayed held
    // until the app quit.
    for id in manager.wallet_ids().await {
        let _ = manager.remove_wallet(&id).await;
    }
}

/// What `tx` pays to `to`.
fn paid_to(tx: &key_wallet::dashcore::Transaction, to: &Address) -> u64 {
    let script = to.script_pubkey();
    tx.output
        .iter()
        .filter(|o| o.script_pubkey == script)
        .map(|o| o.value)
        .sum()
}

/// key-wallet's id of the wallet a seed makes (from its root public key; not secret). FEAT-0041
/// R-12: by the seed, so one phrase with two passphrases is two wallets.
pub fn wallet_id_of(seed: &[u8; 64]) -> Result<WalletId, String> {
    crate::keys_from(seed)
        .map(|wallet| wallet.compute_wallet_id())
        .map_err(|_| "the phrase does not make a wallet".to_owned())
}

/// The SPV client's configuration: `network`, its store at `storage`, and either `own_node`
/// alone (R-14: never DNS seeds instead) or [`CORE_PEERS`] peers found by the client, at most
/// two from one subnet (`evolite-peers.patch`).
pub(crate) fn client_config(
    network: key_wallet::Network,
    storage: std::path::PathBuf,
    own_node: Option<std::net::SocketAddr>,
) -> ClientConfig {
    let mut config = ClientConfig::new(network)
        .with_storage_path(storage)
        .with_user_agent(format!("/evolite:{}/", env!("CARGO_PKG_VERSION")));
    // SPV item 2b: eight peers, not dash-spv's three, so that a few slow or lying ones
    // neither stall nor eclipse us.
    config.max_peers = CORE_PEERS;
    if let Some(addr) = own_node {
        config.add_peer(addr);
        config = config.with_restrict_to_configured_peers(true);
    }
    config
}

/// How many Core peers the SPV client keeps (SPV item 2b).
pub const CORE_PEERS: u32 = 8;

#[cfg(test)]
mod query_index_tests;
#[cfg(test)]
mod tests;
