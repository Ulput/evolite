//! What the core sends as data, in words: dates and times in local time, operation titles,
//! short addresses, key levels. Russian only for now; the strings live here so a translation
//! has one place to go.

use chrono::{DateTime, Datelike, Local, TimeZone};
use evolite_api::{
    Activity, ActivityKind, Chain, CoreError, Credits, KeyLevel, KeyPurpose, Proof, RequestState,
    SavedAddress, SyncStage, Timestamp, WithdrawalState,
};

const MONTHS: [&str; 12] = [
    tr!("January"),
    tr!("February"),
    tr!("March"),
    tr!("April"),
    tr!("May"),
    tr!("June"),
    tr!("July"),
    tr!("August"),
    tr!("September"),
    tr!("October"),
    tr!("November"),
    tr!("December"),
];
const MONTHS_SHORT: [&str; 12] = [
    tr!("Jan"),
    tr!("Feb"),
    tr!("Mar"),
    tr!("Apr"),
    tr!("May"),
    tr!("Jun"),
    tr!("Jul"),
    tr!("Aug"),
    tr!("Sep"),
    tr!("Oct"),
    tr!("Nov"),
    tr!("Dec"),
];

fn local(at: Timestamp) -> DateTime<Local> {
    Local
        .timestamp_opt(at, 0)
        .single()
        .unwrap_or_else(Local::now)
}

fn days_between(at: DateTime<Local>, now: DateTime<Local>) -> i64 {
    (now.date_naive() - at.date_naive()).num_days()
}

/// A date in the interface language: the month by its name there, the parts in its order.
fn date(t: DateTime<Local>, months: &[&str; 12], year: bool) -> String {
    let lang = crate::i18n::current();
    let month = crate::i18n::t_in(lang, months[t.month0() as usize]);
    crate::i18n::date_in(lang, t.day(), &month, year.then(|| t.year()))
}

/// "27 Oct", with the year when it is not this year.
pub fn date_short(at: Timestamp, now: DateTime<Local>) -> String {
    let t = local(at);
    date(t, &MONTHS_SHORT, t.year() != now.year())
}

/// "12 October 2026".
pub fn date_long(at: Timestamp) -> String {
    date(local(at), &MONTHS, true)
}

/// A day as a date field takes it: "dd.mm.yyyy", local time (FEAT-0023's «Your date»).
pub fn date_input(at: Timestamp) -> String {
    local(at).format("%d.%m.%Y").to_string()
}

/// The end of the day typed as "dd.mm.yyyy" (23:59:59 local time), as Unix seconds.
pub fn date_from_input(text: &str) -> Option<Timestamp> {
    let day = crate::screens::activity_filters::parse_date(text)?;
    let end = day.and_hms_opt(23, 59, 59)?;
    end.and_local_timezone(Local)
        .earliest()
        .map(|t| t.timestamp())
}

/// Whether an operation has a time of its own. A note found by reading the shielded pool has
/// none (`at = 0`): that is said in words, never shown as 1 January 1970 (BUG-47).
pub fn known(at: Timestamp) -> bool {
    at > 0
}

/// BUG-77, the designer's improvement 1: how sure the Core balance is, in words under it —
/// nothing when it was checked in this sync; «as of 23:41», «as of 23:41 yesterday», «as of 27
/// Oct, 23:41» for the last checked one; «Not known yet» when none was checked (a new or restored
/// wallet), shown instead of an amount.
pub fn core_as_of(as_of: Option<evolite_api::CoreAsOf>, now: DateTime<Local>) -> Option<String> {
    use evolite_api::CoreAsOf;
    let line: String = match as_of? {
        CoreAsOf::NotYet => tr!("Not known yet").into(),
        CoreAsOf::Checked(at) => {
            let t = local(at);
            let time = t.format("%H:%M").to_string();
            match days_between(t, now) {
                d if d <= 0 => format!(tr!("as of {time}"), time = time),
                1 => format!(tr!("as of {time} yesterday"), time = time),
                _ => format!(
                    tr!("as of {date}, {time}"),
                    date = date_short(at, now),
                    time = time
                ),
            }
        }
    };
    // In the interface language here: the screens put it beside other words.
    Some(crate::i18n::t_in(crate::i18n::current(), &line).into_owned())
}

/// A Core amount in a row of words (BUG-80), as sure as Core's balance is: «1,2000 DASH»;
/// «1,2000 DASH · as of 23:41» for the last checked one; «Not known yet» when none was checked
/// — never a 0. `amount` comes from [`crate::components::CardAmount::as_core`].
pub fn core_amount(
    b: &evolite_api::Balances,
    amount: crate::components::CardAmount,
    now: DateTime<Local>,
) -> String {
    match (amount.known(), core_as_of(b.core_as_of, now)) {
        (Some(duffs), None) => crate::format::balance(duffs),
        (Some(duffs), Some(line)) => format!("{} · {line}", crate::format::balance(duffs)),
        (None, line) => line.unwrap_or_else(|| "—".to_owned()),
    }
}

/// "just now", "today, 14:32", "yesterday", "27 Oct"; "time unknown" without a time.
pub fn when(at: Timestamp, now: DateTime<Local>) -> String {
    if !known(at) {
        return tr!("time unknown").into();
    }
    let t = local(at);
    if (now - t).num_seconds() < 60 {
        return tr!("just now").into();
    }
    match days_between(t, now) {
        0 => format!(tr!("today, {}"), t.format("%H:%M")),
        1 => tr!("yesterday").into(),
        _ => date_short(at, now),
    }
}

/// In a list grouped by day: "just now" or the time; "time unknown" without one.
pub fn time_of_day(at: Timestamp, now: DateTime<Local>) -> String {
    if !known(at) {
        return tr!("time unknown").into();
    }
    let t = local(at);
    if (now - t).num_seconds() < 60 {
        tr!("just now").into()
    } else {
        t.format("%H:%M").to_string()
    }
}

/// «before · date»: what a line says, then when (a date from [`when`] or [`date_short`]). In
/// the Arabic interface the date goes in translated and isolated (`own`'s marks, which `t`
/// takes off after reordering): a day number after a Latin word («Core · 8 نوفمبر») otherwise
/// took that word's direction (bidi rule W7) and stood apart from its month (BUG-118).
pub fn with_date(before: &str, date: &str) -> String {
    if !crate::i18n::rtl() {
        return format!("{before} · {date}");
    }
    let date = crate::i18n::t_in(crate::i18n::current(), date);
    format!("{before} · {}", crate::i18n::own(&date))
}

/// "12 October 2026, 14:32" or "12 October 2026, just now" for an operation's card: put together
/// here, so "just now" is in the interface language like the month (a sentence of a date and a
/// time is no catalog's key).
pub fn date_time(at: Timestamp, now: DateTime<Local>) -> String {
    date_time_in(crate::i18n::current(), at, now)
}

fn date_time_in(lang: crate::i18n::Lang, at: Timestamp, now: DateTime<Local>) -> String {
    if !known(at) {
        return crate::i18n::t_in(lang, tr!("unknown")).into_owned();
    }
    let time = time_of_day(at, now);
    format!(
        "{}{}{}",
        date_long(at),
        crate::i18n::list_comma(lang),
        crate::i18n::t_in(lang, &time)
    )
}

/// A day header: "Today", "Yesterday", "27 October"; "No date" over operations without a time.
pub fn day(at: Timestamp, now: DateTime<Local>) -> String {
    if !known(at) {
        return tr!("No date").into();
    }
    let t = local(at);
    match days_between(t, now) {
        0 => tr!("Today").into(),
        1 => tr!("Yesterday").into(),
        _ => date(t, &MONTHS, false),
    }
}

/// Where `address` differs from `other`, by character (0-based), for the look-alike warning
/// (FEAT-0035): another length is another address altogether — every character differs.
pub fn differing(address: &str, other: &str) -> Vec<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (address.chars().collect(), other.chars().collect());
    if a.len() != b.len() {
        return (0..a.len()).collect();
    }
    a.iter()
        .zip(&b)
        .enumerate()
        .filter(|(_, (x, y))| x != y)
        .map(|(i, _)| i)
        .collect()
}

/// The positions of [`differing`] as a person counts them: «7–9, 12» (from 1, runs joined).
pub fn positions(at: &[usize]) -> String {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &i in at {
        match runs.last_mut() {
            Some((_, end)) if *end + 1 == i => *end = i,
            _ => runs.push((i, i)),
        }
    }
    runs.iter()
        .map(|&(a, b)| {
            if a == b {
                format!("{}", a + 1)
            } else {
                format!("{}–{}", a + 1, b + 1)
            }
        })
        .collect::<Vec<_>>()
        .join(crate::i18n::list_comma(crate::i18n::current()))
}

/// A known address that `address` only looks like: the same first and last four characters,
/// another middle — how an address planted in the history passes a glance at "yXk3…9f2c".
pub fn lookalike<'a>(address: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let ends = |s: &str| {
        let c: Vec<char> = s.chars().collect();
        (c.len() > 8).then(|| (c[..4].to_vec(), c[c.len() - 4..].to_vec()))
    };
    let mine = ends(address)?;
    known
        .into_iter()
        .find(|k| *k != address && ends(k).as_ref() == Some(&mine))
}

/// "yXk3…9f2c": enough to compare by eye; a name stays as it is.
pub fn short_address(s: &str) -> String {
    if s.starts_with('@') || s.chars().count() <= 12 {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// The English plural form for `n`: "1 payment", "3 payments". The pair is one catalog key
/// ("{} payment|{} payments"); other languages choose their own forms by CLDR category.
pub fn plural(n: u32, one: &str, other: &str) -> String {
    let form = if n == 1 { one } else { other };
    format!("{n} {form}")
}

/// "12 October" (the year when it is not this one).
pub fn date_day(at: Timestamp, now: DateTime<Local>) -> String {
    let t = local(at);
    if t.year() == now.year() {
        // In the interface language: the English month stayed in a translated sentence.
        date(t, &MONTHS, false)
    } else {
        date_long(at)
    }
}

/// An identity in a line of History: its @name as the core gives it, a nameless one by its
/// short ID (FEAT-0014, frame 11) — the core names it by its whole ID.
fn identity_in_line(identity: &str) -> String {
    if identity.starts_with('@') {
        identity.to_owned()
    } else {
        short_id(identity)
    }
}

/// "8rd3Ux7pQf…kV2mJ38R" for an identity ID.
pub fn short_id(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= 20 {
        return id.to_string();
    }
    let head: String = chars[..10].iter().collect();
    let tail: String = chars[chars.len() - 8..].iter().collect();
    format!("{head}…{tail}")
}

/// "1 234 567": the interface language's separator between thousands.
pub fn group_thousands(n: i64) -> String {
    let (group, _) = crate::i18n::separators();
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    if n < 0 {
        out.push('−');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(group);
        }
        out.push(c);
    }
    out
}

/// Where a line's money was, for its row and card (FEAT-0030 A3): «Mixed» for a payment from
/// mixed coins and for mixing itself, else its chain.
pub fn source(act: &Activity) -> &'static str {
    use evolite_api::layer::{CORE, MIXED};
    if act.chain == Chain::Core && act.layers & MIXED != 0 && act.layers & CORE == 0 {
        tr!("Mixed")
    } else {
        chain(act.chain)
    }
}

/// The words before the time in a line's row: what a mixing line did (designer, H1), else its
/// [`source`].
pub fn row_place(act: &Activity) -> String {
    match act.kind {
        ActivityKind::Mixing { fee } if fee > 0 => format!(
            tr!("A mixing round: {amount} went to the fee"),
            amount = crate::format::amount(fee)
        ),
        ActivityKind::Mixing { .. } => tr!("A mixing round: the balance did not change").into(),
        ActivityKind::IntoMixing => tr!("Between your own: coins split into denominations").into(),
        // BUG-145 AC-A3 (FEAT-0019 R-17): a token sent has its amount on the right ([`row_amount`])
        // and the credits fee here, hidden with the amounts (AC-A7).
        ActivityKind::TokenSent { .. } => {
            let fee = crate::format::amount(-act.amount);
            let fee = if crate::format::display().hide {
                crate::format::mask(&fee)
            } else {
                fee
            };
            format!(tr!("fee {}"), fee)
        }
        _ => source(act).into(),
    }
}

/// The amount on the right of a line's row: a token sent shows the token's amount, never as DASH
/// (BUG-145 AC-A3, FEAT-0019 R-17); every other line its DASH amount. The token's name is one
/// word to «Hide amounts», so a hidden row reads «••••», not half a name.
pub fn row_amount(act: &Activity) -> String {
    match &act.kind {
        ActivityKind::TokenSent { token, amount, .. } => format!(
            "−{} {}",
            crate::screens::tokens::state::plain_written(amount),
            token.replace(' ', "\u{a0}")
        ),
        _ => crate::format::signed_amount(act.amount),
    }
}

/// The time in a line's row; what an app spent is as read, not as the app wrote (BUG-145 AC-A4).
pub fn row_time(act: &Activity, now: DateTime<Local>) -> String {
    let time = time_of_day(act.at, now);
    match act.kind {
        ActivityKind::AppSpent { .. } => format!(tr!("as read at {}"), time),
        _ => time,
    }
}

/// Our fonts have no "→", so an asset lock reads "Core to Platform".
pub fn chain(c: Chain) -> &'static str {
    match c {
        Chain::Core => "Core",
        Chain::Platform => "Platform",
        Chain::CoreToPlatform => tr!("Core to Platform"),
    }
}

/// The address-book name of the address a payment out went to, when the person named it.
pub fn book_label<'a>(a: &Activity, book: &'a [SavedAddress]) -> Option<&'a str> {
    let ActivityKind::Sent { to } = &a.kind else {
        return None;
    };
    let address = a.address.as_deref().unwrap_or(to);
    book.iter()
        .find(|e| e.address == address)
        .map(|e| e.label.as_str())
}

/// [`activity_title`] with the address-book name in place of an address the person named.
pub fn activity_title_in(a: &Activity, book: &[SavedAddress]) -> String {
    match book_label(a, book) {
        Some(label) => format!(tr!("Sent · {label}"), label = label),
        None => activity_title(a),
    }
}

pub fn activity_title(a: &Activity) -> String {
    match &a.kind {
        // A contact: by its @name, or a short id when it has none.
        ActivityKind::Received { from: Some(from) }
            if from.starts_with('@') || from.contains('…') =>
        {
            format!(tr!("Received from {from}"), from = from)
        }
        ActivityKind::Received { from: Some(from) } => {
            format!(tr!("Received · {from}"), from = from)
        }
        ActivityKind::Received { from: None } => tr!("Received").into(),
        ActivityKind::Sent { to } if to.is_empty() => tr!("Sent").into(),
        ActivityKind::Sent { to } => format!(tr!("Sent {}"), short_address(to)),
        ActivityKind::SelfTransfer { .. } => tr!("Transfer between own addresses").into(),
        // FEAT-0030 AC-A7: made in DashPay or Dash Core; evolite does not mix yet.
        ActivityKind::Mixing { .. } => tr!("Mixing (CoinJoin)").into(),
        ActivityKind::IntoMixing => tr!("Into mixing").into(),
        ActivityKind::PlatformReceived => tr!("Received at the Platform address").into(),
        // An asset lock: an identity's creation or top-up (BUG-57); one the wallet cannot place
        // goes to Platform, nothing more is known.
        // From the wallet's Platform addresses (FEAT-0015 R-11).
        ActivityKind::TopUp { identity, .. } if a.chain == Chain::Platform => format!(
            tr!("Top-up of {} from the Platform address"),
            identity_in_line(identity)
        ),
        // To another address of the wallet's own: only its fee left (QA pass 21).
        ActivityKind::AddressSent { .. } if a.flow == evolite_api::Flow::Between => {
            tr!("Transfer between own addresses").into()
        }
        ActivityKind::AddressSent { .. } => tr!("Sent from the Platform address").into(),
        ActivityKind::AddressWithdrawal { .. } => {
            tr!("Withdrawn to Core from the Platform address").into()
        }
        // FEAT-0015 stage 2 (frame 26).
        ActivityKind::AddressFunded => tr!("Transfer from Core to the Platform address").into(),
        ActivityKind::TopUp {
            identity,
            creates: true,
            ..
        } if identity.is_empty() => tr!("Identity creation").into(),
        ActivityKind::TopUp {
            identity,
            creates: true,
            ..
        } => format!(
            tr!("Creation of the identity {identity}"),
            identity = identity_in_line(identity)
        ),
        ActivityKind::TopUp { identity, .. } if identity.is_empty() => {
            tr!("Transfer to Platform").into()
        }
        ActivityKind::TopUp { identity, .. } => {
            format!(
                tr!("Top-up of {identity}"),
                identity = identity_in_line(identity)
            )
        }
        ActivityKind::AppSignIn { app } => {
            format!(tr!("Signed in to {app} · keys added"), app = app)
        }
        // BUG-145 (the analyst, AC-A1–A3): in pairs with the sign-in; a token's amount is the
        // second line ([`row_place`]), as a credit transfer names only whom.
        ActivityKind::AppRenewed { app } => {
            format!(tr!("Signed in to {app} · keys replaced"), app = app)
        }
        ActivityKind::AppDisconnected { app } => {
            format!(tr!("Signed out of {app} · keys disabled"), app = app)
        }
        ActivityKind::TokenSent { to, .. } => {
            format!(tr!("Token transfer to {}"), short_address(to))
        }
        ActivityKind::AppSpent { app } => format!(tr!("{app} spent from its limit"), app = app),
        ActivityKind::CreditTransfer { to } => {
            format!(tr!("Credit transfer to {}"), short_address(to))
        }
        ActivityKind::Shielded { from: Some(from) } => {
            format!(tr!("Shielded from {from}"), from = from)
        }
        ActivityKind::Shielded { from: None } => tr!("Shielded into the private pool").into(),
        ActivityKind::PoolReceived => tr!("Received privately").into(),
        ActivityKind::PoolSent => tr!("Sent privately").into(),
        ActivityKind::Withdrawal { to, .. } => {
            format!(tr!("Credit withdrawal to {}"), short_address(to))
        }
        ActivityKind::ContactRequest {
            to,
            accepting: true,
            ..
        } => format!(tr!("You accepted the request from {to}"), to = to),
        ActivityKind::ContactRequest { to, .. } => {
            format!(tr!("Contact request to {to}"), to = to)
        }
        // Theirs, answering ours: the request it answered is the line before (BUG-58).
        ActivityKind::ContactRequestIn {
            from,
            state: RequestState::TheyAccepted,
            ..
        } => format!(tr!("Your request was accepted: {from}"), from = from),
        ActivityKind::ContactRequestIn { from, .. } => {
            format!(tr!("Contact request from {from}"), from = from)
        }
        ActivityKind::ProfileSaved => tr!("DashPay profile written").into(),
        ActivityKind::NameRegistered { label } => {
            format!(tr!("The name @{label} is registered"), label = label)
        }
    }
}

/// How a withdrawal or a contact request to us stands, for its row and card (BUG-58); `None`
/// for other operations and before Platform was read.
pub fn activity_status(a: &Activity) -> Option<&'static str> {
    Some(match &a.kind {
        ActivityKind::Withdrawal {
            status: Some(status),
            ..
        }
        | ActivityKind::AddressWithdrawal {
            status: Some(status),
            ..
        } => match status {
            WithdrawalState::Queued | WithdrawalState::Pooled => tr!("In the queue"),
            WithdrawalState::Broadcasted => tr!("Being paid out"),
            WithdrawalState::Complete => tr!("Paid out"),
            WithdrawalState::Expired => tr!("Retrying the payout"),
            WithdrawalState::Failed => tr!("Not paid out"),
        },
        ActivityKind::ContactRequestIn { state, .. } => match state {
            RequestState::Waiting => tr!("Waiting for an answer"),
            RequestState::Accepted => tr!("Accepted"),
            RequestState::TheyAccepted => tr!("You are contacts now"),
            RequestState::Hidden => tr!("Hidden"),
        },
        _ => return None,
    })
}

/// What went wrong with an operation, said in red after its row's line (BUG-57): an asset
/// lock whose credits Platform has not added — its chip is only the Core transaction's.
pub fn activity_alert(a: &Activity) -> Option<&'static str> {
    match &a.kind {
        ActivityKind::TopUp {
            credits: Credits::NotAdded,
            ..
        } => Some(tr!("credits not added")),
        ActivityKind::TopUp {
            credits: Credits::Used,
            ..
        } => Some(tr!("credits: used in Platform")),
        ActivityKind::TopUp {
            credits: Credits::UsedPartly { .. },
            ..
        } => Some(tr!("credits: partly used")),
        ActivityKind::TopUp {
            credits: Credits::NotReceived,
            ..
        } => Some(tr!("Platform has not received these DASH")),
        ActivityKind::TopUp {
            credits: Credits::Unknown,
            ..
        } => Some(tr!("credits: no record")),
        _ => None,
    }
}

/// An asset lock whose credits are not known to have arrived (BUG-57, R-3а): its chip speaks only
/// of the Core transaction.
pub fn core_chip(a: &Activity) -> bool {
    matches!(a.kind, ActivityKind::TopUp { credits, .. } if credits != Credits::Added)
}

/// A lock's credits told calmly (FEAT-0016, BUG-57): muted words, nothing to continue, «Open
/// Identity» to see the credits.
pub fn credits_quiet(a: &Activity) -> bool {
    matches!(
        a.kind,
        ActivityKind::TopUp {
            credits: Credits::Used
                | Credits::UsedPartly { .. }
                | Credits::NotReceived
                | Credits::Unknown,
            ..
        }
    )
}

/// How the value of the card's «Credits» line is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Plain,
    Quiet,
    Bad,
}

/// What Platform's proof means for this lock, under the card's «Credits» line (FEAT-0016).
pub fn credits_explained(a: &Activity) -> Option<String> {
    let ActivityKind::TopUp {
        credits, creates, ..
    } = &a.kind
    else {
        return None;
    };
    Some(match credits {
        Credits::Unknown if *creates => tr!(
            "This identity was created on another computer or its result was not kept here, so evolite does not know whether Platform added the credits. The DASH left Core for good. The identity's credits are on the Identity screen."
        )
        .into(),
        Credits::Unknown => tr!(
            "This top-up was made on another computer or its result was not kept here, so evolite does not know whether Platform added the credits. The DASH left Core for good. The identity's credits are on the Identity screen."
        )
        .into(),
        Credits::Used => tr!(
            "Platform has proved that these DASH were used. It does not record which identity got the credits. The identity's credits are on the Identity screen."
        )
        .into(),
        Credits::UsedPartly { left } => format!(
            tr!(
                "Platform has proved that only part of these DASH was used: {} is left. It does not record which identity got the credits."
            ),
            crate::format::amount(*left)
        ),
        Credits::NotReceived => tr!(
            "Platform has proved that these DASH have not reached it. They left Core for good. There is no unfinished top-up here to continue."
        )
        .into(),
        _ => return None,
    })
}

/// Whether an asset lock's credits came, for its card (BUG-57): the words and whether they
/// are bad news (red). `None` when the wallet does not know, and for other operations.
pub fn activity_credits(a: &Activity) -> Option<(String, Tone)> {
    match &a.kind {
        ActivityKind::TopUp { credits, .. } => match credits {
            Credits::Added => Some((tr!("added").into(), Tone::Plain)),
            Credits::NotAdded => Some((tr!("not added").into(), Tone::Bad)),
            Credits::Unknown => Some((tr!("no record").into(), Tone::Quiet)),
            Credits::Used => Some((tr!("used in Platform").into(), Tone::Quiet)),
            Credits::UsedPartly { left } => Some((
                format!(tr!("partly used, {} left"), crate::format::amount(*left)),
                Tone::Quiet,
            )),
            Credits::NotReceived => Some((
                tr!("Platform has not received these DASH").into(),
                Tone::Quiet,
            )),
        },
        _ => None,
    }
}

/// The identity of ours a request to us came to, when the wallet has several: "for @name";
/// the one ours went from: "from @name" (BUG-69).
pub fn activity_for(a: &Activity) -> Option<String> {
    match &a.kind {
        ActivityKind::ContactRequestIn {
            ours: Some(ours), ..
        } => Some(format!(tr!("for {name}"), name = ours)),
        ActivityKind::ContactRequest {
            ours: Some(ours), ..
        } => Some(format!(tr!("from {name}"), name = ours)),
        _ => None,
    }
}

pub fn activity_icon(a: &Activity) -> &'static str {
    match &a.kind {
        ActivityKind::Received { .. }
        | ActivityKind::PlatformReceived
        | ActivityKind::PoolReceived => "receive",
        ActivityKind::Sent { .. } | ActivityKind::PoolSent => "send",
        ActivityKind::TopUp { .. } => "top-up",
        ActivityKind::SelfTransfer { .. } => "transfer",
        ActivityKind::Mixing { .. } | ActivityKind::IntoMixing => "mixed",
        ActivityKind::AppSignIn { .. } => "app-connect",
        ActivityKind::AppRenewed { .. }
        | ActivityKind::AppDisconnected { .. }
        | ActivityKind::AppSpent { .. } => "app-connect",
        ActivityKind::TokenSent { .. } => "token",
        ActivityKind::CreditTransfer { .. } => "transfer",
        ActivityKind::Shielded { .. } => "shield",
        ActivityKind::Withdrawal { .. } => "send",
        ActivityKind::AddressSent { .. } => "platform-address",
        ActivityKind::AddressWithdrawal { .. } => "withdraw",
        ActivityKind::AddressFunded => "top-up",
        ActivityKind::ContactRequest { .. } | ActivityKind::ContactRequestIn { .. } => "contacts",
        ActivityKind::ProfileSaved => "identity",
        ActivityKind::NameRegistered { .. } => "identity",
    }
}

/// The status chip: icon and label. In plain words; the Dash term (InstantLock, ChainLock,
/// mempool) is in [`proof_hint`], over the chip.
pub fn proof(p: Proof) -> (&'static str, &'static str) {
    match p {
        Proof::InstantLock => ("instantsend", tr!("Confirmed")),
        Proof::ChainLock => ("chainlock", tr!("Final")),
        Proof::InBlock => ("confirmed", tr!("In a block")),
        Proof::Proved => ("confirmed", tr!("Proved")),
        Proof::Mempool => ("pending", tr!("Pending")),
        Proof::Sending => ("pending", tr!("Being sent")),
        Proof::Unconfirmed => ("warning", tr!("Not confirmed by the network")),
    }
}

/// What a status means, with its Dash term: the chip's tooltip.
pub fn proof_hint(p: Proof) -> &'static str {
    match p {
        Proof::InstantLock => tr!(
            "InstantLock: a quorum of masternodes locked this payment within seconds. It cannot be reversed."
        ),
        Proof::ChainLock => {
            tr!("ChainLock: masternodes signed the block with this payment. It is final.")
        }
        Proof::InBlock => tr!("The payment is in a block; the block is not ChainLocked yet."),
        Proof::Proved => {
            tr!(
                "Platform answered with a proof that this computer checked against the quorum keys."
            )
        }
        Proof::Mempool => {
            tr!("In the mempool: the network has the payment, but it is not confirmed yet.")
        }
        Proof::Sending => tr!("The payment was handed to the network; its answer is awaited."),
        Proof::Unconfirmed => {
            tr!("The network has not confirmed this payment. Do not send it again yet.")
        }
    }
}

pub fn key_level(level: KeyLevel) -> &'static str {
    match level {
        KeyLevel::Master => "MASTER",
        KeyLevel::Critical => "CRITICAL",
        KeyLevel::High => "HIGH",
        KeyLevel::Medium => "MEDIUM",
    }
}

pub fn key_purpose(purpose: &KeyPurpose) -> String {
    match purpose {
        KeyPurpose::Identity => tr!("managing the identity").into(),
        KeyPurpose::Documents => tr!("signing documents").into(),
        KeyPurpose::Transfers => tr!("credit transfers").into(),
        KeyPurpose::Encryption => tr!("DashPay encryption").into(),
        KeyPurpose::App(app) => format!("{app} · App Connect"),
    }
}

/// After this long without an answer, the wait for a name is said in words.
pub const NAME_SLOW_AFTER: std::time::Duration = std::time::Duration::from_secs(15);

/// What stands under a name while the core has not answered: «Checking», then, when the answer
/// is long in coming, that it is the network and the name will still appear (BUG-87).
pub fn name_waiting(asked_at: Option<std::time::Instant>) -> &'static str {
    if asked_at.is_some_and(|at| at.elapsed() >= NAME_SLOW_AFTER) {
        tr!(
            "The network is slow to answer. We keep checking: the name appears here as soon as the answer comes."
        )
    } else {
        tr!("Checking the name in DPNS…")
    }
}

/// Why a DPNS name has no answer, in words, and whether the person can fix it by typing
/// (`true`, shown in red) or must wait for the network (`false`, shown muted).
pub fn name_problem(e: &CoreError) -> (String, bool) {
    match e {
        CoreError::NameNotFound => (tr!("There is no such name in DPNS — check the spelling.").into(), true),
        CoreError::BadName => (tr!("This is not a Dash name: 3–63 letters, digits and hyphens").into(), true),
        CoreError::NotSynced => (
            tr!("We check the name once the masternode lists load: without them there is nothing to check Platform's answer with.")
                .into(),
            false,
        ),
        CoreError::NotAvailable(why) => (why.clone(), false),
        other => (format!(tr!("The name is not verified: {}"), error_text(other)), false),
    }
}

/// What went wrong, in words a person reads: never the core's debug form. The words of the
/// network or a library behind a failure are for [`error_details`], not this line.
/// A payment from «Mixed» refused for its excess (FEAT-0030 AC-A12, АРХ-9а), in the analyst's
/// words: too large, with an amount that has none; or changed since the review, to confirm again.
pub fn mixed_excess_words(
    excess: evolite_api::Duffs,
    suggested: Option<evolite_api::Duffs>,
) -> String {
    let excess = crate::format::amount(excess);
    match suggested {
        Some(suggested) => format!(
            tr!(
                "The excess {excess} is too large: mixed coins give no change. Change the amount to {suggested}."
            ),
            excess = excess,
            suggested = crate::format::amount(suggested)
        ),
        None => format!(
            tr!("The excess has changed: {excess} now. Confirm again."),
            excess = excess
        ),
    }
}

pub fn error_text(e: &CoreError) -> String {
    match e {
        // The core's reasons are written for people already.
        CoreError::NotAvailable(why) => why.trim_end_matches('.').to_owned(),
        CoreError::WrongPassword => tr!("wrong password").into(),
        CoreError::BadPhrase => {
            tr!("the phrase does not fit — check the words and their order").into()
        }
        CoreError::NameNotFound => tr!("no such name in DPNS").into(),
        CoreError::BadName => {
            tr!("this is not a Dash name: 3–63 letters, digits and hyphens").into()
        }
        CoreError::BadAmount => tr!("the amount must be more than zero").into(),
        CoreError::BadAddress => match evolite_core::network::current() {
            evolite_api::Network::Testnet => tr!("this is not a Dash testnet address").into(),
            evolite_api::Network::Mainnet => tr!("this is not a Dash mainnet address").into(),
        },
        CoreError::NotEnough => tr!("not enough funds for the amount and the fee").into(),
        CoreError::NotSynced => tr!("wait for the sync to finish").into(),
        CoreError::BadLink => tr!("this is not an app link evolite knows").into(),
        CoreError::TooManyTries { .. } => tr!("too many wrong passwords").into(),
        CoreError::PicturesOff(_) => tr!("not drawn on this system").into(),
        CoreError::AppLink(why) => crate::screens::connect2::refusal_words(why, "app")
            .1
            .trim_end_matches('.')
            .to_owned(),
        CoreError::MixedExcess { excess, suggested } => mixed_excess_words(*excess, *suggested),
        CoreError::Failed(why) => failure(why).into(),
    }
}

/// A failure's own words (the network's, Platform's or a library's), for "Details": they
/// help someone who knows them and say nothing to others. `None` for the other errors, whose
/// line says it all.
pub fn error_details(e: &CoreError) -> Option<String> {
    match e {
        CoreError::Failed(why) if !why.trim().is_empty() => Some(why.trim().to_owned()),
        _ => None,
    }
}

/// A failure in a few words, by what its own text tells of it.
fn failure(why: &str) -> &'static str {
    let why = why.to_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| why.contains(w));
    // The notes file on this computer (FEAT-0003): a disk's failure, not the network's.
    if any(&["notes.enc"]) {
        return if any(&["storage_full"]) {
            tr!("there is no space left on the disk")
        } else {
            tr!("the notes file could not be written on this computer")
        };
    }
    // The same transition is still in the network's cache of seen ones (BUG-7): no verdict.
    if any(&["already exists in cache", "already in mempool"]) {
        return tr!("the network still holds the earlier attempt — try again in a few minutes");
    }
    // Platform took the asset lock before (code 24): the earlier attempt went through.
    if any(&["cannot be reused", "already consumed", "already taken"]) {
        return tr!("Platform had already taken this money on an earlier attempt");
    }
    // A lock made for another identity or purpose (`AssetLockFundingMismatch`).
    if any(&["ineligible"]) {
        return tr!("this locked money was made for another identity or purpose");
    }
    if any(&["timeout", "timed out", "deadline"]) {
        tr!("the network did not answer in time — try again")
    } else if any(&[
        "connect",
        "unreachable",
        "dns",
        "no peers",
        "network",
        "transport",
    ]) {
        tr!("no connection to the network — check your connection")
    } else if any(&["insufficient", "balance"]) {
        tr!("not enough funds for the amount and the fee")
    } else if any(&["not started", "not loaded", "not found"]) {
        tr!("the wallet is not ready yet — wait for the sync")
    } else if any(&["broadcast", "rejected", "consensus", "invalid", "proof"]) {
        tr!("the network rejected the operation")
    } else {
        tr!("the network or Platform returned an error")
    }
}

/// "Not sent: the network did not answer in time — try again": what did not happen, then
/// why.
pub fn failed(what: &str, e: &CoreError) -> String {
    // A template of its own: the catalogs translate both halves and join them as their language
    // does ("{} : {}." in French); the plain join was a sentence no catalog knew.
    format!(tr!("{}: {}."), what, error_text(e))
}

/// "412 MB", "1,2 GB", "640 KB": a size on disk, in binary units as Windows shows them.
pub fn bytes(n: u64) -> String {
    const K: u64 = 1024;
    let (value, unit) = match n {
        n if n >= K * K * K => (n * 10 / (K * K * K), tr!("GB")),
        n if n >= K * K => (n * 10 / (K * K), tr!("MB")),
        n => (n.div_ceil(K) * 10, tr!("KB")),
    };
    // In the interface language: the unit's name and the decimal mark ("1.2 GB").
    let lang = crate::i18n::current();
    let unit = crate::i18n::t_in(lang, unit);
    let (_, decimal) = crate::i18n::separators_in(lang);
    // A decimal only under ten: "1,2 GB", "412 MB".
    if value < 100 && value % 10 != 0 {
        format!("{}{decimal}{} {unit}", value / 10, value % 10)
    } else {
        format!("{} {unit}", value / 10)
    }
}

/// Whether the sync has got as far as the wallet's coins: headers, masternode lists and the
/// filters are done (only the ChainLock of the latest block may be awaited). Until then the
/// balance is not final and a payment could spend coins the wallet does not know about yet.
pub fn coins_known(stage: SyncStage) -> bool {
    matches!(stage, SyncStage::ChainLock | SyncStage::Done)
}

/// Why sending waits: on the disabled «Send» buttons and on the Send screen.
pub const WAIT_FOR_SYNC: &str =
    tr!("Wait for the sync to finish: the wallet does not know all its coins yet.");

/// The running stage and, when the core can count it, how far it is: «Block headers · 47 %»,
/// or just «Masternode lists» — a stage with nothing to count says no number, neither 0 nor
/// 100 % (BUG-60, BUG-61).
pub fn sync_title(sync: &evolite_api::SyncInfo) -> String {
    let stage = sync_stage(sync.stage);
    match sync.progress {
        Some(part) => format!("{stage} · {} %", (part * 100.0).round() as i32),
        None => stage.to_owned(),
    }
}

/// What a sync stage is called on screen.
pub fn sync_stage(stage: SyncStage) -> &'static str {
    match stage {
        SyncStage::Connecting => tr!("Looking for network nodes"),
        SyncStage::Headers => tr!("Block headers"),
        SyncStage::Masternodes => tr!("Masternode lists"),
        SyncStage::Quorums => tr!("Checking masternode quorums"),
        SyncStage::Filters => tr!("Block filters"),
        SyncStage::Transactions => tr!("Your transactions"),
        SyncStage::Platform => tr!("Platform: identities and names"),
        SyncStage::ChainLock => tr!("ChainLock of the latest block"),
        SyncStage::Done => tr!("Done"),
    }
}

/// "+0,2500 DASH", "−1,0000 DASH".
pub fn signed_amount(duffs: i64) -> String {
    crate::format::signed_amount(duffs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    /// A name's read cut off at its limit, in the core's words: what the person reads is that
    /// the network did not answer, and it is not a mistake in the name (BUG-87).
    #[test]
    fn a_name_read_cut_off_says_the_network_did_not_answer_in_time() {
        let (line, can_fix) = name_problem(&CoreError::Failed("timed out".into()));
        assert!(
            line.contains(tr!("the network did not answer in time — try again")),
            "{line}"
        );
        assert!(!can_fix, "typing cannot fix a slow network");
    }

    #[test]
    fn a_failure_and_its_reason_are_both_in_the_interface_language() {
        use crate::i18n::{Lang, t_in};
        let shown = failed(
            tr!("Not topped up"),
            &CoreError::Failed("status 500 from the node".into()),
        );
        assert_eq!(
            shown,
            "Not topped up: the network or Platform returned an error."
        );
        let ru = t_in(Lang::Ru, &shown);
        assert!(
            ru.contains(t_in(Lang::Ru, "Not topped up").as_ref()),
            "{ru}"
        );
        assert!(
            ru.contains(t_in(Lang::Ru, "the network or Platform returned an error").as_ref()),
            "{ru}"
        );
        // A reason the core wrote: one of its own catalog keys.
        let why = CoreError::NotAvailable(
            tr!("First finish the stopped top-up — its money is already locked").into(),
        );
        let shown = failed(tr!("Not topped up"), &why);
        let ru = t_in(Lang::Ru, &shown);
        assert!(!ru.contains("First finish"), "{ru}");
    }

    #[test]
    fn just_now_in_a_card_is_in_the_interface_language() {
        use crate::i18n::{Lang, t_in};
        let now = Local::now();
        let shown = date_time_in(Lang::Ru, now.timestamp(), now);
        assert!(!shown.contains("just now"), "{shown}");
        assert!(
            shown.ends_with(t_in(Lang::Ru, "just now").as_ref()),
            "{shown}"
        );
    }

    /// BUG-47: an operation without a time (a note found in the pool, `at = 0`) is said to have
    /// none — in the row, the card and the day header — never 1 January 1970.
    #[test]
    fn no_time_is_said_in_words_not_as_1970() {
        use crate::i18n::{Lang, t_in};
        let now = Local::now();
        assert_eq!(when(0, now), "time unknown");
        assert_eq!(time_of_day(0, now), "time unknown");
        assert_eq!(day(0, now), "No date");
        assert_eq!(date_time_in(Lang::En, 0, now), "unknown");
        let ru = date_time_in(Lang::Ru, 0, now);
        assert_eq!(ru, t_in(Lang::Ru, "unknown"));
        assert!(!ru.contains("1970") && ru != "unknown", "{ru}");
        // A time of its own is still shown as before.
        assert_eq!(day(now.timestamp(), now), "Today");
        assert!(!when(now.timestamp() - 3600 * 24 * 40, now).contains("unknown"));
    }

    /// FEAT-0035: where two addresses differ — in the middle, at the ends, in several places;
    /// another length marks the whole address.
    #[test]
    fn where_two_addresses_differ() {
        let mine = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5a9f2c";
        assert!(differing(mine, mine).is_empty());
        let middle = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5a9f2c".replacen("D8v", "Q0q", 1);
        assert_eq!(differing(&middle, mine), vec![8, 9, 10]);
        assert_eq!(positions(&differing(&middle, mine)), "9–11");
        // The first and last five characters too.
        let ends = format!("Z{}Z", &mine[1..mine.len() - 1]);
        assert_eq!(differing(&ends, mine), vec![0, mine.len() - 1]);
        assert_eq!(
            positions(&differing(&ends, mine)),
            format!("1, {}", mine.len())
        );
        // Several places.
        assert_eq!(positions(&[3, 4, 10, 20, 21, 22]), "4–5, 11, 21–23");
        // Another length: all of it.
        assert_eq!(differing("yXk3Tq9m", mine), (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn an_address_and_its_lookalikes() {
        let mine = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5a9f2c";
        // Planted: the same four characters at each end, another middle.
        let planted = "yXk3AAAAAAAAAAAAAAAAAAAAAAAAAAAA9f2c";
        assert_eq!(lookalike(planted, [mine, "yNw3aE6u"]), Some(mine));
        assert_eq!(
            lookalike(mine, [mine]),
            None,
            "the same address is not a lookalike"
        );
        assert_eq!(
            lookalike("yQq1BBBBBBBB9f2c", [mine]),
            None,
            "other first characters"
        );
    }

    #[test]
    fn sizes_on_disk() {
        assert_eq!(bytes(412 * 1024 * 1024), "412 MB");
        assert_eq!(bytes(1288 * 1024 * 1024), "1.2 GB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3 GB");
        assert_eq!(bytes(640 * 1024 + 1), "641 KB");
        assert_eq!(bytes(0), "0 KB");
    }

    #[test]
    fn says_every_error_in_words() {
        let every = [
            CoreError::NotAvailable("The wallet is locked.".into()),
            CoreError::WrongPassword,
            CoreError::BadPhrase,
            CoreError::NameNotFound,
            CoreError::BadName,
            CoreError::BadAmount,
            CoreError::BadAddress,
            CoreError::NotEnough,
            CoreError::NotSynced,
            CoreError::Failed("status: Unavailable, message: \"tcp connect error\"".into()),
            CoreError::BadLink,
            CoreError::TooManyTries { until: 0 },
        ];
        for e in &every {
            let line = error_text(e);
            assert!(!line.is_empty(), "{e:?}");
            // No debug form, no variant names, no library text on the line itself.
            for bad in ["(", "Failed", "NotAvailable", "status:", "\""] {
                assert!(!line.contains(bad), "{e:?} -> {line}");
            }
        }
        assert_eq!(
            error_text(&every[0]),
            tr!("The wallet is locked"),
            "the core's own words, no dot"
        );
        assert_eq!(
            failed(tr!("Not sent"), &CoreError::NotEnough),
            "Not sent: not enough funds for the amount and the fee."
        );
    }

    #[test]
    fn tells_failures_apart_and_keeps_their_words_for_details() {
        let f = |why: &str| error_text(&CoreError::Failed(why.into()));
        assert_eq!(
            f("request timed out"),
            tr!("the network did not answer in time — try again")
        );
        assert_eq!(
            f("tcp connect error"),
            tr!("no connection to the network — check your connection")
        );
        assert_eq!(
            f("Insufficient funds"),
            tr!("not enough funds for the amount and the fee")
        );
        assert_eq!(
            f("the chain is not started"),
            tr!("the wallet is not ready yet — wait for the sync")
        );
        assert_eq!(
            f("transaction broadcast rejected: dust"),
            tr!("the network rejected the operation")
        );
        assert_eq!(
            f("something else"),
            tr!("the network or Platform returned an error")
        );
        assert_eq!(
            error_details(&CoreError::Failed(" request timed out ".into())).as_deref(),
            Some("request timed out")
        );
        assert_eq!(error_details(&CoreError::WrongPassword), None);
        assert_eq!(error_details(&CoreError::Failed("  ".into())), None);
    }

    /// BUG-7: the texts of platform-wallet's asset-lock errors (`error.rs` at 575d961) get
    /// their own words, not "the network or Platform returned an error".
    #[test]
    fn tells_asset_lock_failures_apart() {
        let f = |why: &str| error_text(&CoreError::Failed(why.into()));
        assert_eq!(
            f("SDK error: Object already exists: tx already exists in cache"),
            tr!("the network still holds the earlier attempt — try again in a few minutes")
        );
        let taken = tr!("Platform had already taken this money on an earlier attempt");
        for why in [
            "Asset lock 0a1b…:0 cannot be reused; Platform completion is unconfirmed",
            "Asset lock was already consumed",
        ] {
            assert_eq!(f(why), taken, "{why}");
        }
        assert_eq!(
            f(
                "Asset lock 0a1b…:0 is ineligible for IdentityTopUp index 2: it is IdentityTopUp index 0"
            ),
            tr!("this locked money was made for another identity or purpose")
        );
    }

    /// BUG-77: a Core balance checked earlier says when — today by the time, yesterday so,
    /// earlier by the date and the time; one never checked says so; one checked now says
    /// nothing.
    #[test]
    fn an_earlier_core_balance_says_when_it_was_checked() {
        use evolite_api::CoreAsOf;
        let now = Local.with_ymd_and_hms(2026, 11, 7, 12, 0, 0).unwrap();
        let at = |d: u32, h: u32, m: u32| {
            Local
                .with_ymd_and_hms(2026, 11, 7 - d, h, m, 0)
                .unwrap()
                .timestamp()
        };
        let f = |a| core_as_of(Some(CoreAsOf::Checked(a)), now).unwrap();
        assert_eq!(core_as_of(None, now), None);
        assert_eq!(
            core_as_of(Some(CoreAsOf::NotYet), now).as_deref(),
            Some(tr!("Not known yet"))
        );
        assert_eq!(f(at(0, 9, 30)), "as of 09:30");
        assert_eq!(f(at(1, 23, 41)), "as of 23:41 yesterday");
        let older = f(at(3, 9, 30));
        assert!(
            older.starts_with("as of ") && older.ends_with(", 09:30") && older.contains('4'),
            "{older}"
        );
    }

    #[test]
    fn says_when_relative_to_now() {
        let now = Local::now();
        let ts = |d: Duration| (now - d).timestamp();
        assert_eq!(when(ts(Duration::seconds(5)), now), tr!("just now"));
        assert_eq!(day(ts(Duration::seconds(5)), now), tr!("Today"));
        assert_eq!(when(ts(Duration::days(1)), now), tr!("yesterday"));
        assert_eq!(day(ts(Duration::days(1)), now), tr!("Yesterday"));
    }

    #[test]
    fn shortens_addresses_and_ids() {
        assert_eq!(
            short_address("yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5a9f2c"),
            "yXk3…9f2c"
        );
        assert_eq!(short_address("@alice"), "@alice");
        assert_eq!(
            short_id("8rd3Ux7pQfZ2cW9yLmT4aB6nE1sK5gH3dR7vXkV2mJ38R"),
            "8rd3Ux7pQf…kV2mJ38R"
        );
        // By characters, not bytes: text that is not Base58 does not split a letter.
        assert_eq!(
            short_id("identifier-not-from-the-core-1234"),
            "identifier…ore-1234"
        );
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(group_thousands(120_000_000_000), "120,000,000,000");
        assert_eq!(group_thousands(12), "12");
    }
}

/// What a key is for, with the key that reads DashPay requests told apart from the one they
/// are encrypted to.
pub fn key_role(key: &evolite_api::IdentityKey) -> String {
    match &key.purpose {
        KeyPurpose::Encryption if key.decrypts => tr!("DashPay decryption").into(),
        purpose => key_purpose(purpose),
    }
}

/// A key's signature scheme, short, for the end of its row.
pub fn key_kind(kind: evolite_api::KeyKind) -> &'static str {
    match kind {
        evolite_api::KeyKind::Ecdsa => "ECDSA",
        evolite_api::KeyKind::EcdsaHash160 => "ECDSA · hash160",
        evolite_api::KeyKind::Bls => "BLS",
        evolite_api::KeyKind::EddsaHash160 => "EdDSA · hash160",
        evolite_api::KeyKind::Bip13Script => tr!("BIP13 script"),
    }
}

#[cfg(test)]
mod key_label_tests {
    use super::{key_kind, key_role};
    use evolite_api::{IdentityKey, KeyKind, KeyLevel, KeyPurpose};

    #[test]
    fn dashpay_keys_are_told_apart_and_kinds_named() {
        let key = |decrypts| IdentityKey {
            id: 4,
            level: KeyLevel::Medium,
            purpose: KeyPurpose::Encryption,
            kind: KeyKind::Ecdsa,
            decrypts,
        };
        assert_eq!(key_role(&key(false)), tr!("DashPay encryption"));
        assert_eq!(key_role(&key(true)), tr!("DashPay decryption"));
        assert_eq!(key_kind(KeyKind::Bls), "BLS");
        assert_eq!(key_kind(KeyKind::EcdsaHash160), "ECDSA · hash160");
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;
    use evolite_api::{Chain, Flow, Proof};

    fn lock(credits: Credits) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::TopUp {
                identity: String::new(),
                creates: false,
                credits,
            },
            chain: Chain::CoreToPlatform,
            at: 1,
            amount: -1,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers: 0,
            flow: Flow::Between,
            key: String::new(),
            note: None,
            rate: None,
            detail: None,
        }
    }

    /// QA pass 21 (3): a transfer from the Platform addresses to another of the wallet's own is
    /// named as one between own addresses; to someone else it stays «Sent».
    #[test]
    fn a_transfer_between_own_platform_addresses_is_named_so() {
        let mut line = Activity {
            kind: ActivityKind::AddressSent {
                to: "tdash1own".into(),
            },
            chain: Chain::Platform,
            ..lock(Credits::Added)
        };
        assert_eq!(activity_title(&line), "Transfer between own addresses");
        line.flow = Flow::Out;
        assert_eq!(activity_title(&line), "Sent from the Platform address");
    }

    /// R-3а, BUG-57: the chip names Core for every lock not known to be added; only a lock not
    /// added is red and can be continued; the others are said calmly.
    #[test]
    fn every_lock_not_added_names_core_and_only_one_is_red() {
        assert!(!core_chip(&lock(Credits::Added)));
        for credits in [
            Credits::NotAdded,
            Credits::Unknown,
            Credits::Used,
            Credits::UsedPartly { left: 1 },
            Credits::NotReceived,
        ] {
            let line = lock(credits);
            assert!(core_chip(&line), "{credits:?}");
            assert_eq!(
                credits_quiet(&line),
                credits != Credits::NotAdded,
                "{credits:?}"
            );
            let tone = activity_credits(&line).map(|(_, tone)| tone);
            assert_eq!(
                tone == Some(Tone::Bad),
                credits == Credits::NotAdded,
                "{credits:?}"
            );
        }
    }

    /// BUG-118 (2): a date and its time, and a list of word numbers, are joined by the
    /// language's own comma — «،» in Arabic. Every other language keeps exactly what it had:
    /// «, » (the kickoff's condition, all 15 languages checked).
    #[test]
    fn dates_and_lists_take_the_languages_comma_and_only_arabic_changes() {
        use crate::i18n::{Lang, set, t_in};
        let now = Local.with_ymd_and_hms(2026, 11, 8, 23, 0, 0).unwrap();
        let at = Local
            .with_ymd_and_hms(2026, 11, 8, 22, 11, 0)
            .unwrap()
            .timestamp();
        for lang in Lang::ALL {
            set(lang);
            let shown = date_time(at, now);
            let list = positions(&[3, 4, 10]);
            let before = format!("{}, {}", date_long(at), t_in(lang, "22:11"));
            set(Lang::En);
            if lang == Lang::Ar {
                assert!(shown.contains("، 22:11") && !shown.contains(','), "{shown}");
                assert_eq!(list, "4–5، 11");
            } else {
                assert_eq!(shown, before, "{lang:?}");
                assert_eq!(list, "4–5, 11", "{lang:?}");
            }
        }
    }
}
