//! W2 «New NFT collection», its review W3 and the steps after signing (FEAT-0038 UC-4,
//! AC-A4, AC-A16): the name (the contract's description, 3–100 characters, no link), «Belongs
//! to a contract» (optional), the rules the standard guarantees (nothing to choose), and the
//! summary with the fee checked by the core before the password ([`Command::CheckContract`]).
//! Signed, it registers the collection and, when asked, writes its «About the collection»;
//! then the collection's page (W4).

use super::*;
use crate::components::spend_flow::SpendFlow;
use crate::components::{
    FieldKind, Part, dialog_with_summary, error_notice, field_label, kv_row_fit, notice, text_field,
};
use evolite_api::contracts::{
    ABOUT_NOT_WRITTEN, ABOUT_UNCLEAR, ContractCheck, ContractDraft, ContractFee, ContractRefusal,
};
use evolite_api::{Duffs, Spend};

/// The name's bounds: the contract's description (module V1 rules, 3–100).
const NAME_MIN: usize = 3;
const NAME_MAX: usize = 100;
/// How long the form rests before the core is asked for the fee again.
const SETTLE_SECS: f64 = 0.6;

/// The form, its check, its review and its steps.
#[derive(Default)]
pub struct NewCollection {
    name: String,
    main: String,
    /// The draft last sent to the core for its check, and when the form last changed.
    asked: Option<ContractDraft>,
    changed_at: f64,
    /// The core's check of the draft it names.
    check: Option<(ContractDraft, Result<ContractCheck, CoreError>)>,
    /// BUG-266: when a «not synced» check was first seen; it is asked again.
    not_synced: evolite_api::retry::Retry,
    /// W7 r2: the sync's stage when the form last looked.
    sync_seen: Option<evolite_api::SyncStage>,
    /// W3 open.
    review: bool,
    /// What was signed: the name, the identity, the fee; then the answer.
    pub(crate) flow: SpendFlow<Signed>,
    /// The collection the registration made, once the core said so.
    registered: Option<String>,
}

/// What W3 showed when it was signed, for the steps.
#[derive(Default, Debug, PartialEq, Clone)]
pub struct Signed {
    name: String,
    identity: String,
    about: bool,
}

impl NewCollection {
    /// «Create NFT» on W1: a fresh form, unless one is signed and still going.
    pub fn open(&mut self) {
        if !self.signed() {
            self.forget();
        }
    }

    /// Something was signed and its steps are shown.
    pub fn signed(&self) -> bool {
        self.flow.signed().is_some()
    }

    /// Back to an empty form, the password wiped.
    pub fn forget(&mut self) {
        self.flow.forget();
        *self = Self::default();
    }

    /// `--screen workshop-new`, `-review`, `-steps`: the form with `name`, its review open (it
    /// waits for the core's check), or its steps as if signed and waiting.
    #[cfg(feature = "dev-tools")]
    pub fn preset(&mut self, name: &str, review: bool, steps: bool) {
        self.name = name.into();
        self.review = review;
        // Asked of the core at once: the review opens without waiting for the form to rest.
        self.changed_at = -SETTLE_SECS;
        if steps {
            self.flow
                .set_stage(crate::components::spend_flow::Stage::Waiting {
                    ask: 0,
                    signed: Signed {
                        name: name.into(),
                        identity: "@satoshi".into(),
                        about: true,
                    },
                });
            self.registered = Some("FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P".into());
        }
    }

    /// The draft the form makes now.
    fn draft(&self) -> ContractDraft {
        let main = self.main.trim();
        ContractDraft::NftCollection {
            description: Some(self.name.trim().to_owned()),
            keywords: Vec::new(),
            main_contract: (!main.is_empty()).then(|| main.to_owned()),
        }
    }

    /// [`evolite_api::Event::ContractChecked`]: kept for the draft it checked.
    pub fn on_checked(&mut self, draft: &ContractDraft, result: &Result<ContractCheck, CoreError>) {
        if self.asked.as_ref() == Some(draft) {
            self.check = Some((draft.clone(), result.clone()));
        }
    }

    /// [`evolite_api::Event::ContractRegistered`]: the collection is there.
    pub fn on_registered(&mut self, id: &str) {
        if self.signed() {
            self.registered = Some(id.to_owned());
        }
    }

    /// The `Done` of the registration; whether it was this form's.
    pub fn on_done(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) -> bool {
        let mine = self.flow.on_done(ask, result);
        if mine && self.flow.password.is_wrong() {
            self.review = true;
        }
        mine
    }

    /// The check of what the form shows now, if it came.
    fn checked(&self) -> Option<&Result<ContractCheck, CoreError>> {
        self.check
            .as_ref()
            .filter(|(d, _)| *d == self.draft())
            .map(|(_, r)| r)
    }
}

/// Why the name cannot go, said under it; `None`: it can.
fn name_problem(name: &str) -> Option<String> {
    let name = name.trim();
    let n = name.chars().count();
    if n == 0 {
        return Some(String::new());
    }
    if evolite_api::series::has_link(name) {
        return Some(
            tr!("No links or site addresses: people find the collection by its name.").into(),
        );
    }
    (!(NAME_MIN..=NAME_MAX).contains(&n)).then(|| {
        format!(
            tr!("The name takes {min} to {max} characters."),
            min = NAME_MIN,
            max = NAME_MAX
        )
    })
}

/// A refusal before the password as the screen says it (AC-A16: the missing key with its way).
fn refusal_words(r: &ContractRefusal, who: &str) -> String {
    match r {
        ContractRefusal::NoKey => tr!(
            "This identity has no CRITICAL key for writing contracts. Add one in Identity › Keys."
        )
        .into(),
        ContractRefusal::OnItsWay => {
            tr!("The earlier registration may still go through — wait a couple of minutes").into()
        }
        ContractRefusal::NotEnoughCredits { have, need } => format!(
            tr!("Not enough credits: about {need} is needed, {name} has {have}."),
            need = crate::format::amount(*need),
            name = crate::i18n::own(who),
            have = crate::format::amount(*have)
        ),
        ContractRefusal::DescriptionLength(_) => format!(
            tr!("The name takes {min} to {max} characters."),
            min = NAME_MIN,
            max = NAME_MAX
        ),
        ContractRefusal::MainContractId => tr!("This is not a contract id.").into(),
        ContractRefusal::MainContractMissing => {
            tr!("There is no contract with this id on Dash Platform.").into()
        }
        ContractRefusal::NewerNetwork(_) => {
            tr!("The network runs a newer version of Dash Platform. Update evolite.").into()
        }
        ContractRefusal::TooManyKeywords(_)
        | ContractRefusal::KeywordLength { .. }
        | ContractRefusal::KeywordCharacter { .. }
        | ContractRefusal::KeywordRepeated { .. }
        | ContractRefusal::Invalid(_) => {
            tr!("Dash Platform's rules refuse this collection.").into()
        }
    }
}

/// W2, or its steps once signed.
pub(super) fn page(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut WorkshopState) {
    let p = a.p;
    let wallet = cx.wallet().clone();
    let Some(identity) = wallet.identity.as_ref().map(|i| i.id.clone()) else {
        state.page = Page::Home;
        return;
    };
    ui.spacing_mut().item_spacing.y = 16.0;
    if crate::components::back_link(ui, a, tr!("Workshop")).clicked() && !state.new.flow.waiting() {
        state.page = Page::Home;
        if !state.new.flow.waiting() && state.new.signed() {
            state.new.forget();
        }
        return;
    }
    page_header(ui, a, tr!("New NFT collection"), None, |_, _| {});
    if state.new.signed() {
        steps(ui, a, cx, state);
        return;
    }
    let form = &mut state.new;
    let now = ui.input(|i| i.time);
    let who = super::super::tokens::shown_name(&wallet);
    // Ask the core again once the form rests on a draft it has not checked.
    let draft = form.draft();
    let fits = name_problem(&form.name).is_none();
    // BUG-266: «wait for the sync» is not the last word: asked again a moment later.
    let answer = super::not_synced(form.checked());
    let moved = super::sync_moved(&mut form.sync_seen, cx.snap.sync.stage);
    if super::retry_not_synced(ui, now, answer, moved, &mut form.not_synced) {
        form.asked = None;
        form.check = None;
    }
    if fits && form.asked.as_ref() != Some(&draft) {
        if now - form.changed_at >= SETTLE_SECS {
            form.asked = Some(draft.clone());
            cx.send(Command::CheckContract {
                identity: identity.clone(),
                draft: draft.clone(),
            });
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(SETTLE_SECS));
        }
    }
    let checked = form.checked().cloned();
    let fee = checked
        .as_ref()
        .and_then(|c| c.as_ref().ok())
        .and_then(|c| c.fee);
    let credits = wallet
        .balances
        .credits_read
        .then_some(wallet.balances.credits);
    // Why «Check» waits: the name, the core's refusal, the credits (W2, «as in Send»).
    let why: Option<String> = match name_problem(&form.name) {
        Some(problem) => Some(problem),
        None => match &checked {
            None => Some(String::new()),
            Some(Err(e)) => Some(crate::text::error_text(e)),
            Some(Ok(c)) => match (&c.refusal, c.fee, credits) {
                (Some(r), _, _) => Some(refusal_words(r, &who)),
                (None, Some(fee), Some(have)) if have < fee.total() => Some(format!(
                    tr!("Not enough credits: about {need} is needed, {name} has {have}."),
                    need = crate::format::amount(fee.total()),
                    name = crate::i18n::own(&who),
                    have = crate::format::amount(have)
                )),
                (None, None, _) | (None, _, None) => Some(String::new()),
                (None, Some(_), Some(_)) => None,
            },
        },
    };
    let wide = ui.available_width() >= 760.0;
    let mut check_pressed = false;
    let mut cancel = false;
    let left = |ui: &mut Ui, a: &mut Assets, form: &mut NewCollection| {
        form_card(ui, a, form, now, checked.as_ref());
    };
    // BUG-272: asked (or about to be) and not answered yet.
    let counting = fits && checked.is_none();
    let mut right = |ui: &mut Ui, a: &mut Assets| {
        summary(
            ui,
            a,
            &who,
            crate::components::Credits::of(&wallet),
            fee,
            counting,
            why.as_deref(),
            &mut check_pressed,
            &mut cancel,
        );
    };
    if wide {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            let total = ui.available_width();
            let side = (total * 0.36).clamp(300.0, 380.0);
            ui.vertical(|ui| {
                ui.set_width(total - side - 20.0);
                left(ui, a, form);
            });
            ui.vertical(|ui| {
                ui.set_width(side);
                right(ui, a);
            });
        });
    } else {
        left(ui, a, form);
        right(ui, a);
    }
    if cancel {
        state.new.forget();
        state.page = Page::Home;
        return;
    }
    if check_pressed && why.is_none() {
        state.new.review = true;
    }
    if state.new.review
        && let Some(fee) = fee
    {
        review(ui, a, cx, &mut state.new, &identity, &who, credits, fee);
    }
    let _ = p;
}

/// The left column: name, «Belongs to a contract», the rules, the warning.
fn form_card(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut NewCollection,
    now: f64,
    checked: Option<&Result<ContractCheck, CoreError>>,
) {
    let p = a.p;
    let width = ui.available_width();
    crate::components::card(&p, egui::Margin::same(24)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 24));
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            field_label(ui, &p, tr!("Collection name"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let n = form.name.trim().chars().count();
                ui.label(txt(
                    &format!("{n} / {NAME_MAX}"),
                    11.0,
                    Weight::Medium,
                    p.muted,
                ));
            });
        });
        let before = (form.name.clone(), form.main.clone());
        text_field(
            ui,
            &p,
            "workshop-collection-name",
            &mut form.name,
            "",
            FieldKind::Text,
            |_| {},
        );
        // Not more than the contract takes: what is typed past it is not kept.
        if form.name.chars().count() > NAME_MAX {
            form.name = form.name.chars().take(NAME_MAX).collect();
        }
        match name_problem(&form.name).filter(|w| !w.is_empty()) {
            Some(problem) => {
                ui.label(txt(&problem, 12.0, Weight::Regular, p.danger));
            }
            None => super::super::tokens::para(
                ui,
                tr!(
                    "This is the contract’s description: people find the collection by it. No links or site addresses."
                ),
                12.0,
                p.muted,
                None,
            ),
        }
        ui.add_space(8.0);
        field_label(ui, &p, tr!("Belongs to a contract · optional"));
        text_field(
            ui,
            &p,
            "workshop-collection-main",
            &mut form.main,
            tr!("Contract id, for example of a game or an app"),
            FieldKind::Text,
            |_| {},
        );
        let main_described = checked
            .and_then(|c| c.as_ref().ok())
            .and_then(|c| c.main_description.as_ref());
        match main_described.filter(|_| !form.main.trim().is_empty()) {
            Some(described) => {
                ui.label(txt(
                    &crate::i18n::own(described),
                    12.0,
                    Weight::Medium,
                    p.text,
                ));
            }
            None => super::super::tokens::para(
                ui,
                tr!(
                    "Written to the “About the collection” document — it can be changed or deleted later."
                ),
                12.0,
                p.muted,
                None,
            ),
        }
        if (form.name.clone(), form.main.clone()) != before {
            form.changed_at = now;
        }
        ui.add_space(8.0);
        rules(ui, a);
        ui.add_space(8.0);
        crate::components::warning(
            ui,
            a,
            tr!(
                "A collection cannot be changed or deleted once made: the standard requires it, and that is why people trust it. Check the name."
            ),
        );
    });
}

/// «Collection rules»: what the standard guarantees, each with a tick; nothing to choose
/// (AC-A4, AC-D4).
fn rules(ui: &mut Ui, a: &mut Assets) {
    let p = a.p;
    let width = ui.available_width();
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(10)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(width - 32.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(txt(tr!("Collection rules"), 14.0, Weight::SemiBold, p.text));
                ui.label(txt("NFT evolite v1", 12.0, Weight::Medium, p.accent));
            });
            for rule in [
                tr!("Only you issue: the collection is closed"),
                tr!("Copies can be given and sold"),
                tr!("You can see who a copy came from"),
                tr!("The owner of a copy can delete it"),
                tr!("The picture and description cannot change after issue"),
                tr!("No moderators"),
            ] {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.add(a.icons.icon("check", p.muted, &p, 14.0));
                    // BUG-258: a long rule wraps, not widens the column.
                    ui.add(crate::components::para(rule, 13.0, Weight::Regular, p.text));
                });
            }
        });
}

/// The right column «Summary»: identity, credits, the fee, credits left; why «Check» waits.
#[allow(clippy::too_many_arguments)]
fn summary(
    ui: &mut Ui,
    a: &mut Assets,
    who: &str,
    credits: crate::components::Credits,
    fee: Option<ContractFee>,
    counting: bool,
    why: Option<&str>,
    check: &mut bool,
    cancel: &mut bool,
) {
    let p = a.p;
    let width = ui.available_width();
    let value = crate::theme::mono(13.0, Weight::Medium);
    // BUG-259 r1: the «About» document's estimate carries its margin (MINT_FIRST) — then the
    // total is «at most»; the registration alone is the network's own count, «≈».
    let padded = fee.is_some_and(|f| f.about > 0);
    let about = |d: Option<Duffs>| {
        if padded {
            super::at_most(d, counting)
        } else {
            super::estimate(d, counting)
        }
    };
    crate::components::card(&p, egui::Margin::same(24)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 24));
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.label(txt(tr!("Summary"), 18.0, Weight::SemiBold, p.text));
        kv_row_fit(ui, &p, tr!("Identity"), &crate::i18n::own(who), value.clone());
        crate::components::divider(ui, &p);
        let total = fee.map(|f| f.total());
        kv_row_fit(ui, &p, tr!("Collection registration"), &about(total), value.clone());
        // FEAT-0057 П3 (R-3.1): the one line «Credits: now › after» for the registration.
        crate::components::credits_after(
            ui,
            a,
            &crate::i18n::own(who),
            credits,
            match total {
                Some(t) if !counting && padded => crate::components::Charge::AtMost(t),
                Some(t) if !counting => crate::components::Charge::About(t),
                _ => crate::components::Charge::Calculating,
            },
        );
        super::super::tokens::para(
            ui,
            tr!(
                "The price is an estimate before signing. It is paid with the identity’s credits, not DASH from Core."
            ),
            12.0,
            p.muted,
            None,
        );
        crate::components::button_row(ui, |ui| {
            if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                *cancel = true;
            }
            *check = button_enabled(
                ui,
                a,
                tr!("Check"),
                ButtonKind::Primary,
                None,
                why.is_none(),
            )
            .clicked();
        });
        if let Some(why) = why.filter(|w| !w.is_empty()) {
            super::super::tokens::para(ui, why, 12.0, p.danger, None);
        }
    });
}

/// W3 «Check the collection»: the summary, the warning, the password, «Sign and create».
#[allow(clippy::too_many_arguments)]
fn review(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut NewCollection,
    identity: &str,
    who: &str,
    credits: Option<Duffs>,
    fee: ContractFee,
) {
    let name = form.name.trim().to_owned();
    let draft = form.draft();
    let about = !form.main.trim().is_empty();
    let op = Spend::RegisterContract {
        identity: identity.to_owned(),
        draft: draft.clone(),
        fee_max: fee.total(),
    };
    // FEAT-0004 FR-7: the large spend's tick is shown from the start, in the summary.
    form.flow.password.watch(Some(&op));
    // BUG-249 (W0–W4 r2 Must 2): a spend of 10+ DASH: its summary — the fee, the credits after,
    // that it cannot be undone, the tick — pinned over the password, read before the button.
    let closed = dialog_with_summary(
        ui,
        a,
        "workshop-check-collection",
        "nft",
        tr!("Check the collection"),
        480.0,
        |ui, a, part| {
            let p = a.p;
            let value = crate::theme::mono(13.0, Weight::Medium);
            if part == Part::Summary {
                ui.spacing_mut().item_spacing.y = 10.0;
                kv_row_fit(
                    ui,
                    &p,
                    tr!("Registration"),
                    &format!("≈ {}", crate::format::amount(fee.total())),
                    value.clone(),
                );
                if let Some(c) = credits {
                    kv_row_fit(
                        ui,
                        &p,
                        tr!("Credits after"),
                        &format!("≈ {}", crate::format::amount(c - fee.total())),
                        value.clone(),
                    );
                }
                crate::components::warning(
                    ui,
                    a,
                    tr!(
                        "Once signed, the collection cannot be changed or deleted. The registration is not refunded."
                    ),
                );
                form.flow.password.summary_tick(ui, &p);
                return false;
            }
            if part == Part::Footer {
                let mut close = false;
                form.flow
                    .password
                    .field(ui, &p, "workshop-collection-password", true);
                if let Some(Err(e)) = form
                    .flow
                    .result()
                    .filter(|_| !form.flow.password.is_wrong())
                {
                    error_notice(
                        ui,
                        a,
                        &crate::text::error_text(e),
                        crate::text::error_details(e).as_deref(),
                    );
                }
                let ready = form.flow.password.ready(true) && form.flow.can_act();
                let mut go = false;
                crate::components::button_row(ui, |ui| {
                    if button(ui, a, tr!("Back"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    go = button_enabled(
                        ui,
                        a,
                        tr!("Sign and create"),
                        ButtonKind::Primary,
                        None,
                        ready,
                    )
                    .clicked();
                });
                if go {
                    let signed = Signed {
                        name: name.clone(),
                        identity: who.to_owned(),
                        about,
                    };
                    if let Some(command) = form.flow.command(op.clone(), true, signed) {
                        cx.send(command);
                    }
                }
                return close;
            }
            ui.spacing_mut().item_spacing.y = 10.0;
            kv_row_fit(
                ui,
                &p,
                tr!("Collection name"),
                &crate::i18n::own(&name),
                value.clone(),
            );
            kv_row_fit(
                ui,
                &p,
                tr!("On behalf of"),
                &crate::i18n::own(who),
                value.clone(),
            );
            kv_row_fit(
                ui,
                &p,
                tr!("What is made"),
                &format!(
                    "{}, NFT evolite v1",
                    crate::i18n::t(tr!("closed collection"))
                ),
                value.clone(),
            );
            if about {
                kv_row_fit(
                    ui,
                    &p,
                    tr!("Belongs to"),
                    &crate::text::whole(form.main.trim()),
                    value.clone(),
                );
            }
            false
        },
    );
    // Closed only while nothing waits: the answer comes to the steps.
    if closed && !form.flow.waiting() {
        form.review = false;
        if !form.signed() {
            form.flow.forget();
        }
    }
    if form.signed() && !form.flow.password.is_wrong() {
        form.review = false;
    }
}

/// The steps after signing: «Registering the contract › About the collection › Done», then the
/// collection's page.
fn steps(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut WorkshopState) {
    let p = a.p;
    let form = &mut state.new;
    let Some(signed) = form.flow.signed().cloned() else {
        return;
    };
    let result = form.flow.result().cloned();
    let registered = form.registered.clone();
    // «About the collection» not written, or not known (its answer lost, r1 Should 3): the
    // collection is made either way; `true` — not known.
    let about_failed = match &result {
        Some(Err(CoreError::Failed(why))) if registered.is_some() => {
            [(ABOUT_NOT_WRITTEN, false), (ABOUT_UNCLEAR, true)]
                .into_iter()
                .find_map(|(head, unclear)| {
                    why.strip_prefix(head)
                        .map(|rest| (rest.trim_start_matches(':').trim().to_owned(), unclear))
                })
        }
        _ => None,
    };
    // Done well: straight to the collection, its list read again.
    if let (Some(Ok(())), Some(id)) = (&result, &registered) {
        let id = id.clone();
        form.forget();
        state.read_again();
        state.page = Page::Collection(id);
        return;
    }
    let width = ui.available_width().min(560.0);
    crate::components::card(&p, egui::Margin::same(24)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 24));
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.label(txt(
            &format!("«{}»", crate::i18n::own(&signed.name)),
            16.0,
            Weight::SemiBold,
            p.text,
        ));
        let step = |ui: &mut Ui, a: &mut Assets, words: &str, done: Option<bool>| {
            let p = a.p;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (icon, color) = match done {
                    Some(true) => ("check", p.accent),
                    Some(false) => ("warning", p.danger),
                    None => ("pending", p.muted),
                };
                ui.add(a.icons.icon(icon, color, &p, 16.0));
                // BUG-258: wraps beside its icon at the medium window.
                ui.add(crate::components::para(words, 14.0, Weight::Regular, p.text));
            });
        };
        let failed_before = matches!(&result, Some(Err(_))) && registered.is_none();
        step(
            ui,
            a,
            tr!("Registering the contract"),
            if registered.is_some() {
                Some(true)
            } else if failed_before {
                Some(false)
            } else {
                None
            },
        );
        if signed.about {
            step(
                ui,
                a,
                tr!("The “About the collection” document"),
                match (&result, &about_failed) {
                    (_, Some((_, false))) => Some(false),
                    (Some(Ok(())), None) => Some(true),
                    _ => None,
                },
            );
        }
        step(ui, a, tr!("Done"), result.as_ref().map(|r| r.is_ok()).filter(|ok| *ok));
        match (&result, &about_failed) {
            (None, _) => {
                ui.label(txt(
                    tr!("This takes a few seconds."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
            (Some(_), Some((why, unclear))) => {
                let words = if *unclear {
                    tr!(
                        "The collection is made. Whether its “About the collection” document was written is not known: the network did not answer. Open the collection to see."
                    )
                } else {
                    tr!(
                        "The collection is made, but its “About the collection” document was not written."
                    )
                };
                notice(ui, a, "warning", p.accent, words);
                if !why.is_empty() {
                    crate::components::details_line(ui, &p, why);
                }
            }
            (Some(Err(e)), None) => {
                error_notice(
                    ui,
                    a,
                    &crate::text::error_text(e),
                    crate::text::error_details(e).as_deref(),
                );
            }
            (Some(Ok(())), None) => {}
        }
        if result.is_some() {
            crate::components::button_row(ui, |ui| match &registered {
                Some(id) => {
                    if button(ui, a, tr!("Open the collection"), ButtonKind::Primary, None)
                        .clicked()
                    {
                        let id = id.clone();
                        state.new.forget();
                        state.read_again();
                        state.page = Page::Collection(id);
                    }
                }
                None => {
                    if button(ui, a, tr!("Back"), ButtonKind::Secondary, None).clicked() {
                        // Unclear whether it went: the form is kept, its check asked again.
                        let (name, main) = (state.new.name.clone(), state.new.main.clone());
                        state.new.forget();
                        state.new.name = name;
                        state.new.main = main;
                    }
                }
            });
        }
    });
    let _ = cx;
}
