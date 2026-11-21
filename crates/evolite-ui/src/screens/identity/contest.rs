//! The words of contested names (FEAT-0021): the answer of a name check under the field (N1), the
//! line under a name in «DPNS names» (N4), and why a claim was refused. Durations and the fund come
//! from the core's [`Fees`] (R-5, AC-T5), never from figures here.

use evolite_api::{DpnsName, Fees, IdentityChoice, NameCheck, NameState, Timestamp};

use crate::text;

/// How a check's answer is drawn (N1): free in the accent, claimable as text, refused in danger,
/// already ours muted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Free,
    Claimable,
    Refused,
    Ours,
}

/// The colour of an answer of `tone` (N1).
pub fn tone_color(tone: Tone, p: &crate::theme::Palette) -> egui::Color32 {
    match tone {
        Tone::Free => p.accent,
        Tone::Claimable => p.text,
        Tone::Refused => p.danger,
        Tone::Ours => p.muted,
    }
}

/// «14 days», «90 minutes»: a vote's span in the words of the language.
pub fn span(secs: u64) -> String {
    let days = secs / 86_400;
    if days >= 1 {
        text::plural(days as u32, tr!("day"), tr!("days"))
    } else {
        text::plural((secs / 60).max(1) as u32, tr!("minute"), tr!("minutes"))
    }
}

/// R-6, under a contested answer: one name whatever the look-alike letters and the case.
pub fn similar_letters() -> &'static str {
    tr!(
        "Similar letters are one name: o and 0, i, l and 1; capital and small letters are the same."
    )
}

/// The answer under the name field for `label` (N1): the words and their tone. `names` — the shown
/// identity's own names (one of them is no news: «already a name of this identity»); `identities`
/// — the wallet's, for the one that already asked (R-7).
pub fn check_line(
    label: &str,
    check: NameCheck,
    fees: &Fees,
    names: &[DpnsName],
    identities: &[IdentityChoice],
    now: chrono::DateTime<chrono::Local>,
) -> (String, Tone) {
    let fund = crate::format::amount(fees.name_contest_fund);
    let end_of = |join_until: Timestamp| {
        join_until - fees.name_join_secs as i64 + fees.name_vote_secs as i64
    };
    let ours = names.iter().any(|n| {
        n.state.owned()
            && evolite_core::dpns::normalized(&n.label) == evolite_core::dpns::normalized(label)
    });
    match check {
        NameCheck::Available => (
            format!(
                tr!("@{label} is free · a regular name, no vote"),
                label = label
            ),
            Tone::Free,
        ),
        NameCheck::NeedsVote => (
            format!(
                tr!(
                    "@{label} is a contested name: masternodes award it by vote. Fund {fund}, result in about {span}."
                ),
                label = label,
                fund = fund,
                span = span(fees.name_vote_secs)
            ),
            Tone::Claimable,
        ),
        NameCheck::Joinable {
            contenders,
            join_until,
        } => (
            format!(
                tr!(
                    "@{label} is already contested: {contenders}. You can join until {until} · result about {end}."
                ),
                label = label,
                contenders = text::plural(contenders, tr!("contender"), tr!("contenders")),
                until = text::date_time_in_line(join_until, now),
                end = text::date_day_in_line(end_of(join_until), now)
            ),
            Tone::Claimable,
        ),
        NameCheck::JoinClosed {
            ends_about: Some(end),
        } => (
            format!(
                tr!(
                    "@{label} is contested, but it can no longer be joined: the window closed on {closed}. Result about {end}."
                ),
                label = label,
                closed = text::date_day_in_line(
                    end - fees.name_vote_secs as i64 + fees.name_join_secs as i64,
                    now
                ),
                end = text::date_day_in_line(end, now)
            ),
            Tone::Refused,
        ),
        NameCheck::JoinClosed { ends_about: None } => (
            format!(
                tr!("@{label} is contested, but it can no longer be joined."),
                label = label
            ),
            Tone::Refused,
        ),
        NameCheck::Taken if ours => (
            format!(
                tr!("@{label} is already a name of this identity"),
                label = label
            ),
            Tone::Ours,
        ),
        NameCheck::Taken => (
            format!(tr!("@{label} is already taken"), label = label),
            Tone::Refused,
        ),
        NameCheck::Locked => (
            format!(
                tr!("@{label} is blocked by the masternodes — it cannot be claimed"),
                label = label
            ),
            Tone::Refused,
        ),
        NameCheck::AlreadyRequested { other: None } => (
            format!(
                tr!(
                    "You are already in the vote for @{label} from this identity. Follow it in “DPNS names”."
                ),
                label = label
            ),
            Tone::Ours,
        ),
        NameCheck::AlreadyRequested { other: Some(slot) } => {
            let other = identities
                .iter()
                .find(|i| i.slot == slot)
                .map(|i| match &i.name {
                    Some(name) => format!("@{}", crate::i18n::own(name)),
                    None => text::short_id(&i.id),
                })
                .unwrap_or_default();
            (
                format!(
                    tr!(
                        "@{label} is already in a vote for your identity {other}: two identities of one wallet don't claim one name"
                    ),
                    label = label,
                    other = other
                ),
                Tone::Refused,
            )
        }
    }
}

/// The line under a name in «DPNS names» (N4): registered, in the vote and for how long, waiting
/// for the result, or how it ended — with the fund that was not returned (R-12, AC-T20).
pub fn state_line(name: &DpnsName, fees: &Fees, now: Timestamp) -> String {
    let fund = crate::format::amount(fees.name_contest_fund);
    match name.state {
        NameState::Registered | NameState::Won { .. } if name.primary => {
            tr!("main · verified").into()
        }
        NameState::Registered | NameState::Won { .. } => tr!("verified").into(),
        NameState::Voting {
            ends_about,
            contenders,
            ours,
            ..
        } => format!(
            tr!("masternode vote · {left} left · {contenders} · {ours} for you"),
            left = span((ends_about - now).max(60) as u64),
            contenders = text::plural(contenders, tr!("contender"), tr!("contenders")),
            ours = text::plural(ours, tr!("vote"), tr!("votes"))
        ),
        NameState::AwaitingResult { .. } => tr!("the vote is over · waiting for the result").into(),
        NameState::Lost { at } => format!(
            tr!("went to another identity · the fund {fund} was not returned · {hides}"),
            fund = fund,
            hides = hides_in(at, now)
        ),
        NameState::Locked { at } => format!(
            tr!("blocked by the masternodes · the fund {fund} was not returned · {hides}"),
            fund = fund,
            hides = hides_in(at, now)
        ),
    }
}

/// Beside the state's chip in the contest card (N5, N5b): how long the vote still runs, that it
/// ended, or when it ended and, while a lost or blocked name is still listed, when it leaves the
/// list (R-12).
pub fn when_line(name: &DpnsName, now: Timestamp) -> String {
    let date = |at| text::date_day_in_line(at, chrono::Local::now());
    match name.state {
        NameState::Voting { ends_about, .. } => format!(
            tr!("{left} left"),
            left = span((ends_about - now).max(60) as u64)
        ),
        NameState::AwaitingResult { .. } => tr!("the vote is over, waiting for the result").into(),
        NameState::Lost { at } | NameState::Locked { at } if name.listed() => {
            format!("{} · {}", date(at), hides_in(at, now))
        }
        NameState::Won { at } | NameState::Lost { at } | NameState::Locked { at } => date(at),
        NameState::Registered => String::new(),
    }
}

/// When a lost or blocked name leaves «DPNS names» (R-12): 30 days after its result.
pub fn hides_in(result_at: Timestamp, now: Timestamp) -> String {
    let left_days = (result_at + 30 * 86_400 - now).max(0) / 86_400;
    match left_days {
        0 => tr!("leaves the list today").into(),
        1 => tr!("leaves the list tomorrow").into(),
        n => format!(
            tr!("leaves the list in {days}"),
            days = text::plural(n as u32, tr!("day"), tr!("days"))
        ),
    }
}

/// BUG-227: which claim Overview's notice tells of: a vote under way before any result, then
/// the latest result.
fn notice_rank(state: NameState) -> (bool, Timestamp) {
    match state {
        NameState::Voting { ends_about, .. } => (true, ends_about),
        NameState::Won { at } | NameState::Lost { at } | NameState::Locked { at } => (false, at),
        _ => (false, 0),
    }
}

/// Overview's notice of a claim of the shown identity (FEAT-0021 V2, entry 2): while it is in its
/// vote, how long, when the result comes and the votes for it; once it has a result, the result.
/// Both with «Open the vote». «×» closes each until the next: the vote's until the result, kept
/// across restarts in `name-requests.json` (review r1, Should 1); the result's for good.
pub fn overview_notice(
    ui: &mut egui::Ui,
    a: &mut crate::components::Assets,
    cx: &mut crate::context::Ctx,
) {
    let Some(identity) = cx.wallet().identity.clone() else {
        return;
    };
    let now = cx.now;
    let shown = |n: &&DpnsName| {
        let Some(c) = &n.contest else {
            return false;
        };
        match n.state {
            NameState::Voting { .. } => !c.notice_closed,
            NameState::Won { .. } | NameState::Lost { .. } | NameState::Locked { .. } => {
                c.listed && !c.result_notice_closed
            }
            _ => false,
        }
    };
    // BUG-227: one notice — a vote under way first, else the latest result: not the first
    // name in the list (an older result stayed after a newer name was won).
    let Some(name) = identity
        .names
        .iter()
        .filter(shown)
        .max_by_key(|n| notice_rank(n.state))
    else {
        return;
    };
    let (title, body) = match name.state {
        NameState::Voting {
            ends_about, ours, ..
        } => {
            let all: u32 = name
                .contest
                .as_ref()
                .map_or(0, |c| c.contenders.iter().map(|(_, v)| *v).sum());
            (
                format!(
                    tr!("The name @{label} is in the masternode vote · {left} left"),
                    label = name.label,
                    left = span((ends_about - now.timestamp()).max(60) as u64)
                ),
                format!(
                    tr!("Result about {date}. {ours} of {all} are for you."),
                    // Kept whole: «29. November» in German has a «. » of its own (B1).
                    date = crate::i18n::own(&text::date_time_in_line(ends_about, now)),
                    ours = text::plural(ours, tr!("vote"), tr!("votes")),
                    all = all
                ),
            )
        }
        state => {
            let Some(note) = crate::notify::name_contest(&name.label, state) else {
                return;
            };
            (note.title, note.body)
        }
    };
    let answer = crate::components::notice_closable(
        ui,
        a,
        "vote",
        &title,
        &body,
        Some(("chevron-right", tr!("Open the vote"))),
    );
    if answer.action {
        cx.open_contest(&identity.id, &name.label);
    }
    if answer.closed {
        cx.send(evolite_api::Command::CloseNameNotice {
            identity: identity.id.clone(),
            label: name.label.clone(),
        });
    }
}

/// Why a claim was refused, without the name (the error's line).
pub fn unavailable(check: NameCheck) -> &'static str {
    match check {
        NameCheck::Taken => tr!("the name is already taken"),
        NameCheck::Locked => tr!("the name is blocked by the masternodes"),
        NameCheck::JoinClosed { .. } => tr!("the vote for the name takes no more claims"),
        NameCheck::AlreadyRequested { .. } => tr!("this wallet already asked for the name"),
        NameCheck::Available | NameCheck::NeedsVote | NameCheck::Joinable { .. } => {
            tr!("the name cannot be claimed now")
        }
    }
}

/// The fund changed between the review and the signature (R-2): nothing was sent.
pub fn fund_changed(fund: evolite_api::Duffs) -> String {
    format!(
        tr!("the vote's fund is now {fund}: check the name again"),
        fund = crate::format::amount(fund)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-T5 on screen: the vote's span is the core's, in days on mainnet and minutes on testnet.
    #[test]
    fn a_votes_span_comes_from_the_core_in_its_own_unit() {
        crate::i18n::set(crate::i18n::Lang::En);
        assert_eq!(span(14 * 86_400), "14 days");
        assert_eq!(span(90 * 60), "90 minutes");
        assert_eq!(span(1), "1 minute");
    }

    /// BUG-227: two results, the older first in the list: the notice tells of the latest; a
    /// vote under way comes before any result.
    #[test]
    fn the_notice_tells_of_the_latest_result() {
        let older = NameState::Won { at: 1_000 };
        let newer = NameState::Won { at: 2_000 };
        let mut states = [older, newer];
        assert_eq!(
            states.iter().copied().max_by_key(|s| notice_rank(*s)),
            Some(newer)
        );
        states.reverse();
        assert_eq!(
            states.iter().copied().max_by_key(|s| notice_rank(*s)),
            Some(newer)
        );
        let voting = NameState::Voting {
            join_until: 500,
            ends_about: 600,
            contenders: 2,
            ours: 0,
            lock: 0,
            abstain: 0,
        };
        assert!(notice_rank(voting) > notice_rank(newer));
    }

    /// BUG-224: «New identity»'s note on contested names, in Russian, with the network's span
    /// inside: «длительностью 90 минут», not «90 minutes».
    #[test]
    fn the_contested_note_says_its_span_in_the_language() {
        use crate::i18n::{Lang, t_in};
        crate::i18n::set(Lang::Ru);
        let line = format!(
            tr!(
                "Names shorter than 20 characters made only of letters, 0, 1 and hyphens are contested: masternodes award them by a vote of {span}, and they cost more. A digit from 2 to 9 makes a name regular."
            ),
            span = span(90 * 60)
        );
        let shown = t_in(Lang::Ru, &line);
        crate::i18n::set(Lang::En);
        assert!(shown.contains("90 минут"), "{shown}");
        assert!(!shown.contains("minutes"), "{shown}");
    }

    /// BUG-218: N5's «Others may join» in Russian is translated whole, the date and the span
    /// inside it: not «ещё until 18 ноября…». The date is in Russian already, so it goes in
    /// marked as own words (BUG-189); the span is English until the catalog translates it at
    /// display, through its own key, so it goes in as it is.
    #[test]
    fn the_join_line_is_translated_whole() {
        use crate::i18n::{Lang, t_in};
        crate::i18n::set(Lang::Ru);
        let now = chrono::Local::now();
        let until = (now + chrono::Duration::days(2)).timestamp();
        let line = format!(
            tr!("until {date} · {left} left"),
            date = text::date_time_in_line(until, now),
            left = span((until - now.timestamp()) as u64)
        );
        let shown = t_in(Lang::Ru, &line);
        crate::i18n::set(Lang::En);
        assert!(shown.starts_with("до "), "{shown}");
        for english in ["until", "left", "days"] {
            assert!(!shown.contains(english), "{shown}");
        }
    }
}
