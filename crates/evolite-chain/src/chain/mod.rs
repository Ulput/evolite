//! The chain side of the core on platform-wallet: one `PlatformWalletManager` with its SPV client
//! (dash-spv) and a SQLite store (platform-wallet-storage). The manager holds the wallet watch-only
//! — no private keys: the core signs with keys it derives from the phrase while the wallet is
//! open. History, coins and the scan height are on disk, so opening the wallet again picks up
//! where it was instead of rescanning.
//!
//! Proofs of Platform data are checked against quorum keys from this SPV client's own verified
//! masternode list ([`SpvContextProvider`]), not a server's.

mod contacts;
mod identities;
mod store_read;

pub use contacts::*;
pub use identities::*;
pub use store_read::*;

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
use simple_signer::signer::SimpleSigner;

use super::provider::{NotYet, SpvContextProvider};
use super::sync::{Phase, Reporter, SyncStatus, VerifiedLocks};
use super::{dashpay, identity, nfts};

/// The SQLite store with the DashPay backfill record and the spent-coin claims it does not keep
/// (BUG-77, [`super::store`]).
type Manager = PlatformWalletManager<super::store::Store<SqlitePersister>>;

/// Where the chain side keeps its files.
#[derive(Clone)]
pub struct ChainPaths {
    /// dash-spv's headers, filters and masternode lists.
    pub spv: PathBuf,
    /// platform-wallet's state: public data only, never keys.
    pub store: PathBuf,
    /// "Own node": Core's only peer, read at the start ([`super::node`]).
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
    /// The store is open and its wallets are loaded: a manager is in `manager`, maybe before
    /// its SPV client starts (BUG-25: that took 12 s in a release build and half a minute or
    /// more in a debug one, and the wallet's identities, balance and history waited for it).
    store_open: AtomicBool,
    /// Its core has been replaced: a start not yet made is not made.
    retired: AtomicBool,
    status: Arc<Mutex<SyncStatus>>,
    /// What Platform said of top-up locks that the store cannot keep: "already taken" while
    /// the lock has no ChainLock proof yet (platform-wallet then leaves its status as it was).
    /// Until the app closes.
    top_up_notes: Mutex<HashMap<key_wallet::dashcore::OutPoint, TopUpNote>>,
    /// Transactions whose InstantLock was verified (R-1): the rest are not shown as InstantSend.
    instant_locks: VerifiedLocks,
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

impl Default for Chain {
    fn default() -> Self {
        Self {
            manager: tokio::sync::Mutex::new(None),
            starting: tokio::sync::Mutex::new(()),
            running: AtomicBool::new(false),
            store_open: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            status: Arc::new(Mutex::new(SyncStatus {
                network: crate::network::name(),
                ..Default::default()
            })),
            top_up_notes: Mutex::new(HashMap::new()),
            instant_locks: VerifiedLocks::default(),
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
    /// The broadcaster saw the network accept it; `false`: maybe sent, not resent.
    pub relayed: bool,
}

/// A payment of `amount` to `to`, not yet funded: of final coins only (in a block or
/// InstantSend-locked), as an InstantSend lock of this payment needs them too (R-5).
pub fn payment_builder(to: &Address, amount: u64) -> TransactionBuilder {
    TransactionBuilder::new()
        .require_final_inputs()
        .add_output(to, amount)
}

impl Chain {
    /// Whether the InstantLock of `txid` was verified in this run (R-1, BUG-26).
    pub fn instant_lock_verified(&self, txid: &key_wallet::dashcore::Txid) -> bool {
        self.instant_locks.contains(txid)
    }

    pub fn status(&self) -> SyncStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Opens the store at `store` and restores the wallets saved there, before the SPV client
    /// starts: the manager is in [`Self::manager`] from then on.
    async fn open_store(
        &self,
        store: &std::path::Path,
        handler: Arc<dyn PlatformEventHandler>,
    ) -> Result<Arc<Manager>, String> {
        // One persister per file (BUG-65 r1): an open store is taken, never opened again.
        debug_assert!(!self.store_open(), "the store is open already");
        // The context provider needs the manager's SPV client, the manager needs the SDK:
        // the SDK is built with a placeholder and gets ours once the manager exists.
        let sdk = match crate::network::current() {
            evolite_api::Network::Testnet => SdkBuilder::new_testnet(),
            evolite_api::Network::Mainnet => SdkBuilder::new_mainnet(),
        }
        .with_context_provider(NotYet)
        .build()
        .map_err(|e| e.to_string())?;
        if let Some(dir) = store.parent() {
            super::folders::create(dir).map_err(|e| e.to_string())?;
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
            .and_then(super::store_refusal::refused);
        let persister =
            super::store::Store::new(opened.map_err(|e| e.to_string())?, store.to_path_buf());
        let manager = Arc::new(PlatformWalletManager::new(
            Arc::new(sdk),
            Arc::new(persister),
            handler,
        ));
        manager.sdk().set_context_provider(SpvContextProvider::new(
            manager.spv_arc(),
            self.status.clone(),
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
            .with_instant_locks(self.instant_locks.clone());
        reporter.update(|s| {
            s.phase = Phase::Starting;
            s.error = None;
            s.begin_run();
        });
        let result = async {
            // "Own node": that node alone, or no sync at all — never DNS seeds instead.
            let own = super::node::load(&paths.core_node);
            reporter.update(|s| s.core_node = own.clone());
            let peer = match &own {
                Some(address) => Some(super::node::resolve(address).await?),
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
                None => self.open_store(&paths.store, handler).await?,
            };
            let mut config = ClientConfig::new(crate::network::dash())
                .with_storage_path(paths.spv.clone())
                .with_user_agent(format!("/evolite:{}/", env!("CARGO_PKG_VERSION")));
            if let Some(addr) = peer {
                config.add_peer(addr);
                config = config.with_restrict_to_configured_peers(true);
            }
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
            .with_instant_locks(self.instant_locks.clone());
        self.open_store(&paths.store, Arc::new(reporter))
            .await
            .map(|_| ())
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

    /// The wallet of `phrase`: the one saved in the store if it is there (no rescan), otherwise
    /// a new watch-only entry scanned from `birth_height`. The phrase only derives public keys
    /// here; platform-wallet keeps no private ones.
    pub async fn open_wallet(
        &self,
        phrase: &str,
        birth_height: u32,
    ) -> Result<(WalletId, Arc<PlatformWallet>), String> {
        let manager = self.manager().await?;
        let id = wallet_id_of(phrase)?;
        if let Some(wallet) = manager.get_wallet(&id).await {
            return Ok((id, wallet));
        }
        let wallet = manager
            .create_wallet_from_mnemonic(
                phrase,
                crate::network::dash(),
                WalletAccountCreationOptions::Default,
                Some(birth_height),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok((wallet.wallet_id(), wallet))
    }

    /// The Core nodes connected now, each as the masternode list knows it.
    pub async fn core_peers(&self) -> Vec<evolite_api::NodeInfo> {
        use platform_wallet::spv::SpvPeerNodeType;
        let Ok(manager) = super::turn::timed("core_peers_manager", self.manager()).await else {
            return Vec::new();
        };
        let spv = manager.spv_arc();
        let mut peers: Vec<_> = super::turn::timed("core_peers_connected", spv.connected_peers())
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
        to: CreditPayee,
        credits: u64,
    ) -> Result<(), String> {
        let (from, signer) = Self::first_identity(wallet, keys).await?;
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

    /// Builds, signs (with `keys`) and broadcasts a payment to `to`. Coin selection and the
    /// reservation of the spent coins are platform-wallet's; a payment with no clear answer is
    /// reported, never resent.
    pub async fn pay(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: &Address,
        amount: u64,
    ) -> Result<Paid, String> {
        let builder = payment_builder(to, amount);
        let signed = wallet
            .core()
            .finalize_transaction(builder, &SEND_FUNDING_SOURCES, 0, keys)
            .await
            .map_err(|e| e.to_string())?;
        let fee = signed.fee();
        let txid = signed.transaction().txid().to_string();
        match wallet.core().broadcast_finalized_transaction(&signed).await {
            Ok(_) => Ok(Paid {
                txid,
                fee,
                relayed: true,
            }),
            Err(platform_wallet::PlatformWalletError::TransactionBroadcastUnconfirmed(_)) => {
                Ok(Paid {
                    txid,
                    fee,
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
            tear_down(&manager).await;
        }
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

/// key-wallet's id of the wallet a phrase makes (from its root public key; not secret).
pub fn wallet_id_of(phrase: &str) -> Result<WalletId, String> {
    let mnemonic = key_wallet::mnemonic::Mnemonic::from_phrase(phrase)
        .map_err(|_| "the phrase does not make a wallet".to_owned())?;
    let wallet = key_wallet::Wallet::from_mnemonic(
        mnemonic,
        crate::network::dash(),
        WalletAccountCreationOptions::Default,
    )
    .map_err(|e| e.to_string())?;
    Ok(wallet.compute_wallet_id())
}

#[cfg(test)]
mod tests;
