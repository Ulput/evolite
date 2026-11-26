//! FEAT-0052: CoinJoin's own lines — mixing rounds, collateral payments and the coins prepared
//! for mixing (`Mixing`, `IntoMixing`) — one line a local day in History and in Overview's latest
//! operations. The line opens the day's card: what the day did and the list of its operations,
//! each opening its own card, whose «Back» comes back here. Only on screen: the CSV keeps a line
//! a transaction (R-7), and a search shows them one by one (R-6).

use super::txt;
use crate::components::{
    Assets, ButtonKind, Part, TxRow, button, button_row, dialog_with_header_and_footer, kv_row,
    paint_icon_box, tx_row_clickable,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::{Weight, mono, sans};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use egui::{Align, Layout, Ui};
use evolite_api::{Activity, ActivityKind, Duffs, Proof, Timestamp};
use std::collections::HashMap;

/// R-1: a line of CoinJoin's own — a round (a collateral paid among them) or the preparation.
/// A payment from mixed coins is not one: it stays a line of its own.
pub fn is_mixing(act: &Activity) -> bool {
    matches!(
        act.kind,
        ActivityKind::Mixing { .. } | ActivityKind::IntoMixing
    )
}

/// R-2: the local calendar day of `at` by `tz`; `None` for a line with no time (BUG-47).
pub fn day_of<Tz: TimeZone>(at: Timestamp, tz: &Tz) -> Option<NaiveDate> {
    if !text::known(at) {
        return None;
    }
    tz.timestamp_opt(at, 0).single().map(|t| t.date_naive())
}

/// One line of the list: an operation, or a day of CoinJoin's (its operations, in the list's
/// order).
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    One(usize),
    Day(Vec<usize>),
}

/// The lines of `shown` (indexes into `activity`, in its order): CoinJoin's of a day together,
/// where the latest of them stands (R-10 — one more that day moves the line, it makes no second);
/// the rest as they were. One CoinJoin operation of a day is a day's line too, so it looks the
/// same (R-2).
pub fn lines<Tz: TimeZone>(activity: &[Activity], shown: &[usize], tz: &Tz) -> Vec<Line> {
    // A day's operations, and where the latest of them is in `shown` (the first of equal times).
    let mut days: HashMap<Option<NaiveDate>, (Vec<usize>, usize)> = HashMap::new();
    for (place, &i) in shown.iter().enumerate() {
        let act = &activity[i];
        if !is_mixing(act) {
            continue;
        }
        let (members, latest) = days
            .entry(day_of(act.at, tz))
            .or_insert_with(|| (Vec::new(), place));
        if act.at > activity[shown[*latest]].at {
            *latest = place;
        }
        members.push(i);
    }
    let mut out = Vec::with_capacity(shown.len());
    for (place, &i) in shown.iter().enumerate() {
        let act = &activity[i];
        if !is_mixing(act) {
            out.push(Line::One(i));
            continue;
        }
        if let Some((members, latest)) = days.get_mut(&day_of(act.at, tz))
            && *latest == place
        {
            out.push(Line::Day(std::mem::take(members)));
        }
    }
    out
}

/// What a day of mixing did (UC-2).
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Its operations.
    pub count: usize,
    /// Mixing rounds, collateral payments among them.
    pub rounds: usize,
    /// The preparations.
    pub preparations: usize,
    /// The coins the preparations made and what went into them: their own outputs, change not
    /// counted. `None` when a preparation's details were not kept — unknown, not 0.
    pub prepared: Option<(usize, Duffs)>,
    /// What the day changed in the balance (R-4): its fees, negative; 0 — none.
    pub change: Duffs,
    /// R-3: the least sure of their statuses — «Pending» if one is, the seal only if all are
    /// checked.
    pub proof: Proof,
    /// The first one's time and the latest one's (R-11: the line's times).
    pub earliest: Timestamp,
    pub latest: Timestamp,
}

/// How sure a status is, for [`Summary::proof`]: not confirmed by the network, pending, done,
/// checked by evolite. FEAT-0052 r1 Should: «Not confirmed» below «Pending», so a day with both
/// warns as FEAT-0051 does, whichever of them came first.
fn sureness(p: Proof) -> u8 {
    match p {
        Proof::Unconfirmed => 0,
        Proof::Mempool | Proof::Sending => 1,
        _ if text::checked(p) => 3,
        _ => 2,
    }
}

/// [`Summary`] of a day's operations (none — `None`).
pub fn summary(members: &[&Activity]) -> Option<Summary> {
    let least = members.iter().min_by_key(|a| sureness(a.proof))?;
    let preparations: Vec<&&Activity> = members
        .iter()
        .filter(|a| matches!(a.kind, ActivityKind::IntoMixing))
        .collect();
    let prepared = preparations
        .iter()
        .try_fold((0, 0), |(n, sum): (usize, Duffs), a| {
            let to = &a.detail.as_deref()?.own_to;
            Some((
                n + to.len(),
                sum + to.iter().map(|(_, d)| *d).sum::<Duffs>(),
            ))
        });
    Some(Summary {
        count: members.len(),
        rounds: members.len() - preparations.len(),
        preparations: preparations.len(),
        prepared,
        change: members.iter().map(|a| a.amount).sum(),
        proof: least.proof,
        earliest: members.iter().map(|a| a.at).min().unwrap_or_default(),
        latest: members.iter().map(|a| a.at).max().unwrap_or_default(),
    })
}

/// R-8: «Mixing coins» in the plain mode, «Mixing (CoinJoin)» in the advanced one.
pub fn title() -> &'static str {
    if text::advanced() {
        tr!("Mixing (CoinJoin)")
    } else {
        tr!("Mixing coins")
    }
}

/// The day's date: «25 November» (with the year when not this one); «No date» without a time.
fn date(s: &Summary, now: DateTime<Local>) -> String {
    if text::known(s.latest) {
        text::date_day(s.latest, now)
    } else {
        text::day(s.latest, now)
    }
}

/// R-11 (AC-A10): the line's words under its title — «6 operations · 10:00–12:05», the local
/// times of its first and latest operation as the rows beside it write them; «1 operation ·
/// 10:00» for one, or all in one minute; «6 operations» for those with no time (BUG-47). The
/// date is the day's heading over it already; the card and the reader keep it ([`dated`]). On
/// the Overview (`day`: its now), with no headings, the day before the times as the rows beside
/// it say it: «6 operations · yesterday, 10:00–12:05».
pub fn detail(s: &Summary, day: Option<DateTime<Local>>) -> String {
    if !text::known(s.latest) {
        return text::plural(s.count as u32, tr!("operation"), tr!("operations"));
    }
    let clock = |at: Timestamp| {
        Local
            .timestamp_opt(at, 0)
            .single()
            .map_or_else(String::new, |t| t.format("%H:%M").to_string())
    };
    let (first, last) = (clock(s.earliest), clock(s.latest));
    // The range by the catalog's template: in Arabic its order is the language's.
    let times = if first == last {
        last
    } else {
        crate::i18n::t_in(crate::i18n::current(), tr!("{}–{}"))
            .replacen("{}", &first, 1)
            .replacen("{}", &last, 1)
    };
    let times = match day {
        Some(now) => text::with_day(s.latest, &times, now),
        None => times,
    };
    let times = if crate::i18n::rtl() {
        crate::i18n::own(&times)
    } else {
        times
    };
    text::plural_in(
        s.count as u32,
        tr!("{} operation · {}"),
        tr!("{} operations · {}"),
    )
    .replacen("{}", &times, 1)
}

/// UC-2: «6 operations · 25 November» — the day's card's words under its title, with its date.
pub fn dated(s: &Summary, now: DateTime<Local>) -> String {
    // In Arabic the date isolated, as `text::with_date` puts it (BUG-118).
    let date = if crate::i18n::rtl() {
        crate::i18n::own(&date(s, now))
    } else {
        date(s, now)
    };
    text::plural_in(
        s.count as u32,
        tr!("{} operation · {}"),
        tr!("{} operations · {}"),
    )
    .replacen("{}", &date, 1)
}

/// R-4: the fees, with their minus; «no fees» when none left.
pub fn amount(s: &Summary) -> String {
    if s.change == 0 {
        tr!("no fees").into()
    } else {
        text::signed_amount(s.change)
    }
}

/// R-9: the line's one name — «Mixing, 6 operations, 25 November, fees −0.0001, Completed»,
/// with «cryptographically checked» when all are (FEAT-0051 R-3). Amounts hidden, the fees too.
pub fn spoken_name(s: &Summary, now: DateTime<Local>) -> String {
    let sp = crate::i18n::spoken;
    let hidden = crate::format::display().hide;
    let template = if s.change == 0 && !hidden {
        tr!("Mixing, {}, {}, no fees, {}")
    } else {
        tr!("Mixing, {}, {}, fees {}, {}")
    };
    let fees = if hidden {
        "••••".to_owned()
    } else {
        text::signed_amount(s.change)
    };
    let mut parts = vec![
        sp(&text::plural(
            s.count as u32,
            tr!("operation"),
            tr!("operations"),
        )),
        sp(&date(s, now)),
    ];
    if s.change != 0 || hidden {
        parts.push(fees);
    }
    parts.push(sp(text::proof(s.proof).1));
    let mut name = sp(template);
    for part in parts {
        name = name.replacen("{}", &part, 1);
    }
    if text::checked(s.proof) {
        name.push_str(", ");
        name.push_str(&sp(text::CHECKED));
    }
    name
}

/// The day's line in a list — `with_day` on the Overview, which has no day headings (R-11). Its
/// click opens the day's card ([`open_day`]).
pub fn day_row(
    ui: &mut Ui,
    a: &mut Assets,
    now: DateTime<Local>,
    members: &[&Activity],
    with_day: bool,
) {
    let Some(s) = summary(members) else {
        return;
    };
    let detail = detail(&s, with_day.then_some(now));
    let name = spoken_name(&s, now);
    // Hidden amounts: «no fees» says the amount too.
    let amount = if crate::format::display().hide {
        "••••".to_owned()
    } else {
        amount(&s)
    };
    let row = tx_row_clickable(
        ui,
        a,
        &TxRow {
            icon: "mixed",
            title: title(),
            detail: &detail,
            proof: Some(s.proof),
            amount: &amount,
            alert: None,
            quiet: false,
            core_chip: false,
            neutral: None,
            note: None,
            fiat: None,
            dim_amount: false,
            name: Some(&name),
        },
    );
    if row.clicked() {
        open_day(ui.ctx(), day_of(members[0].at, &Local));
    }
}

/// The open day's card: its date (`None` — the operations with no time).
#[derive(Clone, Copy, PartialEq, Debug)]
struct OpenDay(Option<NaiveDate>);

fn day_id() -> egui::Id {
    egui::Id::new("mixing-day")
}

/// Opens the card of the day `day` (its line clicked; the presets).
pub fn open_day(ctx: &egui::Context, day: Option<NaiveDate>) {
    ctx.data_mut(|d| d.insert_temp(day_id(), OpenDay(day)));
    crate::components::dialog_middle_to_top(ctx, "mixing-day");
}

/// Closes the day's card.
pub fn close_day(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<OpenDay>(day_id()));
}

/// Whether a day's card is open: an operation's card opened from it says «Back» (UC-3).
pub fn day_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<OpenDay>(day_id()).is_some())
}

/// The day's card, if one is open and no operation's card is over it. History and Overview show
/// the same one.
pub fn day_window(ui: &mut Ui, a: &mut Assets, cx: &Ctx) {
    let Some(OpenDay(day)) = ui.data(|d| d.get_temp::<OpenDay>(day_id())) else {
        return;
    };
    let wallet = cx.wallet();
    if super::activity_card::card_shown(ui.ctx(), wallet) {
        return;
    }
    let members: Vec<&Activity> = wallet
        .activity
        .iter()
        .filter(|act| is_mixing(act) && day_of(act.at, &Local) == day)
        .collect();
    // Its operations not here (yet: a preset's before its snapshot): nothing to show.
    let Some(s) = summary(&members) else {
        return;
    };
    let now = cx.now;
    let mut open = None;
    let closed = dialog_with_header_and_footer(
        ui,
        a,
        "mixing-day",
        464.0,
        |ui, a| {
            let p = a.p;
            let (boxr, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
            paint_icon_box(ui, a, boxr, 12, "mixed", 22.0, p.ink);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(4.0);
                ui.label(txt(title(), 20.0, Weight::Bold, p.text));
                ui.label(txt(&dated(&s, now), 14.0, Weight::Medium, p.muted));
            });
        },
        |ui, a, part| {
            match part {
                Part::Summary => {}
                Part::Middle => {
                    if let Some(id) = day_body(ui, a, &s, &members, now) {
                        open = Some(id);
                    }
                }
                Part::Footer => {
                    let mut close = false;
                    button_row(ui, |ui| {
                        close = button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked();
                    });
                    return close;
                }
            }
            false
        },
    );
    if closed {
        close_day(ui.ctx());
    } else if let Some(id) = open {
        // UC-3: its own card over this one; «Back» there comes back here.
        super::activity_card::open_card(ui.ctx(), id);
    }
}

/// The day's card under its header: the fees and the status, what mixing does to the balance,
/// what the day did, and its operations — the one clicked.
fn day_body(
    ui: &mut Ui,
    a: &mut Assets,
    s: &Summary,
    members: &[&Activity],
    now: DateTime<Local>,
) -> Option<u64> {
    let p = a.p;
    ui.horizontal(|ui| {
        crate::components::amount_label(ui, &amount(s), mono(28.0, Weight::Medium), p.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            crate::components::status_chip_of(ui, a, s.proof, false);
            crate::components::proof_seal(ui, a, s.proof);
        });
    });
    ui.add(crate::components::para(
        tr!("Mixing does not change the balance, only the fees leave."),
        13.0,
        Weight::Regular,
        p.muted,
    ));
    let value = mono(13.0, Weight::Medium);
    let count = |n: usize| crate::format::count(n as u64);
    kv_row(
        ui,
        &p,
        tr!("Mixing rounds"),
        &count(s.rounds),
        value.clone(),
    );
    // Unknown is a dash, never 0 (a preparation whose details were not kept).
    let (coins, into) = match s.prepared {
        Some((coins, into)) => (count(coins), crate::format::amount(into)),
        None => ("—".to_owned(), "—".to_owned()),
    };
    kv_row(ui, &p, tr!("Coins prepared"), &coins, value.clone());
    kv_row(ui, &p, tr!("Into mixing"), &into, value.clone());
    kv_row(ui, &p, tr!("Fees"), &amount(s), value);
    ui.add_space(6.0);
    let mut clicked = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for act in members {
            if member_row(ui, a, act, now).clicked() {
                clicked = Some(act.id);
            }
        }
    });
    clicked
}

/// R-8: what an operation of the day is — «Round» or «Preparing coins» in the plain mode, its
/// History title in the advanced one.
pub fn member_kind(act: &Activity) -> String {
    match act.kind {
        _ if text::advanced() => text::activity_title_in(act, &[]),
        ActivityKind::IntoMixing => tr!("Preparing coins").into(),
        _ => tr!("Round").into(),
    }
}

/// What an operation of the day took: its amount, or «no fee» (muted) for a round that took
/// nothing — «0.0000 DASH» read as rounded (the designer, 2026-11-26).
pub fn member_amount(act: &Activity) -> Option<String> {
    (act.amount != 0 || text::cost_unknown(act)).then(|| text::row_amount(act))
}

/// One operation of the day in its card (UC-2): what it is and when on the left; on the right
/// its fee over its status (the seal and the words) — not the list's status column, which left
/// the title ≈ 90 px and cut «Preparing coins» (the designer, §8d-2).
fn member_row(ui: &mut Ui, a: &mut Assets, act: &Activity, now: DateTime<Local>) -> egui::Response {
    use crate::components::{named, paint_text, text_width};
    use egui::{Align2, Rect, Sense, pos2, vec2};
    let p = a.p;
    let kind = member_kind(act);
    let time = text::time_of_day(act.at, now);
    let hidden = crate::format::display().hide;
    let (amount, amount_font, amount_color) = match member_amount(act) {
        Some(_) if hidden => ("••••".to_owned(), mono(14.0, Weight::Medium), p.text),
        Some(amount) => (amount, mono(14.0, Weight::Medium), p.text),
        None => (
            tr!("no fee").to_owned(),
            sans(13.0, Weight::Regular),
            p.muted,
        ),
    };
    let (icon, status) = text::proof(act.proof);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
    let sp = crate::i18n::spoken;
    let mut name = [sp(&kind), sp(&amount), sp(&time), sp(status)].join(" · ");
    if text::checked(act.proof) {
        name = format!("{name} · {}", sp(text::CHECKED));
    }
    named(&resp, egui::WidgetType::Button, true, &name);
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(12), p.surface);
    }
    let cy = rect.center().y;
    let boxr = Rect::from_min_size(pos2(rect.left() + 16.0, cy - 18.0), vec2(36.0, 36.0));
    paint_icon_box(ui, a, boxr, 10, text::activity_icon(act), 18.0, p.ink);
    // The right: the fee, and under it the seal, the status's icon and words.
    let right = rect.right() - 16.0;
    let amount_w = text_width(ui, &amount, amount_font.clone());
    paint_text(
        ui,
        pos2(right, cy - 8.0),
        Align2::RIGHT_CENTER,
        &amount,
        amount_font,
        amount_color,
    );
    let status_font = sans(12.0, Weight::Medium);
    let words = paint_text(
        ui,
        pos2(right, cy + 10.0),
        Align2::RIGHT_CENTER,
        status,
        status_font,
        p.muted,
    );
    let mark = Rect::from_center_size(pos2(words.left() - 4.0 - 7.0, cy + 10.0), vec2(14.0, 14.0));
    a.icons.icon(icon, p.muted, &p, 14.0).paint_at(ui, mark);
    let mut status_left = mark.left();
    if let Some(seal) =
        crate::components::proof_seal_at(ui, a, act.proof, pos2(mark.left() - 6.0 - 8.0, cy + 10.0))
    {
        status_left = seal.left();
    }
    let right_w = amount_w.max(right - status_left);
    // The left: the kind whole in the rest of the row, the time under it.
    let tx = boxr.right() + 14.0;
    let room = (right - right_w - 14.0 - tx).max(40.0);
    let title_font = sans(14.0, Weight::SemiBold);
    // Cut with «…» only when it still does not fit (a long language at 800).
    let mut job = egui::text::LayoutJob::single_section(
        crate::i18n::t(&kind).into_owned(),
        egui::TextFormat::simple(title_font, p.text),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: room,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let title = ui.painter().layout_job(job);
    let at = pos2(tx, cy - 8.0 - title.size().y / 2.0);
    ui.painter().galley(at, title, p.text);
    paint_text(
        ui,
        pos2(tx, cy + 10.0),
        Align2::LEFT_CENTER,
        &time,
        sans(12.0, Weight::Regular),
        p.muted,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The designer's frames and the tests (`--screen activity-mixing*`, `overview-mixing`): a day
/// of mixing yesterday — a preparation, four rounds and a collateral paid — and a payment from
/// the mixed coins the same day, which stays a line of its own (A1).
#[cfg(feature = "dev-tools")]
pub fn example_day(now: DateTime<Local>) -> Vec<Activity> {
    use evolite_api::{Chain, Detail, Flow, layer};
    let yesterday = now.date_naive().pred_opt().unwrap_or(now.date_naive());
    let at = |h: u32, m: u32| {
        yesterday
            .and_hms_opt(h, m, 0)
            .and_then(|t| Local.from_local_datetime(&t).earliest())
            .map_or(0, |t| t.timestamp())
    };
    let line = |n: u64, kind: ActivityKind, at: Timestamp, amount: Duffs| {
        let (layers, flow) = match kind {
            ActivityKind::IntoMixing => (layer::CORE | layer::MIXED, Flow::Between),
            ActivityKind::Sent { .. } => (layer::MIXED, Flow::Out),
            _ => (layer::MIXED, Flow::Between),
        };
        let id = EXAMPLE_IDS + n;
        Activity {
            id,
            kind,
            chain: Chain::Core,
            at,
            amount,
            proof: Proof::ChainLock,
            txid: Some(format!("{id:016x}{}", "5c".repeat(24))),
            fee: (amount < 0).then_some(-amount),
            block: Some(1_390_000 + n as u32),
            address: None,
            layers,
            flow,
            key: String::new(),
            detail: None,
            note: None,
            rate: None,
            tax_transfer: false,
        }
    };
    let coinjoin = |n: u32| format!("yMixD{n:02}q7Kp2Wm9Nq6Tv1Bx8Hd3Lc5Fs4Gj");
    let denominations = (1..=5)
        .map(|n| (coinjoin(n), 10_000_100))
        .collect::<Vec<_>>();
    let mut prep = line(1, ActivityKind::IntoMixing, at(10, 0), -374);
    prep.detail = Some(Box::new(Detail {
        own_from: vec!["yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f".into()],
        own_to: denominations.clone(),
        ..Detail::default()
    }));
    let mut paid = line(
        8,
        ActivityKind::Sent {
            to: "yZr4Kp8Wm2Nq6Tv1Bx9Hd3Lc7Fs5Gj0Aa".into(),
        },
        at(12, 30),
        -10_002_200,
    );
    paid.fee = Some(2_200);
    paid.address = Some("yZr4Kp8Wm2Nq6Tv1Bx9Hd3Lc7Fs5Gj0Aa".into());
    let mut out = vec![
        paid,
        line(7, ActivityKind::Mixing { fee: 0 }, at(12, 5), 0),
        line(6, ActivityKind::Mixing { fee: 0 }, at(11, 40), 0),
        line(5, ActivityKind::Mixing { fee: 10_000 }, at(11, 10), -10_000),
        line(4, ActivityKind::Mixing { fee: 0 }, at(10, 45), 0),
        line(3, ActivityKind::Mixing { fee: 0 }, at(10, 20), 0),
        prep,
    ];
    for act in out
        .iter_mut()
        .filter(|a| matches!(a.kind, ActivityKind::Mixing { .. }))
    {
        act.fee = match act.kind {
            ActivityKind::Mixing { fee } if fee > 0 => Some(fee),
            _ => None,
        };
        act.detail = Some(Box::new(Detail {
            own_from: denominations.iter().map(|(a, _)| a.clone()).collect(),
            own_to: denominations.clone(),
            ..Detail::default()
        }));
    }
    out
}

/// The ids of [`example_day`]'s lines start after this.
#[cfg(feature = "dev-tools")]
pub const EXAMPLE_IDS: u64 = 0x0052_0000;

/// Puts [`example_day`] into a wallet's `activity` once, each line before the first older one —
/// the wallet's own lines keep their order.
#[cfg(feature = "dev-tools")]
pub fn with_example_day(activity: &mut Vec<Activity>, now: DateTime<Local>) {
    if activity
        .iter()
        .any(|a| a.id > EXAMPLE_IDS && a.id <= EXAMPLE_IDS + 8)
    {
        return;
    }
    for act in example_day(now) {
        let at = activity
            .iter()
            .position(|old| text::known(old.at) && old.at < act.at)
            .unwrap_or(activity.len());
        activity.insert(at, act);
    }
}

#[cfg(test)]
mod tests;
