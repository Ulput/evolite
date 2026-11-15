//! The contest card (FEAT-0021 N5, N5b): one claim's masternode vote — where it stands, when it
//! ends, the fund, every claim with its votes (this identity's marked), the votes to block and to
//! abstain, the rule of the result and when the votes were proved. With a lost or blocked result
//! still listed: «Claim another name» and «Hide from the list» (R-12). Every entry opens it
//! through [`IdentityState::open_contest`] (AC-T21).

use super::*;
use crate::components::{Part, dialog_with_footer};
use evolite_api::NameState;

/// The card, when one is open. Closes itself when its name is no longer the shown identity's.
pub(super) fn contest_card(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut IdentityState,
    id: &Identity,
    asked: &mut Vec<Command>,
) {
    let Some((identity, label)) = state.contest.clone() else {
        return;
    };
    let name = (identity == id.id)
        .then(|| id.names.iter().find(|n| n.label == label))
        .flatten();
    let Some(name) = name else {
        state.contest = None;
        return;
    };
    let contest = name.contest.clone().unwrap_or_default();
    let finished = matches!(
        name.state,
        NameState::Lost { .. } | NameState::Locked { .. } | NameState::Won { .. }
    );
    let title = if finished {
        format!(tr!("@{label} — result of the vote"), label = label)
    } else {
        format!(tr!("@{label} — vote"), label = label)
    };
    let now = chrono::Local::now();
    let can_hide = matches!(
        name.state,
        NameState::Lost { .. } | NameState::Locked { .. }
    ) && name.listed();
    let (mut another, mut hide, mut relink) = (false, false, false);
    // АРХ-6, AC-T12: a link the claim kept but Platform did not take, written again alone.
    let retry = contest
        .link
        .clone()
        .filter(|_| !contest.link_written && name.state.open());
    let password = &mut state.link_password;
    let close = dialog_with_footer(ui, a, "contest", "vote", &title, 520.0, |ui, a, part| {
        let p = a.p;
        if part == Part::Footer {
            let mut close = false;
            if retry.is_some() && password.field(ui, &p, "contest-link-password", true) {
                relink = true;
            }
            button_row(ui, |ui| {
                if retry.is_some()
                    && button_enabled(
                        ui,
                        a,
                        tr!("Write the link again"),
                        ButtonKind::Secondary,
                        None,
                        password.ready(true),
                    )
                    .clicked()
                {
                    relink = true;
                }
                if can_hide
                    && button(ui, a, tr!("Hide from the list"), ButtonKind::Ghost, None).clicked()
                {
                    hide = true;
                }
                if finished
                    && !matches!(name.state, NameState::Won { .. })
                    && button(ui, a, tr!("Claim another name"), ButtonKind::Ghost, None).clicked()
                {
                    another = true;
                }
                if button(ui, a, tr!("Close"), ButtonKind::Primary, None).clicked() {
                    close = true;
                }
            });
            return close;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (icon, chip_label, verified) = state_chip(name.state);
            crate::components::chip(ui, a, icon, chip_label, verified);
            ui.label(text(
                &super::contest::when_line(name, now.timestamp()),
                SMALL,
                p.muted,
            ));
        });
        let value = style(UI.weight(Weight::Medium));
        match name.state {
            NameState::Voting {
                join_until,
                ends_about,
                ..
            } => {
                kv_row(
                    ui,
                    &p,
                    tr!("Result"),
                    &format!("≈ {}", crate::text::date_time(ends_about, now)),
                    value.clone(),
                );
                let join = if join_until > now.timestamp() {
                    format!(
                        tr!("until {date} · {left} left"),
                        date = crate::text::date_time(join_until, now),
                        left = super::contest::span((join_until - now.timestamp()) as u64)
                    )
                } else {
                    tr!("closed").to_string()
                };
                kv_row(ui, &p, tr!("Others may join"), &join, value.clone());
            }
            NameState::AwaitingResult { ends_about } => kv_row(
                ui,
                &p,
                tr!("Result"),
                &format!(
                    tr!("the vote ended about {date}, waiting for the result"),
                    date = crate::text::date_time(ends_about, now)
                ),
                value.clone(),
            ),
            NameState::Won { at } | NameState::Lost { at } | NameState::Locked { at } => kv_row(
                ui,
                &p,
                tr!("Result"),
                &crate::text::date_day(at, now),
                value.clone(),
            ),
            NameState::Registered => {}
        }
        kv_row(
            ui,
            &p,
            tr!("Fund"),
            &format!(
                tr!("{fund} · paid, not returned"),
                fund = dash(contest.fund)
            ),
            value.clone(),
        );
        // АРХ-6: the claim's link, and whether Platform has it (AC-T12).
        if let Some(link) = &contest.link {
            kv_row(ui, &p, tr!("Identity confirmation"), link, value.clone());
            if !contest.link_written {
                ui.add(
                    crate::components::para(
                        tr!("The request is in; the link was not written. Writing it again pays only the link, never the fund again."),
                        SMALL.size,
                        SMALL.weight,
                        p.danger,
                    )
                    .line_height(SMALL.line),
                );
            }
        }
        divider(ui, &p);
        ui.label(text(tr!("Contenders"), SMALL, p.muted));
        let mut claims = contest.contenders.clone();
        claims.sort_by_key(|(_, votes)| std::cmp::Reverse(*votes));
        for (who, votes) in &claims {
            let ours = *who == id.id;
            let who_text = if ours {
                format!(
                    tr!("{name} · your identity"),
                    name = id
                        .name()
                        .map_or_else(|| crate::text::short_id(&id.id), |n| format!("@{n}"))
                )
            } else {
                crate::text::short_id(who)
            };
            let mut votes_text = crate::text::plural(*votes, tr!("vote"), tr!("votes"));
            if contest.winner.as_deref() == Some(who.as_str()) {
                votes_text = format!(tr!("{votes} · got the name"), votes = votes_text);
            }
            kv_row(ui, &p, &who_text, &votes_text, value.clone());
        }
        if let NameState::Voting { lock, abstain, .. } = name.state {
            kv_row(
                ui,
                &p,
                tr!("Against all (block)"),
                &crate::text::plural(lock, tr!("vote"), tr!("votes")),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Abstained"),
                &crate::text::plural(abstain, tr!("vote"), tr!("votes")),
                value.clone(),
            );
        }
        notice(
            ui,
            a,
            "info",
            p.muted,
            tr!(
                "The most votes win. If more vote against all than for the leader, the name is blocked. On a tie the earliest request gets it."
            ),
        );
        if let Some(at) = contest.checked_at {
            ui.label(text(
                &format!(
                    tr!("Votes proved by Platform · updated {time}"),
                    time = crate::text::time_of_day(at, now)
                ),
                SMALL,
                p.muted,
            ));
        }
        false
    });
    if relink && let Some(url) = retry {
        let op = evolite_api::Spend::SetNameLink {
            label: label.clone(),
            url: Some(url),
        };
        asked.push(state.link_password.command(op, true));
    }
    if hide {
        asked.push(Command::HideNameRequest {
            identity: identity.clone(),
            label: label.clone(),
        });
        state.contest = None;
    } else if another {
        state.contest = None;
        state.adding_name = Some(NameForm::default());
    } else if close {
        state.contest = None;
    }
}

/// The state's chip: its icon, words, and whether it is drawn as verified.
fn state_chip(state: NameState) -> (&'static str, &'static str, bool) {
    match state {
        NameState::Voting { .. } => ("vote", tr!("Vote"), true),
        NameState::AwaitingResult { .. } => ("pending", tr!("Waiting for the result"), false),
        NameState::Won { .. } | NameState::Registered => ("confirmed", tr!("Won"), true),
        NameState::Lost { .. } => ("username", tr!("Went to another"), false),
        NameState::Locked { .. } => ("warning", tr!("Blocked"), false),
    }
}
