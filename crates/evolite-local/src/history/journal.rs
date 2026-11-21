//! What this wallet did on Platform: credit transfers and withdrawals, contact requests, profile
//! writes, DPNS names, shields into the pool. What each really cost is kept too, and the next
//! one of its kind is estimated from it (Platform meters these writes; no formula gives them).
//! Each is written here when Platform accepts it (`Source::Here`). What Platform keeps as
//! documents of the wallet's identities — DPNS names, contact requests, the profile,
//! withdrawals — is also read back from the network (`Source::Network`, `platform_history`)
//! and merged in by the document's key ([`merge`]), so a wallet restored elsewhere shows them
//! too (BUG-58, R-33). An entry written here is never removed.
//! Public data only (names, addresses, amounts, times). A missing or unreadable file is no
//! entries; it is never a reason to refuse anything. New fields are all `serde(default)`: an
//! older build reads a newer file and the other way round.

use crate::json_file::{Entries, State, Store};
use evolite_api::{
    Activity, ActivityKind, Chain, Detail, Duffs, IdentityRef, Proof, RequestState, Timestamp,
    WithdrawalState,
};
use std::collections::HashMap;
use std::path::PathBuf;

/// Where an entry comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Source {
    /// Written on this computer when Platform accepted it: amount and fee are known.
    #[default]
    Here,
    /// Read from a document on Platform (proved): no fee, an amount only when the document
    /// has one.
    Network,
}

/// FEAT-0045 (the architect, 2026-11-21): whether a line is done — not a withdrawal still on
/// its way, not a contact request still waiting for its answer. A backup takes only these: the
/// network gives the truth of the others, and an old «on its way» would hang without its
/// operation.
pub fn finished(line: &Entry) -> bool {
    !matches!(
        line.status,
        Some(WithdrawalStatus::Queued | WithdrawalStatus::Pooled | WithdrawalStatus::Broadcasted)
    ) && line.reply != Some(Reply::Waiting)
}

/// A withdrawal document's `status` (the Withdrawals system contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WithdrawalStatus {
    Queued,
    Pooled,
    Broadcasted,
    Complete,
    Expired,
    /// The Withdrawals contract v2: the payout can never be mined by Core.
    Failed,
}

impl WithdrawalStatus {
    pub fn of(status: u64) -> Option<Self> {
        Some(match status {
            0 => Self::Queued,
            1 => Self::Pooled,
            2 => Self::Broadcasted,
            3 => Self::Complete,
            4 => Self::Expired,
            5 => Self::Failed,
            _ => return None,
        })
    }
}

impl WithdrawalStatus {
    fn shown(self) -> WithdrawalState {
        match self {
            Self::Queued => WithdrawalState::Queued,
            Self::Pooled => WithdrawalState::Pooled,
            Self::Broadcasted => WithdrawalState::Broadcasted,
            Self::Complete => WithdrawalState::Complete,
            Self::Expired => WithdrawalState::Expired,
            Self::Failed => WithdrawalState::Failed,
        }
    }
}

/// Where a contact request stands between us and them, as DashPay says it now (BUG-58): on a
/// request to us, how it was answered; on ours, [`Reply::Accepted`] when ours answered theirs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Reply {
    /// Theirs is not answered.
    Waiting,
    /// Ours came after theirs: we accepted it.
    Accepted,
    /// Theirs came after ours: they accepted ours.
    TheyAccepted,
    /// We hid theirs.
    Hidden,
}

impl Reply {
    fn shown(self) -> RequestState {
        match self {
            Self::Waiting => RequestState::Waiting,
            Self::Accepted => RequestState::Accepted,
            Self::TheyAccepted => RequestState::TheyAccepted,
            Self::Hidden => RequestState::Hidden,
        }
    }
}

/// Entries further apart than this are different operations when matched without a key.
const SAME_TIME: Timestamp = 10 * 60;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum What {
    /// Credits to an identity (`@name`) or a Platform address.
    Credits {
        to: String,
    },
    Withdrawal {
        to: String,
    },
    ContactRequest {
        to: String,
    },
    Profile,
    /// A DPNS name registered for the identity.
    Name {
        label: String,
    },
    /// A contested name claimed into its masternode vote: the write and the fund (FEAT-0021).
    NameClaim {
        label: String,
    },
    /// The link of a claimed name written on its own, or removed (FEAT-0021 r2 Should 2).
    NameLink {
        label: String,
        removed: bool,
    },
    /// How a claim's vote ended: "won", "lost" or "locked"; nothing paid (FEAT-0021 N6).
    NameResult {
        label: String,
        outcome: String,
    },
    /// Credits (or a Platform address's) moved into the shielded pool; `from` says which.
    Shield {
        from: String,
    },
    /// From the wallet's Platform addresses to a Platform address (FEAT-0015). Platform keeps
    /// no history of an address: only this line says it happened (R-9).
    AddressSent {
        to: String,
    },
    /// From the wallet's Platform addresses to a Core address of this wallet; `key` is its
    /// withdrawal document (`wd:<id>`), computed when it was signed.
    AddressWithdrawal {
        to: String,
    },
    /// One of the wallet's identities topped up from its Platform addresses (`@name` or id).
    AddressTopUp {
        identity: String,
    },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub at: Timestamp,
    pub what: What,
    /// What left the identity's credits, in duffs; 0 when not known.
    pub spent: Duffs,
    /// The part of `spent` that went to the payee (or into the pool), not to Platform: what is
    /// left is the fee. 0 in entries written before it was kept.
    #[serde(default)]
    pub moved: Duffs,
    /// The wallet's identity it was done as (base58), when known.
    #[serde(default)]
    pub identity: Option<String>,
    /// The document's key on Platform (`dpns:<id>`, `profile:<id>:<revision>`, `wd:<id>`; a
    /// contact request ours to them `cr:<ours>:<to>`, theirs to us `cri:<from>:<ours>`, one
    /// per pair however often its key was rotated); `None` for an entry not matched to one yet.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub source: Source,
    /// A withdrawal's status, from its document.
    #[serde(default)]
    pub status: Option<WithdrawalStatus>,
    /// The Platform block height from the document, when it has one.
    #[serde(default)]
    pub block: Option<u64>,
    /// The line's id in the history, fixed once given ([`give_lines`]): matching an entry to its
    /// document later changes its key and time, not its id (the chosen line, notes on lines).
    #[serde(default)]
    pub line: Option<u64>,
    /// A contact request to us (`what` is `ContactRequest` naming the sender, key
    /// `cri:<from>:<ours>`), not ours. Kept as a flag, not a kind of its own: an older build
    /// reads the file still (it shows the line as a request of ours) instead of losing it.
    #[serde(default)]
    pub theirs: bool,
    /// How a contact request stands (both ways), from the last read of DashPay.
    #[serde(default)]
    pub reply: Option<Reply>,
    /// A withdrawal's `transactionIndex`: the index of its asset unlock on Core, written by
    /// Platform when the withdrawal is pooled (FEAT-0005 АРХ-2).
    #[serde(default)]
    pub transaction_index: Option<u64>,
    /// What the entry really is when it is one of the kinds an older build does not know (BUG-145):
    /// `what` then says `Credits` to the app or the recipient, which such a build shows as a
    /// credit transfer of the fee, instead of failing to read the whole file and writing over it.
    /// A kind of a newer build this one does not know reads as none, the same way.
    #[serde(default, deserialize_with = "lenient")]
    pub kind: Option<Kind>,
    /// Whether `spent` is what it cost (BUG-176). `false` for a Platform write read from the
    /// network with no entry of this computer's own: its document carries no fee. An entry of an
    /// older build has none and reads as known; a known cost is not written.
    #[serde(default = "known", skip_serializing_if = "is_known")]
    pub cost_known: bool,
}

fn known() -> bool {
    true
}

fn is_known(cost_known: &bool) -> bool {
    *cost_known
}

impl Default for Entry {
    /// An entry written here, before its fields are set.
    fn default() -> Self {
        Self {
            at: 0,
            what: What::Profile,
            spent: 0,
            moved: 0,
            identity: None,
            key: None,
            source: Source::Here,
            status: None,
            block: None,
            line: None,
            theirs: false,
            reply: None,
            transaction_index: None,
            kind: None,
            cost_known: true,
        }
    }
}

/// The kinds of entry written under [`Entry::kind`] (BUG-145): spends of the identity's credits
/// that are no transfer, kept apart so an older build can still read the file.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    /// App Connect v2: an app's keys put on the identity, or renewed; `app` as it calls itself.
    AppConnect {
        app: String,
        /// A sign-in again on the same connection: its keys replaced.
        #[serde(default)]
        renewal: bool,
    },
    /// App Connect v2: an app's keys disabled.
    AppDisconnect { app: String },
    /// FEAT-0020: the identity's keys changed from the keys screen — the numbers added and
    /// disabled; `spent` is what the update cost (BUG-145).
    KeysChanged {
        #[serde(default)]
        added: Vec<u32>,
        #[serde(default)]
        disabled: Vec<u32>,
        /// Of `added`, how many are bound to a contract (DashPay's pair): what the fee of the
        /// next one is learnt from, a key of each kind (BUG-183). 0 before it was kept.
        #[serde(default)]
        bound: u32,
    },
    /// A token sent: which, to whom and how much, in words. `spent` is the fee only. `units`:
    /// the amount in the token's smallest units, kept from FEAT-0019 T-1 on (its document is
    /// matched by it, and the balance's change since a read leaves it out); none before.
    TokenSent {
        token: String,
        to: String,
        amount: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        units: Option<u64>,
        /// The recipient's identity id (base58), kept with `units` (the architect, T-1: two
        /// equal sends to two people a minute apart are two documents, matched each to its own).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_id: Option<String>,
    },
    /// FEAT-0038 R-15, AC-T-W4: an NFT of the identity's own collection sold, from
    /// DocumentHistory's `purchase` (`key` `nftsold:<purchase id>`). `spent` is minus the price:
    /// it came in. `model` and `serial` are `None` when the instance was deleted after the sale;
    /// `buyer`: `@name` when known, else the id.
    NftSold {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        serial: Option<u32>,
        buyer: String,
    },
    /// A token received (FEAT-0019 T-1, UC-5б): its `transfer` document, read with a proof
    /// (`key` `tokx:<id>`). `from`: the sender. Nothing was spent.
    TokenReceived {
        token: String,
        from: String,
        amount: String,
        units: u64,
    },
    /// A token's balance moved between two proved reads with no operation of this wallet (UC-5в):
    /// the token keeps no transfer history. `up`: it grew.
    TokenBalance {
        token: String,
        amount: String,
        units: u64,
        up: bool,
    },
    /// What an app spent from its keys' budgets, as Platform says: one entry an app a day.
    AppSpent { app: String },
    /// A token's distribution claimed (FEAT-0019 T-2.3): the amount its proved `claim` document
    /// says (`key` `tokx:<id>`). `spent` is the fee.
    TokenClaimed {
        token: String,
        amount: String,
        units: u64,
    },
    /// A token bought from its issuer (T-2.5): the count. `spent` is what it cost with the fee;
    /// keyed `tokx:<id>` when the token keeps its purchases.
    TokenBought {
        token: String,
        amount: String,
        units: u64,
    },
    /// An issuer action (FEAT-0019 T-3.10): what, how much (mint, burn) and on whom (freeze,
    /// unfreeze, destroy). `spent` is the fee; keyed `tokx:<id>` when Platform wrote a document.
    TokenIssuer {
        token: String,
        act: IssuerKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        amount: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        units: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<String>,
    },
    /// A write of `token` Platform refused that still took credits (BUG-205, review r1 S1): not
    /// done, its fee charged. `spent` is the fee.
    NotDone { token: String },
    /// FEAT-0020-resend: contact requests sent again after the DashPay pair was replaced,
    /// `sent` of `of`, `left` still owed; `spent` is what they cost this identity.
    ContactsResent { sent: u32, of: u32, left: u32 },
}

/// What an issuer action did ([`Kind::TokenIssuer`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IssuerKind {
    Minted,
    Burned,
    Frozen,
    Unfrozen,
    Destroyed,
    Paused,
    Resumed,
    PriceChanged,
    SettingsChanged,
}

impl Kind {
    /// Whom an older build names: the app, or the recipient.
    fn named(&self) -> String {
        match self {
            Kind::AppConnect { app, .. } | Kind::AppDisconnect { app } | Kind::AppSpent { app } => {
                app.clone()
            }
            Kind::TokenSent { to, .. } => to.clone(),
            Kind::KeysChanged { .. } => "keys".into(),
            Kind::TokenReceived { from, .. } => from.clone(),
            Kind::NftSold { buyer, .. } => buyer.clone(),
            Kind::TokenBalance { token, .. }
            | Kind::TokenClaimed { token, .. }
            | Kind::TokenBought { token, .. }
            | Kind::TokenIssuer { token, .. }
            | Kind::NotDone { token } => token.clone(),
            Kind::ContactsResent { .. } => "contacts".into(),
        }
    }
}

/// [`evolite_api::tokens::TokenAct`] of an issuer line, for History.
fn token_act(
    act: IssuerKind,
    amount: Option<&str>,
    who: Option<&str>,
) -> evolite_api::tokens::TokenAct {
    use evolite_api::tokens::TokenAct;
    let amount = amount.unwrap_or_default().to_owned();
    let who = who.unwrap_or_default().to_owned();
    match act {
        IssuerKind::Minted => TokenAct::Minted { amount },
        IssuerKind::Burned => TokenAct::Burned { amount },
        IssuerKind::Frozen => TokenAct::Frozen { who },
        IssuerKind::Unfrozen => TokenAct::Unfrozen { who },
        IssuerKind::Destroyed => TokenAct::Destroyed { who },
        IssuerKind::Paused => TokenAct::Paused,
        IssuerKind::Resumed => TokenAct::Resumed,
        IssuerKind::PriceChanged => TokenAct::PriceChanged,
        IssuerKind::SettingsChanged => TokenAct::SettingsChanged,
    }
}

/// A kind this build does not know (a newer one's) reads as none, not as a broken file.
fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Kind>, D::Error> {
    let value: Option<serde_json::Value> = serde::Deserialize::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// An entry of `kind`, written here: all of `spent` went to Platform (`moved` 0).
pub fn entry_of(kind: Kind, at: Timestamp, spent: Duffs, identity: Option<String>) -> Entry {
    Entry {
        at,
        what: What::Credits { to: kind.named() },
        spent,
        moved: 0,
        identity,
        kind: Some(kind),
        ..Default::default()
    }
}

/// Adds `delta` an app spent to the day's entry of that app for `identity`, or starts the day's
/// entry (BUG-145 AC-T3): one line an app a day, timed at the last read. Days are UTC days.
/// Returns whether anything changed.
pub fn fold_app_spent(
    entries: &mut Vec<Entry>,
    app: &str,
    identity: &str,
    delta: Duffs,
    at: Timestamp,
) -> bool {
    if delta <= 0 {
        return false;
    }
    let day = |t: Timestamp| t.div_euclid(86_400);
    let today = entries.iter_mut().find(|e| {
        e.kind
            == Some(Kind::AppSpent {
                app: app.to_owned(),
            })
            && e.identity.as_deref() == Some(identity)
            && day(e.at) == day(at)
    });
    match today {
        Some(entry) => {
            entry.spent += delta;
            entry.at = entry.at.max(at);
        }
        None => {
            let mut entry = entry_of(
                Kind::AppSpent {
                    app: app.to_owned(),
                },
                at,
                delta,
                Some(identity.to_owned()),
            );
            // The sum is Platform's, proved: not a write of this computer.
            entry.source = Source::Network;
            entries.push(entry);
            give_lines(entries);
        }
    }
    true
}

/// Gives each own token send in `local` that has no document yet the key of its `transfer`
/// document in `network` (FEAT-0019 T-1.3): the same identity, token, recipient and units, within
/// [`SAME_TIME`]. [`merge`] then joins them by the key, and the send is one line; a document
/// with no send here (a restore, R-19) stays to be added. Each document keys one send at most.
pub fn key_token_sends(local: &mut [Entry], network: &[Entry]) {
    for found in network {
        let (
            Some(key),
            Some(Kind::TokenSent {
                token,
                units,
                to_id,
                ..
            }),
        ) = (&found.key, &found.kind)
        else {
            continue;
        };
        if local.iter().any(|e| e.key.as_ref() == Some(key)) {
            continue;
        }
        let own = local.iter_mut().find(|e| {
            e.key.is_none()
                && e.identity == found.identity
                && (e.at - found.at).abs() <= SAME_TIME
                && matches!(&e.kind, Some(Kind::TokenSent { token: t, units: u, to_id: r, .. })
                    if t == token && u.is_some() && u == units && r.is_some() && r == to_id)
        });
        if let Some(own) = own {
            own.key = Some(key.clone());
        }
    }
}

/// Puts what was read from Platform (`network`) into the entries (`local`): by the document's
/// key; an entry without one (written before keys were kept, or before Platform was read)
/// matches the same kind and target within [`SAME_TIME`] of the document's time. A match takes
/// the document's key, identity, time and status and keeps its own amount and fee; a document
/// that matches nothing is added. Nothing is ever removed, and no line's id moves: each is fixed
/// before anything is matched. Returns whether anything changed.
pub fn merge(local: &mut Vec<Entry>, network: Vec<Entry>) -> bool {
    let mut changed = give_lines(local);
    for found in network {
        let Some(key) = found.key.clone() else {
            continue;
        };
        let by_key = local.iter().position(|e| e.key.as_deref() == Some(&key));
        let at = by_key.or_else(|| {
            local.iter().position(|e| {
                e.key.is_none()
                    && !found.theirs
                    && !e.theirs
                    && e.what == found.what
                    && (e.identity.is_none() || e.identity == found.identity)
                    && (e.at - found.at).abs() <= SAME_TIME
            })
        });
        match at {
            Some(i) => {
                let entry = &mut local[i];
                let before = entry.clone();
                entry.key = Some(key);
                entry.identity = entry.identity.take().or(found.identity);
                // The document's time is proved; a time of 0 is none. Matched by its key before,
                // the line keeps the earliest: a contact request's line is the first one sent
                // to them (later ones only rotate its key).
                if found.at > 0 {
                    entry.at = if by_key.is_some() && entry.at > 0 {
                        entry.at.min(found.at)
                    } else {
                        found.at
                    };
                }
                entry.status = found.status.or(entry.status);
                entry.reply = found.reply.or(entry.reply);
                entry.transaction_index = found.transaction_index.or(entry.transaction_index);
                entry.block = found.block.or(entry.block);
                // An entry read before keeps up with what the document says now; one of this
                // computer keeps its cost (BUG-176).
                if entry.source == Source::Network {
                    entry.spent = found.spent;
                    entry.moved = found.moved;
                    entry.cost_known = found.cost_known;
                }
                changed |= *entry != before;
            }
            None => {
                local.push(found);
                give_lines(local);
                changed = true;
            }
        }
    }
    changed
}

/// Fixes the id of each entry that has none yet: from the document's key, else from the entry
/// itself (time, kind, amounts) — never from its place in the file, which a merged-in document
/// would shift. Two entries alike to the second take the next free id, in the file's order.
/// Above the txid-derived ids by half the range, as before. Returns whether any was given.
/// An id made from the entry uses `What`'s `Debug` form, which no build promises to keep: it
/// only has to last until the id is in the file, which the first [`merge`] after an update does.
fn give_lines(entries: &mut [Entry]) -> bool {
    if entries.iter().all(|e| e.line.is_some()) {
        return false;
    }
    let mut taken: std::collections::HashSet<u64> = entries.iter().filter_map(|e| e.line).collect();
    for entry in entries.iter_mut().filter(|e| e.line.is_none()) {
        let text = match &entry.key {
            Some(key) => key.clone(),
            None => format!(
                "{}:{:?}:{}:{}",
                entry.at, entry.what, entry.spent, entry.moved
            ),
        };
        // FNV-1a, 64 bits: stable across builds and runs, unlike the std hasher.
        let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        }) >> 2;
        let mut step = 0_u64;
        let line = loop {
            let line = u64::MAX / 2 + (hash.wrapping_add(step) & (u64::MAX >> 2));
            if taken.insert(line) {
                break line;
            }
            step += 1;
        };
        entry.line = Some(line);
    }
    true
}

pub struct Journal {
    /// The entries, read once and kept up to date with what is written; a file that does not
    /// read is set aside, never written over (BUG-204), and entries of a kind this build does
    /// not know are written back as they were.
    store: Store<Entries<Entry>>,
    /// What each top-up from this computer added ([`Journal::top_ups`]).
    top_ups: Store<HashMap<String, Duffs>>,
    /// Lines added while the file was set aside (review r1, Should 1): shown, and written first
    /// thing once the file is back or started over ([`Journal::keep_pending`]). Lost if evolite
    /// closes before (the banner says so).
    pending: std::sync::Mutex<Vec<Entry>>,
}

/// Lines' ids for the entries read without one: by an older build, or before ids were kept.
fn prepare(entries: &mut Entries<Entry>) {
    give_lines(&mut entries.known);
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        Self {
            top_ups: Store::new(path.with_file_name("top-up-credits.json")),
            store: Store::with_prepare(path, prepare),
            pending: Default::default(),
        }
    }

    /// Whether the folder this journal is kept in is still there (BUG-205 r1 C2): a line from a
    /// task that outlived its wallet's deletion is not written, lest the folder be made again.
    pub fn folder_there(&self) -> bool {
        self.store.path().parent().is_some_and(|dir| dir.is_dir())
    }

    pub fn entries(&self) -> Vec<Entry> {
        let mut all = self.store.read(|e| e.known.clone());
        all.extend(self.pending().iter().cloned());
        all
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The lines kept in memory while the file was set aside, into the file now that it reads
    /// again or was started over (review r1, Should 1). Kept in memory if this write fails too.
    pub fn keep_pending(&self) {
        let pending = std::mem::take(&mut *self.pending());
        if pending.is_empty() {
            return;
        }
        let kept = self.store.update_kept(|e| {
            e.known.extend(pending.iter().cloned());
            give_lines(&mut e.known);
            ((), true)
        });
        if kept.is_err() {
            self.pending().extend(pending);
        }
    }

    /// Whether the journal's file, and the top-ups' beside it, read or were set aside
    /// (BUG-204): the core says so to the screens.
    pub fn state(&self) -> State {
        self.store.state()
    }

    pub fn top_ups_state(&self) -> State {
        self.top_ups.state()
    }

    /// The journal's file and the top-ups', for «Try again» and «Start over».
    pub fn stores(&self) -> (&Store<Entries<Entry>>, &Store<HashMap<String, Duffs>>) {
        (&self.store, &self.top_ups)
    }

    /// FEAT-0045: the lines a backup takes — finished ones only ([`finished`]).
    pub fn finished_entries(&self) -> Vec<Entry> {
        self.store
            .read(|e| e.known.iter().filter(|l| finished(l)).cloned().collect())
    }

    /// FEAT-0045 R-9: a backup's finished lines added — one this file has (by its line id, or
    /// by its document key) stays as it is here. Written once, only when one was added. How
    /// many were added.
    pub fn merge_backup(&self, from: &[Entry]) -> Result<usize, String> {
        self.store
            .update(|e| {
                let mut added = 0;
                for line in from.iter().filter(|l| finished(l)) {
                    let here = e.known.iter().any(|l| {
                        (line.line.is_some() && l.line == line.line)
                            || (line.key.is_some() && l.key == line.key)
                    });
                    if !here {
                        e.known.push(line.clone());
                        added += 1;
                    }
                }
                (added, added > 0)
            })
            .map_err(|e| e.to_string())
    }

    /// FEAT-0045 R-9: a backup's top-ups added, one known here kept. How many were added.
    pub fn merge_top_ups(&self, from: &HashMap<String, Duffs>) -> Result<usize, String> {
        self.top_ups
            .update(|all| {
                let before = all.len();
                for (txid, added) in from {
                    all.entry(txid.clone()).or_insert(*added);
                }
                let n = all.len() - before;
                (n, n > 0)
            })
            .map_err(|e| e.to_string())
    }

    /// Keeps `entry`; written beside and renamed over, so a crash leaves the old file or the
    /// new one. A file that cannot be written loses only the history line. One set aside is not
    /// written over (BUG-204): the line waits in memory, shown, until the file is back or started
    /// over (review r1, Should 1).
    /// Returns the line's id ([`give_lines`]): fixed for good, so a note can go to it.
    pub fn add(&self, entry: Entry) -> Option<u64> {
        let mut entry = Some(entry);
        let kept = self.store.update_kept(|e| {
            e.known.push(entry.take().expect("one entry"));
            give_lines(&mut e.known);
            (e.known.last().and_then(|e| e.line), true)
        });
        match kept {
            Ok((line, _)) => line,
            Err(_) => {
                let mut pending = self.pending();
                pending.push(entry.take()?);
                give_lines(&mut pending);
                pending.last().and_then(|e| e.line)
            }
        }
    }

    /// How many entries there are: a new one since an earlier count says a change of the credits
    /// has its line (BUG-147).
    pub fn count(&self) -> usize {
        self.store.read(|e| e.known.len()) + self.pending().len()
    }

    /// [`fold_app_spent`] into the file (BUG-145): what app `app` spent since the last read, as
    /// `identity`, at `at`.
    pub fn add_app_spent(&self, app: &str, identity: &str, delta: Duffs, at: Timestamp) {
        let _ = self.store.update_kept(|e| {
            let changed = fold_app_spent(&mut e.known, app, identity, delta, at);
            ((), changed)
        });
    }

    /// [`merge_network`](Self::merge_network) of tokens' `transfer` documents (FEAT-0019 T-1):
    /// each own send written here first takes its document's key ([`key_token_sends`]).
    pub fn merge_token_network(&self, network: Vec<Entry>) -> Result<bool, String> {
        let _ = self.store.update_kept(|e| {
            key_token_sends(&mut e.known, &network);
            ((), false)
        });
        self.merge_network(network)
    }

    /// [`merge`]s what was read from Platform and writes the file when anything changed, or
    /// when an earlier write failed (what it held is in memory only). Returns whether it
    /// changed; an error while the file does not have every entry, so that what was read is not
    /// counted as kept (the marks stay, R-33 p. 2) — this pass's or an earlier one's — and
    /// while the file is set aside.
    pub fn merge_network(&self, network: Vec<Entry>) -> Result<bool, String> {
        let (changed, written) = self
            .store
            .update_kept(|e| {
                let changed = merge(&mut e.known, network);
                (changed, changed)
            })
            .map_err(|e| e.to_string())?;
        written.map_err(|e| e.to_string())?;
        Ok(changed)
    }

    /// What each top-up from this computer added to the identity, in duffs, by the txid of its
    /// lock (FR-3): `top-up-credits.json` beside the journal, not an entry of it, as a top-up is
    /// a Core line and an entry would show as a second one. Nothing kept reads as empty.
    pub fn top_ups(&self) -> HashMap<String, Duffs> {
        self.top_ups.read(HashMap::clone)
    }

    /// Keeps what the top-up by the lock `txid` added. A file that cannot be written, or was set
    /// aside, loses only this amount: the card then says «added» without it.
    pub fn keep_top_up(&self, txid: &str, added: Duffs) {
        let _ = self.top_ups.update(|all| {
            all.insert(txid.to_owned(), added);
            ((), true)
        });
    }

    /// What the last entry of a kind (`kind`) cost in fees, in duffs; `None` before the first
    /// one whose fee is known.
    pub fn last_fee(&self, kind: impl Fn(&What) -> bool) -> Option<Duffs> {
        self.entries()
            .iter()
            .rev()
            .filter(|e| kind(&e.what))
            .find_map(fee_of)
    }

    /// The dearest fee among the last `recent` entries of a kind (`kind`) whose fee is known.
    pub fn dearest_fee(&self, kind: impl Fn(&What) -> bool, recent: usize) -> Option<Duffs> {
        self.entries()
            .iter()
            .rev()
            .filter(|e| kind(&e.what))
            .filter_map(fee_of)
            .take(recent)
            .max()
    }
}

/// The line's id: the one fixed in the entry ([`give_lines`]; every entry the journal holds
/// has one).
fn line_id(entry: &Entry) -> u64 {
    entry.line.unwrap_or_else(|| {
        let mut one = [entry.clone()];
        give_lines(&mut one);
        one[0].line.unwrap_or_default()
    })
}

/// An entry as a line of the history: on Platform, proved (it was written only once Platform
/// accepted it, or read from a proved document).
pub fn activity(entry: &Entry) -> Activity {
    let kind = match (&entry.kind, &entry.what) {
        (Some(Kind::AppConnect { app, renewal }), _) if *renewal => {
            ActivityKind::AppRenewed { app: app.clone() }
        }
        (Some(Kind::AppConnect { app, .. }), _) => ActivityKind::AppSignIn { app: app.clone() },
        (Some(Kind::AppDisconnect { app }), _) => {
            ActivityKind::AppDisconnected { app: app.clone() }
        }
        (
            Some(Kind::KeysChanged {
                added, disabled, ..
            }),
            _,
        ) => ActivityKind::KeysChanged {
            added: added.clone(),
            disabled: disabled.clone(),
        },
        (
            Some(Kind::TokenSent {
                token, to, amount, ..
            }),
            _,
        ) => ActivityKind::TokenSent {
            token: token.clone(),
            to: to.clone(),
            amount: amount.clone(),
        },
        (
            Some(Kind::NftSold {
                model,
                serial,
                buyer,
            }),
            _,
        ) => ActivityKind::NftSold {
            model: model.clone(),
            serial: *serial,
            buyer: buyer.clone(),
        },
        (
            Some(Kind::TokenReceived {
                token,
                from,
                amount,
                ..
            }),
            _,
        ) => ActivityKind::TokenReceived {
            token: token.clone(),
            from: from.clone(),
            amount: amount.clone(),
        },
        (
            Some(Kind::TokenBalance {
                token, amount, up, ..
            }),
            _,
        ) => ActivityKind::TokenBalance {
            token: token.clone(),
            amount: amount.clone(),
            up: *up,
        },
        (Some(Kind::AppSpent { app }), _) => ActivityKind::AppSpent { app: app.clone() },
        (Some(Kind::TokenClaimed { token, amount, .. }), _) => ActivityKind::TokenClaimed {
            token: token.clone(),
            amount: amount.clone(),
        },
        (Some(Kind::TokenBought { token, amount, .. }), _) => ActivityKind::TokenBought {
            token: token.clone(),
            amount: amount.clone(),
        },
        (
            Some(Kind::TokenIssuer {
                token,
                act,
                amount,
                who,
                ..
            }),
            _,
        ) => ActivityKind::TokenIssuer {
            token: token.clone(),
            act: token_act(*act, amount.as_deref(), who.as_deref()),
        },
        (Some(Kind::NotDone { token }), _) => ActivityKind::NotDone {
            token: token.clone(),
        },
        (Some(Kind::ContactsResent { sent, of, left }), _) => ActivityKind::ContactsResent {
            sent: *sent,
            of: *of,
            left: *left,
        },
        (None, what) => match what {
            What::Credits { to } => ActivityKind::CreditTransfer { to: to.clone() },
            What::Withdrawal { to } => ActivityKind::Withdrawal {
                to: to.clone(),
                status: entry.status.map(WithdrawalStatus::shown),
            },
            What::ContactRequest { to } if entry.theirs => ActivityKind::ContactRequestIn {
                from: to.clone(),
                ours: None,
                state: entry.reply.unwrap_or(Reply::Waiting).shown(),
            },
            What::ContactRequest { to } => ActivityKind::ContactRequest {
                to: to.clone(),
                accepting: entry.reply == Some(Reply::Accepted),
                ours: None,
            },
            What::Profile => ActivityKind::ProfileSaved,
            // BUG-226: a contested name read back from Platform (a fresh copy, its claim not in
            // this journal) was won in the vote, not «registered»: its domain is written when
            // the vote is won. With the claim or result here it is hidden (`won_by_contest`).
            What::Name { label }
                if entry.source == Source::Network && evolite_common::dpns::is_contested(label) =>
            {
                ActivityKind::NameContestEnded {
                    label: label.clone(),
                    outcome: evolite_api::NameOutcome::Won,
                }
            }
            What::Name { label } => ActivityKind::NameRegistered {
                label: label.clone(),
            },
            What::NameClaim { label } => ActivityKind::NameClaimed {
                label: label.clone(),
            },
            What::NameLink { label, removed } => ActivityKind::NameLinkWritten {
                label: label.clone(),
                removed: *removed,
            },
            What::NameResult { label, outcome } => ActivityKind::NameContestEnded {
                label: label.clone(),
                outcome: match outcome.as_str() {
                    "won" => evolite_api::NameOutcome::Won,
                    "locked" => evolite_api::NameOutcome::Locked,
                    _ => evolite_api::NameOutcome::Lost,
                },
            },
            What::Shield { from } => ActivityKind::Shielded {
                from: Some(from.clone()),
            },
            What::AddressSent { to } => ActivityKind::AddressSent { to: to.clone() },
            What::AddressWithdrawal { to } => ActivityKind::AddressWithdrawal {
                to: to.clone(),
                status: entry.status.map(WithdrawalStatus::shown),
            },
            What::AddressTopUp { identity } => ActivityKind::TopUp {
                identity: identity.clone(),
                creates: false,
                credits: evolite_api::Credits::Added,
            },
        },
    };
    let (layers, flow) = evolite_common::layers::of(&kind, Chain::Platform);
    Activity {
        id: line_id(entry),
        kind,
        chain: Chain::Platform,
        at: entry.at,
        amount: -entry.spent,
        proof: Proof::Proved,
        txid: None,
        fee: fee_of(entry),
        block: None,
        address: None,
        layers,
        flow,
        key: entry.line.map(line_key).unwrap_or_default(),
        detail: Some(Box::new(detail_of(entry))),
        note: None,
        rate: None,
        tax_transfer: false,
    }
}

/// The card's part of an entry (FEAT-0005b): the identity it was done as and the document it
/// wrote. A kind done as an identity that names none was written before that was kept (FR-6): a
/// shield can come from a Platform address, which is no identity.
fn detail_of(entry: &Entry) -> Detail {
    let identity = entry.identity.as_ref().map(|id| IdentityRef {
        id: id.clone(),
        name: None,
    });
    Detail {
        incomplete: identity.is_none() && !matches!(entry.what, What::Shield { .. }),
        identity,
        document: entry.key.as_deref().and_then(document_id).map(Into::into),
        cost_unknown: !entry.cost_known,
        ..Detail::default()
    }
}

/// The id of the document in a line's key (АРХ-1): `dpns:<id>`, `wd:<id>`, `profile:<id>:<rev>`.
/// A contact request's key is the pair of identities, not a document's id (AC-T6): none.
fn document_id(key: &str) -> Option<&str> {
    let (kind, rest) = key.split_once(':')?;
    match kind {
        "dpns" | "wd" => Some(rest),
        "profile" => rest.split(':').next(),
        _ => None,
    }
}

/// [`Activity::key`] of a line with this id.
pub fn line_key(line: u64) -> String {
    format!("platform:{line:016x}")
}

/// What Platform took for an entry, when that is known: the balance was read before and after
/// (`spent` > 0), and for a kind that sends money, what it sent is kept (`moved`) and less.
fn fee_of(entry: &Entry) -> Option<Duffs> {
    if entry.spent <= 0 {
        return None;
    }
    if moves_money(&entry.what) {
        (entry.moved > 0 && entry.spent > entry.moved).then(|| entry.spent - entry.moved)
    } else {
        Some(entry.spent)
    }
}

/// A transfer from the wallet's Platform addresses to another address of its own (QA pass 21):
/// what it moved stayed in the wallet, so the line is its fee alone, between own addresses.
pub fn between_own_addresses(line: &mut Activity) {
    if matches!(line.kind, ActivityKind::AddressSent { .. }) {
        line.amount = -line.fee.unwrap_or(-line.amount);
        line.flow = evolite_api::Flow::Between;
    }
}

/// Whether a kind of entry sends money somewhere (so `spent` is more than the fee).
/// BUG-191: a name read from Platform (a `domain` document) that this wallet claimed in a
/// contest: its claim and its result are its lines, and «registered», at the claim's time, would
/// say it was got while the vote went on.
pub fn won_by_contest(entry: &Entry, entries: &[Entry]) -> bool {
    let What::Name { label } = &entry.what else {
        return false;
    };
    entry.source == Source::Network
        && entries.iter().any(|e| {
            matches!(&e.what, What::NameClaim { label: claimed } | What::NameResult { label: claimed, .. }
                if claimed.eq_ignore_ascii_case(label))
                && (e.identity.is_none() || entry.identity.is_none() || e.identity == entry.identity)
        })
}

fn moves_money(what: &What) -> bool {
    matches!(
        what,
        // BUG-190/191: a claim moves its fund into the vote; its fee is the writes.
        What::NameClaim { .. }
            | What::Credits { .. }
            | What::Withdrawal { .. }
            | What::Shield { .. }
            | What::AddressSent { .. }
            | What::AddressWithdrawal { .. }
            | What::AddressTopUp { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-205 r1 C2: a journal whose wallet folder was deleted says so, so a late line is not
    /// written and the folder not made again.
    #[test]
    fn a_journal_knows_its_folder_is_gone() {
        let dir = std::env::temp_dir().join(format!("evolite-journal-gone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let journal = Journal::new(dir.join("platform-history.json"));
        assert!(journal.folder_there());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!journal.folder_there());
        assert!(!dir.exists(), "asking makes nothing");
    }

    /// BUG-205 (review r1 S1): a token write Platform refused that still took credits is a line
    /// of its own, «not done», with the fee; it is kept in the file and read back as it was.
    #[test]
    fn a_refused_write_that_took_a_fee_is_a_line_of_its_own() {
        let line = entry_of(
            Kind::NotDone {
                token: "Evo Coffee".into(),
            },
            1_700_000_000,
            120,
            Some("A".into()),
        );
        let back: Vec<Entry> =
            serde_json::from_str(&serde_json::to_string(std::slice::from_ref(&line)).unwrap())
                .unwrap();
        assert_eq!(back, vec![line.clone()]);
        assert!(matches!(
            activity(&line).kind,
            ActivityKind::NotDone { token } if token == "Evo Coffee"
        ));
        assert_eq!(line.spent, 120);
    }

    /// QA pass 21 (3): 0,001 from one of the wallet's Platform addresses to another of its own: the
    /// line is the fee alone, between own addresses — not «Sent −0,00109478».
    #[test]
    fn a_transfer_to_an_own_platform_address_is_its_fee_alone() {
        let entry = Entry {
            at: 1_794_431_677,
            what: What::AddressSent {
                to: "tdash1own".into(),
            },
            spent: 109_478,
            moved: 100_000,
            ..Default::default()
        };
        let mut line = activity(&entry);
        between_own_addresses(&mut line);
        assert_eq!(line.amount, -9_478);
        assert_eq!(line.flow, evolite_api::Flow::Between);
        assert_eq!(line.fee, Some(9_478));
    }

    #[test]
    fn keeps_entries_across_loads() {
        let dir = std::env::temp_dir().join(format!("evolite-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("platform-history.json");
        let journal = Journal::new(path.clone());
        assert!(journal.entries().is_empty(), "no file, no entries");
        journal.add(Entry {
            at: 1_000,
            what: What::Credits { to: "@bob".into() },
            spent: 100_000,
            moved: 0,
            ..Default::default()
        });
        journal.add(Entry {
            at: 1_001,
            what: What::Profile,
            spent: 0,
            moved: 0,
            ..Default::default()
        });
        let again = Journal::new(path);
        assert_eq!(again.entries().len(), 2);
        let line = activity(&again.entries()[0]);
        assert_eq!(
            line.kind,
            ActivityKind::CreditTransfer { to: "@bob".into() }
        );
        assert_eq!((line.amount, line.chain), (-100_000, Chain::Platform));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_next_fee_is_what_the_last_one_cost() {
        let dir = std::env::temp_dir().join(format!("evolite-journal-fee-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = Journal::new(dir.join("platform-history.json"));
        let credits = |w: &What| matches!(w, What::Credits { .. });
        assert_eq!(journal.last_fee(credits), None, "nothing written yet");
        let entry = |what, spent, moved| Entry {
            at: 1_000,
            what,
            spent,
            moved,
            ..Default::default()
        };
        // Written before `moved` was kept: the fee cannot be told from the amount.
        journal.add(entry(What::Credits { to: "@a".into() }, 100_000, 0));
        assert_eq!(journal.last_fee(credits), None);
        journal.add(entry(What::Credits { to: "@b".into() }, 100_150, 100_000));
        assert_eq!(journal.last_fee(credits), Some(150));
        // A shield from a Platform address before its sync: the fee is not known, the line
        // shows none and the estimate keeps the last known one.
        journal.add(entry(
            What::Shield {
                from: evolite_common::layers::FROM_PLATFORM_ADDRESS.into(),
            },
            300_000,
            300_000,
        ));
        let shield = |w: &What| matches!(w, What::Shield { .. });
        assert_eq!(journal.last_fee(shield), None);
        assert_eq!(activity(&journal.entries()[2]).fee, None);
        // A name moves nothing: all it took is the fee.
        journal.add(entry(
            What::Name {
                label: "evolite".into(),
            },
            12_345,
            0,
        ));
        let line = activity(&journal.entries()[3]);
        assert_eq!(
            (line.kind, line.fee),
            (
                ActivityKind::NameRegistered {
                    label: "evolite".into()
                },
                Some(12_345)
            )
        );
        // BUG-190/191: a claim's line takes its fund and writes; its fee is the writes alone.
        journal.add(entry(
            What::NameClaim {
                label: "sam".into(),
            },
            10_192_905,
            10_000_000,
        ));
        let entries = journal.entries();
        let line = activity(&entries[entries.len() - 1]);
        assert_eq!((line.amount, line.fee), (-10_192_905, Some(192_905)));
        // FEAT-0021 r2 Should 2: a name's link written on its own, or removed, has its line
        // with what it cost, on the identity.
        for removed in [false, true] {
            journal.add(entry(
                What::NameLink {
                    label: "sam".into(),
                    removed,
                },
                4_321,
                0,
            ));
            let entries = journal.entries();
            let line = activity(&entries[entries.len() - 1]);
            assert_eq!(
                (line.kind, line.fee),
                (
                    ActivityKind::NameLinkWritten {
                        label: "sam".into(),
                        removed
                    },
                    Some(4_321)
                )
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// BUG-191: a name read from Platform that this wallet claimed in a contest is said by its
    /// claim and result lines, not again as «registered»; a regular name read so is a line.
    #[test]
    fn a_name_won_by_contest_is_not_said_registered() {
        let claim = here(
            1_000,
            What::NameClaim {
                label: "Sam".into(),
            },
            10_192_905,
        );
        let read = |label: &str| Entry {
            source: Source::Network,
            ..here(
                1_000,
                What::Name {
                    label: label.into(),
                },
                0,
            )
        };
        let entries = vec![claim, read("sam"), read("alice")];
        assert!(
            won_by_contest(&entries[1], &entries),
            "claimed: its result says it"
        );
        assert!(!won_by_contest(&entries[2], &entries), "a regular name");
        let mine = here(
            1_000,
            What::Name {
                label: "sam".into(),
            },
            57_979,
        );
        assert!(
            !won_by_contest(&mine, &entries),
            "only a name read from Platform"
        );
    }

    /// BUG-226: History rebuilt from Platform on a fresh copy, no claim in the journal: a
    /// contested name it owns is the vote won, not «registered»; a regular name, and a contested
    /// one registered here, are registrations.
    #[test]
    fn a_contested_name_read_back_alone_is_the_vote_won() {
        let read = |label: &str| Entry {
            source: Source::Network,
            ..here(
                1_000,
                What::Name {
                    label: label.into(),
                },
                0,
            )
        };
        assert_eq!(
            activity(&read("qakestrelx")).kind,
            ActivityKind::NameContestEnded {
                label: "qakestrelx".into(),
                outcome: evolite_api::NameOutcome::Won,
            }
        );
        assert_eq!(
            activity(&read("ozzieq31")).kind,
            ActivityKind::NameRegistered {
                label: "ozzieq31".into()
            }
        );
        let mine = here(
            1_000,
            What::Name {
                label: "sam".into(),
            },
            57_979,
        );
        assert!(matches!(
            activity(&mine).kind,
            ActivityKind::NameRegistered { .. }
        ));
    }

    fn here(at: Timestamp, what: What, spent: Duffs) -> Entry {
        Entry {
            at,
            what,
            spent,
            ..Default::default()
        }
    }

    fn read(at: Timestamp, what: What, key: &str) -> Entry {
        Entry {
            at,
            what,
            identity: Some("me".into()),
            key: Some(key.into()),
            source: Source::Network,
            ..Default::default()
        }
    }

    fn name(label: &str) -> What {
        What::Name {
            label: label.into(),
        }
    }

    /// FEAT-0005b: the card's part of an entry. A file from before the identity was kept reads
    /// as it did and its line says so once (AC-T1, FR-6); a read entry names its identity and
    /// its document, a contact request's pair of ids is no document's (AC-T6).
    #[test]
    fn an_entry_gives_its_identity_and_document() {
        let old = r#"[{"at":5,"what":{"Name":{"label":"a"}},"spent":9}]"#;
        let entries: Vec<Entry> = serde_json::from_str(old).expect("an old file reads");
        assert_eq!(entries[0].transaction_index, None);
        let line = activity(&entries[0]);
        let d = line.detail.as_deref().expect("detail");
        assert!(d.incomplete && d.identity.is_none() && d.document.is_none());

        let keyed = |key: &str| {
            let mut e = read(5, name("a"), key);
            e.line = Some(1);
            let line = activity(&e);
            let d = line.detail.expect("detail");
            assert!(!d.incomplete);
            assert_eq!(d.identity.expect("named").id, "me");
            d.document
        };
        assert_eq!(keyed("dpns:Abc").as_deref(), Some("Abc"));
        assert_eq!(keyed("wd:Abc").as_deref(), Some("Abc"));
        assert_eq!(keyed("profile:Abc:3").as_deref(), Some("Abc"));
        assert_eq!(keyed("cr:Me:Them"), None);
        assert_eq!(keyed("cri:Them:Me"), None);
    }

    /// FR-3: what a top-up added survives a restart, beside the journal.
    #[test]
    fn a_top_ups_amount_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("evolite-topups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("platform-history.json");
        let journal = Journal::new(path.clone());
        assert!(journal.top_ups().is_empty(), "no file, nothing");
        journal.keep_top_up("aa", 42_298);
        assert_eq!(journal.top_ups().get("aa"), Some(&42_298));
        assert_eq!(Journal::new(path).top_ups().get("aa"), Some(&42_298));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A shield from a Platform address has no identity to name: nothing is missing.
    #[test]
    fn a_shield_without_an_identity_is_whole() {
        let line = activity(&here(5, What::Shield { from: "a".into() }, 9));
        assert!(!line.detail.expect("detail").incomplete);
    }

    /// The index read from the document is kept by a merge; a later read without it keeps it.
    #[test]
    fn a_merge_keeps_the_withdrawals_index() {
        let to = || What::Withdrawal { to: "yA".into() };
        let doc = |index| {
            let mut e = read(5, to(), "wd:1");
            e.transaction_index = index;
            e
        };
        let mut local = Vec::new();
        merge(&mut local, vec![doc(None)]);
        merge(&mut local, vec![doc(Some(10))]);
        merge(&mut local, vec![doc(None)]);
        assert_eq!(local[0].transaction_index, Some(10));
    }

    /// BUG-58: reading the same documents again changes nothing and adds nothing.
    #[test]
    fn reading_the_network_twice_adds_nothing_the_second_time() {
        let mut local = Vec::new();
        let docs = || {
            vec![
                read(5_000, name("a"), "dpns:1"),
                read(6_000, name("b"), "dpns:2"),
            ]
        };
        assert!(merge(&mut local, docs()));
        assert_eq!(local.len(), 2);
        assert!(!merge(&mut local, docs()), "a second pass is no change");
        assert_eq!(local.len(), 2);
    }

    /// BUG-58: an entry written here before keys were kept is the document within 10 min: it
    /// takes the key and the document's time, and keeps what it cost.
    #[test]
    fn an_old_entry_within_ten_minutes_is_the_document() {
        let mut local = vec![here(10_000, name("a"), 1_234)];
        assert!(merge(
            &mut local,
            vec![read(10_000 + 590, name("a"), "dpns:1")]
        ));
        assert_eq!(local.len(), 1);
        let e = &local[0];
        assert_eq!((e.at, e.spent, e.source), (10_590, 1_234, Source::Here));
        assert_eq!(e.key.as_deref(), Some("dpns:1"));
        assert_eq!(e.identity.as_deref(), Some("me"));
        // Further than 10 min, or another name: a new line, the old one stays.
        let mut local = vec![here(10_000, name("a"), 1_234)];
        merge(&mut local, vec![read(10_000 + 601, name("a"), "dpns:1")]);
        merge(&mut local, vec![read(10_000, name("b"), "dpns:2")]);
        assert_eq!(local.len(), 3);
        assert_eq!(local[0].key, None);
    }

    /// BUG-58: Platform keeps only the profile's last revision: earlier "Profile saved" lines
    /// stay, the last one written here becomes the document.
    #[test]
    fn two_profile_saves_keep_both_lines() {
        let mut local = vec![
            here(1_000, What::Profile, 10),
            here(50_000, What::Profile, 11),
        ];
        merge(&mut local, vec![read(50_100, What::Profile, "profile:p:2")]);
        assert_eq!(local.len(), 2);
        assert_eq!(
            local[0].key, None,
            "the first save is not on Platform any more"
        );
        assert_eq!(local[1].key.as_deref(), Some("profile:p:2"));
        // The next revision is a line of its own.
        merge(&mut local, vec![read(90_000, What::Profile, "profile:p:3")]);
        assert_eq!(local.len(), 3);
    }

    /// BUG-58: a withdrawal read again follows its status; its line is not doubled.
    #[test]
    fn a_withdrawal_follows_its_status() {
        let to = || What::Withdrawal { to: "yAddr".into() };
        let doc = |status| Entry {
            status: Some(status),
            spent: 5_000,
            moved: 5_000,
            ..read(7_000, to(), "wd:9")
        };
        let mut local = vec![here(6_900, to(), 5_100)];
        assert!(merge(&mut local, vec![doc(WithdrawalStatus::Queued)]));
        assert!(merge(&mut local, vec![doc(WithdrawalStatus::Complete)]));
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].status, Some(WithdrawalStatus::Complete));
        assert_eq!(local[0].spent, 5_100, "what it cost here stays");
        // A withdrawal documented and then gone from Platform: the line stays.
        assert!(!merge(&mut local, Vec::new()));
        assert_eq!(local.len(), 1);
    }

    /// BUG-58: the file written by this build, read by the next run (and an older file without
    /// the new fields, read by this one); line ids follow the key, not the place in the file.
    #[test]
    fn merged_entries_survive_a_restart_with_the_same_line_ids() {
        let dir =
            std::env::temp_dir().join(format!("evolite-journal-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("platform-history.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            br#"[{"at":1000,"what":{"Name":{"label":"a"}},"spent":7}]"#,
        )
        .unwrap();
        let journal = Journal::new(path.clone());
        let old_id = activity(&journal.entries()[0]).id;
        assert!(
            journal
                .merge_network(vec![read(500, name("z"), "dpns:0")])
                .unwrap()
        );
        let again = Journal::new(path);
        let entries = again.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].spent, entries[0].source), (7, Source::Here));
        assert_eq!(activity(&entries[0]).id, old_id, "an id does not move");
        assert_eq!(entries[1].key.as_deref(), Some("dpns:0"));
        assert_ne!(activity(&entries[0]).id, activity(&entries[1]).id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-58 r1 should 1: an entry written before keys were kept keeps its line's id when it is
    /// first matched to its document (its key and time change), and after a restart.
    #[test]
    fn an_old_entry_keeps_its_line_id_when_it_is_matched() {
        let dir = std::env::temp_dir().join(format!("evolite-journal-line-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("platform-history.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            br#"[{"at":1000,"what":{"Name":{"label":"a"}},"spent":7}]"#,
        )
        .unwrap();
        let journal = Journal::new(path.clone());
        let old_id = activity(&journal.entries()[0]).id;
        assert!(
            journal
                .merge_network(vec![read(1_300, name("a"), "dpns:1")])
                .unwrap()
        );
        let entries = journal.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (entries[0].at, entries[0].key.as_deref()),
            (1_300, Some("dpns:1"))
        );
        assert_eq!(activity(&entries[0]).id, old_id, "matched, the id stays");
        let again = Journal::new(path);
        assert_eq!(
            activity(&again.entries()[0]).id,
            old_id,
            "and after a restart"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-58 r1 should 2: two entries alike to the second (two transfers to @bob at once) are
    /// two lines, each with its own id, the same after a restart.
    #[test]
    fn two_alike_entries_have_their_own_ids() {
        let dir =
            std::env::temp_dir().join(format!("evolite-journal-twins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("platform-history.json");
        let journal = Journal::new(path.clone());
        let bob = || What::Credits { to: "@bob".into() };
        journal.add(here(1_000, bob(), 100));
        journal.add(here(1_000, bob(), 100));
        let ids = |j: &Journal| {
            j.entries()
                .iter()
                .map(|e| activity(e).id)
                .collect::<Vec<_>>()
        };
        let first = ids(&journal);
        assert_ne!(first[0], first[1]);
        assert_eq!(ids(&Journal::new(path)), first);
        // Another amount is another id of its own, not the next one.
        let mut other = vec![here(1_000, bob(), 100), here(1_000, bob(), 200)];
        give_lines(&mut other);
        assert_eq!(other[0].line, Some(first[0]));
        assert_ne!(other[1].line, Some(first[1]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-58 r1 should 3: a journal that cannot be written says so, so the marks stay.
    #[test]
    fn a_journal_that_cannot_be_written_is_an_error() {
        let dir = std::env::temp_dir().join(format!("evolite-journal-ro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A file where the wallet's folder should be: nothing can be written under it.
        let blocker = dir.join("wallet");
        std::fs::write(&blocker, b"").unwrap();
        let journal = Journal::new(blocker.join("platform-history.json"));
        assert!(
            journal
                .merge_network(vec![read(1_000, name("a"), "dpns:1")])
                .is_err()
        );
        // Still not written: an error again, so the marks keep waiting.
        assert!(journal.merge_network(Vec::new()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Architect probe (BUG-58 r2): a failed write, then a pass that reads the same again once the
    /// folder is writable, must write the file before the marks may move.
    #[test]
    fn arch_probe_failed_write_is_written_by_the_next_pass() {
        let dir =
            std::env::temp_dir().join(format!("evolite-journal-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("wallet");
        std::fs::write(&blocker, b"").unwrap();
        let path = blocker.join("platform-history.json");
        let journal = Journal::new(path.clone());
        assert!(
            journal
                .merge_network(vec![read(1_000, name("a"), "dpns:1")])
                .is_err()
        );
        // The folder becomes writable (an antivirus lock let go).
        std::fs::remove_file(&blocker).unwrap();
        let second = journal.merge_network(vec![read(1_000, name("a"), "dpns:1")]);
        // The pass may move the marks only if what it read is on disk.
        assert!(second.is_ok(), "{second:?}");
        assert_eq!(
            Journal::new(path).entries().len(),
            1,
            "marks would move, file has nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-58 r1 must 1: a contact request's key is the pair; a later request to the same
    /// recipient (its key rotated) is the same line, at the first one's time.
    #[test]
    fn a_rotated_contact_request_is_the_same_line() {
        let to = || What::ContactRequest { to: "@bob".into() };
        let mut local = Vec::new();
        merge(&mut local, vec![read(1_000, to(), "cr:me:bob")]);
        let id = local[0].line;
        merge(&mut local, vec![read(9_000, to(), "cr:me:bob")]);
        assert_eq!(local.len(), 1);
        assert_eq!((local[0].at, local[0].line), (1_000, id));
    }

    /// BUG-58 part 2: a request to us never takes over a request of ours written here (the
    /// same person, within 10 min), and reading it again changes how it stands, not its line.
    #[test]
    fn a_request_to_us_is_a_line_of_its_own_that_follows_its_answer() {
        let bob = || What::ContactRequest { to: "@bob".into() };
        let theirs = |reply| Entry {
            theirs: true,
            reply: Some(reply),
            ..read(1_100, bob(), "cri:bob:me")
        };
        let mut local = vec![here(1_000, bob(), 50)];
        assert!(merge(&mut local, vec![theirs(Reply::Waiting)]));
        assert_eq!(local.len(), 2);
        assert_eq!(local[0].key, None, "ours stays ours");
        let line = local[1].line;
        assert!(matches!(
            activity(&local[1]).kind,
            ActivityKind::ContactRequestIn {
                state: RequestState::Waiting,
                ..
            }
        ));
        assert!(merge(&mut local, vec![theirs(Reply::Accepted)]));
        assert_eq!(local.len(), 2);
        assert_eq!(
            (local[1].reply, local[1].line),
            (Some(Reply::Accepted), line)
        );
        // An older build keeps reading the file: the flag is a field, not a new kind.
        let text = serde_json::to_string(&local).unwrap();
        assert!(
            text.contains(r#""what":{"ContactRequest":{"to":"@bob"}}"#),
            "{text}"
        );
    }

    /// FEAT-0015 AC-T7: the operations from the Platform addresses keep what the history needs
    /// (recipient, what left, what moved, the withdrawal's document), read back as they were
    /// written; a file from before them still reads.
    #[test]
    fn the_addresses_operations_go_through_the_file() {
        let entries = vec![
            Entry {
                at: 10,
                what: What::AddressSent {
                    to: "tdash1qq7x".into(),
                },
                spent: 6_600,
                moved: 100,
                ..Default::default()
            },
            Entry {
                at: 11,
                what: What::AddressWithdrawal { to: "yXk3".into() },
                spent: 401_000,
                moved: 1_000,
                key: Some("wd:9Ab".into()),
                ..Default::default()
            },
            Entry {
                at: 12,
                what: What::AddressTopUp {
                    identity: "@kestrel7".into(),
                },
                spent: 1_500,
                moved: 500,
                ..Default::default()
            },
        ];
        let text = serde_json::to_string(&entries).unwrap();
        let back: Vec<Entry> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, entries);
        let lines: Vec<ActivityKind> = back.iter().map(|e| activity(e).kind).collect();
        assert_eq!(
            lines[0],
            ActivityKind::AddressSent {
                to: "tdash1qq7x".into()
            }
        );
        assert!(matches!(
            lines[1],
            ActivityKind::AddressWithdrawal { status: None, .. }
        ));
        assert!(
            matches!(&lines[2], ActivityKind::TopUp { identity, creates: false, .. } if identity == "@kestrel7")
        );
        // The fee is what left beyond what moved; on Platform, proved.
        let sent = activity(&back[0]);
        assert_eq!(
            (sent.amount, sent.fee, sent.chain),
            (-6_600, Some(6_500), Chain::Platform)
        );
        // A file written before these kinds (only the old ones) reads as before.
        let old = r#"[{"at":1,"what":{"Credits":{"to":"@bob"}},"spent":5,"moved":0}]"#;
        let read: Vec<Entry> = serde_json::from_str(old).unwrap();
        assert_eq!(read.len(), 1);
    }

    /// BUG-145 AC-T3: what an app spent, read twice in a day, is one line of the two; the next day
    /// a new one; another app or identity its own.
    #[test]
    fn an_apps_spending_is_one_line_a_day() {
        let day = 86_400;
        let mut entries = Vec::new();
        assert!(fold_app_spent(
            &mut entries,
            "Yappr",
            "me",
            1_000,
            10 * day + 100
        ));
        assert!(fold_app_spent(
            &mut entries,
            "Yappr",
            "me",
            600,
            10 * day + 5_000
        ));
        assert_eq!(entries.len(), 1, "one line for the day");
        assert_eq!((entries[0].spent, entries[0].at), (1_600, 10 * day + 5_000));
        assert_eq!(
            entries[0].kind,
            Some(Kind::AppSpent {
                app: "Yappr".into()
            })
        );
        assert_eq!(entries[0].source, Source::Network, "proved by Platform");
        assert!(fold_app_spent(
            &mut entries,
            "Yappr",
            "me",
            300,
            11 * day + 10
        ));
        assert!(fold_app_spent(
            &mut entries,
            "Other",
            "me",
            50,
            11 * day + 20
        ));
        assert!(fold_app_spent(
            &mut entries,
            "Yappr",
            "you",
            70,
            11 * day + 30
        ));
        assert_eq!(entries.len(), 4);
        assert!(
            !fold_app_spent(&mut entries, "Yappr", "me", 0, 11 * day + 40),
            "nothing spent"
        );
        let line = activity(&entries[0]);
        assert_eq!(
            line.kind,
            ActivityKind::AppSpent {
                app: "Yappr".into()
            }
        );
        assert_eq!(line.amount, -1_600);
    }

    /// BUG-145 AC-A1: a sign-in again on the same connection is «keys replaced»; an entry written
    /// before renewals were kept reads as a first sign-in.
    #[test]
    fn a_renewal_reads_as_keys_replaced() {
        let renewed = entry_of(
            Kind::AppConnect {
                app: "Yappr".into(),
                renewal: true,
            },
            1,
            18,
            None,
        );
        assert_eq!(
            activity(&renewed).kind,
            ActivityKind::AppRenewed {
                app: "Yappr".into()
            }
        );
        let older: Kind = serde_json::from_str(r#"{"AppConnect":{"app":"Yappr"}}"#).unwrap();
        assert_eq!(
            activity(&entry_of(older, 1, 18, None)).kind,
            ActivityKind::AppSignIn {
                app: "Yappr".into()
            }
        );
    }

    /// BUG-145 AC-T4: an older file reads; a file with the new kinds reads, in an older build too,
    /// as credit lines of the fee (it does not lose the file); a newer kind this build does not
    /// know reads as none.
    #[test]
    fn the_new_kinds_leave_older_and_newer_builds_reading() {
        let entry = entry_of(
            Kind::TokenSent {
                token: "evoliteQA".into(),
                to: "@qa".into(),
                amount: "12.34".into(),
                units: None,
                to_id: None,
            },
            100,
            1_081,
            None,
        );
        let text = serde_json::to_string(&vec![entry.clone()]).unwrap();
        let back: Vec<Entry> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, vec![entry.clone()]);
        assert_eq!(
            activity(&back[0]).kind,
            ActivityKind::TokenSent {
                token: "evoliteQA".into(),
                to: "@qa".into(),
                amount: "12.34".into(),
            }
        );
        // An older build: no `kind` field known — the line is a credit transfer to the recipient.
        #[derive(serde::Deserialize)]
        struct Older {
            what: What,
            spent: Duffs,
            #[serde(default)]
            moved: Duffs,
        }
        let older: Vec<Older> = serde_json::from_str(&text).expect("an older build reads it");
        assert_eq!(older[0].what, What::Credits { to: "@qa".into() });
        assert_eq!((older[0].spent, older[0].moved), (1_081, 0));
        // A newer kind than this build knows.
        let newer = text.replace("TokenSent", "SomethingLater");
        let read: Vec<Entry> = serde_json::from_str(&newer).expect("a newer file reads");
        assert_eq!(read[0].kind, None);
        assert_eq!(
            activity(&read[0]).kind,
            ActivityKind::CreditTransfer { to: "@qa".into() }
        );
    }

    /// BUG-176 AC-T1: a profile read from the network with no entry here has no known cost; one
    /// saved here keeps its cost when its document is read.
    #[test]
    fn a_write_found_only_on_the_network_has_no_known_cost() {
        let found = |key: &str| Entry {
            at: 2_000,
            what: What::Profile,
            identity: Some("ME".into()),
            key: Some(key.into()),
            source: Source::Network,
            cost_known: false,
            ..Default::default()
        };
        // Saved here, with what it cost; its document read later.
        let mut local = vec![Entry {
            at: 1_990,
            what: What::Profile,
            spent: 46_850,
            identity: Some("ME".into()),
            ..Default::default()
        }];
        merge(&mut local, vec![found("profile:P:2")]);
        assert_eq!(local.len(), 1);
        assert!(local[0].cost_known, "saved here: its cost");
        assert_eq!(local[0].spent, 46_850);
        assert!(!activity(&local[0]).detail.expect("detail").cost_unknown);
        // Saved on another device: only the document.
        let mut other: Vec<Entry> = Vec::new();
        merge(&mut other, vec![found("profile:P:3")]);
        assert!(!other[0].cost_known);
        assert!(activity(&other[0]).detail.expect("detail").cost_unknown);
    }

    /// BUG-176 AC-T3: an entry of an older build has no field and reads as known; a known cost is
    /// not written, an unknown one is.
    #[test]
    fn an_older_entry_reads_as_known() {
        let old = r#"{"at":1,"what":"Profile","spent":46850}"#;
        let entry: Entry = serde_json::from_str(old).expect("an older entry");
        assert!(entry.cost_known);
        let written = serde_json::to_string(&entry).unwrap();
        assert!(!written.contains("cost_known"), "{written}");
        let unknown = Entry {
            cost_known: false,
            ..entry
        };
        let written = serde_json::to_string(&unknown).unwrap();
        assert!(written.contains("\"cost_known\":false"), "{written}");
    }

    fn journal_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "evolite-journal-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The bytes of each `<file>.unreadable-*` in `dir`.
    fn set_aside_files(dir: &std::path::Path, file: &str) -> Vec<Vec<u8>> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{file}.unreadable-"))
            })
            .map(|e| std::fs::read(e.path()).unwrap())
            .collect()
    }

    /// BUG-204: an older build reads a file with a kind of `What` it does not know — it keeps
    /// the lines it knows, and its next line does not drop the newer ones.
    #[test]
    fn an_older_build_keeps_the_lines_of_a_newer_kind() {
        let dir = journal_dir("newer");
        let path = dir.join("platform-history.json");
        let journal = Journal::new(path.clone());
        journal.add(here(1_000, name("a"), 100));
        journal.add(here(2_000, name("b"), 100));
        // A newer build wrote a kind this one does not know.
        let text = std::fs::read_to_string(&path).unwrap();
        let newer = text.replacen("\"Name\"", "\"SomethingLater\"", 1);
        assert_ne!(newer, text);
        std::fs::write(&path, &newer).unwrap();
        let older = Journal::new(path.clone());
        assert_eq!(older.entries().len(), 1, "the line it knows");
        assert_eq!(older.state(), State::Readable);
        older.add(here(3_000, name("c"), 100));
        let back = std::fs::read_to_string(&path).unwrap();
        assert!(
            back.contains("SomethingLater"),
            "the newer line is still there"
        );
        assert_eq!(Journal::new(path).count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-204: a journal that does not read is set aside, not written over by the next line
    /// nor by what is read from Platform; «Start over» begins a new one, the old one stays.
    #[test]
    fn a_damaged_journal_is_set_aside_not_written_over() {
        let dir = journal_dir("damaged");
        let path = dir.join("platform-history.json");
        std::fs::write(&path, b"[{\"at\": 1, \"what\"").unwrap();
        let journal = Journal::new(path.clone());
        assert!(journal.entries().is_empty());
        assert!(matches!(journal.state(), State::SetAside { .. }));
        assert!(
            journal
                .merge_network(vec![read(2_000, name("b"), "dpns:2")])
                .is_err()
        );
        assert!(!path.exists(), "nothing written in its place");
        assert_eq!(
            set_aside_files(&dir, "platform-history.json"),
            [b"[{\"at\": 1, \"what\"".to_vec()]
        );
        journal.stores().0.start_anew();
        journal.add(here(1_000, name("a"), 100));
        assert_eq!(Journal::new(path).count(), 1);
        assert_eq!(set_aside_files(&dir, "platform-history.json").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-204 (AC-A2): a journal another program held is set aside; once it can be read,
    /// «Try again» puts it back and every line is there.
    #[test]
    fn a_held_journal_comes_back_with_try_again() {
        let dir = journal_dir("held");
        let path = dir.join("platform-history.json");
        Journal::new(path.clone()).add(here(1_000, name("a"), 100));
        let kept = std::fs::read(&path).unwrap();
        // Not readable as a file (a folder in its place), on every system.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(path.join("in-the-way")).unwrap();
        let journal = Journal::new(path.clone());
        let State::SetAside { path: aside, .. } = journal.state() else {
            panic!("set aside");
        };
        // The program let go: the file is readable where it was put.
        std::fs::remove_dir_all(&aside).unwrap();
        std::fs::write(&aside, &kept).unwrap();
        assert_eq!(journal.stores().0.try_again(), State::Readable);
        assert_eq!(journal.count(), 1);
        journal.add(here(2_000, name("b"), 100));
        assert_eq!(Journal::new(path).count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review r1, Should 1: a line added while the journal is set aside is shown and kept in
    /// memory, not written over the file; «Start over» writes it first, and so does «Try again»
    /// once the file reads, after what the file held.
    #[test]
    fn lines_added_while_set_aside_wait_and_are_written_when_it_is_back() {
        let dir = journal_dir("pending");
        let path = dir.join("platform-history.json");
        std::fs::write(&path, b"[{").unwrap();
        let journal = Journal::new(path.clone());
        let line = journal.add(here(1_000, name("a"), 100));
        assert!(line.is_some(), "a line id for its note");
        assert_eq!(journal.count(), 1, "shown");
        assert_eq!(journal.entries()[0].line, line);
        assert!(!path.exists(), "not written over the file set aside");
        journal.stores().0.start_anew();
        journal.keep_pending();
        let back = Journal::new(path.clone());
        assert_eq!(back.count(), 1);
        assert_eq!(back.entries()[0].line, line, "the same id: its note stays");
        // «Try again»: what the file held, then what waited.
        Journal::new(path.clone()).add(here(500, name("old"), 100));
        let kept = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(path.join("in-the-way")).unwrap();
        let held = Journal::new(path.clone());
        let State::SetAside { path: aside, .. } = held.state() else {
            panic!("set aside");
        };
        held.add(here(2_000, name("b"), 100));
        std::fs::remove_dir_all(&aside).unwrap();
        std::fs::write(&aside, &kept).unwrap();
        assert_eq!(held.stores().0.try_again(), State::Readable);
        held.keep_pending();
        assert_eq!(Journal::new(path).count(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-212: a top-ups file that does not read is set aside, not written over.
    #[test]
    fn a_damaged_top_ups_file_is_set_aside() {
        let dir = journal_dir("top-ups");
        let path = dir.join("platform-history.json");
        std::fs::write(dir.join("top-up-credits.json"), b"{\"aa\": 12").unwrap();
        let journal = Journal::new(path);
        assert!(journal.top_ups().is_empty());
        journal.keep_top_up("bb", 7);
        assert!(!dir.join("top-up-credits.json").exists());
        assert_eq!(
            set_aside_files(&dir, "top-up-credits.json"),
            [b"{\"aa\": 12".to_vec()]
        );
        assert!(matches!(journal.top_ups_state(), State::SetAside { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod token_history_tests {
    use super::*;

    fn sent(at: Timestamp, units: Option<u64>) -> Entry {
        entry_of(
            Kind::TokenSent {
                token: "evoliteQA".into(),
                to: "@bob".into(),
                amount: "12.34".into(),
                units,
                to_id: Some("BOBID".into()),
            },
            at,
            1_000,
            Some("ME".into()),
        )
    }

    /// The `transfer` document of an own send, as the core makes it from TokenHistory.
    fn document(id: &str, at: Timestamp, units: u64) -> Entry {
        let mut entry = entry_of(
            Kind::TokenSent {
                token: "evoliteQA".into(),
                to: "BOBID".into(),
                amount: "12.34".into(),
                units: Some(units),
                to_id: Some("BOBID".into()),
            },
            at,
            0,
            Some("ME".into()),
        );
        entry.key = Some(format!("tokx:{id}"));
        entry.source = Source::Network;
        entry
    }

    /// T-1.3: an own send and its document are one line: the journal's line takes the
    /// document's key, and the merge adds nothing.
    #[test]
    fn an_own_send_and_its_document_are_one_line() {
        let mut local = vec![sent(1_000, Some(1_234))];
        let network = vec![document("D1", 1_030, 1_234)];
        key_token_sends(&mut local, &network);
        assert_eq!(local[0].key.as_deref(), Some("tokx:D1"));
        merge(&mut local, network);
        assert_eq!(local.len(), 1, "one line");
        assert_eq!(local[0].spent, 1_000, "the fee written here stays");
    }

    /// T-1.3, R-19: after a restore there is no journal line: the document is the line.
    #[test]
    fn after_a_restore_the_document_is_the_line() {
        let mut local = Vec::new();
        let network = vec![document("D1", 1_030, 1_234)];
        key_token_sends(&mut local, &network);
        merge(&mut local, network);
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].key.as_deref(), Some("tokx:D1"));
    }

    /// Another amount, another hour, or a line written before units were kept: not the same
    /// send — the document is a line of its own, never taken for another.
    #[test]
    fn a_send_unlike_the_document_is_not_matched() {
        for own in [
            sent(1_000, Some(999)),
            sent(1_000 + 3_600, Some(1_234)),
            sent(1_000, None),
        ] {
            let mut local = vec![own];
            let network = vec![document("D1", 1_030, 1_234)];
            key_token_sends(&mut local, &network);
            assert_eq!(local[0].key, None);
            merge(&mut local, network);
            assert_eq!(local.len(), 2);
        }
    }

    /// The architect's T-1 note: two equal sends to two people a minute apart are not merged —
    /// each document keys the send to its own recipient.
    #[test]
    fn equal_sends_to_two_people_stay_two() {
        let mut to_carol = sent(1_060, Some(1_234));
        if let Some(Kind::TokenSent { to_id, .. }) = &mut to_carol.kind {
            *to_id = Some("CAROLID".into());
        }
        let mut local = vec![sent(1_000, Some(1_234)), to_carol];
        let mut for_carol = document("D2", 1_070, 1_234);
        if let Some(Kind::TokenSent { to_id, .. }) = &mut for_carol.kind {
            *to_id = Some("CAROLID".into());
        }
        let network = vec![for_carol, document("D1", 1_010, 1_234)];
        key_token_sends(&mut local, &network);
        assert_eq!(local[0].key.as_deref(), Some("tokx:D1"), "Bob's");
        assert_eq!(local[1].key.as_deref(), Some("tokx:D2"), "Carol's");
    }

    /// Two sends of the same amount, two documents: each document keys one send, and a document
    /// read again keys nothing more (T-1.2: no line twice).
    #[test]
    fn each_document_keys_one_send() {
        let mut local = vec![sent(1_000, Some(1_234)), sent(1_100, Some(1_234))];
        let network = vec![document("D1", 1_010, 1_234), document("D2", 1_110, 1_234)];
        key_token_sends(&mut local, &network);
        let keys: Vec<_> = local.iter().map(|e| e.key.clone()).collect();
        assert_eq!(
            keys,
            vec![Some("tokx:D1".to_owned()), Some("tokx:D2".to_owned())]
        );
        merge(&mut local, network.clone());
        key_token_sends(&mut local, &network);
        merge(&mut local, network);
        assert_eq!(local.len(), 2, "read again: still two");
    }

    /// T-1.2: a received document read twice is one line; it is a received token in History.
    #[test]
    fn a_received_token_read_twice_is_one_line() {
        let mut received = entry_of(
            Kind::TokenReceived {
                token: "evoliteQA".into(),
                from: "@alice".into(),
                amount: "5.00".into(),
                units: 500,
            },
            2_000,
            0,
            Some("ME".into()),
        );
        received.key = Some("tokx:R1".into());
        received.source = Source::Network;
        let mut local = Vec::new();
        merge(&mut local, vec![received.clone()]);
        merge(&mut local, vec![received]);
        assert_eq!(local.len(), 1);
        assert_eq!(
            activity(&local[0]).kind,
            ActivityKind::TokenReceived {
                token: "evoliteQA".into(),
                from: "@alice".into(),
                amount: "5.00".into(),
            }
        );
    }
}

#[cfg(test)]
mod key_change_tests {
    use super::*;

    /// FEAT-0020, BUG-145: a change of the identity's keys is a line of its History with what it
    /// cost — the numbers added and disabled, the credits it took, flowing out.
    #[test]
    fn a_key_change_is_a_line_with_its_cost() {
        let entry = entry_of(
            Kind::KeysChanged {
                added: vec![7],
                disabled: vec![0],
                bound: 0,
            },
            1_793_000_000,
            1_234,
            Some("Id".into()),
        );
        let line = activity(&entry);
        assert_eq!(
            line.kind,
            ActivityKind::KeysChanged {
                added: vec![7],
                disabled: vec![0]
            }
        );
        assert_eq!(line.amount, -1_234, "the cost in duffs");
        // Read back from the file: the kind and its numbers stay.
        let text = serde_json::to_string(&entry).unwrap();
        let back: Entry = serde_json::from_str(&text).unwrap();
        assert_eq!(activity(&back).kind, line.kind);
    }
}
