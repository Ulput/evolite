//! What the core sends as data, in words: dates and times in local time, operation titles,
//! short addresses, key levels. Russian only for now; the strings live here so a translation
//! has one place to go.

use chrono::{DateTime, Datelike, Local, TimeZone};
use evolite_api::{
    Activity, ActivityKind, Chain, CoreError, KeyLevel, KeyPurpose, Proof, SavedAddress, SyncStage,
    Timestamp,
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

/// Whether an operation has a time of its own. A note found by reading the shielded pool has
/// none (`at = 0`): that is said in words, never shown as 1 January 1970 (BUG-47).
pub fn known(at: Timestamp) -> bool {
    at > 0
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
    format!("{}, {}", date_long(at), crate::i18n::t_in(lang, &time))
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

/// An address in groups of four ("yXk3 Tq9m D8vJ …"), to compare with the one meant: only
/// for the eye, never copied this way.
pub fn grouped(address: &str) -> String {
    let chars: Vec<char> = address.chars().collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
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
        ActivityKind::SelfTransfer => tr!("Transfer between own addresses").into(),
        ActivityKind::PlatformReceived => tr!("Received at the Platform address").into(),
        // An asset lock seen on Core: which identity or address it funds, Core does not say.
        ActivityKind::TopUp { identity } if identity.is_empty() => {
            tr!("Transfer to Platform").into()
        }
        ActivityKind::TopUp { identity } => {
            format!(tr!("Top-up of {identity}"), identity = identity)
        }
        ActivityKind::AppSignIn { app } => {
            format!(tr!("Signed in to {app} · keys added"), app = app)
        }
        ActivityKind::CreditTransfer { to } => {
            format!(tr!("Credit transfer to {}"), short_address(to))
        }
        ActivityKind::Shielded { from: Some(from) } => {
            format!(tr!("Shielded from {from}"), from = from)
        }
        ActivityKind::Shielded { from: None } => tr!("Shielded into the private pool").into(),
        ActivityKind::PoolReceived => tr!("Received privately").into(),
        ActivityKind::PoolSent => tr!("Sent privately").into(),
        ActivityKind::Withdrawal { to } => {
            format!(tr!("Credit withdrawal to {}"), short_address(to))
        }
        ActivityKind::ContactRequest { to } => format!(tr!("Contact request to {to}"), to = to),
        ActivityKind::ProfileSaved => tr!("DashPay profile written").into(),
        ActivityKind::NameRegistered { label } => {
            format!(tr!("The name @{label} is registered"), label = label)
        }
    }
}

pub fn activity_icon(a: &Activity) -> &'static str {
    match &a.kind {
        ActivityKind::Received { .. }
        | ActivityKind::PlatformReceived
        | ActivityKind::PoolReceived => "receive",
        ActivityKind::Sent { .. } | ActivityKind::PoolSent => "send",
        ActivityKind::TopUp { .. } => "top-up",
        ActivityKind::SelfTransfer => "transfer",
        ActivityKind::AppSignIn { .. } => "app-connect",
        ActivityKind::CreditTransfer { .. } => "transfer",
        ActivityKind::Shielded { .. } => "shield",
        ActivityKind::Withdrawal { .. } => "send",
        ActivityKind::ContactRequest { .. } => "contacts",
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

/// What a sync stage is called on screen.
pub fn sync_stage(stage: SyncStage) -> &'static str {
    match stage {
        SyncStage::Connecting => tr!("Looking for network nodes"),
        SyncStage::Headers => tr!("Block headers"),
        SyncStage::Masternodes => tr!("Masternode lists"),
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

    #[test]
    fn an_address_in_groups_and_its_lookalikes() {
        let mine = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5a9f2c";
        assert_eq!(grouped("yXk3Tq9mD8"), "yXk3 Tq9m D8");
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
