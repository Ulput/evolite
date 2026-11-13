//! 07 · Create an identity: a DPNS name (checked by the core; optional for one more identity,
//! FEAT-0014), the amount put into credits from Core — it reaches the identity whole, every fee
//! goes on top of it with a margin (BUG-93) — the costs, and what will happen step by step;
//! «Check the new identity» with the password before anything is locked. The core does the
//! asset lock and the rest.

use super::txt;
use crate::components::{
    Assets, ButtonKind, CardAmount, CardColumn, FieldKind, Part, SpendPassword, StepState,
    amount_field, button, button_enabled, card, dialog_with_footer, divider, field_label, inner,
    kv_row, notice, numbered_step_at, option_card_in, option_card_width, page_header,
    password_asked, text_field,
};
use crate::context::Ctx;
use crate::text::name_problem;
use crate::theme::{Weight, mono, sans};
use egui::{Align, Layout, Margin, Ui, vec2};
use evolite_api::{Command, CoreError, Duffs, NameCheck, Spend, UnfinishedIdentity};

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
    /// «Check the new identity» is open over the form (FEAT-0014, frame 7); the password, when
    /// asked, is typed there.
    pub confirming: bool,
    /// The identity the wallet acted as when the creation was signed (`Some(None)`: none):
    /// once the new one is made, Identity tells it by that (analyst-44, the banner).
    pub before: Option<Option<String>>,
    /// «Hide» was pressed in the steps over the form (frame 8): until the next creation.
    pub steps_hidden: bool,
}

impl CreateIdentityForm {
    /// The core's answer to `CreateIdentity` or `ResumeIdentity`: true when the identity is made
    /// (the app then shows it on Identity with its banner).
    pub fn on_done(&mut self, result: &Result<(), CoreError>) -> bool {
        self.waiting = false;
        self.step = None;
        self.password.on_answer(result);
        // A wrong password is said under its field, in the review it was typed in: nothing
        // was locked.
        if self.password.is_wrong() {
            self.confirming = true;
            return false;
        }
        self.result = Some(result.clone());
        result.is_ok()
    }

    /// The core's progress: `fraction` is the step it is on out of four.
    pub fn on_progress(&mut self, fraction: f32) {
        self.step = Some(((fraction * 4.0).round() as usize).clamp(1, 4));
    }

    /// The creation (or its continuation) goes to the core from here: `chosen` is the identity
    /// the wallet acts as now.
    fn sent(&mut self, command: Command, chosen: Option<String>, cx: &mut Ctx) {
        cx.send(command);
        self.result = None;
        self.waiting = true;
        self.before = Some(chosen);
        self.steps_hidden = false;
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
            confirming: false,
            before: None,
            steps_hidden: false,
        }
    }
}

fn dash(d: i64) -> String {
    crate::format::amount(d)
}

/// What «Create» would lock, as the review lists it (FEAT-0014, frame 7).
#[derive(Clone, Debug, PartialEq)]
struct Plan {
    /// The DPNS label; `None`: an identity without a name (R-7).
    label: Option<String>,
    /// The amount asked for: what ends up as credits, whole (BUG-93).
    top_up: Duffs,
    name_fee: Duffs,
    create_fee: Duffs,
    /// The asset lock: the amount and Platform's fees on top (BUG-93), as the core locks it
    /// (`Fees::identity_lock_for`).
    lock: Duffs,
    /// The Core fee of the lock's transaction.
    core_fee: Duffs,
}

impl Plan {
    /// What leaves Core: the lock and its transaction's fee (BUG-93).
    fn total(&self) -> Duffs {
        self.lock.saturating_add(self.core_fee)
    }
}

/// What the amount means (BUG-93), under it on the form and in the review: it becomes credits
/// whole, the fees are on top with a margin, and what the margin leaves stays as credits.
fn fees_note() -> &'static str {
    tr!(
        "This amount becomes credits in full. Platform's fees are added on top with a margin; what the margin leaves stays as credits too."
    )
}

/// Whether Core can pay `total` and the lock's own fee: `None` while Core's balance is not
/// checked now (BUG-77: then nothing is spent, frame 10), else the least it takes when Core
/// has less (frame 9).
fn short_of(core: CardAmount, total: Duffs, core_fee: Duffs) -> Option<Option<Duffs>> {
    match core {
        CardAmount::Now(core) => Some(
            (crate::format::left_after(core, total, core_fee) < 0)
                .then(|| total.saturating_add(core_fee)),
        ),
        CardAmount::Earlier(_) | CardAmount::Unknown => None,
    }
}

/// The example under an empty name: the first identity's name with «-apps» for one more
/// (frame 4), a made-up one for the first.
fn example(wallet: &evolite_api::Wallet) -> String {
    match wallet.identity.as_ref().and_then(|i| i.name()) {
        Some(name) => format!(tr!("e.g. {}"), crate::i18n::own(&format!("{name}-apps"))),
        None => tr!("e.g. satoshi-2026").into(),
    }
}

pub fn create_identity(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut CreateIdentityForm) {
    let p = a.p;
    crate::format::fill_suggested(&mut form.amount, &mut form.suggested);
    let wallet = cx.wallet();
    let fees = &wallet.fees;
    // One more identity (FEAT-0014): «New identity», and the name may stay empty (R-7).
    let more = wallet.identity.is_some();
    let chosen = wallet.identity.as_ref().map(|i| i.id.clone());
    ui.spacing_mut().item_spacing.y = 24.0;
    let title = if more {
        tr!("New identity")
    } else {
        tr!("Create an identity")
    };
    page_header(ui, a, title, None, |_, _| {});

    // A run cut short is finished first: its money is already locked.
    if let Some(unfinished) = &wallet.unfinished_identity {
        resume_card(ui, a, cx, form, unfinished, chosen);
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
    // No name at all: allowed for one more identity, and then nothing is paid for one.
    let nameless = more && label.is_empty();
    // A contested name goes to a masternode vote, which the core cannot start: it is
    // explained at once and not offered, so the price shown is always a regular name's.
    let name_fee = if nameless { 0 } else { fees.name_on_top() };
    let top_up = crate::format::parse_amount(&form.amount).filter(|d| *d > 0);
    // What the asset lock takes — the amount asked for, whole, and on top of it the fees for
    // the name and the creation, as the core locks them (BUG-93) — and the Core fee of the
    // lock's transaction on top of that, as for a top-up.
    let core_fee = top_up.map_or(fees.core_payment, |t| {
        fees.core_payment_for(fees.identity_lock_for(t, !nameless))
    });
    let plan = top_up.map(|top_up| Plan {
        label: (!nameless).then(|| label.clone()),
        top_up,
        name_fee,
        create_fee: fees.identity_create,
        lock: fees.identity_lock_for(top_up, !nameless),
        core_fee,
    });
    let lock = plan.as_ref().map(|plan| plan.lock);
    let total = plan.as_ref().map(Plan::total);
    // Core as sure as it is (BUG-80): not known yet, it is not «less than the amount» either;
    // not checked now, nothing is locked from it (BUG-77, frame 10).
    let core = CardAmount::core(&wallet.balances);
    let core_text = crate::text::core_amount(&wallet.balances, core, cx.now);
    let short = lock.map(|l| short_of(core, l, core_fee));
    let checked = matches!(core, CardAmount::Now(_));
    let too_much = matches!(short, Some(Some(Some(_))));
    // Platform takes no asset lock below its minimum, and the DASH in one it refuses stay
    // locked (BUG-54 review r1 must); with a name, the credits after the creation also have
    // to pay for it, or the DASH are locked and the name is refused (BUG-93).
    let least = fees.identity_amount_least(!nameless);
    let too_little = top_up.is_some_and(|t| t < least);
    let name_ok = nameless || matches!(check, Some(Ok(NameCheck::Available)));

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
    let ask = password_asked(cx.snap) && !form.waiting;
    let ready = name_ok && plan.is_some() && checked && !too_much && !too_little && !form.waiting;

    let facts = FormFacts {
        wallet,
        more,
        label: &label,
        check: &check,
        nameless,
        name_ok,
        name_fee,
        core_fee,
        least,
        too_little,
        core,
        core_text: &core_text,
        short,
        checked,
        total,
        ready,
    };
    let mut draw_form = |ui: &mut Ui, a: &mut Assets| {
        new_identity_card(ui, a, cx, form, &facts, form_w);
    };

    let steps = Steps {
        locked: lock.unwrap_or(0),
        current,
        // The name's step, when there may be none (frame 4): said as optional.
        name: if nameless {
            None
        } else {
            Some(shown_name.clone())
        },
    };
    let draw_steps = |ui: &mut Ui, a: &mut Assets| {
        card(&p, Margin::same(24)).show(ui, |ui| {
            ui.set_width(inner(if side { steps_w } else { total_w }, 24));
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.label(txt(tr!("What happens"), 15.0, Weight::SemiBold, p.text));
            steps.draw(ui, a);
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

    // Frame 8 (AC-D9): while the core works, its steps over the form until «Hide»; they go on
    // in «What happens» beside it.
    if form.waiting && !form.steps_hidden {
        let title = if label.is_empty() {
            tr!("Identity creation").to_string()
        } else {
            format!(
                tr!("Creation of the identity {}"),
                crate::i18n::own(&shown_name)
            )
        };
        if creating_steps(ui, a, &title, &steps) {
            form.steps_hidden = true;
        }
    }

    if form.confirming {
        match plan {
            // The form changed under the review (a snapshot, an answer): «Sign and create»
            // waits for it to be right again.
            Some(plan) => {
                let can = Review { ask, ready, chosen };
                check_new_identity(ui, a, cx, form, &plan, &core_text, can)
            }
            // No amount any more: nothing to review.
            None => {
                form.confirming = false;
                form.password.forget();
            }
        }
    }
}

/// What one frame of the form knows, worked out before anything is drawn.
struct FormFacts<'a> {
    wallet: &'a evolite_api::Wallet,
    /// One more identity (FEAT-0014): the name may stay empty.
    more: bool,
    label: &'a str,
    check: &'a Option<Result<NameCheck, CoreError>>,
    nameless: bool,
    name_ok: bool,
    name_fee: Duffs,
    core_fee: Duffs,
    least: Duffs,
    too_little: bool,
    core: CardAmount,
    core_text: &'a str,
    short: Option<Option<Option<Duffs>>>,
    checked: bool,
    total: Option<Duffs>,
    ready: bool,
}

/// The form's card: the name, the top-up, what it costs and the core's answer.
fn new_identity_card(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut CreateIdentityForm,
    v: &FormFacts,
    form_w: f32,
) {
    let p = a.p;
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(inner(form_w, 24));
        ui.spacing_mut().item_spacing.y = 16.0;
        name_section(ui, a, form, v);
        divider(ui, &p);
        ui.label(txt(tr!("2 · Top-up"), 15.0, Weight::SemiBold, p.text));
        pay_from(ui, a, v.core_text);
        top_up_field(ui, form, v, &p);
        cost_rows(ui, a, v);
        balance_notice(ui, a, v);
        form_buttons(ui, a, cx, form, v);
        form_answer(ui, a, form);
    });
}

/// «1 · Name»: the field, what DPNS says of the name, and the note on contested names.
fn name_section(ui: &mut Ui, a: &mut Assets, form: &mut CreateIdentityForm, v: &FormFacts) {
    let p = a.p;
    let wallet = v.wallet;
    let (more, label, check) = (v.more, v.label, v.check);
    let (nameless, name_ok) = (v.nameless, v.name_ok);
    let heading = if more {
        tr!("1 · Name (optional)")
    } else {
        tr!("1 · Name")
    };
    ui.label(txt(heading, 15.0, Weight::SemiBold, p.text));
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, &p, tr!("Username"));
        if text_field(
            ui,
            &p,
            "identity-name",
            &mut form.name,
            &example(wallet),
            FieldKind::Text,
            |_| {},
        )
        .changed()
        {
            form.result = None;
        }
    });
    let (line, color) = match &check {
                // An empty name is no mistake for one more identity: what it means (frame 4).
                None if nameless => (
                    Some(tr!("Without a name nobody finds it by @name or adds it to DashPay contacts. You can add a name later on the Identity screen.").to_string()),
                    p.muted,
                ),
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
                ui.add(
                    a.icons
                        .icon("check", if nameless { p.muted } else { p.text }, &p, 16.0),
                );
            }
            ui.add(crate::components::para(&line, 13.0, Weight::Regular, color));
        });
    }
    // About short contested names: hidden while there is no name to be one (frame 4).
    if !nameless {
        notice(
            ui,
            a,
            "info",
            p.muted,
            tr!(
                "Names shorter than 20 characters made only of letters, 0, 1 and hyphens are contested: masternodes award them by a vote of up to 14 days, and they cost more. A digit from 2 to 9 makes a name regular."
            ),
        );
    }
}

/// How much to turn into credits, and what is wrong with that amount.
fn top_up_field(
    ui: &mut Ui,
    form: &mut CreateIdentityForm,
    v: &FormFacts,
    p: &crate::theme::Palette,
) {
    let p = *p;
    let fees = &v.wallet.fees;
    let (core, name_fee, too_little, nameless, least) =
        (v.core, v.name_fee, v.too_little, v.nameless, v.least);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, &p, tr!("How much to turn into credits"));
        let field = amount_field(ui, &p, "identity-amount", &mut form.amount, true);
        if field.all {
            let all =
                (core.known().unwrap_or(0) - name_fee - fees.identity_create - fees.core_payment)
                    .max(0);
            form.amount = crate::format::input_text(all);
        }
        // Too little for Core is said in place of the InstantSend hint below (the
        // designer, frame 9): the amount itself may be right.
        if too_little {
            let hint = if nameless {
                format!(
                    tr!("At least {} — Dash Platform takes no smaller asset lock"),
                    dash(least)
                )
            } else {
                format!(
                    tr!("At least {} — less may not cover the name's registration"),
                    dash(least)
                )
            };
            ui.label(txt(&hint, 12.0, Weight::Regular, p.danger));
        } else {
            ui.label(txt(fees_note(), 12.0, Weight::Regular, p.muted));
        }
    });
}

/// What the identity will cost, line by line.
fn cost_rows(ui: &mut Ui, a: &mut Assets, v: &FormFacts) {
    let p = a.p;
    let fees = &v.wallet.fees;
    let (core, core_text) = (v.core, v.core_text);
    let (nameless, name_fee, core_fee) = (v.nameless, v.name_fee, v.core_fee);
    let value = mono(13.0, Weight::Medium);
    crate::components::kv_row_eye_in(
        ui,
        a,
        tr!("From the Core wallet"),
        core_text,
        value.clone(),
        core.color(&p),
    );
    let name_cost = if nameless {
        tr!("no name · 0").to_string()
    } else {
        format!("≈ {}", dash(name_fee))
    };
    kv_row(ui, &p, tr!("Name registration"), &name_cost, value.clone());
    kv_row(
        ui,
        &p,
        tr!("Making an identity"),
        &format!("≈ {}", dash(fees.identity_create)),
        value.clone(),
    );
    kv_row(
        ui,
        &p,
        tr!("Core fee"),
        &format!("≈ {}", dash(core_fee)),
        value,
    );
}

/// Whether Core can pay for it, and how the asset lock will be confirmed.
fn balance_notice(ui: &mut Ui, a: &mut Assets, v: &FormFacts) {
    let p = a.p;
    let (short, core, checked) = (v.short, v.core, v.checked);
    match (short, core) {
        (Some(Some(Some(needed))), CardAmount::Now(has)) => {
            crate::components::error_notice(
                ui,
                a,
                &format!(
                    tr!("At least ≈ {} is needed, Core has {}. Lower the amount or top up Core."),
                    dash(needed),
                    crate::format::balance(has)
                ),
                None,
            );
        }
        _ if !checked => notice(
            ui,
            a,
            "instantsend",
            p.accent,
            tr!(
                "The Core balance is still being checked: you can create an identity once the sync is done."
            ),
        ),
        _ => notice(
            ui,
            a,
            "instantsend",
            p.accent,
            tr!(
                "The asset lock is confirmed by InstantSend in seconds; if not, we wait for a ChainLock."
            ),
        ),
    }
}

/// «Create» and «Cancel».
fn form_buttons(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut CreateIdentityForm,
    v: &FormFacts,
) {
    let (total, ready) = (v.total, v.ready);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let label = match total {
            Some(t) => format!(tr!("Create for {}"), format!("≈ {}", dash(t))),
            None => tr!("Create").into(),
        };
        // The password and the last look come in «Check the new identity» (frame 7).
        if button_enabled(ui, a, &label, ButtonKind::Primary, None, ready).clicked() {
            form.confirming = true;
            form.result = None;
        }
        if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
            cx.go(super::Screen::Identity);
        }
    });
}

/// While the core works, and its answer.
fn form_answer(ui: &mut Ui, a: &mut Assets, form: &CreateIdentityForm) {
    let p = a.p;
    match &form.result {
        _ if form.waiting => notice(
            ui,
            a,
            "pending",
            p.muted,
            tr!(
                "Creating: asset lock, confirmation, identity and name — about a minute. Do not close evolite."
            ),
        ),
        Some(Err(e @ (CoreError::NotAvailable(_) | CoreError::NotSynced))) => {
            notice(ui, a, "info", p.muted, &create_error(e))
        }
        Some(Err(e)) => crate::components::error_notice(
            ui,
            a,
            &create_error(e),
            crate::text::error_details(e).as_deref(),
        ),
        Some(Ok(())) => notice(ui, a, "confirmed", p.accent, tr!("Identity made.")),
        None => {}
    }
}

/// The four steps of «What happens» (frame 4) and of the steps over the form (frame 8): what
/// is locked, the step the core is on, and the name's step (`None`: no name, so optional).
struct Steps {
    locked: Duffs,
    current: Option<usize>,
    name: Option<String>,
}

impl Steps {
    fn draw(&self, ui: &mut Ui, a: &mut Assets) {
        let p = a.p;
        let state = |n| StepState::of(n, self.current);
        numbered_step_at(
            ui,
            a,
            1,
            tr!("Asset lock on Core"),
            &format!(
                tr!("{} are locked at the top-up address"),
                dash(self.locked)
            ),
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
        let (name_step, name_detail) = match &self.name {
            None => (
                tr!("Name, if given").to_string(),
                tr!("without a name this step is skipped"),
            ),
            Some(shown_name) => (
                format!(tr!("Name {shown_name}"), shown_name = shown_name),
                tr!("a DPNS claim, preorder + domain"),
            ),
        };
        numbered_step_at(ui, a, 4, &name_step, name_detail, state(4));
        divider(ui, &p);
        ui.add(
            crate::components::para(
                    tr!("If the connection drops after step 1, the money is not lost: evolite shows “Continue” here and finishes from the same place."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                )
                .line_height(18.0),
        );
    }
}

/// «Creating identity …» (frame 8, AC-D9): the core's steps over the form while it works.
/// «Hide» or «×» closes the window only — the creation goes on, and its steps stay in «What
/// happens». True when closed.
fn creating_steps(ui: &mut Ui, a: &mut Assets, title: &str, steps: &Steps) -> bool {
    dialog_with_footer(
        ui,
        a,
        "create-identity-steps",
        "identity",
        title,
        440.0,
        |ui, a, part| {
            if part == Part::Footer {
                // The footer (BUG-122): always in the window.
                let mut hide = false;
                crate::components::button_row(ui, |ui| {
                    hide = button(ui, a, tr!("Hide"), ButtonKind::Secondary, None).clicked();
                });
                return hide;
            }
            ui.spacing_mut().item_spacing.y = 14.0;
            steps.draw(ui, a);
            false
        },
    )
}

/// «Pay from» (the customer, 2026-11-07): the sources of the money in equal columns as Send's
/// «From» (BUG-67, BUG-81), with only «From Core» in FEAT-0014. The grid is the one of two
/// columns, so the next source stands beside it without the form changing.
fn pay_from(ui: &mut Ui, a: &mut Assets, core_text: &str) {
    let p = a.p;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, &p, tr!("Pay from"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            let title = tr!("From Core");
            let words = option_card_width(ui, title, core_text, false);
            let with_icon = option_card_width(ui, title, core_text, true);
            let width = crate::components::from_columns(ui.available_width(), 2, words);
            let column = CardColumn {
                width,
                icon: width >= with_icon,
            };
            // The only source is always the chosen one.
            option_card_in(ui, a, "core", title, core_text, true, column);
        });
    });
}

/// What «Check the new identity» may do: whether the password is asked, and whether the form
/// is still right to create (the name checked, Core checked and enough, not sent yet), and the
/// identity the wallet acts as now.
struct Review {
    ask: bool,
    ready: bool,
    chosen: Option<String>,
}

/// «Check the new identity» (FEAT-0014, frame 7; R-6): what is locked and for what, that it
/// cannot be undone, who will see the name, and the password when asked. «Sign and create»
/// sends the creation; the review closes, and opens again on a wrong password.
fn check_new_identity(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut CreateIdentityForm,
    plan: &Plan,
    core_text: &str,
    can: Review,
) {
    let p = a.p;
    let Review {
        ask,
        ready: form_ready,
        chosen,
    } = can;
    let mut create = false;
    let closed = dialog_with_footer(
        ui,
        a,
        "create-identity-confirm",
        "identity",
        tr!("Check the new identity"),
        440.0,
        |ui, a, part| {
            if part == Part::Footer {
                // The footer (BUG-122): always in the window.
                // In a dialog Enter in the field presses the row's primary (`button_row_spend`).
                form.password.field(ui, &p, "create-identity-password", ask);
                let mut close = false;
                crate::components::button_row_spend(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let ready = form_ready && form.password.ready(ask);
                    if button_enabled(
                        ui,
                        a,
                        tr!("Sign and create"),
                        ButtonKind::Primary,
                        None,
                        ready,
                    )
                    .clicked()
                        && ready
                    {
                        create = true;
                        close = true;
                    }
                });
                return close;
            }
            ui.spacing_mut().item_spacing.y = 14.0;
            let value = mono(13.0, Weight::Medium);
            kv_row(
                ui,
                &p,
                tr!("From"),
                &format!("Core · {core_text}"),
                sans(13.0, Weight::Medium),
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("New identity · name"));
                match &plan.label {
                    Some(label) => {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(crate::i18n::t(&format!("@{label}")))
                                    .font(mono(14.0, Weight::Medium))
                                    .color(p.text),
                            )
                            .wrap(),
                        );
                    }
                    None => {
                        ui.label(txt(tr!("no name"), 14.0, Weight::Medium, p.muted));
                    }
                }
            });
            kv_row(
                ui,
                &p,
                tr!("Into credits"),
                &dash(plan.top_up),
                value.clone(),
            );
            if let Some(label) = &plan.label {
                kv_row(
                    ui,
                    &p,
                    &format!(tr!("Name {}"), crate::i18n::own(&format!("@{label}"))),
                    &format!("≈ {}", dash(plan.name_fee)),
                    value.clone(),
                );
            }
            kv_row(
                ui,
                &p,
                tr!("Making an identity"),
                &format!("≈ {}", dash(plan.create_fee)),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Core fee"),
                &format!("≈ {}", dash(plan.core_fee)),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Total from Core"),
                &format!("≈ {}", dash(plan.total())),
                value,
            );
            ui.add(crate::components::para(
                fees_note(),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            ui.add(
                crate::components::para(
                        tr!("The DASH are locked in Core for Platform. Once signed, this cannot be undone; if the connection drops, evolite continues from the same place."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(18.0),
            );
            let public = match &plan.label {
                Some(label) => format!(
                    tr!("The name {} will be visible to everyone: people find you by it and can add you to contacts."),
                    crate::i18n::own(&format!("@{label}"))
                ),
                None => tr!("Without a name nobody finds it by @name or adds it to DashPay contacts. You can add a name later on the Identity screen.").into(),
            };
            notice(ui, a, "eye", p.muted, &public);
            false
        },
    );
    if create {
        let op = Spend::CreateIdentity {
            name: plan.label.clone(),
            top_up: plan.top_up,
        };
        let command = form.password.command(op, ask);
        form.sent(command, chosen, cx);
        form.confirming = false;
    } else if closed {
        form.confirming = false;
        form.password.forget();
    }
}

/// "Creation was interrupted": what is locked, the name it was for, and "Continue", which
/// finishes it from where it stopped without locking more.
fn resume_card(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut CreateIdentityForm,
    unfinished: &UnfinishedIdentity,
    chosen: Option<String>,
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
            crate::components::para(&what, 13.0, Weight::Regular, p.muted).line_height(20.0),
        );
        if let Some(Err(e)) = &form.result {
            let text = match (e, unfinished.locked) {
                // BUG-93: the identity exists, it is its credits that ran out.
                (CoreError::NotEnough, None) => tr!("Not enough credits for the name. Top up this identity on the Identity screen, then press Continue.").to_string(),
                _ => create_error(e),
            };
            crate::components::error_notice(ui, a, &text, crate::text::error_details(e).as_deref());
            // BUG-93 review should 2: the top-up goes to the chosen identity, and after a failed
            // creation that is still the old one: choosing this one first.
            if let (CoreError::NotEnough, Some(id)) = (e, unfinished.identity.as_ref())
                && button(ui, a, tr!("Top up"), ButtonKind::Secondary, None).clicked()
            {
                cx.send(Command::ChooseIdentity { id: id.clone() });
                cx.top_up();
            }
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
        // BUG-97: the made identity's credits have to carry the name again; only known for
        // the identity shown, the core refuses the others itself, before anything is sent.
        let wallet = cx.wallet();
        let needed = wallet.fees.name_credits_needed();
        let shown_short = unfinished.locked.is_none()
            && unfinished.identity.is_some()
            && wallet.identity.as_ref().map(|i| &i.id) == unfinished.identity.as_ref()
            && wallet.balances.credits_read
            && wallet.balances.credits < needed;
        if shown_short {
            let has = crate::format::balance(wallet.balances.credits);
            let text = format!(
                tr!("Not enough credits for the name: ≈ {} needed, the identity has {}."),
                dash(needed),
                has
            );
            ui.add(crate::components::para(&text, 13.0, Weight::Regular, p.muted));
            if let Some(id) = unfinished.identity.as_ref()
                && button(ui, a, tr!("Top up"), ButtonKind::Secondary, None).clicked()
            {
                cx.send(Command::ChooseIdentity { id: id.clone() });
                cx.top_up();
            }
        }
        let entered = form.password.field(ui, &p, "resume-identity-password", ask);
        let ready = !form.waiting && form.password.ready(ask) && !shown_short;
        let label = if form.waiting { tr!("Continuing…") } else { tr!("Continue") };
        if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked()
            || (entered && ready)
        {
            let command = form.password.command(Spend::ResumeIdentity, ask);
            form.sent(command, chosen, cx);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{CardAmount, CreateIdentityForm, short_of};
    use evolite_api::CoreError;

    /// FEAT-0014 (frames 9, 10; BUG-77): Core checked now says how much is missing; a balance
    /// of an earlier check, or none, says nothing of the kind and spends nothing.
    #[test]
    fn short_of_core_only_when_checked_now() {
        assert_eq!(
            short_of(CardAmount::Now(15_000_000), 30_002_000, 226),
            Some(Some(30_002_226))
        );
        assert_eq!(
            short_of(CardAmount::Now(841_200_000), 30_002_000, 226),
            Some(None)
        );
        assert_eq!(
            short_of(CardAmount::Earlier(841_200_000), 30_002_000, 226),
            None
        );
        assert_eq!(short_of(CardAmount::Unknown, 30_002_000, 226), None);
    }

    /// FEAT-0014: a wrong password opens the review again, with nothing made; only an `Ok`
    /// counts as made.
    #[test]
    fn a_wrong_password_reopens_the_review() {
        let mut form = CreateIdentityForm {
            waiting: true,
            ..Default::default()
        };
        assert!(!form.on_done(&Err(CoreError::WrongPassword)));
        assert!(form.confirming && !form.waiting && form.result.is_none());
        form.confirming = false;
        assert!(!form.on_done(&Err(CoreError::NotEnough)));
        assert!(!form.confirming);
        assert!(form.on_done(&Ok(())));
    }

    /// TECH-price-check AC-T2: the fees on top of the amount, as this form's plan adds them
    /// up: its total without the amount itself.
    #[test]
    fn the_price_on_record_is_the_one_the_form_adds_up() {
        use evolite_core::price::{Seen, declared};
        let fees = crate::screens::shielded::tests::fees();
        let seen = Seen {
            fees: fees.clone(),
            core: 0,
            credits: 0,
            platform_address: 0,
            pool: 0,
            mixed: 0,
            platform_inputs: Vec::new(),
            first_name: false,
        };
        for (name, named) in [(Some("bob".to_string()), true), (None, false)] {
            let top_up = 500;
            let lock = fees.identity_lock_for(top_up, named);
            let plan = super::Plan {
                label: name.clone(),
                top_up,
                name_fee: if named { fees.name_on_top() } else { 0 },
                create_fee: fees.identity_create,
                lock,
                core_fee: fees.core_payment_for(lock),
            };
            let op = evolite_api::Spend::CreateIdentity { name, top_up };
            assert_eq!(declared(&op, &seen, 0).declared, plan.total() - top_up);
        }
    }
}
