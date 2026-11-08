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
mod attempts;
mod avatars;
mod chain;
mod connected_apps;
mod contact_addresses;
mod convert;
mod core_balance;
mod dashpay;
mod hidden_requests;
mod identity;
mod journal;
mod json_file;
mod mempool;
mod mined;
mod nfts;
mod node;
mod notes;
mod outgoing;
mod pending;
mod platform_history;
mod platform_sending;
mod pool;
mod pool_history;
mod provider;
mod rates;
mod security;
mod store;
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
use tokio::sync::watch;
use zeroize::Zeroizing;

use crate::dpns;
use chain::{Chain, ChainPaths, CreditPayee};
use convert::{
    activity, coins_known, count_checked, fees, not_an_address, payee, platform_fees, sync_info,
};
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
/// How long making a wallet waits for a chain start under way: the new wallet's birth height
/// is the tip of the stored headers, and without them it would be scanned from the genesis.
const START_WAIT: Duration = Duration::from_secs(60);
/// How long a chain start that failed right after the chain before it stopped is tried again.
const START_AGAIN: Duration = Duration::from_secs(15);
/// A contact's DPNS name that could not be read is asked again after this long.
const NAME_RETRY: Duration = Duration::from_secs(30);

/// A Platform read that failed, as the UI is told: "wait for the sync" when it was not made
/// because the masternode lists are not in yet ([`Chain::ready`]).
fn read_failure(e: String) -> CoreError {
    if e == chain::NOT_READY {
        CoreError::NotSynced
    } else {
        CoreError::Failed(e)
    }
}

/// Where the core keeps its files, per network: the SPV storage, platform-wallet's store and
/// the vault.
#[derive(Clone)]
pub struct Paths {
    /// The data folder itself: "Disk space" counts all of it.
    pub root: PathBuf,
    pub chain: ChainPaths,
    pub vault: PathBuf,
    /// Names asked for identities still being made ([`pending::PendingNames`]).
    pub pending_names: PathBuf,
    /// What the wallet did on Platform ([`journal::Journal`]).
    pub journal: PathBuf,
    /// How far Platform was read back into the journal ([`platform_history::Marks`]).
    pub platform_sync: PathBuf,
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
    /// The block each Core transaction was seen in ([`mined::Mined`]).
    pub mined: PathBuf,
    /// The last checked Core balance and its time ([`core_balance::CoreBalance`], BUG-77).
    pub core_balance: PathBuf,
    /// The shielded pool's note store and commitment tree ([`pool`]).
    pub pool: PathBuf,
    /// Apps signed in as an identity ([`connected_apps::ConnectedApps`]).
    pub connected_apps: PathBuf,
    /// The person's payment notes, encrypted ([`notes::Notes`], FEAT-0003): only here; a
    /// resync keeps them, deleting the wallet deletes them with its folder.
    pub notes: PathBuf,
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
            platform_sync: wallets.join("platform-sync.json"),
            contact_addresses: wallets.join("contact-addresses.json"),
            hidden_requests: wallets.join("hidden-requests.json"),
            pool_history: wallets.join("pool-history.json"),
            resync: wallets.join("resync-on-start"),
            address_book: wallets.join("address-book.json"),
            mempool: wallets.join("mempool-seen.json"),
            mined: wallets.join("mined.json"),
            core_balance: wallets.join("core-balance.json"),
            pool: wallets.join("shielded-pool.db"),
            connected_apps: wallets.join("connected-apps.json"),
            notes: wallets.join("notes.enc"),
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

/// The wall clock in Unix seconds, as the wrong-password count keeps it (FEAT-0002).
fn wall_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
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
    // Still starting (F3): the wallet is open, its sync not yet — "wait for the sync".
    if !chain.running() {
        return Err(CoreError::NotSynced);
    }
    chain
        .wallet(id)
        .await
        .ok_or_else(|| CoreError::NotAvailable(NOT_LOADED.into()))
}

/// What a payment from Core may spend: the wallet's coins, each once (BUG-77) — not
/// platform-wallet's `balance()`, which counts a coin two accounts hold twice. The callers ask
/// [`coins_known`] first: an unchecked balance is no basis for a payment.
async fn core_pays(wallet: &platform_wallet::PlatformWallet, amount: u64) -> Result<(), CoreError> {
    core_covers(&wallet.state().await.core_wallet, amount)
}

/// [`core_pays`] on the wallet's state: [`CoreError::NotEnough`] when its coins, each once, are
/// less than `amount`.
fn core_covers(info: &key_wallet::wallet::ManagedWalletInfo, amount: u64) -> Result<(), CoreError> {
    if (convert::core_balance(info).max(0) as u64) < amount {
        return Err(CoreError::NotEnough);
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The chain of the wallet a wallet is being added from, kept running while the onboarding
/// lasts (BUG-76): watch-only, its keys gone with its core's lock. "Back" gives it to the next
/// core of that wallet as it is — no stop, no second start of the SPV client (each start reads
/// the stored headers, filters and masternode lists again: 14–55 s in a debug build). Any other
/// way out of the onboarding stops it first ([`Real::start_after`]). The onboarding starts no
/// chain of its own until its wallet is written: the SPV files are shared, and two clients
/// cannot hold them.
///
/// With it goes what describes this chain rather than its core: its start under way and what
/// it found, the flag its sync callbacks raise, how far it was asked to rescan (BUG-64) and what
/// that rescan is owed for, and what was set up in its manager (the pool, contacts' accounts).
struct Parked {
    wallet: String,
    chain: Arc<Chain>,
    starting: Option<tokio::task::JoinHandle<bool>>,
    stopped: Option<watch::Receiver<bool>>,
    start_tried: bool,
    leftovers: bool,
    last_start: tokio::time::Instant,
    dirty: Arc<AtomicBool>,
    rescanned_from: Option<u32>,
    /// A rescan asked and not finished yet: noted after "Back" (BUG-82 review r1 should 1).
    rescan_owed: Option<OwedRescan>,
    pool_wallet: Arc<std::sync::Mutex<Option<WalletId>>>,
    contact_accounts_for: Arc<std::sync::atomic::AtomicUsize>,
    contact_accounts_busy: Arc<AtomicBool>,
}

struct Real {
    paths: Paths,
    /// This core's wallet: its folder in `wallets/<network>/`.
    wallet: String,
    /// The wallets of this computer (the index, checked against the folders when read).
    index: wallets::Index,
    /// Their wrong passwords in a row, for `Unlock` and `SwitchWallet` only (FEAT-0002).
    ///
    /// Read from the files with each new `Real`, so a switch of wallets starts it afresh: the
    /// monotonic clock of the last wrong passwords (R-31, a recommendation, not an AC) is lost
    /// then, and a clock set forward after a switch ends a pause early, as after a restart
    /// (review r1, should 2). The counts and ends on the disk stay.
    attempts: attempts::Attempts,
    /// How the pending phrase came (new, typed, from DashPay): kept in the index on create.
    pending_origin: wallets::Origin,
    /// The wallet to open next, asked by `SwitchWallet` (or a new one by `AddWallet`): the loop
    /// then makes a new core over it, as after a deletion.
    switch_to: Option<String>,
    /// The vault of `switch_to` as `SwitchWallet` opened it with its password: the new core
    /// takes it as it is, without a second Argon2 (F2, BUG-24). Gone with this core.
    switch_opened: Option<(vault::VaultFile, Zeroizing<String>)>,
    /// While a wallet is being added: the one to go back to on "Cancel".
    adding_from: Option<String>,
    /// While a wallet is being added: that one's chain, still running (BUG-76).
    parked: Option<Parked>,
    /// "Password for every send" of this core's wallet ([`security`]): every [`Spend`] passes
    /// [`Real::spend_gate`] first.
    password_each_send: bool,
    chain: Arc<Chain>,
    /// The chain's start off the loop ([`Real::start_chain`]); it answers whether the wallet's
    /// folder was clear of a deleted wallet's files (else [`Real::leftovers`]).
    starting: Option<tokio::task::JoinHandle<bool>>,
    /// Turns true once every chain before this core's has let its store go: the next core's
    /// start may open its own store then (BUG-65, [`Real::start_chain`]).
    stopped: Option<watch::Receiver<bool>>,
    /// A start was tried over a clear folder (the loop then retries a failed one).
    start_tried: bool,
    /// When the last start was asked.
    last_start: tokio::time::Instant,
    /// The phrase of the open wallet until platform-wallet has it: unlocking does not wait for
    /// the chain (F3, BUG-24); [`Real::attach_wallet`] opens it there once the chain runs.
    attach: Option<Zeroizing<String>>,
    /// When opening it there failed last: tried again after [`RETRY_START`].
    attach_failed: Option<tokio::time::Instant>,
    /// The headers' tip the core before this one saw: a wallet made here before this chain has
    /// headers is born there.
    birth_hint: Option<u32>,
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
    /// The block each Core transaction was seen in, for one the wallet took out of it (BUG-64).
    mined: mined::Mined,
    /// The last checked Core balance, shown while the coins are not known (BUG-77).
    core_balance: core_balance::CoreBalance,
    /// The lowest height the filters were asked to be scanned again from in this run (BUG-64).
    rescanned_from: Option<u32>,
    /// A rescan asked in this run and not finished yet (review r2 should 1).
    rescan_owed: Option<OwedRescan>,
    /// Lock after this long without a `Touch`; `None`: only when the app closes.
    auto_lock: Option<Duration>,
    last_touch: tokio::time::Instant,
    /// Profile pictures, downloaded and checked here, drawn by `artist`.
    avatars: avatars::Avatars,
    /// The identity's NFTs from Platform, with their pictures.
    nfts: nfts::Nfts,
    /// The sandboxed renderer; `None` in tests.
    artist: Option<crate::art::Artist>,
    /// Where each wallet's picture cache is (`<base>/<id>`): another wallet's deleted with it.
    art_base: Option<PathBuf>,
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
    /// That lookup was complete: every slot answered (BUG-9). Until then a lock without its
    /// identity may belong to one in an unanswered slot.
    identities_complete: Arc<AtomicBool>,
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
    /// The asset locks the store keeps as consumed (BUG-57 r1), read with the history.
    consumed_locks: std::collections::HashSet<String>,
    /// The top-up addresses of the identities' slots, derived from the keys, for the locks a
    /// restore left without a top-up account (BUG-57 follow-up).
    top_ups: identity::TopUpScripts,
    history_read: Option<(usize, tokio::time::Instant)>,
    /// Core payments of this run: who they went to, and those the history does not show yet.
    outgoing: Arc<std::sync::Mutex<outgoing::Outgoing>>,
    /// Sends on Platform of this run (credits, the pool) until the history has their lines.
    platform_sending: Arc<std::sync::Mutex<platform_sending::PlatformSending>>,
    /// What the wallet did on Platform, for the history.
    journal: Arc<journal::Journal>,
    /// A pass reading Platform back into the journal is running (one at a time).
    history_busy: Arc<AtomicBool>,
    /// When the next pass is due; `None`: as soon as the identities are known.
    history_next: Option<std::time::Instant>,
    /// Addresses of contacts' keys already paid: never paid again, after a restart too.
    contact_paid: Arc<contact_addresses::ContactAddresses>,
    /// Senders whose requests were hidden: still hidden after a resync.
    hidden: Arc<hidden_requests::HiddenRequests>,
    /// The pool's operations with the amounts the scan loses after a restart.
    pool_history: pool_history::PoolHistory,
    /// The open wallet's payment notes, decrypted, with their key (FEAT-0003): opened with the
    /// wallet, `None` once it locks (a lock, a switch, "Add a wallet"). Shared with the sends
    /// in flight, which keep their note once the network takes them.
    notes: Arc<std::sync::Mutex<Option<notes::Notes>>>,
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
        /// The identity the request showed in «Sign in as» (BUG-29).
        identity: [u8; 32],
    },
    /// `dash-st:`: add the keys of the sign-in to this app (contract, generation); the app's
    /// `identity` is the one the request was shown for.
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

/// Why a note was not saved, for "Details": the kind of failure, never the note.
fn note_error(e: notes::NoteError) -> CoreError {
    match e {
        notes::NoteError::TooMany => CoreError::NotAvailable(crate::note_text::TOO_MANY.into()),
        notes::NoteError::NotRead => CoreError::NotAvailable(
            tr!(
                "The notes file was not read: notes can be changed once the wallet is opened again"
            )
            .into(),
        ),
        notes::NoteError::Write(kind) => CoreError::Failed(format!("notes.enc: {kind}")),
    }
}

/// A send's note as the core keeps it: cleaned, `None` when empty; too long refuses the send
/// before anything leaves the wallet (the form cuts it to what a note holds).
fn send_note(note: Option<NoteText>) -> Result<Option<String>, CoreError> {
    match note {
        None => Ok(None),
        Some(note) => crate::note_text::kept(&note.0)
            .map_err(|_| CoreError::NotAvailable(crate::note_text::TOO_LONG.into())),
    }
}

/// Keeps a send's note once the network took it (FEAT-0003, AC-T3), off the loop's thread and
/// before the send's answer goes out. A wallet locked meanwhile keeps nothing: its key is gone.
/// A failed write is in the journal by the file's name and the kind of error only.
async fn keep_note(
    notes: Arc<std::sync::Mutex<Option<notes::Notes>>>,
    keep: impl FnOnce(&mut notes::Notes) + Send + 'static,
) {
    let _ = tokio::task::spawn_blocking(move || {
        if let Some(notes) = notes.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            keep(notes);
        }
    })
    .await;
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

/// The fee a Core payment of `amount` is shown with while it is sent: as the coins it takes
/// make it (the same estimate as the Send screen's, `Fees::core_payment_for`), not as spending
/// every coin of the wallet would (BUG-55 review, should 2).
fn payment_fee(coin_values: Vec<u64>, amount: Duffs) -> Duffs {
    let mut estimate = fees(coin_values.len());
    estimate.core_coins = convert::core_coins(coin_values);
    estimate.core_payment_for(amount)
}

/// How often Platform is read back into the journal while a wallet is open (R-33).
const HISTORY_EVERY: Duration = Duration::from_secs(10 * 60);
/// The first pause before an identity lookup is tried again, and the longest one. Short at
/// first: a scan with unanswered slots or names is tried again soon (BUG-9).
const DISCOVERY_RETRY: Duration = Duration::from_secs(10);
const DISCOVERY_RETRY_MAX: Duration = Duration::from_secs(5 * 60);

/// A rescan asked in this run and not finished yet (BUG-64 review r2 should 1). platform-wallet
/// rewinds the wallet's filter height in memory only, and the climb back is the rescan; what it
/// is for is noted on disk only once the wallet is back at `up_to`, where it stood when asked —
/// a run closed before that asks again at the next start.
#[derive(Debug, PartialEq)]
struct OwedRescan {
    wallet: WalletId,
    up_to: u32,
    /// Lines of `mempool-seen.json` and of `mined.json` it is for.
    stale: Vec<String>,
    lost: Vec<String>,
}

impl OwedRescan {
    /// One more rescan asked for `wallet` at height `synced`. One owed for another wallet is
    /// dropped: it is asked again in a later run.
    fn add(
        owed: &mut Option<Self>,
        wallet: WalletId,
        synced: u32,
        stale: Vec<String>,
        lost: Vec<String>,
    ) {
        let o = match owed {
            Some(o) if o.wallet == wallet => o,
            _ => owed.insert(Self {
                wallet,
                up_to: synced,
                stale: Vec::new(),
                lost: Vec::new(),
            }),
        };
        o.up_to = o.up_to.max(synced);
        o.stale.extend(stale);
        o.lost.extend(lost);
    }

    /// The owed rescan, taken, when `wallet`'s filters are at `synced` and that is where it
    /// was asked from or above.
    fn finished(owed: &mut Option<Self>, wallet: &WalletId, synced: u32) -> Option<Self> {
        owed.take_if(|o| o.wallet == *wallet && synced >= o.up_to)
    }
}

/// What one pass of the identity search means for it (BUG-9).
#[derive(Debug, PartialEq)]
struct DiscoveryStep {
    /// For the snapshot.
    status: IdentitySearch,
    /// The phrase's identities were looked up: shown with their balances kept fresh, and a
    /// tracked top-up lock shown as unfinished.
    found: bool,
    /// Look again after the pause.
    again: bool,
}

/// [`DiscoveryStep`] of a pass, `found_before` by an earlier pass of the same search. A pass
/// with slots or names missing shows what it found and goes on searching; only a complete one
/// is done. A failed pass is told only while no identity was found: beside identities the
/// search just goes on.
fn discovery_step(pass: Result<chain::Discovery, String>, found_before: bool) -> DiscoveryStep {
    match pass {
        Ok(pass) => DiscoveryStep {
            status: if pass.complete {
                IdentitySearch::Done
            } else {
                IdentitySearch::Searching
            },
            found: true,
            again: !pass.complete,
        },
        Err(why) => DiscoveryStep {
            status: if found_before {
                IdentitySearch::Searching
            } else {
                IdentitySearch::Failed(why)
            },
            found: found_before,
            again: true,
        },
    }
}

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
        || text == chain::top_up_settled_text()
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

    fn pending_names(&self) -> pending::PendingNames {
        pending::PendingNames::new(self.paths.pending_names.clone())
    }

    /// Asks the wallet to scan its compact filters again from `from` (BUG-64): a block it
    /// took a transaction out of is matched and applied again, and the store gets it. Once
    /// the sync is done, and only below the lowest height asked in this run, so it never
    /// repeats itself; the journal says why (`lost_block` or `stale`). What it is for — the
    /// `stale` lines of the mempool file, the `lost` ones of `mined.json` — is owed until the
    /// wallet's filters are back at `synced`, where they stand now ([`OwedRescan`]).
    async fn rescan_from(
        &mut self,
        id: WalletId,
        from: u32,
        why: &'static str,
        synced: u32,
        stale: Vec<String>,
        lost: Vec<String>,
    ) {
        if self.chain.status().phase != sync::Phase::Synced
            || self.rescanned_from.is_some_and(|asked| asked <= from)
        {
            return;
        }
        let Ok(manager) = self.chain.manager().await else {
            return;
        };
        self.rescanned_from = Some(from);
        crate::diag::record(crate::diag::Record::Answer {
            what: "rescan",
            outcome: why,
        });
        let asked = matches!(
            tokio::task::spawn_blocking(move || manager.spv_rescan_filters_blocking(&id, from))
                .await,
            Ok(true)
        );
        if asked {
            OwedRescan::add(&mut self.rescan_owed, id, synced, stale, lost);
        }
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
        let read = tokio::task::spawn_blocking(move || {
            (
                chain::stored_history(&store, &id),
                chain::consumed_locks(&store, &id),
            )
        })
        .await;
        if let Ok((records, consumed)) = read {
            if let Ok(records) = records {
                self.stored_history = records;
            }
            if let Ok(consumed) = consumed {
                self.consumed_locks = consumed;
            }
        }
    }

    /// What a cut-short "Create an identity" left, if anything: a lock without its identity, or
    /// an identity without the name this computer remembers for it. Nothing while one is being
    /// made: its lock exists before Platform has the identity. Nor before a complete lookup
    /// (BUG-9): the identity may sit in a slot Platform did not answer for.
    async fn unfinished_identity(
        &self,
        wallet: &platform_wallet::PlatformWallet,
    ) -> Option<UnfinishedIdentity> {
        if self.identity_busy.load(Ordering::Relaxed)
            || !self.identities_complete.load(Ordering::Relaxed)
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
                identity::finish_resumed(&pending, id, index)
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
        core_pays(&wallet, amount).await?;
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
                        ..Default::default()
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
        self.answer_later(async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let to = match chain.resolve_name(&label).await {
                    Ok(Some(to)) => to,
                    Ok(None) => return Err(CoreError::NameNotFound),
                    Err(e) => return Err(read_failure(e)),
                };
                let to = Identifier::from_string(&to, Encoding::Base58)
                    .map_err(|e| CoreError::Failed(e.to_string()))?;
                // By platform-wallet's state of the identity the request goes from, now: the
                // snapshot may be of the one shown before "Act as", and a repeat is paid (BUG-13).
                let (from, standing) = Chain::contact_standing(&wallet, &to)
                    .await
                    .map_err(CoreError::Failed)?;
                if let Some(why) = repeat_request(standing, &label) {
                    return Err(CoreError::NotAvailable(why));
                }
                // Checked, signed, paid and keyed as that one identity, whatever "Act as"
                // shows meanwhile (BUG-13 r1 should 1).
                let before = chain.identity_credits(&wallet, &from, false).await;
                chain
                    .add_contact(&wallet, &keys, &from, to)
                    .await
                    .map_err(dashpay_failure)?;
                // Keyed as the read of Platform keys it (BUG-58): one line, not two.
                journal.add(platform_history::sent_request(
                    &from,
                    &to,
                    &label,
                    now(),
                    spent_credits(before, chain.identity_credits(&wallet, &from, true).await)
                        .unwrap_or(0),
                ));
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
                            ..Default::default()
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
                tr!("Name up to 25 characters, “About” up to 140").into(),
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
                    ..Default::default()
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

    /// Reads Platform back into the journal off the loop (BUG-58, R-33): first once the
    /// wallet's identities are known and proofs can be checked, then every [`HISTORY_EVERY`];
    /// at once again after a new identity lookup. One pass at a time. Its result is kept only
    /// for the wallet and the unlock it was started for. A failed pass (the network, or a
    /// journal that cannot be written) leaves the marks where they were and is tried again
    /// [`HISTORY_EVERY`] later.
    fn read_platform_history(&mut self) {
        let Some(id) = self.unlocked.as_ref().map(|o| o.id) else {
            self.history_next = None;
            return;
        };
        if !self.identities_complete.load(Ordering::Relaxed) {
            // A lookup is running: the pass after it comes at once.
            self.history_next = None;
            return;
        }
        if !self.chain.platform_ready()
            || self.history_busy.load(Ordering::Relaxed)
            || self
                .history_next
                .is_some_and(|due| std::time::Instant::now() < due)
        {
            return;
        }
        self.history_next = Some(std::time::Instant::now() + HISTORY_EVERY);
        self.history_busy.store(true, Ordering::Relaxed);
        let busy = self.history_busy.clone();
        let run = self.discovery_run.load(Ordering::Relaxed);
        let current = self.discovery_run.clone();
        let chain = self.chain.clone();
        let journal = self.journal.clone();
        let marks_path = self.paths.platform_sync.clone();
        let names = self.names.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let refresh = self.refresher();
        tokio::spawn(async move {
            // Lowered on every way out, a panic in the read too.
            struct Done(Arc<AtomicBool>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Relaxed);
                }
            }
            let _done = Done(busy);
            let Some(wallet) = chain.wallet(&id).await else {
                return;
            };
            let marks = platform_history::Marks::load(&marks_path);
            match platform_history::read(&chain, &wallet, id, &marks, &names).await {
                // A lock (or a new lookup) since the start: this answer is for no one.
                Ok(_) if current.load(Ordering::Relaxed) != run => {}
                Ok((entries, next)) => {
                    // The entries first, then the marks: a crash between them reads again,
                    // and the keys drop the repeat. Not kept (the file's failure is in the
                    // journal already): the marks stay where they were, the next pass reads
                    // the same again.
                    if let Ok(changed) = journal.merge_network(entries) {
                        if changed {
                            refresh();
                        }
                        if next != marks {
                            let _ = next.save(&marks_path);
                        }
                    }
                }
                Err(e) => crate::diag::platform_failed("platform_history", &e),
            }
        });
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
                    Ok(id) => {
                        tokio::time::timeout(nfts::ASK_TIMEOUT, chain.jembe_gifts(id, &known))
                            .await
                            .unwrap_or_else(|_| Err(nfts::no_answer().into()))
                    }
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
    ///
    /// Nothing before the masternode lists are in (BUG-78): the read would fail its proof and
    /// get DAPI nodes banned, every half minute for each contact. None is marked asked, so the
    /// snapshot that comes once they are in (the sync status changes) asks them all.
    fn ask_names(&mut self, ids: Vec<String>) {
        if !self.chain.platform_ready() {
            return;
        }
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
    /// the keys go and the vault is deleted — without it the wallet cannot be opened again. The
    /// rest of the wallet's folder (store, names, journal, contacts' addresses) goes once
    /// nothing holds it: the loop makes the core anew, stops this chain off the loop and
    /// [`clear_other_leftovers`] removes it before the next chain starts. Headers and filters
    /// (`spv`) stay: they hold nothing of the wallet.
    async fn delete_wallet(&mut self, password: Option<&Password>) -> Result<(), CoreError> {
        if self.saved.is_none() {
            return Err(CoreError::NotAvailable(NO_VAULT.into()));
        }
        if let Some(password) = password {
            self.open_vault(password).await?;
        }
        self.lock().await?;
        // Whatever happens next, the core starts again: the loop stops this chain off the
        // loop (seconds, while a sync pass ends) and the next core shows its wallet — or the
        // onboarding — at once (F4, BUG-21).
        self.restart = true;
        let removed = match std::fs::remove_file(&self.paths.vault) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
        match removed {
            Ok(()) => {
                self.saved = None;
                // Its wrong passwords go with it: the same phrase restored starts afresh.
                let id = self.wallet.clone();
                self.attempts.forget(&id);
                Ok(())
            }
            Err(e) => Err(CoreError::Failed(format!(
                tr!("The wallet was not deleted: {e}"),
                e = e
            ))),
        }
    }

    /// Removes another wallet of this computer (its "…" in Settings; R-28, BUG-44), by its
    /// own password — none is refused. Its store is never opened here: only its files go,
    /// the vault first (after it the folder is a leftover that [`clear_other_leftovers`] and
    /// the index's loading take), then the folder (best effort), its record and its pictures.
    /// The open wallet stays as it is: not locked, its chain not stopped.
    async fn delete_other(
        &mut self,
        id: &str,
        password: Option<&Password>,
    ) -> Result<(), CoreError> {
        let password = password.ok_or(CoreError::WrongPassword)?;
        if self.switch_to.as_deref() == Some(id) || self.adding_from.as_deref() == Some(id) {
            return Err(CoreError::NotAvailable(
                tr!("This wallet cannot be deleted now").into(),
            ));
        }
        self.open_wallet_vault(id, password).await?;
        let folder = wallets::wallet_dir(&self.paths.root, id);
        match std::fs::remove_file(folder.join("wallet.json")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(CoreError::Failed(format!(
                    tr!("The wallet was not deleted: {e}"),
                    e = e
                )));
            }
        }
        self.attempts.forget(id);
        let _ = std::fs::remove_dir_all(&folder);
        self.index.wallets.retain(|r| r.id != id);
        if self.index.last.as_deref() == Some(id) {
            self.index.last = None;
        }
        self.save_index();
        if let Some(base) = &self.art_base {
            let _ = std::fs::remove_dir_all(base.join(id));
        }
        Ok(())
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
        self.attach = None;
        self.receive_address = None;
        // The notes' key and every decrypted note go with the keys (AC-T6).
        *self.notes.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }

    /// The wallet's payment notes, opened with it (FEAT-0003): the key from the seed, the file
    /// read once here, off the loop's thread (up to 6 MB to decrypt, AC-T9), and kept in memory
    /// until the lock.
    async fn open_notes(&self, seed: &[u8], wallet_id: &str) {
        let path = self.paths.notes.clone();
        let seed = Zeroizing::new(seed.to_vec());
        let wallet_id = wallet_id.to_owned();
        let notes =
            tokio::task::spawn_blocking(move || notes::Notes::open(path, &seed, &wallet_id))
                .await
                .ok();
        *self.notes.lock().unwrap_or_else(|e| e.into_inner()) = notes;
    }

    /// `Command::SetNote`: the note of an operation of the history, written off the loop's
    /// thread. The text never goes into an error or the journal.
    async fn set_note(&self, key: String, text: &str) -> Result<(), CoreError> {
        let text = crate::note_text::kept(text)
            .map_err(|_| CoreError::NotAvailable(crate::note_text::TOO_LONG.into()))?;
        if !["core:", "platform:", "pool:"]
            .iter()
            .any(|kind| key.starts_with(kind))
        {
            return Err(CoreError::NotAvailable(
                tr!("There is no such operation in the history").into(),
            ));
        }
        let notes = self.notes.clone();
        tokio::task::spawn_blocking(move || {
            let mut notes = notes.lock().unwrap_or_else(|e| e.into_inner());
            let notes = notes
                .as_mut()
                .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
            notes.set(&key, text).map_err(note_error)
        })
        .await
        .map_err(|_| CoreError::Failed("notes".into()))?
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
    fn wallet_entries(&mut self) -> Vec<WalletEntry> {
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
                wrong_tries: self.attempts.failures(&r.id),
                wait_until: self
                    .attempts
                    .check(&r.id, wall_secs())
                    .map(|t| t as Timestamp),
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
        let mut lost_block = None;
        // How far the wallet's filters are: back where a rescan was asked, it has finished.
        let mut synced = 0;
        let mut identities = Vec::new();
        // What each asset lock in the history is for, by the account it pays (BUG-57).
        let mut lock_fors: HashMap<String, identity::LockFor> = HashMap::new();
        // Every identity's credits, the one worked as among them (BUG-56), and the coins'
        // values for the fee of a payment (BUG-55).
        let mut all_credits: Duffs = 0;
        let mut coin_values = Vec::new();
        // Read before the coins are counted, and again after ([`count_checked`]).
        let status_before = self.chain.status();
        // The wallet's own Core addresses (FEAT-0009: a withdrawal to one is a move between own).
        let mut own_core: Vec<String> = Vec::new();
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
                own_core = wallet
                    .state()
                    .await
                    .core_wallet
                    .monitored_addresses()
                    .iter()
                    .map(ToString::to_string)
                    .collect();
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
                synced = state.core_wallet.synced_height();
                let now = now();
                let tip = self.chain.status().headers.map_or(0, |h| h.height);
                // The store's records first: one per transaction, as platform-wallet folds the
                // accounts it touches. Then this run's records the store has not got yet, one
                // per transaction too, each input and output once over the accounts (BUG-85), so
                // an asset lock — which touches two accounts — is one line, not two.
                let stored_ids: std::collections::HashSet<_> =
                    self.stored_history.iter().map(|r| r.txid).collect();
                let fresh = convert::one_per_transaction(
                    state
                        .core_wallet
                        .transaction_history()
                        .into_iter()
                        .filter(|r| !stored_ids.contains(&r.txid)),
                );
                // A spend recorded before the coin it spends (a restore's rescan) gets its
                // input back from our own outputs.
                let mut all: Vec<_> = self.stored_history.iter().cloned().chain(fresh).collect();
                convert::attribute_late_inputs(&mut all);
                let locks: Vec<_> = all
                    .iter()
                    .filter(|r| {
                        r.transaction_type
                            == key_wallet::transaction_checking::transaction_router::TransactionType::AssetLock
                    })
                    .map(|r| (r.txid.to_string(), &r.transaction))
                    .collect();
                lock_fors = identity::lock_fors(
                    &state.core_wallet.accounts,
                    &mut self.top_ups,
                    id,
                    self.unlocked.as_ref().and_then(|o| o.keys.as_ref()),
                    state
                        .identity_manager
                        .managed_identities()
                        .filter(|m| m.wallet_id == Some(id))
                        .filter_map(|m| m.identity_index),
                    &locks,
                );
                // One the wallet took back out of its block is shown in it, and the filters are
                // scanned again from there so the wallet and the store get it back (BUG-64).
                lost_block = self
                    .mined
                    .recall(&mut all, self.chain.status().chainlock_height);
                let records: Vec<(String, Activity)> = all
                    .iter()
                    .map(|r| {
                        let txid = r.txid.to_string();
                        // A block gives its own time: only what is out of one is kept (each
                        // new one is a write of the file).
                        let seen = match r.context.block_info() {
                            Some(_) => now,
                            None => self.mempool.first_seen(&txid, now, tip),
                        };
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
                all_credits = mine
                    .iter()
                    .map(|m| identity::credits_as_duffs(m.identity.balance()))
                    .sum();
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
                // A request names both sides as the wallet knows them now: our identities by
                // their names, contacts by theirs (BUG-69), and which identity of ours it
                // went from or came to when there are several (BUG-58); one hidden here
                // before a resync stays hidden.
                let own: std::collections::HashMap<&str, String> = identities
                    .iter()
                    .filter_map(|c| Some((c.id.as_str(), format!("@{}", c.name.as_ref()?))))
                    .collect();
                let named = |id: &str| {
                    own.get(id).cloned().or_else(|| {
                        names
                            .get(id)
                            .cloned()
                            .flatten()
                            .map(|label| format!("@{label}"))
                    })
                };
                history.extend(self.journal.entries().iter().map(|entry| {
                    let mut line = journal::activity(entry);
                    platform_history::name_request_sides(
                        &mut line.kind,
                        entry,
                        identities.len() > 1,
                        named,
                    );
                    if let ActivityKind::ContactRequestIn { state, .. } = &mut line.kind {
                        let from = entry
                            .key
                            .as_deref()
                            .and_then(|k| k.strip_prefix("cri:"))
                            .and_then(|k| k.split(':').next());
                        if *state == evolite_api::RequestState::Waiting
                            && from.is_some_and(|f| self.hidden.contains(f))
                        {
                            *state = evolite_api::RequestState::Hidden;
                        }
                    }
                    line
                }));
                (
                    // Each coin once, not platform-wallet's sum of the accounts (BUG-77).
                    Some(convert::core_balance(&state.core_wallet)),
                    {
                        // Coins, and the addresses holding them (the overview counts those).
                        let utxos = convert::coins_once(&state.core_wallet);
                        let holding: std::collections::HashSet<_> =
                            utxos.iter().map(|u| u.address.clone()).collect();
                        coin_values = utxos.iter().map(|u| u.value()).collect();
                        (utxos.len(), holding.len())
                    },
                    history,
                    shown,
                    platform,
                )
            }
            None => (None, (0, 0), Vec::new(), None, None),
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
        let mut fees = self.fees_now(coins).await;
        fees.core_coins = convert::core_coins(coin_values);
        // The pool's own operations are in the history too (only the owner sees them), and a
        // Core asset lock made for the pool says so instead of "Transfer to Platform".
        if let Some(pool) = &pool {
            history.extend(pool.operations.iter().cloned());
        }
        // Sends on Platform shown from "Sign" until their own lines are here (BUG-12).
        let mut history = self
            .platform_sending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .apply(history, now());
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
            // "Transfer to Platform" says whom — the identity it registered or tops up — and
            // whether Platform took it (BUG-57).
            let tracked: HashMap<String, identity::Tracked> = wallet
                .asset_locks()
                .list_tracked_locks()
                .await
                .into_iter()
                .map(|l| {
                    (
                        l.out_point.txid.to_string(),
                        identity::Tracked {
                            funding: l.funding_type,
                            slot: l.identity_index,
                            consumed: l.status.is_consumed(),
                        },
                    )
                })
                .collect();
            // The locks Identity offers to continue, and only while it offers that: the same
            // rules as `Wallet::unfinished_*` (not while one is being made, only after the
            // search for the identities).
            let mut waiting = std::collections::HashSet::new();
            if unfinished_top_up.is_some() {
                for lock in self.chain.unfinished_top_ups(&wallet).await {
                    if !lock.awaiting_chain_lock {
                        waiting.insert(lock.out_point.txid.to_string());
                    }
                }
            }
            if unfinished.as_ref().is_some_and(|u| u.locked.is_some())
                && let Some(lock) = self.chain.unfinished(&wallet).await
            {
                waiting.insert(lock.out_point.txid.to_string());
            }
            for act in history.iter_mut() {
                if let (
                    ActivityKind::TopUp {
                        identity,
                        creates,
                        credits,
                    },
                    Some(txid),
                ) = (&mut act.kind, act.txid.as_deref())
                {
                    let (who, made, taken) = identity::lock_line(
                        txid,
                        lock_fors.get(txid).copied(),
                        tracked.get(txid).copied(),
                        self.consumed_locks.contains(txid),
                        waiting.contains(txid),
                        &identities,
                    );
                    if identity.is_empty() {
                        *identity = who;
                    }
                    (*creates, *credits) = (made, taken);
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
            now(),
        );
        // A payment long out of a block may have lost its block before this file knew it
        // (BUG-64): the filters are scanned again from the wallet's birth to find it — once for
        // each such payment, also across restarts (review r1 should 1). Noted as done only when
        // the rescan has run to its end (review r2 should 1), and a lost block is noted too
        // (should 2): one cut short is asked again at the next start, one finished never again.
        if let Some(done) = OwedRescan::finished(&mut self.rescan_owed, &id, synced) {
            self.mempool.rescanned(&done.stale);
            self.mined.rescanned(&done.lost);
        }
        // Read once, before a rescan is asked: a rescan asked below is owed in this snapshot.
        let sync_done = self.chain.status().phase == sync::Phase::Synced;
        if let Some(lost) = lost_block {
            let from = lost.from.saturating_sub(1);
            let why = "lost_block";
            self.rescan_from(id, from, why, synced, Vec::new(), lost.txids)
                .await;
        } else {
            let stale = self.mempool.not_rescanned(&history);
            if !stale.is_empty() {
                let from = meta.birth_height;
                self.rescan_from(id, from, "stale", synced, stale, Vec::new())
                    .await;
            }
        }
        // Stale since an earlier run and not rescanned yet: most likely in a block the store
        // lost, not a payment still coming or going (BUG-82). Left out only while its rescan
        // is ahead (not synced yet) or owed (review r1 must 1); shown once it has said.
        let owed = self
            .rescan_owed
            .as_ref()
            .filter(|o| o.wallet == id)
            .map_or(&[][..], |o| o.stale.as_slice());
        self.mempool
            .leave_out_checking(&mut history, owed, sync_done);
        // The person's notes on their lines (FEAT-0003); a send's note waiting for its line goes
        // there first, off the loop's thread: binding it writes the file (AC-T9, review r1
        // should 2). Rare: once per send out of the pool.
        let to_bind = self
            .notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|notes| notes.lines_to_bind(&history));
        if let Some(lines) = to_bind {
            let notes = self.notes.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Some(notes) = notes.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                    notes.bind(&lines);
                }
            })
            .await;
        }
        let notes_error = match self
            .notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            Some(notes) => {
                for line in history.iter_mut().filter(|a| !a.key.is_empty()) {
                    line.note = notes.get(&line.key).map(str::to_owned);
                }
                notes.trouble()
            }
            None => None,
        };
        history.sort_by_key(|a| std::cmp::Reverse(a.at));
        // Only a checked balance is shown: while the filters are not at the tip (a start, a
        // rewound pass, a rescan) the last checked one with its time, or none (BUG-77).
        let (core, core_as_of) = self.core_balance.shown(
            balance.unwrap_or(0),
            count_checked(balance.is_some(), &status_before, &self.chain.status()),
            now(),
        );
        let mut shown_wallet = Wallet {
            password_each_send: self.password_each_send,
            notes_error,
            balances: Balances {
                core,
                core_as_of,
                core_addresses: addresses as u32,
                credits,
                all_credits,
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
        };
        // Which balances each line changes and which way, with what is the wallet's own
        // (FEAT-0009 R-1, R-2).
        crate::layers::settle_wallet(&mut shown_wallet, &own_core);
        shown_wallet
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
                tr!("We do not send from Core to a Platform address yet (an asset lock is needed). Send from “Credits”").into(),
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
    /// with no answer may still go through. "Password for every send" is checked before, by
    /// [`Real::spend_gate`].
    async fn send(
        &mut self,
        from: Source,
        to: &str,
        amount: Duffs,
        note: Option<NoteText>,
    ) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let id = open.id;
        let note = send_note(note)?;
        match from {
            Source::Core => {}
            Source::Credits => return self.send_credits(id, to, amount, note),
            Source::Pool => return self.send_from_pool(to, amount, note),
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
        core_pays(&wallet, amount).await?;
        let keys = self
            .unlocked
            .as_ref()
            .and_then(|open| open.keys.clone())
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        // In the history from now on, as "sending", until the wallet sees it; its fee as the
        // payment's own coins make it (BUG-55 review, should 2).
        let values: Vec<u64> = convert::coins_once(&wallet.state().await.core_wallet)
            .iter()
            .map(|u| u.value())
            .collect();
        let fee = payment_fee(values, amount as Duffs);
        let key = self
            .outgoing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start(to, amount as Duffs, fee, now());
        let outgoing = self.outgoing.clone();
        let contact_paid = self.contact_paid.clone();
        let chain = self.chain.clone();
        let out = self.out.clone();
        let wake = self.wake.clone();
        let notes = self.notes.clone();
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
            // Taken (or maybe taken, with its txid): the note goes with the txid (AC-T3).
            if let (Ok(paid), Some(text)) = (&paid, note) {
                let key = format!("core:{}", paid.txid);
                keep_note(notes, move |n| {
                    let _ = n.set(&key, Some(text));
                })
                .await;
            }
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
                    // The labels the layers read the source from (FEAT-0009).
                    ShieldSource::Credits => (
                        crate::layers::FROM_CREDITS,
                        chain.first_identity_credits(&wallet, false).await,
                    ),
                    ShieldSource::PlatformAddress => (
                        crate::layers::FROM_PLATFORM_ADDRESS,
                        chain.platform_address(&wallet).await.ok().map(|(_, c)| c),
                    ),
                    ShieldSource::Core => (crate::layers::FROM_CORE, None),
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
                        ..Default::default()
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
    fn send_from_pool(
        &self,
        to: &str,
        amount: Duffs,
        note: Option<String>,
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
                    To::Private => chain.pool_transfer(&wallet, &seed[..], &to, credits).await,
                    To::Platform => chain.unshield(&wallet, &seed[..], &to, credits).await,
                    To::Core => chain.pool_withdraw(&wallet, &seed[..], &to, credits).await,
                }
                // Handed over but not proved: maybe gone, so "sent, no answer", never resent.
                .map(|()| true)
                .or_else(|e| match e {
                    pool::PoolError::Unconfirmed => Ok(false),
                    other => Err(pool_error(other)),
                });
                // Its line (and so its key) comes with the pool's store: the note waits for it,
                // encrypted, after a restart too (FEAT-0003).
                if let (Ok(_), Some(text)) = (&sent, note) {
                    keep_note(notes, move |n| {
                        n.set_pending(notes::Pending::Pool, amount, signed, text)
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
    fn send_credits(
        &self,
        id: WalletId,
        to: &str,
        amount: Duffs,
        note: Option<String>,
    ) -> Result<(), CoreError> {
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
        let notes = self.notes.clone();
        // In the history from now on, as "being sent", until the journal has its line (BUG-12).
        let sending = self.platform_sending.clone();
        let key = sending.lock().unwrap_or_else(|e| e.into_inner()).start(
            platform_sending::credits_line(&what),
            amount,
            now(),
        );
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
                        Err(e) => return Err(read_failure(e)),
                    },
                };
                chain
                    .send_credits(&wallet, &keys, payee, credits)
                    .await
                    .map_err(dashpay_failure)?;
                // `send_credits` read the balance back: the difference is the amount and fee.
                let after = chain.first_identity_credits(&wallet, false).await;
                let line = journal.add(journal::Entry {
                    at: now(),
                    what,
                    spent: spent_credits(before, after).unwrap_or(amount),
                    moved: amount,
                    ..Default::default()
                });
                // The journal's line is the transfer's for good: its note goes there (AC-T3).
                if let (Some(line), Some(text)) = (line, note) {
                    let key = format!("platform:{line:016x}");
                    keep_note(notes, move |n| {
                        let _ = n.set(&key, Some(text));
                    })
                    .await;
                }
                Ok::<(), CoreError>(())
            }
            .await
            .map(|()| SentPayment {
                txid: String::new(),
                amount,
                fee: 0,
                relayed: true,
            });
            sending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .answered(key, result.is_ok(), now());
            dirty.store(true, Ordering::Relaxed);
            Event::Sent(result)
        });
        Ok(())
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
    fn start_chain(
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
    async fn start_ended(&mut self) -> bool {
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
    async fn start_settled(&mut self, wait: Duration) {
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
    fn retry_start(&mut self) {
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
    fn network_chain(&self) -> &Arc<Chain> {
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
    fn start_after(&mut self, parked: Parked, ahead: bool) {
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
    async fn shut_down(&mut self) {
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
    async fn attach_wallet(&mut self) -> Result<(), String> {
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

    /// [`Self::unlock`] with this wallet's wrong passwords counted (FEAT-0002): in a pause it
    /// is refused before the vault is touched; else the try is written down as wrong before
    /// Argon2, then cleared by a right password or taken back by an error of another kind.
    async fn unlock_counted(&mut self, password: &Password) -> Result<(), CoreError> {
        let id = self.wallet.clone();
        let before = self.attempts.begin(&id, wall_secs()).map_err(too_many)?;
        let result = self.unlock(password).await;
        self.attempts.record(&id, before, counted(&result));
        result
    }

    /// [`Self::open_wallet_vault`] for "Open "…"" (`SwitchWallet` with the password), counted
    /// as [`Self::unlock_counted`] for wallet `id`. The other callers of the vault (deleting
    /// another wallet, a payment's password) do not count: FEAT-0004 (R-31).
    async fn open_other_counted(
        &mut self,
        id: &str,
        password: &Password,
    ) -> Result<(vault::VaultFile, Zeroizing<String>), CoreError> {
        let before = self.attempts.begin(id, wall_secs()).map_err(too_many)?;
        let result = self.open_wallet_vault(id, password).await;
        self.attempts.record(id, before, counted(&result));
        result
    }

    /// Opens the vault and restarts sync with the wallet's keys.
    async fn unlock(&mut self, password: &Password) -> Result<(), CoreError> {
        let (file, phrase) = self.open_vault(password).await?;
        self.unlock_opened(file, phrase).await
    }

    /// Unlocks with a vault already opened by its password (one Argon2 per switch, F2). The
    /// wallet is open at once; platform-wallet gets it when the chain runs (F3), so a password
    /// typed while the chain starts no longer waits for the start.
    async fn unlock_opened(
        &mut self,
        file: vault::VaultFile,
        phrase: Zeroizing<String>,
    ) -> Result<(), CoreError> {
        let meta = file.meta;
        if meta.network != network() {
            return Err(CoreError::NotAvailable(format!(
                tr!("The wallet on this computer is for the {} network"),
                meta.network
            )));
        }
        // Never over a deleted wallet's files (BUG-8): no store opens over them.
        if self.leftovers {
            return Err(CoreError::NotAvailable(LEFTOVERS.into()));
        }
        self.last_touch = tokio::time::Instant::now();
        let keys = keys_of(&phrase)?;
        // The id platform-wallet gives the wallet: from the same root key.
        let id = keys.compute_wallet_id();
        let seed = seed_of(&phrase)?;
        self.open_notes(seed.as_ref(), &meta.wallet_id).await;
        self.unlocked = Some(Unlocked {
            id,
            meta,
            keys: Some(keys),
            seed: Some(seed),
        });
        self.attach = Some(phrase);
        self.attach_failed = None;
        if let Err(e) = self.attach_wallet().await {
            // The chain runs and refused the wallet: as before, it stays locked.
            self.lock().await?;
            return Err(CoreError::Failed(e));
        }
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
    /// 10 s, then less and less often (up to 5 min), while this unlock lasts; so is a pass with
    /// slots or names missing (BUG-9), though what it found is shown and its balances kept
    /// fresh at once. A newer search (`FindIdentities`) or locking stops this one, and its keys
    /// go with it; a complete pass lets them go too.
    fn discover_identities(&self) {
        let Some((id, Some(keys))) = self.unlocked.as_ref().map(|o| (o.id, o.keys.clone())) else {
            return;
        };
        let run = self.discovery_run.fetch_add(1, Ordering::Relaxed) + 1;
        let current = self.discovery_run.clone();
        let chain = self.chain.clone();
        let search = self.identity_search.clone();
        let found = self.identities_found.clone();
        let complete = self.identities_complete.clone();
        let refresh = self.refresher();
        let stale = self.balances_stale.clone();
        found.store(false, Ordering::Relaxed);
        complete.store(false, Ordering::Relaxed);
        set_search(&search, IdentitySearch::Searching);
        refresh();
        tokio::spawn(async move {
            let live = move || current.load(Ordering::Relaxed) == run;
            chain.until_platform_ready().await;
            let mut pause = DISCOVERY_RETRY;
            let mut names_read = std::collections::HashSet::new();
            while live() {
                let result = match chain.wallet(&id).await {
                    Some(wallet) => {
                        chain
                            .discover_identities_again(&wallet, &keys, &mut names_read)
                            .await
                    }
                    None => Err(tr!("the wallet is not loaded yet").into()),
                };
                if !live() {
                    break;
                }
                let was_found = found.load(Ordering::Relaxed);
                // Every slot answered is enough for the unfinished lock (BUG-9 r2 should-1):
                // names still missing are read on the next pass. Once answered, it stays so
                // for this search.
                if result.as_ref().is_ok_and(|d| d.slots_complete) {
                    complete.store(true, Ordering::Relaxed);
                }
                let step = discovery_step(result, was_found);
                set_search(&search, step.status);
                if step.found && !was_found {
                    found.store(true, Ordering::Relaxed);
                    tokio::spawn(keep_balances_fresh(
                        chain.clone(),
                        id,
                        live.clone(),
                        stale.clone(),
                        refresh.clone(),
                    ));
                }
                refresh();
                if !step.again {
                    // The keys are not needed to read balances: let them go now.
                    return;
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

    /// The fees the snapshot shows for a wallet of `coins` coins (without
    /// [`Fees::core_coins`]): the pool's and Platform's at the protocol version the SDK speaks,
    /// and what this data folder last really paid for each kind of write. One place, so what a
    /// form promises is what the core charges (BUG-54).
    async fn fees_now(&self, coins: usize) -> Fees {
        let mut fees = fees(coins);
        if let Ok(sdk) = self.chain.sdk().await {
            pool::set_fees(&mut fees, sdk.version());
            platform_fees(&mut fees, sdk.version(), &self.journal);
        }
        fees
    }

    /// Checks what can be checked at once and starts the identity in the background: the name
    /// check, the asset lock ([`convert::identity_lock`]: `top_up`, the name and the
    /// creation), the identity and its name take a minute or more. The answer is
    /// `Done { what: "identity" }`.
    async fn create_identity(
        &mut self,
        name: Option<&str>,
        top_up: Duffs,
    ) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let id = open.id;
        let label = identity::new_identity_label(name)?;
        let top_up = positive_duffs(top_up)?;
        // The name and the identity's creation at the fees the form showed (BUG-54).
        // Below Platform's least lock it would be refused and its DASH stay locked: not made
        // (review r1 must; the form says how much at least).
        let amount = convert::identity_lock(top_up, &self.fees_now(0).await, label.is_some())
            .ok_or(CoreError::BadAmount)?;
        let status = self.chain.status();
        if !coins_known(&status, true)
            || status.phase == Phase::Error
            || !self.chain.platform_ready()
        {
            return Err(CoreError::NotSynced);
        }
        let wallet = loaded(&self.chain, &id).await?;
        core_pays(&wallet, amount).await?;
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
            let available = match &label {
                Some(label) => chain.name_available(label).await,
                None => Ok(true),
            };
            let result = match available {
                Ok(false) => Err(CoreError::NotAvailable(format!(
                    tr!("The name @{label} is already taken, the money is untouched"),
                    label = label.as_deref().unwrap_or_default()
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
                    // with it (ResumeIdentity). Without a name nothing is kept, or the
                    // nameless identity would look cut short for ever (R-7, `nameless_slot`).
                    identity::remember_name(&pending, index, label.as_deref())?;
                    chain
                        .create_identity(&wallet, &keys, index, label.as_deref(), amount)
                        .await?;
                    pending.remove(index)?;
                    Ok(index)
                }
                .await
                .map_err(core_failure),
            };
            // The new identity is the one actions go from (analyst-44); on a failure the
            // choice stays.
            let result = identity::choose_created(id, result);
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
        // A new wallet is born at the tip of the headers: this chain's, or the one the wallet
        // before saw ("Add a wallet"). The chain may still be starting (the onboarding no
        // longer waits for it, F3); with neither known, its start is waited for — without
        // headers the wallet would be scanned from the genesis.
        let birth_height = if restored {
            0
        } else {
            let known = |core: &Self| {
                core.network_chain()
                    .status()
                    .headers
                    .map(|h| h.height)
                    .max(core.birth_hint)
            };
            match known(self) {
                Some(height) => height,
                None if self.parked.is_some() => {
                    // The parked chain's start, still under way, brings them (BUG-76).
                    let deadline = tokio::time::Instant::now() + START_WAIT;
                    while known(self).is_none()
                        && tokio::time::Instant::now() < deadline
                        && self
                            .parked
                            .as_ref()
                            .and_then(|p| p.starting.as_ref())
                            .is_some_and(|start| !start.is_finished())
                    {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                    known(self).unwrap_or(0)
                }
                None => {
                    self.start_settled(START_WAIT).await;
                    known(self).unwrap_or(0)
                }
            }
        };
        if self.leftovers {
            return Err(CoreError::NotAvailable(LEFTOVERS.into()));
        }
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
        // Added over a parked chain: that one stops, then this wallet's own starts (BUG-76).
        // Nothing to open ahead: a new wallet's store has nothing to show yet.
        if let Some(parked) = self.parked.take() {
            self.start_after(parked, false);
        }
        let id = keys.compute_wallet_id();
        let seed = seed_of(&phrase)?;
        self.open_notes(seed.as_ref(), &meta.wallet_id).await;
        self.unlocked = Some(Unlocked {
            id,
            meta,
            keys: Some(keys),
            seed: Some(seed),
        });
        self.attach = Some(phrase);
        self.attach_failed = None;
        if let Err(e) = self.attach_wallet().await {
            // The vault is "Locked" now: the interface goes to the unlock screen.
            self.lock().await?;
            return Err(CoreError::NotAvailable(format!(
                tr!(
                    "The wallet is saved, but the sync did not start ({e}). Sign in with your password — evolite starts it again"
                ),
                e = e
            )));
        }
        self.receive_address = None;
        self.discover_identities();
        Ok(())
    }

    /// "Password for every send" (BUG-42, R-27), before anything of a spend is checked, locked
    /// or signed: when the wallet asks for it, no password is a refusal and a wrong one is
    /// [`CoreError::WrongPassword`]. The attempt counter and the pauses are FEAT-0004's, here.
    async fn spend_gate(&self, password: Option<&Password>) -> Result<(), CoreError> {
        if !self.password_each_send {
            return Ok(());
        }
        match password {
            None => Err(CoreError::NotAvailable(crate::PASSWORD_NEEDED.into())),
            Some(password) => self.open_vault(password).await.map(|_| ()),
        }
    }

    /// Settings → Security → "Password for every send": on at once; off only with the
    /// wallet's password, which [`Real::spend_gate`] would ask anyway.
    async fn set_password_each_send(
        &mut self,
        on: bool,
        password: Option<&Password>,
    ) -> Result<(), CoreError> {
        if self.unlocked.is_none() {
            return Err(CoreError::NotAvailable(tr!("The wallet is locked").into()));
        }
        if !on {
            let password = password.ok_or(CoreError::WrongPassword)?;
            self.open_vault(password).await?;
        }
        security::save(&self.paths.root, &self.wallet, on).map_err(|e| {
            CoreError::NotAvailable(format!(tr!("The setting was not saved: {e}"), e = e))
        })?;
        self.password_each_send = on;
        Ok(())
    }

    /// A spend past [`Real::spend_gate`], answered as each operation always was.
    async fn spend(&mut self, op: Spend) -> (Option<Event>, bool) {
        let event = match op {
            Spend::Send {
                from,
                to,
                amount,
                note,
            } => match self.send(from, &to, amount, note).await {
                // The answer comes from the broadcast task.
                Ok(()) => return (None, true),
                Err(e) => Event::Sent(Err(e)),
            },
            Spend::CreateIdentity { name, top_up } => {
                match self.create_identity(name.as_deref(), top_up).await {
                    // The answer comes from the background task.
                    Ok(()) => return (None, true),
                    Err(e) => Event::Done {
                        what: "identity",
                        result: Err(e),
                    },
                }
            }
            Spend::ResumeIdentity => match self.resume_identity() {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: "identity",
                    result: Err(e),
                },
            },
            Spend::Shield { from, amount } => match self.shield(from, amount) {
                Ok(()) => Event::Progress {
                    what: "shield",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: "shield",
                    result: Err(e),
                },
            },
            Spend::GiftNft { .. } => Event::Done {
                what: "gift",
                result: Err(CoreError::NotAvailable(NO_GIFTS.into())),
            },
            Spend::AddContact { name } => match self.add_contact(&name) {
                Ok(()) => return (None, false),
                Err(e) => Event::Done {
                    what: "contact",
                    result: Err(e),
                },
            },
            Spend::AcceptContact { identity_id } => {
                match self.contact_job(&identity_id, ContactJob::Accept) {
                    Ok(()) => return (None, false),
                    Err(e) => Event::Done {
                        what: "contact",
                        result: Err(e),
                    },
                }
            }
            Spend::SaveProfile {
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
            Spend::TopUpIdentity { amount } => match self.top_up_identity(amount).await {
                // The answer comes from the background task.
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: "top-up",
                    result: Err(e),
                },
            },
            Spend::ResumeTopUp => match self.resume_top_up() {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: "top-up",
                    result: Err(e),
                },
            },
            Spend::Unshield { amount } => match self.unshield(amount) {
                Ok(()) => Event::Progress {
                    what: "unshield",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: "unshield",
                    result: Err(e),
                },
            },
            Spend::TopUpFromPool { amount } => match self.top_up_from_pool(amount) {
                Ok(()) => Event::Progress {
                    what: "pool-top-up",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: "pool-top-up",
                    result: Err(e),
                },
            },
            Spend::AddName { name } => match self.add_name(&name) {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: "name",
                    result: Err(e),
                },
            },
            Spend::ApproveApp { id } => match self.answer_app(id, true) {
                // The answer comes from the background task.
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
        };
        (Some(event), true)
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
                    let chain = self.network_chain().clone();
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
                result: self.unlock_counted(&password).await,
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
            Command::HideRequest { identity_id } => {
                match self.contact_job(&identity_id, ContactJob::Hide) {
                    Ok(()) => return (None, false),
                    Err(e) => Event::Done {
                        what: "contact",
                        result: Err(e),
                    },
                }
            }
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
            // Every spend passes the one gate first (R-27): refused, it is answered as the
            // operation's own failure, and nothing of it was checked, locked or signed.
            Command::Spend { op, password } => {
                if let Err(why) = self.spend_gate(password.as_ref()).await {
                    return (Some(op.refused(why)), true);
                }
                return self.spend(op).await;
            }
            Command::SetPasswordEachSend { on, password } => Event::Done {
                what: "password-each-send",
                result: self.set_password_each_send(on, password.as_ref()).await,
            },
            // Turning a request down spends nothing: no password.
            Command::DeclineApp { id } => match self.answer_app(id, false) {
                Ok(_) => Event::Done {
                    what: "app",
                    result: Ok(()),
                },
                Err(e) => Event::Done {
                    what: "app",
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
            // Asked with the next snapshot (`shown_nfts`), as a due refresh is.
            Command::RefreshNfts => {
                self.nfts.ask_now();
                return (None, true);
            }
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
            Command::DeleteWallet { password, wallet } => {
                match wallet.filter(|id| *id != self.wallet) {
                    // Another wallet: not "delete", which the app takes for the open one gone.
                    Some(id) => Event::Done {
                        what: "delete-other",
                        result: self.delete_other(&id, password.as_ref()).await,
                    },
                    None => Event::Done {
                        what: "delete",
                        result: self.delete_wallet(password.as_ref()).await,
                    },
                }
            }
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
                                result: self.unlock_counted(&password).await,
                            }),
                            true,
                        ),
                        _ => (None, false),
                    };
                }
                // The other wallet's password first: a wrong one changes nothing here but its
                // count, which the next snapshot shows (FEAT-0002).
                let opened = match &password {
                    Some(password) => match self.open_other_counted(&id, password).await {
                        Ok(opened) => Some(opened),
                        Err(e) => {
                            return (
                                Some(Event::Done {
                                    what: "switch",
                                    result: Err(e),
                                }),
                                true,
                            );
                        }
                    },
                    None => None,
                };
                self.index.last = Some(id.clone());
                self.save_index();
                self.switch_to = Some(id);
                // The new core opens it as opened here (the loop answers "switch").
                self.switch_opened = opened;
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
            Command::SetNote { key, text } => {
                let result = self.set_note(key, &text.0).await;
                let changed = result.is_ok();
                return (
                    Some(Event::Done {
                        what: "note",
                        result,
                    }),
                    changed,
                );
            }
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

/// The answer while a pause lasts (FEAT-0002).
fn too_many(until: u64) -> CoreError {
    CoreError::TooManyTries {
        until: Timestamp::try_from(until).unwrap_or(Timestamp::MAX),
    }
}

/// How a counted password check ended: only `WrongPassword` is the password's fault (UC-7).
fn counted<T>(result: &Result<T, CoreError>) -> crate::attempts::Outcome {
    use crate::attempts::Outcome;
    match result {
        Ok(_) => Outcome::Right,
        Err(CoreError::WrongPassword) => Outcome::Wrong,
        Err(_) => Outcome::Other,
    }
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

/// When the next core may open its store ahead of its SPV client ([`Real::start_chain`],
/// BUG-65): only when the core before had another wallet. The same wallet again (a restart
/// after a delete that failed, say) waits for the chain before to let its store go: one
/// store is never open twice.
fn open_ahead_after(
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
fn hand_on_chain(old: &mut Real, new: &mut Real, adding: bool) {
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
        let snapshot = core.snapshot().await;
        let _ = out.send(Event::Snapshot(Box::new(snapshot)));
        wake();
        core.start_chain(None, None, None, false);
        // When the last DashPay pass the snapshot knows of finished.
        let mut dashpay_pass = None;
        loop {
            if core.start_ended().await {
                core.dirty.store(true, Ordering::Relaxed);
            }
            core.retry_start();
            // The open wallet into platform-wallet as soon as the store is open, settled there
            // once the chain runs (BUG-25; a failure is tried again later, and the sync status
            // says what is wrong with the chain itself). The
            // identity lookup begun at the unlock starts again on the loaded wallet, without
            // its pause after "the wallet is not loaded yet".
            if core.attach.is_some() && core.attach_wallet().await.is_ok() && core.attach.is_none()
            {
                core.discover_identities();
            }
            // Platform address balances come with proofs: from the masternode lists on.
            if core.chain.platform_ready() {
                core.chain.start_platform_sync().await;
                core.connect_pool();
                core.build_contact_accounts();
                core.read_platform_history();
                // A DashPay pass brings requests and contacts without any other event: the
                // snapshot is made again after each one (BUG-13).
                if new_pass(&mut dashpay_pass, core.chain.dashpay_synced_at().await) {
                    core.dirty.store(true, Ordering::Relaxed);
                }
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
                            let _ = core.lock().await;
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
                                let result = core.unlock_opened(file, phrase).await;
                                let _ = out.send(Event::Done {
                                    what: "switch",
                                    result,
                                });
                            }
                            core.dirty.store(false, Ordering::Relaxed);
                            let snapshot = core.snapshot().await;
                            let _ = out.send(Event::Snapshot(Box::new(snapshot)));
                            wake();
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        core.shut_down().await;
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

/// Whether a sync pass finished since `last` (and notes it): `at` is when the latest did.
fn new_pass(last: &mut Option<u64>, at: Option<u64>) -> bool {
    if at.is_none() || at == *last {
        return false;
    }
    *last = at;
    true
}

#[cfg(test)]
mod tests {
    /// BUG-55 review, should 2: a small payment from a wallet of many coins is shown with the
    /// fee of the one coin it takes, as the Send screen estimated it.
    #[test]
    fn a_sending_payment_is_shown_with_the_fee_of_its_own_coins() {
        let many = vec![100_000_000_u64; 30];
        let one = super::payment_fee(vec![100_000_000], 1_000_000);
        assert_eq!(super::payment_fee(many.clone(), 1_000_000), one);
        assert!(one < super::fees(many.len()).core_payment);
    }

    use super::*;

    /// BUG-64 review r2 should 1: a rescan is finished only once the wallet's filters are back
    /// where they stood when it was asked — not while they climb, not for another wallet; a
    /// second one asked meanwhile adds what it is for; a wallet switch drops what was owed.
    #[test]
    fn a_rescan_is_noted_only_when_it_has_run_to_its_end() {
        let (a, b): (WalletId, WalletId) = ([0xa; 32], [0xb; 32]);
        let mut owed = None;
        let s = |t: &str| vec![t.to_owned()];
        OwedRescan::add(&mut owed, a, 1_600_000, s("stale"), Vec::new());
        // Rewound to the birth and climbing.
        assert_eq!(OwedRescan::finished(&mut owed, &a, 1_400_000), None);
        assert_eq!(
            OwedRescan::finished(&mut owed, &b, 1_700_000),
            None,
            "another wallet"
        );
        OwedRescan::add(&mut owed, a, 1_600_002, Vec::new(), s("lost"));
        assert_eq!(OwedRescan::finished(&mut owed, &a, 1_600_001), None);
        let done = OwedRescan::finished(&mut owed, &a, 1_600_002).expect("back at the tip");
        assert_eq!((done.stale, done.lost), (s("stale"), s("lost")));
        assert!(owed.is_none(), "noted once");
        // Asked for a, then for b: a's is dropped (asked again in a later run).
        OwedRescan::add(&mut owed, a, 10, s("x"), Vec::new());
        OwedRescan::add(&mut owed, b, 20, s("y"), Vec::new());
        let done = OwedRescan::finished(&mut owed, &b, 20).expect("b's");
        assert_eq!(done.stale, s("y"));
    }

    /// BUG-13: each DashPay pass makes the snapshot again, once.
    #[test]
    fn a_new_dashpay_pass_is_noticed_once() {
        let mut last = None;
        assert!(!new_pass(&mut last, None), "no pass yet");
        assert!(new_pass(&mut last, Some(100)));
        assert!(!new_pass(&mut last, Some(100)), "the same pass");
        assert!(new_pass(&mut last, Some(115)));
        assert!(
            !new_pass(&mut last, None),
            "a chain gone keeps what was seen"
        );
        assert_eq!(last, Some(115));
    }

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
        let backfill = store::backfill_file(&paths.chain.store);
        for file in [
            &paths.vault,
            &paths.journal,
            &paths.platform_sync,
            &paths.contact_addresses,
            &paths.chain.store,
            &store_wal,
            &backfill,
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
        // BUG-58: Platform is read back from the start; the journal stays.
        assert!(!paths.platform_sync.exists());
        // BUG-77: both marks go — the backfill record, and the spent coins with the store.
        assert!(
            !backfill.exists(),
            "the backfill record vouches for a scan that is gone"
        );
        assert_eq!(
            store::spent_claims(&paths.chain.store, &[0; 32]),
            store::Claims::default()
        );
        assert!(!paths.resync.exists(), "once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-0003 (AC-T7): "Sync again" keeps the payment notes, and they still open, on the
    /// same keys.
    #[test]
    fn a_resync_keeps_the_payment_notes() {
        let dir = fresh_dir("resync-notes");
        let paths = Paths::new(dir.clone(), TEST_WALLET);
        std::fs::create_dir_all(paths.chain.spv.join("headers")).unwrap();
        std::fs::create_dir_all(paths.vault.parent().unwrap()).unwrap();
        std::fs::write(&paths.chain.store, b"x").unwrap();
        let seed = [7u8; 64];
        notes::Notes::open(paths.notes.clone(), &seed, "ab")
            .set("core:aa", Some("rent".into()))
            .unwrap();
        std::fs::write(&paths.resync, b"").unwrap();
        assert!(resync_on_start(&paths).unwrap());
        assert!(!paths.chain.store.exists());
        let back = notes::Notes::open(paths.notes.clone(), &seed, "ab");
        assert_eq!(back.get("core:aa"), Some("rent"));
        assert!(!back.unreadable());
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
        // BUG-7 review: a lock taken on an earlier attempt is done, said as is (not "failed").
        assert_eq!(
            identity_failure(chain::top_up_settled_text().into()),
            CoreError::NotAvailable(chain::top_up_settled_text().into())
        );
    }

    /// BUG-9 r1: a complete pass is done; one with slots or names missing shows what it found
    /// and searches on; a failed one is told only while nothing was found.
    #[test]
    fn an_identity_search_is_done_only_when_complete() {
        let pass = |complete: bool| {
            Ok(chain::Discovery {
                found: 1,
                complete,
                slots_complete: complete,
                missing: (!complete).then(|| "slots".to_string()),
            })
        };
        let step = |status, found, again| DiscoveryStep {
            status,
            found,
            again,
        };
        // Complete.
        assert_eq!(
            discovery_step(pass(true), false),
            step(IdentitySearch::Done, true, false)
        );
        // Unanswered slots, or names not read: found, still searching, looking again.
        assert_eq!(
            discovery_step(pass(false), false),
            step(IdentitySearch::Searching, true, true)
        );
        assert_eq!(
            discovery_step(pass(false), true),
            step(IdentitySearch::Searching, true, true)
        );
        // Failed with nothing found yet: why; after a pass that found some: no error beside them.
        assert_eq!(
            discovery_step(Err("timeout".into()), false),
            step(IdentitySearch::Failed("timeout".into()), false, true)
        );
        assert_eq!(
            discovery_step(Err("timeout".into()), true),
            step(IdentitySearch::Searching, true, true)
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

    /// BUG-8: unlocking starts the chain itself, but never over a deleted wallet's files —
    /// the right password then says why, and the wallet stays locked.
    #[tokio::test]
    async fn unlocking_over_leftovers_says_restart() {
        let mut core = core_with_vault("unlock-leftovers", "pw");
        core.leftovers = true;
        assert_eq!(
            core.unlock(&pw("pw")).await,
            Err(CoreError::NotAvailable(LEFTOVERS.into()))
        );
        assert!(core.unlocked.is_none());
        // A wrong password is still said first.
        assert_eq!(core.unlock(&pw("no")).await, Err(CoreError::WrongPassword));
    }

    /// F3 (BUG-24): the password opens the wallet while the chain is still starting; the
    /// wallet goes into platform-wallet once it runs, and until then a payment or a Platform
    /// action is "wait for the sync", not an error. Locking forgets the phrase kept for that.
    #[tokio::test]
    async fn unlocking_does_not_wait_for_the_chain() {
        let mut core = core_with_vault("unlock-no-chain", "pw");
        assert!(!core.chain.running());
        assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
        let id = core.unlocked.as_ref().map(|o| o.id).unwrap();
        assert_eq!(id, chain::wallet_id_of(PHRASE).unwrap());
        assert!(core.attach.is_some(), "opened in platform-wallet later");
        assert_eq!(core.snapshot().await.vault, VaultState::Unlocked);
        assert_eq!(
            loaded(&core.chain, &id).await.err(),
            Some(CoreError::NotSynced)
        );
        core.lock().await.unwrap();
        assert!(core.attach.is_none() && core.unlocked.is_none());
    }

    /// The text of every file under `dir`, as anyone with the disk could read it.
    fn every_file_text(dir: &std::path::Path) -> String {
        let mut text = String::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                text += &every_file_text(&path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                text += &String::from_utf8_lossy(&bytes);
            }
        }
        text
    }

    fn has_notes(core: &Real) -> bool {
        core.notes.lock().unwrap().is_some()
    }

    /// FEAT-0003: the notes open with the wallet and go with its lock (the key and every
    /// decrypted note), are kept encrypted only, come back after the next unlock — with a new
    /// password too, since their key is the seed's (AC-T8) — and are not taken while locked.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_notes_open_with_the_wallet_and_go_with_its_lock() {
        let mut core = core_with_vault("notes-lock", "pw");
        assert!(!has_notes(&core));
        assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
        assert!(has_notes(&core));
        let note = |text: &str| Command::SetNote {
            key: "core:ab12".into(),
            text: NoteText(text.into()),
        };
        let (event, changed) = core.handle(note("For the logo ZQX-7731")).await;
        assert!(changed);
        assert!(matches!(
            event,
            Some(Event::Done {
                what: "note",
                result: Ok(())
            })
        ));
        assert!(core.paths.notes.exists());
        let folder = core.paths.vault.parent().unwrap().to_owned();
        assert!(!every_file_text(&folder).contains("ZQX-7731"));
        assert!(!every_file_text(&folder).contains("ab12"));
        // Not an operation's key: refused, nothing kept.
        let (event, _) = core
            .handle(Command::SetNote {
                key: "../wallet.json".into(),
                text: NoteText("x".into()),
            })
            .await;
        assert!(matches!(event, Some(Event::Done { result: Err(_), .. })));

        core.lock().await.unwrap();
        assert!(!has_notes(&core), "the key and the notes go with the lock");
        let (event, changed) = core.handle(note("while locked")).await;
        assert!(!changed && matches!(event, Some(Event::Done { result: Err(_), .. })));

        assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
        let (event, _) = core
            .handle(Command::ChangePassword {
                old: pw("pw"),
                new: pw("a new password 2026"),
            })
            .await;
        assert!(matches!(event, Some(Event::Done { result: Ok(()), .. })));
        core.lock().await.unwrap();
        assert_eq!(core.unlock(&pw("a new password 2026")).await, Ok(()));
        let notes = core.notes.lock().unwrap();
        let notes = notes.as_ref().unwrap();
        assert_eq!(notes.get("core:ab12"), Some("For the logo ZQX-7731"));
        assert!(!notes.unreadable());
    }

    /// Review r1 must 1 (§4a, lock → unlock): a notes file the unlock could not read (a folder
    /// in its place: no access) is not written over, "Details" says why, and the next unlock
    /// that reads it saves notes again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_notes_file_not_read_at_the_unlock_is_kept_until_one_reads_it() {
        let mut core = core_with_vault("notes-not-read", "pw");
        std::fs::create_dir_all(core.paths.notes.join("in-the-way")).unwrap();
        assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
        assert_eq!(
            core.notes.lock().unwrap().as_ref().unwrap().trouble(),
            Some(evolite_api::NotesError::NotRead)
        );
        let note = || Command::SetNote {
            key: "core:ab12".into(),
            text: NoteText("rent".into()),
        };
        let (event, _) = core.handle(note()).await;
        assert!(matches!(
            event,
            Some(Event::Done {
                what: "note",
                result: Err(CoreError::NotAvailable(_))
            })
        ));
        assert!(core.paths.notes.join("in-the-way").is_dir());
        core.lock().await.unwrap();
        std::fs::remove_dir_all(&core.paths.notes).unwrap();
        assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
        let (event, _) = core.handle(note()).await;
        assert!(matches!(event, Some(Event::Done { result: Ok(()), .. })));
        assert_eq!(core.notes.lock().unwrap().as_ref().unwrap().trouble(), None);
    }

    /// FEAT-0003 with BUG-76: "Add a wallet" locks this one (the loop locks the core it
    /// replaces), so its notes go; "Back" opens it again with its password, and they come back.
    #[tokio::test(flavor = "multi_thread")]
    async fn adding_a_wallet_drops_the_notes_and_back_opens_them_again() {
        let mut core = core_with_vault("notes-add", "pw");
        assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
        core.handle(Command::SetNote {
            key: "platform:00ff".into(),
            text: NoteText("rent".into()),
        })
        .await;
        let (_, changed) = core.handle(Command::AddWallet).await;
        assert!(changed && core.switch_to.is_some());
        // What the loop does with the core it replaces.
        core.lock().await.unwrap();
        assert!(!has_notes(&core) && core.unlocked.is_none());
        let dir = core.paths.root.clone();
        drop(core);
        // "Back": a new core for this wallet, locked until its password.
        let mut back = core_at(&dir, TEST_WALLET);
        assert!(!has_notes(&back));
        assert_eq!(back.snapshot().await.wallet.map(|w| w.notes_error), None);
        assert_eq!(back.unlock(&pw("pw")).await, Ok(()));
        assert_eq!(
            back.notes
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .get("platform:00ff"),
            Some("rent")
        );
    }

    /// FEAT-0003 (AC-T7, UC-7): a deleted wallet's notes go with its folder, so the same
    /// phrase restored later finds none.
    #[tokio::test]
    async fn a_deleted_wallets_notes_go_with_its_folder() {
        let dir = fresh_dir("notes-deleted");
        let paths = Paths::new(dir.clone(), TEST_WALLET);
        notes::Notes::open(paths.notes.clone(), &[1; 64], "ab")
            .set("core:aa", Some("rent".into()))
            .unwrap();
        // Deleted: the vault is gone, the rest is left for the next start.
        assert!(!paths.vault.exists() && paths.notes.exists());
        assert!(clear_leftovers(&paths).await);
        assert!(!paths.notes.exists());
        let restored = notes::Notes::open(paths.notes.clone(), &[1; 64], "ab");
        assert!(!restored.any() && !restored.unreadable());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-25: the open wallet goes into platform-wallet as soon as the store is open, so what
    /// the store has (identities, balance, history) is shown while the SPV client still starts;
    /// the phrase is kept until the client runs, and a payment still waits for the sync.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_store_is_read_before_the_chain_runs() {
        let mut core = core_with_vault("attach-store", "pw");
        assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
        let id = core.unlocked.as_ref().map(|o| o.id).unwrap();
        core.chain
            .open_store_only(&core.paths.chain)
            .await
            .expect("the store opens");
        assert!(core.chain.store_open() && !core.chain.running());
        assert_eq!(core.attach_wallet().await, Ok(()));
        assert!(core.chain.wallet(&id).await.is_some(), "before the client");
        assert!(core.attach.is_some(), "settled once the client runs");
        assert_eq!(
            loaded(&core.chain, &id).await.err(),
            Some(CoreError::NotSynced)
        );
        // Again while the client starts: nothing new to open, nothing to redraw.
        core.dirty.store(false, Ordering::Relaxed);
        assert_eq!(core.attach_wallet().await, Ok(()));
        assert!(!core.dirty.load(Ordering::Relaxed));
        // The client did not start (r1): the store goes with its wallets, and the next try
        // opens it again with the wallet put in anew.
        let first = core.chain.manager().await.expect("the manager");
        core.chain.let_store_go().await;
        assert!(!core.chain.store_open() && core.chain.wallet(&id).await.is_none());
        assert!(
            first.wallet_ids().await.is_empty(),
            "torn down, not only dropped"
        );
        drop(first);
        core.chain
            .open_store_only(&core.paths.chain)
            .await
            .expect("the store opens again");
        assert_eq!(core.attach_wallet().await, Ok(()));
        assert!(core.chain.wallet(&id).await.is_some());
        core.chain.stop().await;
        assert!(!core.chain.store_open());
    }

    /// FEAT-0009 review r1 must 1: the snapshot's lines are settled with what is the wallet's
    /// own ([`crate::layers::settle_wallet`], tested there with every kind it changes). A
    /// withdrawal to one of the wallet's Core addresses is a move between own balances, which
    /// only the wallet knows.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_snapshot_settles_its_lines_with_the_wallets_own_addresses() {
        let mut core = core_with_vault("settle-own", "pw");
        assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
        let open = core.unlocked.as_ref().unwrap();
        let (id, meta) = (open.id, open.meta.clone());
        core.chain
            .open_store_only(&core.paths.chain)
            .await
            .expect("the store opens");
        assert_eq!(core.attach_wallet().await, Ok(()));
        let wallet = core.chain.wallet(&id).await.expect("the wallet is in");
        let mine = wallet
            .state()
            .await
            .core_wallet
            .monitored_addresses()
            .first()
            .map(ToString::to_string)
            .expect("an address of the wallet");
        drop(wallet);
        for to in [mine.clone(), "yQeRmBYHdNaSt7QnKpGWLzRaZDZpSkdeKC".into()] {
            core.journal.add(journal::Entry {
                at: 1_793_000_000,
                what: journal::What::Withdrawal { to },
                spent: 100_000,
                moved: 99_000,
                ..Default::default()
            });
        }
        let shown = core.wallet(id, meta).await;
        let flow_to = |to: &str| {
            shown
                .activity
                .iter()
                .find(|a| matches!(&a.kind, ActivityKind::Withdrawal { to: t, .. } if t == to))
                .map(|a| a.flow)
        };
        assert_eq!(flow_to(&mine), Some(Flow::Between), "to the wallet's own");
        assert_eq!(
            flow_to("yQeRmBYHdNaSt7QnKpGWLzRaZDZpSkdeKC"),
            Some(Flow::Out)
        );
        core.chain.stop().await;
    }

    /// "Add a wallet" → "Create": the new wallet is born at the tip the wallet before saw, and
    /// is made while this core's chain still starts; platform-wallet gets it later.
    #[tokio::test]
    async fn a_wallet_is_made_while_the_chain_starts() {
        let dir = fresh_dir("create-starting");
        let mut core = core_at(&dir, "cccc0003");
        core.pending = Some((Zeroizing::new(PHRASE.into()), false));
        core.birth_hint = Some(777);
        core.starting = Some(tokio::spawn(std::future::pending::<bool>()));
        let made =
            tokio::time::timeout(Duration::from_secs(20), core.create(&pw("Second-wallet-7")))
                .await;
        assert_eq!(
            made.ok(),
            Some(Ok(())),
            "made without waiting for the start"
        );
        assert_eq!(core.saved.as_ref().map(|m| m.birth_height), Some(777));
        assert!(core.unlocked.is_some() && core.attach.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Making a wallet waits for the chain's start and takes its answer once: the loop then
    /// never polls the finished start again (that panicked the core thread).
    #[tokio::test]
    async fn a_settled_start_is_taken_once() {
        let mut core = core_with_vault("start-settled", "pw");
        core.starting = Some(tokio::spawn(async { false }));
        core.start_settled(Duration::from_secs(5)).await;
        assert!(core.starting.is_none() && core.leftovers && !core.start_tried);
        assert!(!core.start_ended().await);
        core.start_settled(Duration::from_secs(5)).await;
        // A start that does not end in time is left to the loop.
        core.starting = Some(tokio::spawn(std::future::pending::<bool>()));
        core.start_settled(Duration::from_millis(10)).await;
        assert!(core.starting.is_some());
        assert!(!core.start_ended().await);
    }

    /// F4 (BUG-21): deleting forgets the vault at once — the next snapshot says "no wallet"
    /// without waiting for the chain to stop (the loop stops it off the loop, F3).
    #[tokio::test]
    async fn deleting_says_no_wallet_at_once() {
        let mut core = core_with_vault("delete-at-once", "pw");
        assert_eq!(
            core.delete_wallet(Some(&pw("no"))).await,
            Err(CoreError::WrongPassword)
        );
        assert!(core.saved.is_some() && !core.restart);
        assert_eq!(core.delete_wallet(Some(&pw("pw"))).await, Ok(()));
        assert!(core.restart && !core.paths.vault.exists());
        assert_eq!(core.snapshot().await.vault, VaultState::NoWallet);
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
                p.platform_sync.clone(),
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
        assert!(core.switch_opened.is_some());
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

    /// BUG-65: "Add a wallet" → "Back": the wallet gone back to opens its own store while the
    /// adding chain's start (its SPV client, ≈50 s) still runs, so after the password its
    /// identities, balance and history are there at once — not only once that start and its
    /// stop have ended. The SPV client still waits for them (the SPV files are shared).
    #[tokio::test(flavor = "multi_thread")]
    async fn going_back_from_adding_reads_the_store_at_once() {
        let dir = fresh_dir("back-from-adding");
        seal_wallet(&dir, "bbbb0002");
        // The adding core: a new folder with no vault, its chain's start still under way.
        let adding = core_at(&dir, "cccc0003");
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let adding_start = tokio::spawn(async move {
            let _ = held.await;
            false
        });
        // The chain before the adding one has stopped: its start said so.
        let (said, before) = watch::channel(false);
        let _ = said.send(true);
        adding.chain.retire();

        let mut back = core_at(&dir, "bbbb0002");
        back.start_chain(
            Some(adding.chain.clone()),
            Some(adding_start),
            Some(before),
            true,
        );
        let opened = async {
            while !back.chain.store_open() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        assert!(
            tokio::time::timeout(Duration::from_secs(20), opened)
                .await
                .is_ok(),
            "the store opens while the adding chain still starts"
        );
        assert!(!back.chain.running(), "the SPV client waits");
        assert!(!back.starting.as_ref().unwrap().is_finished());
        // The password now: the wallet is read from the store before the client runs.
        assert_eq!(back.unlock(&pw("pw")).await, Ok(()));
        let id = back.unlocked.as_ref().map(|o| o.id).unwrap();
        assert!(back.chain.wallet(&id).await.is_some(), "at once");
        // The next core's store may open once this one's start has stopped the adding chain.
        let mut next = back.stopped.clone().unwrap();
        assert!(!*next.borrow());
        // No SPV client in a test: this chain is replaced before its turn comes.
        back.chain.retire();
        let _ = release.send(());
        let started = back.starting.take().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(20), started)
                .await
                .is_ok()
        );
        assert!(next.wait_for(|s| *s).await.is_ok());
        assert!(!back.chain.running());
        back.chain.stop().await;
        assert!(!back.chain.store_open());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-65, the other way: without word that the chains before have let their stores go —
    /// the core before had the same wallet, say — the store waits for the start before.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_store_held_before_is_not_opened_ahead() {
        let dir = fresh_dir("back-same-wallet");
        seal_wallet(&dir, "bbbb0002");
        let before = core_at(&dir, "bbbb0002");
        before.chain.retire();
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let start_before = tokio::spawn(async move {
            let _ = held.await;
            false
        });
        let mut back = core_at(&dir, "bbbb0002");
        back.start_chain(Some(before.chain.clone()), Some(start_before), None, false);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            !back.chain.store_open(),
            "not while the one before may hold it"
        );
        back.chain.retire();
        let _ = release.send(());
        let started = back.starting.take().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(20), started)
                .await
                .is_ok()
        );
        assert!(!back.chain.store_open());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-65 r1: the loop lets the next core open its store ahead only when the core before
    /// had another wallet — the same one again (a restart after a delete that failed) would
    /// open its store a second time.
    #[test]
    fn only_another_wallet_opens_its_store_ahead() {
        let said = || {
            let (stopped, before) = watch::channel(true);
            (stopped, Some(before))
        };
        let (_a, before) = said();
        assert!(open_ahead_after("aaaa0001", "bbbb0002", before).is_some());
        let (_b, before) = said();
        assert!(open_ahead_after("bbbb0002", "bbbb0002", before).is_none());
        assert!(open_ahead_after("aaaa0001", "bbbb0002", None).is_none());
    }

    /// A start whose store was opened ahead, after an adding start that ends when `release`
    /// is sent; the store is open when this returns.
    async fn opened_ahead(dir: &std::path::Path) -> (Real, Real, tokio::sync::oneshot::Sender<()>) {
        seal_wallet(dir, "bbbb0002");
        let adding = core_at(dir, "cccc0003");
        adding.chain.retire();
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let adding_start = tokio::spawn(async move {
            let _ = held.await;
            false
        });
        let (said, before) = watch::channel(false);
        let _ = said.send(true);
        let mut back = core_at(dir, "bbbb0002");
        back.start_chain(
            Some(adding.chain.clone()),
            Some(adding_start),
            Some(before),
            true,
        );
        let opened = async {
            while !back.chain.store_open() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        assert!(
            tokio::time::timeout(Duration::from_secs(20), opened)
                .await
                .is_ok()
        );
        (adding, back, release)
    }

    /// BUG-65 r1: the wallet whose store was opened ahead is deleted before its SPV client's
    /// turn, and its core replaced: the start ends at once — no 10 s of trying to remove a
    /// folder its own store holds, which held up the next wallet's start — starting nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_wallet_deleted_while_its_store_waits_holds_up_nothing() {
        let dir = fresh_dir("deleted-in-the-window");
        let (_adding, mut back, release) = opened_ahead(&dir).await;
        // Deleted: the vault goes at once (F4); the loop replaces the core.
        std::fs::remove_file(&back.paths.vault).expect("the vault");
        back.chain.retire();
        let _ = release.send(());
        let started = back.starting.take().unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(5), started).await;
        assert_eq!(ended.ok().map(|r| r.ok()), Some(Some(false)), "at once");
        assert!(!back.chain.running());
        back.chain.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same before the core is replaced: a deleted wallet's store opened ahead is not
    /// cleared under itself, and no SPV client starts for it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deleted_wallets_open_store_is_not_cleared_under_it() {
        let dir = fresh_dir("deleted-not-retired");
        let (_adding, mut back, release) = opened_ahead(&dir).await;
        std::fs::remove_file(&back.paths.vault).expect("the vault");
        let _ = release.send(());
        let started = back.starting.take().unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(5), started).await;
        assert_eq!(ended.ok().map(|r| r.ok()), Some(Some(false)), "at once");
        assert!(!back.chain.running(), "no client for a deleted wallet");
        back.chain.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The core of wallet "bbbb0002" with its store open, as after its unlock (no SPV client
    /// in a test), and the onboarding that "Add a wallet" makes from it: its chain parked.
    async fn adding_from_open_store(dir: &std::path::Path) -> (Arc<Chain>, Real) {
        seal_wallet(dir, "bbbb0002");
        let mut from = core_at(dir, "bbbb0002");
        from.chain
            .open_store_only(&from.paths.chain)
            .await
            .expect("the store opens");
        from.rescanned_from = Some(1_234);
        // A rescan asked before "Add a wallet" and not finished yet (BUG-82 review r1 should 1).
        OwedRescan::add(
            &mut from.rescan_owed,
            [0xa; 32],
            1_600_000,
            vec!["t".into()],
            Vec::new(),
        );
        let chain = from.chain.clone();
        let mut adding = core_at(dir, "cccc0003");
        hand_on_chain(&mut from, &mut adding, true);
        (chain, adding)
    }

    fn count(n: &std::sync::atomic::AtomicUsize) -> usize {
        n.load(Ordering::Relaxed)
    }

    /// The masternode lists in, as dash-spv reports them once they are.
    fn lists_in(chain: &Chain) {
        chain.set_status(|s| {
            s.masternodes = Some(sync::Stage {
                height: 1_000,
                target: 1_000,
                done: true,
            })
        });
    }

    /// Waits up to five seconds for `n` to reach `want` (a spawned ask).
    async fn counted(n: &std::sync::atomic::AtomicUsize, want: usize) -> usize {
        for _ in 0..50 {
            if count(n) >= want {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        count(n)
    }

    fn some_identity() -> String {
        Identifier::from([7; 32]).to_string(Encoding::Base58)
    }

    /// BUG-78: no Platform read leaves before the masternode lists are in — not even with
    /// the lists under way — and it is not counted as made; once they are, it goes out.
    #[tokio::test]
    async fn no_platform_read_goes_out_before_the_lists() {
        let chain = Chain::default();
        let id = Identifier::from([7; 32]);
        assert_eq!(chain.ready(), Err(chain::NOT_READY.to_owned()));
        chain.set_status(|s| {
            s.masternodes = Some(sync::Stage {
                height: 10,
                target: 1_000,
                done: false,
            })
        });
        assert_eq!(chain.name_of(id).await, Err(chain::NOT_READY.into()));
        assert_eq!(
            chain.resolve_name("dash").await,
            Err(chain::NOT_READY.into())
        );
        assert_eq!(
            chain.profile_of(id).await.err(),
            Some(chain::NOT_READY.into())
        );
        assert_eq!(
            chain.jembe_gifts(id, &[]).await.err(),
            Some(chain::NOT_READY.into())
        );
        assert_eq!(count(&chain.proof_reads), 0, "nothing went out");
        lists_in(&chain);
        assert_eq!(chain.ready(), Ok(()));
        // Past the guard: here it stops at the missing SDK, in the app it asks the network.
        assert_eq!(
            chain.name_of(id).await,
            Err("the chain is not started".into())
        );
        assert_eq!(count(&chain.proof_reads), 2);
    }

    /// BUG-78 (a): contacts read from the store before the lists are in are not asked for
    /// their names, nor marked asked; the first snapshot after the lists asks them.
    #[tokio::test(flavor = "multi_thread")]
    async fn contact_names_wait_for_the_lists_and_then_are_asked() {
        let mut core = core_with_vault("b78-names", "pw");
        let id = some_identity();
        core.ask_names(vec![id.clone()]);
        core.ask_names(vec![id.clone()]);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(core.names_asked.lock().unwrap().is_empty(), "none marked");
        assert_eq!(count(&core.chain.proof_reads), 0, "none asked");
        lists_in(&core.chain);
        core.ask_names(vec![id.clone()]);
        assert!(core.names_asked.lock().unwrap().contains(&id));
        assert_eq!(counted(&core.chain.proof_reads, 1).await, 1, "asked once");
        // Being asked: the next snapshot does not ask it again.
        core.ask_names(vec![id]);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(count(&core.chain.proof_reads), 1);
    }

    /// BUG-78 next to BUG-76: after "Add a wallet" → "Back" the wallet's core asks names
    /// through the parked chain it took over, once that chain has its lists.
    #[tokio::test(flavor = "multi_thread")]
    async fn after_going_back_from_adding_names_wait_for_the_parked_chains_lists() {
        let dir = fresh_dir("b78-back");
        let (chain, mut adding) = adding_from_open_store(&dir).await;
        let mut back = core_at(&dir, "bbbb0002");
        hand_on_chain(&mut adding, &mut back, false);
        assert!(Arc::ptr_eq(&back.chain, &chain));
        let id = some_identity();
        back.ask_names(vec![id.clone()]);
        assert!(back.names_asked.lock().unwrap().is_empty());
        lists_in(&chain);
        back.ask_names(vec![id]);
        assert_eq!(counted(&chain.proof_reads, 1).await, 1);
        back.chain.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-78: a read not made for want of the lists is "wait for the sync" to the UI.
    #[test]
    fn a_read_not_made_yet_is_wait_for_the_sync() {
        assert_eq!(read_failure(chain::NOT_READY.into()), CoreError::NotSynced);
        assert_eq!(
            read_failure("no answer".into()),
            CoreError::Failed("no answer".into())
        );
    }

    /// A chain's start under way, ending when the sender is used (or dropped).
    fn held_start() -> (
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<bool>,
    ) {
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let start = tokio::spawn(async move {
            let _ = held.await;
            true
        });
        (release, start)
    }

    /// BUG-76: "Add a wallet" → "Back" keeps the wallet's chain — not stopped, not started
    /// again, its store open throughout — and how far it was rescanned (BUG-64). The
    /// onboarding meanwhile starts no chain of its own, not even when a retry would be due.
    #[tokio::test(flavor = "multi_thread")]
    async fn going_back_from_adding_keeps_the_chain_running() {
        let dir = fresh_dir("b76-back");
        let (chain, mut adding) = adding_from_open_store(&dir).await;
        assert!(adding.parked.is_some());
        assert!(!Arc::ptr_eq(&adding.chain, &chain));
        assert!(adding.starting.is_none(), "the onboarding starts nothing");
        // A retry is due by the clock: still nothing (were it made, it would only count).
        adding.chain.retire();
        adding.last_start = tokio::time::Instant::now() - RETRY_START * 2;
        adding.retry_start();
        assert_eq!(count(&adding.chain.starts), 0, "no start while adding");
        assert!(chain.store_open() && !chain.retired());

        let mut back = core_at(&dir, "bbbb0002");
        hand_on_chain(&mut adding, &mut back, false);
        assert!(Arc::ptr_eq(&back.chain, &chain), "the same chain");
        assert_eq!((count(&chain.starts), count(&chain.stops)), (0, 0));
        assert!(chain.store_open() && !chain.retired());
        assert_eq!(back.rescanned_from, Some(1_234));
        let owed = back
            .rescan_owed
            .as_ref()
            .map(|o| (o.up_to, o.stale.clone()));
        assert_eq!(
            owed,
            Some((1_600_000, vec!["t".to_owned()])),
            "still owed after Back"
        );
        assert!(adding.rescan_owed.is_none());
        assert!(back.parked.is_none() && back.starting.is_none());
        // The password: the wallet is in that store at once.
        assert_eq!(back.unlock(&pw("pw")).await, Ok(()));
        let id = back.unlocked.as_ref().map(|o| o.id).unwrap();
        assert!(back.chain.wallet(&id).await.is_some());
        back.chain.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-76: "Add a wallet" → "Create": the parked chain stops (its start under way ends
    /// first) before the new wallet's chain starts, and the next core is told only then.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_wallet_made_while_adding_stops_the_parked_chain_first() {
        let dir = fresh_dir("b76-create");
        let (chain, mut adding) = adding_from_open_store(&dir).await;
        let (release, start) = held_start();
        adding.parked.as_mut().unwrap().starting = Some(start);
        adding.pending = Some((Zeroizing::new(PHRASE.into()), false));
        adding.birth_hint = Some(777);
        assert_eq!(adding.create(&pw("Second-wallet-7")).await, Ok(()));
        // No SPV client in a test: the new chain is retired before its turn comes.
        adding.chain.retire();
        assert!(adding.parked.is_none());
        assert!(chain.retired(), "a start of it waiting in line is not made");
        assert_eq!(count(&adding.chain.starts), 1, "its own start is asked");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(chain.store_open(), "the start before is waited for");
        let mut next = adding.stopped.clone().unwrap();
        assert!(!*next.borrow());
        let _ = release.send(());
        let started = adding.starting.take().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(20), started)
                .await
                .is_ok()
        );
        assert_eq!(count(&chain.stops), 1, "the parked chain is stopped");
        assert!(!chain.store_open());
        assert!(next.wait_for(|s| *s).await.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-76: from the onboarding to a third wallet (not "Back"): the parked chain stops
    /// before that wallet's start, as after a switch.
    #[tokio::test(flavor = "multi_thread")]
    async fn another_wallet_after_adding_stops_the_parked_chain() {
        let dir = fresh_dir("b76-third");
        let (chain, mut adding) = adding_from_open_store(&dir).await;
        let (release, start) = held_start();
        adding.parked.as_mut().unwrap().starting = Some(start);
        seal_wallet(&dir, "aaaa0001");
        let mut third = core_at(&dir, "aaaa0001");
        hand_on_chain(&mut adding, &mut third, false);
        // No SPV client in a test: the new chain is retired before its turn comes.
        third.chain.retire();
        let _ = release.send(());
        assert!(!Arc::ptr_eq(&third.chain, &chain));
        let started = third.starting.take().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(20), started)
                .await
                .is_ok()
        );
        assert_eq!(count(&chain.stops), 1);
        assert!(!chain.store_open() && chain.retired());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-76 r1, should 1: a parked chain that does not run ("Add a wallet" while offline)
    /// is started again during the onboarding — that chain, not the onboarding's own — but
    /// not before [`RETRY_START`], and not over a start of it still under way.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_parked_chain_that_did_not_start_is_started_again() {
        let dir = fresh_dir("b76-retry-parked");
        let (chain, mut adding) = adding_from_open_store(&dir).await;
        // No SPV client in a test: retired, a start that is made only counts.
        chain.retire();
        let due = tokio::time::Instant::now() - RETRY_START * 2;
        adding.parked.as_mut().unwrap().last_start = tokio::time::Instant::now();
        adding.retry_start();
        assert_eq!(count(&chain.starts), 0, "not due yet");
        let (release, start) = held_start();
        {
            let parked = adding.parked.as_mut().unwrap();
            parked.starting = Some(start);
            parked.last_start = due;
        }
        adding.retry_start();
        assert_eq!(count(&chain.starts), 0, "a start under way is not doubled");
        let _ = release.send(());
        let finished = || {
            adding
                .parked
                .as_ref()
                .and_then(|p| p.starting.as_ref())
                .is_some_and(|t| t.is_finished())
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !finished() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        adding.retry_start();
        assert_eq!(count(&chain.starts), 1, "the parked chain is started again");
        assert_eq!(
            count(&adding.chain.starts),
            0,
            "the onboarding's own is not"
        );
        let again = adding.parked.as_mut().unwrap().starting.take().unwrap();
        assert_eq!(again.await.ok(), Some(false), "retired: nothing is made");
        adding.retry_start();
        assert_eq!(
            count(&chain.starts),
            1,
            "just tried: not again before RETRY_START"
        );
        chain.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-76 r1, should 2 (M3): "Add a wallet" → "Create" births the new wallet at the tip of
    /// the parked chain's headers, not at the lower hint the wallet before left.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_wallet_made_while_adding_is_born_at_the_parked_chains_tip() {
        let dir = fresh_dir("b76-birth");
        let (chain, mut adding) = adding_from_open_store(&dir).await;
        chain.set_status(|s| {
            s.headers = Some(sync::Stage {
                height: 900,
                target: 900,
                done: true,
            })
        });
        let (release, start) = held_start();
        adding.parked.as_mut().unwrap().starting = Some(start);
        adding.pending = Some((Zeroizing::new(PHRASE.into()), false));
        adding.birth_hint = Some(777);
        assert_eq!(adding.create(&pw("Second-wallet-7")).await, Ok(()));
        // No SPV client in a test: the new chain is retired before its turn comes.
        adding.chain.retire();
        assert_eq!(adding.saved.as_ref().map(|m| m.birth_height), Some(900));
        let _ = release.send(());
        let started = adding.starting.take().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(20), started)
                .await
                .is_ok()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-76 r1, should 3 (M4): a DashPay import during "Add a wallet" reads Platform through
    /// the parked chain: the onboarding's own never starts, and through it the import would
    /// wait out its 30 s and find nothing. Without an onboarding, through the core's own.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dashpay_import_while_adding_goes_through_the_parked_chain() {
        let dir = fresh_dir("b76-dashpay");
        seal_wallet(&dir, "bbbb0002");
        let mut from = core_at(&dir, "bbbb0002");
        assert!(
            Arc::ptr_eq(from.network_chain(), &from.chain),
            "no onboarding"
        );
        let chain = from.chain.clone();
        let mut adding = core_at(&dir, "cccc0003");
        hand_on_chain(&mut from, &mut adding, true);
        assert!(Arc::ptr_eq(adding.network_chain(), &chain));
        // Proofs can be checked; no store is open, so the scan ends at once, off the network.
        chain.set_status(|s| {
            s.masternodes = Some(sync::Stage {
                height: 1,
                target: 1,
                done: true,
            })
        });
        let other = "legal winner thank year wave sausage worth useful legal winner thank yellow";
        let phrase = Phrase::new(other.split_whitespace().map(str::to_owned).collect());
        let (event, _) = adding
            .handle(Command::UsePhrase {
                phrase,
                dashpay: true,
            })
            .await;
        assert!(event.is_none(), "answered later");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while count(&chain.dashpay_scans) == 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(count(&chain.dashpay_scans), 1, "through the parked chain");
        assert_eq!(count(&adding.chain.dashpay_scans), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-76: the app closes during the onboarding: the parked chain stops with the core's
    /// own, its store let go.
    #[tokio::test(flavor = "multi_thread")]
    async fn quitting_while_adding_stops_both_chains() {
        let dir = fresh_dir("b76-quit");
        let (chain, mut adding) = adding_from_open_store(&dir).await;
        let own = adding.chain.clone();
        adding.shut_down().await;
        assert_eq!((count(&own.stops), count(&chain.stops)), (1, 1));
        assert!(!chain.store_open() && adding.parked.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-76, the paths that park nothing keep their order: a switch stops the chain before
    /// the next wallet's start (BUG-21/22/24), and opens that store ahead only for another
    /// wallet (BUG-65).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_switch_still_stops_the_chain_before() {
        let dir = fresh_dir("b76-switch");
        seal_wallet(&dir, "aaaa0001");
        seal_wallet(&dir, "bbbb0002");
        let mut from = core_at(&dir, "bbbb0002");
        from.chain
            .open_store_only(&from.paths.chain)
            .await
            .expect("the store opens");
        let chain = from.chain.clone();
        let (release, start) = held_start();
        from.starting = Some(start);
        let mut to = core_at(&dir, "aaaa0001");
        hand_on_chain(&mut from, &mut to, false);
        to.chain.retire();
        let _ = release.send(());
        assert!(to.parked.is_none() && chain.retired());
        let started = to.starting.take().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(20), started)
                .await
                .is_ok()
        );
        assert_eq!(count(&chain.stops), 1);
        assert!(!chain.store_open());
        assert_eq!(count(&to.chain.starts), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R-28 (BUG-44): another wallet goes by its own password, files only; the open one stays
    /// unlocked and no new core is asked for.
    #[tokio::test]
    async fn deletes_another_wallet_by_its_password_and_keeps_the_open_one() {
        let dir = fresh_dir("delete-other");
        seal_wallet(&dir, "aaaa0001");
        seal_wallet(&dir, "bbbb0002");
        let art = dir.join("art");
        std::fs::create_dir_all(art.join("bbbb0002")).unwrap();
        std::fs::create_dir_all(art.join("aaaa0001")).unwrap();
        let mut core = core_at(&dir, "aaaa0001");
        core.art_base = Some(art.clone());
        assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
        let other = wallets::wallet_dir(&dir, "bbbb0002");
        std::fs::write(other.join("platform-wallet.db"), b"store").unwrap();
        let delete = |password: Option<&str>, wallet: &str| Command::DeleteWallet {
            password: password.map(pw),
            wallet: Some(wallet.into()),
        };
        let answer = |event: Option<Event>| match event {
            Some(Event::Done { what, result }) => (what, result),
            other => panic!("{other:?}"),
        };
        let untouched = |core: &Real| {
            assert!(core.unlocked.is_some(), "the open wallet stays unlocked");
            assert!(!core.restart && core.switch_to.is_none(), "no new core");
        };

        // A wrong password, or none: nothing goes.
        for password in [Some("wrong"), None] {
            let (event, _) = core.handle(delete(password, "bbbb0002")).await;
            assert_eq!(
                answer(event),
                ("delete-other", Err(CoreError::WrongPassword))
            );
            assert!(other.join("wallet.json").exists());
            untouched(&core);
        }
        // Its own password: its folder, record and pictures go; the open one's stay.
        let (event, changed) = core.handle(delete(Some("pw"), "bbbb0002")).await;
        assert_eq!(answer(event), ("delete-other", Ok(())));
        assert!(changed, "the list of wallets is sent again");
        assert!(!other.exists());
        assert!(!art.join("bbbb0002").exists() && art.join("aaaa0001").exists());
        let ids: Vec<String> = wallets::load(&dir)
            .wallets
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["aaaa0001"]);
        assert_eq!(core.wallet_entries().len(), 1);
        assert!(core.paths.vault.exists());
        untouched(&core);
        // Not there any more.
        let (event, _) = core.handle(delete(Some("pw"), "bbbb0002")).await;
        assert!(matches!(
            answer(event),
            ("delete-other", Err(CoreError::NotAvailable(_)))
        ));
        // The open one's id is the open one's deletion, as before.
        let (event, _) = core.handle(delete(Some("wrong"), "aaaa0001")).await;
        assert_eq!(answer(event), ("delete", Err(CoreError::WrongPassword)));
        untouched(&core);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn entry(core: &mut Real, id: &str) -> WalletEntry {
        core.wallet_entries()
            .into_iter()
            .find(|w| w.id == id)
            .expect("the wallet is listed")
    }

    fn answer_of(event: Option<Event>) -> (&'static str, Result<(), CoreError>) {
        match event {
            Some(Event::Done { what, result }) => (what, result),
            other => panic!("{other:?}"),
        }
    }

    /// FEAT-0002 (AC-A2, AC-A5, AC-A10, AC-T2): the 5th wrong password in a row starts a pause
    /// of 30 s; in it even the right password is refused without the vault being read, and a
    /// new core over the folder (a restart) is in the same pause.
    #[tokio::test]
    async fn the_fifth_wrong_password_pauses_the_unlock_even_after_a_restart() {
        let dir = fresh_dir("unlock-pause");
        seal_wallet(&dir, "aaaa0001");
        seal_wallet(&dir, "bbbb0002");
        let mut core = core_at(&dir, "aaaa0001");
        let unlock = |p: &str| Command::Unlock { password: pw(p) };
        for n in 1..=5 {
            let (event, changed) = core.handle(unlock("no")).await;
            assert_eq!(
                answer_of(event),
                ("unlock", Err(CoreError::WrongPassword)),
                "try {n}"
            );
            assert!(changed, "the count goes out with the next snapshot");
            assert_eq!(entry(&mut core, "aaaa0001").wrong_tries, n);
            assert_eq!(entry(&mut core, "aaaa0001").wait_until.is_some(), n == 5);
        }
        let until = entry(&mut core, "aaaa0001").wait_until.unwrap();
        assert!((29..=31).contains(&(until - now())), "{}", until - now());
        // The vault is not even read: without it the answer is still the pause.
        let vault = wallets::wallet_dir(&dir, "aaaa0001").join("wallet.json");
        let sealed = std::fs::read(&vault).unwrap();
        std::fs::remove_file(&vault).unwrap();
        let (event, _) = core.handle(unlock("pw")).await;
        assert_eq!(
            answer_of(event),
            ("unlock", Err(CoreError::TooManyTries { until }))
        );
        std::fs::write(&vault, sealed).unwrap();
        assert!(core.unlocked.is_none());
        assert_eq!(
            entry(&mut core, "aaaa0001").wrong_tries,
            5,
            "a refusal is no try"
        );
        // The other wallet has no pause.
        let other = entry(&mut core, "bbbb0002");
        assert_eq!((other.wrong_tries, other.wait_until), (0, None));
        // A restart: the same pause.
        let mut again = core_at(&dir, "aaaa0001");
        assert_eq!(entry(&mut again, "aaaa0001").wait_until, Some(until));
        assert_eq!(entry(&mut again, "aaaa0001").wrong_tries, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-0002 (AC-A4, AC-A6, AC-A7, AC-A8, AC-A9): errors that are not the password do not
    /// count, the right one clears the count, "Open "…"" counts for the wallet it opens, and a
    /// deleted wallet's count goes with it.
    #[tokio::test]
    async fn only_wrong_passwords_count_each_for_its_own_wallet() {
        let dir = fresh_dir("unlock-count");
        seal_wallet(&dir, "aaaa0001");
        seal_wallet(&dir, "bbbb0002");
        let mut core = core_at(&dir, "aaaa0001");
        let unlock = |p: &str| Command::Unlock { password: pw(p) };
        for _ in 0..2 {
            core.handle(unlock("no")).await;
        }
        // The right password over a deleted wallet's files: an error, but not the password's.
        core.leftovers = true;
        let (event, _) = core.handle(unlock("pw")).await;
        assert!(matches!(
            answer_of(event),
            ("unlock", Err(CoreError::NotAvailable(_)))
        ));
        assert_eq!(entry(&mut core, "aaaa0001").wrong_tries, 2);
        core.leftovers = false;
        let (event, _) = core.handle(unlock("pw")).await;
        assert_eq!(answer_of(event), ("unlock", Ok(())));
        assert_eq!(entry(&mut core, "aaaa0001").wrong_tries, 0);
        assert_eq!(
            attempts::Attempts::read(&dir, ["aaaa0001"]).failures("aaaa0001"),
            0
        );
        // "Open "Savings"" with a wrong password: Savings counts, the open wallet does not.
        let switch = |p: &str| Command::SwitchWallet {
            id: "bbbb0002".into(),
            password: Some(pw(p)),
        };
        let (event, changed) = core.handle(switch("no")).await;
        assert_eq!(answer_of(event), ("switch", Err(CoreError::WrongPassword)));
        assert!(changed);
        assert_eq!(entry(&mut core, "bbbb0002").wrong_tries, 1);
        assert_eq!(entry(&mut core, "aaaa0001").wrong_tries, 0);
        assert!(core.unlocked.is_some(), "the open wallet stays open");
        // Savings deleted: nothing of its count is left.
        let (event, _) = core
            .handle(Command::DeleteWallet {
                password: Some(pw("pw")),
                wallet: Some("bbbb0002".into()),
            })
            .await;
        assert_eq!(answer_of(event), ("delete-other", Ok(())));
        assert_eq!(core.attempts.failures("bbbb0002"), 0);
        assert!(
            !wallets::wallet_dir(&dir, "bbbb0002")
                .join(attempts::FILE)
                .exists()
        );
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

    /// The core of `core_with_vault` with its wallet open (no keys: nothing can be signed).
    fn opened(name: &str, password: &str) -> Real {
        let mut core = core_with_vault(name, password);
        let meta = core.saved.clone().unwrap();
        core.unlocked = Some(Unlocked {
            id: [0; 32],
            meta,
            keys: None,
            seed: None,
        });
        core
    }

    /// Events compared by what they say (`Event` has no `PartialEq`).
    fn debug(event: &Option<Event>) -> String {
        format!("{event:?}")
    }

    /// Every spend there is (R-27 AC-T1).
    fn every_spend() -> Vec<Spend> {
        vec![
            Spend::Send {
                from: Source::Core,
                to: "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ".into(),
                amount: 10_000,
                note: None,
            },
            Spend::TopUpIdentity { amount: 10_000 },
            Spend::ResumeTopUp,
            Spend::CreateIdentity {
                name: Some("satoshi-test".into()),
                top_up: 10_000,
            },
            Spend::CreateIdentity {
                name: None,
                top_up: 10_000,
            },
            Spend::ResumeIdentity,
            Spend::AddName {
                name: "satoshi-test".into(),
            },
            Spend::Shield {
                from: ShieldSource::Core,
                amount: 10_000,
            },
            Spend::Unshield { amount: 10_000 },
            Spend::TopUpFromPool { amount: 10_000 },
            Spend::AddContact {
                name: "alice".into(),
            },
            Spend::AcceptContact {
                identity_id: "F5teLR3Vq4JjzQA7jgdGjZQvJbKpMubbu5dsC4MJd1Eh".into(),
            },
            Spend::SaveProfile {
                display: "A".into(),
                about: String::new(),
                avatar_url: None,
            },
            Spend::ApproveApp { id: 7 },
            Spend::GiftNft {
                id: "nft".into(),
                to: "@alice".into(),
            },
        ]
    }

    /// BUG-42, R-27 AC-T1: with "password for every send" on, no spend goes without the
    /// password, and none with a wrong one; each is answered as its own failure, and the core
    /// went no further than the gate (no lock, no broadcast: the answer is the gate's).
    #[tokio::test]
    async fn with_the_setting_on_no_spend_goes_without_the_right_password() {
        let mut core = opened("gate", "right");
        assert!(core.password_each_send, "on by default");
        for op in every_spend() {
            let none = Command::Spend {
                op: op.clone(),
                password: None,
            };
            let (event, _) = core.handle(none).await;
            assert_eq!(
                debug(&event),
                debug(&Some(op.refused(CoreError::NotAvailable(
                    crate::PASSWORD_NEEDED.into()
                )))),
                "{} without the password",
                op.what()
            );
            let wrong = Command::Spend {
                op: op.clone(),
                password: Some(pw("wrong")),
            };
            let (event, _) = core.handle(wrong).await;
            assert_eq!(
                debug(&event),
                debug(&Some(op.refused(CoreError::WrongPassword))),
                "{} with a wrong password",
                op.what()
            );
        }
        // The right one gets past the gate, to the payment's own checks: not synced.
        let right = Command::Spend {
            op: every_spend().remove(0),
            password: Some(pw("right")),
        };
        assert_eq!(
            debug(&core.handle(right).await.0),
            debug(&Some(Event::Sent(Err(CoreError::NotSynced))))
        );
    }

    /// R-27 AC-T2: off only with the wallet's password; then a spend needs none, and the
    /// choice is kept for the next core of this wallet.
    #[tokio::test]
    async fn the_setting_goes_off_only_with_the_password() {
        let mut core = opened("each-off", "right");
        let off = |password: Option<&str>| Command::SetPasswordEachSend {
            on: false,
            password: password.map(pw),
        };
        for password in [None, Some("wrong")] {
            assert!(matches!(
                core.handle(off(password)).await.0,
                Some(Event::Done {
                    what: "password-each-send",
                    result: Err(CoreError::WrongPassword)
                })
            ));
            assert!(core.password_each_send);
        }
        let (event, _) = core.handle(off(Some("right"))).await;
        assert!(matches!(
            event,
            Some(Event::Done {
                what: "password-each-send",
                result: Ok(())
            })
        ));
        assert!(!core.password_each_send);
        assert!(!security::load(&core.paths.root, &core.wallet), "kept");
        let send = Command::Spend {
            op: every_spend().remove(0),
            password: None,
        };
        assert_eq!(
            debug(&core.handle(send).await.0),
            debug(&Some(Event::Sent(Err(CoreError::NotSynced))))
        );
        // On again at once, without the password.
        let on = Command::SetPasswordEachSend {
            on: true,
            password: None,
        };
        core.handle(on).await;
        assert!(core.password_each_send && security::load(&core.paths.root, &core.wallet));
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
                None,
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
        let answer = |id, approve| match approve {
            true => Command::Spend {
                op: Spend::ApproveApp { id },
                password: Some(pw("pw")),
            },
            false => Command::DeclineApp { id },
        };
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

    /// BUG-77 r1 should 1: a payment from Core (send, top-up, a new identity) is checked against
    /// the coins each once. werner555 had 7 coins in two accounts: platform-wallet's balance
    /// counted them twice and would have let a payment of more than the wallet has through.
    #[test]
    fn a_coin_two_accounts_hold_does_not_pay_twice() {
        use key_wallet::dashcore::hashes::Hash;
        use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
        use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
        let keys = keys_of(PHRASE).unwrap();
        let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
        let address = key_wallet::dashcore::Address::new(
            key_wallet::Network::Testnet,
            key_wallet::dashcore::address::Payload::PubkeyHash(
                key_wallet::dashcore::PubkeyHash::from_byte_array([2; 20]),
            ),
        );
        let mut coin = key_wallet::Utxo::new(
            key_wallet::dashcore::OutPoint::new(
                key_wallet::dashcore::Txid::from_byte_array([2; 32]),
                0,
            ),
            key_wallet::dashcore::TxOut {
                value: 5_050_000,
                script_pubkey: Default::default(),
            },
            address,
            1_570_000,
            false,
        );
        coin.is_confirmed = true;
        {
            let mut accounts = info.accounts_mut().all_funding_accounts_mut();
            accounts[0].utxos.insert(coin.outpoint, coin.clone());
            accounts[1].utxos.insert(coin.outpoint, coin);
        }
        info.update_balance();
        assert_eq!(info.balance().total(), 10_100_000, "upstream: twice");
        assert!(core_covers(&info, 5_050_000).is_ok());
        assert!(matches!(
            core_covers(&info, 5_050_001),
            Err(CoreError::NotEnough)
        ));
        assert!(matches!(
            core_covers(&info, 10_100_000),
            Err(CoreError::NotEnough)
        ));
    }

    /// CS-1a r2 must 1: «Try again» reaches the NFT ask through the core. After a failed
    /// ask `RefreshNfts` makes the next snapshot ask at once, not after `REFRESH`, and asks
    /// for a new snapshot to do it.
    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_nfts_asks_at_once_after_a_failed_ask() {
        let mut core = core_at(&fresh_dir("refresh-nfts"), TEST_WALLET);
        let owner = "owner";
        assert!(core.nfts.due(owner), "the first ask");
        core.nfts.found(owner, "@o", Err("timeout".into()));
        assert!(
            !core.nfts.due(owner),
            "on its own, the next ask waits for REFRESH"
        );
        let (event, snapshot) = core.handle(Command::RefreshNfts).await;
        assert!(
            event.is_none() && snapshot,
            "no answer of its own: a new snapshot"
        );
        assert!(core.nfts.due(owner), "asked again at once");
    }
}
