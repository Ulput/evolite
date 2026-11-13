//! Topping up credits: the dialog, what it says of a top-up under way, its errors.

use super::*;
use crate::components::{Part, dialog_with_footer};

/// "Top up the identity": DASH from Core into the identity's credits, what it costs and what
/// it gives. Closing the window does not stop a top-up that is running.
pub(super) fn top_up_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut IdentityState,
    wallet: &Wallet,
    id: &Identity,
    now: chrono::DateTime<chrono::Local>,
    asked: &mut Vec<Command>,
) {
    let Some(form) = &mut state.top_up else {
        return;
    };
    crate::format::fill_suggested(&mut form.amount, &mut form.suggested);
    let me = id
        .name()
        .map_or_else(|| "identity".to_string(), |n| format!("@{n}"));
    let fees = &wallet.fees;
    // Core as sure as it is (BUG-80): not known yet, no «More than there is on Core».
    let core_amount = CardAmount::core(&wallet.balances);
    let core = core_amount.known();
    // The asset lock is one Core transaction: its fee is about a typical payment's.
    let fee = fees.core_payment;
    let running = state.topping_up;
    let mut send = None;
    // What the middle reckoned of the amount, for the footer's button.
    let mut checked = (None, false, false);
    let close = dialog_with_footer(
        ui,
        a,
        "top-up",
        "top-up",
        tr!("Top up the identity"),
        464.0,
        |ui, a, part| {
            let p = a.p;
            if part == Part::Footer {
                // The footer (BUG-122): the password and the buttons, always in the window.
                let (amount, too_much, too_little) = checked;
                let ask = wallet.password_each_send && running.is_none();
                let entered = state.top_up_password.field(ui, &p, "top-up-password", ask);
                let mut close = false;
                crate::components::button_row_spend(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let label = match (running, amount) {
                        (Some(_), _) => tr!("Topping up…").to_string(),
                        (None, Some(d)) => format!(tr!("Top up by {}"), dash(d)),
                        (None, None) => tr!("Top up").to_string(),
                    };
                    let ready = running.is_none()
                        && amount.is_some()
                        && !too_much
                        && !too_little
                        && state.top_up_password.ready(ask);
                    if button_enabled(ui, a, &label, ButtonKind::Primary, None, ready).clicked()
                        || (entered && ready)
                    {
                        send = amount;
                    }
                });
                return close;
            }
            ui.add(
                crate::components::para(
                        &format!(
                            tr!("DASH from Core become credits of {me}: they pay for writes to Platform — names, contacts, profile.")
                        , me = me),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(20.0),
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                field_label(ui, &p, tr!("How much to turn into credits"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for duffs in [10_000_000, 50_000_000, 100_000_000] {
                        let label = crate::format::short_number(duffs);
                        let on = crate::format::parse_amount(&form.amount) == Some(duffs);
                        if pill(ui, &p, &label, on).clicked() {
                            form.amount = crate::format::input_text(duffs);
                        }
                    }
                });
                let field = amount_field(ui, &p, "top-up-amount", &mut form.amount, true);
                if field.all {
                    let all = core.unwrap_or(0) - fee - fees.top_up_on_top();
                    form.amount = crate::format::input_text(all.max(0));
                }
            });
            let amount = crate::format::parse_amount(&form.amount).filter(|d| *d > 0);
            let too_much = match (amount, core) {
                (Some(d), Some(core)) => {
                    crate::format::left_after(core, d + fees.top_up_on_top(), fee) < 0
                }
                _ => false,
            };
            if too_much {
                ui.label(txt(
                    tr!("More than there is on Core"),
                    12.0,
                    Weight::Regular,
                    p.danger,
                ));
            }
            // BUG-93 review must 1: Platform refuses a smaller lock and its DASH stay in it.
            let least = fees.top_up_amount_least();
            let too_little = amount.is_some_and(|d| d < least);
            if too_little {
                ui.label(txt(
                    &format!(
                        tr!("At least {} — Dash Platform takes no smaller asset lock"),
                        dash(least)
                    ),
                    12.0,
                    Weight::Regular,
                    p.danger,
                ));
            }
            let after = wallet.balances.credits.saturating_add(amount.unwrap_or(0));
            let value = mono(13.0, Weight::Medium);
            crate::components::kv_row_eye_in(
                ui,
                a,
                tr!("From the Core wallet"),
                &text::core_amount(&wallet.balances, core_amount, now),
                value.clone(),
                core_amount.color(&p),
            );
            kv_row(
                ui,
                &p,
                tr!("Core fee"),
                &format!("≈ {}", dash(fee)),
                value.clone(),
            );
            // BUG-93: the fee of Platform goes on top, the amount reaches the identity whole.
            kv_row(
                ui,
                &p,
                tr!("Platform fee"),
                &format!("≈ {}", dash(fees.top_up_on_top())),
                value.clone(),
            );
            if let Some(d) = amount {
                kv_row(
                    ui,
                    &p,
                    tr!("Total from Core"),
                    &format!("≈ {}", dash(d + fees.top_up_on_top() + fee)),
                    value.clone(),
                );
            }
            kv_row(
                ui,
                &p,
                tr!("On the identity after"),
                // Reckoned from the credits: «—» until they are checked in this run (BUG-116).
                &if wallet.balances.credits_checked {
                    format!(
                        tr!("{} · ≈ {} operations"),
                        crate::format::balance(after),
                        operations(after, fees)
                    )
                } else {
                    "—".to_string()
                },
                value,
            );
            notice(
                ui,
                a,
                "instantsend",
                p.accent,
                tr!(
                    "The asset lock is confirmed by InstantSend in seconds; if not, we wait for a ChainLock."
                ),
            );
            match (&running, &state.top_up_result) {
                (Some(_), _) => notice(
                    ui,
                    a,
                    "pending",
                    p.muted,
                    tr!(
                        "Topping up: locking DASH on Core, waiting for InstantSend, Platform adds the credits — usually under a minute. You can close the window."
                    ),
                ),
                (None, Some((duffs, result))) => top_up_answer(ui, a, *duffs, result),
                (None, None) => {}
            }
            checked = (amount, too_much, too_little);
            false
        },
    );
    if let Some(amount) = send {
        let ask = wallet.password_each_send;
        asked.push(
            state
                .top_up_password
                .command(Spend::TopUpIdentity { amount }, ask),
        );
        state.topping_up = Some(amount);
        state.top_up_result = None;
    } else if close {
        state.top_up = None;
        state.top_up_password.forget();
    }
}

/// Above the cards: a top-up still running with its window closed, its answer, and a top-up
/// cut short with "Continue the top-up".
pub(super) fn top_up_status(
    ui: &mut Ui,
    a: &mut Assets,
    wallet: &Wallet,
    state: &mut IdentityState,
    asked: &mut Vec<Command>,
) {
    let p = a.p;
    if state.top_up.is_none() {
        match (&state.topping_up, &state.top_up_result) {
            (Some(duffs), _) => notice(
                ui,
                a,
                "pending",
                p.muted,
                &format!(
                    tr!(
                        "Topping up the identity by {}: asset lock, InstantSend, Platform — usually under a minute."
                    ),
                    dash(*duffs)
                ),
            ),
            (None, Some((duffs, result))) => top_up_answer(ui, a, *duffs, result),
            (None, None) => {}
        }
    }
    let Some(unfinished) = wallet.unfinished_top_up else {
        return;
    };
    if state.topping_up.is_some() {
        return;
    }
    let (text, action) = unfinished_top_up_text(&unfinished, state.top_up_result.as_ref());
    notice(ui, a, "warning", p.accent, &text);
    // Continuing spends the lock already made: "password for every payment" asks too.
    let ask = wallet.password_each_send;
    let entered = state
        .resume_password
        .field(ui, &p, "resume-top-up-password", ask);
    let ready = state.resume_password.ready(ask);
    if button_enabled(ui, a, action, ButtonKind::Primary, None, ready).clicked()
        || (entered && ready)
    {
        asked.push(state.resume_password.command(Spend::ResumeTopUp, ask));
        state.topping_up = Some(unfinished.locked);
        state.top_up_result = None;
    }
}

/// The notice of a cut-short top-up and its button: one that can be continued, or one
/// Platform already took and whose ChainLock is awaited ("Check again"). When the last answer
/// already explains the wait, the notice does not repeat it.
pub(super) fn unfinished_top_up_text(
    unfinished: &evolite_api::UnfinishedTopUp,
    last: Option<&(evolite_api::Duffs, Result<(), CoreError>)>,
) -> (String, &'static str) {
    if unfinished.awaiting_chain_lock {
        let explained =
            matches!(last, Some((_, Err(CoreError::NotAvailable(why)))) if why != TOP_UP_SETTLED);
        let text = if explained {
            format!(
                tr!("{} of the top-up are awaiting the ChainLock of their block."),
                dash(unfinished.locked)
            )
        } else {
            format!(
                tr!(
                    "Platform has already taken the {} of the stopped top-up: the credits should be on the identity. evolite confirms it once their block gets a ChainLock, usually within a few minutes. You can top up again meanwhile."
                ),
                dash(unfinished.locked)
            )
        };
        (text, tr!("Check again"))
    } else {
        (
            format!(
                tr!(
                    "The top-up stopped: {} are already locked on Core. Continue — no new money is needed."
                ),
                dash(unfinished.locked)
            ),
            tr!("Continue the top-up"),
        )
    }
}

/// The answer to the last top-up: done, or why not with the network's words under "Details".
fn top_up_answer(
    ui: &mut Ui,
    a: &mut Assets,
    duffs: evolite_api::Duffs,
    result: &Result<(), CoreError>,
) {
    match (top_up_done_text(duffs, result), result) {
        (Some(text), _) => {
            let p = a.p;
            notice(ui, a, "confirmed", p.accent, &text);
        }
        (None, Err(e)) => crate::components::error_notice(
            ui,
            a,
            &top_up_error(e),
            crate::text::error_details(e).as_deref(),
        ),
        (None, Ok(())) => {}
    }
}

/// Why the top-up did not go through, in words.
pub(super) fn top_up_error(e: &CoreError) -> String {
    match e {
        // The core's reason is a catalog key: the catalog adds the language's own full stop.
        CoreError::NotAvailable(why) => format!("{}.", why.trim_end_matches('.')),
        CoreError::NotSynced => {
            tr!("Wait for the sync to finish: all the wallet's coins and the masternode lists are needed.").into()
        }
        CoreError::NotEnough => tr!("Not enough DASH on Core: the amount and the fee are needed.").into(),
        CoreError::BadAmount => tr!("The amount must be more than zero.").into(),
        other => format!(
            tr!("{} If the DASH are already locked, evolite shows “Continue the top-up”."),
            crate::text::failed(tr!("Not topped up"), other)
        ),
    }
}

/// [`top_up_blocked`] on its own facts. A top-up Platform already took (only its ChainLock is
/// awaited) does not hold a new one back.
pub(super) fn top_up_blocked_by(
    has_identity: bool,
    unfinished: Option<evolite_api::UnfinishedTopUp>,
    running: bool,
) -> Option<&'static str> {
    if !has_identity {
        Some(tr!("Make an identity first"))
    } else if running {
        Some(tr!("A top-up is already running"))
    } else if unfinished.is_some_and(|u| !u.awaiting_chain_lock) {
        Some(tr!("First finish the stopped top-up"))
    } else {
        None
    }
}

/// "Top up the identity" as typed, before it is sent.
pub struct TopUpForm {
    pub amount: String,
    /// 0.1 DASH in `amount` until the person changes it (`format::fill_suggested`).
    pub suggested: Option<i64>,
}

impl Default for TopUpForm {
    fn default() -> Self {
        Self {
            amount: crate::format::input_text(10_000_000),
            suggested: Some(10_000_000),
        }
    }
}

/// The core's answer to a top-up whose lock Platform took on an earlier attempt: the same
/// catalog key as `top_up_settled_text` in evolite-core `real/chain.rs`.
pub(super) const TOP_UP_SETTLED: &str = tr!(
    "Platform says it took these DASH on an earlier attempt, so the credits are already on the identity. No new money is needed"
);

/// The top-up is done: what to say. `None`: it did not go through ([`top_up_error`]). A lock
/// taken on an earlier attempt is done too, but no credits were added by this one (BUG-7 review).
pub(super) fn top_up_done_text(
    duffs: evolite_api::Duffs,
    result: &Result<(), CoreError>,
) -> Option<String> {
    match result {
        Ok(()) => Some(format!(tr!("Credits added: +{}."), dash(duffs))),
        Err(CoreError::NotAvailable(why)) if why == TOP_UP_SETTLED => Some(format!("{why}.")),
        Err(_) => None,
    }
}

/// Why "Top up" cannot start now, if it cannot.
pub(super) fn top_up_blocked(wallet: &Wallet, state: &IdentityState) -> Option<&'static str> {
    top_up_blocked_by(
        wallet.identity.is_some(),
        wallet.unfinished_top_up,
        state.topping_up.is_some(),
    )
}

pub(super) fn open_top_up(state: &mut IdentityState) {
    state.open_top_up();
}
