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

mod busy;
mod dispatch;
mod lifecycle;
mod ops;
mod prices;
mod run;
mod sent;
mod snapshot;
mod works;

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
    chain, contact_scans, convert, dashpay, identity, mined, nfts, platform_history, pool, store,
    sync, turn,
};
use evolite_local::{
    address_book, attempts, connected_apps, contact_addresses, contact_names, core_balance,
    folders, hidden_requests, journal, mempool, name_requests, node, notes, pending, pool_history,
    rates, resend_debt, security, vault, wallets,
};
use sync::{Phase, SyncStatus};
use vault::{KdfParams, VaultError, VaultMeta};

/// What the real wallet does not do yet (Platform itself is connected, these are not).
const NO_GIFTS: &str =
    tr!("An NFT cannot be given from evolite yet: transfers come in later versions");
/// FEAT-0019 part 1: the API is there, the real core's tokens come in part 3.
const NO_TOKENS: &str = tr!("Tokens cannot be read or sent from evolite yet");
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

/// How many contacts' names are read at once (SYNC-4): less load on DAPI.
const NAMES_AT_ONCE: usize = 4;

/// Whether Platform refused a transition for a key or its signature (BUG-175): a key disabled or
/// missing there, a purpose or level it does not allow, a revision behind, a signature that does
/// not check — what a key set changed in another app looks like. By the consensus errors' names
/// and their words (rs-dpp 575d961).
fn about_keys(text: &str) -> bool {
    const NAMES: [&str; 9] = [
        "PublicKeyIsDisabled",
        "IdentityPublicKeyIsDisabled",
        "InvalidSignaturePublicKeyPurpose",
        "InvalidSignaturePublicKeySecurityLevel",
        "InvalidStateTransitionSignature",
        "MissingPublicKey",
        "InvalidIdentityRevision",
        "WrongPublicKeyPurpose",
        "PublicKeySecurityLevelNotMet",
    ];
    const WORDS: [&str; 7] = [
        " is disabled",
        "invalid state transition signature",
        "doesn't exist",
        "has invalid revision",
        "invalid public key purpose",
        "invalid identity key purpose",
        "invalid public key security level",
    ];
    let lower = text.to_lowercase();
    NAMES.iter().any(|n| text.contains(n))
        || (lower.contains("key") || lower.contains("signature") || lower.contains("revision"))
            && WORDS.iter().any(|w| lower.contains(w))
}

/// Whether an answer is a refusal for a key (BUG-175): then the identities are read again.
fn refused_for_keys(event: &Event) -> bool {
    matches!(
        event,
        Event::Done {
            result: Err(CoreError::KeysChanged),
            ..
        } | Event::Sent {
            result: Err(CoreError::KeysChanged),
            ..
        }
    )
}

/// A Platform write that failed, as the UI is told — one rule for every write
/// (TECH-platform-errors, audit Д3; before, five functions disagreed):
/// - not made because the masternode lists are not in yet: wait for the sync;
/// - refused for a key (disabled, wrong purpose or level, a revision behind): the keys are read
///   again ([`CoreError::KeysChanged`], BUG-175);
/// - a name's credits not known: said as it is, nothing started (BUG-97);
/// - Platform's refusals for lack of credits — «insufficient …», «… is not enough to pay … fee»
///   — and only those: «Platform balance unavailable…» is a read that failed (BUG-107);
/// - no identity yet: make one first;
/// - else the text, for the details line.
///
/// What only one kind of write knows (the coins' own refusal, a top-up's chain lock) its caller
/// says first.
pub(in crate::real) fn platform_failure(text: String) -> CoreError {
    let lower = text.to_lowercase();
    if text == chain::NOT_READY {
        CoreError::NotSynced
    } else if about_keys(&text) {
        CoreError::KeysChanged
    } else if text.contains(chain::name_credits_unknown_text()) {
        CoreError::NotAvailable(chain::name_credits_unknown_text().into())
    } else if lower.contains("insufficient") || lower.contains("is not enough") {
        CoreError::NotEnough
    } else if text == tr!("The wallet has no identity") {
        CoreError::NotAvailable(tr!("Make an identity first").into())
    } else {
        CoreError::Failed(text)
    }
}

#[cfg(test)]
mod platform_failure_tests {
    use super::*;

    /// TECH-platform-errors: one table for every write, by Platform's own words. Before, «… is
    /// not enough to pay … fee» was `NotEnough` only for DashPay, a key's refusal was not
    /// `KeysChanged` for a name or a pool move, and «no identity» was said two ways.
    #[test]
    fn every_write_reads_platforms_refusals_alike() {
        let not_enough = [
            "Insufficient identity abc balance 126042000 required 190000000",
            "Current credits balance 1 is not enough to pay 2 fee",
        ];
        for text in not_enough {
            assert_eq!(
                platform_failure(text.into()),
                CoreError::NotEnough,
                "{text}"
            );
        }
        assert_eq!(
            platform_failure("Protocol error: IdentityPublicKeyIsDisabledError key 3".into()),
            CoreError::KeysChanged
        );
        assert_eq!(
            platform_failure(chain::NOT_READY.into()),
            CoreError::NotSynced
        );
        assert_eq!(
            platform_failure(tr!("The wallet has no identity").into()),
            CoreError::NotAvailable(tr!("Make an identity first").into())
        );
        let unavailable =
            "Platform balance unavailable for identity abc; retry against an up-to-date node";
        assert!(matches!(
            platform_failure(unavailable.into()),
            CoreError::Failed(_)
        ));
    }
}

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
    /// The wallet's claims on contested names (FEAT-0021, [`name_requests::NameRequests`]).
    pub name_requests: PathBuf,
    /// What the wallet did on Platform ([`journal::Journal`]).
    pub journal: PathBuf,
    /// How far Platform was read back into the journal ([`platform_history::Marks`]).
    pub platform_sync: PathBuf,
    /// What Platform proved about the wallet's asset locks (FEAT-0016).
    pub lock_status: PathBuf,
    /// How far the tokens' history was read, and their last proved balances (FEAT-0019 T-1).
    pub token_history: PathBuf,
    /// Where each funding of the Platform address was signed to go (BUG-171).
    pub fund_intents: PathBuf,
    /// The wallet's own InstantLocks kept across a restart (BUG-173).
    pub kept_locks: PathBuf,
    /// Addresses of contacts' keys already paid ([`contact_addresses::ContactAddresses`]).
    pub contact_addresses: PathBuf,
    /// Senders whose contact requests were hidden ([`hidden_requests::HiddenRequests`]).
    pub hidden_requests: PathBuf,
    /// Contact requests owed after a DashPay pair was replaced ([`resend_debt::ResendDebt`],
    /// FEAT-0020-resend).
    pub contact_resend: PathBuf,
    /// Contacts' names as last read ([`contact_names::ContactNames`], SYNC-4).
    pub contact_names: PathBuf,
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
    /// The DashPay receiving accounts rescanned for from the birth ([`contact_scans::ContactScans`],
    /// BUG-150).
    pub contact_scans: PathBuf,
    /// Outputs of ours owed a rescan for their coin (BUG-232 review r2), the same rule.
    pub gap_scans: PathBuf,
    /// The last checked Core balance and its time ([`core_balance::CoreBalance`], BUG-77).
    pub core_balance: PathBuf,
    /// The shielded pool's note store and commitment tree ([`pool`]).
    pub pool: PathBuf,
    /// Apps signed in as an identity ([`connected_apps::ConnectedApps`]).
    pub connected_apps: PathBuf,
    /// The tokens the wallet follows and their proved contracts (FEAT-0019), `tokens.json`.
    pub tokens: PathBuf,
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
            name_requests: wallets.join("name-requests.json"),
            journal: wallets.join("platform-history.json"),
            platform_sync: wallets.join("platform-sync.json"),
            lock_status: wallets.join("lock-status.json"),
            token_history: wallets.join("token-history.json"),
            fund_intents: wallets.join("fund-intents.json"),
            kept_locks: wallets.join("instant-locks.json"),
            contact_addresses: wallets.join("contact-addresses.json"),
            hidden_requests: wallets.join("hidden-requests.json"),
            contact_resend: wallets.join("contact-resend.json"),
            contact_names: wallets.join("contact-names.json"),
            pool_history: wallets.join("pool-history.json"),
            resync: wallets.join("resync-on-start"),
            address_book: wallets.join("address-book.json"),
            mempool: wallets.join("mempool-seen.json"),
            mined: wallets.join("mined.json"),
            contact_scans: wallets.join("contact-rescans.json"),
            gap_scans: wallets.join("gap-rescans.json"),
            core_balance: wallets.join("core-balance.json"),
            pool: wallets.join("shielded-pool.db"),
            connected_apps: wallets.join("connected-apps.json"),
            tokens: wallets.join("tokens.json"),
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
    seed: Option<Arc<Seed>>,
}

/// The phrase's BIP-39 seed in memory locked to RAM (SEC-5, `evolite-memlock`): shared, not
/// copied, by the pool's moves.
type Seed = evolite_memlock::Locked<64>;

/// The seed's 64 bytes, for what takes a slice.
fn seed_bytes(seed: &Seed) -> &[u8] {
    seed.as_ref()
}

/// The phrase's BIP-39 seed (no passphrase), locked to RAM and zeroed when dropped (SEC-5).
/// `to_seed`'s own array is zeroed at once.
fn seed_of(phrase: &str) -> Result<Arc<Seed>, CoreError> {
    let mnemonic = Mnemonic::from_phrase(phrase).map_err(|_| CoreError::BadPhrase)?;
    let mut seed = Zeroizing::new(mnemonic.to_seed(""));
    let mut locked = Seed::new();
    locked.as_mut().copy_from_slice(&*seed);
    zeroize::Zeroize::zeroize(&mut *seed);
    Ok(Arc::new(locked))
}

/// The most bytes a phrase takes: 24 words of the longest word lists (Japanese, 3 bytes a
/// character) with their separators.
const PHRASE_BYTES: usize = 1024;

/// The open wallet's phrase until platform-wallet has it, locked to RAM (SEC-5): its bytes, then
/// zeros.
pub(in crate::real) struct LockedPhrase(evolite_memlock::Locked<PHRASE_BYTES>);

impl LockedPhrase {
    /// `None` when the phrase does not fit (no phrase is that long).
    fn new(phrase: &str) -> Option<Self> {
        if phrase.len() >= PHRASE_BYTES {
            return None;
        }
        let mut locked = evolite_memlock::Locked::new();
        locked.as_mut()[..phrase.len()].copy_from_slice(phrase.as_bytes());
        Some(Self(locked))
    }

    fn as_str(&self) -> &str {
        let bytes = self.0.as_ref();
        let len = bytes.iter().position(|b| *b == 0).unwrap_or(PHRASE_BYTES);
        std::str::from_utf8(&bytes[..len]).unwrap_or_default()
    }
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
    keyed_tasks: ops::keyed::KeyedTasks,
    /// TECH-busy-guard, BUG-210: the identity's flag, a key change not known to have landed
    /// and the fundings whose resume failed go with the chain: «Back» finds a task started
    /// before «Add a wallet» still holding the identity.
    identity_busy: busy::BusyFlag,
    unclear: sent::Unclear,
    fund_resume_failed: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

struct Real {
    paths: Paths,
    /// This core's wallet: its folder in `wallets/<network>/`.
    wallet: String,
    /// The wallets of this computer (the index, checked against the folders when read).
    index: wallets::Index,
    /// Their wrong passwords in a row (FEAT-0002), for every password of a wallet — a spend's
    /// too since FEAT-0004 A (R-31); only a spend that asks no password does not count.
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
    /// FEAT-0004 FR-7: this wallet's threshold of a large spend ([`security::load_large`]).
    large_spend: Option<Duffs>,
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
    /// Locked to RAM, out of the page file (SEC-5).
    attach: Option<LockedPhrase>,
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
    /// What the wallet catches up on Platform now (BUG-137): the snapshot's `SyncInfo::platform`.
    works: Arc<works::Works>,
    /// The highest tip seen and when it was first seen (BUG-155): `SyncInfo::tip_at`.
    tip_seen: std::sync::Mutex<(u32, Option<evolite_api::Timestamp>)>,
    /// SPV item 2a: the tip on this run's steady clock: when it grew, whether at all, since
    /// when synced.
    tip_grew: std::sync::Mutex<snapshot::TipClock>,
    /// SPV item 2a: «no new blocks» was said; the journal line comes once on entering it.
    stale_noted: AtomicBool,
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
    /// «Features for advanced users» (FEAT-0040…0046): off until the app says on, and off
    /// again at the lock.
    pub(in crate::real) advanced_features: bool,
    /// "Fiat rate": the price fetched last, and the task fetching it.
    fiat: Arc<std::sync::Mutex<Option<FiatRate>>>,
    fiat_task: Option<tokio::task::JoinHandle<()>>,
    /// Р1: what the last good update check learnt, the task checking, and whether a wallet was
    /// open in this run (the first request waits for it, review r1).
    update: Arc<std::sync::Mutex<Option<evolite_api::UpdateInfo>>>,
    update_task: Option<tokio::task::JoinHandle<()>>,
    wallet_opened: Arc<std::sync::atomic::AtomicBool>,
    /// «Fiat rate»'s currency while it is on: a payment's rate must be in it (FEAT-0008).
    fiat_currency: Option<String>,
    /// "Disk space": bytes of the data folder, and when they were counted.
    disk: Option<(tokio::time::Instant, u64)>,
    /// The Core and DAPI nodes, and when they were asked.
    nodes: Option<(tokio::time::Instant, Vec<NodeInfo>, Vec<NodeInfo>)>,
    /// When payments still out of a block were first seen, kept between runs.
    mempool: mempool::MempoolSeen,
    /// The block each Core transaction was seen in, for one the wallet took out of it (BUG-64).
    mined: mined::Mined,
    /// The contacts' receiving accounts whose rescan from the birth has run to its end (BUG-150).
    contact_scans: contact_scans::ContactScans,
    /// BUG-232 review r2: outputs of ours owed a rescan for their coin, once (twice when the
    /// first gives none), across starts.
    gap_scans: contact_scans::ContactScans,
    /// The wallet whose coins were written to the journal after the first filter pass (BUG-150).
    coins_after_pass: Option<WalletId>,
    /// The identity the last snapshot showed as «Act as», with its wallet (BUG-206 r1): what a
    /// command acts as, pinned when it comes. Not the raw choice: until a chosen identity has
    /// loaded the screen shows (and the person confirms) the lowest one.
    shown_now: Option<(WalletId, Identifier)>,
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
    /// The Workshop's reading (FEAT-0038 item 2): where each identity's sales stand.
    workshop: ops::workshop::WorkshopState,
    /// The tokens: what the searches found and the wallet's `tokens.json` (FEAT-0019).
    tokens: ops::tokens::TokenState,
    /// The sandboxed renderer; `None` in tests.
    artist: Option<crate::art::Artist>,
    /// Where each wallet's picture cache is (`<base>/<id>`): another wallet's deleted with it.
    art_base: Option<PathBuf>,
    /// DPNS names of contacts by identity (base58), asked for once each.
    names: Arc<std::sync::Mutex<HashMap<String, Option<String>>>>,
    /// Asked or being asked; one that failed leaves after [`NAME_RETRY`], to be asked again.
    names_asked: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// Contacts' names kept on this computer for the first paint (SYNC-4).
    contact_names: Arc<contact_names::ContactNames>,
    /// Names shown from that file and not read from the network yet in this run: asked all the
    /// same (the customer: «the information must be refreshed»).
    names_cached: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// At most [`NAMES_AT_ONCE`] names read at a time (SYNC-4).
    names_gate: Arc<tokio::sync::Semaphore>,
    /// The contacts of the last snapshot: who may be paid by name from Core.
    contacts: Vec<Contact>,
    /// The name suggestions' cache and the search under way (FEAT-0039).
    name_search: Arc<std::sync::Mutex<ops::name_search::NameSearch>>,
    /// An identity is being made or finished: its lock is not "unfinished" meanwhile.
    identity_busy: busy::BusyFlag,
    /// Paid writes whose outcome is not known yet (BUG-184, TECH-after-send-outcome): each
    /// line waits for a read that shows it.
    unclear: sent::Unclear,
    /// A payment from the Platform addresses is on its way: one at a time (FEAT-0015 R-2).
    address_busy: Arc<AtomicBool>,
    /// Each Platform address (bech32m) with its credits as the last snapshot showed them: the plan the
    /// last look showed is made from these (FEAT-0015).
    shown_address_inputs: Vec<(String, u64)>,
    /// The wallet whose shielded pool is connected (or being connected) in this run.
    pool_wallet: Arc<std::sync::Mutex<Option<WalletId>>>,
    /// The background tasks holding the keys or the seed, cut by the lock (SEC-6).
    keyed_tasks: ops::keyed::KeyedTasks,
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
    /// Woken after a name claim (FEAT-0021): the open requests' votes are read at once.
    names_stale: Arc<tokio::sync::Notify>,
    /// This network has the identity-verify contract, read with proof (FEAT-0021 AC-T13).
    identity_links: Arc<AtomicBool>,
    /// Every identity's credits were read from Platform in this run (BUG-115): what the store
    /// kept from before is checked.
    credits_checked: Arc<AtomicBool>,
    /// "Check the phrase" asked and not answered yet: the questions and their right words.
    phrase_check: Option<(Vec<PhraseQuestion>, crate::phrase_check::Answers)>,
    /// The wallet's history as the store keeps it (one record per transaction), and when it
    /// was read: how many records the wallet in memory had then.
    stored_history: Vec<key_wallet::managed_account::transaction_record::TransactionRecord>,
    /// The store's own scripts with what each is to the wallet (`chain::stored_own`), read with
    /// [`Self::stored_history`]: with the live pools, what is ours to the history (BUG-232).
    stored_own: std::collections::HashMap<
        Vec<u8>,
        key_wallet::managed_account::transaction_record::OutputRole,
    >,
    /// The asset locks the store keeps as consumed (BUG-57 r1), read with the history.
    consumed_locks: std::collections::HashSet<String>,
    /// The top-up addresses of the identities' slots, derived from the keys, for the locks a
    /// restore left without a top-up account (BUG-57 follow-up).
    top_ups: identity::TopUpScripts,
    history_read: Option<(usize, tokio::time::Instant)>,
    /// BUG-194: the wallet and its count of records when its address pools were last written to
    /// the store ([`evolite_chain::chain::Chain::save_pools`]): again whenever the count moves —
    /// a payment out, one in, a pass of the filters that found one — and on the lock.
    pools_saved: Option<(WalletId, usize)>,
    /// BUG-232: the wallet and its counts of records (live, stored) when its outputs not known
    /// as ours were last looked for past its pools' gap ([`Self::adopt_past_the_gap`]).
    gap_looked: Option<(WalletId, usize, usize)>,
    /// BUG-232 review M1: a rescan owed to the addresses taken in past the gap, from this
    /// height, asked once the filters are synced ([`take_past_the_gap`]).
    past_the_gap_from: Option<(WalletId, u32)>,
    /// TECH-spend-flow: the number of the `Command::Spend` being handled, for the answer of a
    /// job it starts ([`Self::answer_later`]).
    spend_ask: Option<u64>,
    /// BUG-232 review r1: outputs of ours with no coin yet whose rescan is owed now
    /// ([`convert::uncredited_outputs`]), each with its wallet (review r2 S1); noted in
    /// `gap-rescans.json` when that wallet's rescan is done ([`settle_gap_pending`]), forgotten
    /// with the lock (owed again at the next start).
    gap_pending: Vec<(WalletId, key_wallet::dashcore::OutPoint)>,
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
    lock_status: Arc<evolite_chain::lock_status::LockStatuses>,
    /// Where each funding of the Platform address was signed to go (BUG-171).
    fund_intents: Arc<evolite_chain::fund_intents::FundIntents>,
    /// The wallet's own InstantLocks kept across a restart (BUG-173).
    kept_locks: Arc<evolite_chain::kept_locks::KeptLocks>,
    /// The own transactions whose lock this unlock saw verified (TECH-finality-journal).
    own_locks: std::sync::Mutex<ops::kept_locks::OwnLocks>,
    lock_status_busy: Arc<AtomicBool>,
    /// The address fundings whose «Continue» failed in this run, by txid: they no longer hold
    /// a new funding back (BUG-160).
    fund_resume_failed: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// When the next pass is due; `None`: as soon as the identities are known.
    history_next: Option<std::time::Instant>,
    /// Addresses of contacts' keys already paid: never paid again, after a restart too.
    contact_paid: Arc<contact_addresses::ContactAddresses>,
    /// The wallet's requests for contested names: one for the wallet, so that its changes take
    /// turns (BUG-213).
    name_requests: Arc<name_requests::NameRequests>,
    /// Senders whose requests were hidden: still hidden after a resync.
    hidden: Arc<hidden_requests::HiddenRequests>,
    /// FEAT-0020-resend: the requests owed after a DashPay pair was replaced, and where the
    /// batches stand.
    resend_debt: Arc<resend_debt::ResendDebt>,
    resend_run: Arc<ops::resend::ResendRun>,
    /// The pool's operations with the amounts the scan loses after a restart.
    pool_history: pool_history::PoolHistory,
    /// The open wallet's payment notes, decrypted, with their key (FEAT-0003): opened with the
    /// wallet, `None` once it locks (a lock, a switch, "Add a wallet"). Shared with the sends
    /// in flight, which keep their note once the network takes them.
    notes: Arc<std::sync::Mutex<Option<notes::Notes>>>,
    /// Which history lines this unlock has seen, for the rate of operations first seen
    /// (FEAT-0008, R-4); reset when the notes open.
    first_seen: std::sync::Mutex<FirstSeen>,
    /// Core addresses the person named, to pay again.
    address_book: address_book::AddressBook,
    /// The address shown in the last look before paying each contact (by identity): a payment
    /// to a contact is signed only to it.
    previews: Arc<std::sync::Mutex<HashMap<String, String>>>,
    /// Apps signed in as an identity.
    apps: Arc<connected_apps::ConnectedApps>,
    /// This phrase's public keys of each identity by its revision (FEAT-0020 АРХ-1): whose a key is.
    this_keys: ops::identity_keys::ThisKeys,
    /// App requests shown and not answered yet, by request id.
    app_requests: Arc<std::sync::Mutex<HashMap<u64, AppJob>>>,
    next_app_request: Arc<std::sync::atomic::AtomicU64>,
    /// What Platform last said of App Connect v2 session keys (`ReadAppBudgets`).
    app_reads: Arc<std::sync::Mutex<ops::app_connect2::Reads>>,
}

/// What an app's request asks, as checked when its link was opened: App Connect v2 only
/// (FEAT-0023; v1 is not supported).
enum AppJob {
    Connect2(Box<ops::app_connect2::Connect2Job>),
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
    /// The contacts' receiving accounts it is for (BUG-150, `contact-rescans.json`).
    contacts: Vec<String>,
    /// When the one for contacts was asked and how many DashPay coins the wallet had then
    /// (BUG-150; BUG-221: the coins, not history lines — a found coin of a known transaction
    /// adds no line).
    contacts_asked: Option<(std::time::Instant, usize)>,
    /// For addresses taken in past the gap (BUG-232): once done, the coins it gave go to the
    /// store (review r1 M1).
    past_the_gap: bool,
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
                contacts: Vec::new(),
                contacts_asked: None,
                past_the_gap: false,
            }),
        };
        o.up_to = o.up_to.max(synced);
        o.stale.extend(stale);
        o.lost.extend(lost);
    }

    /// One more rescan asked for `wallet` at `synced`, for the contacts' receiving `accounts`
    /// (BUG-150).
    fn add_contacts(
        owed: &mut Option<Self>,
        wallet: WalletId,
        synced: u32,
        accounts: Vec<String>,
        coins: usize,
    ) {
        Self::add(owed, wallet, synced, Vec::new(), Vec::new());
        if let Some(o) = owed.as_mut() {
            o.contacts.extend(accounts);
            o.contacts_asked
                .get_or_insert((std::time::Instant::now(), coins));
        }
    }

    /// The rescan owed for `wallet` is for addresses taken in past the gap too (BUG-232).
    fn for_past_the_gap(owed: &mut Option<Self>, wallet: &WalletId) {
        if let Some(o) = owed.as_mut().filter(|o| o.wallet == *wallet) {
            o.past_the_gap = true;
        }
    }

    /// The contacts' receiving accounts a rescan is under way for.
    fn contacts_under_way<'a>(owed: &'a Option<Self>, wallet: &WalletId) -> &'a [String] {
        owed.as_ref()
            .filter(|o| o.wallet == *wallet)
            .map_or(&[][..], |o| o.contacts.as_slice())
    }

    /// The owed rescan, taken, when `wallet`'s filters are at `synced` and that is where it
    /// was asked to end ([`rescan_end`]) or above.
    fn finished(owed: &mut Option<Self>, wallet: &WalletId, synced: u32) -> Option<Self> {
        owed.take_if(|o| o.wallet == *wallet && synced >= o.up_to)
    }
}

/// BUG-221 (live, qa-f27): where a rescan asked now ends — the headers' tip, or the wallet's
/// own height if that is higher. Not the wallet's height alone: adding a contact's account
/// rewinds it to the birth by itself (rust-dashcore#649), so at the ask it may already be far
/// below the tip, and «back where it was» was then true on the next snapshot, with the scan
/// still ahead (`rescan_done` with `secs` 0 and `found` 0, the balance said final).
fn rescan_end(synced: u32, status: &sync::SyncStatus) -> u32 {
    status
        .headers
        .as_ref()
        .map_or(synced, |h| synced.max(h.height))
}

/// BUG-232 review M1: the rescan owed to addresses taken in past the gap, noted (the lower
/// height if one is noted already) until the filters are synced: then taken, once. One noted
/// for another wallet gives way to this one's.
fn note_past_the_gap(pending: &mut Option<(WalletId, u32)>, wallet: WalletId, from: u32) {
    *pending = Some(match *pending {
        Some((w, f)) if w == wallet => (wallet, f.min(from)),
        _ => (wallet, from),
    });
}

/// BUG-232 review r2 S1: `wallet`'s outputs owed a rescan for their coin, taken out of `pending`
/// once its rescan is done and split into those still with no coin (in `uncredited`) and those
/// that got one; another wallet's stay where they are.
fn settle_gap_pending(
    pending: &mut Vec<(WalletId, key_wallet::dashcore::OutPoint)>,
    wallet: &WalletId,
    uncredited: &[key_wallet::dashcore::OutPoint],
) -> (
    Vec<key_wallet::dashcore::OutPoint>,
    Vec<key_wallet::dashcore::OutPoint>,
) {
    let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(pending)
        .into_iter()
        .partition(|(w, _)| w == wallet);
    *pending = others;
    mine.into_iter()
        .map(|(_, o)| o)
        .partition(|o| uncredited.contains(o))
}

/// The height [`note_past_the_gap`] noted for `wallet`, taken when the filters are synced.
fn take_past_the_gap(
    pending: &mut Option<(WalletId, u32)>,
    wallet: &WalletId,
    synced: bool,
) -> Option<u32> {
    if !synced {
        return None;
    }
    pending.take_if(|(w, _)| w == wallet).map(|(_, from)| from)
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

impl Real {
    /// A background read of Platform starts (BUG-137): the sync card's mark moves until the
    /// guard is dropped.
    pub(crate) fn work(&self, kind: PlatformWork) -> works::Working {
        self.works.start(kind)
    }
}
