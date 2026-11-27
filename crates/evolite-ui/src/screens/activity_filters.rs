//! History's filters (FEAT-0009): the layer pills, direction, status, type, period and the
//! search, as pure functions and the screen's state. The core says which balances a line
//! changes and which way ([`Activity::layers`], [`Activity::flow`]); here they are only read.

use crate::text;
use chrono::{DateTime, Datelike, Days, NaiveDate, TimeZone};
use evolite_api::layer;
use evolite_api::{Activity, ActivityKind, Credits, Duffs, Flow, Proof, SavedAddress};

/// The layer pills, in their order: «All», «Core», «Platform» (everything on Platform: the
/// identity and the Platform address), «Private pool», «Identity», «Tokens» (FEAT-0019 T-1,
/// the designer's H1: the sixth chip after «Identity»).
pub const LAYERS: [u8; 7] = [
    0,
    layer::CORE,
    layer::MIXED,
    layer::IDENTITY | layer::PLATFORM_ADDRESS,
    layer::POOL,
    layer::IDENTITY,
    layer::TOKENS,
];

/// The index of «Tokens» in [`LAYERS`].
pub const TOKENS_LAYER: usize = 6;

/// How far an operation is (АРХ-1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Pending,
    Confirmed,
    Final,
}

/// АРХ-1: in the mempool, being sent (or out of a block for long) and an asset lock whose
/// credits Platform has not added — pending; an InstantLock or a block — confirmed; a ChainLock
/// or a Platform proof — final. A lock with no news of its credits goes by its Core proof.
pub fn status_of(a: &Activity) -> Status {
    if let ActivityKind::TopUp {
        credits: Credits::NotAdded,
        ..
    } = a.kind
    {
        return Status::Pending;
    }
    match a.proof {
        Proof::Mempool | Proof::Sending | Proof::Unconfirmed => Status::Pending,
        Proof::InstantLock | Proof::InstantLockChecking | Proof::InBlock => Status::Confirmed,
        Proof::ChainLock | Proof::Proved => Status::Final,
    }
}

/// The types of «Type» (UC-3), as bits of [`Filters::kinds`].
pub mod kind {
    pub const PAYMENTS: u8 = 1;
    pub const SELF_TRANSFER: u8 = 2;
    pub const IDENTITY: u8 = 4;
    pub const NAMES: u8 = 8;
    pub const CONTACTS: u8 = 16;
    pub const POOL: u8 = 32;
    pub const APPS: u8 = 64;
    /// FEAT-0052 R-5: CoinJoin's own operations — the days of mixing.
    pub const MIXING: u8 = 128;
    /// In the panel's order, with their words.
    pub const ALL: [(u8, &str); 8] = [
        (PAYMENTS, tr!("Payments")),
        (SELF_TRANSFER, tr!("Transfer to yourself")),
        (IDENTITY, "Identity"),
        (NAMES, tr!("Names")),
        (CONTACTS, tr!("Contacts and profile")),
        (POOL, tr!("Pool")),
        (APPS, tr!("Apps")),
        (MIXING, tr!("Mixing")),
    ];
}

/// The type of a line, one bit of [`kind`].
pub fn kind_of(a: &Activity) -> u8 {
    use ActivityKind::*;
    match a.kind {
        Received { .. }
        | Sent { .. }
        | PlatformReceived
        | AddressSent { .. }
        | AddressWithdrawal { .. } => kind::PAYMENTS,
        // FEAT-0015 stage 2: from Core to the wallet's own Platform address, a move between its own.
        SelfTransfer { .. } | IdentityTransfer { .. } | AddressFunded => kind::SELF_TRANSFER,
        // FEAT-0052 R-5: a mixing round and the preparation are «Mixing» (FEAT-0030 had them
        // with the transfers to yourself): only that filter shows the days of mixing.
        Mixing { .. } | IntoMixing => kind::MIXING,
        // FEAT-0038 R-15: an NFT sold, its price in the identity's credits.
        TopUp { .. }
        | Withdrawal { .. }
        | CreditTransfer { .. }
        | KeysChanged { .. }
        | NftSold { .. }
        | NftReceived { .. }
        | NftSeries { .. }
        | CollectionCreated { .. }
        | ModelCreated { .. } => kind::IDENTITY,
        NameRegistered { .. }
        | NameClaimed { .. }
        | NameLinkWritten { .. }
        | NameContestEnded { .. } => kind::NAMES,
        ContactRequest { .. } | ContactRequestIn { .. } | ProfileSaved | ContactsResent { .. } => {
            kind::CONTACTS
        }
        Shielded { .. } | PoolReceived | PoolSent | PoolToCore { .. } | PoolToIdentity { .. } => {
            kind::POOL
        }
        // BUG-145: what apps did with their keys and budgets; a token sent is the identity's.
        AppSignIn { .. } | AppRenewed { .. } | AppDisconnected { .. } | AppSpent { .. } => {
            kind::APPS
        }
        TokenSent { .. }
        | TokenReceived { .. }
        | TokenBalance { .. }
        | TokenClaimed { .. }
        | TokenBought { .. }
        | TokenIssuer { .. }
        | NotDone { .. } => kind::IDENTITY,
    }
}

/// «Period» (UC-4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Period {
    #[default]
    All,
    Today,
    /// The last so many days, today among them.
    Days(u32),
    ThisYear,
    /// From one day to another, both in; either way round.
    Custom(NaiveDate, NaiveDate),
}

impl Period {
    /// The days it keeps, both in, by the calendar of `now`'s time zone; none for «All dates».
    pub fn days(self, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
        match self {
            Period::All => None,
            Period::Today => Some((today, today)),
            Period::Days(n) => Some((
                today
                    .checked_sub_days(Days::new(n.saturating_sub(1) as u64))
                    .unwrap_or(today),
                today,
            )),
            Period::ThisYear => Some((
                NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(today),
                today,
            )),
            Period::Custom(a, b) => Some((a.min(b), a.max(b))),
        }
    }
}

/// Whether the operation at `at` is in `period`, with days as `now`'s time zone counts them
/// (midnight to midnight). An operation with no time (BUG-47) only in «All dates».
pub fn in_period<Tz: TimeZone>(
    period: Period,
    at: evolite_api::Timestamp,
    now: &DateTime<Tz>,
) -> bool {
    let Some((from, to)) = period.days(now.date_naive()) else {
        return true;
    };
    if at == 0 {
        return false;
    }
    let Some(t) = now.timezone().timestamp_opt(at, 0).single() else {
        return false;
    };
    let day = t.date_naive();
    from <= day && day <= to
}

/// What narrows the list besides the search: the layer pill and the panel's choices.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Filters {
    /// The layer pill: an index into [`LAYERS`].
    pub layer: usize,
    pub flow: Option<Flow>,
    pub status: Option<Status>,
    /// Bits of [`kind`]; none — every type.
    pub kinds: u8,
    pub period: Period,
}

/// One filter of the panel shown as a chip under the pills.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chip {
    Flow(Flow),
    Status(Status),
    Kind(u8),
    Period(Period),
}

impl Filters {
    /// The panel's filters on, one chip each (a type each), in the panel's order: their number
    /// is the one on the «Filters» button.
    pub fn chips(&self) -> Vec<Chip> {
        let mut chips = Vec::new();
        chips.extend(self.flow.map(Chip::Flow));
        chips.extend(self.status.map(Chip::Status));
        for (bit, _) in kind::ALL {
            if self.kinds & bit != 0 {
                chips.push(Chip::Kind(bit));
            }
        }
        if self.period != Period::All {
            chips.push(Chip::Period(self.period));
        }
        chips
    }

    /// The chip's cross.
    pub fn remove(&mut self, chip: Chip) {
        match chip {
            Chip::Flow(_) => self.flow = None,
            Chip::Status(_) => self.status = None,
            Chip::Kind(bit) => self.kinds &= !bit,
            Chip::Period(_) => self.period = Period::All,
        }
    }

    /// «Reset»: the panel's filters off; the layer pill stays.
    pub fn reset(&mut self) {
        *self = Filters {
            layer: self.layer,
            ..Filters::default()
        };
    }
}

/// The words of a chip (English keys).
pub fn chip_label(chip: Chip) -> String {
    match chip {
        Chip::Flow(f) => flow_label(Some(f)).into(),
        Chip::Status(s) => status_label(Some(s)).into(),
        Chip::Kind(bit) => kind::ALL
            .iter()
            .find(|(b, _)| *b == bit)
            .map_or("", |(_, w)| w)
            .into(),
        Chip::Period(p) => period_label(p),
    }
}

pub fn flow_label(flow: Option<Flow>) -> &'static str {
    match flow {
        None => tr!("Any"),
        Some(Flow::In) => tr!("In"),
        Some(Flow::Out) => tr!("Out"),
        Some(Flow::Between) => tr!("Between your own"),
    }
}

pub fn status_label(status: Option<Status>) -> &'static str {
    match status {
        None => tr!("Any"),
        Some(Status::Pending) => tr!("Pending"),
        Some(Status::Confirmed) => tr!("Confirmed"),
        Some(Status::Final) => tr!("Final"),
    }
}

/// A period's words (English; a custom one with its dates).
pub fn period_label(period: Period) -> String {
    let t = |s: &str| s.to_owned();
    match period {
        Period::All => t(tr!("All dates")),
        Period::Today => t(tr!("Today")),
        Period::Days(n) => text::plural(n, tr!("day"), tr!("days")),
        Period::ThisYear => t(tr!("This year")),
        Period::Custom(a, b) => {
            let (a, b) = (a.min(b), a.max(b));
            format!("{} – {}", date_text(a), date_text(b))
        }
    }
}

/// A day as the date fields show and take it.
pub fn date_text(d: NaiveDate) -> String {
    d.format("%d.%m.%Y").to_string()
}

/// A day typed into a date field: «01.10.2026» (dots, slashes or dashes between).
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let s: String = s.trim().replace(['/', '-'], ".");
    NaiveDate::parse_from_str(&s, "%d.%m.%Y").ok()
}

/// Search (`q` lowercased and trimmed) by what the row shows — the title, the amount — and what
/// it stands for: the whole address or name, and the txid (an operation's id is its txid's first
/// 16 hex digits, so a pasted txid finds it). Spaces between thousands may be typed as plain
/// spaces. An amount is read as a number, with a comma or a dot (R-4): `0.2` finds «+0,2000».
pub fn matches_query(act: &Activity, q: &str, book: &[SavedAddress]) -> bool {
    if q.is_empty() {
        return true;
    }
    if crate::format::parse_search_amount(q).is_some_and(|d| d != 0 && act.amount.abs() == d) {
        return true;
    }
    let counterparty = match &act.kind {
        ActivityKind::Received { from: Some(who) }
        | ActivityKind::Shielded { from: Some(who) }
        | ActivityKind::Sent { to: who }
        | ActivityKind::CreditTransfer { to: who }
        | ActivityKind::Withdrawal { to: who, .. }
        | ActivityKind::AddressSent { to: who }
        | ActivityKind::AddressWithdrawal { to: who, .. }
        | ActivityKind::ContactRequest { to: who, .. }
        | ActivityKind::ContactRequestIn { from: who, .. }
        | ActivityKind::TopUp { identity: who, .. } => who.as_str(),
        _ => "",
    };
    // The payment's Core address too: a payment in names no one, only our own address it came
    // to, and a payment to a contact shows the @name with the address beside it. And the whole
    // txid, as the card shows it. And the person's note (FEAT-0003, FR-9), any case. And the
    // identity it was done as and its document's id, as the card names them (FEAT-0005, FR-9).
    let detail = act.detail.as_deref();
    // FEAT-0056: a payment to several is found by any of its recipients.
    let recipients: String = text::sent_to(act)
        .iter()
        .map(|r| format!("{} {} ", r.address, r.name.as_deref().unwrap_or_default()))
        .collect();
    let identity = detail.and_then(|d| d.identity.as_ref());
    let shown = format!(
        "{} {} {} {} {} {} {} {} {} {}",
        text::activity_title_in(act, book),
        recipients,
        crate::format::number(act.amount, true),
        counterparty,
        act.address.as_deref().unwrap_or_default(),
        act.txid.as_deref().unwrap_or_default(),
        act.note.as_deref().unwrap_or_default(),
        identity.map_or("", |i| i.id.as_str()),
        identity.and_then(|i| i.name.as_deref()).unwrap_or_default(),
        detail
            .and_then(|d| d.document.as_deref())
            .unwrap_or_default()
    )
    .replace('\u{a0}', " ")
    .to_lowercase();
    let txid = q.len() >= 8 && q.chars().all(|c| c.is_ascii_hexdigit()) && {
        let id = format!("{:016x}", act.id);
        id.starts_with(&q[..q.len().min(16)])
    };
    shown.contains(q) || txid
}

/// AC-T4: whether a line stays — every group must keep it (R-3), and within «Type» any one
/// ticked type does. `q`: the search, lowercased and trimmed.
pub fn keep<Tz: TimeZone>(
    f: &Filters,
    act: &Activity,
    q: &str,
    book: &[SavedAddress],
    now: &DateTime<Tz>,
) -> bool {
    let layer = LAYERS.get(f.layer).copied().unwrap_or(0);
    (layer == 0 || act.layers & layer != 0)
        && f.flow.is_none_or(|flow| act.flow == flow)
        && f.status.is_none_or(|s| status_of(act) == s)
        && (f.kinds == 0 || kind_of(act) & f.kinds != 0)
        && in_period(f.period, act.at, now)
        && matches_query(act, q, book)
}

/// The screen's state: the filters, the search, the panel and the list they leave, worked out
/// again only when one of them or the snapshot changes (AC-T7). Not saved: a new wallet, a lock
/// and a restart start from «All» (R-5).
#[derive(Default)]
pub struct History {
    pub filters: Filters,
    /// «Token: …» under the «Tokens» layer (FEAT-0019 T-1, the designer's H1): the token chosen,
    /// its id and its name as History writes it; `None` — all.
    pub token: Option<(String, String)>,
    /// The «Tokens» layer was open in the last frame: opening it asks for its history once.
    pub tokens_open: bool,
    /// «Token: …»'s list is open.
    pub token_menu: bool,
    pub query: String,
    /// The «Filters» panel is open.
    pub open: bool,
    /// The custom period's two fields, as typed.
    pub from: String,
    pub to: String,
    /// «Export CSV» asked which lines: `Some(true)` — the shown ones (the default).
    pub export: Option<bool>,
    snapshot: u64,
    cache: Option<(Key, Vec<usize>)>,
}

#[derive(PartialEq)]
struct Key {
    snapshot: u64,
    /// Which lines, cheaply (their number and ids): a wallet changed under the same snapshot
    /// (the tests put lines in) is not shown from an old list.
    lines: (usize, u64),
    filters: Filters,
    query: String,
    today: NaiveDate,
}

impl History {
    /// A new snapshot came: the list is worked out again on the next frame.
    pub fn new_snapshot(&mut self) {
        self.snapshot = self.snapshot.wrapping_add(1);
    }

    /// The indexes into `activity` of the lines shown, in its order.
    pub fn shown<Tz: TimeZone>(
        &mut self,
        activity: &[Activity],
        book: &[SavedAddress],
        now: &DateTime<Tz>,
    ) -> &[usize] {
        let q = self.query.trim().to_lowercase();
        let key = Key {
            snapshot: self.snapshot,
            lines: (
                activity.len(),
                activity.iter().fold(0, |x, a| x.rotate_left(1) ^ a.id),
            ),
            filters: self.filters,
            query: q,
            today: now.date_naive(),
        };
        if self.cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let shown = activity
                .iter()
                .enumerate()
                .filter(|(_, act)| keep(&key.filters, act, &key.query, book, now))
                .map(|(i, _)| i)
                .collect();
            self.cache = Some((key, shown));
        }
        self.cache.as_ref().map_or(&[], |(_, shown)| shown)
    }
}

/// FEAT-0057 П7: what came in and went out over the lines shown, in duffs, how many of the
/// lines in «came» are an estimate (what came to the place a move went is not on the line), and
/// how many were left out as pending (R-7.2). `None` on «Tokens» (R-7.3: tokens are not DASH).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Totals {
    pub came: Duffs,
    pub went: Duffs,
    pub estimated: u32,
    /// FEAT-0057 П7 r1 Must: the lines whose part in «went» is not exact — a write whose cost is
    /// not known (its amount an estimate or 0), a move whose Core fee is not known.
    pub went_estimated: u32,
    pub pending: u32,
    /// The lines added (pending ones not): «so far by {n} operations» while History loads.
    pub counted: u32,
}

/// The layer a move between the wallet's own places left (its amount is what left it, fees
/// included); `None` for one within a single place (a send to oneself, a CoinJoin round), where
/// only its fee leaves.
fn source_of(act: &Activity) -> Option<u8> {
    use ActivityKind::*;
    match &act.kind {
        TopUp { .. } if act.chain == evolite_api::Chain::Platform => Some(layer::PLATFORM_ADDRESS),
        TopUp { .. } | IntoMixing | AddressFunded => Some(layer::CORE),
        Shielded { .. } => Some(act.layers & !layer::POOL).filter(|l| *l != 0),
        PoolToCore { .. } => Some(layer::POOL),
        AddressWithdrawal { .. } => Some(layer::PLATFORM_ADDRESS),
        _ => None,
    }
}

/// What a line's own fee is: a CoinJoin round's is its whole amount (the denominations stay,
/// А-3); else the Core fee the wallet paid, `None` when not known.
fn fee_of(act: &Activity) -> Option<Duffs> {
    match act.kind {
        ActivityKind::Mixing { fee } => Some(fee),
        _ => act.fee,
    }
}

/// The architect on R-7.1: what came to the place a move went, when the line knows it — a
/// top-up's credits Platform added (`credits_added`).
fn arrived_of(act: &Activity) -> Option<Duffs> {
    act.detail.as_ref()?.credits_added
}

/// FEAT-0057 П7, R-7.1–R-7.3 (А-3, the architect on R-7.1): the totals of `shown` in the layer
/// pill `layer_pill` (an index into [`LAYERS`]). With fees: an incoming line adds its amount to
/// «came», an outgoing one its amount to «went». A move between the wallet's own places:
/// - at «All» only its fees go — Core's, and Platform's when what arrived is known (the money
///   stays the wallet's);
/// - in the place it left, its whole amount goes (IntoMixing at «Core»: the mixed coins are not
///   Core's, BUG-121);
/// - in the place it came to: what arrived when the line knows it, else its amount less its Core
///   fee — the most that can have come, counted in `estimated`;
/// - within one place, only its fee.
///
/// Pending lines are counted apart, not added (they are not in the balance yet).
pub fn totals(shown: &[&Activity], layer_pill: usize) -> Option<Totals> {
    if layer_pill == TOKENS_LAYER {
        return None;
    }
    let pill = LAYERS.get(layer_pill).copied().unwrap_or(0);
    let mut t = Totals::default();
    for act in shown {
        if status_of(act) == Status::Pending {
            t.pending += 1;
            continue;
        }
        t.counted += 1;
        let size = act.amount.abs();
        match act.flow {
            Flow::In => t.came += size,
            Flow::Out => {
                t.went += size;
                // r1 Must: what it cost is not known (BUG-176) — never added as exact.
                if act.detail.as_ref().is_some_and(|d| d.cost_unknown) {
                    t.went_estimated += 1;
                }
            }
            Flow::Between => match source_of(act) {
                Some(from) if pill != 0 && pill & from != 0 => t.went += size,
                Some(_) if pill != 0 => match arrived_of(act) {
                    Some(arrived) => t.came += arrived,
                    None => {
                        // The most that can have come: what left, less the Core fee when it
                        // is known (Platform's own is not).
                        t.came += match fee_of(act) {
                            Some(fee) => size - fee,
                            None => size,
                        };
                        t.estimated += 1;
                    }
                },
                // «All»: the fees only. What arrived known: Platform's too (what left less
                // what came).
                Some(_) if arrived_of(act).is_some() => {
                    t.went += arrived_of(act).map_or(0, |arrived| (size - arrived).max(0));
                }
                // r1 Must: a fee not known adds nothing — and says so.
                _ => match fee_of(act) {
                    Some(fee) => t.went += fee,
                    None => t.went_estimated += 1,
                },
            },
        }
    }
    Some(t)
}

#[cfg(test)]
mod tests;
