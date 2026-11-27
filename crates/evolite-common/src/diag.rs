//! The diagnostic journal: what happened, for a problem
//! report, and nothing that is the person's. Records are typed — numbers, flags and names from
//! the code — so a phrase, a key, an address, an amount or a name has nowhere to go. The only
//! text is a panic's and a sync error's, and it goes through [`scrub()`] first.
//!
//! One JSON line per record in `logs/evolite-YYYY-MM-DD[.n].log` of the data folder; a file is
//! at most 2 MB, at most 5 files are kept, none older than 7 days. A thread writes them: no
//! frame and no core loop waits for the disk. A journal that cannot be written loses records,
//! never anything else.

mod core_events;
mod scrub;

pub use core_events::command_name;
pub use core_events::error_kind;
pub use core_events::{Watch, answer, command};
pub use scrub::{phrase_words, scrub};

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_FILES: usize = 5;
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What the app is and where it runs, at the start.
#[derive(Clone, Debug, PartialEq)]
pub struct Build {
    pub version: &'static str,
    pub commit: &'static str,
    /// `release` or `debug`.
    pub profile: &'static str,
    pub os: &'static str,
    pub os_version: String,
    pub language: &'static str,
    pub theme: &'static str,
    /// Points per pixel of the window, e.g. 1.5.
    pub scale: f32,
    pub network: &'static str,
    /// "Own node" is set: whether, never which.
    pub own_node: bool,
    pub demo: bool,
    /// SEC-8 (Пр3): the build's features that matter: `dev-tools`, or empty.
    pub features: &'static str,
    /// What `spv-network.log` holds (dev-tools builds): `network`, dash-spv's peer lines;
    /// `custom`, the filter `EVOLITE_SPV_LOG` gave (BUG-221 trace); `none` in other builds.
    pub spv_log: &'static str,
}

/// An amount in duffs as the journal writes it (BUG-216): two significant digits, the rest
/// zeros (125 342 101 → 120 000 000). The size stays for a report; the exact sum, which with
/// the time finds a transaction on the chain, does not.
pub fn rough(duffs: i64) -> i64 {
    let mut unit = 1i64;
    while (duffs / unit).abs() >= 100 {
        unit *= 10;
    }
    duffs / unit * unit
}

/// One record of the journal.
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    Start(Build),
    /// The sync's stage, heights and peers, when the stage or the number of peers changes.
    Sync {
        stage: &'static str,
        headers: u32,
        filters: u32,
        tip: u32,
        chainlocked: bool,
        peers: u32,
    },
    /// TECH-spv-prune: the SPV client started: opening its storage and replaying the stored
    /// masternode messages into the engine took this long, with this many messages; the newest
    /// list's quorums after the replay, verified and not (review r1 S2).
    SpvStart {
        storage_ms: u64,
        load_engine_ms: u64,
        messages: u32,
        verified: u32,
        unverified: u32,
    },
    /// TECH-spv-prune review r1 S2: the newest list's quorums once the first masternode sync
    /// of the run completed, verified and not.
    MnQuorums {
        verified: u32,
        unverified: u32,
    },
    /// TECH-spv-prune: stored masternode messages the replay no longer needs were deleted.
    MnPruned {
        count: u32,
    },
    /// A new sync error, scrubbed.
    SyncError {
        text: String,
    },
    /// A command went to the core.
    Command {
        name: &'static str,
    },
    /// The core answered: what, and `ok` or the kind of error.
    Answer {
        what: &'static str,
        outcome: &'static str,
    },
    Vault {
        state: &'static str,
    },
    /// A picture the renderer did not draw, and why.
    Art {
        reason: &'static str,
    },
    Screen {
        name: &'static str,
    },
    /// BUG-71: in the first second of a screen, a kind of input that can scroll it came. Both are
    /// names fixed in the code: the screen's and the input's.
    ScrollMoved {
        screen: &'static str,
        cause: &'static str,
    },
    /// BUG-71: in its first second, before the person scrolled it, a screen's main area left its
    /// top by `px` points. The screen's name and a number.
    ScrollShift {
        screen: &'static str,
        px: u32,
    },
    SlowFrame {
        ms: u64,
    },
    /// A step of the core ended after a long time (`real/turn.rs`): its name, fixed in the
    /// code, and how long it took.
    SlowStep {
        step: &'static str,
        ms: u64,
    },
    /// A step of the core is past the limit and has not come back: how long so far.
    StuckStep {
        step: &'static str,
        ms: u64,
    },
    /// A core file that did not save: its name only, what was done and the I/O error's kind.
    Storage {
        file: String,
        op: &'static str,
        error: &'static str,
    },
    /// A Platform operation failed: what it was and the error's kind (its type names only,
    /// as `Sdk/Protocol`), never its text, ids or amounts.
    Platform {
        op: &'static str,
        error: String,
    },
    /// BUG-323: a proved read of an App Connect request failed: its stage (`group`, `contract`,
    /// `enc_keys`), the error's kind (type names only) and the protocol version the SDK saw —
    /// never the id or the error's text.
    ConnectRead {
        stage: &'static str,
        error: String,
        protocol: Option<u32>,
    },
    /// BUG-350 (review nit): an app's spending is not known because one of its session keys has
    /// no read budget — so QA tells it from «the app spent nothing». Once a run per key; the key's
    /// number only, no amount, no app, no identity.
    AppSpentUnknown {
        key: u32,
    },
    /// A panic: where, and its message and stack, scrubbed.
    Panic {
        location: String,
        message: String,
        backtrace: String,
    },
    /// The window did not open (no graphics adapter, a driver error): the error, scrubbed.
    StartFailed {
        text: String,
    },
    /// The last run ended without closing its window (a crash, a reset): the saved window
    /// size and placement were dropped, so a maximized window does not open again at once.
    WindowReset,
    /// BUG-231 R-6: a payment to a contact was refused before the password because its channel
    /// is broken (its request points to a disabled key). No name, no id.
    ContactChannelBroken,
    /// Р1: an update check, by what came of it (`newer`, `same`, or why nothing was believed:
    /// `no_keys`, `unreachable`, `bad_signature`, `bad_file`, `replay`) and the date of the
    /// file believed. No URL, no text from the network.
    UpdateCheck {
        result: &'static str,
        date: Option<String>,
    },
    /// SEC-7: the warning before the phrase was shown on Linux, for this kind of session:
    /// `x11`, `wayland` or `unknown`. Never a word of the phrase.
    PhraseWarning {
        session: &'static str,
    },
    /// What the load did with a wallet's saved DashPay backfill record (BUG-77): `kept`, `none`
    /// or `rejected_*` and why, and how many contacts the record taken back covers.
    Backfill {
        outcome: &'static str,
        contacts: u32,
    },
    /// The store's spent coins claimed at the load (BUG-77): claimed, spent but with no spender
    /// in a block (not claimed), and claimed but still among the wallet's coins.
    SpentClaims {
        claimed: u32,
        not_in_block: u32,
        still_held: u32,
    },
    /// The wallet's coins by kind of account (BUG-150): `when` at the unlock before the network and
    /// after the first filter pass; how many and their sum in duffs. Numbers only.
    Coins {
        when: &'static str,
        bip44: (u32, u64),
        bip32: (u32, u64),
        dashpay: (u32, u64),
        coinjoin: (u32, u64),
        /// After the unlock's move of contacts' coins out of account 0 (`unlock_rehomed`, BUG-148):
        /// how many it moved. None on the other lines.
        moved: Option<u32>,
    },
    /// A page of a token's history read (FEAT-0019 T-1.8): which side (`to`, `from`), why
    /// (`moved`: a balance moved; `asked`: History asked) and how many documents it had. Only
    /// counts: no token, identity, document id or amount.
    TokenHistory {
        side: &'static str,
        why: &'static str,
        docs: u32,
    },
    /// A rescan asked for the contacts' receiving accounts (BUG-221, SYNC-7): for how many, and
    /// from which height (where the earliest of their contacts begins, else the birth).
    DashpayRescan {
        accounts: u32,
        from_height: u32,
    },
    /// A rescan from the birth for the contacts' receiving accounts ran to its end (BUG-150): for
    /// how many accounts, how many history records it brought, how long it took.
    RescanDone {
        accounts: u32,
        found: u32,
        secs: u64,
    },
    /// BUG-232 review r1 M1: the rescan after addresses were taken in past the gap is done; how
    /// many coins of outputs once read as paid out the wallet has, written to the store.
    PastTheGapCoins {
        coins: u32,
    },
    /// BUG-232: outputs paid to addresses of ours past the pools' gap were taken in: how many,
    /// and the BIP44 account 0's change pool after it, its highest used and highest made index
    /// (counts only, no address).
    PastTheGap {
        adopted: u32,
        change_used: u32,
        change_made: u32,
    },
    /// A spend's price (TECH-price-check): the kind of spend, what the form promised, what was
    /// taken (`None`: not known, or refused), whether the balance covered the promise, and `ok` or
    /// the refusal's code; and the balance the core judged it by when it read one anew (`held`,
    /// BUG-141: the form may have planned from a balance the core then proved smaller). Only the
    /// price and that balance: no address, name, amount sent or text of the network.
    Price {
        what: &'static str,
        declared: evolite_api::Duffs,
        actual: Option<evolite_api::Duffs>,
        enough: bool,
        reason: &'static str,
        held: Option<evolite_api::Duffs>,
    },
    /// SPV item 2a: no new block for `minutes` (by both clocks), once on entering that state.
    SyncStale {
        minutes: u32,
    },
    /// SPV item 3 (TECH-finality-journal): a build of platform-wallet's own (`asset_lock` or
    /// `contact_pay`) began with the coins not final frozen — how many coins were in a block,
    /// InstantSend-locked by a lock verified in this run, locked by one not verified, and how many
    /// it froze. Counts of coins only: no txid, no amount.
    FinalCoins {
        build: &'static str,
        confirmed: u32,
        is_verified: u32,
        is_unverified: u32,
        frozen: u32,
    },
    /// The InstantLock of one of the wallet's own transactions was verified in this run, so long
    /// after the unlock (TECH-finality-journal). No txid.
    OwnIslockVerified {
        since_unlock_s: u64,
    },
    /// At the unlock: how many of the wallet's InstantLocks were kept from before (BUG-173), each
    /// read «Confirmed · checking» until verified again (TECH-finality-journal).
    KeptIslocks {
        count: u32,
    },
    /// BUG-260: the CoinJoin account's denominations by their rounds — counts of coins only, no
    /// amounts and no outpoints — written when they change: a «Mixed» lower than another wallet's
    /// is seen without a copy of the store.
    MixedRounds {
        none: u32,
        some: u32,
        full: u32,
        unknown: u32,
    },
    /// «Features for advanced users» turned on or off (FEAT-0040…0046): only that, as 1 or 0.
    AdvancedFeatures {
        on: bool,
    },
    /// Whether the open wallet's phrase and seed are locked to RAM, out of the page file
    /// (SEC-5, `evolite-memlock`): once a run; when not, why (`quota` or `other`).
    Memlock {
        ok: bool,
        error: &'static str,
    },
    /// The first fresh picture of an area since the unlock (the sync-speed plan §4, SYNC-0):
    /// `what` (identities, balances, names, contacts_named, token_rows, nfts, history, pool),
    /// seconds since the unlock, and from this computer's store or the network ([`fresh`]).
    Fresh {
        what: &'static str,
        since_unlock_s: u64,
        from: &'static str,
    },
    /// Platform's proved reads may go, so long after the unlock; by quorums from the disk or
    /// the network (SYNC-0). Once per unlock.
    PlatformReady {
        since_unlock_s: u64,
        quorums: &'static str,
    },
    /// A pass reading the Platform history: identities, queries sent, how long, and how many
    /// identities failed (SYNC-0). Counts only.
    HistoryPass {
        identities: u32,
        requests: u32,
        ms: u64,
        failed: u32,
    },
    /// Contacts' names read in one go: asked of the network, shown from the store, how long.
    ContactNames {
        asked: u32,
        cached: u32,
        ms: u64,
    },
    /// FEAT-0020-resend: a batch of contact requests sent again stopped — numbers only, no ids
    /// or names (AC-T9).
    ContactResend {
        asked: u32,
        sent: u32,
        already: u32,
        failed: u32,
        version_full: u32,
        unclear: u32,
    },
    /// BUG-240: one read of the shielded pool, when it differs from the last one written —
    /// numbers only, no address or note: the ledger's state (`ready`, `restore_incomplete`,
    /// `unbound`, `error`), whether the wallet's account is among its accounts, its spendable
    /// credits (rough, BUG-216), the unspent notes read from the store and their sum (rough), and
    /// whether those came fresh or, the store busy, from the last read (`notes`: `fresh` /
    /// `cache`), the last scanned index and where the balance comes from (`scanned`,
    /// `restored`, `none`).
    PoolRead {
        ledger: &'static str,
        has_account: bool,
        spendable: Option<u64>,
        unspent: Option<u32>,
        unspent_sum: Option<u64>,
        last_scanned: Option<u64>,
        source: &'static str,
        notes: &'static str,
    },
    /// BUG-240 r2: a pass of the pool's background scan ended — how many wallets it scanned
    /// (`ok`), skipped and failed, the notes it found and the entries it scanned, and the kind of
    /// a failure (`timeout`, `not_ready`, `quorum_wait`, `proof`, `network`, `other`; never its
    /// words).
    /// BUG-240 r3: `repeated` — how many more passes ended the same way after this line was
    /// first written (written again with it when the outcome changes; 0 on the first).
    /// BUG-240 r4: `stale_retries` — the SDK's retries for a node behind since the last pass
    /// (all Platform reads in that time, the pass's mostly); `None` where they are not counted
    /// (builds without dev-tools install no `tracing` subscriber).
    PoolPass {
        ok: u32,
        skipped: u32,
        failed: u32,
        new_notes: u32,
        scanned: u64,
        error: &'static str,
        repeated: u32,
        stale_retries: Option<u32>,
    },
    /// The token rows' facts read: tokens, queries, how long (SYNC-0).
    TokenFacts {
        tokens: u32,
        requests: u32,
        ms: u64,
    },
    /// A token's page read, how long (SYNC-0).
    TokenPage {
        ms: u64,
    },
    /// A funding of the Platform address went to Platform (BUG-171, QA's AC-T5): by which proof
    /// of its lock (`instant` or `chain`), the duffs locked and what the address gained (`None`:
    /// not known, or refused) — what Platform kept is the difference. Kinds and amounts only:
    /// no txid, no address.
    AddressFund {
        proof: &'static str,
        locked: evolite_api::Duffs,
        credited: Option<evolite_api::Duffs>,
    },
    /// FEAT-0048 r1 Should 1: an identity created from the pool: the exit (one of the network's
    /// fixed amounts) and the fee Platform took from it, exact — the shown fee is tuned by
    /// these. No id, no name.
    IdentityFromPoolFee {
        exit: evolite_api::Duffs,
        fee: evolite_api::Duffs,
    },
    /// A stage of the token search ended (BUG-136, the architect's measure): its name, fixed in
    /// the code (`pages`, `ids`, `versions`, `contracts`, `balances`), its requests and its ms.
    /// Counts only: no id, no token, no amount.
    TokenSearchStage {
        stage: &'static str,
        requests: u32,
        ms: u64,
    },
    /// A «Workshop» series run ended (FEAT-0038): its writes and how they stand, and its ms.
    /// Numbers only: no id, no serial.
    SeriesEnded {
        total: u32,
        done: u32,
        refused: u32,
        unclear: u32,
        ms: u64,
    },
    /// The token search changed the size of its requests with contracts' bodies (architect's
    /// decision on batch 39): the new size and the ms of the request that changed it (a
    /// timeout makes it smaller, answers in a row larger). Counts only: no id.
    CrawlPage {
        size: u32,
        ms: u64,
    },
    /// A pass of the token search ended: the network's contracts, the token ids among them, the
    /// identities whose balances were asked, and its ms. Counts only.
    TokenSearchEnd {
        contracts: u32,
        tokens: u32,
        identities: u32,
        ms: u64,
    },
    /// The network's contracts kept on this computer after a pass (FEAT-0031 C5): how many,
    /// and the bytes of their bodies. Counts only.
    ContractsCached {
        rows: u64,
        bytes: u64,
    },
    /// A contract registered by the wallet (FEAT-0038 module V1), for the fee's calibration:
    /// its document types, indexes and keywords, its size, the estimate and what was charged
    /// (duffs; `None` when the read after it failed), the protocol, the ms, and whether it was
    /// found after a lost answer. Numbers only: no contract id, no owner.
    ContractRegistered {
        types: u32,
        indexes: u32,
        keywords: u32,
        bytes: u64,
        estimate: u64,
        charged: Option<u64>,
        protocol: u32,
        ms: u64,
        recovered: bool,
    },
    /// A name search for the suggestions ended (FEAT-0039, AC-A8): how many names, and whether
    /// they came from the session's cache without asking the network. Never the text typed,
    /// the names or the pictures' addresses.
    NameSearch {
        found: u32,
        cached: bool,
    },
}

impl Record {
    /// The JSON line, without the time.
    fn fields(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Record::Start(b) => json!({
                "k": "start", "version": b.version, "commit": b.commit, "profile": b.profile,
                "os": b.os, "osVersion": b.os_version, "language": b.language,
                "theme": b.theme, "scale": b.scale, "network": b.network,
                "ownNode": b.own_node, "demo": b.demo, "features": b.features,
                "spvLog": b.spv_log,
            }),
            Record::Sync {
                stage,
                headers,
                filters,
                tip,
                chainlocked,
                peers,
            } => json!({
                "k": "sync", "stage": stage, "headers": headers, "filters": filters,
                "tip": tip, "chainlocked": chainlocked, "peers": peers,
            }),
            Record::SyncError { text } => json!({"k": "sync_error", "text": scrub(text)}),
            Record::Command { name } => json!({"k": "command", "name": name}),
            Record::Answer { what, outcome } => {
                json!({"k": "answer", "what": what, "outcome": outcome})
            }
            Record::Vault { state } => json!({"k": "vault", "state": state}),
            Record::Art { reason } => json!({"k": "art", "reason": reason}),
            Record::Screen { name } => json!({"k": "screen", "name": name}),
            Record::ScrollMoved { screen, cause } => {
                json!({"k": "scroll_moved", "screen": screen, "cause": cause})
            }
            Record::ScrollShift { screen, px } => {
                json!({"k": "scroll_shift", "screen": screen, "px": px})
            }
            Record::SlowFrame { ms } => json!({"k": "slow_frame", "ms": ms}),
            Record::SlowStep { step, ms } => json!({"k": "slow_step", "step": step, "ms": ms}),
            Record::StuckStep { step, ms } => {
                json!({"k": "stuck_step", "step": step, "ms": ms})
            }
            Record::Storage { file, op, error } => {
                json!({"k": "storage", "file": scrub(file), "op": op, "error": error})
            }
            Record::Panic {
                location,
                message,
                backtrace,
            } => json!({
                "k": "panic", "location": scrub(location), "message": scrub(message),
                "backtrace": scrub(backtrace),
            }),
            Record::StartFailed { text } => json!({"k": "start_failed", "text": scrub(text)}),
            Record::SpvStart {
                storage_ms,
                load_engine_ms,
                messages,
                verified,
                unverified,
            } => json!({
                "k": "spv_start", "storageMs": storage_ms, "loadEngineMs": load_engine_ms,
                "messages": messages, "verified": verified, "unverified": unverified,
            }),
            Record::MnQuorums {
                verified,
                unverified,
            } => json!({"k": "mn_quorums", "verified": verified, "unverified": unverified}),
            Record::MnPruned { count } => json!({"k": "mn_pruned", "count": count}),
            Record::Platform { op, error } => {
                json!({"k": "platform", "op": op, "error": error})
            }
            Record::ConnectRead {
                stage,
                error,
                protocol,
            } => json!({"k": "connect_read", "stage": stage, "error": error, "protocol": protocol}),
            Record::AppSpentUnknown { key } => json!({"k": "app_spent_unknown", "key": key}),
            Record::WindowReset => json!({"k": "window_reset"}),
            Record::ContactChannelBroken => json!({"k": "contact_channel_broken"}),
            Record::UpdateCheck { result, date } => {
                json!({"k": "update_check", "result": result, "date": date})
            }
            Record::PhraseWarning { session } => {
                json!({"k": "phrase_warning", "session": session})
            }
            Record::Backfill { outcome, contacts } => {
                json!({"k": "backfill", "outcome": outcome, "contacts": contacts})
            }
            Record::SpentClaims {
                claimed,
                not_in_block,
                still_held,
            } => json!({
                "k": "spent_claims", "claimed": claimed, "notInBlock": not_in_block,
                "stillHeld": still_held,
            }),
            Record::Coins {
                when,
                bip44,
                bip32,
                dashpay,
                coinjoin,
                moved,
            } => {
                // Sums rough (BUG-216): an exact one finds the wallet's coins on the chain.
                let mut fields = json!({
                    "k": "coins", "when": when,
                    "bip44": [bip44.0, rough(bip44.1 as i64)], "bip32": [bip32.0, rough(bip32.1 as i64)],
                    "dashpay": [dashpay.0, rough(dashpay.1 as i64)],
                    "coinjoin": [coinjoin.0, rough(coinjoin.1 as i64)],
                });
                if let Some(moved) = moved {
                    fields["moved"] = (*moved).into();
                }
                fields
            }
            Record::TokenHistory { side, why, docs } => {
                json!({"k": "token_history", "side": side, "why": why, "docs": docs})
            }
            Record::DashpayRescan {
                accounts,
                from_height,
            } => json!({"k": "dashpay_rescan", "accounts": accounts, "from_height": from_height}),
            Record::PastTheGap {
                adopted,
                change_used,
                change_made,
            } => json!({
                "k": "past_the_gap", "adopted": adopted, "changeUsed": change_used,
                "changeMade": change_made,
            }),
            Record::PastTheGapCoins { coins } => json!({
                "k": "past_the_gap_coins", "coins": coins,
            }),
            Record::RescanDone {
                accounts,
                found,
                secs,
            } => json!({
                "k": "rescan_done", "why": "contact_account", "accounts": accounts,
                "found": found, "secs": secs,
            }),
            Record::Price {
                what,
                declared,
                actual,
                enough,
                reason,
                held,
            } => json!({
                // The balance rough (BUG-216); the price itself is a fee.
                "k": "price", "what": what, "declared": declared, "actual": actual,
                "enough": enough, "reason": reason, "held": (*held).map(rough),
            }),
            Record::SyncStale { minutes } => json!({ "k": "sync_stale", "minutes": minutes }),
            Record::FinalCoins {
                build,
                confirmed,
                is_verified,
                is_unverified,
                frozen,
            } => json!({
                "k": "final_coins", "build": build, "confirmed": confirmed,
                "is_verified": is_verified, "is_unverified": is_unverified, "frozen": frozen,
            }),
            Record::OwnIslockVerified { since_unlock_s } => {
                json!({"k": "own_islock_verified", "since_unlock_s": since_unlock_s})
            }
            Record::KeptIslocks { count } => json!({"k": "kept_islocks", "count": count}),
            Record::MixedRounds {
                none,
                some,
                full,
                unknown,
            } => json!({
                "k": "mixed_rounds", "none": none, "some": some, "full": full, "unknown": unknown,
            }),
            Record::AdvancedFeatures { on } => {
                json!({"k": "advanced_features", "on": u8::from(*on)})
            }
            Record::Memlock { ok, error } => {
                if *ok {
                    json!({"k": "memlock", "ok": true})
                } else {
                    json!({"k": "memlock", "ok": false, "error": error})
                }
            }
            Record::Fresh {
                what,
                since_unlock_s,
                from,
            } => {
                json!({"k": "fresh", "what": what, "since_unlock_s": since_unlock_s, "from": from})
            }
            Record::PlatformReady {
                since_unlock_s,
                quorums,
            } => {
                json!({"k": "platform_ready", "since_unlock_s": since_unlock_s, "quorums": quorums})
            }
            Record::HistoryPass {
                identities,
                requests,
                ms,
                failed,
            } => json!({
                "k": "history_pass", "identities": identities, "requests": requests, "ms": ms,
                "failed": failed,
            }),
            Record::ContactNames { asked, cached, ms } => {
                json!({"k": "contact_names", "asked": asked, "cached": cached, "ms": ms})
            }
            Record::TokenFacts {
                tokens,
                requests,
                ms,
            } => json!({"k": "token_facts", "tokens": tokens, "requests": requests, "ms": ms}),
            Record::ContactResend {
                asked,
                sent,
                already,
                failed,
                version_full,
                unclear,
            } => json!({
                "k": "contact_resend", "asked": asked, "sent": sent, "already": already,
                "failed": failed, "version_full": version_full, "unclear": unclear,
            }),
            Record::TokenPage { ms } => json!({"k": "token_page", "ms": ms}),
            Record::PoolPass {
                ok,
                skipped,
                failed,
                new_notes,
                scanned,
                error,
                repeated,
                stale_retries,
            } => json!({
                "k": "pool_pass", "ok": ok, "skipped": skipped, "failed": failed,
                "newNotes": new_notes, "scanned": scanned, "error": error, "repeated": repeated,
                "staleRetries": stale_retries,
            }),
            Record::PoolRead {
                ledger,
                has_account,
                spendable,
                unspent,
                unspent_sum,
                last_scanned,
                source,
                notes,
            } => json!({
                "k": "pool_read", "ledger": ledger, "hasAccount": has_account,
                "spendable": spendable.map(|c| rough((c / 1000) as i64)),
                "unspent": unspent,
                "unspentSum": unspent_sum.map(|c| rough((c / 1000) as i64)),
                "lastScanned": last_scanned, "source": source, "notes": notes,
            }),
            Record::AddressFund {
                proof,
                locked,
                credited,
            } => json!({
                // The amounts rough (BUG-216); what Platform kept, a fee, exact.
                "k": "address_fund", "proof": proof, "locked": rough(*locked),
                "credited": (*credited).map(rough),
                "kept": (*credited).map(|c| locked.saturating_sub(c)),
            }),
            Record::IdentityFromPoolFee { exit, fee } => json!({
                "k": "identity_from_pool_fee", "exit": exit, "fee": fee,
            }),
            Record::SeriesEnded {
                total,
                done,
                refused,
                unclear,
                ms,
            } => json!({
                "k": "series_ended", "total": total, "done": done, "refused": refused,
                "unclear": unclear, "ms": ms,
            }),
            Record::TokenSearchStage {
                stage,
                requests,
                ms,
            } => json!({
                "k": "token_search_stage", "stage": stage, "requests": requests, "ms": ms,
            }),
            Record::CrawlPage { size, ms } => json!({ "k": "crawl_page", "size": size, "ms": ms }),
            Record::NameSearch { found, cached } => json!({
                "k": "name_search", "found": found, "cached": cached,
            }),
            Record::ContractsCached { rows, bytes } => json!({
                "k": "contracts_cached", "rows": rows, "bytes": bytes,
            }),
            Record::ContractRegistered {
                types,
                indexes,
                keywords,
                bytes,
                estimate,
                charged,
                protocol,
                ms,
                recovered,
            } => json!({
                "k": "contract_registered", "types": types, "indexes": indexes,
                "keywords": keywords, "bytes": bytes, "estimate": estimate, "charged": charged,
                "protocol": protocol, "ms": ms, "recovered": recovered,
            }),
            Record::TokenSearchEnd {
                contracts,
                tokens,
                identities,
                ms,
            } => json!({
                "k": "token_search_end", "contracts": contracts, "tokens": tokens,
                "identities": identities, "ms": ms,
            }),
        }
    }

    /// The line written: the time first, then the fields.
    fn line(&self, at: SystemTime) -> String {
        let mut fields = self.fields();
        let mut line = serde_json::Map::new();
        line.insert("t".into(), timestamp(at).into());
        if let Some(map) = fields.as_object_mut() {
            line.append(map);
        }
        serde_json::Value::Object(line).to_string()
    }
}

/// The kind of an I/O error, without its text (which may carry a path).
pub fn io_error_kind(error: &std::io::Error) -> &'static str {
    use std::io::ErrorKind::*;
    match error.kind() {
        NotFound => "not_found",
        PermissionDenied => "permission_denied",
        AlreadyExists => "already_exists",
        StorageFull => "storage_full",
        ReadOnlyFilesystem => "read_only",
        IsADirectory => "is_a_directory",
        InvalidData => "invalid_data",
        Interrupted => "interrupted",
        _ => "other",
    }
}

/// A Platform operation of the core that failed, by the kind of its error: `debug` is the
/// error's `{:?}` form, of which only the leading type names are kept.
pub fn platform_failed(op: &'static str, debug: &str) {
    record(Record::Platform {
        op,
        error: error_kind_of_debug(debug),
    });
}

/// `Sdk(Protocol(ConsensusError(..)))` → `Sdk/Protocol/ConsensusError`: up to three nested
/// variant names of a `{:?}` form, nothing of the data inside.
pub fn error_kind_of_debug(debug: &str) -> String {
    let mut names = Vec::new();
    let mut rest = debug.trim_start();
    while names.len() < 3 {
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_uppercase()) {
            break;
        }
        rest = &rest[name.len()..];
        names.push(name);
        match rest.strip_prefix('(') {
            Some(inner) => rest = inner.trim_start(),
            None => break,
        }
    }
    if names.is_empty() {
        "other".into()
    } else {
        names.join("/")
    }
}

/// A file of the core that did not save: its name only.
pub fn storage_failed(path: &Path, op: &'static str, error: &std::io::Error) {
    record(Record::Storage {
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        op,
        error: io_error_kind(error),
    });
}

/// `2026-11-01T12:00:00.123Z`.
fn timestamp(at: SystemTime) -> String {
    let d = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (y, m, day) = civil(days as i64);
    format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60,
        d.subsec_millis()
    )
}

/// The date of a day number since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

struct Journal {
    dir: PathBuf,
    lines: Mutex<Sender<String>>,
}

static JOURNAL: OnceLock<Journal> = OnceLock::new();

/// Starts the journal in `dir` (made if needed), removes files past the limits. Once per run;
/// later calls are ignored.
pub fn init(dir: &Path) {
    if JOURNAL.get().is_some() {
        return;
    }
    let _ = std::fs::create_dir_all(dir);
    prune(dir, SystemTime::now());
    let (tx, rx) = channel::<String>();
    let folder = dir.to_owned();
    let started = std::thread::Builder::new()
        .name("evolite-journal".into())
        .spawn(move || {
            let mut writer = Writer::new(folder);
            while let Ok(line) = rx.recv() {
                writer.write(&line);
            }
        });
    if started.is_ok() {
        let _ = JOURNAL.set(Journal {
            dir: dir.to_owned(),
            lines: Mutex::new(tx),
        });
    }
}

/// BUG-240 r4: the SDK's retries for a node behind since they were last taken.
static STALE_RETRIES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Whether anything counts them (a dev-tools build's `tracing` layer).
static STALE_COUNTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// BUG-240 r4: from now on the SDK's retries for a node behind are counted ([`stale_retry`]).
pub fn count_stale_retries() {
    STALE_COUNTED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// BUG-240 r4: the SDK asked another node, this one behind.
pub fn stale_retry() {
    STALE_RETRIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// BUG-240 r4: the retries for a node behind since the last call; `None` when not counted.
pub fn take_stale_retries() -> Option<u32> {
    STALE_COUNTED
        .load(std::sync::atomic::Ordering::Relaxed)
        .then(|| STALE_RETRIES.swap(0, std::sync::atomic::Ordering::Relaxed))
}

/// Keeps `record`, when the journal is on.
pub fn record(record: Record) {
    if let Some(journal) = JOURNAL.get() {
        let line = record.line(SystemTime::now());
        let _ = journal
            .lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(line);
    }
}

/// Writes `record` now, on this thread: for a panic, after which the writer may not run.
pub fn record_now(record: Record) {
    if let Some(journal) = JOURNAL.get() {
        let mut writer = Writer::new(journal.dir.clone());
        writer.write(&record.line(SystemTime::now()));
    }
}

/// The journal's folder, when it is on.
pub fn dir() -> Option<PathBuf> {
    JOURNAL.get().map(|j| j.dir.clone())
}

/// The journal's files, oldest first.
pub fn files() -> Vec<PathBuf> {
    dir().map(|d| journal_files(&d)).unwrap_or_default()
}

/// "Clear journal": every file goes; what comes next starts a new one.
pub fn clear() -> std::io::Result<()> {
    for file in files() {
        std::fs::remove_file(file)?;
    }
    Ok(())
}

/// The day and number of a journal file (`evolite-2026-11-01.3.log` → ("2026-11-01", 3)), or
/// `None` for any other file.
fn order(path: &Path) -> Option<(String, u32)> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_prefix("evolite-")?.strip_suffix(".log")?;
    let (day, n) = match stem.split_once('.') {
        Some((day, n)) => (day, n.parse().ok()?),
        None => (stem, 0),
    };
    (day.len() == 10).then(|| (day.to_owned(), n))
}

/// The journal files in `dir`, oldest first.
fn journal_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<((String, u32), PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| Some((order(&p)?, p)))
        .collect();
    files.sort();
    files.into_iter().map(|(_, p)| p).collect()
}

/// Removes files older than [`MAX_AGE`] and all but the newest [`MAX_FILES`].
fn prune(dir: &Path, now: SystemTime) {
    let files = journal_files(dir);
    let excess = files.len().saturating_sub(MAX_FILES);
    for (i, file) in files.iter().enumerate() {
        let old = std::fs::metadata(file)
            .and_then(|m| m.modified())
            .is_ok_and(|t| now.duration_since(t).unwrap_or_default() > MAX_AGE);
        if i < excess || old {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// Appends lines to today's file, moving to the next one past [`MAX_FILE`].
struct Writer {
    dir: PathBuf,
    current: Option<PathBuf>,
}

impl Writer {
    fn new(dir: PathBuf) -> Self {
        Self { dir, current: None }
    }

    /// Today's file with room for `len` more bytes: the last one of today, else the next.
    fn file_for(&mut self, len: u64) -> PathBuf {
        let day = timestamp(SystemTime::now())[..10].to_owned();
        let fits = |p: &Path| std::fs::metadata(p).map_or(0, |m| m.len()) + len <= MAX_FILE;
        let today = |p: &Path| order(p).is_some_and(|(d, _)| d == day);
        if self.current.as_deref().is_none_or(|p| !today(p)) {
            // At the start, or past midnight: today's last file, if there is one.
            self.current = journal_files(&self.dir).into_iter().rfind(|p| today(p));
        }
        if let Some(current) = &self.current
            && fits(current)
        {
            return current.clone();
        }
        let next = self
            .current
            .as_deref()
            .and_then(order)
            .map_or(0, |(_, n)| n + 1);
        let path = self.dir.join(if next == 0 {
            format!("evolite-{day}.log")
        } else {
            format!("evolite-{day}.{next}.log")
        });
        // Made before the limits are applied, so they count it.
        let _ = std::fs::File::create(&path);
        prune(&self.dir, SystemTime::now());
        self.current = Some(path.clone());
        path
    }

    fn write(&mut self, line: &str) {
        let path = self.file_for(line.len() as u64 + 1);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// The unlock's clock and the (area, source) pairs said since (SYNC-0): one process, one open
/// wallet at a time.
/// The unlock's instant and the (area, source) pairs said since.
type FreshState = Option<(
    std::time::Instant,
    std::collections::HashSet<(&'static str, &'static str)>,
)>;

static FRESH: std::sync::Mutex<FreshState> = std::sync::Mutex::new(None);

/// A wallet was unlocked or opened (switched to): the clock of [`fresh`] starts again and every
/// area may be said once more.
pub fn unlocked() {
    *FRESH.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((std::time::Instant::now(), Default::default()));
}

/// The first fresh picture of `what` from `from` (`store` | `network`) since the unlock: the
/// journal's `fresh` line, once per area and source until the next unlock (the sync-speed plan).
pub fn fresh(what: &'static str, from: &'static str) {
    fresh_once(what, from);
}

/// [`fresh`], saying whether this call wrote the line.
fn fresh_once(what: &'static str, from: &'static str) -> bool {
    let since = {
        let mut state = FRESH.lock().unwrap_or_else(|e| e.into_inner());
        let Some((at, said)) = state.as_mut() else {
            return false;
        };
        if !said.insert((what, from)) {
            return false;
        }
        at.elapsed().as_secs()
    };
    record(Record::Fresh {
        what,
        since_unlock_s: since,
        from,
    });
    true
}

/// Platform's proved reads may go now (SYNC-0): `quorums` from the `disk` or the `network`. Once
/// per unlock.
pub fn platform_ready(quorums: &'static str) {
    let since = {
        let mut state = FRESH.lock().unwrap_or_else(|e| e.into_inner());
        let Some((at, said)) = state.as_mut() else {
            return;
        };
        if !said.insert(("platform_ready", quorums)) {
            return;
        }
        at.elapsed().as_secs()
    };
    record(Record::PlatformReady {
        since_unlock_s: since,
        quorums,
    });
}

/// The tests of [`fresh`] share its one clock: they take turns.
#[cfg(test)]
fn fresh_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    TURN.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-323: a failed read of an App Connect request is journaled as its stage, the error's
    /// kind and the protocol version — never the group's or contract's id, never the text.
    #[test]
    fn a_connect_read_is_its_stage_kind_and_protocol_only() {
        let line = Record::ConnectRead {
            stage: "contract",
            error: error_kind_of_debug("Sdk(Protocol(DecodingError(\"8mjmcXmf bad\")))"),
            protocol: Some(11),
        }
        .line(UNIX_EPOCH);
        assert!(
            line.contains(r#""k":"connect_read","stage":"contract","error":"Sdk/Protocol/DecodingError","protocol":11"#),
            "{line}"
        );
        assert!(!line.contains("8mjm") && !line.contains("bad"), "{line}");
    }

    /// BUG-350: an app's spending not known is journaled as the key's number only.
    #[test]
    fn an_unknown_apps_spending_is_the_key_number_only() {
        let line = Record::AppSpentUnknown { key: 9 }.line(UNIX_EPOCH);
        assert!(line.contains(r#""k":"app_spent_unknown","key":9"#), "{line}");
    }

    /// FEAT-0019 T-1.8: a page of a token's history is journaled as counts only — no token,
    /// identity, document id or amount can be in its line.
    #[test]
    fn a_token_history_page_is_counts_only() {
        let line = Record::TokenHistory {
            side: "to",
            why: "moved",
            docs: 3,
        }
        .fields();
        assert_eq!(
            line,
            serde_json::json!({"k": "token_history", "side": "to", "why": "moved", "docs": 3})
        );
    }

    #[test]
    fn a_platform_error_keeps_its_type_names_only() {
        assert_eq!(
            error_kind_of_debug("AssetLockAlreadyConsumed(OutPoint { txid: 0a1b, vout: 0 })"),
            "AssetLockAlreadyConsumed/OutPoint"
        );
        assert_eq!(
            error_kind_of_debug("Sdk(Protocol(ConsensusError(Basic(X))))"),
            "Sdk/Protocol/ConsensusError"
        );
        assert_eq!(
            error_kind_of_debug("Sdk(Generic(\"yJ1kAddress 12 DASH\"))"),
            "Sdk/Generic"
        );
        assert_eq!(error_kind_of_debug("\"text\""), "other");
        let line = Record::Platform {
            op: "top_up",
            error: "Sdk/Generic".into(),
        }
        .line(UNIX_EPOCH);
        assert!(line.contains(r#""k":"platform","op":"top_up","error":"Sdk/Generic""#));
    }

    #[test]
    fn writes_the_time_and_typed_fields_only() {
        let at = UNIX_EPOCH + Duration::from_millis(1_761_998_400_123);
        let line = Record::Answer {
            what: "done:send",
            outcome: "not_enough",
        }
        .line(at);
        assert_eq!(
            line,
            r#"{"t":"2025-11-01T12:00:00.123Z","k":"answer","what":"done:send","outcome":"not_enough"}"#
        );
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    /// BUG-148: the coins line after the unlock's move carries how many moved; the other coins
    /// lines read as before, without it. Numbers only.
    #[test]
    fn a_coins_line_says_how_many_moved_only_after_the_move() {
        let coins = |when, moved| Record::Coins {
            when,
            bip44: (1, 5_000_000),
            bip32: (0, 0),
            dashpay: (1, 100_000),
            coinjoin: (0, 0),
            moved,
        };
        assert_eq!(
            coins("unlock_rehomed", Some(1)).line(UNIX_EPOCH),
            r#"{"t":"1970-01-01T00:00:00.000Z","k":"coins","when":"unlock_rehomed","bip44":[1,5000000],"bip32":[0,0],"dashpay":[1,100000],"coinjoin":[0,0],"moved":1}"#
        );
        assert!(!coins("unlock", None).line(UNIX_EPOCH).contains("moved"));
    }

    /// BUG-71: a shift line is the screen's name and a number of points, nothing else.
    #[test]
    fn a_shift_line_is_the_screen_and_the_points_only() {
        let record = Record::ScrollShift {
            screen: "activity",
            px: 78,
        };
        let fields = record.fields();
        let mut names: Vec<_> = fields.as_object().unwrap().keys().cloned().collect();
        names.sort();
        assert_eq!(names, ["k", "px", "screen"], "{fields}");
        assert_eq!(fields["k"], "scroll_shift");
        assert_eq!(fields["px"], 78);
        let line: serde_json::Value = serde_json::from_str(&record.line(UNIX_EPOCH)).unwrap();
        assert_eq!(line.as_object().unwrap().len(), 4, "{line}");
    }

    /// BUG-71: a scroll line is the screen and the cause, names of the code, nothing else.
    #[test]
    fn a_scroll_line_is_the_screen_and_the_cause_only() {
        let record = Record::ScrollMoved {
            screen: "activity",
            cause: "wheel_before",
        };
        let fields = record.fields();
        let mut names: Vec<_> = fields.as_object().unwrap().keys().cloned().collect();
        names.sort();
        assert_eq!(names, ["cause", "k", "screen"], "{fields}");
        assert_eq!(fields["k"], "scroll_moved");
        let line: serde_json::Value = serde_json::from_str(&record.line(UNIX_EPOCH)).unwrap();
        assert_eq!(line.as_object().unwrap().len(), 4, "{line}");
    }

    #[test]
    fn a_slow_step_keeps_its_name_and_milliseconds_only() {
        let step = "core_peers";
        let cases = [
            (Record::SlowStep { step, ms: 7 }, "slow_step"),
            (Record::StuckStep { step, ms: 7 }, "stuck_step"),
        ];
        for (record, kind) in cases {
            let fields = record.fields();
            let mut names: Vec<_> = fields.as_object().unwrap().keys().cloned().collect();
            names.sort();
            assert_eq!(names, ["k", "ms", "step"], "{fields}");
            assert_eq!(fields["k"], kind);
            assert_eq!(fields["step"], step);
            assert_eq!(fields["ms"], 7);
            let line: serde_json::Value = serde_json::from_str(&record.line(UNIX_EPOCH)).unwrap();
            assert_eq!(line.as_object().unwrap().len(), 4, "{line}");
        }
    }

    /// BUG-136: the token search's lines are counts and ms, nothing of the wallet.
    #[test]
    fn the_token_search_lines_are_counts_only() {
        let stage = Record::TokenSearchStage {
            stage: "pages",
            requests: 42,
            ms: 61_000,
        };
        let end = Record::TokenSearchEnd {
            contracts: 4_190,
            tokens: 1_322,
            identities: 8,
            ms: 150_000,
        };
        let registered = Record::ContractRegistered {
            types: 3,
            indexes: 3,
            keywords: 0,
            bytes: 2_347,
            estimate: 19_414_290,
            charged: Some(19_161_179),
            protocol: 14,
            ms: 9_426,
            recovered: false,
        };
        let page = Record::CrawlPage {
            size: 10,
            ms: 20_000,
        };
        for (record, kind, names) in [
            (page, "crawl_page", &["k", "ms", "size"][..]),
            (
                registered,
                "contract_registered",
                &[
                    "bytes",
                    "charged",
                    "estimate",
                    "indexes",
                    "k",
                    "keywords",
                    "ms",
                    "protocol",
                    "recovered",
                    "types",
                ][..],
            ),
            (
                stage,
                "token_search_stage",
                &["k", "ms", "requests", "stage"][..],
            ),
            (
                end,
                "token_search_end",
                &["contracts", "identities", "k", "ms", "tokens"][..],
            ),
        ] {
            let fields = record.fields();
            let mut keys: Vec<_> = fields.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            assert_eq!(keys, names, "{fields}");
            assert_eq!(fields["k"], kind);
        }
    }

    /// FEAT-0039 AC-A8: a name search's line is a count and a flag, nothing typed.
    #[test]
    fn the_name_search_line_is_a_count_and_a_flag() {
        let fields = Record::NameSearch {
            found: 7,
            cached: true,
        }
        .fields();
        let mut keys: Vec<_> = fields.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["cached", "found", "k"], "{fields}");
        assert_eq!(fields["k"], "name_search");
        assert_eq!(fields["found"], 7);
        assert_eq!(fields["cached"], true);
    }

    #[test]
    fn the_text_in_a_record_is_scrubbed() {
        let line = Record::Panic {
            location: r"C:\Users\Ελένη\evolite\src\app.rs:12:5".into(),
            message: "called unwrap on yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ".into(),
            backtrace: String::new(),
        }
        .line(UNIX_EPOCH);
        assert!(!line.contains("Ελένη") && !line.contains("yMrDQ"), "{line}");
        let line = Record::SyncError {
            text: "peer 192.168.1.10:19999 disconnected".into(),
        }
        .line(UNIX_EPOCH);
        assert!(!line.contains("192.168"), "{line}");
    }

    /// AC-T3: a price record holds its five fields, and none of them can hold text of the
    /// person's: the names are the code's, the rest are numbers and a flag.
    #[test]
    fn a_price_record_is_only_the_price() {
        let line = Record::Price {
            what: "send_credits",
            declared: 12,
            actual: None,
            enough: true,
            reason: "not_enough",
            held: Some(2_875),
        }
        .line(UNIX_EPOCH);
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "actual", "declared", "enough", "held", "k", "reason", "t", "what"
            ],
            "{line}"
        );
        assert_eq!(value["k"], "price");
        assert!(value["actual"].is_null());
    }

    /// BUG-171 (QA's AC-T5): the funding line says the lock's proof and the amounts, and holds
    /// nothing else — no txid, no address.
    #[test]
    fn an_address_fund_record_is_the_proof_and_the_amounts() {
        let line = Record::AddressFund {
            proof: "chain",
            locked: 112_000,
            credited: Some(97_064),
        }
        .line(UNIX_EPOCH);
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["credited", "k", "kept", "locked", "proof", "t"],
            "{line}"
        );
        assert_eq!(value["k"], "address_fund");
        assert_eq!(value["proof"], "chain");
        // BUG-216: the amounts rough, what Platform kept (a fee) exact.
        assert_eq!(value["locked"], 110_000);
        assert_eq!(value["credited"], 97_000);
        assert_eq!(value["kept"], 14_936);
        assert!(
            !line.contains("97064") && !line.contains("112000"),
            "{line}"
        );
    }

    /// TECH-finality-journal: the three lines of the coins' finality, each its own kind with
    /// its counts and nothing else — no txid, no amount.
    #[test]
    fn the_finality_lines_are_counts_only() {
        let lines = [
            (
                Record::FinalCoins {
                    build: "asset_lock",
                    confirmed: 4,
                    is_verified: 1,
                    is_unverified: 2,
                    frozen: 2,
                },
                vec![
                    "build",
                    "confirmed",
                    "frozen",
                    "is_unverified",
                    "is_verified",
                    "k",
                    "t",
                ],
                "final_coins",
            ),
            (
                Record::OwnIslockVerified { since_unlock_s: 31 },
                vec!["k", "since_unlock_s", "t"],
                "own_islock_verified",
            ),
            (
                Record::KeptIslocks { count: 3 },
                vec!["count", "k", "t"],
                "kept_islocks",
            ),
            (
                Record::AdvancedFeatures { on: true },
                vec!["k", "on", "t"],
                "advanced_features",
            ),
            (
                Record::MixedRounds {
                    none: 3,
                    some: 4,
                    full: 71,
                    unknown: 0,
                },
                vec!["full", "k", "none", "some", "t", "unknown"],
                "mixed_rounds",
            ),
        ];
        for (record, keys, kind) in lines {
            let line = record.line(UNIX_EPOCH);
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            let mut found: Vec<&str> = value
                .as_object()
                .unwrap()
                .keys()
                .map(|k| k.as_str())
                .collect();
            found.sort_unstable();
            assert_eq!(found, keys, "{line}");
            assert_eq!(value["k"], kind);
            // Every value but the kind, the build and the time is a small count.
            for (key, v) in value.as_object().unwrap() {
                if !["k", "t", "build"].contains(&key.as_str()) {
                    assert!(v.as_u64().is_some_and(|n| n < 1_000), "{key}: {line}");
                }
            }
        }
    }

    /// BUG-216: two significant digits, either sign; small amounts as they are.
    #[test]
    fn an_amount_in_the_journal_is_rough() {
        assert_eq!(rough(125_342_101), 120_000_000);
        assert_eq!(rough(-2_875), -2_800);
        assert_eq!(rough(99), 99);
        assert_eq!(rough(0), 0);
        assert_eq!(rough(i64::MAX) / 10_i64.pow(17), 92);
    }

    /// BUG-216: no exact balance or sum of coins in the journal.
    #[test]
    fn coins_and_a_price_have_no_exact_sums() {
        let coins = Record::Coins {
            when: "unlock",
            bip44: (3, 125_342_101),
            bip32: (0, 0),
            dashpay: (1, 7_654_321),
            coinjoin: (0, 0),
            moved: None,
        }
        .line(UNIX_EPOCH);
        assert!(
            coins.contains("[3,120000000]") && coins.contains("[1,7600000]"),
            "{coins}"
        );
        assert!(!coins.contains("125342101") && !coins.contains("7654321"));
        let price = Record::Price {
            what: "send",
            declared: 226,
            actual: Some(226),
            enough: false,
            reason: "not_enough",
            held: Some(84_120_333),
        }
        .line(UNIX_EPOCH);
        assert!(price.contains(r#""held":84000000"#), "{price}");
        assert!(
            price.contains(r#""declared":226"#),
            "the fee as it is: {price}"
        );
    }

    /// SEC-5: the memlock line is whether it held and, when not, why; no address.
    #[test]
    fn the_memlock_line_is_ok_or_why() {
        let ok = Record::Memlock {
            ok: true,
            error: "",
        }
        .line(UNIX_EPOCH);
        assert_eq!(
            ok,
            r#"{"t":"1970-01-01T00:00:00.000Z","k":"memlock","ok":true}"#
        );
        let refused = Record::Memlock {
            ok: false,
            error: "quota",
        }
        .line(UNIX_EPOCH);
        assert!(
            refused.ends_with(r#""ok":false,"error":"quota"}"#),
            "{refused}"
        );
    }

    /// SYNC-0: `fresh` says each area and source once per unlock; a new unlock says it again.
    #[test]
    fn fresh_is_said_once_per_area_and_source_until_the_next_unlock() {
        let _guard = fresh_test_lock();
        unlocked();
        assert!(fresh_once("history_test", "store"));
        assert!(!fresh_once("history_test", "store"), "once");
        assert!(
            fresh_once("history_test", "network"),
            "the network's is its own"
        );
        unlocked();
        assert!(fresh_once("history_test", "store"), "again after an unlock");
    }

    /// SYNC-0: the sync-speed plan's lines hold their counts and names only.
    #[test]
    fn the_sync_speed_lines_are_counts_only() {
        for (record, keys) in [
            (
                Record::Fresh {
                    what: "history",
                    since_unlock_s: 41,
                    from: "network",
                },
                vec!["from", "k", "since_unlock_s", "t", "what"],
            ),
            (
                Record::PlatformReady {
                    since_unlock_s: 30,
                    quorums: "network",
                },
                vec!["k", "quorums", "since_unlock_s", "t"],
            ),
            (
                Record::HistoryPass {
                    identities: 8,
                    requests: 17,
                    ms: 5_120,
                    failed: 1,
                },
                vec!["failed", "identities", "k", "ms", "requests", "t"],
            ),
            (
                Record::ContactNames {
                    asked: 3,
                    cached: 12,
                    ms: 900,
                },
                vec!["asked", "cached", "k", "ms", "t"],
            ),
            (
                Record::TokenFacts {
                    tokens: 5,
                    requests: 3,
                    ms: 700,
                },
                vec!["k", "ms", "requests", "t", "tokens"],
            ),
            (Record::TokenPage { ms: 1_200 }, vec!["k", "ms", "t"]),
            (
                Record::ContactResend {
                    asked: 3,
                    sent: 1,
                    already: 1,
                    failed: 1,
                    version_full: 0,
                    unclear: 0,
                },
                vec![
                    "already",
                    "asked",
                    "failed",
                    "k",
                    "sent",
                    "t",
                    "unclear",
                    "version_full",
                ],
            ),
            (
                Record::PoolPass {
                    ok: 0,
                    skipped: 0,
                    failed: 1,
                    new_notes: 0,
                    scanned: 0,
                    error: "timeout",
                    repeated: 2,
                    stale_retries: Some(14),
                },
                vec![
                    "error",
                    "failed",
                    "k",
                    "newNotes",
                    "ok",
                    "repeated",
                    "scanned",
                    "skipped",
                    "staleRetries",
                    "t",
                ],
            ),
            (
                Record::PoolRead {
                    ledger: "ready",
                    has_account: false,
                    spendable: None,
                    unspent: Some(8),
                    unspent_sum: Some(1_067_134_000),
                    last_scanned: None,
                    source: "",
                    notes: "fresh",
                },
                vec![
                    "hasAccount",
                    "k",
                    "lastScanned",
                    "ledger",
                    "notes",
                    "source",
                    "spendable",
                    "t",
                    "unspent",
                    "unspentSum",
                ],
            ),
        ] {
            let line = record.line(UNIX_EPOCH);
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            let mut found: Vec<&str> = value
                .as_object()
                .unwrap()
                .keys()
                .map(|k| k.as_str())
                .collect();
            found.sort_unstable();
            assert_eq!(found, keys, "{line}");
        }
    }

    #[test]
    fn rotates_and_keeps_at_most_five_files() {
        let dir = std::env::temp_dir().join(format!("evolite-diag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut writer = Writer::new(dir.clone());
        let line = "x".repeat(400 * 1024);
        for _ in 0..40 {
            writer.write(&line);
        }
        let files = journal_files(&dir);
        assert!(files.len() <= MAX_FILES, "{files:?}");
        assert!(
            files
                .iter()
                .all(|f| std::fs::metadata(f).unwrap().len() <= MAX_FILE)
        );
        // Files that are not the journal's are left alone.
        std::fs::write(dir.join("notes.txt"), b"mine").unwrap();
        prune(&dir, SystemTime::now() + MAX_AGE * 2);
        assert!(journal_files(&dir).is_empty(), "past seven days: gone");
        assert!(dir.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
