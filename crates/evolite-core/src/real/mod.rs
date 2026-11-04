//! The real core on platform-wallet (branch `platform-wallet`): the SPV client, the wallet's
//! state and its SQLite store are platform-wallet's ([`chain`]); the phrase is encrypted on disk
//! (the vault) and the keys are derived from it only while the wallet is open.
//!
//! Core payments are built and broadcast by platform-wallet and signed with those keys. DPNS
//! names are checked and resolved with proofs. What is not wired yet answers
//! `CoreError::NotAvailable` with the reason: the rest of Platform (identity, pool, apps, NFTs). The phrase lives only here: in `pending` between the onboarding steps, in
//! the vault, and as keys in [`Unlocked`]; platform-wallet holds the wallet watch-only.

mod address_book;
mod app_chain;
mod app_connect;
mod app_jobs;
mod avatars;
mod chain;
mod connected_apps;
mod contact_addresses;
mod convert;
mod dashpay;
mod hidden_requests;
mod identity;
mod journal;
mod json_file;
mod mempool;
mod nfts;
mod node;
mod outgoing;
mod pending;
mod pool;
mod pool_history;
mod provider;
mod rates;
mod sync;
mod vault;
mod wallets;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use evolite_api::*;
use key_wallet::mnemonic::{Language, Mnemonic};
use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use key_wallet_manager::WalletId;
use zeroize::Zeroizing;

use crate::dpns;
use chain::{Chain, ChainPaths, CreditPayee};
use convert::{activity, coins_known, fees, not_an_address, payee, platform_fees, sync_info};
use dpp::address_funds::PlatformAddress;
use dpp::identity::accessors::IdentityGettersV0;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use sync::{Phase, SyncStatus};
use vault::{KdfParams, VaultError, VaultMeta};

/// What the real wallet does not do yet (Platform itself is connected, these are not).
const NO_GIFTS: &str =
    tr!("An NFT cannot be given from evolite yet: transfers come in later versions");
const NO_VAULT: &str = tr!("There is no wallet on this computer");
/// The vault's network label; mainnet only behind an explicit flag, which does not exist yet.
/// The network written into the vault: a vault of another network does not open here.
fn network() -> &'static str {
    crate::network::name()
}
/// After a failed start (offline at launch, say) the client is tried again this often.
const RETRY_START: Duration = Duration::from_secs(30);
/// A contact's DPNS name that could not be read is asked again after this long.
const NAME_RETRY: Duration = Duration::from_secs(30);

/// Where the core keeps its files, per network: the SPV storage, platform-wallet's store and
/// the vault.
pub struct Paths {
    /// The data folder itself: "Disk space" counts all of it.
    pub root: PathBuf,
    pub chain: ChainPaths,
    pub vault: PathBuf,
    /// Names asked for identities still being made ([`pending::PendingNames`]).
    pub pending_names: PathBuf,
    /// What the wallet did on Platform ([`journal::Journal`]).
    pub journal: PathBuf,
    /// Addresses of contacts' keys already paid ([`contact_addresses::ContactAddresses`]).
    pub contact_addresses: PathBuf,
    /// Senders whose contact requests were hidden ([`hidden_requests::HiddenRequests`]).
    pub hidden_requests: PathBuf,
    /// The pool's operations as made ([`pool_history::PoolHistory`]).
    pub pool_history: PathBuf,
    /// "Sync again" was asked: [`resync_on_start`] clears at the next start.
    pub resync: PathBuf,
    /// Core addresses the person named ([`address_book::AddressBook`]).
    pub address_book: PathBuf,
    /// When payments out of a block were first seen ([`mempool::MempoolSeen`]).
    pub mempool: PathBuf,
    /// The shielded pool's note store and commitment tree ([`pool`]).
    pub pool: PathBuf,
    /// Apps signed in as an identity ([`connected_apps::ConnectedApps`]).
    pub connected_apps: PathBuf,
}

impl Paths {
    /// The files of wallet `wallet` (its folder in `wallets/<network>/`) and the ones the
    /// wallets of this data folder share (headers and filters, the Core node).
    pub fn new(data_dir: PathBuf, wallet: &str) -> Self {
        let wallets = wallets::wallet_dir(&data_dir, wallet);
        Self {
            root: data_dir.clone(),
            chain: ChainPaths {
                spv: data_dir.join("spv").join(crate::network::name()),
                store: wallets.join("platform-wallet.db"),
                // Per network: a testnet node is never used for mainnet.
                core_node: data_dir.join(match crate::network::current() {
                    evolite_api::Network::Testnet => "core-node.txt",
                    evolite_api::Network::Mainnet => "core-node-mainnet.txt",
                }),
            },
            vault: wallets.join("wallet.json"),
            pending_names: wallets.join("pending-names.json"),
            journal: wallets.join("platform-history.json"),
            contact_addresses: wallets.join("contact-addresses.json"),
            hidden_requests: wallets.join("hidden-requests.json"),
            pool_history: wallets.join("pool-history.json"),
            resync: wallets.join("resync-on-start"),
            address_book: wallets.join("address-book.json"),
            mempool: wallets.join("mempool-seen.json"),
            pool: wallets.join("shielded-pool.db"),
            connected_apps: wallets.join("connected-apps.json"),
        }
    }
}

/// The resync asked for last run, before anything opens: the headers, filters and
/// masternode lists (`spv`) and the wallet's store (with SQLite's `-wal` and `-shm`) go, so
/// sync starts from scratch and the wallet is scanned again from its birth at the unlock. The
/// vault, the names of identities being made, the Platform journal and contacts' addresses
/// stay — none of them comes back from the chain. Returns whether it cleared.
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
    std::fs::remove_file(&paths.resync)?;
    Ok(true)
}

/// The open wallet: its id in platform-wallet, what the vault says about it, and the keys to
/// sign with (derived from the phrase on unlock, dropped on lock).
struct Unlocked {
    id: WalletId,
    meta: VaultMeta,
    keys: Option<key_wallet::Wallet>,
    /// The phrase's BIP-39 seed: the pool derives its spending keys from it for each move.
    #[cfg_attr(not(feature = "pool"), allow(dead_code))]
    seed: Option<Zeroizing<[u8; 64]>>,
}

/// Signing keys from the phrase; key-wallet's `Wallet` signs Core transactions itself.
pub(crate) fn keys_of(phrase: &str) -> Result<key_wallet::Wallet, CoreError> {
    let mnemonic = Mnemonic::from_phrase(phrase).map_err(|_| CoreError::BadPhrase)?;
    key_wallet::Wallet::from_mnemonic(
        mnemonic,
        crate::network::dash(),
        key_wallet::wallet::initialization::WalletAccountCreationOptions::Default,
    )
    .map_err(|_| CoreError::BadPhrase)
}

/// The phrase's BIP-39 seed (no passphrase), zeroed when dropped.
fn seed_of(phrase: &str) -> Result<Zeroizing<[u8; 64]>, CoreError> {
    let mnemonic = Mnemonic::from_phrase(phrase).map_err(|_| CoreError::BadPhrase)?;
    Ok(Zeroizing::new(mnemonic.to_seed("")))
}

fn now() -> Timestamp {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as Timestamp)
}

/// The first address of the wallet's receive chain (BIP44 account 0, external, index 0).
fn first_receive_address(
    wallet: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> Option<String> {
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::managed_account::managed_account_type::ManagedAccountType;
    let account = wallet.accounts.standard_bip44_accounts.get(&0)?;
    match account.managed_account_type() {
        ManagedAccountType::Standard {
            external_addresses, ..
        } => external_addresses
            .address_at_index(0)
            .map(|a| a.to_string()),
        _ => None,
    }
}

/// Bytes of the files under `dir`, however deep; what cannot be read counts as nothing.
fn dir_size(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(t) if t.is_dir() => dir_size(&entry.path()),
            Ok(t) if t.is_file() => entry.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

/// Said when the open wallet is not (or no longer) in platform-wallet.
const NOT_LOADED: &str = tr!("The wallet is not loaded");

/// The open wallet `id` in platform-wallet, or why not, in the interface's terms.
async fn loaded(
    chain: &Chain,
    id: &WalletId,
) -> Result<Arc<platform_wallet::PlatformWallet>, CoreError> {
    chain
        .wallet(id)
        .await
        .ok_or_else(|| CoreError::NotAvailable(NOT_LOADED.into()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct Real {
    paths: Paths,
    /// This core's wallet: its folder in `wallets/<network>/`.
    wallet: String,
    /// The wallets of this computer (the index, checked against the folders when read).
    index: wallets::Index,
    /// How the pending phrase came (new, typed, from DashPay): kept in the index on create.
    pending_origin: wallets::Origin,
    /// The wallet to open next, asked by `SwitchWallet` (or a new one by `AddWallet`): the loop
    /// then makes a new core over it, as after a deletion.
    switch_to: Option<String>,
    /// The password `SwitchWallet` checked against `switch_to`: the new core opens it with it.
    switch_password: Option<Password>,
    /// While a wallet is being added: the one to go back to on "Cancel".
    adding_from: Option<String>,
    chain: Arc<Chain>,
    /// For answers that come after a wait (the network's answer to a payment).
    out: Sender<Event>,
    /// Set by the sync callback; the loop then sends a fresh snapshot.
    dirty: Arc<AtomicBool>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// A phrase between "new / typed" and "create wallet". Never leaves the core but to the
    /// onboarding screen that must show it.
    pending: Option<(Zeroizing<String>, bool)>,
    /// What the vault on disk says about its wallet; read once, set on create.
    saved: Option<VaultMeta>,
    unlocked: Option<Unlocked>,
    /// The wallet was deleted (or a deletion stopped halfway): the loop makes the core anew,
    /// so nothing of the old wallet stays in memory.
    restart: bool,
    /// A deleted wallet's store is still held in this process: no store opens over it, and no
    /// new wallet is made until evolite starts again ([`clear_leftovers`]).
    leftovers: bool,
    /// The next unused address to receive on, and the history length it was taken at: a new
    /// payment may have used it.
    receive_address: Option<(String, usize)>,
    /// "A new address after each payment": off shows the wallet's first address always.
    fresh_receive: bool,
    /// "Fiat rate": the price fetched last, and the task fetching it.
    fiat: Arc<std::sync::Mutex<Option<FiatRate>>>,
    fiat_task: Option<tokio::task::JoinHandle<()>>,
    /// "Disk space": bytes of the data folder, and when they were counted.
    disk: Option<(tokio::time::Instant, u64)>,
    /// The Core and DAPI nodes, and when they were asked.
    nodes: Option<(tokio::time::Instant, Vec<NodeInfo>, Vec<NodeInfo>)>,
    /// When payments still out of a block were first seen, kept between runs.
    mempool: mempool::MempoolSeen,
    /// Lock after this long without a `Touch`; `None`: only when the app closes.
    auto_lock: Option<Duration>,
    last_touch: tokio::time::Instant,
    /// Profile pictures, downloaded and checked here, drawn by `artist`.
    avatars: avatars::Avatars,
    /// The identity's NFTs from Platform, with their pictures.
    nfts: nfts::Nfts,
    /// The sandboxed renderer; `None` in tests.
    artist: Option<crate::art::Artist>,
    /// DPNS names of contacts by identity (base58), asked for once each.
    names: Arc<std::sync::Mutex<HashMap<String, Option<String>>>>,
    /// Asked or being asked; one that failed leaves after [`NAME_RETRY`], to be asked again.
    names_asked: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// The contacts of the last snapshot: who may be paid by name from Core.
    contacts: Vec<Contact>,
    /// An identity is being made or finished: its lock is not "unfinished" meanwhile.
    identity_busy: Arc<AtomicBool>,
    /// The wallet whose shielded pool is connected (or being connected) in this run.
    pool_wallet: Arc<std::sync::Mutex<Option<WalletId>>>,
    /// A pool move (its proof) is under way: one at a time.
    pool_busy: Arc<AtomicBool>,
    /// How many established contacts had their key accounts built this run, and
    /// whether that is under way now.
    contact_accounts_for: Arc<std::sync::atomic::AtomicUsize>,
    contact_accounts_busy: Arc<AtomicBool>,
    /// The phrase's identities were looked up this session: until then a tracked lock may
    /// belong to an identity Platform already made, and is not shown as unfinished.
    identities_found: Arc<AtomicBool>,
    /// Where that lookup stands, for the snapshot.
    identity_search: Arc<std::sync::Mutex<IdentitySearch>>,
    /// The number of the current lookup: a lookup whose number is not current stops.
    discovery_run: Arc<std::sync::atomic::AtomicU64>,
    /// Woken after a write: the identities' credit balances are read from Platform again.
    balances_stale: Arc<tokio::sync::Notify>,
    /// "Check the phrase" asked and not answered yet: the questions and their right words.
    phrase_check: Option<(Vec<PhraseQuestion>, crate::phrase_check::Answers)>,
    /// The wallet's history as the store keeps it (one record per transaction), and when it
    /// was read: how many records the wallet in memory had then.
    stored_history: Vec<key_wallet::managed_account::transaction_record::TransactionRecord>,
    history_read: Option<(usize, tokio::time::Instant)>,
    /// Core payments of this run: who they went to, and those the history does not show yet.
    outgoing: Arc<std::sync::Mutex<outgoing::Outgoing>>,
    /// What the wallet did on Platform, for the history.
    journal: Arc<journal::Journal>,
    /// Addresses of contacts' keys already paid: never paid again, after a restart too.
    contact_paid: Arc<contact_addresses::ContactAddresses>,
    /// Senders whose requests were hidden: still hidden after a resync.
    hidden: Arc<hidden_requests::HiddenRequests>,
    /// The pool's operations with the amounts the scan loses after a restart.
    pool_history: pool_history::PoolHistory,
    /// Core addresses the person named, to pay again.
    address_book: address_book::AddressBook,
    /// The address shown in the last look before paying each contact (by identity): a payment
    /// to a contact is signed only to it.
    previews: Arc<std::sync::Mutex<HashMap<String, String>>>,
    /// Apps signed in as an identity.
    apps: Arc<connected_apps::ConnectedApps>,
    /// App requests shown and not answered yet, by request id.
    app_requests: Arc<std::sync::Mutex<HashMap<u64, AppJob>>>,
    next_app_request: Arc<std::sync::atomic::AtomicU64>,
}

/// What an app's request asks, as checked when its link was opened.
enum AppJob {
    /// `dash-key:`: answer the sign-in.
    SignIn {
        request: app_connect::KeyRequest,
        name: String,
    },
    /// `dash-st:`: add the keys of the sign-in to this app (contract, generation).
    AddKeys { app: connected_apps::App },
}

/// Where a Core payment goes. A name is paid only as a DashPay contact: to the addresses of
/// the key its answer to our request gave us.
enum CorePayee {
    Address(key_wallet::dashcore::Address),
    Contact(Identifier),
}

/// An answer to someone's contact request.
enum ContactJob {
    Accept,
    Hide,
}

/// Why a contact request to `@label` would only repeat one already made, by where its owner
/// stands among our contacts; `None`: send it. A request from them is answered with ours, so it
/// is no reason to hold back.
fn repeat_request(state: Option<ContactState>, label: &str) -> Option<String> {
    match state? {
        ContactState::Established => Some(format!(
            tr!("@{label} is already in your contacts"),
            label = label
        )),
        ContactState::Outgoing => Some(format!(
            tr!(
                "The request to @{label} is already sent — waiting for the answer, no new one is needed"
            ),
            label = label
        )),
        ContactState::Incoming => None,
    }
}

/// How long an import from DashPay waits for the masternode lists before it goes on without
/// the scan. Proofs need them, and on a first start they come only after every header — many
/// minutes: the onboarding goes on, and the identity is found after the wallet's sync instead.
const DASHPAY_SCAN_WAIT: Duration = Duration::from_secs(30);

/// The pause before the contacts' key accounts are tried again while some are missing.
const CONTACT_ACCOUNTS_RETRY: Duration = Duration::from_secs(30);

/// Whether the key accounts of `established` contacts are all there (`accounts` built, `None`
/// when the try failed): only then is the count kept and no more tries made. A try
/// that read no contact's key (the sweep had not queued them) is not success.
fn contact_accounts_done(established: usize, accounts: Option<usize>) -> bool {
    accounts.is_some_and(|built| built >= established)
}

/// What O5 shows for a phrase imported from DashPay: its identity, credits, keys, name
/// and contacts, read from Platform with proofs. `None` — and the onboarding goes straight on —
/// when the phrase has no identity, or Platform could not be read in time.
async fn scan_dashpay(chain: &Chain, keys: &key_wallet::Wallet) -> Option<DashPayScan> {
    tokio::time::timeout(DASHPAY_SCAN_WAIT, chain.until_platform_ready())
        .await
        .ok()?;
    let found = chain.scan_dashpay(keys).await.ok()?;
    dashpay_scan_of(found)
}

/// O5's rows from what [`Chain::scan_dashpay`] found; nothing without an identity. Core
/// balances are not known before the wallet's own sync.
fn dashpay_scan_of(found: chain::DashPayFound) -> Option<DashPayScan> {
    let (id, credits, keys) = found.identity?;
    Some(DashPayScan {
        core: 0,
        core_addresses_used: 0,
        identity_id: Some(id),
        identity_credits: identity::credits_as_duffs(credits),
        identity_keys: keys,
        name: found.name,
        contacts: found.contacts,
        coinjoin: None,
        core_known: false,
    })
}

/// The steps of an identity's creation for "What happens": read once a second from the
/// lock platform-wallet tracks for slot `index`, sent as `Progress { what: "identity" }` (the
/// step out of four) whenever it changes, until the creation ends (`busy` goes down).
fn watch_creation(
    chain: Arc<Chain>,
    wallet: Arc<platform_wallet::PlatformWallet>,
    index: u32,
    busy: Arc<AtomicBool>,
    out: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    tokio::spawn(async move {
        let mut last = 0;
        while busy.load(Ordering::Relaxed) {
            let step = chain.creation_step(&wallet, index).await;
            if step != last {
                last = step;
                let _ = out.send(Event::Progress {
                    what: "identity",
                    fraction: step as f32 / 4.0,
                });
                wake();
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

/// What a Platform write took from the identity's credits, in duffs (1 duff = 1000 credits),
/// when both balances are known and it went down.
fn spent_credits(before: Option<u64>, after: Option<u64>) -> Option<Duffs> {
    let spent = before?.checked_sub(after?)?;
    (spent > 0).then_some((spent / 1000) as Duffs)
}

/// The first slot this computer remembers a name for (`names`, [`pending::PendingNames`])
/// whose identity exists without a DPNS name: made, its name not written.
async fn nameless_slot(
    chain: &Chain,
    wallet: &platform_wallet::PlatformWallet,
    names: &std::collections::BTreeMap<u32, String>,
) -> Option<u32> {
    for slot in names.keys() {
        if chain.nameless(wallet, *slot).await.is_some() {
            return Some(*slot);
        }
    }
    None
}

/// A failed Core payment or asset lock, in the interface's terms.
fn core_failure(text: String) -> CoreError {
    if text.to_lowercase().contains("insufficient") {
        CoreError::NotEnough
    } else {
        CoreError::Failed(text)
    }
}

/// A positive amount of duffs, as the chain takes it.
fn positive_duffs(amount: Duffs) -> Result<u64, CoreError> {
    u64::try_from(amount)
        .ok()
        .filter(|a| *a > 0)
        .ok_or(CoreError::BadAmount)
}

/// A failed DashPay write, in the interface's terms.
fn dashpay_failure(text: String) -> CoreError {
    let lower = text.to_lowercase();
    if lower.contains("insufficient") || lower.contains("balance") {
        CoreError::NotEnough
    } else if text == tr!("The wallet has no identity") {
        CoreError::NotAvailable(text)
    } else {
        CoreError::Failed(text)
    }
}

/// How often the identities' credit balances are read from Platform when nothing asks sooner.
const BALANCES_EVERY: Duration = Duration::from_secs(60);

/// Every how many readings all the identities are read, not only the shown one.
const BALANCES_ALL_EVERY: u32 = 10;

/// A pool that did not connect is tried again after this.
const POOL_RETRY: Duration = Duration::from_secs(60);

/// About how long an Orchard proof takes here, for the progress estimate.
const POOL_PROOF_SECS: f32 = 30.0;

/// A pool move that failed, for the screens; one that may have gone says so and is not retried.
fn pool_error(error: pool::PoolError) -> CoreError {
    match error {
        pool::PoolError::Unconfirmed => CoreError::Failed(
            tr!("Handed to the network, but not confirmed yet — do not repeat it: the next pool check shows the outcome").into(),
        ),
        pool::PoolError::Failed(text) => CoreError::Failed(text),
    }
}

/// A positive amount in credits (1 duff = 1000 credits): a pool move, a credit transfer.
fn positive_credits(amount: Duffs) -> Result<u64, CoreError> {
    positive_duffs(amount)?
        .checked_mul(1000)
        .ok_or(CoreError::BadAmount)
}

/// Keeps the identities' credit balances as Platform has them, while `live` holds:
/// at once, then after each write (`stale`) or every [`BALANCES_EVERY`]. The shown identity is
/// read each time, all of them at first and every [`BALANCES_ALL_EVERY`] times.
async fn keep_balances_fresh(
    chain: Arc<Chain>,
    id: WalletId,
    live: impl Fn() -> bool,
    stale: Arc<tokio::sync::Notify>,
    refresh: impl Fn(),
) {
    let mut round = 0u32;
    while live() {
        let all = round.is_multiple_of(BALANCES_ALL_EVERY);
        round = round.wrapping_add(1);
        if let Some(wallet) = chain.wallet(&id).await
            && chain.refresh_balances(&wallet, all).await.is_ok()
        {
            refresh();
        }
        tokio::select! {
            _ = stale.notified() => {}
            _ = tokio::time::sleep(BALANCES_EVERY) => {}
        }
    }
}

/// The first pause before an identity lookup is tried again, and the longest one.
const DISCOVERY_RETRY: Duration = Duration::from_secs(30);
const DISCOVERY_RETRY_MAX: Duration = Duration::from_secs(5 * 60);

/// The fewest characters a new password may have. The screens ask for more (letters of both
/// cases, a digit, a symbol); the core holds the floor itself, so no other way in sets a
/// password that an offline guess at the vault finds in minutes.
const MIN_PASSWORD: usize = 8;

fn too_short(password: &Password) -> Result<(), CoreError> {
    if password.expose().chars().count() < MIN_PASSWORD {
        return Err(CoreError::NotAvailable(
            tr!("The new password needs at least 8 characters").into(),
        ));
    }
    Ok(())
}

/// The start of this network's Platform and private (Orchard) addresses: `tdash1` on testnet,
/// `dash1` on mainnet.
fn platform_hrp() -> &'static str {
    hrp_of(crate::network::current())
}

fn hrp_of(network: evolite_api::Network) -> &'static str {
    match network {
        evolite_api::Network::Testnet => "tdash1",
        evolite_api::Network::Mainnet => "dash1",
    }
}

/// Whether `to` is a Platform address of this run's network: the same key's address on the
/// other network would take the credits somewhere the person did not mean, and this network's
/// own addresses must work.
fn platform_address_here(to: &str) -> bool {
    platform_address_on(to, crate::network::current())
}

fn platform_address_on(to: &str, network: evolite_api::Network) -> bool {
    matches!(
        PlatformAddress::is_mainnet_bech32m(to),
        Ok(mainnet) if mainnet == (network == evolite_api::Network::Mainnet)
    )
}

/// A random number below `n` from the system's generator: which words of the phrase a check
/// asks must not be guessable.
fn os_dice(n: usize) -> usize {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the OS random generator is available");
    (u64::from_le_bytes(bytes) % n.max(1) as u64) as usize
}

fn set_search(search: &std::sync::Mutex<IdentitySearch>, state: IdentitySearch) {
    if let Ok(mut s) = search.lock() {
        *s = state;
    }
}

/// A failed top-up or name, in the interface's terms. A top-up that fails after its lock loses
/// nothing: the snapshot shows it as unfinished and `ResumeTopUp` finishes it.
fn identity_failure(text: String) -> CoreError {
    if text.to_lowercase().contains("insufficient") {
        CoreError::NotEnough
    } else if text == tr!("The wallet has no identity") {
        CoreError::NotAvailable(tr!("Make an identity first").into())
    } else if text == chain::awaiting_chain_lock_text()
        || text == chain::top_up_identity_unknown_text()
    {
        // Not a failure of the network: what is going on and what to do, as is.
        CoreError::NotAvailable(text)
    } else {
        CoreError::Failed(text)
    }
}

impl Real {
    /// Inside a tokio runtime: the idle timer is a tokio instant.
    fn new(
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
        let contact_paid = Arc::new(contact_addresses::ContactAddresses::new(
            paths.contact_addresses.clone(),
        ));
        let hidden = Arc::new(hidden_requests::HiddenRequests::new(
            paths.hidden_requests.clone(),
        ));
        let pool_history = pool_history::PoolHistory::new(paths.pool_history.clone());
        let paths_apps = paths.connected_apps.clone();
        Self {
            wallet,
            index,
            pending_origin: wallets::Origin::Created,
            switch_to: None,
            switch_password: None,
            adding_from: None,
            journal,
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
            names: Default::default(),
            names_asked: Default::default(),
            contacts: Vec::new(),
            identity_busy: Arc::new(AtomicBool::new(false)),
            pool_wallet: Arc::new(std::sync::Mutex::new(None)),
            pool_busy: Arc::new(AtomicBool::new(false)),
            contact_accounts_for: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            contact_accounts_busy: Arc::new(AtomicBool::new(false)),
            identities_found: Arc::new(AtomicBool::new(false)),
            identity_search: Default::default(),
            discovery_run: Default::default(),
            balances_stale: Default::default(),
            phrase_check: None,
            stored_history: Vec::new(),
            history_read: None,
            outgoing: Default::default(),
            previews: Default::default(),
            apps: Arc::new(connected_apps::ConnectedApps::new(paths_apps)),
            app_requests: Default::default(),
            next_app_request: Default::default(),
        }
    }

    fn pending_names(&self) -> pending::PendingNames {
        pending::PendingNames::new(self.paths.pending_names.clone())
    }

    /// Reads the wallet's history from the store again when the wallet in memory has new
    /// records (`live_count` changed) or 10 s have passed: the store gets them a little after
    /// the wallet does. A read that fails keeps the last one.
    async fn refresh_stored_history(&mut self, id: WalletId, live_count: usize) {
        let fresh = self.history_read.is_some_and(|(count, at)| {
            count == live_count && at.elapsed() < Duration::from_secs(10)
        });
        if fresh {
            return;
        }
        self.history_read = Some((live_count, tokio::time::Instant::now()));
        let store = self.paths.chain.store.clone();
        if let Ok(Ok(records)) =
            tokio::task::spawn_blocking(move || chain::stored_history(&store, &id)).await
        {
            self.stored_history = records;
        }
    }

    /// What a cut-short "Create an identity" left, if anything: a lock without its identity, or
    /// an identity without the name this computer remembers for it. Nothing while one is being
    /// made: its lock exists before Platform has the identity.
    async fn unfinished_identity(
        &self,
        wallet: &platform_wallet::PlatformWallet,
    ) -> Option<UnfinishedIdentity> {
        if self.identity_busy.load(Ordering::Relaxed)
            || !self.identities_found.load(Ordering::Relaxed)
        {
            return None;
        }
        let names = self.pending_names().load();
        if let Some(lock) = self.chain.unfinished(wallet).await {
            return Some(UnfinishedIdentity {
                locked: Some(lock.amount as Duffs),
                name: names.get(&lock.identity_index).cloned(),
            });
        }
        let slot = nameless_slot(&self.chain, wallet, &names).await?;
        Some(UnfinishedIdentity {
            locked: None,
            name: names.get(&slot).cloned(),
        })
    }

    /// Finishes the unfinished identity in the background: no new lock, no new money. The
    /// answer is `Done { what: "identity" }`.
    fn resume_identity(&self) -> Result<(), CoreError> {
        let (id, keys) = self.dashpay_ready()?;
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made").into(),
            ));
        }
        let chain = self.chain.clone();
        let pending = self.pending_names();
        let busy = self.identity_busy.clone();
        let refresh = self.refresher();
        let (out, wake) = (self.out.clone(), self.wake.clone());
        self.answer_later(async move {
            let result = async {
                let wallet = chain.wallet(&id).await.ok_or(NOT_LOADED)?;
                let names = pending.load();
                let index = match chain.unfinished(&wallet).await {
                    Some(lock) => Some(lock.identity_index),
                    None => nameless_slot(&chain, &wallet, &names).await,
                };
                let index = index.ok_or(tr!("There is no unfinished identity"))?;
                watch_creation(
                    chain.clone(),
                    wallet.clone(),
                    index,
                    busy.clone(),
                    out,
                    wake,
                );
                chain
                    .resume_identity(&wallet, &keys, index, names.get(&index).map(String::as_str))
                    .await?;
                pending.remove(index)
            }
            .await
            .map_err(core_failure);
            busy.store(false, Ordering::Relaxed);
            refresh();
            Event::Done {
                what: "identity",
                result,
            }
        });
        Ok(())
    }

    /// What a cut-short top-up left locked on Core, if anything. Nothing while a top-up or an
    /// identity is being made: its lock exists before Platform takes it.
    async fn unfinished_top_up(
        &self,
        wallet: &platform_wallet::PlatformWallet,
    ) -> Option<UnfinishedTopUp> {
        if self.identity_busy.load(Ordering::Relaxed)
            || !self.identities_found.load(Ordering::Relaxed)
        {
            return None;
        }
        self.chain
            .unfinished_top_up(wallet)
            .await
            .map(|lock| UnfinishedTopUp {
                locked: lock.amount as Duffs,
                awaiting_chain_lock: lock.awaiting_chain_lock,
            })
    }

    /// A cut-short top-up that still has to go to Platform (not one Platform already took and
    /// only its ChainLock is awaited): it holds back a new top-up and syncing again.
    async fn top_up_to_finish(&self, wallet: &platform_wallet::PlatformWallet) -> bool {
        self.chain
            .unfinished_top_up(wallet)
            .await
            .is_some_and(|lock| !lock.awaiting_chain_lock)
    }

    /// Checks what can be checked at once and tops up the shown identity in the background
    /// with `amount` duffs from Core (an asset lock, then Platform). The answer is
    /// `Done { what: "top-up" }`.
    async fn top_up_identity(&mut self, amount: Duffs) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let id = open.id;
        let amount = positive_duffs(amount)?;
        let status = self.chain.status();
        if !coins_known(&status, true)
            || status.phase == Phase::Error
            || !self.chain.platform_ready()
        {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        if wallet.balance().total() < amount {
            return Err(CoreError::NotEnough);
        }
        // A cut-short top-up is finished first, with the money already locked for it; one that
        // Platform already took (awaiting its ChainLock) does not hold a new one back.
        if self.top_up_to_finish(&wallet).await {
            return Err(CoreError::NotAvailable(
                tr!("First finish the stopped top-up — its money is already locked").into(),
            ));
        }
        let keys = open
            .keys
            .clone()
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made or topped up").into(),
            ));
        }
        let chain = self.chain.clone();
        let busy = self.identity_busy.clone();
        let refresh = self.refresher();
        self.answer_later(async move {
            let result = chain
                .top_up_identity(&wallet, &keys, amount)
                .await
                .map(|_| ())
                .map_err(identity_failure);
            busy.store(false, Ordering::Relaxed);
            refresh();
            Event::Done {
                what: "top-up",
                result,
            }
        });
        Ok(())
    }

    /// Checks the name at once and registers it for the shown identity in the background (one
    /// more DPNS name, paid from its credits). The answer is `Done { what: "name" }`.
    fn add_name(&self, name: &str) -> Result<(), CoreError> {
        let label = dpns::label(name)?;
        if dpns::is_contested(&label) {
            return Err(CoreError::NotAvailable(
                tr!("Contested names are decided by a masternode vote, which evolite cannot do yet. Add a digit from 2 to 9 to the name or make it longer than 19 characters").into(),
            ));
        }
        let (id, keys) = self.dashpay_ready()?;
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made or topped up").into(),
            ));
        }
        let chain = self.chain.clone();
        let busy = self.identity_busy.clone();
        let refresh = self.refresher();
        let journal = self.journal.clone();
        self.answer_later(async move {
            let result = match chain.name_available(&label).await {
                Ok(false) => Err(CoreError::NotAvailable(format!(
                    tr!("The name @{label} is already taken"),
                    label = label
                ))),
                Err(e) => Err(CoreError::Failed(e)),
                Ok(true) => async {
                    let wallet = chain.wallet(&id).await.ok_or(NOT_LOADED)?;
                    let before = chain.first_identity_credits(&wallet, false).await;
                    chain.add_name(&wallet, &keys, &label).await?;
                    // In the history, with what it cost: the next name is estimated from it.
                    let after = chain.first_identity_credits(&wallet, true).await;
                    journal.add(journal::Entry {
                        at: now(),
                        what: journal::What::Name {
                            label: label.clone(),
                        },
                        spent: spent_credits(before, after).unwrap_or(0),
                        moved: 0,
                    });
                    Ok::<(), String>(())
                }
                .await
                .map_err(identity_failure),
            };
            busy.store(false, Ordering::Relaxed);
            refresh();
            Event::Done {
                what: "name",
                result,
            }
        });
        Ok(())
    }

    /// Finishes the cut-short top-up in the background with the lock already made: no new
    /// money. The answer is `Done { what: "top-up" }`.
    fn resume_top_up(&self) -> Result<(), CoreError> {
        let (id, keys) = self.dashpay_ready()?;
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made or topped up").into(),
            ));
        }
        let chain = self.chain.clone();
        let busy = self.identity_busy.clone();
        let refresh = self.refresher();
        self.answer_later(async move {
            let result = async {
                let wallet = chain.wallet(&id).await.ok_or(NOT_LOADED)?;
                chain.resume_top_up(&wallet, &keys).await
            }
            .await
            .map(|_| ())
            .map_err(identity_failure);
            busy.store(false, Ordering::Relaxed);
            refresh();
            Event::Done {
                what: "top-up",
                result,
            }
        });
        Ok(())
    }

    /// What every DashPay write needs: the open wallet's id and keys, and proofs that can be
    /// checked.
    fn dashpay_ready(&self) -> Result<(WalletId, key_wallet::Wallet), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let keys = open
            .keys
            .clone()
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        if !self.chain.platform_ready() {
            return Err(CoreError::NotSynced);
        }
        Ok((open.id, keys))
    }

    /// Sends our contact request to the owner of `name`, in the background; the answer is
    /// `Done { what: "contact" }`. Not again to someone we already asked or who is a contact:
    /// platform-wallet would write a new request (a key rotation), paid in credits, for nothing.
    fn add_contact(&self, name: &str) -> Result<(), CoreError> {
        let label = dpns::label(name)?;
        let (id, keys) = self.dashpay_ready()?;
        let chain = self.chain.clone();
        let refresh = self.refresher();
        let journal = self.journal.clone();
        let states: HashMap<String, ContactState> = self
            .contacts
            .iter()
            .map(|c| (c.identity_id.clone(), c.state))
            .collect();
        self.answer_later(async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let to = match chain.resolve_name(&label).await {
                    Ok(Some(to)) => to,
                    Ok(None) => return Err(CoreError::NameNotFound),
                    Err(e) => return Err(CoreError::Failed(e)),
                };
                if let Some(why) = repeat_request(states.get(&to).copied(), &label) {
                    return Err(CoreError::NotAvailable(why));
                }
                let to = Identifier::from_string(&to, Encoding::Base58)
                    .map_err(|e| CoreError::Failed(e.to_string()))?;
                let before = chain.first_identity_credits(&wallet, false).await;
                chain
                    .add_contact(&wallet, &keys, to)
                    .await
                    .map_err(dashpay_failure)?;
                journal.add(journal::Entry {
                    at: now(),
                    what: journal::What::ContactRequest {
                        to: format!("@{label}"),
                    },
                    spent: spent_credits(before, chain.first_identity_credits(&wallet, true).await)
                        .unwrap_or(0),
                    moved: 0,
                });
                Ok(())
            }
            .await;
            refresh();
            Event::Done {
                what: "contact",
                result,
            }
        });
        Ok(())
    }

    /// Accepts or hides the request from `identity_id`, in the background.
    fn contact_job(&self, identity_id: &str, job: ContactJob) -> Result<(), CoreError> {
        let from = Identifier::from_string(identity_id, Encoding::Base58)
            .map_err(|e| CoreError::Failed(e.to_string()))?;
        let (id, keys) = self.dashpay_ready()?;
        let chain = self.chain.clone();
        let refresh = self.refresher();
        let journal = self.journal.clone();
        let hidden = self.hidden.clone();
        let sender = identity_id.to_owned();
        // Accepting writes our request back: the history names whom.
        let who = self
            .contacts
            .iter()
            .find(|c| c.identity_id == identity_id)
            .map(Contact::handle)
            .unwrap_or_else(|| identity_id.to_owned());
        self.answer_later(async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                match job {
                    ContactJob::Accept => {
                        let before = chain.first_identity_credits(&wallet, false).await;
                        chain
                            .accept_contact(&wallet, &keys, from)
                            .await
                            .map_err(dashpay_failure)?;
                        let after = chain.first_identity_credits(&wallet, true).await;
                        journal.add(journal::Entry {
                            at: now(),
                            what: journal::What::ContactRequest { to: who },
                            spent: spent_credits(before, after).unwrap_or(0),
                            moved: 0,
                        });
                        Ok(())
                    }
                    ContactJob::Hide => {
                        // Kept outside the store too: a resync removes the store.
                        hidden
                            .add(&sender)
                            .map_err(|e| CoreError::Failed(e.to_string()))?;
                        chain
                            .hide_request(&wallet, &keys, from)
                            .await
                            .map_err(dashpay_failure)
                    }
                }
            }
            .await;
            refresh();
            Event::Done {
                what: "contact",
                result,
            }
        });
        Ok(())
    }

    /// Writes the profile in the background. A picture's link is downloaded first: its hash
    /// goes into the profile, so contacts can check they see the same picture.
    fn save_profile(
        &self,
        display: String,
        about: String,
        avatar_url: Option<String>,
    ) -> Result<(), CoreError> {
        if display.chars().count() > 25 || about.chars().count() > 140 {
            return Err(CoreError::NotAvailable(
                tr!("Name up to 25 characters, «About» up to 140").into(),
            ));
        }
        if avatar_url
            .as_deref()
            .is_some_and(|u| !u.trim().starts_with("https://"))
        {
            return Err(CoreError::NotAvailable(
                tr!("The picture link must start with https://").into(),
            ));
        }
        let (id, keys) = self.dashpay_ready()?;
        let chain = self.chain.clone();
        let refresh = self.refresher();
        let journal = self.journal.clone();
        self.answer_later(async move {
            let result = async {
                let avatar = match avatar_url {
                    Some(url) => {
                        let fetch = url.clone();
                        let bytes = tokio::task::spawn_blocking(move || avatars::download(&fetch))
                            .await
                            .map_err(|e| CoreError::Failed(e.to_string()))?
                            .map_err(|e| {
                                CoreError::NotAvailable(format!(
                                    tr!("The picture did not download: {e}"),
                                    e = e
                                ))
                            })?;
                        Some((url, bytes))
                    }
                    None => None,
                };
                let wallet = loaded(&chain, &id).await?;
                let before = chain.first_identity_credits(&wallet, false).await;
                chain
                    .save_profile(&wallet, &keys, display, about, avatar)
                    .await
                    .map_err(dashpay_failure)?;
                let after = chain.first_identity_credits(&wallet, true).await;
                journal.add(journal::Entry {
                    at: now(),
                    what: journal::What::Profile,
                    spent: spent_credits(before, after).unwrap_or(0),
                    moved: 0,
                });
                Ok(())
            }
            .await;
            refresh();
            Event::Done {
                what: "profile",
                result,
            }
        });
        Ok(())
    }

    /// Something arrived in the background: the next pass sends a fresh snapshot.
    fn refresher(&self) -> impl Fn() + Send + Sync + Clone + 'static {
        let dirty = self.dirty.clone();
        let wake = self.wake.clone();
        move || {
            dirty.store(true, Ordering::Relaxed);
            wake();
        }
    }

    /// The NFTs of the shown identity as last found, and an ask for them in the background
    /// when it is due (proofs must be checkable first).
    fn shown_nfts(&self, identity: Option<&Identity>) -> (Vec<Nft>, NftsStatus) {
        let owner = identity.map(|i| i.id.clone());
        if let Some(owner) = &owner
            && self.chain.platform_ready()
            && self.nfts.due(owner)
        {
            let name = identity
                .and_then(|i| i.names.iter().find(|n| n.primary).or(i.names.first()))
                .map(|n| format!("@{}", n.label))
                .unwrap_or_else(|| owner.chars().take(8).collect());
            let (chain, nfts, refresh) = (self.chain.clone(), self.nfts.clone(), self.refresher());
            let known = nfts.known_models();
            let owner = owner.clone();
            tokio::spawn(async move {
                let result = match Identifier::from_string(&owner, Encoding::Base58) {
                    Ok(id) => chain.jembe_gifts(id, &known).await,
                    Err(e) => Err(e.to_string()),
                };
                nfts.found(&owner, &name, result);
                refresh();
            });
        }
        self.nfts.shown(owner.as_deref())
    }

    /// Asks Platform for the DPNS names of contacts not asked about yet. An ask that failed (no
    /// proofs yet, the network) is asked again half a minute later: without its name a contact
    /// cannot be paid or found by name.
    fn ask_names(&mut self, ids: Vec<String>) {
        for id in ids {
            if !self
                .names_asked
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id.clone())
            {
                continue;
            }
            let chain = self.chain.clone();
            let names = self.names.clone();
            let asked = self.names_asked.clone();
            let refresh = self.refresher();
            tokio::spawn(async move {
                let Ok(identifier) = Identifier::from_string(&id, Encoding::Base58) else {
                    return;
                };
                match chain.name_of(identifier).await {
                    Ok(name) => {
                        names
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(id, name);
                    }
                    Err(_) => {
                        tokio::time::sleep(NAME_RETRY).await;
                        asked.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                    }
                }
                refresh();
            });
        }
    }

    /// Decrypts the vault with `password`; Argon2 runs off the async threads.
    async fn open_vault(
        &self,
        password: &Password,
    ) -> Result<(vault::VaultFile, Zeroizing<String>), CoreError> {
        open_vault_at(&self.paths.vault, password).await
    }

    /// The vault of wallet `id` of this computer: this one's, or another's by its own
    /// password; a wallet not in the index has none.
    async fn open_wallet_vault(
        &self,
        id: &str,
        password: &Password,
    ) -> Result<(vault::VaultFile, Zeroizing<String>), CoreError> {
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
    async fn change_password(&mut self, old: &Password, new: &Password) -> Result<(), CoreError> {
        if new.expose().is_empty() {
            return Err(CoreError::NotAvailable(
                tr!("The new password is empty").into(),
            ));
        }
        too_short(new)?;
        let (file, phrase) = self.open_vault(old).await?;
        let new = Zeroizing::new(new.expose().to_owned());
        let sealed = tokio::task::spawn_blocking(move || {
            vault::seal(&phrase, &new, file.meta, KdfParams::default())
        })
        .await
        .map_err(|_| CoreError::NotAvailable(tr!("The vault was not written").into()))?
        .map_err(vault_error)?;
        vault::save(&self.paths.vault, &sealed).map_err(vault_error)
    }

    /// Removes the wallet from this computer: with `password` it is checked first; then
    /// the keys go, the chain lets go of the store and the vault is deleted — without it the
    /// wallet cannot be opened again. The rest of the wallet's folder (store, names, journal,
    /// contacts' addresses) goes once nothing holds it: the loop makes the core anew and
    /// [`clear_leftovers`] removes it before a store opens. Headers and filters (`spv`) stay:
    /// they hold nothing of the wallet.
    async fn delete_wallet(&mut self, password: Option<&Password>) -> Result<(), CoreError> {
        if self.saved.is_none() {
            return Err(CoreError::NotAvailable(NO_VAULT.into()));
        }
        if let Some(password) = password {
            self.open_vault(password).await?;
        }
        self.lock().await?;
        self.chain.stop().await;
        // The store is closed now: whatever happens next, the core starts again.
        self.restart = true;
        match std::fs::remove_file(&self.paths.vault) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(CoreError::Failed(format!(
                tr!("The wallet was not deleted: {e}"),
                e = e
            ))),
        }
    }

    /// "Sync again": leaves a mark for [`resync_on_start`]; the app starts
    /// again for it. Not while an identity is being made or topped up, nor when one was cut
    /// short: its lock lives in the store, and "Continue" would be lost with it.
    async fn ask_resync(&self) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let unfinished = self.identity_busy.load(Ordering::Relaxed)
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
    async fn lock(&mut self) -> Result<(), CoreError> {
        // An identity lookup still retrying stops and lets go of the keys; the balance
        // reading that follows a lookup stops too.
        self.discovery_run.fetch_add(1, Ordering::Relaxed);
        self.balances_stale.notify_one();
        // The right words of an unanswered phrase check go with the keys.
        self.phrase_check = None;
        // An app's request is not answered after the next unlock with what was shown before.
        self.app_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.unlocked = None;
        self.receive_address = None;
        Ok(())
    }

    /// Asks the network off the loop and sends the answer when it comes. A name asked before
    /// proofs can be checked gets `NotSynced` at once and the real answer once the masternode
    /// lists are in; the interface keeps the last answer for the name it shows.
    fn answer_later(&self, question: impl Future<Output = Event> + Send + 'static) {
        let out = self.out.clone();
        let wake = self.wake.clone();
        let stale = self.balances_stale.clone();
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
    fn idle_too_long(&self) -> bool {
        self.unlocked.is_some()
            && self
                .auto_lock
                .is_some_and(|after| self.last_touch.elapsed() >= after)
    }

    fn publisher(&self) -> Arc<dyn Fn(&SyncStatus) + Send + Sync> {
        let dirty = self.dirty.clone();
        let wake = self.wake.clone();
        Arc::new(move |_: &SyncStatus| {
            dirty.store(true, Ordering::Relaxed);
            wake();
        })
    }

    async fn snapshot(&mut self) -> Snapshot {
        let status = self.chain.status();
        let vault = match (&self.unlocked, &self.saved) {
            (Some(_), _) => VaultState::Unlocked,
            (None, Some(_)) => VaultState::Locked,
            (None, None) => VaultState::NoWallet,
        };
        let wallet = match &self.unlocked {
            Some(open) => Some(self.wallet(open.id, open.meta.clone()).await),
            None => None,
        };
        // DAPI nodes the SDK may ask now (not banned for a bad answer).
        // Asked again every few seconds at most (the masternode list is large).
        let fresh = self
            .nodes
            .as_ref()
            .is_some_and(|(at, _, _)| at.elapsed() < Duration::from_secs(5));
        if !fresh {
            let core = self.chain.core_peers().await;
            let dapi = self.chain.dapi_nodes().await;
            self.nodes = Some((tokio::time::Instant::now(), core, dapi));
        }
        let (core_nodes, dapi_nodes) = self
            .nodes
            .as_ref()
            .map(|(_, c, d)| (c.clone(), d.clone()))
            .unwrap_or_default();
        let platform_nodes = dapi_nodes
            .iter()
            .filter(|n| n.banned_until.is_none())
            .count() as u32;
        Snapshot {
            network: crate::network::current(),
            sync: SyncInfo {
                disk_bytes: self.disk_bytes(),
                platform_nodes,
                core_nodes,
                dapi_nodes,
                core_node_next: {
                    let saved = node::load(&self.paths.chain.core_node);
                    // Empty: back to DNS seeds at the next start.
                    (saved != status.core_node).then(|| saved.unwrap_or_default())
                },
                ..sync_info(&status, self.unlocked.is_some())
            },
            vault,
            wallet,
            demo: false,
            elsewhere: false,
            fiat: self.fiat.lock().map(|f| f.clone()).unwrap_or_default(),
            wallets: self.wallet_entries(),
            adding_wallet: self.adding_from.is_some(),
        }
    }

    /// The wallets of this computer for the switcher, this core's marked open.
    fn wallet_entries(&self) -> Vec<WalletEntry> {
        self.index
            .wallets
            .iter()
            .map(|r| WalletEntry {
                id: r.id.clone(),
                name: r.name.clone(),
                color: r.color,
                origin: match r.origin {
                    wallets::Origin::Created => WalletOrigin::Created,
                    wallets::Origin::Restored => WalletOrigin::Restored,
                    wallets::Origin::DashPay => WalletOrigin::DashPay,
                },
                created_at: r.created_at,
                last_opened: r.last_opened,
                open: r.id == self.wallet,
            })
            .collect()
    }

    /// Writes the index after a change; the folders stay the truth (see [`wallets::load`]).
    fn save_index(&self) {
        let _ = wallets::save(&self.paths.root, &self.index);
    }

    /// The other wallet of this computer the phrase already is (same `WalletId`), if any: a
    /// phrase is one wallet here, not two folders syncing the same keys.
    fn same_wallet(&self, phrase: &str) -> Option<String> {
        let id = chain::wallet_id_of(phrase).ok()?;
        let id = hex(&id);
        self.index
            .wallets
            .iter()
            .filter(|r| r.id != self.wallet)
            .find(|r| {
                vault::load(&wallets::wallet_dir(&self.paths.root, &r.id).join("wallet.json"))
                    .ok()
                    .flatten()
                    .is_some_and(|f| f.meta.wallet_id == id)
            })
            .map(|r| r.name.clone())
    }

    /// "Disk space": the data folder's bytes, counted again once a minute at most.
    fn disk_bytes(&mut self) -> Option<u64> {
        let fresh = self
            .disk
            .is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(60));
        if !fresh {
            self.disk = Some((tokio::time::Instant::now(), dir_size(&self.paths.root)));
        }
        self.disk.map(|(_, bytes)| bytes)
    }

    /// "Fiat rate": fetch the price of DASH in `currency` now and every [`rates::EVERY`];
    /// `None` stops and forgets it. A failed fetch keeps the last price and is tried again with
    /// the next round.
    fn fiat_rate(&mut self, currency: Option<String>) {
        if let Some(task) = self.fiat_task.take() {
            task.abort();
        }
        let slot = self.fiat.clone();
        let Some(currency) = currency else {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return;
        };
        if slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|f| f.currency != currency)
        {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        let (dirty, wake) = (self.dirty.clone(), self.wake.clone());
        self.fiat_task = Some(tokio::spawn(async move {
            loop {
                let asked = currency.clone();
                if let Ok(Ok(price)) =
                    tokio::task::spawn_blocking(move || rates::fetch(&asked)).await
                {
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(FiatRate {
                        currency: currency.clone(),
                        price,
                        at: now(),
                    });
                    dirty.store(true, Ordering::Relaxed);
                    wake();
                }
                tokio::time::sleep(rates::EVERY).await;
            }
        }));
    }

    async fn wallet(&mut self, id: WalletId, meta: VaultMeta) -> Wallet {
        let mut unfinished = None;
        let mut unfinished_top_up = None;
        let mut identities = Vec::new();
        let (balance, (coins, addresses), mut history, shown, platform) = match self
            .chain
            .wallet(&id)
            .await
        {
            Some(wallet) => {
                let records = {
                    let state = wallet.state().await;
                    state.core_wallet.transaction_history().len()
                };
                if !self.fresh_receive {
                    // One address for every payment: the first of the wallet's receive chain
                    // (m/44'/1'/0'/0/0), the same after a restart or a restore.
                    let state = wallet.state().await;
                    self.receive_address =
                        first_receive_address(&state.core_wallet).map(|a| (a, records));
                } else if self
                    .receive_address
                    .as_ref()
                    .is_none_or(|(_, at)| *at != records)
                {
                    self.receive_address = wallet
                        .core()
                        .next_receive_address_for_account(0)
                        .await
                        .ok()
                        .map(|a| (a.to_string(), records));
                }
                let platform = self.chain.platform_address(&wallet).await.ok();
                unfinished = self.unfinished_identity(&wallet).await;
                unfinished_top_up = self.unfinished_top_up(&wallet).await;
                let live_count = {
                    let state = wallet.state().await;
                    state.core_wallet.transaction_history().len()
                };
                self.refresh_stored_history(id, live_count).await;
                let state = wallet.state().await;
                let now = now();
                let tip = self.chain.status().headers.map_or(0, |h| h.height);
                // The store's records first: one per transaction, as platform-wallet folds the
                // accounts it touches. Then this run's records the store has not got yet, one
                // per transaction too (the spending account's), so an asset lock — which
                // touches two accounts — is one line, not two.
                let stored_ids: std::collections::HashSet<_> =
                    self.stored_history.iter().map(|r| r.txid).collect();
                let mut fresh: Vec<
                    &key_wallet::managed_account::transaction_record::TransactionRecord,
                > = Vec::new();
                for r in state.core_wallet.transaction_history() {
                    if stored_ids.contains(&r.txid) {
                        continue;
                    }
                    match fresh.iter_mut().find(|f| f.txid == r.txid) {
                        Some(f) if f.input_details.is_empty() && !r.input_details.is_empty() => {
                            *f = r
                        }
                        Some(_) => {}
                        None => fresh.push(r),
                    }
                }
                // A spend recorded before the coin it spends (a restore's rescan) gets its
                // input back from our own outputs.
                let mut all: Vec<_> = self.stored_history.iter().chain(fresh).cloned().collect();
                convert::attribute_late_inputs(&mut all);
                let records: Vec<(String, Activity)> = all
                    .iter()
                    .map(|r| {
                        let txid = r.txid.to_string();
                        let seen = self.mempool.first_seen(&txid, now, tip);
                        let verified = self.chain.instant_lock_verified(&r.txid);
                        (txid, activity(r, seen, verified))
                    })
                    .collect();
                let paid_at: HashMap<String, Timestamp> =
                    records.iter().map(|(t, a)| (t.clone(), a.at)).collect();
                // The interface shows one identity: the first slot of the phrase, with its
                // DashPay profile and contacts.
                let names = self.names.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let avatars = self.avatars.clone();
                let refresh = self.refresher();
                let avatar = |owner: &Identifier, p: &platform_wallet::DashPayProfile| {
                    avatars.want(
                        &dashpay::base58(owner),
                        p.avatar_url.as_deref(),
                        p.avatar_hash,
                        refresh.clone(),
                    )
                };
                let mine: Vec<_> = state
                    .identity_manager
                    .managed_identities()
                    .filter(|m| m.wallet_id == Some(id) && m.identity_index.is_some())
                    .collect();
                identities = mine
                    .iter()
                    .map(|m| IdentityChoice {
                        id: m.identity.id().to_string(Encoding::Base58),
                        name: identity::to_api(m).name().map(str::to_owned),
                        slot: m.identity_index.unwrap_or_default(),
                    })
                    .collect();
                identities.sort_by_key(|c| c.slot);
                let slot = chain::shown_slot(id, mine.iter().filter_map(|m| m.identity_index));
                // The identity the person works as (the first one until they choose).
                let first = mine.iter().copied().find(|m| m.identity_index == slot);
                let shown = first.map(|m| {
                    let mut api = identity::to_api(m);
                    api.profile = m
                        .dashpay()
                        .profile
                        .as_ref()
                        .map(|p| dashpay::profile_of(p, avatar(&m.identity.id(), p)));
                    (
                        api,
                        identity::credits_as_duffs(m.identity.balance()),
                        dashpay::contacts_of(m, &names, avatar, |txid| paid_at.get(txid).copied())
                            .into_iter()
                            // Hidden before a resync: the new store does not know it.
                            .filter(|c| {
                                c.state != ContactState::Incoming
                                    || !self.hidden.contains(&c.identity_id)
                            })
                            .collect::<Vec<_>>(),
                    )
                });
                // Payments to and from contacts say who; this run's payments too, and they
                // show until the history has them.
                let records = match first {
                    Some(m) => dashpay::name_payments(m, &names, records),
                    None => records,
                };
                let mut history = self
                    .outgoing
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .apply(records);
                // And what the wallet did on Platform, which Core does not see.
                history.extend(
                    self.journal
                        .entries()
                        .iter()
                        .enumerate()
                        .map(|(n, e)| journal::activity(e, n)),
                );
                (
                    wallet.balance().total(),
                    {
                        // Coins, and the addresses holding them (the overview counts those).
                        let utxos = state.core_wallet.utxos();
                        let holding: std::collections::HashSet<_> =
                            utxos.iter().map(|u| u.address.clone()).collect();
                        (utxos.len(), holding.len())
                    },
                    history,
                    shown,
                    platform,
                )
            }
            None => (0, (0, 0), Vec::new(), None, None),
        };
        let (platform_address, platform_credits) = platform.unwrap_or_default();
        // The shielded pool, once connected: local reads, no network.
        let (pool, unfinished_shield) = match self.chain.wallet(&id).await {
            Some(wallet) => (
                self.chain.pool_view(&wallet).await,
                self.chain
                    .unfinished_pool_shield(&wallet)
                    .await
                    .map(|duffs| duffs as Duffs),
            ),
            None => (None, None),
        };
        // Asked to connect for this wallet and not ready yet.
        let connecting_pool = pool.is_none()
            && *self.pool_wallet.lock().unwrap_or_else(|e| e.into_inner()) == Some(id);
        // What the scan lost after a restart (a top-up from the pool as 0), put back.
        let pool = pool.map(|mut p| {
            p.operations = self.pool_history.merge(std::mem::take(&mut p.operations));
            p
        });
        // The pool's fees from Platform's formulas at the protocol version the SDK speaks.
        let mut fees = fees(coins);
        if let Ok(sdk) = self.chain.sdk().await {
            pool::set_fees(&mut fees, sdk.version());
            platform_fees(&mut fees, sdk.version(), &self.journal);
        }
        // The pool's own operations are in the history too (only the owner sees them), and a
        // Core asset lock made for the pool says so instead of "Transfer to Platform".
        if let Some(pool) = &pool {
            history.extend(pool.operations.iter().cloned());
        }
        if let Some(wallet) = self.chain.wallet(&id).await {
            let pool_locks = self.chain.pool_locks(&wallet).await;
            for act in history.iter_mut() {
                let locked = act.txid.as_ref().and_then(|t| pool_locks.get(t));
                if let (ActivityKind::TopUp { .. }, Some(locked)) = (&act.kind, locked) {
                    act.kind = ActivityKind::Shielded {
                        from: Some("Core".into()),
                    };
                    // key-wallet's net for these locks counts the change but not the lock: what
                    // left Core is the lock and its fee.
                    act.amount = -(*locked as Duffs + act.fee.unwrap_or(0));
                }
            }
            // "Transfer to Platform" says whom: the identity it registered or tops up.
            let tracked: HashMap<String, (platform_wallet::AssetLockFundingType, u32)> = wallet
                .asset_locks()
                .list_tracked_locks()
                .await
                .into_iter()
                .map(|l| {
                    (
                        l.out_point.txid.to_string(),
                        (l.funding_type, l.identity_index),
                    )
                })
                .collect();
            for act in history.iter_mut() {
                if let (ActivityKind::TopUp { identity }, Some(txid)) =
                    (&mut act.kind, act.txid.as_deref())
                    && identity.is_empty()
                    && let Some(who) =
                        identity::funded_identity(txid, tracked.get(txid).copied(), &identities)
                {
                    *identity = who;
                }
            }
        }
        let credits = shown.as_ref().map_or(0, |(_, c, _)| *c);
        let mut contacts = shown
            .as_ref()
            .map(|(_, _, contacts)| contacts.clone())
            .unwrap_or_default();
        let unnamed = contacts
            .iter()
            .filter(|c| c.name.is_none())
            .map(|c| c.identity_id.clone())
            .collect();
        self.ask_names(unnamed);
        self.contacts = contacts.clone();
        let (nfts, nfts_status) = self.shown_nfts(shown.as_ref().map(|(identity, _, _)| identity));
        let apps = app_jobs::shown_apps(
            &self.apps.all(),
            shown.as_ref().map(|(identity, _, _)| identity.id.as_str()),
        );
        if let Some(wallet) = self.chain.wallet(&id).await {
            let mut keys = self.chain.contact_key_addresses(&wallet).await;
            // And the ones paid in earlier runs, kept beside the store.
            for c in &contacts {
                for address in self.contact_paid.paid(&c.identity_id) {
                    keys.insert(address, c.identity_id.clone());
                }
            }
            convert::name_contact_payments(&mut history, &keys, &contacts);
            convert::count_contact_payments(&history, &keys, &mut contacts);
        }
        convert::settle(&mut history, self.chain.status().chainlock_height);
        self.mempool.mark_stale(
            &mut history,
            self.chain.status().headers.map_or(0, |h| h.height),
        );
        history.sort_by_key(|a| std::cmp::Reverse(a.at));
        Wallet {
            balances: Balances {
                core: balance as Duffs,
                core_addresses: addresses as u32,
                credits,
                platform_address: identity::credits_as_duffs(platform_credits),
                shielded: pool
                    .as_ref()
                    .map_or(0, |p| identity::credits_as_duffs(p.credits)),
            },
            activity: history,
            core_address: self
                .receive_address
                .as_ref()
                .map(|(a, _)| a.clone())
                .unwrap_or_default(),
            platform_address,
            identity: shown.map(|(identity, _, _)| identity),
            pool: match pool {
                Some(pool) => Pool {
                    notes: pool.notes,
                    checked_at: pool.checked_at,
                    address: Some(pool.address),
                    unfinished_shield,
                    connecting: false,
                    operations: pool.operations,
                },
                None => Pool {
                    notes: 0,
                    checked_at: None,
                    address: None,
                    unfinished_shield,
                    connecting: connecting_pool,
                    operations: Vec::new(),
                },
            },
            apps,
            nfts,
            nfts_status,
            contacts,
            unfinished_identity: unfinished,
            unfinished_top_up,
            identities,
            address_book: self
                .address_book
                .entries()
                .into_iter()
                .map(|e| SavedAddress {
                    address: e.address,
                    label: e.label,
                })
                .collect(),
            identity_search: self
                .identity_search
                .lock()
                .map(|s| s.clone())
                .unwrap_or_default(),
            fees,
            phrase_words: meta.word_count as usize,
            created_at: meta.created_at as Timestamp,
        }
    }

    /// Who a Core payment to `to` goes to: a testnet address, or an established contact by name
    /// or — one without a name — by its identity id.
    fn core_payee(&self, to: &str) -> Result<CorePayee, CoreError> {
        let by_id = self
            .contacts
            .iter()
            .find(|c| c.identity_id == to.trim() && c.state == ContactState::Established);
        if let Some(contact) = by_id {
            Ok(CorePayee::Contact(
                Identifier::from_string(&contact.identity_id, Encoding::Base58)
                    .map_err(|e| CoreError::Failed(e.to_string()))?,
            ))
        } else if to.trim().starts_with('@') {
            let label = dpns::label(to)?;
            // By the name in the last snapshot, or one learnt since (a lookup, `ResolveName`).
            let names = self.names.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let name_of = |c: &Contact| {
                c.name
                    .clone()
                    .or_else(|| names.get(&c.identity_id).cloned().flatten())
            };
            let established = || {
                self.contacts
                    .iter()
                    .filter(|c| c.state == ContactState::Established)
            };
            let contact = established()
                .find(|c| name_of(c).as_deref() == Some(label.as_str()))
                .ok_or_else(|| {
                    if established().any(|c| name_of(c).is_none()) {
                        // Some contacts' names are still on their way: it may be one of them.
                        CoreError::NotAvailable(
                            tr!("Contacts' names are still loading — try again in a few seconds")
                                .into(),
                        )
                    } else {
                        CoreError::NotAvailable(format!(
                            tr!("@{label} is not in your contacts: a Core payment by name goes only to a DashPay contact")
                        , label = label))
                    }
                })?;
            Ok(CorePayee::Contact(
                Identifier::from_string(&contact.identity_id, Encoding::Base58)
                    .map_err(|e| CoreError::Failed(e.to_string()))?,
            ))
        } else if PlatformAddress::from_bech32m_string(to.trim()).is_ok() {
            Err(CoreError::NotAvailable(
                tr!("We do not send from Core to a Platform address yet (an asset lock is needed). Send from «Credits»").into(),
            ))
        } else {
            Ok(CorePayee::Address(
                payee(to)?
                    .require_network(crate::network::dash())
                    .map_err(|_| CoreError::BadAddress)?,
            ))
        }
    }

    /// The address a Core payment to `to` goes to, answered as `PaymentPreview`: an address at
    /// once, a contact's next address from its key in the background. The contact's is kept:
    /// the payment is signed only to it.
    fn preview_payment(&self, to: String) -> Result<(), CoreError> {
        // Asked as the address is typed: a typo is said apart from another network's address.
        let payee = self.core_payee(&to).map_err(|e| match e {
            CoreError::BadAddress => not_an_address(&to).unwrap_or(CoreError::BadAddress),
            e => e,
        })?;
        let contact = match payee {
            CorePayee::Address(address) => {
                let _ = self.out.send(Event::PaymentPreview {
                    to,
                    result: Ok(address.to_string()),
                });
                return Ok(());
            }
            CorePayee::Contact(contact) => contact,
        };
        let (id, keys) = self.dashpay_ready()?;
        let chain = self.chain.clone();
        let previews = self.previews.clone();
        let paid = self.contact_paid.paid(&contact.to_string(Encoding::Base58));
        self.answer_later(async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let address = chain
                    .contact_address(&wallet, &keys, contact, &paid)
                    .await
                    .map_err(CoreError::Failed)?
                    .ok_or(CoreError::NotAvailable(
                        tr!("No address from the contact's key yet — try again in a minute").into(),
                    ))?;
                previews
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(contact.to_string(Encoding::Base58), address.clone());
                Ok(address)
            }
            .await;
            Event::PaymentPreview { to, result }
        });
        Ok(())
    }

    /// Signs a Core payment and hands it to the network in the background: the network's answer
    /// takes up to a minute and the loop must go on sending snapshots. Never resent: a payment
    /// with no answer may still go through. `password`: "password for every payment" is on, and
    /// nothing is signed until it opens the vault.
    async fn send(
        &mut self,
        from: Source,
        to: &str,
        amount: Duffs,
        password: Option<&Password>,
    ) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let id = open.id;
        if let Some(password) = password {
            self.open_vault(password).await?;
        }
        match from {
            Source::Core => {}
            Source::Credits => return self.send_credits(id, to, amount),
            Source::Pool => return self.send_from_pool(to, amount),
        }
        let payee = self.core_payee(to)?;
        // A contact is paid only to the address the last look before signing showed.
        let shown = match &payee {
            CorePayee::Address(_) => None,
            CorePayee::Contact(contact) => Some(
                self.previews
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&contact.to_string(Encoding::Base58))
                    .ok_or(CoreError::NotAvailable(
                        tr!("The contact's address was not shown before signing — check the payment again")
                            .into(),
                    ))?,
            ),
        };
        let amount = positive_duffs(amount)?;
        // Coins are known only after the filters have been checked up to the tip.
        let status = self.chain.status();
        if !coins_known(&status, true) || status.phase == Phase::Error {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        if wallet.balance().total() < amount {
            return Err(CoreError::NotEnough);
        }
        let keys = self
            .unlocked
            .as_ref()
            .and_then(|open| open.keys.clone())
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        // In the history from now on, as "sending", until the wallet sees it.
        let coins = wallet.state().await.core_wallet.utxos().len();
        let key = self
            .outgoing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start(to, amount as Duffs, fees(coins).core_payment, now());
        let outgoing = self.outgoing.clone();
        let contact_paid = self.contact_paid.clone();
        let chain = self.chain.clone();
        let out = self.out.clone();
        let wake = self.wake.clone();
        let dirty = self.dirty.clone();
        // Building, signing and the network's answer happen off the loop: the broadcast waits
        // for peers to relay the payment.
        tokio::spawn(async move {
            let paid = match (&payee, shown) {
                (CorePayee::Address(address), _) => {
                    chain.pay(&wallet, &keys, address, amount).await
                }
                (CorePayee::Contact(to), shown) => {
                    let shown = shown.unwrap_or_default();
                    let contact = to.to_string(Encoding::Base58);
                    let before = contact_paid.paid(&contact);
                    let paid = chain
                        .pay_contact(&wallet, &keys, *to, &shown, &before, amount)
                        .await;
                    // Taken, or maybe taken: never this address again, after a restart too.
                    if paid.is_ok() {
                        contact_paid.add(&contact, &shown);
                    }
                    paid
                }
            };
            outgoing.lock().unwrap_or_else(|e| e.into_inner()).answered(
                key,
                match &paid {
                    Ok(paid) => outgoing::Answer::Sent {
                        txid: Some(paid.txid.clone()),
                        fee: paid.fee as Duffs,
                    },
                    Err(_) => outgoing::Answer::Refused,
                },
            );
            let result = paid
                .map(|paid| SentPayment {
                    txid: paid.txid,
                    amount: amount as Duffs,
                    fee: paid.fee as Duffs,
                    relayed: paid.relayed,
                })
                .map_err(core_failure);
            let _ = out.send(Event::Sent(result));
            dirty.store(true, Ordering::Relaxed);
            wake();
        });
        Ok(())
    }

    /// Builds the key accounts of the established contacts in the background once the DashPay
    /// sync has found them: a payment to a contact is named by its @name, and counted in
    /// its payments, also in a wallet restored from its phrase and before any payment in this
    /// run. Again when more contacts are established; a failure is tried on a later round.
    fn build_contact_accounts(&self) {
        let Some((id, Some(keys))) = self.unlocked.as_ref().map(|o| (o.id, o.keys.clone())) else {
            return;
        };
        let established = self
            .contacts
            .iter()
            .filter(|c| c.state == ContactState::Established)
            .count();
        if established <= self.contact_accounts_for.load(Ordering::Relaxed)
            || self.contact_accounts_busy.swap(true, Ordering::Relaxed)
        {
            return;
        }
        let chain = self.chain.clone();
        let built = self.contact_accounts_for.clone();
        let busy = self.contact_accounts_busy.clone();
        let dirty = self.dirty.clone();
        let wake = self.wake.clone();
        tokio::spawn(async move {
            let accounts = match chain.wallet(&id).await {
                Some(wallet) => chain.build_contact_accounts(&wallet, &keys).await.ok(),
                None => None,
            };
            if accounts.is_some() {
                // The history names the contacts' payments with what was built.
                dirty.store(true, Ordering::Relaxed);
                wake();
            }
            if contact_accounts_done(established, accounts) {
                built.store(established, Ordering::Relaxed);
            } else {
                // Not every contact has its account yet (the sweep or a contact's key was not
                // there): tried again, not before a pause.
                tokio::time::sleep(CONTACT_ACCOUNTS_RETRY).await;
            }
            busy.store(false, Ordering::Relaxed);
        });
    }

    /// Connects the open wallet's shielded pool once per run, in the background. The
    /// first time needs the seed (the viewing keys are saved then), so only an unlocked
    /// wallet; a failure is tried again a minute later.
    fn connect_pool(&self) {
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
        tokio::spawn(async move {
            let result = match chain.wallet(&id).await {
                Some(wallet) => chain.connect_pool(&wallet, &seed[..], &db).await,
                None => Err(tr!("the wallet is not loaded").to_owned()),
            };
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
    }

    /// What every pool move needs: the open wallet with its keys and seed, proofs, a connected
    /// pool, and no other move under way (it is marked under way here).
    fn pool_move(&self) -> Result<(WalletId, key_wallet::Wallet, Zeroizing<[u8; 64]>), CoreError> {
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
    fn pool_job<T: Send + 'static>(
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
        self.answer_later(async move {
            let result = job.await;
            busy.store(false, Ordering::Relaxed);
            dirty.store(true, Ordering::Relaxed);
            answer(result)
        });
    }

    /// "Hide": credits of the identity the interface shows, the wallet's Platform
    /// addresses, or Core funds (an asset lock) into the pool, to the wallet's own private
    /// address.
    fn shield(&self, from: ShieldSource, amount: Duffs) -> Result<(), CoreError> {
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
        self.pool_job(
            "shield",
            async move {
                let wallet = loaded(&chain, &id).await?;
                // Credits and Platform addresses leave a line in the history (Core has its own,
                // the asset lock); what left them is read before and after.
                let (label, before) = match from {
                    ShieldSource::Credits => (
                        tr!("identity credits"),
                        chain.first_identity_credits(&wallet, false).await,
                    ),
                    ShieldSource::PlatformAddress => (
                        tr!("a Platform address"),
                        chain.platform_address(&wallet).await.ok().map(|(_, c)| c),
                    ),
                    ShieldSource::Core => ("Core", None),
                };
                match from {
                    ShieldSource::Credits => {
                        chain.shield_from_identity(&wallet, &keys, credits).await
                    }
                    ShieldSource::PlatformAddress => {
                        chain
                            .shield_from_platform_address(&wallet, &keys, credits)
                            .await
                    }
                    // Duffs locked on Core; an unfinished lock is finished instead.
                    ShieldSource::Core => {
                        chain.shield_from_core(&wallet, &keys, credits / 1000).await
                    }
                }
                .map_err(pool_error)?;
                if from != ShieldSource::Core {
                    let after = match from {
                        ShieldSource::Credits => chain.first_identity_credits(&wallet, true).await,
                        _ => chain.platform_address(&wallet).await.ok().map(|(_, c)| c),
                    };
                    let moved = amount;
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
                    });
                }
                Ok(())
            },
            |result| Event::Done {
                what: "shield",
                result,
            },
        );
        Ok(())
    }

    /// "Unshield": out of the pool to the wallet's own Platform address.
    fn unshield(&self, amount: Duffs) -> Result<(), CoreError> {
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
                    .unshield(&wallet, &seed[..], &to, credits)
                    .await
                    .map_err(pool_error)
            },
            |result| Event::Done {
                what: "unshield",
                result,
            },
        );
        Ok(())
    }

    /// "Unshield" into the identity's credits: out of the pool into the identity the interface
    /// shows, which gets `amount`; Platform's fee comes from the notes on top.
    fn top_up_from_pool(&self, amount: Duffs) -> Result<(), CoreError> {
        let credits = positive_credits(amount)?;
        let (id, keys, seed) = self.pool_move()?;
        let chain = self.chain.clone();
        self.pool_job(
            "pool-top-up",
            async move {
                let wallet = loaded(&chain, &id).await?;
                let (identity, _) = Chain::first_identity(&wallet, &keys)
                    .await
                    .map_err(CoreError::NotAvailable)?;
                chain
                    .top_up_from_pool(&wallet, &seed[..], &identity.id(), credits)
                    .await
                    .map_err(pool_error)
            },
            |result| Event::Done {
                what: "pool-top-up",
                result,
            },
        );
        Ok(())
    }

    /// "Send" from the pool: to a private address it stays private; to a Platform address
    /// it leaves the pool (and is seen); to a Core address it is withdrawn, paid out by Platform
    /// minutes later. Answered with `Sent`.
    fn send_from_pool(&self, to: &str, amount: Duffs) -> Result<(), CoreError> {
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
        self.pool_job(
            "send",
            async move {
                let wallet = loaded(&chain, &id).await?;
                match kind {
                    To::Private => chain.pool_transfer(&wallet, &seed[..], &to, credits).await,
                    To::Platform => chain.unshield(&wallet, &seed[..], &to, credits).await,
                    To::Core => chain.pool_withdraw(&wallet, &seed[..], &to, credits).await,
                }
                // Handed over but not proved: maybe gone, so "sent, no answer", never resent.
                .map(|()| true)
                .or_else(|e| match e {
                    pool::PoolError::Unconfirmed => Ok(false),
                    other => Err(pool_error(other)),
                })
            },
            move |result| {
                Event::Sent(result.map(|relayed| SentPayment {
                    txid: String::new(),
                    amount,
                    fee: 0,
                    relayed,
                }))
            },
        );
        Ok(())
    }

    /// Credits of the wallet's identity to `to`: `@name` (a transfer to that identity), a
    /// Platform address `tdash1…` (credits stay on Platform) or a Core address (a withdrawal,
    /// paid out on Core later). Signed and sent in the background; the answer is `Event::Sent`
    /// with no txid (Platform proves the transition itself).
    fn send_credits(&self, id: WalletId, to: &str, amount: Duffs) -> Result<(), CoreError> {
        let credits = positive_credits(amount)?;
        let to = to.trim();
        enum To {
            Name(String),
            Ready(CreditPayee),
        }
        // And how the history will say it.
        let (to, what) = if to.starts_with('@') {
            let label = dpns::label(to)?;
            let what = journal::What::Credits {
                to: format!("@{label}"),
            };
            (To::Name(label), what)
        } else if let Ok(address) = PlatformAddress::from_bech32m_string(to) {
            // The same key's address on the other network is not where the person meant.
            if !platform_address_here(to) {
                return Err(CoreError::BadAddress);
            }
            let what = journal::What::Credits { to: to.to_owned() };
            (To::Ready(CreditPayee::Platform(address)), what)
        } else {
            let address = payee(to)?
                .require_network(crate::network::dash())
                .map_err(|_| CoreError::BadAddress)?;
            let what = journal::What::Withdrawal {
                to: address.to_string(),
            };
            (To::Ready(CreditPayee::Core(address)), what)
        };
        if !self.chain.platform_ready() {
            return Err(CoreError::NotSynced);
        }
        let keys = self
            .unlocked
            .as_ref()
            .and_then(|open| open.keys.clone())
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        let chain = self.chain.clone();
        let dirty = self.dirty.clone();
        let journal = self.journal.clone();
        self.answer_later(async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let before = chain.first_identity_credits(&wallet, false).await;
                let payee = match to {
                    To::Ready(payee) => payee,
                    To::Name(label) => match chain.resolve_name(&label).await {
                        Ok(Some(id)) => CreditPayee::Identity(
                            Identifier::from_string(&id, Encoding::Base58)
                                .map_err(|e| CoreError::Failed(e.to_string()))?,
                        ),
                        Ok(None) => return Err(CoreError::NameNotFound),
                        Err(e) => return Err(CoreError::Failed(e)),
                    },
                };
                chain
                    .send_credits(&wallet, &keys, payee, credits)
                    .await
                    .map_err(dashpay_failure)?;
                // `send_credits` read the balance back: the difference is the amount and fee.
                let after = chain.first_identity_credits(&wallet, false).await;
                journal.add(journal::Entry {
                    at: now(),
                    what,
                    spent: spent_credits(before, after).unwrap_or(amount),
                    moved: amount,
                });
                Ok::<(), CoreError>(())
            }
            .await
            .map(|()| SentPayment {
                txid: String::new(),
                amount,
                fee: 0,
                relayed: true,
            });
            dirty.store(true, Ordering::Relaxed);
            Event::Sent(result)
        });
        Ok(())
    }

    /// Opens the vault and restarts sync with the wallet's keys.
    async fn unlock(&mut self, password: &Password) -> Result<(), CoreError> {
        let (file, phrase) = self.open_vault(password).await?;
        let meta = file.meta;
        if meta.network != network() {
            return Err(CoreError::NotAvailable(format!(
                tr!("The wallet on this computer is for the {} network"),
                meta.network
            )));
        }
        self.last_touch = tokio::time::Instant::now();
        let (id, _) = self
            .chain
            .open_wallet(&phrase, meta.birth_height)
            .await
            .map_err(CoreError::Failed)?;
        let keys = keys_of(&phrase)?;
        self.unlocked = Some(Unlocked {
            id,
            meta,
            keys: Some(keys),
            seed: Some(seed_of(&phrase)?),
        });
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

    /// Looks for the phrase's identities in the background once proofs can be checked; what is
    /// found goes to the store and so into the next snapshot. A failed try is made again after
    /// 30 s, then less and less often (up to 5 min), while this unlock lasts. A newer search
    /// (`FindIdentities`) or locking stops this one, and its keys go with it.
    fn discover_identities(&self) {
        let Some((id, Some(keys))) = self.unlocked.as_ref().map(|o| (o.id, o.keys.clone())) else {
            return;
        };
        let run = self.discovery_run.fetch_add(1, Ordering::Relaxed) + 1;
        let current = self.discovery_run.clone();
        let chain = self.chain.clone();
        let search = self.identity_search.clone();
        let found = self.identities_found.clone();
        let refresh = self.refresher();
        let stale = self.balances_stale.clone();
        found.store(false, Ordering::Relaxed);
        set_search(&search, IdentitySearch::Searching);
        refresh();
        tokio::spawn(async move {
            let live = move || current.load(Ordering::Relaxed) == run;
            chain.until_platform_ready().await;
            let mut pause = DISCOVERY_RETRY;
            while live() {
                let result = match chain.wallet(&id).await {
                    Some(wallet) => chain.discover_identities(&wallet, &keys).await.map(|_| ()),
                    None => Err(tr!("the wallet is not loaded yet").into()),
                };
                if !live() {
                    break;
                }
                match result {
                    Ok(()) => {
                        found.store(true, Ordering::Relaxed);
                        set_search(&search, IdentitySearch::Done);
                        refresh();
                        // The keys are not needed to read balances: let them go now.
                        drop(keys);
                        keep_balances_fresh(chain, id, live, stale, refresh).await;
                        return;
                    }
                    Err(why) => {
                        set_search(&search, IdentitySearch::Failed(why));
                        refresh();
                    }
                }
                // A second at a time, so a lock or a newer search ends the wait at once.
                let until = tokio::time::Instant::now() + pause;
                while live() && tokio::time::Instant::now() < until {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                pause = (pause * 2).min(DISCOVERY_RETRY_MAX);
            }
        });
    }

    /// Checks what can be checked at once and starts the identity in the background: the name
    /// check, the asset lock (`top_up` plus the name's fee), the identity and its name take a
    /// minute or more. The answer is `Done { what: "identity" }`.
    async fn create_identity(&mut self, name: &str, top_up: Duffs) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let id = open.id;
        let label = dpns::label(name)?;
        if dpns::is_contested(&label) {
            return Err(CoreError::NotAvailable(
                tr!("Contested names are decided by a masternode vote, which evolite cannot do yet. Add a digit from 2 to 9 to the name or make it longer than 19 characters").into(),
            ));
        }
        let top_up = positive_duffs(top_up)?;
        let amount = top_up + fees(0).name_regular as u64;
        let status = self.chain.status();
        if !coins_known(&status, true)
            || status.phase == Phase::Error
            || !self.chain.platform_ready()
        {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        if wallet.balance().total() < amount {
            return Err(CoreError::NotEnough);
        }
        // A cut-short identity is finished first, with the money already locked for it.
        if self.unfinished_identity(&wallet).await.is_some() {
            return Err(CoreError::NotAvailable(
                tr!("First finish the stopped identity — its money is already locked").into(),
            ));
        }
        let keys = open
            .keys
            .clone()
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        if self.identity_busy.swap(true, Ordering::Relaxed) {
            return Err(CoreError::NotAvailable(
                tr!("An identity is already being made").into(),
            ));
        }
        let chain = self.chain.clone();
        let dirty = self.dirty.clone();
        let pending = self.pending_names();
        let busy = self.identity_busy.clone();
        let (out, wake) = (self.out.clone(), self.wake.clone());
        self.answer_later(async move {
            let result = match chain.name_available(&label).await {
                Ok(false) => Err(CoreError::NotAvailable(format!(
                    tr!("The name @{label} is already taken, the money is untouched"),
                    label = label
                ))),
                Err(e) => Err(CoreError::Failed(e)),
                Ok(true) => async {
                    let index = chain.next_identity_index(&wallet, &keys).await?;
                    watch_creation(
                        chain.clone(),
                        wallet.clone(),
                        index,
                        busy.clone(),
                        out,
                        wake,
                    );
                    // The name is kept before any money moves: a run cut short is finished
                    // with it (ResumeIdentity).
                    pending.set(index, &label)?;
                    chain
                        .create_identity(&wallet, &keys, index, &label, amount)
                        .await?;
                    pending.remove(index)
                }
                .await
                .map_err(core_failure),
            };
            busy.store(false, Ordering::Relaxed);
            dirty.store(true, Ordering::Relaxed);
            Event::Done {
                what: "identity",
                result,
            }
        });
        Ok(())
    }

    /// Seals the pending phrase with the password, saves the vault, starts the wallet's sync.
    /// A restored wallet is scanned from the start (its age is unknown); a new one from the tip.
    /// The phrase stays pending until the vault is written, so a failure can be retried.
    async fn create(&mut self, password: &Password) -> Result<(), CoreError> {
        if password.expose().is_empty() {
            return Err(CoreError::WrongPassword);
        }
        too_short(password)?;
        if self.saved.is_some() {
            return Err(CoreError::NotAvailable(
                tr!("There is already a wallet on this computer").into(),
            ));
        }
        let (phrase, restored) = self.pending.clone().ok_or(CoreError::BadPhrase)?;
        let birth_height = if restored {
            0
        } else {
            self.chain.status().headers.map_or(0, |h| h.height)
        };
        let keys = keys_of(&phrase)?;
        let meta = VaultMeta {
            network: network().into(),
            wallet_id: hex(&keys.compute_wallet_id()),
            birth_height,
            word_count: phrase.split_whitespace().count() as u8,
            created_at: now() as u64,
        };
        let password = Zeroizing::new(password.expose().to_owned());
        let file = {
            let phrase = phrase.clone();
            let meta = meta.clone();
            tokio::task::spawn_blocking(move || {
                vault::seal(&phrase, &password, meta, KdfParams::default())
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
        let (id, _) = self
            .chain
            .open_wallet(&phrase, birth_height)
            .await
            .map_err(|e| {
                // The vault is "Locked" now: the interface goes to the unlock screen.
                CoreError::NotAvailable(format!(
                    tr!("The wallet is saved, but the sync did not start ({e}). Sign in with your password — evolite starts it again")
                , e = e))
            })?;
        self.unlocked = Some(Unlocked {
            id,
            meta,
            keys: Some(keys),
            seed: Some(seed_of(&phrase)?),
        });
        self.receive_address = None;
        self.discover_identities();
        Ok(())
    }

    async fn handle(&mut self, command: Command) -> (Option<Event>, bool) {
        let event = match command {
            Command::NewPhrase { words } => {
                let n = if words == 24 { 24 } else { 12 };
                match Mnemonic::generate(n, Language::English) {
                    Ok(mnemonic) => {
                        let phrase = Zeroizing::new(mnemonic.phrase());
                        let words = phrase.split_whitespace().map(str::to_owned).collect();
                        self.pending = Some((phrase, false));
                        self.pending_origin = wallets::Origin::Created;
                        Event::NewPhrase(Phrase::new(words))
                    }
                    Err(_) => Event::Done {
                        what: "create",
                        result: Err(CoreError::BadPhrase),
                    },
                }
            }
            Command::UsePhrase { phrase, dashpay } => {
                let text = Zeroizing::new(phrase.words().join(" "));
                if !Mnemonic::validate(&text) {
                    Event::PhraseAccepted(Err(CoreError::BadPhrase))
                } else if let Some(name) = self.same_wallet(&text) {
                    // One phrase, one wallet on this computer.
                    Event::PhraseAccepted(Err(CoreError::NotAvailable(if name.is_empty() {
                        tr!("This wallet is already on this computer").into()
                    } else {
                        format!(
                            tr!("This wallet is already on this computer — “{name}”"),
                            name = name
                        )
                    })))
                } else if !dashpay {
                    self.pending = Some((text, true));
                    self.pending_origin = wallets::Origin::Restored;
                    Event::PhraseAccepted(Ok(None))
                } else {
                    self.pending_origin = wallets::Origin::DashPay;
                    // Import from DashPay: what the phrase has on Platform is read now,
                    // before the wallet is made, for O5. Its keys are dropped after the scan.
                    let keys = keys_of(&text);
                    self.pending = Some((text, true));
                    let chain = self.chain.clone();
                    self.answer_later(async move {
                        let found = match keys {
                            Ok(keys) => scan_dashpay(&chain, &keys).await,
                            Err(_) => None,
                        };
                        Event::PhraseAccepted(Ok(found))
                    });
                    return (None, false);
                }
            }
            Command::CreateWallet { .. } if self.leftovers => Event::Done {
                what: "create",
                result: Err(CoreError::NotAvailable(LEFTOVERS.into())),
            },
            Command::CreateWallet { password } => Event::Done {
                what: "create",
                result: self.create(&password).await,
            },
            Command::Unlock { password } => Event::Done {
                what: "unlock",
                result: self.unlock(&password).await,
            },
            Command::Lock => Event::Done {
                what: "lock",
                result: self.lock().await,
            },
            Command::AutoLock { after_secs } => {
                self.auto_lock = after_secs.map(Duration::from_secs);
                self.last_touch = tokio::time::Instant::now();
                return (None, false);
            }
            Command::Touch => {
                self.last_touch = tokio::time::Instant::now();
                return (None, false);
            }
            Command::CoreNode { address } => {
                let result = match address.as_deref().map(node::normalize).transpose() {
                    Err(why) => Err(CoreError::NotAvailable(why)),
                    Ok(address) => {
                        let found = match &address {
                            Some(a) => node::resolve(a).await.map(|_| ()),
                            None => Ok(()),
                        };
                        match found {
                            Err(why) => Err(CoreError::NotAvailable(why)),
                            Ok(()) => node::save(&self.paths.chain.core_node, address.as_deref())
                                .map_err(|e| CoreError::Failed(e.to_string())),
                        }
                    }
                };
                Event::Done {
                    what: "core-node",
                    result,
                }
            }
            Command::FiatRate { currency } => {
                self.fiat_rate(currency);
                return (None, true);
            }
            Command::ReceiveAddress { fresh } => {
                self.fresh_receive = fresh;
                // The next snapshot takes the address again.
                self.receive_address = None;
                return (None, true);
            }
            Command::ChangePassword { old, new } => Event::Done {
                what: "password",
                result: self.change_password(&old, &new).await,
            },
            Command::RevealPhrase { password, wallet } => {
                let opened = match wallet.filter(|id| *id != self.wallet) {
                    Some(id) => self.open_wallet_vault(&id, &password).await,
                    None => self.open_vault(&password).await,
                };
                let result = opened.map(|(_, phrase)| {
                    Phrase::new(phrase.split_whitespace().map(str::to_owned).collect())
                });
                Event::PhraseRevealed(result)
            }
            Command::Send {
                from,
                to,
                amount,
                password,
            } => match self.send(from, &to, amount, password.as_ref()).await {
                // The answer comes from the broadcast task.
                Ok(()) => return (None, true),
                Err(e) => Event::Sent(Err(e)),
            },
            Command::ResolveName { name } => match dpns::label(&name) {
                Err(e) => Event::NameResolved {
                    name,
                    result: Err(e),
                },
                Ok(label) => {
                    if !self.chain.platform_ready() {
                        let _ = self.out.send(Event::NameResolved {
                            name: name.clone(),
                            result: Err(CoreError::NotSynced),
                        });
                    }
                    let chain = self.chain.clone();
                    // Where the name stands among our contacts, as of the last snapshot.
                    let states: HashMap<String, ContactState> = self
                        .contacts
                        .iter()
                        .map(|c| (c.identity_id.clone(), c.state))
                        .collect();
                    let avatars = self.avatars.clone();
                    let refresh = self.refresher();
                    let names = self.names.clone();
                    let renamed = self.refresher();
                    self.answer_later(async move {
                        chain.until_platform_ready().await;
                        let result = match chain.resolve_name(&label).await {
                            Ok(Some(identity_id)) => {
                                // A contact's name known from here on: a payment to `@name`
                                // finds the contact without waiting for its own name lookup.
                                let known = names
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .insert(identity_id.clone(), Some(label.clone()));
                                if known != Some(Some(label.clone())) {
                                    renamed();
                                }
                                // A profile that cannot be read is no reason to refuse the name.
                                let profile =
                                    match Identifier::from_string(&identity_id, Encoding::Base58) {
                                        Ok(id) => chain.profile_of(id).await.ok().flatten(),
                                        Err(_) => None,
                                    };
                                let profile = profile.map(|p| {
                                    let key = avatars.want(
                                        &identity_id,
                                        p.avatar_url.as_deref(),
                                        p.avatar_hash,
                                        refresh,
                                    );
                                    dashpay::profile_of(&p, key)
                                });
                                Ok(ResolvedName {
                                    name: label,
                                    display: profile.as_ref().and_then(|p| p.display.clone()),
                                    contact: states.get(&identity_id).copied(),
                                    identity_id,
                                    profile,
                                })
                            }
                            Ok(None) => Err(CoreError::NameNotFound),
                            Err(e) => Err(CoreError::Failed(e)),
                        };
                        Event::NameResolved { name, result }
                    });
                    return (None, false);
                }
            },
            Command::CheckName { label: asked } => match dpns::label(&asked) {
                Err(e) => Event::NameChecked {
                    label: asked,
                    result: Err(e),
                },
                // A contested name is told by its letters alone: said at once, with no wait for
                // Platform and no price shown first. evolite cannot take one either way.
                Ok(label) if dpns::is_contested(&label) => Event::NameChecked {
                    label: asked,
                    result: Ok(NameCheck::Contested),
                },
                Ok(label) => {
                    if !self.chain.platform_ready() {
                        let _ = self.out.send(Event::NameChecked {
                            label: asked.clone(),
                            result: Err(CoreError::NotSynced),
                        });
                    }
                    let chain = self.chain.clone();
                    self.answer_later(async move {
                        chain.until_platform_ready().await;
                        let result = match chain.name_available(&label).await {
                            Ok(false) => Ok(NameCheck::Taken),
                            Ok(true) => Ok(NameCheck::Available),
                            Err(e) => Err(CoreError::Failed(e)),
                        };
                        Event::NameChecked {
                            label: asked,
                            result,
                        }
                    });
                    return (None, false);
                }
            },
            Command::OpenAppLink { text } => match self.open_app_link(&text) {
                // The decoded request comes from the background task.
                Ok(()) => return (None, false),
                Err(e) => Event::AppRequest(Err(e)),
            },
            Command::CreateIdentity { name, top_up } => {
                match self.create_identity(&name, top_up).await {
                    // The answer comes from the background task.
                    Ok(()) => return (None, true),
                    Err(e) => Event::Done {
                        what: "identity",
                        result: Err(e),
                    },
                }
            }
            Command::ResumeIdentity => match self.resume_identity() {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: "identity",
                    result: Err(e),
                },
            },
            Command::Shield { from, amount } => match self.shield(from, amount) {
                Ok(()) => Event::Progress {
                    what: "shield",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: "shield",
                    result: Err(e),
                },
            },
            Command::GiftNft { .. } => Event::Done {
                what: "gift",
                result: Err(CoreError::NotAvailable(NO_GIFTS.into())),
            },
            // NFT pictures as their documents store them (packed), or profile pictures: a
            // plain image file, never packed.
            Command::DrawArt { art, side, urgent } => {
                let stored = self.nfts.picture(&art).or_else(|| {
                    self.avatars.picture(&art).map(|bytes| crate::art::Stored {
                        bytes,
                        packing: evolite_render::Packing::Plain,
                    })
                });
                match (stored, &self.artist) {
                    (Some(stored), Some(artist)) => {
                        artist.draw(art, side, urgent, stored);
                        return (None, false);
                    }
                    _ => Event::Art {
                        art,
                        side,
                        partial: false,
                        result: Err(CoreError::NotAvailable(tr!("No picture").into())),
                    },
                }
            }
            Command::AddContact { name } => match self.add_contact(&name) {
                Ok(()) => return (None, false),
                Err(e) => Event::Done {
                    what: "contact",
                    result: Err(e),
                },
            },
            Command::AcceptContact { identity_id } => {
                match self.contact_job(&identity_id, ContactJob::Accept) {
                    Ok(()) => return (None, false),
                    Err(e) => Event::Done {
                        what: "contact",
                        result: Err(e),
                    },
                }
            }
            Command::HideRequest { identity_id } => {
                match self.contact_job(&identity_id, ContactJob::Hide) {
                    Ok(()) => return (None, false),
                    Err(e) => Event::Done {
                        what: "contact",
                        result: Err(e),
                    },
                }
            }
            Command::SaveProfile {
                display,
                about,
                avatar_url,
            } => match self.save_profile(display, about, avatar_url) {
                Ok(()) => return (None, false),
                Err(e) => Event::Done {
                    what: "profile",
                    result: Err(e),
                },
            },
            Command::PreviewPayment { to } => match self.preview_payment(to.clone()) {
                Ok(()) => return (None, false),
                Err(e) => Event::PaymentPreview { to, result: Err(e) },
            },
            // As the demo answers it: `app` is the answer to a request on screen, and a
            // disconnect is not one.
            Command::DisconnectApp { id } => match self.disconnect_app(&id) {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: "disconnect",
                    result: Err(e),
                },
            },
            Command::AnswerApp { id, approve } => match self.answer_app(id, approve) {
                // Approved: the answer comes from the background task.
                Ok(true) => return (None, true),
                Ok(false) => Event::Done {
                    what: "app",
                    result: Ok(()),
                },
                Err(e) => Event::Done {
                    what: "app",
                    result: Err(e),
                },
            },
            Command::TopUpIdentity { amount } => match self.top_up_identity(amount).await {
                // The answer comes from the background task.
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: "top-up",
                    result: Err(e),
                },
            },
            Command::ResumeTopUp => match self.resume_top_up() {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: "top-up",
                    result: Err(e),
                },
            },
            Command::SaveAddress { address, label } => {
                // Only a Core testnet address: a name pays as a contact, not from the book.
                let result = payee(&address)
                    .and_then(|a| {
                        a.require_network(crate::network::dash())
                            .map_err(|_| CoreError::BadAddress)
                    })
                    .and_then(|a| {
                        self.address_book
                            .save(&a.to_string(), &label)
                            .map_err(CoreError::NotAvailable)
                    });
                self.dirty.store(true, Ordering::Relaxed);
                Event::Done {
                    what: "address-book",
                    result,
                }
            }
            Command::ForgetAddress { address } => {
                let result = self
                    .address_book
                    .forget(address.trim())
                    .map_err(CoreError::NotAvailable);
                self.dirty.store(true, Ordering::Relaxed);
                Event::Done {
                    what: "address-book",
                    result,
                }
            }
            Command::Unshield { amount } => match self.unshield(amount) {
                Ok(()) => Event::Progress {
                    what: "unshield",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: "unshield",
                    result: Err(e),
                },
            },
            Command::TopUpFromPool { amount } => match self.top_up_from_pool(amount) {
                Ok(()) => Event::Progress {
                    what: "pool-top-up",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: "pool-top-up",
                    result: Err(e),
                },
            },
            Command::FindIdentities => {
                self.discover_identities();
                return (None, true);
            }
            Command::AskPhraseCheck { password } => match self.open_vault(&password).await {
                Ok((_, phrase)) => {
                    let words: Vec<&str> = phrase.split_whitespace().collect();
                    let (questions, answers) = crate::phrase_check::questions(&words, &mut os_dice);
                    self.phrase_check = Some((questions.clone(), answers));
                    Event::PhraseCheck(Ok(questions))
                }
                Err(e) => Event::PhraseCheck(Err(e)),
            },
            Command::CheckPhraseWords { picks } => match self.phrase_check.take() {
                Some((questions, answers)) => Event::PhraseChecked {
                    wrong: crate::phrase_check::first_wrong(&questions, &answers, &picks),
                },
                // Answered already, or the wallet locked meanwhile: ask again.
                None => Event::PhraseCheck(Err(CoreError::NotAvailable(
                    tr!("The check has ended: start it again").into(),
                ))),
            },
            Command::ChooseIdentity { id } => {
                if let Some(open) = &self.unlocked
                    && let Some(wallet) = self.chain.wallet(&open.id).await
                {
                    let slot = {
                        let state = wallet.state().await;
                        state
                            .identity_manager
                            .managed_identities()
                            .find(|m| {
                                m.wallet_id == Some(open.id)
                                    && m.identity.id().to_string(Encoding::Base58) == id
                            })
                            .and_then(|m| m.identity_index)
                    };
                    if let Some(slot) = slot {
                        chain::choose_identity(open.id, slot);
                        // The contacts, NFTs and names of the other identity come next, and
                        // its credits as Platform has them now.
                        self.contacts.clear();
                        self.balances_stale.notify_one();
                    }
                }
                return (None, true);
            }
            Command::DeleteWallet { password } => Event::Done {
                what: "delete",
                result: self.delete_wallet(password.as_ref()).await,
            },
            Command::Resync => Event::Done {
                what: "resync",
                result: self.ask_resync().await,
            },
            Command::SwitchWallet { id, password } => {
                if self.index.get(&id).is_none() {
                    return (None, false);
                }
                if id == self.wallet {
                    // Figma 28: the wallet chosen on the unlock screen is this one.
                    return match password {
                        Some(password) if self.unlocked.is_none() => (
                            Some(Event::Done {
                                what: "switch",
                                result: self.unlock(&password).await,
                            }),
                            true,
                        ),
                        _ => (None, false),
                    };
                }
                if let Some(password) = &password {
                    // The other wallet's password first: a wrong one changes nothing here.
                    if let Err(e) = self.open_wallet_vault(&id, password).await {
                        return (
                            Some(Event::Done {
                                what: "switch",
                                result: Err(e),
                            }),
                            false,
                        );
                    }
                }
                self.index.last = Some(id.clone());
                self.save_index();
                self.switch_to = Some(id);
                // The new core opens it with this password (the loop answers "switch").
                self.switch_password = password;
                return (None, true);
            }
            Command::AddWallet => {
                // The first wallet's onboarding is no adding: there is nothing to go back to.
                if self.adding_from.is_some() || self.saved.is_none() {
                    return (None, false);
                }
                // An empty folder for the onboarding; it is a wallet once its vault is written.
                // The current one stays the one opened next until then.
                self.switch_to = Some(wallets::new_id(&self.paths.root));
                self.adding_from = Some(self.wallet.clone());
                return (None, true);
            }
            Command::CancelAddWallet => match self.adding_from.clone() {
                Some(from) if self.saved.is_none() => {
                    self.switch_to = Some(from);
                    return (None, true);
                }
                _ => return (None, false),
            },
            Command::RenameWallet { id, name, color } => {
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
                    what: "wallet",
                    result,
                }
            }
            Command::AddName { name } => match self.add_name(&name) {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: "name",
                    result: Err(e),
                },
            },
        };
        (Some(event), true)
    }
}

/// Opens the vault at `path` with `password`, off the async threads (Argon2id takes a while).
async fn open_vault_at(
    path: &std::path::Path,
    password: &Password,
) -> Result<(vault::VaultFile, Zeroizing<String>), CoreError> {
    let file = vault::load(path)
        .map_err(vault_error)?
        .ok_or(CoreError::NotAvailable(NO_VAULT.into()))?;
    let opened = file.clone();
    let password = Zeroizing::new(password.expose().to_owned());
    let phrase = tokio::task::spawn_blocking(move || vault::open(&opened, &password))
        .await
        .map_err(|_| CoreError::NotAvailable(tr!("The vault did not open").into()))?
        .map_err(vault_error)?;
    Ok((file, phrase))
}

fn vault_error(e: VaultError) -> CoreError {
    match e {
        VaultError::WrongPassword => CoreError::WrongPassword,
        other => CoreError::NotAvailable(format!(tr!("Vault: {other}"), other = other)),
    }
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
async fn clear_leftovers(paths: &Paths) -> bool {
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

/// The wallet to open: the one open last (or the first) of the index, else a new folder for
/// the onboarding — it becomes a wallet once its vault is written there.
fn opening_wallet(data_dir: &std::path::Path) -> String {
    wallets::load(data_dir)
        .current()
        .map(|r| r.id.clone())
        .unwrap_or_else(|| wallets::new_id(data_dir))
}

/// What deleted wallets (or onboardings left halfway) left in `wallets/<network>/`: folders
/// without a vault, other than `keep` — the one about to open. Best effort: a folder still held
/// is tried again at the next start or switch.
fn clear_other_leftovers(data_dir: &std::path::Path, keep: &str) {
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

/// Said when a deleted wallet's files are still held by this run of evolite.
const LEFTOVERS: &str = tr!(
    "The wallet is deleted, but its files are still held by this run of evolite. Restart evolite — they are deleted at start, and then you can make or restore a wallet"
);

/// How often a second window asks again whether the first one has closed.
const LOCK_RETRY: Duration = Duration::from_secs(3);

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
        };
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
        let mut started = clear_leftovers(&core.paths).await;
        core.leftovers = !started;
        // The window needs no SPV client to ask for the password: it gets what is known now.
        // Starting the client opens the stored headers and filters — 12 s in a release build
        // with a synced store, half a minute in a debug one — and the window said only
        // "Connecting to the wallet core…" all that time.
        let snapshot = core.snapshot().await;
        let _ = out.send(Event::Snapshot(Box::new(snapshot)));
        wake();
        if started {
            let _ = core.chain.start(&core.paths.chain, core.publisher()).await;
        }
        let mut last_start = tokio::time::Instant::now();
        loop {
            if !started && last_start.elapsed() >= RETRY_START {
                last_start = tokio::time::Instant::now();
                started = clear_leftovers(&core.paths).await;
                core.leftovers = !started;
                if started {
                    let _ = core.chain.start(&core.paths.chain, core.publisher()).await;
                }
            }
            if started
                && core.chain.status().phase == Phase::Error
                && last_start.elapsed() >= RETRY_START
            {
                last_start = tokio::time::Instant::now();
                let _ = core.chain.start(&core.paths.chain, core.publisher()).await;
            }
            // Platform address balances come with proofs: from the masternode lists on.
            if core.chain.platform_ready() {
                core.chain.start_platform_sync().await;
                core.connect_pool();
                core.build_contact_accounts();
            }
            loop {
                match inbox.try_recv() {
                    Ok(command) => {
                        let (event, changed) = core.handle(command).await;
                        answer(&out, &*wake, event);
                        if changed {
                            core.dirty.store(true, Ordering::Relaxed);
                        }
                        if core.restart || core.switch_to.is_some() {
                            // A deleted wallet, or another one asked for: this one is
                            // locked (its keys gone with this core), its chain stopped and its
                            // store let go; a new core opens the next wallet, as at start.
                            let deleted = core.restart.then(|| core.wallet.clone());
                            let next = core.switch_to.take();
                            let switch_password = core.switch_password.take();
                            // "Cancel" while adding goes back to this one.
                            let adding_from = core.adding_from.take();
                            let _ = core.lock().await;
                            core.chain.stop().await;
                            // A deleted wallet's pictures go too.
                            if let (Some(base), Some(id)) = (&art_base, &deleted) {
                                let _ = std::fs::remove_dir_all(base.join(id));
                            }
                            drop(core.artist.take());
                            let wallet = next.unwrap_or_else(|| opening_wallet(&data_dir));
                            clear_other_leftovers(&data_dir, &wallet);
                            core = Real::new(data_dir.clone(), wallet, out.clone(), wake.clone());
                            // Back at the wallet the adding started from: adding is over.
                            core.adding_from = adding_from.filter(|from| *from != core.wallet);
                            core.artist = Some(crate::art::Artist::start(
                                out.clone(),
                                wake.clone(),
                                art_for(&core.wallet),
                            ));
                            started = clear_leftovers(&core.paths).await;
                            core.leftovers = !started;
                            if started {
                                let _ = core.chain.start(&core.paths.chain, core.publisher()).await;
                            }
                            last_start = tokio::time::Instant::now();
                            // Figma 29: the password checked before the switch opens it now.
                            if let Some(password) = switch_password {
                                let result = core.unlock(&password).await;
                                let _ = out.send(Event::Done {
                                    what: "switch",
                                    result,
                                });
                            }
                            core.dirty.store(true, Ordering::Relaxed);
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        core.chain.stop().await;
                        return;
                    }
                }
            }
            if core.idle_too_long() {
                let result = core.lock().await;
                let _ = out.send(Event::Done {
                    what: "lock",
                    result,
                });
                core.dirty.store(true, Ordering::Relaxed);
            }
            if core.dirty.swap(false, Ordering::Relaxed) {
                let snapshot = core.snapshot().await;
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
fn answer(out: &Sender<Event>, wake: &(dyn Fn() + Send + Sync), event: Option<Event>) {
    if let Some(event) = event {
        let _ = out.send(event);
    }
    wake();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_without_a_new_snapshot_still_wakes_the_interface() {
        let (out, events) = std::sync::mpsc::channel();
        let woken = std::sync::atomic::AtomicUsize::new(0);
        let wake = || {
            woken.fetch_add(1, Ordering::Relaxed);
        };
        answer(
            &out,
            &wake,
            Some(Event::Done {
                what: "unlock",
                result: Err(CoreError::WrongPassword),
            }),
        );
        assert!(matches!(
            events.try_recv(),
            Ok(Event::Done {
                what: "unlock",
                result: Err(CoreError::WrongPassword)
            })
        ));
        assert_eq!(woken.load(Ordering::Relaxed), 1);
        // A command answered from inside `handle`, or by a snapshot later: woken all the same.
        answer(&out, &wake, None);
        assert_eq!(woken.load(Ordering::Relaxed), 2);
    }

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn a_resync_clears_the_chain_and_the_store_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("evolite-resync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::new(dir.clone(), TEST_WALLET);
        let wallets = paths.vault.parent().unwrap().to_owned();
        std::fs::create_dir_all(&wallets).unwrap();
        std::fs::create_dir_all(paths.chain.spv.join("headers")).unwrap();
        let store_wal = wallets.join("platform-wallet.db-wal");
        for file in [
            &paths.vault,
            &paths.journal,
            &paths.contact_addresses,
            &paths.chain.store,
            &store_wal,
        ] {
            std::fs::write(file, b"x").unwrap();
        }
        assert!(
            !resync_on_start(&paths).unwrap(),
            "nothing asked: nothing goes"
        );
        assert!(paths.chain.store.exists());

        std::fs::write(&paths.resync, b"").unwrap();
        assert!(resync_on_start(&paths).unwrap());
        assert!(!paths.chain.spv.exists() && !paths.chain.store.exists() && !store_wal.exists());
        assert!(paths.vault.exists() && paths.journal.exists() && paths.contact_addresses.exists());
        assert!(!paths.resync.exists(), "once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_data_folder_one_evolite() {
        let dir = std::env::temp_dir().join(format!("evolite-lock-{}", std::process::id()));
        let first = lock_data_dir(&dir).unwrap();
        assert!(first.is_some(), "the first window gets the folder");
        assert!(
            lock_data_dir(&dir).unwrap().is_none(),
            "a second one waits while the first is open"
        );
        drop(first);
        assert!(
            lock_data_dir(&dir).unwrap().is_some(),
            "and gets it once the first has closed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A wallet restored from its phrase has its established contacts in the store at once,
    /// before platform-wallet's sweep has queued their keys: a try that built nothing
    /// is not success, and the count is kept only once every contact has its account.
    #[test]
    fn a_restored_wallet_tries_its_contacts_accounts_until_all_are_built() {
        // Two established contacts read from the store; the queue was empty: none built.
        assert!(!contact_accounts_done(2, Some(0)));
        // The try failed (no network): not done either.
        assert!(!contact_accounts_done(2, None));
        // One of two: tried again.
        assert!(!contact_accounts_done(2, Some(1)));
        // Both: kept, no more tries until more contacts are established.
        assert!(contact_accounts_done(2, Some(2)));
    }

    #[test]
    fn a_dashpay_scan_shows_what_platform_has() {
        // No identity on Platform for the phrase: no O5, the onboarding goes on.
        assert_eq!(dashpay_scan_of(chain::DashPayFound::default()), None);
        let scan = dashpay_scan_of(chain::DashPayFound {
            identity: Some(("HFZQD1".into(), 2_391_207_000, 6)),
            name: Some("ferrowyn58".into()),
            contacts: 2,
        })
        .expect("an identity");
        assert_eq!(scan.identity_credits, 2_391_207, "credits as duffs");
        assert_eq!(
            (scan.identity_keys, scan.name.as_deref(), scan.contacts),
            (6, Some("ferrowyn58"), 2)
        );
        assert!(
            !scan.core_known,
            "Core balances come with the wallet's sync"
        );
    }

    #[test]
    fn identity_failures_read_as_the_interface_expects() {
        assert_eq!(
            identity_failure("Insufficient funds: need 100000".into()),
            CoreError::NotEnough
        );
        assert_eq!(
            identity_failure(tr!("The wallet has no identity").into()),
            CoreError::NotAvailable(tr!("Make an identity first").into())
        );
        assert_eq!(
            identity_failure("timeout".into()),
            CoreError::Failed("timeout".into())
        );
    }

    /// A lookup that is no longer current stops: locking bumps the number.
    #[tokio::test]
    async fn locking_ends_the_identity_lookup() {
        let mut core = core_with_vault("lookup", "pw");
        let before = core.discovery_run.load(Ordering::Relaxed);
        core.lock().await.unwrap();
        assert_ne!(core.discovery_run.load(Ordering::Relaxed), before);
        // Not unlocked: nothing to look for, the state stays as it was.
        core.discover_identities();
        assert_eq!(
            *core.identity_search.lock().unwrap(),
            IdentitySearch::Searching
        );
    }

    /// The wallet folder the tests use.
    const TEST_WALLET: &str = "0000beef";

    /// A core over a vault sealed with `password` in a fresh temp dir; nothing is started.
    fn core_with_vault(name: &str, password: &str) -> Real {
        let dir = std::env::temp_dir().join(format!("evolite-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::new(dir.clone(), TEST_WALLET);
        let meta = VaultMeta {
            network: network().into(),
            wallet_id: "ab".repeat(32),
            birth_height: 1_000,
            word_count: 12,
            created_at: 1_793_000_000,
        };
        let cheap = KdfParams {
            memory_kib: 256,
            iterations: 1,
            parallelism: 1,
        };
        let file = vault::seal(PHRASE, password, meta, cheap).unwrap();
        vault::save(&paths.vault, &file).unwrap();
        let (out, _) = std::sync::mpsc::channel();
        Real::new(dir, TEST_WALLET.into(), out, Arc::new(|| {}))
    }

    fn pw(text: &str) -> Password {
        Password::new(text.into())
    }

    /// Seals PHRASE (with its real `WalletId`) as wallet `id` of the data folder `dir`.
    fn seal_wallet(dir: &std::path::Path, id: &str) {
        let meta = VaultMeta {
            network: network().into(),
            wallet_id: hex(&chain::wallet_id_of(PHRASE).unwrap()),
            birth_height: 1_000,
            word_count: 12,
            created_at: 1_793_000_000,
        };
        let cheap = KdfParams {
            memory_kib: 256,
            iterations: 1,
            parallelism: 1,
        };
        let file = vault::seal(PHRASE, "pw", meta, cheap).unwrap();
        vault::save(&wallets::wallet_dir(dir, id).join("wallet.json"), &file).unwrap();
    }

    fn core_at(dir: &std::path::Path, id: &str) -> Real {
        let (out, _) = std::sync::mpsc::channel();
        Real::new(dir.to_owned(), id.into(), out, Arc::new(|| {}))
    }

    fn fresh_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evolite-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Each wallet's files are in its own folder: one wallet's store, journals and vault are
    /// never another's. Only headers and filters and the chosen Core node are shared.
    #[test]
    fn wallets_share_no_files_but_the_chain() {
        let dir = fresh_dir("paths");
        let (a, b) = (
            Paths::new(dir.clone(), "aaaa0001"),
            Paths::new(dir.clone(), "bbbb0002"),
        );
        let own = |p: &Paths| {
            vec![
                p.chain.store.clone(),
                p.vault.clone(),
                p.pending_names.clone(),
                p.journal.clone(),
                p.contact_addresses.clone(),
                p.hidden_requests.clone(),
                p.pool_history.clone(),
                p.resync.clone(),
                p.address_book.clone(),
                p.mempool.clone(),
                p.pool.clone(),
                p.connected_apps.clone(),
            ]
        };
        for path in own(&a) {
            assert!(
                path.starts_with(wallets::wallet_dir(&dir, "aaaa0001")),
                "{path:?}"
            );
        }
        for path in own(&b) {
            assert!(
                path.starts_with(wallets::wallet_dir(&dir, "bbbb0002")),
                "{path:?}"
            );
        }
        assert_eq!(a.chain.spv, b.chain.spv);
        assert_eq!(a.chain.core_node, b.chain.core_node);
    }

    /// A phrase already a wallet here is refused when another wallet is added, by name.
    #[tokio::test]
    async fn a_phrase_already_here_is_not_a_second_wallet() {
        let dir = fresh_dir("same");
        seal_wallet(&dir, "aaaa0001");
        let mut core = core_at(&dir, "bbbb0002");
        let phrase = Phrase::new(PHRASE.split_whitespace().map(str::to_owned).collect());
        let (event, _) = core
            .handle(Command::UsePhrase {
                phrase,
                dashpay: false,
            })
            .await;
        match event {
            Some(Event::PhraseAccepted(Err(CoreError::NotAvailable(text)))) => {
                assert!(text.contains("already on this computer"), "{text}")
            }
            other => panic!("{other:?}"),
        }
        assert!(core.pending.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Switching asks the loop for the other wallet and remembers it for the next start; an
    /// added wallet starts in a folder of its own, and "Cancel" goes back.
    #[tokio::test]
    async fn switching_and_adding_go_through_the_index() {
        let dir = fresh_dir("switch");
        seal_wallet(&dir, "aaaa0001");
        seal_wallet(&dir, "bbbb0002");
        let mut core = core_at(&dir, "aaaa0001");
        let open: Vec<bool> = core.wallet_entries().iter().map(|w| w.open).collect();
        assert_eq!(open, [true, false]);

        // Nowhere to go: the open one, or one not here.
        for id in ["aaaa0001", "cccc0003"] {
            core.handle(Command::SwitchWallet {
                id: id.into(),
                password: None,
            })
            .await;
            assert!(core.switch_to.is_none());
        }
        // Figma 29: with the other wallet's password, a wrong one changes nothing.
        let (event, _) = core
            .handle(Command::SwitchWallet {
                id: "bbbb0002".into(),
                password: Some(pw("wrong")),
            })
            .await;
        assert!(matches!(
            event,
            Some(Event::Done {
                what: "switch",
                result: Err(CoreError::WrongPassword)
            })
        ));
        assert!(core.switch_to.is_none());
        assert_eq!(wallets::load(&dir).current().unwrap().id, "aaaa0001");
        let (event, _) = core
            .handle(Command::SwitchWallet {
                id: "bbbb0002".into(),
                password: Some(pw("pw")),
            })
            .await;
        assert!(
            event.is_none(),
            "the new core answers once it has opened it"
        );
        assert_eq!(core.switch_to.as_deref(), Some("bbbb0002"));
        assert!(core.switch_password.is_some());
        // The other wallet's phrase by its own password (its "…" → "Show the phrase").
        let (event, _) = core
            .handle(Command::RevealPhrase {
                password: pw("pw"),
                wallet: Some("bbbb0002".into()),
            })
            .await;
        assert!(matches!(event, Some(Event::PhraseRevealed(Ok(_)))));
        assert_eq!(wallets::load(&dir).current().unwrap().id, "bbbb0002");

        let mut core = core_at(&dir, "bbbb0002");
        core.handle(Command::AddWallet).await;
        let new = core.switch_to.clone().unwrap();
        assert!(new != "aaaa0001" && new != "bbbb0002");
        assert_eq!(core.adding_from.as_deref(), Some("bbbb0002"));
        // Until its vault is written the new folder is no wallet: the index has two.
        assert_eq!(wallets::load(&dir).wallets.len(), 2);

        let mut adding = core_at(&dir, &new);
        adding.adding_from = core.adding_from.take();
        assert!(adding.snapshot().await.adding_wallet);
        adding.handle(Command::CancelAddWallet).await;
        assert_eq!(adding.switch_to.as_deref(), Some("bbbb0002"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn renames_a_wallet_within_24_characters() {
        let dir = fresh_dir("rename");
        seal_wallet(&dir, "aaaa0001");
        let mut core = core_at(&dir, "aaaa0001");
        let rename = |name: &str| Command::RenameWallet {
            id: "aaaa0001".into(),
            name: name.into(),
            color: 4,
        };
        for name in ["   ", &"ω".repeat(25)] {
            let (event, _) = core.handle(rename(name)).await;
            assert!(matches!(
                event,
                Some(Event::Done {
                    what: "wallet",
                    result: Err(_)
                })
            ));
        }
        let (event, _) = core.handle(rename(" Savings ")).await;
        assert!(matches!(
            event,
            Some(Event::Done {
                what: "wallet",
                result: Ok(())
            })
        ));
        assert_eq!(wallets::load(&dir).wallets[0].name, tr!("Savings"));
        assert_eq!(wallets::load(&dir).wallets[0].color, 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "Check the phrase" on a real vault: the password opens it, the words come with the right
    /// one among the choices, the right picks pass, a wrong one is named, a check is answered
    /// once, and a lock forgets an unanswered one.
    #[tokio::test]
    async fn checks_the_phrase_without_giving_it_out() {
        let mut core = core_with_vault("check", "pw");
        let words: Vec<&str> = PHRASE.split_whitespace().collect();
        let ask = |password: &str| Command::AskPhraseCheck {
            password: pw(password),
        };
        let (event, _) = core.handle(ask("wrong")).await;
        assert!(matches!(
            event,
            Some(Event::PhraseCheck(Err(CoreError::WrongPassword)))
        ));
        let questions = |event: Option<Event>| match event {
            Some(Event::PhraseCheck(Ok(q))) => q,
            other => panic!("{other:?}"),
        };
        let asked = questions(core.handle(ask("pw")).await.0);
        assert_eq!(asked.len(), 4);
        let right: Vec<String> = asked
            .iter()
            .map(|q| words[q.number - 1].to_owned())
            .collect();
        let (event, _) = core
            .handle(Command::CheckPhraseWords {
                picks: right.clone(),
            })
            .await;
        assert!(matches!(event, Some(Event::PhraseChecked { wrong: None })));
        // Answered once: the same picks again need a new check.
        let (event, _) = core
            .handle(Command::CheckPhraseWords { picks: right })
            .await;
        assert!(matches!(event, Some(Event::PhraseCheck(Err(_)))));

        let asked = questions(core.handle(ask("pw")).await.0);
        let mut picks: Vec<String> = asked
            .iter()
            .map(|q| words[q.number - 1].to_owned())
            .collect();
        picks[1] = asked[1]
            .choices
            .iter()
            .find(|c| **c != picks[1])
            .unwrap()
            .clone();
        let (event, _) = core.handle(Command::CheckPhraseWords { picks }).await;
        assert!(
            matches!(event, Some(Event::PhraseChecked { wrong: Some(n) }) if n == asked[1].number)
        );

        questions(core.handle(ask("pw")).await.0);
        core.lock().await.unwrap();
        assert!(
            core.phrase_check.is_none(),
            "a lock forgets the right words"
        );
    }

    #[tokio::test]
    async fn changes_the_password_and_keeps_the_phrase() {
        let mut core = core_with_vault("password", "old one");
        assert_eq!(
            core.change_password(&pw("wrong"), &pw("new one!")).await,
            Err(CoreError::WrongPassword)
        );
        assert!(core.change_password(&pw("old one"), &pw("")).await.is_err());
        // The core holds the floor itself, whatever screen sent the command.
        assert_eq!(
            core.change_password(&pw("old one"), &pw("1234567")).await,
            Err(CoreError::NotAvailable(
                tr!("The new password needs at least 8 characters").into()
            ))
        );
        core.change_password(&pw("old one"), &pw("new one!"))
            .await
            .unwrap();
        assert_eq!(
            core.open_vault(&pw("old one")).await.err(),
            Some(CoreError::WrongPassword)
        );
        let (file, phrase) = core.open_vault(&pw("new one!")).await.unwrap();
        assert_eq!(phrase.as_str(), PHRASE);
        assert_eq!(file.meta.birth_height, 1_000);
        // Sealed with the full cost again, not the cheap one it had.
        assert_eq!(file.kdf, KdfParams::default());
    }

    #[tokio::test]
    async fn a_wrong_password_signs_nothing() {
        let mut core = core_with_vault("send", "right");
        let meta = core.saved.clone().unwrap();
        core.unlocked = Some(Unlocked {
            id: [0; 32],
            meta,
            keys: None,
            seed: None,
        });
        let to = "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ";
        assert_eq!(
            core.send(Source::Core, to, 10_000, Some(&pw("wrong")))
                .await,
            Err(CoreError::WrongPassword)
        );
        // The right one gets past the check, to the next one: the wallet is not synced.
        assert_eq!(
            core.send(Source::Core, to, 10_000, Some(&pw("right")))
                .await,
            Err(CoreError::NotSynced)
        );
    }

    #[test]
    fn platform_addresses_only_of_the_network_in_force() {
        use evolite_api::Network::{Mainnet, Testnet};
        let address = |network| PlatformAddress::P2pkh([7; 20]).to_bech32m_string(network);
        let (main, test) = (
            address(key_wallet::Network::Mainnet),
            address(key_wallet::Network::Testnet),
        );
        assert!(platform_address_on(&test, Testnet) && !platform_address_on(&main, Testnet));
        // On mainnet its own addresses go through and testnet's are refused — before, the
        // other way round.
        assert!(platform_address_on(&main, Mainnet) && !platform_address_on(&test, Mainnet));
        assert!(!platform_address_on("tdash1 not an address", Testnet));
        assert!(main.starts_with(hrp_of(Mainnet)) && !main.starts_with(hrp_of(Testnet)));
        assert!(test.starts_with(hrp_of(Testnet)) && !test.starts_with(hrp_of(Mainnet)));
    }

    #[test]
    fn app_links_name_the_network_in_force() {
        use evolite_api::Network::{Mainnet, Testnet};
        assert_eq!(app_connect::link_network(Testnet), Some("t"));
        // The key-exchange contract is testnet's: no app sign-in on mainnet yet.
        assert_eq!(app_connect::link_network(Mainnet), None);
    }

    #[tokio::test]
    async fn credits_refuse_bad_payees_before_asking_the_network() {
        let mut core = core_with_vault("credits", "pw");
        let meta = core.saved.clone().unwrap();
        core.unlocked = Some(Unlocked {
            id: [0; 32],
            meta,
            keys: None,
            seed: None,
        });
        let address = |network| PlatformAddress::P2pkh([7; 20]).to_bech32m_string(network);
        let mainnet = address(key_wallet::Network::Mainnet);
        let testnet = address(crate::network::dash());
        assert!(testnet.starts_with("tdash1"), "{testnet}");
        assert_eq!(
            core.send(Source::Credits, &mainnet, 1_000, None).await,
            Err(CoreError::BadAddress)
        );
        assert_eq!(
            core.send(Source::Credits, "@a_b", 1_000, None).await,
            Err(CoreError::BadName)
        );
        assert_eq!(
            core.send(Source::Credits, &testnet, 0, None).await,
            Err(CoreError::BadAmount)
        );
        // A good payee gets as far as the network: proofs cannot be checked yet.
        assert_eq!(
            core.send(Source::Credits, &testnet, 1_000, None).await,
            Err(CoreError::NotSynced)
        );
        // From Core a Platform address is refused with the reason, not as a bad address.
        assert!(matches!(
            core.send(Source::Core, &testnet, 1_000, None).await,
            Err(CoreError::NotAvailable(_))
        ));
    }

    #[test]
    fn counts_what_a_write_cost_in_duffs() {
        assert_eq!(spent_credits(Some(5_000_000), Some(4_000_000)), Some(1_000));
        // Unknown, or the balance did not go down (a refresh had not landed): not guessed.
        assert_eq!(spent_credits(None, Some(1)), None);
        assert_eq!(spent_credits(Some(1_000), Some(1_000)), None);
        assert_eq!(spent_credits(Some(1_000), Some(2_000)), None);
    }

    #[test]
    fn asks_a_contact_only_once() {
        assert!(repeat_request(Some(ContactState::Established), "bob").is_some());
        assert!(repeat_request(Some(ContactState::Outgoing), "bob").is_some());
        // Their request is answered with ours; a stranger gets one.
        assert_eq!(repeat_request(Some(ContactState::Incoming), "bob"), None);
        assert_eq!(repeat_request(None, "bob"), None);
    }

    #[tokio::test]
    async fn a_contact_is_paid_only_to_the_address_shown() {
        let (out, events) = std::sync::mpsc::channel();
        let mut core = core_with_vault("preview", "pw");
        core.out = out;
        let meta = core.saved.clone().unwrap();
        core.unlocked = Some(Unlocked {
            id: [0; 32],
            meta,
            keys: None,
            seed: None,
        });
        // An address is its own preview.
        let address = "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ";
        assert_eq!(core.preview_payment(address.into()), Ok(()));
        match events.try_recv() {
            Ok(Event::PaymentPreview { to, result }) => {
                assert_eq!(to, address);
                assert_eq!(result.as_deref(), Ok(address));
            }
            other => panic!("{other:?}"),
        }
        // A name that is not a contact has no address from a key.
        assert!(matches!(
            core.preview_payment("@bob".into()),
            Err(CoreError::NotAvailable(_))
        ));
        // A contact whose address was not shown is not paid.
        core.contacts = vec![Contact {
            identity_id: "F5teLR3Vq4JjzQA7jgdGjZQvJbKpMubbu5dsC4MJd1Eh".into(),
            name: Some("bob".into()),
            profile: Profile::default(),
            state: ContactState::Established,
            since: 0,
            payments: 0,
            last_payment: None,
        }];
        assert!(matches!(
            core.send(Source::Core, "@bob", 1_000, None).await,
            Err(CoreError::NotAvailable(_))
        ));
        // A contact without a name is paid by its identity id, not taken for an address.
        core.contacts[0].name = None;
        assert!(matches!(
            core.send(
                Source::Core,
                "F5teLR3Vq4JjzQA7jgdGjZQvJbKpMubbu5dsC4MJd1Eh",
                1_000,
                None
            )
            .await,
            Err(CoreError::NotAvailable(_))
        ));
    }

    #[tokio::test]
    async fn names_wait_for_the_masternode_lists() {
        let dir = std::env::temp_dir().join(format!("evolite-test-names-{}", std::process::id()));
        let (out, answers) = std::sync::mpsc::channel();
        let mut core = Real::new(dir, TEST_WALLET.into(), out, Arc::new(|| {}));
        // No proof can be checked yet: "not synced" now, the answer later from the task.
        let (event, _) = core
            .handle(Command::CheckName {
                label: "@Evolite2".into(),
            })
            .await;
        assert!(event.is_none());
        match answers.try_recv() {
            Ok(Event::NameChecked { label, result }) => {
                assert_eq!(label, "@Evolite2", "the label comes back as asked");
                assert_eq!(result, Err(CoreError::NotSynced));
            }
            other => panic!("{other:?}"),
        }
        // A contested name (letters only, under 20) is said at once, with no wait.
        let (event, _) = core
            .handle(Command::CheckName {
                label: "@Evolite".into(),
            })
            .await;
        assert!(
            matches!(
                &event,
                Some(Event::NameChecked {
                    label,
                    result: Ok(NameCheck::Contested),
                }) if label == "@Evolite"
            ),
            "{event:?}"
        );
        // Not a name at all: refused at once, nothing waits.
        let (event, _) = core
            .handle(Command::ResolveName {
                name: "@a_b".into(),
            })
            .await;
        assert!(matches!(
            event,
            Some(Event::NameResolved {
                result: Err(CoreError::BadName),
                ..
            })
        ));
        assert!(answers.try_recv().is_err());
    }

    /// A disconnect is answered as `disconnect`, as the demo does: the screen takes `app` for the
    /// answer to the request it shows.
    #[tokio::test]
    async fn a_disconnect_is_not_taken_for_an_answer_to_a_request() {
        let mut core = core_with_vault("disconnect", "pw");
        let (event, _) = core
            .handle(Command::DisconnectApp { id: "app".into() })
            .await;
        assert!(matches!(
            event,
            Some(Event::Done {
                what: "disconnect",
                ..
            })
        ));
    }

    /// An app's link is refused at once when it is not one or is for another network, and a
    /// request is never approved that the person was not shown.
    #[tokio::test]
    async fn app_links_are_checked_before_anything_is_signed() {
        let mut core = core_with_vault("app-links", "pw");
        let request = |text: &str| Command::OpenAppLink { text: text.into() };
        let (event, _) = core.handle(request("https://evil.example")).await;
        assert!(matches!(
            event,
            Some(Event::AppRequest(Err(CoreError::BadLink)))
        ));
        let (event, _) = core
            .handle(request("dash-key:3mJr7AoUXx2Wqd?n=m&v=1"))
            .await;
        assert!(matches!(
            event,
            Some(Event::AppRequest(Err(CoreError::NotAvailable(_))))
        ));
        // Declining is always answered; approving a request never shown is refused.
        let answer = |id, approve| Command::AnswerApp { id, approve };
        let (event, _) = core.handle(answer(7, false)).await;
        assert!(matches!(
            event,
            Some(Event::Done {
                what: "app",
                result: Ok(())
            })
        ));
        let (event, _) = core.handle(answer(7, true)).await;
        assert!(matches!(
            event,
            Some(Event::Done {
                what: "app",
                result: Err(CoreError::NotAvailable(_))
            })
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn locks_after_the_idle_time_only_when_open() {
        let mut core = core_with_vault("idle", "pw");
        core.auto_lock = Some(Duration::from_secs(300));
        tokio::time::advance(Duration::from_secs(301)).await;
        assert!(!core.idle_too_long(), "nothing to lock");
        let meta = core.saved.clone().unwrap();
        core.unlocked = Some(Unlocked {
            id: [0; 32],
            meta,
            keys: None,
            seed: None,
        });
        assert!(core.idle_too_long());
        core.last_touch = tokio::time::Instant::now();
        assert!(!core.idle_too_long());
        core.auto_lock = None;
        tokio::time::advance(Duration::from_secs(10_000)).await;
        assert!(!core.idle_too_long(), "\"on close\": never by time");
    }
}
