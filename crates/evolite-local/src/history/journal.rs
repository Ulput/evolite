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

use evolite_api::{
    Activity, ActivityKind, Chain, Detail, Duffs, IdentityRef, Proof, RequestState, Timestamp,
    WithdrawalState,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

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
    /// A token sent: which, to whom and how much, in words. `spent` is the fee only.
    TokenSent {
        token: String,
        to: String,
        amount: String,
    },
    /// What an app spent from its keys' budgets, as Platform says: one entry an app a day.
    AppSpent { app: String },
}

impl Kind {
    /// Whom an older build names: the app, or the recipient.
    fn named(&self) -> String {
        match self {
            Kind::AppConnect { app, .. } | Kind::AppDisconnect { app } | Kind::AppSpent { app } => {
                app.clone()
            }
            Kind::TokenSent { to, .. } => to.clone(),
        }
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
                // An entry read before keeps up with what the document says now.
                if entry.source == Source::Network {
                    entry.spent = found.spent;
                    entry.moved = found.moved;
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
    path: PathBuf,
    /// What each top-up from this computer added, read once ([`Journal::top_ups`]).
    top_ups: Mutex<Option<HashMap<String, Duffs>>>,
    /// Read once, then kept up to date with what is written.
    entries: Mutex<Option<Kept>>,
}

/// The entries in memory and whether the file lags behind them (a write that failed): the next
/// [`Journal::merge_network`] writes it even when it changes nothing (BUG-58 r2).
struct Kept {
    all: Vec<Entry>,
    unsaved: bool,
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            top_ups: Mutex::new(None),
            entries: Mutex::new(None),
        }
    }

    pub fn entries(&self) -> Vec<Entry> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.get_or_insert_with(|| self.kept()).all.clone()
    }

    fn kept(&self) -> Kept {
        Kept {
            all: self.load(),
            unsaved: false,
        }
    }

    /// Writes the entries and says whether the file now has them all.
    fn write(&self, kept: &mut Kept) -> Result<(), String> {
        let result = self.save(&kept.all);
        kept.unsaved = result.is_err();
        result
    }

    /// Keeps `entry`; written beside and renamed over, so a crash leaves the old file or the
    /// new one. A file that cannot be written loses only the history line.
    /// Returns the line's id ([`give_lines`]): fixed for good, so a note can go to it.
    pub fn add(&self, entry: Entry) -> Option<u64> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let kept = entries.get_or_insert_with(|| self.kept());
        kept.all.push(entry);
        give_lines(&mut kept.all);
        let line = kept.all.last().and_then(|e| e.line);
        let _ = self.write(kept);
        line
    }

    /// [`fold_app_spent`] into the file (BUG-145): what app `app` spent since the last read, as
    /// `identity`, at `at`.
    /// How many entries there are: a new one since an earlier count says a change of the credits
    /// has its line (BUG-147).
    pub fn count(&self) -> usize {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.get_or_insert_with(|| self.kept()).all.len()
    }

    pub fn add_app_spent(&self, app: &str, identity: &str, delta: Duffs, at: Timestamp) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let kept = entries.get_or_insert_with(|| self.kept());
        if fold_app_spent(&mut kept.all, app, identity, delta, at) {
            let _ = self.write(kept);
        }
    }

    /// [`merge`]s what was read from Platform and writes the file when anything changed, or
    /// when an earlier write failed (what it held is in memory only). Returns whether it
    /// changed; an error while the file does not have every entry, so that what was read is not
    /// counted as kept (the marks stay, R-33 p. 2) — this pass's or an earlier one's.
    pub fn merge_network(&self, network: Vec<Entry>) -> Result<bool, String> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let kept = entries.get_or_insert_with(|| self.kept());
        let changed = merge(&mut kept.all, network);
        if changed || kept.unsaved {
            self.write(kept)?;
        }
        Ok(changed)
    }

    /// What each top-up from this computer added to the identity, in duffs, by the txid of its
    /// lock (FR-3): `top-up-credits.json` beside the journal, not an entry of it, as a top-up is
    /// a Core line and an entry would show as a second one. Nothing kept reads as empty.
    pub fn top_ups(&self) -> HashMap<String, Duffs> {
        let mut kept = self.top_ups.lock().unwrap_or_else(|e| e.into_inner());
        kept.get_or_insert_with(|| {
            crate::json_file::read(&self.top_ups_path())
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default()
        })
        .clone()
    }

    /// Keeps what the top-up by the lock `txid` added. A file that cannot be written loses only
    /// this amount: the card then says «added» without it.
    pub fn keep_top_up(&self, txid: &str, added: Duffs) {
        let mut all = self.top_ups();
        all.insert(txid.to_owned(), added);
        let _ = crate::json_file::save(&self.top_ups_path(), &all);
        *self.top_ups.lock().unwrap_or_else(|e| e.into_inner()) = Some(all);
    }

    fn top_ups_path(&self) -> PathBuf {
        self.path.with_file_name("top-up-credits.json")
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

    /// The file's entries, each with its line's id (given now to those written without one:
    /// by an older build, or before ids were kept).
    fn load(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = crate::json_file::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        give_lines(&mut entries);
        entries
    }

    fn save(&self, entries: &[Entry]) -> Result<(), String> {
        crate::json_file::save(&self.path, &entries)
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
        (Some(Kind::TokenSent { token, to, amount }), _) => ActivityKind::TokenSent {
            token: token.clone(),
            to: to.clone(),
            amount: amount.clone(),
        },
        (Some(Kind::AppSpent { app }), _) => ActivityKind::AppSpent { app: app.clone() },
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
            What::Name { label } => ActivityKind::NameRegistered {
                label: label.clone(),
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
fn moves_money(what: &What) -> bool {
    matches!(
        what,
        What::Credits { .. }
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
        let _ = std::fs::remove_dir_all(&dir);
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
}
