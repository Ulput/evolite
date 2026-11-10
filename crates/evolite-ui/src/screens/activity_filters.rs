//! History's filters (FEAT-0009): the layer pills, direction, status, type, period and the
//! search, as pure functions and the screen's state. The core says which balances a line
//! changes and which way ([`Activity::layers`], [`Activity::flow`]); here they are only read.

use crate::text;
use chrono::{DateTime, Datelike, Days, NaiveDate, TimeZone};
use evolite_api::layer;
use evolite_api::{Activity, ActivityKind, Credits, Flow, Proof, SavedAddress};

/// The layer pills, in their order: «All», «Core», «Platform» (everything on Platform: the
/// identity and the Platform address), «Private pool», «Identity».
pub const LAYERS: [u8; 5] = [
    0,
    layer::CORE,
    layer::IDENTITY | layer::PLATFORM_ADDRESS,
    layer::POOL,
    layer::IDENTITY,
];

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
        Proof::InstantLock | Proof::InBlock => Status::Confirmed,
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
    /// In the panel's order, with their words.
    pub const ALL: [(u8, &str); 7] = [
        (PAYMENTS, tr!("Payments")),
        (SELF_TRANSFER, tr!("Transfer to yourself")),
        (IDENTITY, "Identity"),
        (NAMES, tr!("Names")),
        (CONTACTS, tr!("Contacts and profile")),
        (POOL, tr!("Pool")),
        (APPS, tr!("Apps")),
    ];
}

/// The type of a line, one bit of [`kind`].
pub fn kind_of(a: &Activity) -> u8 {
    use ActivityKind::*;
    match a.kind {
        Received { .. } | Sent { .. } | PlatformReceived => kind::PAYMENTS,
        SelfTransfer { .. } => kind::SELF_TRANSFER,
        TopUp { .. } | Withdrawal { .. } | CreditTransfer { .. } => kind::IDENTITY,
        NameRegistered { .. } => kind::NAMES,
        ContactRequest { .. } | ContactRequestIn { .. } | ProfileSaved => kind::CONTACTS,
        Shielded { .. } | PoolReceived | PoolSent => kind::POOL,
        AppSignIn { .. } => kind::APPS,
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
    let identity = detail.and_then(|d| d.identity.as_ref());
    let shown = format!(
        "{} {} {} {} {} {} {} {} {}",
        text::activity_title_in(act, book),
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

#[cfg(test)]
mod tests;
