//! What the evolite UI and the wallet core say to each other.
//!
//! The UI sends [`Command`]s and receives [`Event`]s; the main one is a fresh [`Snapshot`] —
//! everything the screens show. Amounts are duffs, times are Unix seconds; the UI formats
//! both. There is no type for a private key here: keys never leave the core. The only secrets
//! that cross the line are what the user types or must see — the phrase and the password —
//! and their types wipe themselves and never print.

#![forbid(unsafe_code)]

use std::fmt;
use std::sync::Arc;
use zeroize::Zeroizing;

pub mod address_fund;
pub mod address_plan;
pub mod coin_plan;
pub mod fiat;
/// Platform tokens (FEAT-0019).
pub mod tokens;

/// 1 DASH = 100 000 000 duffs.
pub type Duffs = i64;
/// Unix time, seconds.
pub type Timestamp = i64;

/// The most characters (extended grapheme clusters) a payment note holds (FEAT-0003).
pub const NOTE_MAX_GRAPHEMES: usize = 140;
/// And the most bytes of UTF-8.
pub const NOTE_MAX_BYTES: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Network {
    Testnet,
    Mainnet,
}

/// The balances an operation changes ([`Activity::layers`], a mask of these): History's layer
/// pills keep an operation in every layer whose money it moves (FEAT-0009 R-1).
pub mod layer {
    /// Core DASH.
    pub const CORE: u8 = 1;
    /// An identity's credits, and what is done as the identity (names, contacts, profile).
    pub const IDENTITY: u8 = 2;
    /// The Platform address's credits.
    pub const PLATFORM_ADDRESS: u8 = 4;
    /// The shielded pool.
    pub const POOL: u8 = 8;
    /// The CoinJoin account's coins, «Mixed» (FEAT-0030 AC-A7).
    pub const MIXED: u8 = 16;
}

/// Which way an operation moved money, for the wallet as a whole (FEAT-0009 R-2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flow {
    In,
    Out,
    /// Between the wallet's own balances: only the fee leaves (a transfer to yourself, a top-up
    /// of your identity, a move into the pool from your own, a withdrawal to your own address,
    /// credits to your own identity).
    Between,
}

/// How far a payment is proved: the UI shows it as a status chip.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Proof {
    /// Seen in the mempool, not locked yet.
    Mempool,
    /// Core: InstantLock with a checked BLS signature.
    InstantLock,
    /// Core: in a block, no ChainLock yet.
    InBlock,
    /// Core: in a block with a checked ChainLock.
    ChainLock,
    /// Platform: the state transition is in a proof checked against the quorum keys.
    Proved,
    /// Core: signed by this wallet and handed to the network, not seen back yet (or the network
    /// gave no clear answer). Never resent.
    Sending,
    /// Core: out of a block for many blocks after it was seen — the network may have
    /// dropped or replaced it. Never resent.
    Unconfirmed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chain {
    Core,
    Platform,
    /// An asset lock: DASH leaves Core and becomes credits on Platform.
    CoreToPlatform,
}

#[derive(Clone, PartialEq, Debug)]
pub enum ActivityKind {
    /// `from` is a DPNS name or an exchange label when known.
    Received {
        from: Option<String>,
    },
    /// To a name or a (shortened by the UI) address; empty when the core does not know it.
    Sent {
        to: String,
    },
    /// Between the wallet's own addresses (or CoinJoin): only the fee leaves. `moved`: what
    /// came to the receiving address in [`Activity::address`] ("Receive" lists it, BUG-33); 0
    /// when not known.
    SelfTransfer {
        moved: Duffs,
    },
    /// A mixing round of the CoinJoin account, made in DashPay or Dash Core (FEAT-0030 A3): the
    /// wallet's denominations in, as many out. `fee`: what it lost (0, or a collateral).
    Mixing {
        fee: Duffs,
    },
    /// The wallet's coins split into denominations on its CoinJoin account (the preparation for
    /// mixing, made in DashPay or Dash Core): a move between its own coins (FEAT-0030 A3).
    IntoMixing,
    /// Platform address received DASH.
    PlatformReceived,
    /// An asset lock for an identity's credits (or for the Platform address when `identity`
    /// is empty and `creates` false): `identity` is the one it registered or tops up, as
    /// `@name` or its id, empty when not known.
    TopUp {
        identity: String,
        /// The lock registered the identity: its creation, not a top-up (BUG-57).
        creates: bool,
        /// Whether Platform turned the locked DASH into credits (BUG-57). The line's `proof`
        /// is only the Core transaction's.
        credits: Credits,
    },
    AppSignIn {
        app: String,
    },
    /// App Connect v2 signed in again on the same connection: its keys replaced (BUG-145).
    AppRenewed {
        app: String,
    },
    /// App Connect v2: the app's session keys disabled (BUG-145).
    AppDisconnected {
        app: String,
    },
    /// A token sent (BUG-145): which and how much, in words, and to whom. The line's amount is the
    /// fee in credits; the token is not DASH.
    TokenSent {
        token: String,
        to: String,
        amount: String,
    },
    /// What an app spent from its keys' budgets, as Platform says (BUG-145): one line an app a
    /// day, timed when it was read.
    AppSpent {
        app: String,
    },
    CreditTransfer {
        to: String,
    },
    Shielded {
        from: Option<String>,
    },
    PoolReceived,
    PoolSent,
    /// Credits withdrawn to a Core address (paid out by Platform later). `status`: how far its
    /// document on Platform got, once read from there (BUG-58); `None` before.
    Withdrawal {
        to: String,
        status: Option<WithdrawalState>,
    },
    /// Our DashPay contact request (to a name or a short id). `accepting`: it answered theirs,
    /// which came first ("You accepted the request from …", BUG-58). `ours`: the `@name` of
    /// the identity it went from when the wallet has more than one (BUG-69).
    ContactRequest {
        to: String,
        accepting: bool,
        ours: Option<String>,
    },
    /// A DashPay contact request to us, one line per sender and identity of ours (BUG-58):
    /// `from` as `ContactRequest::to`; `ours`: the `@name` it came to when the wallet has
    /// more than one identity. Costs nothing: amount 0, no fee.
    ContactRequestIn {
        from: String,
        ours: Option<String>,
        state: RequestState,
    },
    /// The identity's DashPay profile written.
    ProfileSaved,
    /// A DPNS name registered for the identity (paid in credits).
    NameRegistered {
        label: String,
    },
    /// Sent from the wallet's Platform addresses to a Platform address (FEAT-0015 R-11).
    AddressSent {
        to: String,
    },
    /// Withdrawn from the wallet's Platform addresses to a Core address of this wallet; paid out
    /// by Platform later. `status`: its document's, once read; `None` until then (FEAT-0015 R-11).
    AddressWithdrawal {
        to: String,
        status: Option<WithdrawalState>,
    },
    /// An asset lock on Core that funds the wallet's own Platform address (FEAT-0015 stage 2):
    /// «Transfer from Core to the Platform address». The amount is what left Core, the lock and
    /// its fee; a move between the wallet's own, so the arrival on the address is not a line of
    /// its own (AC-T7).
    AddressFunded,
}

/// Whether the DASH of an asset lock became credits on Platform, as far as the wallet knows
/// (BUG-57).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Credits {
    /// Not known: a lock found by a restore, one being sent now, the Platform address's.
    Unknown,
    /// Platform took the lock: the credits are on the identity.
    Added,
    /// Locked on Core and not taken by Platform: [`Wallet::unfinished_top_up`] or
    /// [`Wallet::unfinished_identity`] says so too, and "Continue" on Identity finishes it.
    NotAdded,
    /// Platform proved it spent; who got the credits it does not record (FEAT-0016).
    Used,
    /// Platform proved only part of it spent; `left` duffs are still in the lock.
    UsedPartly { left: Duffs },
    /// Platform proved it has not seen the lock, and no top-up here is left to continue.
    NotReceived,
}

/// A withdrawal document's `status` (the Withdrawals system contract).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WithdrawalState {
    Queued,
    Pooled,
    Broadcasted,
    /// Paid out on Core.
    Complete,
    Expired,
    /// Terminal: the payout can never be mined by Core (the Withdrawals contract, v2).
    Failed,
}

/// Where a contact request to us stands, as DashPay says it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RequestState {
    /// Not answered.
    Waiting,
    /// We answered it with ours.
    Accepted,
    /// It answered ours, sent before: we are contacts.
    TheyAccepted,
    /// Hidden by us (`HideRequest`).
    Hidden,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Activity {
    pub id: u64,
    pub kind: ActivityKind,
    pub chain: Chain,
    /// Unix seconds; 0 when the operation has no known time (a note found in the pool by its
    /// scan): shown as "time unknown", never as 1970, and listed after the dated ones in the
    /// core's order (BUG-47, BUG-58).
    pub at: Timestamp,
    /// Positive comes in, negative goes out (fees included).
    pub amount: Duffs,
    pub proof: Proof,
    /// For the operation's card: the Core transaction, its fee when this wallet paid it, and
    /// the block it is in; `None` where there is none or it is not known.
    pub txid: Option<String>,
    pub fee: Option<Duffs>,
    pub block: Option<u32>,
    /// The Core address of a payment, when the wallet knows it. Out: the payee's, shown beside
    /// their @name and naming a contact's payments. In: our own address it came to, which
    /// "Receive" lists with what each received.
    pub address: Option<String>,
    /// The balances it changes: a mask of [`layer`]'s constants, set by the core (FEAT-0009).
    pub layers: u8,
    /// Which way it moved money for the whole wallet, set by the core (FEAT-0009).
    pub flow: Flow,
    /// The operation's key for its note (FEAT-0003): `core:<txid>`, `platform:<line>`,
    /// `pool:<id>`, the same after a restart and "Sync again"; empty where no note can be
    /// kept (a send not in the history yet).
    pub key: String,
    /// What the person wrote to it, only on this computer ([`Command::SetNote`]).
    pub note: Option<String>,
    /// The rate at the time of the operation (FEAT-0008, R-4): kept beside its note, only on
    /// this computer; `None` for operations from before the rate was on, or with it off.
    pub rate: Option<FiatRate>,
    /// What the operation's card says beyond the row (FEAT-0005); `None` where the core knows
    /// nothing more. Boxed: the snapshot clones every row each frame.
    pub detail: Option<Box<Detail>>,
}

/// An operation's card in full (FEAT-0005): what proved it, which of the wallet's own addresses
/// the coins came from and went to, and whether part of it was not kept. Built by the core from
/// what it has; the card draws no line for what is empty.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Detail {
    /// What proved it, said in words: a verified InstantLock or a ChainLock (BUG-26).
    pub confirm: Option<Confirm>,
    /// The wallet's own addresses its coins came from, one per input (a payment out counts them).
    pub own_from: Vec<String>,
    /// The wallet's own addresses that got coins and how much each, change not included (the
    /// customer, designer-16): a payment in, a transfer between own addresses.
    pub own_to: Vec<(String, Duffs)>,
    /// The identity of this wallet it was done as (a Platform operation, the identity a payout
    /// was withdrawn from), or the one an asset lock is for.
    pub identity: Option<IdentityRef>,
    /// The Platform document it wrote, base58 (AC-A5): to copy and to look up outside evolite.
    /// `None` where the operation writes none or its id was not kept.
    pub document: Option<String>,
    /// [`Activity::key`] of the operation paired with it: a withdrawal and its payout on Core,
    /// each pointing at the other.
    pub link: Option<String>,
    /// A withdrawal Platform finished and whose payout this wallet has seen on Core.
    pub payout: Option<Payout>,
    /// What Platform added to the identity for this asset lock, when it was topped up from this
    /// computer; `None` for an older, restored or foreign one: the card then says only whether.
    pub credits_added: Option<Duffs>,
    /// Something the card needs was not kept or could not be worked out (FR-6): it says so in
    /// one line instead of drawing empty ones.
    pub incomplete: bool,
}

/// An identity of this wallet, as a card names it.
#[derive(Clone, PartialEq, Debug)]
pub struct IdentityRef {
    /// Base58.
    pub id: String,
    /// Its main DPNS name, without the `@`.
    pub name: Option<String>,
}

/// A withdrawal's payout seen on Core.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Payout {
    /// The Core block it is in; `None` while it is out of one.
    pub block: Option<u32>,
}

/// What proved a Core operation, as its card says it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confirm {
    /// A quorum's InstantLock, its signature verified.
    InstantLock,
    /// A ChainLock on the block it is in (or on one over it).
    ChainLock { block: u32 },
}

#[derive(Clone, PartialEq, Debug)]
pub struct Balances {
    /// The Core balance, each coin once (BUG-77): checked in this sync, or the last checked one
    /// while the wallet does not know its coins (see `core_as_of`); 0 when none was checked yet.
    pub core: Duffs,
    /// How sure `core` is (BUG-77, the designer's improvement 1): `None` — checked in this sync
    /// (the filters are at the tip); otherwise the last checked amount and its time, or none
    /// checked yet (a new or restored wallet). «Total» is as sure as `core`. Payments from Core
    /// wait until it is `None`.
    pub core_as_of: Option<CoreAsOf>,
    pub core_addresses: u32,
    /// The credits of the identity the person works as: what Send, a top-up or a withdrawal
    /// spend.
    pub credits: Duffs,
    /// The credits of every identity of the wallet, the one worked as among them (BUG-56): what
    /// the wallet holds on Platform in identities.
    pub all_credits: Duffs,
    pub platform_address: Duffs,
    pub shielded: Duffs,
    /// `all_credits` and `credits` are known: an identity is known here, or the search for them
    /// ended with none. `false`: «—», not a 0 (BUG-91).
    pub credits_read: bool,
    /// The credits were read from Platform in this run, every identity's, and the search for
    /// the identities ended (BUG-115). `credits_read` without it: what the store kept from an
    /// earlier run, not checked yet.
    pub credits_checked: bool,
    /// `platform_address` is known: Platform answered this run, or the store holds an earlier
    /// read that found credits there. `false`: «—», not a 0 (BUG-91).
    pub platform_address_read: bool,
    /// The coins of the CoinJoin account, mixed elsewhere (DashPay, Dash Core), apart from
    /// `core` (FEAT-0030 phase A): `None` when the account holds nothing.
    pub mixed: Option<Mixed>,
}

/// What a payment from «Mixed» costs (FEAT-0030 phase A, АРХ-9а): the recipient gets the amount
/// exactly; the wallet gives `fee` + `excess` on top.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MixedQuote {
    pub amount: Duffs,
    /// The usual fee of the transaction.
    pub fee: Duffs,
    /// What the chosen coins hold over the amount and the fee: mixed coins give no change, so
    /// it goes to the miners with the fee.
    pub excess: Duffs,
    pub rule: MixedExcess,
    /// The coins spent.
    pub coins: u32,
    /// With [`MixedExcess::Refuse`]: an amount the same coins pay with nothing burnt (their
    /// value less the fee).
    pub suggested: Option<Duffs>,
}

/// АРХ-9а's thresholds, as the screens and the demo use them: an excess up to this (0.001 DASH)
/// is paid with the fee without asking. `evolite_coinjoin` holds the core's own; a test of
/// `evolite-core` keeps the two the same.
pub const MIXED_EXCESS_FREE: Duffs = 100_000;
/// ... and up to this share of the amount, in percent.
pub const MIXED_EXCESS_FREE_PERCENT: Duffs = 1;
/// The most a payment from «Mixed» burns, confirmed (0.01 DASH).
pub const MIXED_EXCESS_MAX: Duffs = 1_000_000;

/// What the core does with the excess of a payment from «Mixed» (АРХ-9а).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MixedExcess {
    /// Small enough (≤ 0.001 DASH and ≤ 1 % of the amount): paid with the fee.
    Free,
    /// Larger, up to 0.01 DASH: sent only with this excess confirmed ([`Spend::SendMixed`]'s
    /// `accept_excess`).
    Confirm,
    /// Over 0.01 DASH: refused; [`MixedQuote::suggested`] says an amount without excess.
    Refuse,
}

/// The CoinJoin account's coins (FEAT-0030 AC-T3), in duffs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mixed {
    /// Fully mixed denominations, final: what «Mixed» spends.
    pub spendable: Duffs,
    /// The rest of the account: fewer rounds, collateral, not denominations, not final yet.
    pub unfinished: Duffs,
    /// The rounds that make a coin fully mixed.
    pub rounds_needed: u8,
    /// What «Max» sends from «Mixed» (AC-A4): `spendable` less the usual fee of spending every one
    /// of those coins, with no change. 0 with none.
    pub max: Duffs,
}

impl Balances {
    /// Everything the wallet holds: Core, the CoinJoin account (FEAT-0030: apart from Core, but
    /// the wallet's all the same), every identity's credits, the Platform address and the pool.
    /// Choosing another identity does not change it.
    pub fn total(&self) -> Duffs {
        self.core
            + self.mixed.map_or(0, |m| m.spendable + m.unfinished)
            + self.all_credits
            + self.platform_address
            + self.shielded
    }

    /// Whether every part of [`Self::total`] other than Core is known (BUG-91): the credits checked
    /// in this run, not only kept from an earlier one (BUG-115).
    pub fn platform_known(&self) -> bool {
        self.credits_read && self.credits_checked && self.platform_address_read
    }
}

/// A Core balance not checked in this sync ([`Balances::core_as_of`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoreAsOf {
    /// The last checked balance, checked at this time (kept in the wallet's folder).
    Checked(Timestamp),
    /// None checked yet: the screens say «not known yet», not an amount.
    NotYet,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyncStage {
    /// Looking for peers.
    Connecting,
    Headers,
    /// The masternode lists: dash-spv asked a node for them (QRInfo) and waits for the answer.
    Masternodes,
    /// The answer is in: dash-spv checks the quorums it brought (their BLS signatures, and the
    /// lists of the blocks they were made at, fetched one by one). The last part of the lists.
    Quorums,
    Filters,
    Transactions,
    Platform,
    ChainLock,
    Done,
}

#[derive(Clone, PartialEq, Debug)]
pub struct SyncInfo {
    pub stage: SyncStage,
    /// 0..=1 within the stage, counted from where this run began it. `None`: the stage has
    /// nothing to count (looking for nodes, waiting for a node's answer, checking quorums).
    pub progress: Option<f32>,
    pub tip_height: u32,
    /// Block headers checked so far.
    pub headers_height: u32,
    pub filters_height: u32,
    pub core_peers: u32,
    pub platform_nodes: u32,
    /// Rough time left, minutes, when the core can tell.
    pub minutes_left: Option<u32>,
    /// The tip has a checked ChainLock.
    pub chainlocked: bool,
    /// Why sync stopped, when it did.
    pub error: Option<String>,
    /// The wallet's database refused a folder: what the person has to do, in full. A resync
    /// would not help; sync starts by itself once the folder is changed.
    pub folder_refused: Option<String>,
    /// Bytes the wallet's data folder takes on disk (headers, filters, masternode lists, the
    /// store); `None` until counted.
    pub disk_bytes: Option<u64>,
    /// The only Core node this run connects to (Settings → "Own node"), as it was resolved
    /// at start; `None`: nodes found through DNS seeds.
    pub core_node: Option<String>,
    /// The node saved for the next start, when it differs from this run's; empty: DNS seeds.
    pub core_node_next: Option<String>,
    /// The Core nodes connected now, each by what the masternode list says it is.
    pub core_nodes: Vec<NodeInfo>,
    /// The Platform (DAPI) nodes the SDK knows, and which of them it holds off for now.
    pub dapi_nodes: Vec<NodeInfo>,
    /// What the wallet catches up on Platform now, in the order of [`PlatformWork`], each once;
    /// empty: nothing (BUG-137). Only the core's background reads, never a person's operation.
    pub platform: Vec<PlatformWork>,
}

/// A background read of Platform the wallet catches up with (BUG-137): the sync card's mark
/// moves while one goes. In the order the card names them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum PlatformWork {
    Identities,
    Names,
    Credits,
    Contacts,
    Tokens,
    Pool,
}

impl PlatformWork {
    /// Every kind, in order.
    pub const ALL: [PlatformWork; 6] = [
        PlatformWork::Identities,
        PlatformWork::Names,
        PlatformWork::Credits,
        PlatformWork::Contacts,
        PlatformWork::Tokens,
        PlatformWork::Pool,
    ];
}

/// One node of the network, as Settings → Network → "Nodes" lists it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NodeInfo {
    /// `ip:port` for Core, the URL for DAPI.
    pub address: String,
    pub role: NodeRole,
    /// DAPI: held off after a bad or missing answer, until this time (unix seconds).
    pub banned_until: Option<Timestamp>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeRole {
    /// The masternode list is not in yet.
    Unknown,
    /// Not in the masternode list.
    Regular,
    Masternode,
    /// A high-performance masternode; Platform runs on these.
    Evonode,
}

/// Whether there is a wallet on this computer and whether its keys are open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VaultState {
    NoWallet,
    Locked,
    Unlocked,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyLevel {
    Master,
    Critical,
    High,
    Medium,
}

#[derive(Clone, PartialEq, Debug)]
pub enum KeyPurpose {
    Identity,
    Documents,
    Transfers,
    /// Encrypting and reading DashPay contact requests.
    Encryption,
    /// Added for an app through App Connect.
    App(String),
}

/// A key's signature scheme, as Platform names its key types.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum KeyKind {
    #[default]
    Ecdsa,
    EcdsaHash160,
    Bls,
    EddsaHash160,
    Bip13Script,
}

#[derive(Clone, PartialEq, Debug)]
pub struct IdentityKey {
    pub id: u32,
    pub level: KeyLevel,
    pub purpose: KeyPurpose,
    pub kind: KeyKind,
    /// With [`KeyPurpose::Encryption`]: the key that reads DashPay requests (DECRYPTION), not
    /// the one they are encrypted to.
    pub decrypts: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct DpnsName {
    pub label: String,
    pub primary: bool,
    /// Contested name still in the masternode vote.
    pub voting_days_left: Option<u32>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Identity {
    pub id: String,
    pub created_at: Timestamp,
    pub created_block: u32,
    pub revision: u32,
    /// Where the keys come from, e.g. DIP-13 identity 0.
    pub key_source: String,
    pub names: Vec<DpnsName>,
    pub keys: Vec<IdentityKey>,
    /// Its DashPay profile, once written.
    pub profile: Option<Profile>,
    /// Its slot in the phrase (DIP-13 identity index); DashPay on Android shows only slot 0.
    pub slot: u32,
}

/// What an identity says about itself in DashPay. Only its @name is proved (DPNS); these are
/// its own words.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Profile {
    /// Up to 25 characters.
    pub display: Option<String>,
    /// "Bio", up to 140 characters.
    pub about: Option<String>,
    /// The picture to draw ([`Command::DrawArt`]), once the core has it and it matches the
    /// profile's hash; `None`: the initial on a colour instead.
    pub avatar: Option<String>,
    /// The link the profile gives (https), shown when editing.
    pub avatar_url: Option<String>,
    pub updated_at: Option<Timestamp>,
}

/// What a cut-short "Create an identity" left.
#[derive(Clone, PartialEq, Debug)]
pub struct UnfinishedIdentity {
    /// Duffs locked on Core for it; `None`: the identity is made, only its name is missing.
    pub locked: Option<Duffs>,
    /// The name it was to get, when this computer remembers it.
    pub name: Option<String>,
    /// The identity's id when it is made and only its name is missing: what its credits are
    /// topped up on when the name ran out of them (BUG-93 review, should 2).
    pub identity: Option<String>,
}

/// A top-up of the identity that Platform has not finished in the wallet's eyes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UnfinishedTopUp {
    /// Duffs locked on Core for it.
    pub locked: Duffs,
    /// Platform already said it took the lock (most likely an earlier attempt went through);
    /// the wallet waits for the ChainLock of the lock's block to confirm it. `ResumeTopUp`
    /// then only checks again, and a new top-up is not held back.
    pub awaiting_chain_lock: bool,
}

/// A funding of the wallet's Platform address from Core that Platform has not taken yet: the
/// DASH are locked on Core for it (FEAT-0015 stage 2, R-8).
#[derive(Clone, PartialEq, Debug)]
pub struct UnfinishedAddressTopUp {
    /// Duffs locked on Core for it.
    pub locked: Duffs,
    /// The Platform address (`tdash1…`) the lock is for.
    pub address: String,
}

/// One word of "Check the phrase": its number in the phrase (from 1) and the words to pick from,
/// the right one among them.
#[derive(Clone, PartialEq, Debug)]
pub struct PhraseQuestion {
    pub number: usize,
    pub choices: Vec<String>,
}

/// One of the wallet's identities, to choose which one the wallet acts as.
#[derive(Clone, PartialEq, Debug)]
pub struct IdentityChoice {
    /// Base58.
    pub id: String,
    /// Its main DPNS name.
    pub name: Option<String>,
    /// Its slot in the phrase (DIP-13 identity index).
    pub slot: u32,
}

/// Where the search for the phrase's identities on Platform stands.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum IdentitySearch {
    /// Waiting for the masternode lists, or asking Platform now.
    #[default]
    Searching,
    /// Done: what was found is in the snapshot (maybe nothing).
    Done,
    /// The last try failed (the text says why); the core tries again by itself, and
    /// `FindIdentities` at once.
    Failed(String),
}

/// Where a DashPay contact stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContactState {
    /// Both sent a request: payments go both ways.
    Established,
    /// We asked, no answer yet.
    Outgoing,
    /// They asked, we have not answered.
    Incoming,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Contact {
    pub identity_id: String,
    /// Its DPNS name, without "@", when it has one.
    pub name: Option<String>,
    pub profile: Profile,
    pub state: ContactState,
    /// When the first request between us was made.
    pub since: Timestamp,
    pub payments: u32,
    pub last_payment: Option<Timestamp>,
}

impl Contact {
    /// "@alice", or a short id for an identity without a name.
    pub fn handle(&self) -> String {
        match &self.name {
            Some(n) => format!("@{n}"),
            None => {
                let id = &self.identity_id;
                let n = id.chars().count();
                if n > 12 {
                    let head: String = id.chars().take(6).collect();
                    let tail: String = id.chars().skip(n - 4).collect();
                    format!("{head}…{tail}")
                } else {
                    id.clone()
                }
            }
        }
    }
}

impl Identity {
    /// The primary verified name, without "@".
    pub fn name(&self) -> Option<&str> {
        self.names
            .iter()
            .find(|n| n.primary && n.voting_days_left.is_none())
            .map(|n| n.label.as_str())
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Pool {
    pub notes: u32,
    /// When the wallet last read the pool on Platform; `None`: not read yet.
    pub checked_at: Option<Timestamp>,
    /// The wallet's private address (`tdash1…`, Orchard) to receive into the pool; `None`
    /// while the pool is not connected (a wallet opened without its phrase, or no pool in this
    /// build).
    pub address: Option<String>,
    /// Operations only the owner sees.
    pub operations: Vec<Activity>,
    /// Core funds locked for the pool that Platform has not taken yet (a shield from Core cut
    /// short): `Shield { from: Core }` finishes it with that lock, never a new one.
    pub unfinished_shield: Option<Duffs>,
    /// The pool is being connected (keys bound, notes read): minutes the first time.
    pub connecting: bool,
}

/// An app signed in by App Connect v2 (FEAT-0023), connected or disconnected.
#[derive(Clone, PartialEq, Debug)]
pub struct ConnectedApp {
    /// The app's contract, base58.
    pub id: String,
    /// As the sign-in showed it: the contract owner's `@name`, else the app's own words.
    pub name: String,
    /// What the app calls itself: its own words, not checked (R-8).
    pub label: String,
    pub domain: String,
    pub last_used: Timestamp,
    /// The session keys of the sign-in, each with its limits.
    pub session: Vec<SessionKey>,
    /// When it was disconnected: its keys are disabled, and only the record on this computer
    /// is left ([`Command::ForgetApp`] removes it).
    pub disconnected_at: Option<Timestamp>,
}

/// What an App Connect v2 key is bound to (Platform's contract bounds), ids in base58.
#[derive(Clone, PartialEq, Debug)]
pub enum AppBinding {
    Contract {
        id: String,
    },
    DocumentType {
        contract: String,
        name: String,
    },
    /// A group of contracts its owner may add to later.
    Group {
        id: String,
    },
    /// DashPay's `profile`, the one document type of another owner an app may ask for.
    DashPayProfile,
}

/// The connection a sign-in renews (the same `CONNECTION_ID`): its primary key as it stands.
#[derive(Clone, PartialEq, Debug)]
pub struct Renewal {
    pub budget: Duffs,
    /// What Platform last said was spent; `None` until read.
    pub spent: Option<Duffs>,
    pub until: Timestamp,
}

/// Why an app's link is refused, for the person to read (FEAT-0023, E1–E3). Nothing was spent.
#[derive(Clone, PartialEq, Debug)]
pub enum AppLinkRefusal {
    /// Another version than App Connect v2: `v` is what the link says (`"1"` for v1).
    Version { v: String },
    /// Older than 10 minutes, or too far ahead.
    Stale,
    /// For the main network, where App Connect v2 is not on yet.
    Mainnet,
    /// For another network: its code (`t`, `d`, …).
    OtherNetwork { n: String },
    /// The app's contract is one of Platform's own.
    SystemContract,
    /// The app's contract is not on the network.
    NoContract,
    /// A contract or group of another owner, or another app's documents.
    OthersContract,
    /// An encryption key for a contract that keeps none.
    NoEncryptionKeys,
    /// A field evolite does not know.
    UnknownField,
    /// The network's protocol is older than 14: keys with a budget and an end are not taken.
    OldNetwork,
    /// Not well formed.
    Malformed,
}

/// Where a session key stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionKeyState {
    Live,
    /// Its end passed: Platform refuses what it signs, but it is not disabled.
    Expired,
    Disabled,
}

/// One session key of an App Connect v2 sign-in.
#[derive(Clone, PartialEq, Debug)]
pub struct SessionKey {
    pub id: u32,
    pub level: KeyLevel,
    pub binding: AppBinding,
    pub budget: Duffs,
    pub expires_at: Timestamp,
    pub state: SessionKeyState,
    /// What is left of the budget, as Platform last said ([`Command::ReadAppBudgets`]); `None`
    /// until read.
    pub remaining: Option<Duffs>,
}

/// One session key an App Connect v2 app asks for, as the wallet offers it.
#[derive(Clone, PartialEq, Debug)]
pub struct SessionOffer {
    pub binding: AppBinding,
    pub level: KeyLevel,
    /// What the app proposed; `None`: nothing.
    pub proposed_budget: Option<Duffs>,
    pub proposed_expiry: Option<Timestamp>,
    /// What the wallet offers: the proposal within its limits, its defaults where there is none.
    pub budget: Duffs,
    pub expires_at: Timestamp,
    /// The wallet changed the app's proposal to fit its limits.
    pub changed: bool,
    /// The first key is the primary one: declining it declines the request.
    pub primary: bool,
}

/// One encryption key an App Connect v2 app asks for.
#[derive(Clone, PartialEq, Debug)]
pub struct EncryptionOffer {
    pub contract: String,
    /// Encryption (`true`) or decryption.
    pub encryption: bool,
    /// The app gets it: a new key added, or the identity's own current one; `false`: the
    /// identity has a key for it the wallet cannot send.
    pub given: bool,
    /// A new key is added for it.
    pub added: bool,
}

/// The wallet's limits on a session key, for the dialog: caps and presets.
#[derive(Clone, PartialEq, Debug)]
pub struct SessionLimits {
    pub max_budget: Duffs,
    pub budget_presets: Vec<Duffs>,
    /// Seconds.
    pub min_term: i64,
    pub max_term: i64,
    pub term_presets_days: Vec<i64>,
}

impl SessionLimits {
    /// The wallet's policy (analyst-72's recommendation, as DashPay 11.9.2 and the DIP have it,
    /// and the customer's decision of 2026-11-11): what the app asks for, at most 5 DASH and a
    /// year and at least an hour; 5 DASH and 90 days where it names nothing. The person can
    /// only lower them.
    pub const MAX_BUDGET: Duffs = 500_000_000;
    pub const DEFAULT_BUDGET: Duffs = Self::MAX_BUDGET;
    pub const BUDGET_PRESETS: [Duffs; 4] = [10_000_000, 50_000_000, 200_000_000, 500_000_000];
    pub const DEFAULT_TERM_DAYS: i64 = 90;
    pub const MAX_TERM_DAYS: i64 = 365;
    pub const MIN_TERM_SECS: i64 = 60 * 60;
    pub const TERM_PRESETS_DAYS: [i64; 3] = [30, 90, 365];

    pub fn wallet() -> Self {
        Self {
            max_budget: Self::MAX_BUDGET,
            budget_presets: Self::BUDGET_PRESETS.to_vec(),
            min_term: Self::MIN_TERM_SECS,
            max_term: Self::MAX_TERM_DAYS * 24 * 60 * 60,
            term_presets_days: Self::TERM_PRESETS_DAYS.to_vec(),
        }
    }
}

/// The person's answer to an App Connect v2 request: per asked session key, `None` to leave it
/// out (never the primary one) or the limits chosen.
#[derive(Clone, PartialEq, Debug)]
pub struct SessionGrant {
    pub keys: Vec<Option<(Duffs, Timestamp)>>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum NftOrigin {
    Gift {
        from: String,
        at: Timestamp,
    },
    /// Straight from the issuer's release.
    Release,
    /// Passed to this wallet; when, if the document keeps it (`$transferredAt` is kept only
    /// when the contract asks for it). Platform does not keep who handed it over.
    Received {
        at: Option<Timestamp>,
    },
}

/// Where the wallet's NFTs stand.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum NftsStatus {
    /// What `Wallet::nfts` holds is all there is.
    #[default]
    Ready,
    /// Asked for, the answer is on its way (the first time; later asks keep the last list).
    Loading,
    /// No identity: NFTs belong to identities.
    NoIdentity,
    /// The last ask failed; `Wallet::nfts` is what the one before found.
    Failed(String),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Nft {
    pub id: String,
    pub name: String,
    pub description: String,
    /// The image stored in the document, by its SHA-256 (hex). The bytes stay in the core:
    /// the interface asks for them drawn ([`Command::DrawArt`]), never decodes them itself.
    pub art: String,
    pub art_verified: bool,
    pub origin: NftOrigin,
    pub issuer: String,
    pub contract: String,
    /// Number and edition size; the size is 0 when the issuer set none.
    pub edition: (u32, u32),
    pub owner: String,
    pub price: Option<Duffs>,
}

/// A drawn picture: frames of one size, straight-alpha RGBA, checked by the core. The frames
/// are kept packed (LZ4): an animation is hundreds of full frames, and flat art packs many
/// times smaller. The interface unpacks the one it shows ([`Picture::unpack`]).
#[derive(Clone, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<PictureFrame>,
}

#[derive(Clone, PartialEq)]
pub struct PictureFrame {
    /// `width × height × 4` bytes, LZ4 block.
    packed: Box<[u8]>,
    /// How long it stays on screen; 0 for a still picture.
    pub delay_ms: u32,
}

impl PictureFrame {
    pub fn pack(rgba: &[u8], delay_ms: u32) -> Self {
        Self {
            packed: lz4_flex::block::compress(rgba).into(),
            delay_ms,
        }
    }

    /// A frame packed before (the core's cache); [`Picture::unpack`] checks it.
    pub fn from_packed(packed: Box<[u8]>, delay_ms: u32) -> Self {
        Self { packed, delay_ms }
    }

    pub fn packed(&self) -> &[u8] {
        &self.packed
    }

    pub fn packed_len(&self) -> usize {
        self.packed.len()
    }
}

impl Picture {
    /// Bytes in one unpacked frame.
    pub fn frame_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// Frame `i` into `out` (resized to [`Self::frame_len`]); `false` if there is no such
    /// frame or it does not unpack to exactly that size.
    pub fn unpack(&self, i: usize, out: &mut Vec<u8>) -> bool {
        let Some(frame) = self.frames.get(i) else {
            return false;
        };
        out.resize(self.frame_len(), 0);
        matches!(
            lz4_flex::block::decompress_into(&frame.packed, out),
            Ok(n) if n == out.len()
        )
    }
}

impl fmt::Debug for Picture {
    /// The size, not the pixels.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Picture({} × {}, {} frames)",
            self.width,
            self.height,
            self.frames.len()
        )
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Fees {
    /// A Core payment spending every coin of the wallet (with change): the most one can cost,
    /// what "Max" leaves for the fee. For a given amount: [`Fees::core_payment_for`].
    pub core_payment: Duffs,
    /// The wallet's spendable Core coins, largest first, each with the fee of a payment (with
    /// change) spending it and every coin before it (BUG-55). Empty when not known.
    pub core_coins: Vec<CoreCoin>,
    /// What of the Core balance no payment can sign for yet (BUG-142): a contact's coin filed in
    /// account 0 by the store's early load before its own account holds it. «Max» from Core leaves
    /// it out.
    pub core_unsignable: Duffs,
    /// The most a transfer of credits to an identity costs (what "Fee, at most" says), and to a
    /// Platform address (Platform meters both; a new address stores a row).
    pub credit_transfer: Duffs,
    pub credit_transfer_to_address: Duffs,
    /// A private transfer inside the pool (to a private address). The pool's fees are
    /// Platform's own formulas for a bundle of two Orchard actions; spending more than two
    /// notes at once costs a little more.
    pub pool_transfer: Duffs,
    pub nft_transfer: Duffs,
    /// A typical document write, to estimate "≈ N operation".
    pub document_write: Duffs,
    /// Moving the identity's credits into the shielded pool (the proof is built locally).
    pub shield: Duffs,
    /// What making a new identity takes from its asset lock, with a margin (BUG-93): Platform
    /// meters it (storage of the six keys, processing), so it is paid on top of the amount
    /// asked for and what the margin leaves stays as credits.
    pub identity_create: Duffs,
    /// The least asset lock Platform takes (since protocol 3, `calculate_min_required_fee` of
    /// the creation, six keys): a smaller one is refused and its DASH stay locked.
    /// [`Fees::identity_amount_least`] is the least amount evolite allows.
    pub identity_lock_min: Duffs,
    /// The least asset lock Platform takes to top an identity up: a smaller one is refused,
    /// its DASH stay locked and every next top-up waits for it (BUG-93 review, must 1).
    pub top_up_lock_min: Duffs,
    /// What Platform keeps of a lock that funds the wallet's Platform address, paid on top of
    /// the amount (FEAT-0015 stage 2): an upper estimate; what it does not need stays on the
    /// address. Read through [`Fees::address_fund_on_top`](address_fund).
    pub address_fund_on_top: Duffs,
    /// The least asset lock Platform takes to fund a Platform address (one output), from the
    /// protocol version: a smaller one is refused and its DASH stay locked.
    pub address_fund_lock_min: Duffs,
    /// A DPNS name: a regular one, and a contested one (goes to a masternode vote).
    pub name_regular: Duffs,
    pub name_contested: Duffs,
    /// Adding an app key to the identity.
    pub add_key: Duffs,
    /// A DashPay contact request (a document in Platform, paid in credits).
    pub contact_request: Duffs,
    /// The balance the network wants on the identity before it writes a contact request: more
    /// than the request costs, it checks an estimate first (BUG-107). A margin over what was seen.
    pub contact_request_balance: Duffs,
    /// Writing the DashPay profile.
    pub profile: Duffs,
    /// Shielding from Platform addresses: the fee (metered by Platform, so an estimate), and
    /// what stays on the first address to pay it (it cannot be shielded in the same move).
    pub shield_from_address: Duffs,
    pub shield_reserve: Duffs,
    /// Out of the pool to a Platform address.
    pub unshield: Duffs,
    /// Out of the pool to a Core address (Platform also writes a withdrawal document).
    pub pool_withdrawal: Duffs,
    /// Shielding Core funds: what Platform takes from the lock (the Core transaction fee is
    /// `core_payment` on top).
    pub shield_from_core: Duffs,
    /// Out of the pool to the identity's credits.
    pub pool_top_up: Duffs,
    /// Credits withdrawn to a Core address (the identity's credits; Platform pays out later).
    pub credit_withdrawal: Duffs,
    /// What Platform wants on the identity's balance on top of a withdrawal's amount before it
    /// takes one (`state_transition_min_fees.credit_withdrawal`): a check, not a price — the
    /// withdrawal itself costs about `credit_withdrawal`, but a balance under amount + reserve
    /// is refused, whatever the amount.
    pub credit_withdrawal_reserve: Duffs,
}

/// A Core coin as coin selection sees it ([`Fees::core_coins`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CoreCoin {
    pub value: Duffs,
    /// The fee of a payment spending this coin and every larger one.
    pub fee: Duffs,
}

impl Fees {
    /// The fee of a Core payment of `amount` (0: nothing typed yet — the cheapest payment, from
    /// one coin). As the wallet chooses the coins when no set fits without change: the largest
    /// first, until they cover the amount and their own fee. Not that many coins: what spending
    /// them all costs.
    pub fn core_payment_for(&self, amount: Duffs) -> Duffs {
        let mut held: Duffs = 0;
        for coin in &self.core_coins {
            held = held.saturating_add(coin.value);
            if held >= amount.saturating_add(coin.fee) {
                return coin.fee;
            }
        }
        self.core_coins.last().map_or(self.core_payment, |c| c.fee)
    }

    /// What a name takes from the identity's credits, with a margin, as paid on top of the
    /// amount asked for (BUG-93): never below [`NAME_ON_TOP`], and what the last name of this
    /// data folder cost when that was more.
    pub fn name_on_top(&self) -> Duffs {
        self.name_regular.max(NAME_ON_TOP)
    }

    /// Platform's fees for an identity's creation and (when `named`) its name, paid on top of
    /// the amount asked for (BUG-93): upper estimates, so the amount reaches the identity whole.
    pub fn identity_fees_on_top(&self, named: bool) -> Duffs {
        let name = if named { self.name_on_top() } else { 0 };
        self.identity_create.saturating_add(name)
    }

    /// The asset lock that leaves `amount` as credits (BUG-93): the amount and the fees on top.
    pub fn identity_lock_for(&self, amount: Duffs, named: bool) -> Duffs {
        amount.saturating_add(self.identity_fees_on_top(named))
    }

    /// The least amount to ask for (BUG-93, BUG-97). The lock it makes reaches Platform's own
    /// minimum; with a name, the credits after the creation also cover
    /// [`Self::name_credits_needed`], so a name is not refused for lack of credits after the
    /// DASH are locked: `amount` + what is on top for the name (the creation's own estimate is
    /// spent by the creation) ≥ what the name wants up front. Never below one duff.
    pub fn identity_amount_least(&self, named: bool) -> Duffs {
        let for_the_lock = self
            .identity_lock_min
            .saturating_sub(self.identity_fees_on_top(named));
        let for_the_name = if named {
            self.name_credits_needed()
                .saturating_sub(self.name_on_top())
        } else {
            0
        };
        for_the_lock.max(for_the_name).max(1)
    }

    /// Platform's fee of topping an identity up from Core, on top of the amount asked for
    /// (BUG-93): an upper estimate, so the amount reaches the identity whole.
    pub fn top_up_on_top(&self) -> Duffs {
        TOP_UP_ON_TOP
    }

    /// What the identity must hold before a name's preorder is sent (BUG-97): the preorder, and
    /// what Platform wants for the domain out of what is left after it. A dearer last name
    /// raises it. See [`NAME_DOMAIN_PEAK`].
    pub fn name_credits_needed(&self) -> Duffs {
        NAME_PREORDER.saturating_add(NAME_DOMAIN_PEAK.max(self.name_regular))
    }

    /// What a name is shown to cost (BUG-97): a first name on an identity measured more than a
    /// later one, so it is never shown below [`NAME_FIRST`], whatever the last name here cost.
    pub fn name_price(&self, first: bool) -> Duffs {
        if first {
            self.name_regular.max(NAME_FIRST)
        } else {
            self.name_regular
        }
    }

    /// The least amount to top up by: its lock (the amount and the fee on top) reaches
    /// Platform's [`Self::top_up_lock_min`]. Never below one duff.
    pub fn top_up_amount_least(&self) -> Duffs {
        self.top_up_lock_min.saturating_sub(TOP_UP_ON_TOP).max(1)
    }
}

/// What goes on top for a name (BUG-93). The firm number is the live 222 937 duffs of a creation
/// and a first name together: 145 715 the creation and 77 222 the name (BUG-97, pass 12 measured
/// the creation alone). A name's own cost is less than what it needs up front
/// ([`NAME_DOMAIN_PEAK`]): the identity's credits, not this margin, have to cover that.
const NAME_ON_TOP: Duffs = 90_000;
/// A first preorder on an identity (BUG-97): measured 35 478 duffs on testnet (pass 12, the
/// identity's first DPNS write; a later one 23 559), rounded up. Inferred: the first document of
/// the contract costs more through the identity's first nonce for it.
const NAME_PREORDER: Duffs = 40_000;
/// What Platform wants on the identity for the domain, out of what is left after the preorder
/// (BUG-97; `validate_fees_of_event` compares the balance with the estimated fee before the
/// transition, no refund counted). Measured on testnet: refused with 78 546 left (pass 12);
/// went through with about 117 786 left (pass 8: 298 979 locked − 145 715 created − 35 478).
/// The estimate is not computed here (it needs Drive's dry run), so this is the measured upper
/// side, with what a longer name can add (about 7 500 for 63 characters), rounded up. BUG-93's
/// 92 700 compared with the credits before the preorder and was too low: disproved live.
const NAME_DOMAIN_PEAK: Duffs = 125_000;
/// A first name on an identity measured at most 77 222 duffs (pass 8, preorder and domain;
/// a name cost 57 979 on an identity that had one): the form never shows less for a first name.
const NAME_FIRST: Duffs = 78_000;
/// A top-up's fee on testnet: 8 130 duffs (2026-11-05, 0.01 DASH), rounded up.
const TOP_UP_ON_TOP: Duffs = 10_000;

#[derive(Clone, PartialEq, Debug)]
pub struct Wallet {
    pub balances: Balances,
    /// Newest first.
    pub activity: Vec<Activity>,
    /// Fresh addresses to receive on.
    pub core_address: String,
    pub platform_address: String,
    /// The credits on each of the wallet's Platform addresses, in the addresses' own order (as a
    /// transition keeps them): what [`address_plan::address_plan`] chooses from (FEAT-0015).
    pub platform_inputs: Vec<u64>,
    /// The address (bech32m) of each of [`Self::platform_inputs`], in the same order: a transfer never
    /// pays from the address it pays to ([`address_plan::plan_to`], BUG-141).
    pub platform_input_addresses: Vec<String>,
    pub identity: Option<Identity>,
    pub pool: Pool,
    pub apps: Vec<ConnectedApp>,
    pub nfts: Vec<Nft>,
    pub nfts_status: NftsStatus,
    /// DashPay contacts of the identity, requests both ways included.
    pub contacts: Vec<Contact>,
    /// An identity whose making was cut short: Core funds locked, or the identity made but
    /// not its name. `ResumeIdentity` finishes it.
    pub unfinished_identity: Option<UnfinishedIdentity>,
    pub fees: Fees,
    pub phrase_words: usize,
    pub created_at: Timestamp,
    /// A top-up of the identity that Platform has not taken yet, or not provably.
    /// `ResumeTopUp` finishes it.
    pub unfinished_top_up: Option<UnfinishedTopUp>,
    /// A funding of the Platform address from Core whose lock Platform has not taken yet
    /// (FEAT-0015 stage 2). `ResumeFundAddress` finishes it; until then a new one is not
    /// offered.
    pub unfinished_address_top_up: Option<UnfinishedAddressTopUp>,
    pub identity_search: IdentitySearch,
    /// All the wallet's identities by slot; [`Wallet::identity`] is the chosen one.
    pub identities: Vec<IdentityChoice>,
    /// Core addresses the person named, to pay again (not DashPay contacts), by name.
    pub address_book: Vec<SavedAddress>,
    /// "Password for every send" (Settings → Security), kept by the core for this wallet: on,
    /// every [`Command::Spend`] needs the password; off only by [`Command::SetPasswordEachSend`]
    /// with the password.
    pub password_each_send: bool,
    /// What was wrong with the notes file when the wallet opened, if anything: the history
    /// shows no notes from it (FEAT-0003, E2).
    pub notes_error: Option<NotesError>,
    /// Platform tokens of the identity the interface shows, and the wallet's search for them
    /// (FEAT-0019). [`tokens::Tokens::off`] on mainnet (R-9).
    pub tokens: tokens::Tokens,
}

/// Why the history shows no notes from the file there (FEAT-0003, E2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotesError {
    /// It did not open (damaged, or not this wallet's): put aside, new notes go to a new file.
    PutAside,
    /// It could not be read now (held by another program, no access, too big): left as it is,
    /// and no note is saved until the wallet is opened again and the file is read.
    NotRead,
}

/// A Core address in the address book, with the name the person gave it.
#[derive(Clone, PartialEq, Debug)]
pub struct SavedAddress {
    pub address: String,
    pub label: String,
}

/// Everything the screens show. Sent whole whenever something changes.
#[derive(Clone, PartialEq, Debug)]
pub struct Snapshot {
    pub network: Network,
    pub sync: SyncInfo,
    pub vault: VaultState,
    /// `None` until a wallet is created, restored or unlocked.
    pub wallet: Option<Wallet>,
    /// The data is an example, not a wallet: the screens say so.
    pub demo: bool,
    /// The wallet's data folder is open in another evolite window: this one neither reads
    /// nor writes it (no store, no keys) and says so, until that window closes.
    pub elsewhere: bool,
    /// The price of one DASH in the currency asked with [`Command::FiatRate`], once fetched.
    pub fiat: Option<FiatRate>,
    /// The wallets on this computer for this network, the open one marked; empty while
    /// there is none yet.
    pub wallets: Vec<WalletEntry>,
    /// A wallet is being added: its onboarding shows "Cancel" (`CancelAddWallet`).
    pub adding_wallet: bool,
}

/// A wallet on this computer: for the switcher and Settings → Wallets. Its name is
/// local, never sent anywhere; nothing here is secret.
#[derive(Clone, PartialEq, Debug)]
pub struct WalletEntry {
    pub id: String,
    pub name: String,
    /// One of six wallet colours, by the order wallets were added.
    pub color: u8,
    pub origin: WalletOrigin,
    pub created_at: Timestamp,
    pub last_opened: Option<Timestamp>,
    /// The wallet this window works with (unlocked or waiting for its password).
    pub open: bool,
    /// Wrong passwords in a row for this wallet, on its unlock screen and in "Open "…""
    /// (FEAT-0002); 0 after a right one. The interface warns before the pause by it.
    pub wrong_tries: u32,
    /// Until when the core refuses this wallet's password without checking it (FEAT-0002):
    /// absolute, so the interface counts down by its own clock. `None`: no pause now.
    pub wait_until: Option<Timestamp>,
}

impl WalletEntry {
    /// The letter in its circle: the name's first.
    pub fn letter(&self) -> String {
        self.name
            .chars()
            .next()
            .map_or("?".into(), |c| c.to_uppercase().collect())
    }
}

/// How a wallet came to this computer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WalletOrigin {
    Created,
    Restored,
    DashPay,
}

/// The price of one DASH from the rate service (Settings → "Fiat rate").
/// `Debug` names the currency only: the price and its time stay out of logs (FEAT-0008, AC-T9).
#[derive(Clone, PartialEq)]
pub struct FiatRate {
    /// "USD", "EUR", "RUB".
    pub currency: String,
    /// Hundred-millionths of the currency for one DASH.
    pub price: i64,
    /// When the core got it.
    pub at: Timestamp,
}

impl fmt::Debug for FiatRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FiatRate")
            .field("currency", &self.currency)
            .finish_non_exhaustive()
    }
}

/// A coin of the wallet's Core balance (FEAT-0013): one unspent output it may pay with.
#[derive(Clone, PartialEq, Debug)]
pub struct Coin {
    /// `txid:vout`, as the journal and CSV write a transaction.
    pub outpoint: String,
    pub value: Duffs,
    /// The address it is on.
    pub address: String,
    /// It came to a DashPay account: from whom (their name, else their short id).
    pub from_contact: Option<String>,
    pub state: CoinState,
    /// When it came, if the wallet knows.
    pub at: Option<Timestamp>,
    /// A coin of the mixing account that has not been through all the rounds (FEAT-0030 AC-A6):
    /// the rounds it has (0 for one that is no denomination) and those a coin needs. It goes only
    /// with others like it.
    pub mixing: Option<(u8, u8)>,
}

/// Whether a coin can go into a payment now (R-4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoinState {
    /// In a block, or InstantSend-locked by a lock checked in this run.
    Final,
    /// Not in a block and no checked InstantLock yet (BUG-111): not spendable as final.
    Unconfirmed,
    /// Held by a payment on its way.
    Reserved,
    /// No account of the wallet can sign for it yet (BUG-142, BUG-148): a contact's coin waiting
    /// for its DashPay account. Not to tick.
    Unsignable,
}

/// The coins a Core payment may take (FEAT-0013, the customer's path 2, the architect's
/// answer of 2026-11-11).
#[derive(Clone, PartialEq)]
pub enum CoinChoice {
    Within {
        /// The outpoints (`txid:vout`) chosen.
        coins: Vec<String>,
        /// «Max»: every chosen coin, no change; the amount is what is left after the fee.
        /// Else the payment takes what it needs from them, largest first.
        all: bool,
    },
}

impl CoinChoice {
    pub fn outpoints(&self) -> &[String] {
        match self {
            CoinChoice::Within { coins, .. } => coins,
        }
    }

    pub fn all(&self) -> bool {
        match self {
            CoinChoice::Within { all, .. } => *all,
        }
    }
}

/// The count only: no outpoint in a log (R-11).
impl fmt::Debug for CoinChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Within({} coins, all: {})",
            self.outpoints().len(),
            self.all()
        )
    }
}

/// Where a payment comes from on the Send screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Core,
    /// The CoinJoin account's fully mixed coins (FEAT-0030): paid with [`Spend::SendMixed`], to a
    /// Core address only, no change.
    Mixed,
    Credits,
    Pool,
    /// The wallet's Platform addresses (FEAT-0015): what they do is chosen by the recipient —
    /// a Platform address (transfer), a Core address of this wallet (withdrawal), one of its
    /// identities (top-up). Public on Platform, unlike the pool.
    PlatformAddress,
}

/// Whether a DPNS name can be taken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameCheck {
    Available,
    /// Free, but short names made of letters and 0/1 go to a masternode vote.
    Contested,
    Taken,
}

/// Where credits are shielded from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShieldSource {
    Credits,
    PlatformAddress,
    /// Core funds: an asset lock whose value, less Platform's fee, goes into the pool.
    Core,
}

/// A request from an app, decoded by the core (not described by the app).
#[derive(Clone, PartialEq, Debug)]
pub struct AppRequest {
    pub id: u64,
    pub app: String,
    pub kind: AppRequestKind,
}

#[derive(Clone, PartialEq, Debug)]
pub enum AppRequestKind {
    /// App Connect v2 (`dashpay://connect` or `dash-key:` with `v=2`): session keys bound to the
    /// app's contracts, each with a budget and an end, and its encryption keys. Checked against
    /// Platform before it is shown; answered with [`Spend::ApproveApp`] and a [`SessionGrant`].
    Connect2 {
        /// The app's contract, base58.
        app_id: String,
        /// What the app calls itself: its own words, not checked.
        label: String,
        /// The contract owner's identity (base58) and `@name`, if it has one.
        owner: String,
        owner_name: Option<String>,
        /// The identity signing in, as shown (`@name` or base58).
        identity: String,
        sessions: Vec<SessionOffer>,
        encryption: Vec<EncryptionOffer>,
        limits: SessionLimits,
        /// R-6: the identity has no key of the new kind yet; after this sign-in old Yappr and
        /// DashPay (11.9.1 and earlier) cannot read it any more.
        breaks_old_clients: bool,
        /// A sign-in again with the same connection (`CONNECTION_ID`): its earlier session keys
        /// are disabled in the same update.
        renews: Option<Renewal>,
        /// When the app made the request (Unix seconds).
        created_at: Timestamp,
        /// The host the app asks to return to, if any (only a hint to the app).
        callback_host: Option<String>,
        cost: Duffs,
    },
}

/// What a DashPay import found, shown before the password step.
#[derive(Clone, PartialEq, Debug)]
pub struct DashPayScan {
    pub core: Duffs,
    pub core_addresses_used: u32,
    pub identity_id: Option<String>,
    pub identity_credits: Duffs,
    pub identity_keys: u32,
    pub name: Option<String>,
    pub contacts: u32,
    /// Coins after CoinJoin that evolite sees but cannot mix: (addresses, duffs).
    pub coinjoin: Option<(u32, Duffs)>,
    /// `core` and `core_addresses_used` are known: the real core reads Platform before the
    /// wallet exists, and Core balances only come with the wallet's own sync.
    pub core_known: bool,
}

/// A payment the core signed and handed to the network.
#[derive(Clone, PartialEq, Debug)]
pub struct SentPayment {
    pub txid: String,
    pub amount: Duffs,
    pub fee: Duffs,
    /// Peers we did not send it to announced it back (or it got an InstantLock). `false`:
    /// no sign within a minute — the payment may still go through, it is not resent.
    pub relayed: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ResolvedName {
    pub name: String,
    pub display: Option<String>,
    pub identity_id: String,
    /// Its DashPay profile, when it has one.
    pub profile: Option<Profile>,
    /// Where it stands among our contacts; `None`: not one.
    pub contact: Option<ContactState>,
}

/// A recovery phrase on its way to the screen or back. Wiped on drop (each word, then the
/// list), never printed.
#[derive(Clone, PartialEq)]
pub struct Phrase(Zeroizing<Vec<String>>);

impl Phrase {
    pub fn new(words: Vec<String>) -> Self {
        Self(Zeroizing::new(words))
    }

    pub fn words(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Debug for Phrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Phrase({} words, hidden)", self.0.len())
    }
}

/// A password typed by the user. Wiped on drop, never printed.
#[derive(Clone, PartialEq)]
pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(hidden)")
    }
}

/// A payment note's text in a command (FEAT-0003): `Debug` never shows it, nor its length
/// (AC-T6), so no log of a command can carry it.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct NoteText(pub String);

impl fmt::Debug for NoteText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NoteText(hidden)")
    }
}

#[derive(Debug)]
pub enum Command {
    /// A fresh phrase for a new wallet (12 or 24 words). Answered with [`Event::NewPhrase`].
    NewPhrase {
        words: usize,
    },
    /// Check a typed phrase and, for DashPay, scan its paths. Answered with
    /// [`Event::PhraseAccepted`].
    UsePhrase {
        phrase: Phrase,
        dashpay: bool,
    },
    /// Encrypt the phrase from the last step and start the first sync.
    CreateWallet {
        password: Password,
    },
    /// Open the saved wallet with its password. Answered with `Done { what: DoneWhat::Unlock }`;
    /// after too many wrong passwords in a row, for a while with [`CoreError::TooManyTries`],
    /// the password not checked (FEAT-0002).
    Unlock {
        password: Password,
    },
    /// Forget the keys in memory; sync goes on with headers only.
    Lock,
    /// Lock by itself after this long without [`Command::Touch`]; `None`: only when the app
    /// closes (the keys never outlive the process).
    AutoLock {
        after_secs: Option<u64>,
    },
    /// The person is using the app: the auto-lock timer starts again.
    Touch,
    /// Encrypt the phrase again under a new password. Answered with `Done { what: DoneWhat::Password }`.
    ChangePassword {
        old: Password,
        new: Password,
    },
    ResolveName {
        name: String,
    },
    /// Anything that spends the wallet's money or credits, behind one password gate: with
    /// [`Wallet::password_each_send`] on, `password` is checked against the vault before any
    /// other check, lock of funds or signing (a wrong or missing one: [`CoreError::WrongPassword`]
    /// in the operation's own answer, see [`Spend::refused`]). Each operation is answered as it
    /// always was ([`Spend::what`]).
    Spend {
        op: Spend,
        password: Option<Password>,
    },
    /// Settings → Security → "Password for every send". Turning it off needs the password
    /// (else anyone at the unlocked computer could switch it off and spend); turning it on does
    /// not. Answered with `Done { what: DoneWhat::PasswordEachSend }`.
    SetPasswordEachSend {
        on: bool,
        password: Option<Password>,
    },
    /// Read what is left of the budgets of app `id`'s session keys from Platform. Answered with
    /// a new snapshot (`ConnectedApp::session`), or an error event if Platform cannot be asked.
    ReadAppBudgets {
        id: String,
    },
    /// Remove a disconnected app from the list: only the record on this computer, so no
    /// password and nothing spent (its keys stay disabled on the identity). Answered with
    /// `Done { what: DoneWhat::ForgetApp }`.
    ForgetApp {
        id: String,
    },
    /// Is this DPNS label free? Answered with [`Event::NameChecked`].
    CheckName {
        label: String,
    },
    /// A link or QR text from an app (App Connect v2: `dashpay://connect`, or `dash-key:` with
    /// `v=2` on testnet). Answered with [`Event::AppRequest`] or an error; a v1 link
    /// (`dash-key:` v1, `dash-st:`) is refused with words of its own.
    OpenAppLink {
        text: String,
    },
    /// Turn down an app's request (approving it is [`Spend::ApproveApp`]). Answered with
    /// `Done { what: DoneWhat::App }`.
    DeclineApp {
        id: u64,
    },
    /// Draw a picture (`Nft::art`) `side` pixels across, by the sandboxed renderer. Answered
    /// with [`Event::Art`]: the first frame at once, then the animation. `urgent`: it is on
    /// screen, draw it before the others.
    DrawArt {
        art: String,
        side: u32,
        urgent: bool,
    },
    /// Show the phrase after the password. Answered with [`Event::PhraseRevealed`]. `wallet`:
    /// another wallet of this computer (its "…" → "Show the phrase"), by its own password;
    /// none: the open one.
    RevealPhrase {
        password: Password,
        wallet: Option<String>,
    },
    /// Stop showing requests from this identity (kept on this computer only).
    /// `Done { what: DoneWhat::Contact }`.
    HideRequest {
        identity_id: String,
    },
    /// Where a Core payment to `to` will go, for the last look before signing: the address
    /// itself, or for a DashPay contact (`@name`) the next address of its key. Answered with
    /// [`Event::PaymentPreview`]; a payment to a contact is signed only to the address shown.
    PreviewPayment {
        to: String,
    },
    /// The open wallet's Core coins for «Coins» on Send (FEAT-0013): read from the wallet, no
    /// network (R-14). Answered with [`Event::Coins`] carrying the same `ask`: the form keeps
    /// only the answer to its last ask (a wallet switch, a lock, a dialog opened again).
    ListCoins {
        ask: u64,
    },
    /// What a payment of `amount` to `to` from «Mixed» costs, for the review before the
    /// password (FEAT-0030 phase A): read from the wallet, nothing reserved, nothing sent.
    /// Answered with [`Event::MixedPreview`] carrying the same `ask`.
    PreviewMixed {
        ask: u64,
        to: String,
        amount: Duffs,
    },
    /// Look for the phrase's identities on Platform again, now. The answer is in
    /// [`Wallet::identity_search`].
    FindIdentities,
    /// Ask Platform for the shown identity's NFTs again, now, not at the next refresh
    /// (Collection's «Try again» after a failed load). The answer is in [`Wallet::nfts`] and
    /// [`Wallet::nfts_status`].
    RefreshNfts,
    /// Act as this identity (base58) from now on: the Identity screen, credits, contacts,
    /// profile, top-ups and names follow it.
    ChooseIdentity {
        id: String,
    },
    /// Remove the wallet from this computer: the vault, the store and everything kept next
    /// to them (the money stays on the network, back with the phrase). `password`: checked
    /// first, from Settings; `None` from "Forgot the password?", where it is not known. Answered with
    /// `Done { what: DoneWhat::Delete }`; the core then starts again with no wallet.
    /// `wallet`: another wallet of this computer (its "…" in Settings), by its own password,
    /// which is then required; the open one stays as it is. Answered with
    /// `Done { what: DoneWhat::DeleteOther }` (R-28). `None` or the open one's id: the open one.
    DeleteWallet {
        password: Option<Password>,
        wallet: Option<String>,
    },
    /// "Check the phrase": open the vault with the password and ask a few words of the
    /// phrase by number. Answered with [`Event::PhraseCheck`]; the phrase stays in the core.
    AskPhraseCheck {
        password: Password,
    },
    /// The words picked for the last `AskPhraseCheck`, in its order. Answered with
    /// [`Event::PhraseChecked`]; a check is answered once.
    CheckPhraseWords {
        picks: Vec<String>,
    },
    /// "Sync again": at the next start, before anything opens, the headers,
    /// filters and masternode lists and the wallet's store go, and the wallet is scanned again
    /// from its birth (the vault, the Platform journal and contacts' addresses stay). Refused
    /// while an identity creation or top-up is unfinished: its lock lives in the store.
    /// Answered with `Done { what: DoneWhat::Resync }`; the app then starts again.
    Resync,
    /// Keep a Core testnet address in the address book under `label` (1–40 characters), or
    /// rename it. `Done { what: DoneWhat::AddressBook }`.
    SaveAddress {
        address: String,
        label: String,
    },
    /// Take an address out of the address book. `Done { what: DoneWhat::AddressBook }`.
    ForgetAddress {
        address: String,
    },
    /// Settings → "Own node": from the next start connect only to this Core node
    /// (`host:port`; the testnet port 19999 when none is given); `None`: nodes from DNS seeds
    /// again. The address must resolve now. Answered with `Done { what: DoneWhat::CoreNode }`.
    CoreNode {
        address: Option<String>,
    },
    /// Settings → "Fiat rate": fetch the price of DASH in `currency` now and every
    /// 30 minutes from the rate service, which sees our IP; `None`: stop and forget the price.
    FiatRate {
        currency: Option<String>,
    },
    /// Settings → "A new address after each payment": `true` shows the next unused
    /// receive address, `false` always the wallet's first one.
    ReceiveAddress {
        fresh: bool,
    },
    /// Lock this wallet and open another one of this computer. With `password` (Figma 28,
    /// 29) it is checked against that wallet first: a wrong one changes nothing, the right one
    /// opens it at once; answered with `Done { what: DoneWhat::Switch }`. Without, its unlock screen
    /// comes next and only the snapshots answer. Wrong passwords count for that wallet as on
    /// its unlock screen, so it may be answered with [`CoreError::TooManyTries`] (FEAT-0002).
    SwitchWallet {
        id: String,
        password: Option<Password>,
    },
    /// Start adding a wallet: this one is locked and the onboarding opens for a new one, with
    /// "Cancel" (`CancelAddWallet`) back to this one.
    AddWallet,
    CancelAddWallet,
    /// Keep `text` as the note of the operation `key` ([`Activity::key`]); an empty text
    /// deletes it. Only on this computer, encrypted; never sent anywhere. At most
    /// [`NOTE_MAX_GRAPHEMES`] characters and [`NOTE_MAX_BYTES`] bytes. Answered with
    /// `Done { what: DoneWhat::Note }`.
    SetNote {
        key: String,
        text: NoteText,
    },
    /// Name a wallet of this computer (1–24 characters) and give it one of the six colours
    /// (Figma 30); local, never sent anywhere. Answered with `Done { what: DoneWhat::Wallet }`.
    RenameWallet {
        id: String,
        name: String,
        color: u8,
    },
    /// "Add token": the token by its id, or every token of a contract by the contract's id (one
    /// field takes both), read with a proof. Answered with [`Event::TokenFound`]; nothing is
    /// added until [`Command::TrackToken`]. Testnet only (FEAT-0019 R-9).
    FindToken {
        id: String,
    },
    /// "Add token" by a word: tokens whose contract has `keyword` (an exact match, lower case,
    /// 3–50 characters). Answered with [`Event::TokenFound`].
    SearchTokens {
        keyword: String,
    },
    /// Keep the token in the wallet's list (`on`), or take it out — a found one is hidden and
    /// not shown again by the search. Its balance stays where it is. Local, in the wallet's
    /// folder. Answered with `Done { what: DoneWhat::Token }`.
    TrackToken {
        token_id: String,
        on: bool,
    },
    /// "Find my tokens": go through the network's contracts again now (UC-1а). Only the
    /// snapshots answer ([`Wallet::tokens`]'s search).
    FindMyTokens,
}

/// Everything that spends: Core DASH, credits or the pool. Only through [`Command::Spend`], so
/// no spend can go past the password gate (BUG-42, R-27).
#[derive(Debug, Clone, PartialEq)]
pub enum Spend {
    /// Pay `amount` to `to` (an address, `@name` or a contact) from `from`. Answered with
    /// [`Event::Sent`].
    Send {
        /// The interface's number of this payment; [`Event::Sent`] says it back.
        id: u64,
        from: Source,
        to: String,
        amount: Duffs,
        /// The person's note to the payment (FEAT-0003): kept with the operation once
        /// the network took it, only on this computer; never sent.
        note: Option<NoteText>,
        /// The rate «Summary» showed (FEAT-0008, АРХ-2): the core keeps it with the operation
        /// only if its currency is the setting's and it is at most 5 minutes old. Never logged.
        rate: Option<FiatRate>,
        /// From Core only: the coins the payment may take, chosen by the person (FEAT-0013);
        /// `None`, the wallet chooses as before (R-2).
        coins: Option<CoinChoice>,
    },
    /// Pay exactly `amount` to the Dash address `to` from «Mixed» (FEAT-0030 phase A): fully
    /// mixed coins of the CoinJoin account only, never with other coins, no change. What the
    /// coins hold over the amount and the fee goes with the fee ([`MixedQuote::excess`]).
    /// Answered with [`Event::Sent`], as [`Spend::Send`].
    SendMixed {
        id: u64,
        to: String,
        amount: Duffs,
        note: Option<NoteText>,
        rate: Option<FiatRate>,
        /// The excess the person saw and agreed to ([`MixedQuote::excess`]); the core builds
        /// the payment again and refuses if it burns another excess than this one (АРХ-9а).
        accept_excess: Duffs,
    },
    /// Lock `amount` duffs on Core and add them to the shown identity's credits.
    /// `Done { what: DoneWhat::TopUp }`.
    TopUpIdentity { amount: Duffs },
    /// Finish [`Wallet::unfinished_top_up`] with the lock already made, without locking more.
    /// `Done { what: DoneWhat::TopUp }`.
    ResumeTopUp,
    /// Lock DASH on Core and credit the wallet's own Platform address with them (FEAT-0015
    /// stage 2): the address gets at least `amount` duffs, Platform's processing is on top of it
    /// ([`address_fund::fund_plan`]). `Done { what: DoneWhat::FundAddress }`.
    FundAddress { amount: Duffs },
    /// Finish [`Wallet::unfinished_address_top_up`] with the lock already made, without locking
    /// more. `Done { what: DoneWhat::FundAddress }`.
    ResumeFundAddress,
    /// Lock DASH on Core, create the identity in the phrase's next slot and register `name`;
    /// `None`: an identity without a name (FEAT-0014 R-7). Once made, the new identity is the
    /// chosen one. `Done { what: DoneWhat::Identity }`.
    CreateIdentity { name: Option<String>, top_up: Duffs },
    /// Finish [`Wallet::unfinished_identity`] from where it stopped, without locking more.
    /// `Done { what: DoneWhat::Identity }`.
    ResumeIdentity,
    /// Register one more DPNS name for the shown identity, paid from its credits.
    /// `Done { what: DoneWhat::Name }`.
    AddName { name: String },
    /// Move credits into the pool; the core builds the proof and reports [`Event::Progress`],
    /// then `Done { what: DoneWhat::Shield }`.
    Shield { from: ShieldSource, amount: Duffs },
    /// Take `amount` out of the pool to the wallet's own Platform address; the amount becomes
    /// visible on the network. Answered with `Done { what: DoneWhat::Unshield }`, [`Event::Progress`]
    /// while the proof is built.
    Unshield { amount: Duffs },
    /// Take `amount` out of the pool into the credits of the identity the interface shows; the
    /// identity gets `amount`, the fee comes from the pool on top. Answered with
    /// `Done { what: DoneWhat::PoolTopUp }`, [`Event::Progress`] while the proof is built.
    TopUpFromPool { amount: Duffs },
    /// Send a DashPay contact request to `name` (a DPNS name). Answered with
    /// `Done { what: DoneWhat::Contact }`.
    AddContact { name: String },
    /// Answer an incoming request with ours: the contact is made. `Done { what: DoneWhat::Contact }`.
    AcceptContact { identity_id: String },
    /// Write the identity's DashPay profile. `avatar_url`: https, the core downloads the
    /// picture to put its hash in the profile. `Done { what: DoneWhat::Profile }`.
    SaveProfile {
        display: String,
        about: String,
        avatar_url: Option<String>,
    },
    /// Do what an app asked ([`Event::AppRequest`]): an App Connect v2 sign-in (`Connect2`)
    /// with the person's `grant`. `Done { what: DoneWhat::App }`.
    ApproveApp {
        id: u64,
        grant: Option<SessionGrant>,
    },
    /// Give an NFT to `to`. `Done { what: DoneWhat::Gift }`.
    GiftNft { id: String, to: String },
    /// Send `amount` (in the token's smallest units) of `token_id` from the identity the
    /// interface shows to `to` (an `@name` or an identity id). The fee is in that identity's
    /// credits. No note in stage 1. Answered with `Done { what: DoneWhat::SendToken }`; the
    /// sender's balance is taken from the transfer's proved answer.
    SendToken {
        token_id: String,
        to: String,
        amount: tokens::TokenAmount,
    },
    /// Disconnect app `id`: its session keys disabled (never its encryption keys) in an
    /// IdentityUpdate the identity pays for, so it passes the password gate (BUG-42); its
    /// response deleted. `Done { what: DoneWhat::Disconnect }`.
    DisconnectApp { id: String },
}

impl Spend {
    /// Whether it asks for the wallet's password whatever "Password for every send" says: an App
    /// Connect v2 sign-in, which cannot be undone for the identity (FEAT-0023 round 4, R-6).
    pub fn asks_password_always(&self) -> bool {
        matches!(self, Spend::ApproveApp { grant: Some(_), .. })
    }

    /// The `what` of the operation's `Done` (or `"send"` for [`Event::Sent`]).
    pub fn what(&self) -> DoneWhat {
        match self {
            Spend::Send { .. } | Spend::SendMixed { .. } => DoneWhat::Send,
            Spend::TopUpIdentity { .. } | Spend::ResumeTopUp => DoneWhat::TopUp,
            Spend::FundAddress { .. } | Spend::ResumeFundAddress => DoneWhat::FundAddress,
            Spend::CreateIdentity { .. } | Spend::ResumeIdentity => DoneWhat::Identity,
            Spend::AddName { .. } => DoneWhat::Name,
            Spend::Shield { .. } => DoneWhat::Shield,
            Spend::Unshield { .. } => DoneWhat::Unshield,
            Spend::TopUpFromPool { .. } => DoneWhat::PoolTopUp,
            Spend::AddContact { .. } | Spend::AcceptContact { .. } => DoneWhat::Contact,
            Spend::SaveProfile { .. } => DoneWhat::Profile,
            Spend::ApproveApp { .. } => DoneWhat::App,
            Spend::GiftNft { .. } => DoneWhat::Gift,
            Spend::SendToken { .. } => DoneWhat::SendToken,
            Spend::DisconnectApp { .. } => DoneWhat::Disconnect,
        }
    }

    /// The answer when the operation does not happen at all (the gate, a wallet that is not
    /// there): the event its screen already waits for.
    pub fn refused(&self, why: CoreError) -> Event {
        match self {
            Spend::Send { id, .. } | Spend::SendMixed { id, .. } => Event::Sent {
                id: *id,
                result: Err(why),
            },
            _ => Event::Done {
                what: self.what(),
                result: Err(why),
            },
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum CoreError {
    /// The core cannot do this yet (the prototype has no wallet behind it).
    NotAvailable(String),
    WrongPassword,
    BadPhrase,
    NameNotFound,
    /// Not a DPNS label: 3–63 letters, digits and inner hyphens.
    BadName,
    BadAmount,
    /// Not a Dash address of this network.
    BadAddress,
    /// The balance does not cover the amount and the fee.
    NotEnough,
    /// Wait for the first sync: the wallet does not know its coins yet.
    NotSynced,
    /// The network refused or the build failed; the text is for the details line.
    Failed(String),
    /// Not an app request the wallet knows.
    BadLink,
    /// Too many wrong passwords in a row: the next one is taken after `until` (Unix seconds);
    /// this one was not checked (FEAT-0002).
    TooManyTries {
        until: Timestamp,
    },
    /// This system cannot close the picture renderer in, so no picture from outside is drawn.
    PicturesOff(PicturesOff),
    /// An app's link is refused before anything is shown or spent (FEAT-0023).
    AppLink(AppLinkRefusal),
    /// A payment from «Mixed» is not sent because of its excess (FEAT-0030 АРХ-9а, AC-A12): with
    /// `suggested`, the excess is over the bound and `suggested` is an amount the same coins pay
    /// without one; without, the excess is not the one agreed to (a coin changed since the review)
    /// and `excess` is the one it would be now.
    MixedExcess {
        excess: Duffs,
        suggested: Option<Duffs>,
    },
}

/// Why no picture from outside is drawn on this system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PicturesOff {
    /// Linux without Landlock.
    NoFileIsolation,
    /// macOS would not take the memory cap.
    NoMemoryCap,
    /// The system would not set up the sandbox.
    NoSandbox,
}

/// Which operation an [`Event::Done`] answers. The screens match on it without a catch-all: a new
/// kind of answer has to be given a place (R-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoneWhat {
    AddressBook,
    App,
    /// `ReadAppBudgets`: the app's session keys were read (the new snapshot shows them).
    AppBudgets,
    /// `ForgetApp`: a disconnected app left the list.
    ForgetApp,
    Contact,
    CoreNode,
    Create,
    Delete,
    DeleteOther,
    Disconnect,
    Gift,
    Identity,
    Lock,
    Name,
    Note,
    Password,
    PasswordEachSend,
    PoolTopUp,
    Profile,
    Rescan,
    Resync,
    Send,
    SendToken,
    Shield,
    Switch,
    Token,
    TopUp,
    /// `FundAddress` / `ResumeFundAddress`: the Platform address funded from Core.
    FundAddress,
    Unlock,
    Unshield,
    Wallet,
}

impl DoneWhat {
    /// Its name in the diagnostic journal.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AddressBook => "address-book",
            Self::App => "app",
            Self::AppBudgets => "app-budgets",
            Self::ForgetApp => "forget-app",
            Self::Contact => "contact",
            Self::CoreNode => "core-node",
            Self::Create => "create",
            Self::Delete => "delete",
            Self::DeleteOther => "delete-other",
            Self::Disconnect => "disconnect",
            Self::Gift => "gift",
            Self::Identity => "identity",
            Self::Lock => "lock",
            Self::Name => "name",
            Self::Note => "note",
            Self::Password => "password",
            Self::PasswordEachSend => "password-each-send",
            Self::PoolTopUp => "pool-top-up",
            Self::Profile => "profile",
            Self::Rescan => "rescan",
            Self::Resync => "resync",
            Self::Send => "send",
            Self::SendToken => "send-token",
            Self::Shield => "shield",
            Self::Switch => "switch",
            Self::Token => "token",
            Self::TopUp => "top-up",
            Self::FundAddress => "fund-address",
            Self::Unlock => "unlock",
            Self::Unshield => "unshield",
            Self::Wallet => "wallet",
        }
    }
}

impl fmt::Display for DoneWhat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug)]
pub enum Event {
    Snapshot(Box<Snapshot>),
    NewPhrase(Phrase),
    /// `scan` is filled for a DashPay import.
    PhraseAccepted(Result<Option<DashPayScan>, CoreError>),
    NameResolved {
        name: String,
        result: Result<ResolvedName, CoreError>,
    },
    /// The result of `GiftNft`, `DisconnectApp`, `CreateWallet`, `CreateIdentity`,
    /// `Shield`, `AnswerApp`.
    Done {
        what: DoneWhat,
        result: Result<(), CoreError>,
    },
    PhraseRevealed(Result<Phrase, CoreError>),
    /// The answer to `Send`, once, after the network's answer (up to about a minute).
    /// The answer to the payment `id` of [`Spend::Send`].
    Sent {
        id: u64,
        result: Result<SentPayment, CoreError>,
    },
    /// The answer to `DrawArt`. `partial`: the first frame of an animation, the whole of it
    /// comes in a later `Art`.
    Art {
        art: String,
        side: u32,
        partial: bool,
        result: Result<Arc<Picture>, CoreError>,
    },
    NameChecked {
        label: String,
        result: Result<NameCheck, CoreError>,
    },
    /// How far a long job has got (`what` as in `Done`), 0..=1.
    Progress {
        what: &'static str,
        fraction: f32,
    },
    AppRequest(Result<AppRequest, CoreError>),
    /// The answer to `PreviewPayment`: the Core address the payment to `to` will go to.
    PaymentPreview {
        to: String,
        result: Result<String, CoreError>,
    },
    /// The answer to `ListCoins` (FEAT-0013): the coins a Core payment may be made of.
    Coins {
        ask: u64,
        result: Result<Vec<Coin>, CoreError>,
    },
    /// The answer to `PreviewMixed` (FEAT-0030).
    MixedPreview {
        ask: u64,
        result: Result<MixedQuote, CoreError>,
    },
    /// The answer to `AskPhraseCheck`: the words to pick, or why not (a wrong password).
    PhraseCheck(Result<Vec<PhraseQuestion>, CoreError>),
    /// The answer to `CheckPhraseWords`: `None` all right, else the number of the first word
    /// that did not match (the right word is not told).
    PhraseChecked {
        wrong: Option<usize>,
    },
    /// The answer to [`Command::FindToken`] or [`Command::SearchTokens`]: the tokens found for
    /// `query` (the id or the keyword asked), each read with a proof; empty: none.
    TokenFound {
        query: String,
        result: Result<Vec<tokens::TokenInfo>, CoreError>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-55: the fee of a Core payment is of the coins it takes; with no coins known, the
    /// fee of spending them all.
    #[test]
    fn a_core_payment_costs_the_coins_it_takes() {
        let coin = |value, fee| CoreCoin { value, fee };
        let mut fees = Fees {
            core_payment: 522,
            core_unsignable: 0,
            core_coins: Vec::new(),
            credit_transfer: 0,
            credit_transfer_to_address: 0,
            pool_transfer: 0,
            nft_transfer: 0,
            document_write: 0,
            shield: 0,
            identity_create: 0,
            identity_lock_min: 0,
            top_up_lock_min: 50_500,
            address_fund_on_top: 0,
            address_fund_lock_min: 0,
            name_regular: 0,
            name_contested: 0,
            add_key: 0,
            contact_request: 0,
            contact_request_balance: 0,
            profile: 0,
            shield_from_address: 0,
            shield_reserve: 0,
            unshield: 0,
            pool_withdrawal: 0,
            shield_from_core: 0,
            pool_top_up: 0,
            credit_withdrawal: 0,
            credit_withdrawal_reserve: 0,
        };
        assert_eq!(fees.core_payment_for(1_000), 522, "no coins known");
        fees.core_coins = vec![coin(1_000, 226), coin(500, 374), coin(300, 522)];
        assert_eq!(fees.core_payment_for(0), 226);
        assert_eq!(fees.core_payment_for(774), 226, "the coin and its fee");
        // BUG-93: the amount becomes credits whole; every fee is on top of it.
        fees.identity_lock_min = 241_000;
        fees.identity_create = 175_000;
        fees.name_regular = 65_000;
        assert_eq!(fees.name_on_top(), 90_000, "at least the name's margin");
        assert_eq!(fees.identity_fees_on_top(true), 175_000 + 90_000);
        assert_eq!(fees.identity_fees_on_top(false), 175_000);
        assert_eq!(fees.identity_lock_for(1_000_000, true), 1_265_000);
        assert_eq!(fees.identity_lock_for(1_000_000, false), 1_175_000);
        // BUG-97: what a name wants on the identity before it starts, less the margin on top.
        assert_eq!(fees.name_credits_needed(), 40_000 + 125_000);
        assert_eq!(fees.identity_amount_least(true), 165_000 - 90_000);
        // Without a name: what makes the lock reach Platform's minimum.
        assert_eq!(fees.identity_amount_least(false), 241_000 - 175_000);
        fees.name_regular = 100_000;
        assert_eq!(fees.name_on_top(), 100_000, "a dearer last name is kept");
        // A dearer last name raises the bar as well: 40 000 + 125 000 is still more than 140 000.
        assert_eq!(fees.name_credits_needed(), 165_000);
        assert_eq!(fees.identity_amount_least(true), 165_000 - 100_000);
        fees.name_regular = 200_000;
        assert_eq!(fees.name_credits_needed(), 40_000 + 200_000);
        fees.name_regular = 100_000;
        fees.name_regular = 1_000;
        assert_eq!(
            fees.identity_amount_least(true),
            165_000 - 90_000,
            "a cheap last name does not lower the bar"
        );
        // BUG-97, pass 12: the credits of attempts 1 and 2 did not finish, the third did.
        fees.name_regular = 57_979;
        assert!(95_285 < fees.name_credits_needed(), "attempt 1");
        assert!(102_105 < fees.name_credits_needed(), "attempt 2");
        assert!(180_406 >= fees.name_credits_needed(), "attempt 3");
        // The price: a first name at what a first name measured, a later one at the last.
        assert_eq!(fees.name_price(true), 78_000);
        assert_eq!(fees.name_price(false), 57_979);
        fees.name_regular = 100_000;
        assert_eq!(fees.name_price(true), 100_000, "a dearer last name is kept");
        fees.identity_lock_min = 1;
        fees.identity_create = 1_000_000;
        assert_eq!(fees.identity_amount_least(false), 1, "never below a duff");
        assert_eq!(fees.top_up_on_top(), 10_000);
        // BUG-93 review must 1: the lock of the least amount is Platform's minimum exactly.
        fees.top_up_lock_min = 50_500;
        assert_eq!(fees.top_up_amount_least(), 40_500);
        fees.top_up_lock_min = 9_000;
        assert_eq!(fees.top_up_amount_least(), 1, "never below a duff");

        assert_eq!(fees.core_payment_for(775), 374);
        assert_eq!(fees.core_payment_for(1_278), 522);
        assert_eq!(fees.core_payment_for(9_999), 522, "more than there is");
    }

    #[test]
    fn secrets_do_not_print() {
        let phrase = Phrase::new(vec!["abandon".into(), "about".into()]);
        let password = Password::new("hunter2".into());
        let printed = format!(
            "{phrase:?} {password:?} {:?}",
            Command::RevealPhrase {
                password: password.clone(),
                wallet: None,
            }
        );
        assert!(!printed.contains("abandon"));
        assert!(!printed.contains("hunter2"));
        let note = format!(
            "{:?}",
            Command::SetNote {
                key: "core:aa".into(),
                text: NoteText("For the logo".into()),
            }
        );
        assert!(!note.contains("logo") && !note.contains("12"));
        assert_eq!(phrase.words().len(), 2);
    }

    #[test]
    fn frames_unpack_to_what_was_packed_and_nothing_else() {
        let rgba: Vec<u8> = (0..16 * 16 * 4).map(|i| (i / 64) as u8).collect();
        let picture = Picture {
            width: 16,
            height: 16,
            frames: vec![PictureFrame::pack(&rgba, 50)],
        };
        assert!(picture.frames[0].packed_len() < rgba.len());
        let mut out = Vec::new();
        assert!(picture.unpack(0, &mut out));
        assert_eq!(out, rgba);
        assert!(!picture.unpack(1, &mut out));
        // A frame of another size does not pass for this one.
        let wrong = Picture {
            width: 32,
            ..picture
        };
        assert!(!wrong.unpack(0, &mut out));
    }
}
