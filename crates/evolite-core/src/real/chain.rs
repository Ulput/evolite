//! The chain side of the core on platform-wallet: one `PlatformWalletManager` with its SPV client
//! (dash-spv) and a SQLite store (platform-wallet-storage). The manager holds the wallet watch-only
//! — no private keys: the core signs with keys it derives from the phrase while the wallet is
//! open. History, coins and the scan height are on disk, so opening the wallet again picks up
//! where it was instead of rescanning.
//!
//! Proofs of Platform data are checked against quorum keys from this SPV client's own verified
//! masternode list ([`SpvContextProvider`]), not a server's.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
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
use super::sync::{Phase, Reporter, SyncStatus};
use super::{dashpay, identity, nfts};

type Manager = PlatformWalletManager<SqlitePersister>;

/// Where the chain side keeps its files.
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
    status: Arc<Mutex<SyncStatus>>,
    /// What Platform said of top-up locks that the store cannot keep: "already taken" while
    /// the lock has no ChainLock proof yet (platform-wallet then leaves its status as it was).
    /// Until the app closes.
    top_up_notes: Mutex<HashMap<key_wallet::dashcore::OutPoint, TopUpNote>>,
}

impl Default for Chain {
    fn default() -> Self {
        Self {
            manager: tokio::sync::Mutex::new(None),
            status: Arc::new(Mutex::new(SyncStatus {
                network: crate::network::name(),
                ..Default::default()
            })),
            top_up_notes: Mutex::new(HashMap::new()),
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

/// What [`Chain::scan_dashpay`] found for a phrase.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DashPayFound {
    /// The identity (base58), its credits and how many keys it has.
    pub identity: Option<(String, u64, u32)>,
    pub name: Option<String>,
    /// Contacts established both ways.
    pub contacts: u32,
}

/// The identities of `wallet` among all those platform-wallet keeps.
fn of_wallet(
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

/// Of the wallet's identities `(slot, item)`, the one it works as ([`shown_slot`]), with its
/// slot.
fn shown_of<T>(wallet: WalletId, mine: impl Iterator<Item = (u32, T)>) -> Option<(u32, T)> {
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

/// What "the lock is already taken" means for the person, from the identity's credits before
/// and after (proved) and the lock's status after the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlreadyTaken {
    /// The credits grew: the top-up went through, only its answer was lost.
    Credited(u64),
    /// platform-wallet proved the lock's finality by ChainLock and keeps it as "taken,
    /// consumption unknown": nothing is left to finish.
    Settled(u64),
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
            AlreadyTaken::Settled(after.or(before).unwrap_or(0))
        }
        _ => AlreadyTaken::AwaitChainLock,
    }
}

/// How many times a top-up's lock is sent again as a new transition when the network answers
/// "already in the cache" ([`is_cached_duplicate`]).
const SAME_TRANSITION_RETRIES: u16 = 2;

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
/// `mark_asset_lock_consumption_unknown`; BUG-7, seen on testnet). The answer is otherwise lost
/// in a generic "failed to persist state"; Core finality is proved, Platform took the lock.
pub fn is_taken_with_chain_lock(error: &platform_wallet::PlatformWalletError) -> bool {
    matches!(
        error,
        platform_wallet::PlatformWalletError::Persistence(why)
            if why.contains("asset-lock reconciliation requires persistence capabilities")
    )
}

/// The answer to a top-up whose lock Platform already took but that cannot be confirmed yet;
/// the core passes it on as is (`identity_failure`).
pub fn awaiting_chain_lock_text() -> &'static str {
    tr!(
        "Platform has already taken these DASH, most likely on the earlier attempt, so the credits should already be on the identity. evolite will confirm it once their block gets a ChainLock, usually within a few minutes: then press «Check again». No new money is needed"
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

impl Chain {
    pub fn status(&self) -> SyncStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Opens the store, restores the wallets saved there and starts the SPV client; does
    /// nothing when already running. A failure is in the status and can be retried.
    pub async fn start(
        &self,
        paths: &ChainPaths,
        publish: Arc<dyn Fn(&SyncStatus) + Send + Sync>,
    ) -> Result<(), String> {
        let mut slot = self.manager.lock().await;
        if slot.is_some() {
            return Ok(());
        }
        let reporter = Reporter::new(self.status.clone(), publish);
        reporter.update(|s| {
            s.phase = Phase::Starting;
            s.error = None;
        });
        let result = async {
            // "Own node": that node alone, or no sync at all — never DNS seeds instead.
            let own = super::node::load(&paths.core_node);
            reporter.update(|s| s.core_node = own.clone());
            let peer = match &own {
                Some(address) => Some(super::node::resolve(address).await?),
                None => None,
            };
            // The context provider needs the manager's SPV client, the manager needs the SDK:
            // the SDK is built with a placeholder and gets ours once the manager exists.
            let sdk = match crate::network::current() {
                evolite_api::Network::Testnet => SdkBuilder::new_testnet(),
                evolite_api::Network::Mainnet => SdkBuilder::new_mainnet(),
            }
            .with_context_provider(NotYet)
            .build()
            .map_err(|e| e.to_string())?;
            if let Some(dir) = paths.store.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let persister = SqlitePersister::open(SqlitePersisterConfig::new(paths.store.clone()))
                .map_err(|e| e.to_string())?;
            let handler: Arc<dyn PlatformEventHandler> = Arc::new(reporter.clone());
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
                *slot = Some(manager);
                Ok(())
            }
            Err(error) => {
                reporter.update(|s| {
                    s.phase = Phase::Error;
                    s.error = Some(error.clone());
                });
                Err(error)
            }
        }
    }

    pub(super) async fn manager(&self) -> Result<Arc<Manager>, String> {
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
        let Ok(manager) = self.manager().await else {
            return Vec::new();
        };
        let mut peers: Vec<_> = manager
            .spv_arc()
            .connected_peers()
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
        Ok(self
            .sdk()
            .await?
            .resolve_dpns_name(label)
            .await
            .map_err(|e| e.to_string())?
            .map(|id| id.to_string(Encoding::Base58)))
    }

    /// Looks on Platform for the identities of the phrase (slots from 0 until a gap) with their
    /// DPNS names; platform-wallet keeps what it finds in the store. Needs [`Self::platform_ready`].
    pub async fn discover_identities(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<usize, String> {
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
        Ok(found.len())
    }

    /// The slot a new identity takes: one past the highest of the phrase's identities. They are
    /// looked up first, so a slot taken by an identity the store does not know yet is not reused.
    pub async fn next_identity_index(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<u32, String> {
        self.discover_identities(wallet, keys).await?;
        let state = wallet.state().await;
        let taken = of_wallet(&state.identity_manager, wallet.wallet_id())
            .filter_map(|m| m.identity_index)
            .max();
        // A lock still waiting for its identity holds its slot too.
        drop(state);
        let locked = self.unfinished(wallet).await.map(|u| u.identity_index);
        Ok(taken.max(locked).map_or(0, |i| i + 1))
    }

    /// Creates an identity in slot `index` from `amount` duffs of Core funds (an asset lock,
    /// proved by InstantSend or else ChainLock) and registers `label` for it in DPNS.
    /// platform-wallet tracks the asset lock in the store until Platform takes it, so a run cut
    /// short is finished by [`Self::resume_identity`]. An identity made without its name is an
    /// `Err` that says so.
    pub async fn create_identity(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        index: u32,
        label: &str,
        amount: u64,
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
        self.register_name(wallet, keys, index, &id, label).await?;
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
    ) -> Result<String, String> {
        // Platform may have taken the lock before the run was cut: look first.
        self.discover_identities(wallet, keys).await?;
        let id = match self.unfinished(wallet).await {
            Some(lock) if lock.identity_index == index => {
                self.register_identity(
                    wallet,
                    keys,
                    index,
                    AssetLockFunding::FromExistingAssetLock {
                        out_point: lock.out_point,
                        consume_invitation_voucher: false,
                    },
                )
                .await?
            }
            _ => {
                let state = wallet.state().await;
                in_slot(&state.identity_manager, wallet.wallet_id(), index)
                    .map(|m| m.identity.id())
                    .ok_or(tr!("There is no unfinished identity any more"))?
            }
        };
        if let Some(label) = label
            && self.nameless(wallet, index).await.is_some()
        {
            self.register_name(wallet, keys, index, &id, label).await?;
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
    ) -> Result<(), String> {
        let (identity, signer) = Self::first_identity(wallet, keys).await?;
        let id = identity.id();
        wallet
            .identity()
            .register_name_with_external_signer(&id, label, None, &signer)
            .await
            .map_err(|e| e.to_string())?;
        // The name in the store, so the interface shows it at once.
        let _ = wallet.identity().sync_dpns_names(&id).await;
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
            .min_by_key(|l| (l.awaiting_chain_lock, l.identity_index))
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
    /// failure to record it ([`is_taken_with_chain_lock`]): that is settled, not an error.
    async fn top_up(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        id: &Identifier,
        funding: AssetLockFunding,
    ) -> Result<u64, String> {
        let before = Self::stored_credits(wallet, id).await;
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
            // The lock the cached transition spent: the one asked for, or the one just made.
            let out_point = match &funding {
                AssetLockFunding::FromExistingAssetLock { out_point, .. } => Some(*out_point),
                _ => self.unfinished_top_up(wallet).await.map(|u| u.out_point),
            };
            let Some(out_point) = out_point else {
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
                let out_point = match &funding {
                    AssetLockFunding::FromExistingAssetLock { out_point, .. } => Some(*out_point),
                    _ => self.unfinished_top_up(wallet).await.map(|u| u.out_point),
                };
                let Some(out_point) = out_point else {
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
            AlreadyTaken::Settled(balance) => {
                // The store still has the lock as not taken: not shown again until a restart.
                note(TopUpNote::Credited);
                Ok(balance)
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
    ) -> Result<(), String> {
        let mut signer = SimpleSigner::default();
        signer.add_identity_public_keys(identity::new_identity_keys(keys, index)?);
        wallet
            .identity()
            .register_name_with_external_signer(id, label, None, &signer)
            .await
            .map_err(|e| {
                format!(
                    tr!("the identity is made, but the name @{label} was not written: {e}"),
                    label = label,
                    e = e
                )
            })?;
        // The name in the store, so the interface shows it at once.
        let _ = wallet.identity().sync_dpns_names(id).await;
        Ok(())
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

    /// The identity the interface shows (the chosen one, else the lowest slot: see
    /// [`shown_slot`]), with a signer of its keys derived from `keys`. Every write that acts
    /// "as the identity" goes through here, so they all follow the choice.
    pub(super) async fn first_identity(
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

    /// The DashPay profile of any identity (proved), `None` when it has written none.
    pub async fn profile_of(
        &self,
        id: Identifier,
    ) -> Result<Option<platform_wallet::DashPayProfile>, String> {
        let sdk = self.sdk().await?;
        let contract = Arc::new(
            dpp::system_data_contracts::load_system_data_contract(
                dpp::data_contracts::SystemDataContract::Dashpay,
                sdk.version(),
            )
            .map_err(|e| e.to_string())?,
        );
        let docs = sdk
            .get_profiles(&contract, &[id])
            .await
            .map_err(|e| e.to_string())?;
        Ok(docs.get(&id).map(dashpay::profile_from_document))
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

    /// What a phrase brings from DashPay, read from Platform before any wallet is made:
    /// the first identity of its DIP-13 keys (slots 0 to 4, by the hash of each slot's master
    /// key), its credits and keys, its name, and how many contacts it has established (DIP-15:
    /// requests both ways). All proved. Core balances need the wallet's own sync: not here.
    pub async fn scan_dashpay(&self, keys: &key_wallet::Wallet) -> Result<DashPayFound, String> {
        use dash_sdk::platform::types::identity::PublicKeyHash;
        use dash_sdk::platform::{Fetch, FetchMany};
        use dpp::identity::accessors::IdentityGettersV0;
        let sdk = self.sdk().await?;
        let mut identity = None;
        for slot in 0..5 {
            let (_, _, public) = platform_wallet::derive_identity_auth_keypair(
                keys,
                crate::network::dash(),
                slot,
                0,
            )
            .map_err(|e| e.to_string())?;
            let hash = dpp::util::hash::ripemd160_sha256(&public.to_compressed());
            if let Some(found) = dpp::identity::Identity::fetch(&sdk, PublicKeyHash(hash))
                .await
                .map_err(|e| e.to_string())?
            {
                identity = Some(found);
                break;
            }
        }
        let Some(identity) = identity else {
            return Ok(DashPayFound::default());
        };
        let id = identity.id();
        let name = self.name_of(id).await?;
        // Established contacts: a request we sent to them, and one they sent to us.
        let contract = std::sync::Arc::new(
            dpp::system_data_contracts::load_system_data_contract(
                dpp::system_data_contracts::SystemDataContract::Dashpay,
                sdk.version(),
            )
            .map_err(|e| e.to_string())?,
        );
        let requests = |field: &str, order: &str| {
            use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
            dash_sdk::platform::DocumentQuery::new(contract.clone(), "contactRequest")
                .map(|q| {
                    q.with_where(WhereClause {
                        field: field.into(),
                        operator: WhereOperator::Equal,
                        value: dpp::platform_value::Value::Identifier(id.to_buffer()),
                    })
                    .with_order_by(OrderClause {
                        field: order.into(),
                        ascending: true,
                    })
                    .with_limit(100)
                })
                .map_err(|e| e.to_string())
        };
        let sent = requests("$ownerId", "$createdAt")?;
        let got = requests("toUserId", "$createdAt")?;
        let sent = dpp::document::Document::fetch_many(&sdk, sent)
            .await
            .map_err(|e| e.to_string())?;
        let got = dpp::document::Document::fetch_many(&sdk, got)
            .await
            .map_err(|e| e.to_string())?;
        let to: std::collections::BTreeSet<Identifier> = sent
            .into_values()
            .flatten()
            .filter_map(|d| {
                use dpp::document::DocumentV0Getters;
                d.properties().get("toUserId")?.to_identifier().ok()
            })
            .collect();
        let contacts = got
            .into_values()
            .flatten()
            .filter(|d| {
                use dpp::document::DocumentV0Getters;
                to.contains(&d.owner_id())
            })
            .count() as u32;
        Ok(DashPayFound {
            identity: Some((
                id.to_string(Encoding::Base58),
                identity.balance(),
                identity.public_keys().len() as u32,
            )),
            name,
            contacts,
        })
    }

    /// The DPNS name of an identity, if it has one (proved).
    pub async fn name_of(&self, id: Identifier) -> Result<Option<String>, String> {
        let names = self
            .sdk()
            .await?
            .get_dpns_usernames_by_identity(id, Some(1))
            .await
            .map_err(|e| e.to_string())?;
        Ok(names.into_iter().next().map(|n| n.label))
    }

    /// Sends our DashPay contact request to `to`: an encrypted key to pay us with, signed by
    /// the first identity's keys.
    pub async fn add_contact(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: Identifier,
    ) -> Result<(), String> {
        let (from, signer) = Self::first_identity(wallet, keys).await?;
        let crypto = dashpay::SeedCrypto(keys.clone());
        wallet
            .identity()
            .dashpay()
            .send_contact_request_with_external_signer(
                &from.id(),
                &to,
                None,
                AutoAcceptProofSource::None,
                &signer,
                &crypto,
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Answers `from`'s request with ours: the contact is made.
    pub async fn accept_contact(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        from: Identifier,
    ) -> Result<(), String> {
        let (me, signer) = Self::first_identity(wallet, keys).await?;
        let request = {
            let state = wallet.state().await;
            state
                .identity_manager
                .managed_identity(&me.id())
                .and_then(|m| m.dashpay().incoming_contact_requests().get(&from).cloned())
                .ok_or(tr!("There is no request from this identity"))?
        };
        let crypto = dashpay::SeedCrypto(keys.clone());
        wallet
            .identity()
            .dashpay()
            .accept_contact_request_with_external_signer(&request, &signer, &crypto)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Stops showing `from`'s requests (kept in the store on this computer, nothing is written
    /// to Platform).
    pub async fn hide_request(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        from: Identifier,
    ) -> Result<(), String> {
        let (me, _) = Self::first_identity(wallet, keys).await?;
        wallet
            .identity()
            .dashpay()
            .ignore_contact_sender(&me.id(), &from)
            .await
            .map_err(|e| e.to_string())
    }

    /// Writes the first identity's DashPay profile, created or replaced. `avatar`: the
    /// picture's link and its bytes (downloaded by the caller), for the hash in the profile.
    pub async fn save_profile(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        display: String,
        about: String,
        avatar: Option<(String, Vec<u8>)>,
    ) -> Result<(), String> {
        let (me, signer) = Self::first_identity(wallet, keys).await?;
        let has_profile = {
            let state = wallet.state().await;
            state
                .identity_manager
                .managed_identity(&me.id())
                .is_some_and(|m| m.dashpay().profile.is_some())
        };
        let (avatar_url, avatar_bytes) = match avatar {
            Some((url, bytes)) => (Some(url), Some(bytes)),
            None => (None, None),
        };
        let input = ProfileUpdate {
            display_name: Some(display).filter(|s| !s.is_empty()),
            public_message: Some(about).filter(|s| !s.is_empty()),
            avatar_url,
            avatar_bytes,
        };
        let view = wallet.identity();
        let view = view.dashpay();
        let written = if has_profile {
            view.update_profile_with_external_signer(&me.id(), input, &signer)
                .await
        } else {
            view.create_profile_with_external_signer(&me.id(), input, &signer)
                .await
        };
        written.map(|_| ()).map_err(|e| e.to_string())
    }

    /// Builds the accounts of the keys every established contact gave us (DIP-15), all at once:
    /// platform-wallet otherwise builds one only for the first payment to that contact in a run,
    /// so a wallet restored from its phrase named no past payment and counted none.
    /// Only platform-wallet's DashPay sweep queues the builds — every established contact
    /// without an account — and its own sweep may not have run yet when the contacts are read
    /// from the store, so it runs here first; then the queue is decrypted with the phrase's
    /// keys. Answers how many of its contacts the identity the interface shows has an account for.
    pub async fn build_contact_accounts(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<usize, String> {
        let (me, _) = Self::first_identity(wallet, keys).await?;
        let view = wallet.identity().dashpay();
        view.sync_contact_requests()
            .await
            .map_err(|e| e.to_string())?;
        let crypto = dashpay::SeedCrypto(keys.clone());
        view.drain_pending_contact_crypto_verified(&crypto, None)
            .await
            .map_err(|e| e.to_string())?;
        let state = wallet.state().await;
        let me = me.id().to_buffer();
        Ok(state
            .core_wallet
            .accounts
            .dashpay_external_accounts
            .keys()
            .filter(|key| key.user_identity_id == me)
            .count())
    }

    /// Every address of the contacts' keys the wallet holds (DIP-15 accounts built from their
    /// requests), with the contact's identity in base58: a payment out to one of them was a
    /// payment to that contact, also after a restore from the phrase.
    pub async fn contact_key_addresses(
        &self,
        wallet: &PlatformWallet,
    ) -> std::collections::HashMap<String, String> {
        use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
        let state = wallet.state().await;
        let mut found = std::collections::HashMap::new();
        for (key, account) in state.core_wallet.accounts.dashpay_external_accounts.iter() {
            let friend = Identifier::from(key.friend_identity_id).to_string(Encoding::Base58);
            for pool in account.managed_account_type().address_pools() {
                for info in pool.addresses.values() {
                    found.insert(info.address.to_string(), friend.clone());
                }
            }
        }
        found
    }

    /// The address the next payment to contact `to` goes to (DIP-15: the first unused address of
    /// the key its request gave us), without taking it: [`Self::pay_contact`] takes this one.
    /// `paid`: addresses of this key paid in earlier runs, marked used first (platform-wallet
    /// rebuilds the contact's account after a restart without them). `None` while the
    /// contact's account has no unused address yet.
    pub async fn contact_address(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: Identifier,
        paid: &[String],
    ) -> Result<Option<String>, String> {
        let (me, _) = Self::first_identity(wallet, keys).await?;
        // The contact's account is built from its request on first use, as a payment does.
        let crypto = dashpay::SeedCrypto(keys.clone());
        wallet
            .identity()
            .dashpay()
            .drain_pending_contact_crypto_verified(&crypto, None)
            .await
            .map_err(|e| e.to_string())?;
        let key = DashpayAccountKey {
            index: 0,
            user_identity_id: me.id().to_buffer(),
            friend_identity_id: to.to_buffer(),
        };
        use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
        if !paid.is_empty() {
            let mut state = wallet.state_mut().await;
            if let Some(account) = state
                .core_wallet
                .accounts
                .dashpay_external_accounts
                .get_mut(&key)
            {
                let known: Vec<Address> = account
                    .managed_account_type()
                    .address_pools()
                    .iter()
                    .flat_map(|pool| pool.addresses.values())
                    .filter(|info| paid.contains(&info.address.to_string()))
                    .map(|info| info.address.clone())
                    .collect();
                for address in &known {
                    account.mark_address_used(address);
                }
            }
        }
        let state = wallet.state().await;
        // The pool's addresses by index: the first available is the one a payment takes.
        Ok(state
            .core_wallet
            .accounts
            .dashpay_external_accounts
            .get(&key)
            .and_then(|account| {
                let pools = account.managed_account_type().address_pools();
                let pool = pools.first()?;
                pool.addresses
                    .values()
                    .find(|info| info.is_available())
                    .map(|info| info.address.to_string())
            }))
    }

    /// Pays a DashPay contact from Core: to the next address of the key its request gave us,
    /// signed with our keys — and only if that is still `shown`, the address the person saw
    /// before signing. Never resent: when the network gives no clear answer the payment may
    /// have gone, and it is reported so (`relayed: false`, txid unknown), not as a failure.
    pub async fn pay_contact(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: Identifier,
        shown: &str,
        paid: &[String],
        amount: u64,
    ) -> Result<Paid, String> {
        let (me, _) = Self::first_identity(wallet, keys).await?;
        if self
            .contact_address(wallet, keys, to, paid)
            .await?
            .as_deref()
            != Some(shown)
        {
            return Err(
                tr!("The contact's address changed after the check — nothing was sent, check the payment again")
                    .into(),
            );
        }
        let crypto = dashpay::SeedCrypto(keys.clone());
        let sent = wallet
            .identity()
            .dashpay()
            .send_payment(
                &me.id(),
                &to,
                amount,
                None,
                &identity::AssetLockSigner(keys.clone()),
                &crypto,
            )
            .await;
        match sent {
            Ok((txid, _, fee)) => Ok(Paid {
                txid: txid.to_string(),
                fee,
                relayed: true,
            }),
            Err(platform_wallet::PlatformWalletError::TransactionBroadcastUnconfirmed(_)) => {
                Ok(Paid {
                    txid: String::new(),
                    fee: 0,
                    relayed: false,
                })
            }
            Err(e) => Err(e.to_string()),
        }
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
        let builder = TransactionBuilder::new().add_output(to, amount);
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
        if let Some(manager) = self.manager.lock().await.take() {
            // The wallets go first: the sync workers share the manager's map of them and
            // outlive the manager, and a wallet in it keeps the store open — a deleted wallet's
            // files stayed held until the app quit.
            for id in manager.wallet_ids().await {
                let _ = manager.remove_wallet(&id).await;
            }
            // Everything, in order: SPV, then the sync workers (Platform addresses, DashPay)
            // that block on this runtime and would panic if it went away under them.
            let _ = manager.shutdown().await;
        }
    }
}

/// The wallet's history as kept in the store at `store`. platform-wallet writes every record
/// there but does not load them back into the wallet (it would need the raw transactions), and
/// sync goes on from the stored height: without this, history is empty after a restart.
///
/// Read only, on a connection of its own, in the store's own format (`core_transactions`, a
/// bincode `TransactionRecord` per row — platform-wallet-storage's reader for it is not
/// public). A row that does not decode is left out; a store this does not understand gives no
/// history, never a wrong one.
pub fn stored_history(
    store: &std::path::Path,
    wallet: &WalletId,
) -> Result<Vec<key_wallet::managed_account::transaction_record::TransactionRecord>, String> {
    use rusqlite::OpenFlags;
    // The store's own limit on a blob.
    const LIMIT: usize = platform_wallet_storage::SIZE_LIMIT_BYTES;
    let conn = rusqlite::Connection::open_with_flags(
        store,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT record_blob FROM core_transactions \
             WHERE wallet_id = ?1 AND record_blob IS NOT NULL AND length(record_blob) <= ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![wallet.as_slice(), LIMIT as i64], |row| {
            row.get::<_, Vec<u8>>(0)
        })
        .map_err(|e| e.to_string())?;
    let config = bincode::config::standard().with_limit::<LIMIT>();
    Ok(rows
        .filter_map(Result::ok)
        .filter_map(|blob| {
            bincode::serde::decode_from_slice(&blob, config)
                .ok()
                .filter(|(_, used)| *used == blob.len())
                .map(|(record, _)| record)
        })
        .collect())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// BUG-7: which tracked locks hold "The top-up stopped", and which only await a ChainLock.
    #[test]
    fn an_unfinished_top_up_is_a_lock_platform_has_not_taken() {
        use AssetLockFundingType::{IdentityRegistration, IdentityTopUp};
        use AssetLockStatus::*;
        for status in [Built, Broadcast, InstantSendLocked, ChainLocked] {
            assert_eq!(top_up_state(IdentityTopUp, &status, None), Some(false));
        }
        for status in [Consumed, DrainCompleted, RecoveredFromChain] {
            assert_eq!(
                top_up_state(IdentityTopUp, &status, None),
                None,
                "{status:?}"
            );
        }
        assert_eq!(top_up_state(IdentityRegistration, &Built, None), None);
        // "Already taken" without a ChainLock yet: shown as awaiting, not as to be continued.
        assert_eq!(
            top_up_state(
                IdentityTopUp,
                &InstantSendLocked,
                Some(TopUpNote::AwaitingChainLock)
            ),
            Some(true)
        );
        assert_eq!(
            top_up_state(IdentityTopUp, &InstantSendLocked, Some(TopUpNote::Credited)),
            None
        );
        // Its ChainLock proof is kept: nothing left, whatever was noted.
        assert_eq!(
            top_up_state(
                IdentityTopUp,
                &RecoveredFromChain,
                Some(TopUpNote::AwaitingChainLock)
            ),
            None
        );
    }

    /// BUG-7 (seen on testnet): "already taken" for a ChainLocked lock arrives as platform-wallet's
    /// failure to record it; only that failure counts as such.
    #[test]
    fn a_taken_lock_with_a_chain_lock_is_recognised() {
        use platform_wallet::PlatformWalletError as E;
        // As platform-wallet 575d961 words it with evolite's SQLite store (testnet, BUG-7).
        let refused = E::Persistence(
            "asset-lock reconciliation requires persistence capabilities [\"wallet_restore\"] \
             (missing mask 0x80)"
                .into(),
        );
        assert!(is_taken_with_chain_lock(&refused));
        assert!(!is_cached_duplicate(&refused));
        assert!(!is_taken_with_chain_lock(&E::Persistence(
            "disk full".into()
        )));
    }

    /// BUG-7 (seen on testnet): the same lock sent again is "already in the cache" — that alone
    /// is sent again as a new transition; Platform's own verdicts are not.
    #[test]
    fn only_a_cached_duplicate_is_sent_again() {
        use platform_wallet::PlatformWalletError as E;
        let cached = E::Sdk(dash_sdk::Error::AlreadyExists(
            "tx already exists in cache".into(),
        ));
        assert!(is_cached_duplicate(&cached));
        assert!(!is_cached_duplicate(&E::Sdk(dash_sdk::Error::Generic(
            "x".into()
        ))));
        assert!(!is_cached_duplicate(&E::AssetLockAlreadyConsumed(
            Default::default()
        )));
    }

    /// BUG-7: "the lock is already taken" answers by the identity's credits and the lock.
    #[test]
    fn an_already_taken_lock_is_judged_by_the_credits() {
        use AssetLockStatus::*;
        assert_eq!(
            already_taken(Some(100), Some(50_000_100), Some(InstantSendLocked)),
            AlreadyTaken::Credited(50_000_100)
        );
        assert_eq!(
            already_taken(None, Some(7), None),
            AlreadyTaken::Credited(7)
        );
        // Credited before (an earlier sync saw it), no ChainLock yet: wait, do not fail.
        assert_eq!(
            already_taken(Some(100), Some(100), Some(InstantSendLocked)),
            AlreadyTaken::AwaitChainLock
        );
        assert_eq!(
            already_taken(Some(100), None, Some(InstantSendLocked)),
            AlreadyTaken::AwaitChainLock
        );
        // platform-wallet proved it by ChainLock: settled.
        assert_eq!(
            already_taken(Some(100), Some(100), Some(RecoveredFromChain)),
            AlreadyTaken::Settled(100)
        );
        assert_eq!(
            already_taken(Some(100), Some(100), Some(Consumed)),
            AlreadyTaken::Settled(100)
        );
    }

    #[test]
    fn the_creation_steps_follow_the_lock() {
        assert_eq!(creation_step_of(None), 1);
        assert_eq!(creation_step_of(Some(&AssetLockStatus::Built)), 1);
        assert_eq!(creation_step_of(Some(&AssetLockStatus::Broadcast)), 2);
        assert_eq!(
            creation_step_of(Some(&AssetLockStatus::InstantSendLocked)),
            3
        );
        assert_eq!(creation_step_of(Some(&AssetLockStatus::ChainLocked)), 3);
        assert_eq!(creation_step_of(Some(&AssetLockStatus::Consumed)), 4);
    }

    /// The chosen identity is shown while the wallet has it; otherwise the lowest slot.
    #[test]
    fn the_chosen_identity_wins_while_it_exists() {
        let wallet = [7u8; 32];
        assert_eq!(shown_slot(wallet, [2, 0, 1].into_iter()), Some(0));
        choose_identity(wallet, 1);
        assert_eq!(shown_slot(wallet, [2, 0, 1].into_iter()), Some(1));
        // Another wallet keeps its own choice.
        assert_eq!(shown_slot([8u8; 32], [2, 0, 1].into_iter()), Some(0));
        // A choice the wallet no longer has falls back to the lowest.
        assert_eq!(shown_slot(wallet, [0, 2].into_iter()), Some(0));
        assert_eq!(shown_slot(wallet, std::iter::empty()), None);
    }

    /// What a write costs is read from the identity that signed it: the chosen one, not the
    /// lowest slot (the history said 0 spent after choosing another identity).
    #[test]
    fn credits_are_read_from_the_identity_that_signs() {
        let wallet = [9u8; 32];
        let mine = || [(0, "first"), (1, "second"), (2, "third")].into_iter();
        assert_eq!(shown_of(wallet, mine()), Some((0, "first")));
        choose_identity(wallet, 1);
        assert_eq!(shown_of(wallet, mine()), Some((1, "second")));
        assert_eq!(shown_of(wallet, std::iter::empty::<(u32, &str)>()), None);
    }

    /// A Platform read proved with quorum keys from our own SPV masternode list: no trusted
    /// server in the loop. Testnet, read-only, minutes on a first run.
    /// `cargo test -p evolite-core live_spv_proved_read -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and minutes"]
    fn live_spv_proved_read() {
        // GroveDB proof checks recurse deeply: big stacks, as the core's own thread has.
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_stack_size(8 << 20)
                    .build()
                    .unwrap()
                    .block_on(proved_read())
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Jembe's gifts of a holder on testnet (`EVOLITE_TEST_NAME` in `.env.test.local`, who got
    /// copies at the release): read with proofs, every picture checked against its `imageHash`.
    /// Read-only, no phrase.
    /// `cargo test -p evolite-core live_jembe_gifts -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and minutes"]
    fn live_jembe_gifts() {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_stack_size(8 << 20)
                    .build()
                    .unwrap()
                    .block_on(jembe_gifts_of_a_holder())
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn jembe_gifts_of_a_holder() {
        let dir = std::env::temp_dir().join("evolite-live-provider");
        let paths = ChainPaths {
            spv: dir.join("spv"),
            store: dir.join("platform-wallet.db"),
            core_node: dir.join("core-node.txt"),
        };
        let chain = Chain::default();
        chain
            .start(&paths, Arc::new(|_: &SyncStatus| {}))
            .await
            .expect("chain starts");
        let started = Instant::now();
        while !chain.platform_ready() {
            assert!(
                started.elapsed() < Duration::from_secs(30 * 60),
                "not ready"
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let name = crate::test_name("EVOLITE_TEST_NAME");
        let holder = chain
            .resolve_name(&name)
            .await
            .expect("proved read")
            .expect("the holder's name is registered");
        let id = Identifier::from_string(&holder, Encoding::Base58).unwrap();
        let (gifts, models) = chain.jembe_gifts(id, &[]).await.expect("gifts read");
        println!(
            "{name} = {holder}: {} gifts, {} models",
            gifts.len(),
            models.len()
        );
        assert!(!gifts.is_empty(), "{name} got gifts at the release");
        for model in &models {
            let (art, stored, verified) = nfts::picture(model);
            println!(
                "  {} (format {}): {} B packed, {}, hash {}",
                model.name,
                model.image_format,
                stored.bytes.len(),
                if verified { "verified" } else { "NOT matching" },
                &art[..12]
            );
            assert!(verified, "{} matches its imageHash", model.name);
        }
        for gift in &gifts {
            println!(
                "  gift {} № {} of {}, transferred at {:?}, price {:?}",
                &gift.id[..8],
                gift.serial,
                gift.model_id.chars().take(8).collect::<String>(),
                gift.transferred_at,
                gift.price
            );
            assert!(models.iter().any(|m| m.id == gift.model_id));
            // Only the issuer creates gifts: one the holder has was handed over.
            assert!(!gift.held_by_issuer);
        }
        // Asked again with the models known: none read twice.
        let known: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
        let (again, none) = chain.jembe_gifts(id, &known).await.expect("gifts read");
        assert_eq!((again.len(), none.len()), (gifts.len(), 0));
    }

    async fn proved_read() {
        let dir = std::env::temp_dir().join("evolite-live-provider");
        let paths = ChainPaths {
            spv: dir.join("spv"),
            store: dir.join("platform-wallet.db"),
            core_node: dir.join("core-node.txt"),
        };
        let chain = Chain::default();
        chain
            .start(&paths, Arc::new(|_: &SyncStatus| {}))
            .await
            .expect("chain starts");
        let started = Instant::now();
        loop {
            let s = chain.status();
            if s.masternodes.is_some_and(|m| m.done) {
                println!(
                    "masternode lists ready in {}s: {s:?}",
                    started.elapsed().as_secs()
                );
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30 * 60),
                "not ready: {s:?}"
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        assert!(chain.platform_ready());
        let dash = chain.resolve_name("dash").await.expect("proved read");
        println!("dash.dash -> {dash:?}");
        assert!(dash.is_some(), "dash.dash is registered on testnet");
        assert_eq!(chain.name_available("dash").await, Ok(false));
        assert_eq!(
            chain.resolve_name("evolite-probe-surely-free-42").await,
            Ok(None)
        );
        assert_eq!(
            chain.name_available("evolite-probe-surely-free-42").await,
            Ok(true)
        );
        let sdk = chain.sdk().await.unwrap();
        println!("protocol version {}", sdk.protocol_version_number());
        chain.stop().await;
    }

    /// A value from `.env.test.local` at the workspace root (never committed, never printed).
    fn test_env(key: &str) -> Option<zeroize::Zeroizing<String>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env.test.local");
        let text = zeroize::Zeroizing::new(std::fs::read_to_string(path).ok()?);
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .map(|(_, v)| zeroize::Zeroizing::new(v.trim().trim_matches('"').to_owned()))
    }

    /// Starts the chain on `dir`, opens the test wallet, waits until coins and proofs are
    /// known.
    async fn open_test_wallet(
        dir: &std::path::Path,
    ) -> (Chain, Arc<PlatformWallet>, key_wallet::Wallet) {
        let phrase =
            test_env("EVOLITE_TEST_PHRASE").expect("EVOLITE_TEST_PHRASE in .env.test.local");
        let paths = ChainPaths {
            // Headers and masternode lists from the other live tests: not fetched again.
            spv: std::env::temp_dir()
                .join("evolite-live-send")
                .join("spv")
                .join("testnet"),
            store: dir
                .join("wallets")
                .join("testnet")
                .join("platform-wallet.db"),
            core_node: dir.join("core-node.txt"),
        };
        let chain = Chain::default();
        chain
            .start(&paths, Arc::new(|_: &SyncStatus| {}))
            .await
            .expect("chain starts");
        let (_, wallet) = chain.open_wallet(&phrase, 0).await.expect("wallet opens");
        let keys = super::super::keys_of(&phrase).expect("keys");
        let started = Instant::now();
        loop {
            let s = chain.status();
            let filters = s.filters.is_some_and(|f| f.done);
            if filters && chain.platform_ready() {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(40 * 60),
                "not ready: {s:?}"
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        (chain, wallet, keys)
    }

    /// An identity registration cut short after its money was locked is found again after a
    /// restart (platform-wallet keeps the lock in its store) and finished with its name,
    /// without locking more. Run it twice, as two processes are two runs of the app: the first
    /// locks 0.013 tDASH of the test phrase and stops; the second finds the lock and finishes.
    /// `cargo test -p evolite-core live_resume_identity -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_resume_identity_after_a_restart() {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_stack_size(8 << 20)
                    .build()
                    .unwrap()
                    .block_on(resume_after_restart())
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn resume_after_restart() {
        let dir = std::env::temp_dir().join("evolite-live-resume");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let label = format!("evolite-{}", stamp % 100_000_000 + 22_222_222);

        let (chain, wallet, keys) = open_test_wallet(&dir).await;
        chain.discover_identities(&wallet, &keys).await.unwrap();
        let Some(unfinished) = chain.unfinished(&wallet).await else {
            // The first run: the money is locked, then the app "falls".
            let index = chain.next_identity_index(&wallet, &keys).await.unwrap();
            let (_, _, out_point) = wallet
                .asset_locks()
                .create_funded_asset_lock_proof(
                    1_300_000,
                    0,
                    AssetLockFundingType::IdentityRegistration,
                    index,
                    &identity::AssetLockSigner(keys.clone()),
                )
                .await
                .expect("asset lock");
            println!("locked {out_point} for slot {index}; run the test again to finish it");
            chain.stop().await;
            return;
        };
        // The second run, on the same files.
        println!("found after a restart: {unfinished:?}");
        let index = unfinished.identity_index;
        let started = Instant::now();
        let id = chain
            .resume_identity(&wallet, &keys, index, Some(&label))
            .await
            .expect("resumed");
        println!(
            "identity {id} with @{label} in {}s",
            started.elapsed().as_secs()
        );
        assert!(
            chain.unfinished(&wallet).await.is_none(),
            "nothing unfinished now"
        );
        assert!(
            chain.nameless(&wallet, index).await.is_none(),
            "the name is there"
        );
        assert_eq!(chain.resolve_name(&label).await, Ok(Some(id)));
        chain.stop().await;
    }

    /// Read only: each identity of the test phrase, its credits as the store of the warm wallet
    /// keeps them and as Platform has them now (a proved balance query). Shows whether the
    /// store lags behind, and what a top-up really added. Opens a copy of
    /// `%TEMP%\evolite-live-warm` when there is one, so nothing is scanned again.
    /// `cargo test -p evolite-core live_identity_balances -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a test phrase in .env.test.local"]
    fn live_identity_balances_against_platform() {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_stack_size(8 << 20)
                    .build()
                    .unwrap()
                    .block_on(balances_against_platform())
            })
            .unwrap()
            .join()
            .unwrap();
    }

    fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }

    async fn balances_against_platform() {
        let dir = std::env::temp_dir().join("evolite-live-balances");
        let warm = std::env::temp_dir().join("evolite-live-warm");
        let _ = std::fs::remove_dir_all(&dir);
        if warm.join("wallets").exists() {
            copy_dir(&warm, &dir).expect("copy of the warm wallet");
        }
        let (chain, wallet, keys) = open_test_wallet(&dir).await;
        chain.discover_identities(&wallet, &keys).await.unwrap();
        let kept: Vec<(u32, Identifier, u64)> = {
            let state = wallet.state().await;
            state
                .identity_manager
                .managed_identities()
                .filter(|m| m.wallet_id == Some(wallet.wallet_id()))
                .filter_map(|m| Some((m.identity_index?, m.identity.id(), m.identity.balance())))
                .collect()
        };
        assert!(!kept.is_empty(), "the test phrase has identities");
        for (slot, id, store) in kept {
            let platform = wallet
                .identity()
                .refresh_identity_balance(&id)
                .await
                .expect("a proved balance");
            println!(
                "slot {slot} {}: store {store}, Platform {platform} credits ({:+})",
                id.to_string(Encoding::Base58),
                platform as i128 - store as i128
            );
        }
        chain.stop().await;
    }

    /// A top-up cut short after its money was locked is found again after a restart and
    /// finished from the same lock, without locking more. Run it twice: the first run locks
    /// 0.001 tDASH for the first identity and stops; the second finds the lock and tops up.
    /// `cargo test -p evolite-core live_resume_top_up -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_resume_top_up_after_a_restart() {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_stack_size(8 << 20)
                    .build()
                    .unwrap()
                    .block_on(resume_top_up_after_restart())
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn resume_top_up_after_restart() {
        let dir = std::env::temp_dir().join("evolite-live-resume-top-up");
        let (chain, wallet, keys) = open_test_wallet(&dir).await;
        chain.discover_identities(&wallet, &keys).await.unwrap();
        let first = || async {
            let state = wallet.state().await;
            state
                .identity_manager
                .managed_identities()
                .filter(|m| m.wallet_id == Some(wallet.wallet_id()))
                .filter_map(|m| Some((m.identity_index?, m.identity.balance())))
                .min_by_key(|(index, _)| *index)
                .expect("the test phrase has an identity")
        };
        let Some(unfinished) = chain.unfinished_top_up(&wallet).await else {
            // The first run: the money is locked, then the app "falls".
            let (index, _) = first().await;
            let (_, _, out_point) = wallet
                .asset_locks()
                .create_funded_asset_lock_proof(
                    100_000,
                    0,
                    AssetLockFundingType::IdentityTopUp,
                    index,
                    &identity::AssetLockSigner(keys.clone()),
                )
                .await
                .expect("asset lock");
            println!("locked {out_point} for identity slot {index}; run the test again");
            chain.stop().await;
            return;
        };
        // The second run, on the same files.
        println!("found after a restart: {unfinished:?}");
        let (_, before) = first().await;
        let started = Instant::now();
        let balance = chain.resume_top_up(&wallet, &keys).await.expect("resumed");
        println!(
            "credits {before} -> {balance} in {}s",
            started.elapsed().as_secs()
        );
        assert!(balance > before, "the credits grew");
        assert!(
            chain.unfinished_top_up(&wallet).await.is_none(),
            "nothing unfinished now"
        );
        chain.stop().await;
    }

    /// BUG-7 on testnet: Platform takes a top-up's lock but the wallet never hears of it (the
    /// answer lost), as on the customer's wallet. "Continue" must not be a generic error: it
    /// says the ChainLock is awaited, a new top-up is not held back, and once the ChainLock
    /// comes "Check again" settles the lock. Spends 0.001 tDASH; up to ~15 min.
    /// `cargo test -p evolite-core live_top_up_taken -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_top_up_taken_behind_the_wallets_back() {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_stack_size(8 << 20)
                    .build()
                    .unwrap()
                    .block_on(top_up_taken_behind())
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn top_up_taken_behind() {
        use dash_sdk::platform::transition::top_up_identity::TopUpIdentity;
        let dir = std::env::temp_dir().join("evolite-live-top-up-taken");
        let (chain, wallet, keys) = open_test_wallet(&dir).await;
        chain.discover_identities(&wallet, &keys).await.unwrap();
        let signer = identity::AssetLockSigner(keys.clone());
        // What the store tracks, for the bug file: every top-up lock and the identities.
        for l in wallet.asset_locks().list_tracked_locks().await {
            println!(
                "tracked {:?} slot {} {} duffs {:?} {}",
                l.funding_type, l.identity_index, l.amount, l.status, l.out_point
            );
        }
        {
            let state = wallet.state().await;
            for m in state
                .identity_manager
                .managed_identities()
                .filter(|m| m.wallet_id == Some(wallet.wallet_id()))
            {
                println!(
                    "identity slot {:?} credits {}",
                    m.identity_index,
                    m.identity.balance()
                );
            }
        }
        // Never resume a lock this test did not make: the test phrase is also a person's
        // wallet, whose stopped top-ups are theirs to finish (BUG-7, werner555). The test's
        // own lock is noted in its folder, so a run cut short can be picked up again.
        let mine = dir.join("test-lock.txt");
        let noted = std::fs::read_to_string(&mine).ok();
        let made = if let Some(stopped) = chain.unfinished_top_up(&wallet).await {
            assert_eq!(
                noted.as_deref().map(str::trim),
                Some(stopped.out_point.to_string().as_str()),
                "an unfinished top-up not made by this test is tracked: not touching it"
            );
            stopped.out_point
        } else {
            let (index, who) = {
                let state = wallet.state().await;
                state
                    .identity_manager
                    .managed_identities()
                    .filter(|m| m.wallet_id == Some(wallet.wallet_id()))
                    .filter_map(|m| Some((m.identity_index?, m.identity.clone())))
                    .min_by_key(|(index, _)| *index)
                    .expect("the test phrase has an identity")
            };
            let (proof, path, out_point) = wallet
                .asset_locks()
                .create_funded_asset_lock_proof(
                    100_000,
                    0,
                    AssetLockFundingType::IdentityTopUp,
                    index,
                    &signer,
                )
                .await
                .expect("asset lock");
            std::fs::write(&mine, out_point.to_string()).expect("note the test's lock");
            // Platform takes the lock behind platform-wallet's back: its answer is "lost".
            let sdk = chain.sdk().await.expect("sdk");
            let credits = who
                .top_up_identity_with_signer(&sdk, proof, &path, &signer, None)
                .await
                .expect("Platform took the lock");
            println!("lock {out_point} taken by Platform behind the wallet: {credits} credits");
            // The credits are already in the store, as after a later sync: they do not grow.
            let _ = wallet.identity().refresh_identity(&who.id()).await;
            out_point
        };
        let stopped = chain
            .unfinished_top_up(&wallet)
            .await
            .expect("the wallet still sees the top-up as stopped");
        println!("stopped: {stopped:?}");
        assert_eq!(
            stopped.out_point, made,
            "only the test's own lock is resumed"
        );
        let started = Instant::now();
        let mut settled = false;
        while started.elapsed() < Duration::from_secs(15 * 60) {
            let answer = chain.resume_top_up(&wallet, &keys).await;
            let now = chain.unfinished_top_up(&wallet).await;
            println!(
                "after {}s: {answer:?}; unfinished {now:?}",
                started.elapsed().as_secs()
            );
            match answer {
                Ok(_) => {
                    assert!(now.is_none(), "a settled lock is not shown");
                    settled = true;
                    break;
                }
                Err(text) => {
                    assert_eq!(text, awaiting_chain_lock_text(), "not a generic error");
                    assert!(
                        now.is_some_and(|u| u.awaiting_chain_lock),
                        "shown as awaiting its ChainLock, not to be continued"
                    );
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
        assert!(settled, "the lock settled once its block had a ChainLock");
        chain.stop().await;
    }

    /// A stopped chain lets go of its store, so a deleted wallet's folder can go at once and
    /// a new wallet be made in the same window. Starts the SPV client, so it is run by
    /// hand: `cargo test -p evolite-core a_stopped_chain -- --ignored`.
    #[test]
    #[ignore = "starts the SPV client, which reaches testnet"]
    fn a_stopped_chain_lets_go_of_the_store() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_stack_size(8 << 20)
            .build()
            .unwrap();
        runtime.block_on(async {
            let dir = std::env::temp_dir().join(format!("evolite-b73-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let paths = ChainPaths {
                spv: dir.join("spv").join("testnet"),
                store: dir.join("wallets").join("testnet").join("platform-wallet.db"),
                core_node: dir.join("core-node.txt"),
            };
            let chain = Chain::default();
            chain
                .start(&paths, Arc::new(|_: &SyncStatus| {}))
                .await
                .expect("chain starts");
            let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
            drop(chain.open_wallet(phrase, 0).await.expect("wallet opens"));
            chain.stop().await;
            assert!(
                std::fs::remove_dir_all(dir.join("wallets")).is_ok(),
                "the store is still held after stop"
            );
            let _ = std::fs::remove_dir_all(&dir);
        });
    }
}
