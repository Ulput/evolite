//! The real core on platform-wallet: the SPV client, the wallet's state and its SQLite store are
//! platform-wallet's ([`chain`]); the phrase is encrypted on disk (the vault) and the keys are
//! derived from it only while the wallet is open ([`Unlocked`]). platform-wallet holds the
//! wallet watch-only.
//!
//! This file holds the core's state ([`Real`]) and what every part of it uses. Its methods live
//! by area, as `impl Real` in child modules, which see the private fields:
//!
//! - [`run`]: the loop: commands in, snapshots and answers out;
//! - [`dispatch`]: which operation each command is;
//! - [`snapshot`]: what the interface is shown;
//! - [`lifecycle`]: starting and stopping the chain, the data folder;
//! - `ops::wallet` (a wallet and its password), `ops::core` (Core payments), `ops::notes`,
//!   `ops::identity` (identities, credits, names), `ops::names` (DPNS lookups), `ops::dashpay`,
//!   `ops::pool`, `ops::misc`.
//!
//! Calls into platform-wallet are in [`chain`] and its neighbours (`identity`, `dashpay`, `pool`,
//! …). Tests are in `tests/`, by area.

mod dispatch;
mod lifecycle;
mod ops;
mod prices;
mod run;
mod snapshot;

use lifecycle::*;
use ops::core::*;
use ops::dashpay::*;
use ops::identity::*;
use ops::notes::*;
use ops::pool::*;
use ops::wallet::*;
use ops::{app_jobs, avatars, outgoing, platform_sending};
pub(crate) use run::*;
use snapshot::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use evolite_api::*;
pub(crate) use evolite_chain::keys_of;
use evolite_common::{hex, now, wall_secs};
use key_wallet::mnemonic::{Language, Mnemonic};
use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use key_wallet_manager::WalletId;
use tokio::sync::watch;
use zeroize::Zeroizing;

use crate::dpns;
use crate::network::{platform_address_here, platform_hrp};
use chain::{Chain, ChainPaths, CreditPayee};
use convert::{
    activity, coins_known, count_checked, fees, not_an_address, payee, platform_fees, sync_info,
};
use dpp::address_funds::PlatformAddress;
use dpp::identity::accessors::IdentityGettersV0;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use evolite_chain::{
    app_connect, chain, convert, dashpay, identity, mined, nfts, platform_history, pool, store,
    sync, turn,
};
use evolite_local::{
    address_book, attempts, connected_apps, contact_addresses, core_balance, folders,
    hidden_requests, journal, mempool, node, notes, pending, pool_history, rates, security, vault,
    wallets,
};
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

/// The phrase's BIP-39 seed (no passphrase), zeroed when dropped.
fn seed_of(phrase: &str) -> Result<Zeroizing<[u8; 64]>, CoreError> {
    let mnemonic = Mnemonic::from_phrase(phrase).map_err(|_| CoreError::BadPhrase)?;
    Ok(Zeroizing::new(mnemonic.to_seed("")))
}

/// Said when the open wallet is not (or no longer) in platform-wallet.
const NOT_LOADED: &str = tr!("The wallet is not loaded");

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
    /// What each spend was said to cost, until its answer comes (TECH-price-check).
    prices: prices::PriceWatch,
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

/// How long an import from DashPay waits for the masternode lists before it goes on without
/// the scan. Proofs need them, and on a first start they come only after every header — many
/// minutes: the onboarding goes on, and the identity is found after the wallet's sync instead.
const DASHPAY_SCAN_WAIT: Duration = Duration::from_secs(30);

/// The pause before the contacts' key accounts are tried again while some are missing.
const CONTACT_ACCOUNTS_RETRY: Duration = Duration::from_secs(30);

/// How often the identities' credit balances are read from Platform when nothing asks sooner.
const BALANCES_EVERY: Duration = Duration::from_secs(60);

/// Every how many readings all the identities are read, not only the shown one.
const BALANCES_ALL_EVERY: u32 = 10;

/// A pool that did not connect is tried again after this.
const POOL_RETRY: Duration = Duration::from_secs(60);

/// About how long an Orchard proof takes here, for the progress estimate.
const POOL_PROOF_SECS: f32 = 30.0;

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

/// The fewest characters a new password may have. The screens ask for more (letters of both
/// cases, a digit, a symbol); the core holds the floor itself, so no other way in sets a
/// password that an offline guess at the vault finds in minutes.
const MIN_PASSWORD: usize = 8;

/// Said when a deleted wallet's files are still held by this run of evolite.
const LEFTOVERS: &str = tr!(
    "The wallet is deleted, but its files are still held by this run of evolite. Restart evolite — they are deleted at start, and then you can make or restore a wallet"
);

/// How often a second window asks again whether the first one has closed.
const LOCK_RETRY: Duration = Duration::from_secs(3);

#[cfg(test)]
mod tests;
