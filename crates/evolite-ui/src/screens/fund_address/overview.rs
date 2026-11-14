//! The funding on the Overview: the steps while it runs (frame 23), the stopped one with
//! «Continue the top-up» and its dialog (24, 24b), and what it credited (25).

use super::{FundAddressState, FundForm};
use crate::components::{
    Assets, ButtonKind, Part, StepState, button, button_enabled, dialog, dialog_with_footer,
    divider, kv_row, notice, notice_closable, numbered_step_at, numbered_step_then,
};
use crate::context::Ctx;
use crate::screens::{Screen, txt};
use crate::theme::{Weight, mono};
use egui::Ui;
use evolite_api::{Duffs, Spend, Wallet};

fn dash(d: Duffs) -> String {
    crate::format::amount(d)
}

impl FundAddressState {
    /// «Top up from Core ›» on the Platform address tile: opens the form.
    pub fn open(&mut self) {
        self.form = Some(FundForm::default());
        self.result = None;
    }

    /// Whether a new funding may be started: none running, none stopped (24: the tile's action
    /// is dim and inactive then).
    pub fn may_start(&self, wallet: &Wallet) -> bool {
        self.running.is_none() && wallet.unfinished_address_top_up.is_none()
    }
}

/// What the Overview shows of a funding, above «Total»: the stopped one with its button (24),
/// the answer (25, or why not), and the dialogs (22/22b, 23, 24b).
pub fn on_overview(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut FundAddressState) {
    let wallet = cx.wallet().clone();
    let p = a.p;
    let mut asked = Vec::new();
    match &state.result {
        Some((duffs, Ok(()))) => {
            let answer = notice_closable(
                ui,
                a,
                "confirmed",
                &format!(
                    tr!("Credited to the Platform address: {}"),
                    format!("+{}", dash(*duffs))
                ),
                tr!(
                    "In History — “Transfer from Core to the Platform address”. The amount on the address is visible to everyone."
                ),
                Some(("chevron-right", tr!("Open in History"))),
            );
            if answer.action {
                cx.go(Screen::Activity);
            }
            if answer.closed || answer.action {
                state.result = None;
            }
        }
        Some((_, Err(e))) => {
            let text = crate::text::failed(tr!("The Platform address was not topped up"), e);
            let answer = notice_closable(ui, a, "warning", &text, "", None);
            if answer.closed {
                state.result = None;
            }
        }
        None => {}
    }
    if let Some(unfinished) = &wallet.unfinished_address_top_up
        && state.running.is_none()
    {
        notice(
            ui,
            a,
            "warning",
            p.accent,
            &format!(
                tr!(
                    "The top-up of the Platform address stopped: {} are already locked in Core. Continue — no new money is needed."
                ),
                dash(unfinished.locked)
            ),
        );
        if button(ui, a, tr!("Continue the top-up"), ButtonKind::Primary, None).clicked() {
            state.resume_open = true;
        }
    }
    super::fund_dialog(ui, a, state, &wallet, cx.now, &mut asked);
    if state.running.is_some() && state.steps_open {
        steps_dialog(ui, a, state, &wallet);
    }
    if state.resume_open {
        resume_dialog(ui, a, state, &wallet, &mut asked);
    }
    for command in asked {
        cx.send(command);
    }
}

/// «Topping up the Platform address» (23): the three steps, the one under way, and «Hide».
fn steps_dialog(ui: &mut Ui, a: &mut Assets, state: &mut FundAddressState, wallet: &Wallet) {
    let Some(amount) = state.running else {
        return;
    };
    // Step 1 is done once the lock is made: the snapshot holds it, not taken yet.
    let locked = wallet.unfinished_address_top_up.as_ref().map(|u| u.locked);
    let current = Some(if locked.is_some() { 2 } else { 1 });
    let mut hide = false;
    let close = dialog(
        ui,
        a,
        "fund-address-steps",
        "top-up",
        tr!("Topping up the Platform address"),
        464.0,
        |ui, a| {
            let p = a.p;
            numbered_step_at(
                ui,
                a,
                1,
                tr!("Asset lock in Core"),
                &format!(
                    tr!("{} locked for Platform"),
                    // The lock itself, not with the Core fee (the designer, 2026-11-14).
                    dash(locked.unwrap_or(amount + wallet.fees.address_fund_on_top()))
                ),
                StepState::of(1, current),
            );
            numbered_step_at(
                ui,
                a,
                2,
                tr!("Confirmation"),
                tr!(
                    "InstantLock or ChainLock: waiting for InstantSend, if it does not come, ChainLock up to ~2.5 min"
                ),
                StepState::of(2, current),
            );
            // The address under the words, in the monospace font: Inter draws «x» between digits
            // as «×» (the designer, FEAT-0015-stage2-design).
            numbered_step_then(
                ui,
                a,
                3,
                tr!("Credit to the address"),
                &format!(
                    tr!("Platform checks the lock and credits at least {} to the address"),
                    dash(amount)
                ),
                StepState::of(3, current),
                |ui| {
                    ui.label(
                        egui::RichText::new(&wallet.platform_address)
                            .font(mono(12.0, Weight::Regular))
                            .color(p.muted),
                    );
                },
            );
            divider(ui, &p);
            ui.label(txt(
                tr!("You can hide the window: the top-up goes on. If the connection drops after step 1, the money is not lost: evolite shows “Continue” on the Overview and finishes from the same place."),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            // At the right, as the dialogs' buttons (frame 23).
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if button(ui, a, tr!("Hide"), ButtonKind::Secondary, None).clicked() {
                    hide = true;
                }
            });
            false
        },
    );
    if close || hide {
        state.steps_open = false;
    }
}

/// «Continue the top-up» (24b): what is already locked, what will arrive, the password and
/// «Sign and continue».
fn resume_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut FundAddressState,
    wallet: &Wallet,
    asked: &mut Vec<evolite_api::Command>,
) {
    let Some(unfinished) = wallet.unfinished_address_top_up.clone() else {
        state.resume_open = false;
        return;
    };
    let ask = wallet.password_each_send;
    let mut go = false;
    let close = dialog_with_footer(
        ui,
        a,
        "fund-address-resume",
        "top-up",
        tr!("Continue the top-up"),
        464.0,
        |ui, a, part| {
            let p = a.p;
            if part == Part::Footer {
                let entered =
                    state
                        .resume_password
                        .field(ui, &p, "fund-address-resume-password", ask);
                let ready = state.resume_password.ready(ask);
                let mut close = false;
                crate::components::button_row_spend(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    if button_enabled(
                        ui,
                        a,
                        tr!("Sign and continue"),
                        ButtonKind::Primary,
                        None,
                        ready,
                    )
                    .clicked()
                        || (entered && ready)
                    {
                        go = true;
                    }
                });
                return close;
            }
            ui.add(
                crate::components::para(
                    &format!(
                        tr!("{} are already locked in Core for this wallet's Platform address. evolite sends this lock to Platform — no new money is needed."),
                        dash(unfinished.locked)
                    ),
                    13.0,
                    Weight::Regular,
                    p.muted,
                )
                .line_height(20.0),
            );
            // What the lock brings, as the review says it (the designer): the lock less what is
            // on top.
            let on_top = wallet.fees.address_fund_on_top();
            let value = mono(13.0, Weight::Medium);
            kv_row(
                ui,
                &p,
                tr!("Arrives at the address, at least"),
                &dash((unfinished.locked - on_top).max(0)),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Processing on Platform"),
                &format!("≈ {}", dash(on_top)),
                value,
            );
            ui.label(txt(
                tr!("Whatever is not needed stays on the address."),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            // As far from the password as 22b's groups are (the designer).
            ui.add_space(8.0);
            false
        },
    );
    if go {
        asked.push(state.resume_password.command(Spend::ResumeFundAddress, ask));
        state.running = Some(
            unfinished
                .locked
                .saturating_sub(wallet.fees.address_fund_on_top()),
        );
        state.result = None;
        state.resume_open = false;
        state.steps_open = true;
    } else if close {
        state.resume_open = false;
        state.resume_password.forget();
    }
}
