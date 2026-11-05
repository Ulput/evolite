//! 07 · Create an identity: a DPNS name (checked by the core), a top-up from Core into credits,
//! the costs, and what will happen step by step. The core does the asset lock and the rest.

use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, SpendPassword, StepState, amount_field, button, button_enabled,
    card, divider, field_label, inner, kv_row, notice, numbered_step_at, page_header,
    password_asked, text_field,
};
use crate::context::Ctx;
use crate::text::name_problem;
use crate::theme::{Weight, mono};
use egui::{Align, Layout, Margin, Ui};
use evolite_api::{Command, CoreError, NameCheck, Spend, UnfinishedIdentity};

pub struct CreateIdentityForm {
    pub name: String,
    pub amount: String,
    /// 0.3 DASH in `amount` until the person changes it (`format::fill_suggested`).
    pub suggested: Option<i64>,
    /// The label last sent to the core, and its answer.
    pub asked: String,
    pub check: Option<Result<NameCheck, CoreError>>,
    /// Sent to the core, no answer yet: the button stays off so money is locked only once.
    pub waiting: bool,
    pub result: Option<Result<(), CoreError>>,
    /// The step the core is on (1–4, "What happens"), from its progress.
    pub step: Option<usize>,
    /// "Password for every payment": creating and continuing lock Core DASH (BUG-42).
    pub password: SpendPassword,
}

impl CreateIdentityForm {
    pub fn on_done(&mut self, result: &Result<(), CoreError>) {
        self.waiting = false;
        self.step = None;
        self.password.on_answer(result);
        // A wrong password is said under its field: nothing was locked.
        if !self.password.is_wrong() {
            self.result = Some(result.clone());
        }
    }

    /// The core's progress: `fraction` is the step it is on out of four.
    pub fn on_progress(&mut self, fraction: f32) {
        self.step = Some(((fraction * 4.0).round() as usize).clamp(1, 4));
    }
}

/// Why the identity was not made, in words.
fn create_error(e: &CoreError) -> String {
    match e {
        CoreError::NotAvailable(why) => format!("{why}."),
        CoreError::NotSynced => {
            tr!("Wait for the sync to finish: all the wallet's coins and the masternode lists are needed.").into()
        }
        CoreError::NotEnough => tr!("Not enough for the top-up, the name and the fee.").into(),
        CoreError::BadAmount => tr!("The top-up amount must be more than zero.").into(),
        CoreError::BadName => tr!("This is not a Dash name: 3–63 letters, digits and hyphens.").into(),
        other => crate::text::failed(tr!("Not made"), other),
    }
}

impl Default for CreateIdentityForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            amount: crate::format::input_text(30_000_000),
            suggested: Some(30_000_000),
            asked: String::new(),
            check: None,
            waiting: false,
            result: None,
            step: None,
            password: SpendPassword::default(),
        }
    }
}

fn dash(d: i64) -> String {
    crate::format::amount(d)
}

pub fn create_identity(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut CreateIdentityForm) {
    let p = a.p;
    crate::format::fill_suggested(&mut form.amount, &mut form.suggested);
    let wallet = cx.wallet();
    let fees = &wallet.fees;
    ui.spacing_mut().item_spacing.y = 24.0;
    page_header(ui, a, tr!("Create an identity"), None, |_, _| {});

    // A run cut short is finished first: its money is already locked.
    if let Some(unfinished) = &wallet.unfinished_identity {
        resume_card(ui, a, cx, form, unfinished);
        return;
    }

    // The name goes to the core once per change; the answer comes back as an event.
    let label = form.name.trim().trim_start_matches('@').to_lowercase();
    if label.len() >= 3 && label != form.asked {
        cx.send(Command::CheckName {
            label: label.clone(),
        });
        form.asked = label.clone();
        form.check = None;
    }
    let check = if label.is_empty() || label != form.asked {
        None
    } else {
        form.check.clone()
    };
    // A contested name goes to a masternode vote, which the core cannot start: it is
    // explained at once and not offered, so the price shown is always a regular name's.
    let name_fee = fees.name_regular;
    let top_up = crate::format::parse_amount(&form.amount).filter(|d| *d > 0);
    let total = top_up.map(|t| {
        t.saturating_add(name_fee)
            .saturating_add(fees.identity_create)
    });
    let too_much = total.is_some_and(|t| t > wallet.balances.core);
    let name_ok = matches!(check, Some(Ok(NameCheck::Available)));

    let total_w = ui.available_width();
    let steps_w = 330.0;
    let side = total_w - steps_w - 24.0 >= 460.0;
    let form_w = if side {
        total_w - steps_w - 24.0
    } else {
        total_w
    };
    let shown_name = if label.is_empty() {
        tr!("name").to_string()
    } else {
        format!("@{label}")
    };
    // While the core works: the steps it has done, and the one it is on.
    let current = form.step.filter(|_| form.waiting);

    let mut draw_form = |ui: &mut Ui, a: &mut Assets| {
        card(&p, Margin::same(24)).show(ui, |ui| {
            ui.set_width(inner(form_w, 24));
            ui.spacing_mut().item_spacing.y = 16.0;
            ui.label(txt(tr!("1 · Name"), 15.0, Weight::SemiBold, p.text));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("Username"));
                if text_field(
                    ui,
                    &p,
                    "identity-name",
                    &mut form.name,
                    tr!("e.g. satoshi-2026"),
                    FieldKind::Text,
                    |_| {},
                )
                .changed()
                {
                    form.result = None;
                }
            });
            let (line, color) = match &check {
                None if label.len() >= 3 => (Some(tr!("Checking the name in DPNS…").to_string()), p.muted),
                // Too short to ask DPNS about: say so instead of staying silent.
                None if !label.is_empty() => (
                    Some(tr!("This is not a Dash name: 3–63 letters, digits and hyphens").to_string()),
                    p.danger,
                ),
                None => (None, p.muted),
                Some(Ok(NameCheck::Available)) => (
                    Some(format!(tr!("@{label} is free · a regular name, no vote"), label = label)),
                    p.text,
                ),
                Some(Ok(NameCheck::Contested)) => (
                    Some(format!(
                        tr!("@{label} is a contested name: masternodes award it by vote, which evolite cannot do yet. Add a digit from 2 to 9 or make the name longer than 19 characters.")
                    , label = label)),
                    p.danger,
                ),
                Some(Ok(NameCheck::Taken)) => (Some(format!(tr!("@{label} is already taken"), label = label)), p.danger),
                Some(Err(e)) => {
                    let (why, typo) = name_problem(e);
                    (Some(why), if typo { p.danger } else { p.muted })
                }
            };
            if let Some(line) = line {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if name_ok {
                        ui.add(a.icons.icon("check", p.text, &p, 16.0));
                    }
                    ui.add(egui::Label::new(txt(&line, 13.0, Weight::Regular, color)).wrap());
                });
            }
            notice(
                ui,
                a,
                "info",
                p.muted,
                tr!("Names shorter than 20 characters made only of letters, 0, 1 and hyphens are contested: masternodes award them by a vote of up to 14 days, and they cost more. A digit from 2 to 9 makes a name regular."),
            );
            divider(ui, &p);
            ui.label(txt(tr!("2 · Top-up from Core"), 15.0, Weight::SemiBold, p.text));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("How much to turn into credits"));
                let field = amount_field(ui, &p, "identity-amount", &mut form.amount, true);
                if field.all {
                    let all = (wallet.balances.core - name_fee - fees.identity_create).max(0);
                    form.amount = crate::format::input_text(all);
                }
                if too_much {
                    ui.label(txt(
                        tr!("More than there is on Core"),
                        12.0,
                        Weight::Regular,
                        p.danger,
                    ));
                }
            });
            let value = mono(13.0, Weight::Medium);
            kv_row(ui, &p, tr!("From the Core wallet"), &crate::format::balance(wallet.balances.core), value.clone());
            kv_row(ui, &p, tr!("Name registration"), &dash(name_fee), value.clone());
            kv_row(
                ui,
                &p,
                tr!("Making an identity"),
                &format!("≈ {}", dash(fees.identity_create)),
                value,
            );
            notice(
                ui,
                a,
                "instantsend",
                p.accent,
                tr!("The asset lock is confirmed by InstantSend in seconds; if not, we wait for a ChainLock."),
            );
            let ask = password_asked(cx.snap) && !form.waiting;
            let entered = form.password.field(ui, &p, "create-identity-password", ask);
            let ready = name_ok
                && total.is_some()
                && !too_much
                && !form.waiting
                && form.password.ready(ask);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let label = match total {
                    Some(t) => format!(tr!("Create for {}"), dash(t)),
                    None => tr!("Create").into(),
                };
                if (button_enabled(ui, a, &label, ButtonKind::Primary, None, ready).clicked()
                    || (entered && ready))
                    && let Some(top_up) = top_up
                {
                    let op = Spend::CreateIdentity {
                        name: form.asked.clone(),
                        top_up,
                    };
                    cx.send(form.password.command(op, ask));
                    form.result = None;
                    form.waiting = true;
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    cx.go(super::Screen::Identity);
                }
            });
            match &form.result {
                _ if form.waiting => notice(
                    ui,
                    a,
                    "pending",
                    p.muted,
                    tr!("Creating: asset lock, confirmation, identity and name — about a minute. Do not close evolite."),
                ),
                Some(Err(e @ (CoreError::NotAvailable(_) | CoreError::NotSynced))) => {
                    notice(ui, a, "info", p.muted, &create_error(e))
                }
                Some(Err(e)) => crate::components::error_notice(ui, a, &create_error(e), crate::text::error_details(e).as_deref()),
                Some(Ok(())) => notice(ui, a, "confirmed", p.accent, tr!("Identity made.")),
                None => {}
            }
        });
    };

    let locked = top_up.map_or(0, |t| t + name_fee);
    let draw_steps = |ui: &mut Ui, a: &mut Assets| {
        card(&p, Margin::same(24)).show(ui, |ui| {
            ui.set_width(inner(if side { steps_w } else { total_w }, 24));
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.label(txt(tr!("What happens"), 15.0, Weight::SemiBold, p.text));
            let state = |n| StepState::of(n, current);
            numbered_step_at(
                ui,
                a,
                1,
                tr!("Asset lock on Core"),
                &format!(tr!("{} are locked at the top-up address"), dash(locked)),
                state(1),
            );
            numbered_step_at(
                ui,
                a,
                2,
                tr!("Confirmation"),
                tr!("InstantLock or ChainLock, we check the BLS signature"),
                state(2),
            );
            numbered_step_at(
                ui,
                a,
                3,
                tr!("Identity on Platform"),
                tr!("DIP-13 keys from your phrase"),
                state(3),
            );
            numbered_step_at(
                ui,
                a,
                4,
                &format!(tr!("Name {shown_name}"), shown_name = shown_name),
                tr!("a DPNS claim, preorder + domain"),
                state(4),
            );
            divider(ui, &p);
            ui.add(
                egui::Label::new(
                    txt(
                        tr!("If the connection drops after step 1, the money is not lost: evolite shows “Continue” here and finishes from the same place."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(Some(18.0)),
                )
                .wrap(),
            );
        });
    };

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            if side {
                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 24.0;
                    ui.vertical(|ui| draw_form(ui, a));
                    ui.vertical(|ui| draw_steps(ui, a));
                });
            } else {
                ui.spacing_mut().item_spacing.y = 24.0;
                draw_form(ui, a);
                draw_steps(ui, a);
            }
        });
}

/// "Creation was interrupted": what is locked, the name it was for, and "Continue", which
/// finishes it from where it stopped without locking more.
fn resume_card(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut CreateIdentityForm,
    unfinished: &UnfinishedIdentity,
) {
    let p = a.p;
    let total_w = ui.available_width().min(640.0);
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(inner(total_w, 24));
        ui.spacing_mut().item_spacing.y = 14.0;
        ui.label(txt(tr!("Making the identity stopped"), 15.0, Weight::SemiBold, p.text));
        let name = unfinished
            .name
            .as_deref()
            .map_or_else(|| tr!("no name").to_string(), |n| format!("@{n}"));
        let what = match unfinished.locked {
            Some(locked) => format!(
                tr!("{} are locked on Core for identity {name}, but Platform has not made it yet. evolite continues from the same place: no new money is needed."),
                dash(locked)
            , name = name),
            None => format!(
                tr!("The identity is made, but the name {name} was not written. evolite will write it — the name's fee comes from the identity's credits.")
            , name = name),
        };
        ui.add(
            egui::Label::new(txt(&what, 13.0, Weight::Regular, p.muted).line_height(Some(20.0)))
                .wrap(),
        );
        if let Some(Err(e)) = &form.result {
            crate::components::error_notice(ui, a, &create_error(e), crate::text::error_details(e).as_deref());
        }
        if form.waiting {
            notice(
                ui,
                a,
                "pending",
                p.muted,
                tr!("Continuing: confirmation, identity and name — about a minute. Do not close evolite."),
            );
        }
        let ask = password_asked(cx.snap) && !form.waiting;
        let entered = form.password.field(ui, &p, "resume-identity-password", ask);
        let ready = !form.waiting && form.password.ready(ask);
        let label = if form.waiting { tr!("Continuing…") } else { tr!("Continue") };
        if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked()
            || (entered && ready)
        {
            cx.send(form.password.command(Spend::ResumeIdentity, ask));
            form.result = None;
            form.waiting = true;
        }
    });
}
