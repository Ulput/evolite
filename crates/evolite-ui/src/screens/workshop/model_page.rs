//! W7 «Model» (FEAT-0038 UC-6), W8 «Give» (UC-8) and «Issue more», the series' plate (W12,
//! W12b) on the model's page. Numbers, not a row per copy (AC-D12): how many one has, which
//! are for sale at what price, how many were sold; the actions go as one series — how many,
//! one «Check» pinned over the password (BUG-249), one signature, one write per copy.

use super::*;
use crate::components::spend_flow::SpendFlow;
use crate::components::{
    FieldKind, Part, dialog_with_summary, error_notice, field_label, kv_row, notice, text_field,
};
use evolite_api::series::{SeriesKind, SeriesPlan, SeriesRefusal, SeriesState, SeriesTarget};
use evolite_api::workshop::{Count, Model};
use evolite_api::{Duffs, Spend};

/// How long a form rests before the core plans its series again.
const SETTLE_SECS: f64 = 0.6;

/// Which series the window makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// W8: give free copies to someone.
    Give,
    /// «Issue more»: copies after the highest issued, within the supply.
    Issue,
    /// W12b «Continue»: what a stopped series has left (no plan: its own numbers).
    Continue,
}

/// The series window: how many, to whom, its plan, its password.
pub struct SeriesForm {
    pub action: Action,
    target: SeriesTarget,
    how_many: String,
    to: String,
    /// A stopped series «Continue» resumes, with what it has left at its write's estimate.
    resume: Option<(u64, u32, Duffs)>,
    /// The series resumed gives (else it issues): its window is the one of its kind.
    resume_gives: bool,
    asked: Option<SeriesKind>,
    changed_at: f64,
    plan: Option<(SeriesKind, Result<SeriesPlan, CoreError>)>,
    pub(crate) flow: SpendFlow<()>,
}

impl SeriesForm {
    pub fn new(action: Action, target: SeriesTarget) -> Self {
        Self {
            action,
            target,
            how_many: "1".into(),
            to: String::new(),
            resume: None,
            resume_gives: false,
            asked: None,
            changed_at: 0.0,
            plan: None,
            flow: SpendFlow::default(),
        }
    }

    /// `--screen workshop-give` / `-issue`: two copies, to @alice when given; asked of the core
    /// at once (the sweeps wait only for a quiet screen).
    #[cfg(feature = "dev-tools")]
    pub fn preset(action: Action, target: SeriesTarget) -> Self {
        let mut form = Self::new(action, target);
        form.how_many = "2".into();
        form.to = "@alice".into();
        form.changed_at = -SETTLE_SECS;
        form
    }

    /// W12b: «Continue» the stopped `series`.
    pub fn resume(series: &SeriesState) -> Self {
        let left = series.total.saturating_sub(series.done + series.refused);
        let mut form = Self::new(Action::Continue, series.target.clone());
        form.resume = Some((series.id, left, series.per_write));
        form.resume_gives = matches!(series.kind, SeriesKind::Gift { .. });
        form
    }

    /// The series the form makes now; `None` while «How many» or «To» is not filled.
    fn kind(&self) -> Option<SeriesKind> {
        let count: u32 = self.how_many.trim().parse().ok().filter(|n| *n > 0)?;
        match self.action {
            Action::Give => {
                let to = self.to.trim();
                (!to.is_empty()).then(|| SeriesKind::Gift {
                    count,
                    to: to.to_owned(),
                })
            }
            Action::Issue => Some(SeriesKind::Mint { count }),
            Action::Continue => None,
        }
    }

    /// [`evolite_api::Event::SeriesPlanned`]: kept for the series it planned.
    pub fn on_planned(
        &mut self,
        target: &SeriesTarget,
        kind: &SeriesKind,
        result: &Result<SeriesPlan, CoreError>,
    ) {
        if *target == self.target && self.asked.as_ref() == Some(kind) {
            self.plan = Some((kind.clone(), result.clone()));
        }
    }

    pub fn on_done(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) -> bool {
        self.flow.on_done(ask, result)
    }

    fn planned(&self) -> Option<&Result<SeriesPlan, CoreError>> {
        let kind = self.kind()?;
        self.plan
            .as_ref()
            .filter(|(k, _)| *k == kind)
            .map(|(_, r)| r)
    }

    pub fn forget(&mut self) {
        self.flow.forget();
    }
}

/// A series refusal as W7–W8 say it.
fn refusal_words(r: &SeriesRefusal, who: &str) -> String {
    match r {
        SeriesRefusal::NotEnoughInstances { have: 0 } => tr!("You have no free copies").into(),
        SeriesRefusal::NotEnoughInstances { have } => format!(tr!("Free copies: {}"), have),
        SeriesRefusal::TooFewFound { found } => format!(
            tr!("Free copies found among those read: {}. Try fewer."),
            found
        ),
        SeriesRefusal::PastSupply { left } => {
            format!(tr!("Within the supply, {} more can be issued."), left)
        }
        SeriesRefusal::ToSelf => tr!("You cannot give to yourself").into(),
        SeriesRefusal::NoRecipient => tr!("No one with this name or id was found.").into(),
        SeriesRefusal::SeriesRunning => tr!("A series is running — wait for it or stop it").into(),
        SeriesRefusal::NoKey => tr!(
            "This identity has no CRITICAL key for writing contracts. Add one in Identity › Keys."
        )
        .into(),
        SeriesRefusal::NotEnoughCredits { have, need } => format!(
            tr!("Not enough credits: about {need} is needed, {name} has {have}."),
            need = crate::format::amount(*need),
            name = crate::i18n::own(who),
            have = crate::format::amount(*have)
        ),
        _ => tr!("This series cannot be written.").into(),
    }
}

/// A number the core counted, or «—».
fn count(c: Option<Count>) -> Option<u64> {
    match c? {
        Count::Proved(n) => Some(n),
        Count::Counted { count, .. } => Some(count),
    }
}

/// W7: the model, its numbers, its actions, its series' plate.
pub(super) fn page(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut WorkshopState,
    art: &mut crate::art::ArtCache,
    contract: &str,
    model_id: &str,
) {
    let p = a.p;
    ui.spacing_mut().item_spacing.y = 16.0;
    if crate::components::back_link(ui, a, tr!("Collection")).clicked() {
        state.page = Page::Collection(contract.to_owned());
        return;
    }
    let model: Option<Model> = match &state.collections {
        Some(Ok(all)) => all
            .iter()
            .find(|c| c.contract == contract)
            .and_then(|c| c.models.iter().find(|m| m.id == model_id))
            .cloned(),
        _ => None,
    };
    let Some(model) = model else {
        page_header(ui, a, tr!("Model"), None, |_, _| {});
        ui.label(txt(
            tr!("Reading your collections…"),
            13.0,
            Weight::Regular,
            p.muted,
        ));
        return;
    };
    let wallet = cx.wallet().clone();
    let identity = wallet
        .identity
        .as_ref()
        .map(|i| i.id.clone())
        .unwrap_or_default();
    let target = SeriesTarget {
        identity: identity.clone(),
        contract: contract.to_owned(),
        model: model_id.to_owned(),
    };
    let key = (contract.to_owned(), model_id.to_owned());
    if state.counts_asked.insert(key.clone()) {
        cx.send(Command::ReadWorkshop {
            ask: WorkshopAsk::Counts {
                contract: contract.to_owned(),
                model: model_id.to_owned(),
            },
        });
    }
    let counts = state.counts.get(&key).cloned();
    let mine = counts.as_ref().and_then(|c| count(c.mine));
    let for_sale = counts.as_ref().and_then(|c| count(c.for_sale)).unwrap_or(0);
    let free = mine.map(|m| m.saturating_sub(for_sale));
    let issued = counts.as_ref().and_then(|c| c.issued);
    let series_here: Vec<SeriesState> = wallet
        .series
        .iter()
        .filter(|s| s.target.contract == contract && s.target.model == model_id)
        .cloned()
        .collect();
    let running = wallet
        .series
        .iter()
        .any(|s| s.target.identity == identity && s.running);

    page_header(ui, a, &crate::i18n::own(&model.name), None, |_, _| {});
    let supply_line = format!(
        tr!("supply {} · issued {}"),
        crate::format::count(model.supply as u64),
        issued.map_or("—".to_owned(), |n| crate::format::count(n as u64))
    );
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        let (rect, _) = ui.allocate_exact_size(vec2(160.0, 160.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(12), p.raised);
        if !model.art.is_empty() {
            let mut send = |c: Command| cx.send(c);
            art.paint(ui, &p, &model.art, rect, &mut send);
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(txt(&supply_line, 13.0, Weight::Medium, p.muted));
            if !model.description.is_empty() {
                super::super::tokens::para(
                    ui,
                    &crate::i18n::own(&model.description),
                    13.0,
                    p.text,
                    None,
                );
            }
        });
    });
    // The numbers (AC-D12): «—» until the core counted them.
    let shown = |n: Option<u64>| n.map_or("—".to_owned(), crate::format::count);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        number_tile(ui, a, tr!("You have"), &shown(mine), None);
        let prices: Vec<String> = counts
            .as_ref()
            .map(|c| {
                c.prices
                    .iter()
                    .map(|(price, n)| {
                        format!(
                            tr!("at {} — {}"),
                            crate::format::amount(*price),
                            crate::format::count(*n as u64)
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        number_tile(
            ui,
            a,
            tr!("For sale"),
            &shown(counts.as_ref().map(|_| for_sale)),
            Some(&prices.join("\n")),
        );
        number_tile(
            ui,
            a,
            tr!("Sold"),
            &shown(counts.as_ref().and_then(|c| c.sold).map(u64::from)),
            None,
        );
    });
    // The series' plate (W12, W12b).
    for s in &series_here {
        series_plate(ui, a, cx, state, s);
    }
    // The actions (W7): one series at a time on the identity (AC-A8).
    let left_to_issue = issued.map(|i| model.supply.saturating_sub(i));
    let why_not_give = if running {
        Some(tr!("A series is running — wait for it or stop it"))
    } else if free == Some(0) {
        Some(tr!("You have no free copies"))
    } else {
        None
    };
    let why_not_issue = if running {
        Some(tr!("A series is running — wait for it or stop it"))
    } else if left_to_issue == Some(0) {
        Some(tr!("The whole supply is issued"))
    } else {
        None
    };
    let (mut give, mut issue) = (false, false);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        give = button_enabled(
            ui,
            a,
            tr!("Give…"),
            ButtonKind::Primary,
            None,
            why_not_give.is_none(),
        )
        .clicked();
        issue = button_enabled(
            ui,
            a,
            tr!("Issue more"),
            ButtonKind::Secondary,
            None,
            why_not_issue.is_none(),
        )
        .clicked();
    });
    for why in [why_not_give, why_not_issue].into_iter().flatten().take(1) {
        ui.label(txt(why, 12.0, Weight::Regular, p.muted));
    }
    if give {
        state.series = Some(SeriesForm::new(Action::Give, target.clone()));
    }
    if issue {
        state.series = Some(SeriesForm::new(Action::Issue, target.clone()));
    }
    if state.series.is_some() {
        series_window(ui, a, cx, state, &model.name, free, left_to_issue);
    }
}

/// A number with its label and, below, its parts (the prices).
fn number_tile(ui: &mut Ui, a: &mut Assets, label: &str, value: &str, parts: Option<&str>) {
    let p = a.p;
    crate::components::card(&p, egui::Margin::same(16)).show(ui, |ui| {
        ui.set_width(180.0);
        // In a row of tiles: the label over the number, not beside it.
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(txt(label, 12.0, Weight::Medium, p.muted));
            ui.label(txt(value, 22.0, Weight::SemiBold, p.text));
            if let Some(parts) = parts.filter(|s| !s.is_empty()) {
                ui.label(txt(parts, 12.0, Weight::Regular, p.accent));
            }
        });
    });
}

/// W12 (running) or W12b (stopped with copies left): where the series is, and what can be done.
fn series_plate(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut WorkshopState,
    s: &SeriesState,
) {
    let p = a.p;
    let what = match &s.kind {
        SeriesKind::Gift { .. } => format!(tr!("Given {} of {}"), s.done, s.total),
        SeriesKind::Mint { .. } => format!(tr!("Issued {} of {}"), s.done, s.total),
        _ => format!(tr!("Done {} of {}"), s.done, s.total),
    };
    let left = s.total.saturating_sub(s.done + s.refused);
    let width = ui.available_width();
    crate::components::card(&p, egui::Margin::same(16)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 16));
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.label(txt(
            &what,
            14.0,
            Weight::SemiBold,
            p.text,
        ));
        crate::components::progress_bar(ui, &p, s.done as f32 / s.total.max(1) as f32);
        if s.refused > 0 {
            ui.label(txt(
                &format!(tr!("refused: {}"), s.refused),
                12.0,
                Weight::Regular,
                p.danger,
            ));
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if s.running {
                ui.label(txt(
                    tr!("The series goes on in the background; you can leave this page."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
                if !s.stopping
                    && button(ui, a, tr!("Stop after the current ones"), ButtonKind::Ghost, None)
                        .clicked()
                {
                    cx.send(Command::StopSeries { id: s.id });
                }
            } else {
                if left > 0
                    && button(ui, a, tr!("Continue"), ButtonKind::Primary, None).clicked()
                {
                    state.series = Some(SeriesForm::resume(s));
                }
                if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
                    cx.send(Command::CloseSeries { id: s.id });
                }
            }
        });
        if !s.running && left > 0 {
            super::super::tokens::para(
                ui,
                tr!(
                    "Before continuing, evolite reads again on the network what was sent without an answer — what is done is not repeated. The password is needed."
                ),
                12.0,
                p.muted,
                None,
            );
        }
    });
}

/// W8 «Give», «Issue more», W12b «Continue»: how many (and to whom), the plan, the summary
/// pinned over the password (BUG-249), «Sign and …».
fn series_window(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut WorkshopState,
    model: &str,
    free: Option<u64>,
    left_to_issue: Option<u32>,
) {
    let Some(form) = state.series.as_mut() else {
        return;
    };
    // Signed and answered well: the plate takes over.
    if matches!(form.flow.result(), Some(Ok(()))) {
        state.series = None;
        state.counts.clear();
        state.counts_asked.clear();
        return;
    }
    let now = ui.input(|i| i.time);
    if let Some(kind) = form.kind()
        && form.asked.as_ref() != Some(&kind)
    {
        if now - form.changed_at >= SETTLE_SECS {
            form.asked = Some(kind.clone());
            cx.send(Command::PlanSeries {
                target: form.target.clone(),
                kind,
            });
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(SETTLE_SECS));
        }
    }
    let wallet = cx.wallet().clone();
    let credits = wallet
        .balances
        .credits_read
        .then_some(wallet.balances.credits);
    let who = super::super::tokens::shown_name(&wallet);
    let name = crate::i18n::own(model);
    let title = match form.action {
        Action::Give => format!(tr!("Give “{}”"), name),
        Action::Issue => format!(tr!("Issue more “{}”"), name),
        Action::Continue => format!(tr!("Continue “{}”"), name),
    };
    // One window per kind of series (BUG-249 r3: an id the guard reads): «Continue» opens the
    // one of the series it resumes.
    let gives = match form.action {
        Action::Give => true,
        Action::Issue => false,
        Action::Continue => form.resume_gives,
    };
    // What is signed: the plan's records and total, or a stopped series' rest.
    let planned = form.planned().cloned();
    let (records, total, op, why): (u32, Option<Duffs>, Option<Spend>, Option<String>) =
        match (form.action, &planned, form.resume) {
            (Action::Continue, _, Some((id, left, per_write))) => {
                let fee = per_write.saturating_mul(left as Duffs);
                (
                    left,
                    Some(fee),
                    Some(Spend::ContinueSeries { id, fee_max: fee }),
                    None,
                )
            }
            (_, Some(Ok(plan)), _) => {
                let op = form.kind().map(|kind| Spend::NftBatch {
                    target: form.target.clone(),
                    kind,
                    fee_max: plan.total,
                });
                let why = plan.refusal.as_ref().map(|r| refusal_words(r, &who));
                (plan.serials.len() as u32, Some(plan.total), op, why)
            }
            (_, Some(Err(e)), _) => (0, None, None, Some(crate::text::error_text(e))),
            _ => (0, None, None, Some(String::new())),
        };
    if let Some(op) = &op {
        // FEAT-0004 FR-7: the large spend's tick from the start, in the summary.
        form.flow.password.watch(Some(op));
    }
    let serials = planned
        .as_ref()
        .and_then(|p| p.as_ref().ok())
        .and_then(|p| Some((*p.serials.first()?, *p.serials.last()?)));
    let mut changed = false;
    let body = |ui: &mut Ui, a: &mut Assets, part: Part| {
        let p = a.p;
        let value = crate::theme::mono(13.0, Weight::Medium);
        if part == Part::Summary {
            ui.spacing_mut().item_spacing.y = 10.0;
            if let Some(total) = total {
                kv_row(
                    ui,
                    &p,
                    &crate::text::plural(records, tr!("record"), tr!("records")),
                    &format!("≈ {}", crate::format::amount(total)),
                    value.clone(),
                );
                if let Some(c) = credits {
                    kv_row(
                        ui,
                        &p,
                        tr!("Credits after"),
                        &format!("≈ {}", crate::format::amount(c - total)),
                        value.clone(),
                    );
                }
            }
            if form.action == Action::Give {
                crate::components::warning(
                    ui,
                    a,
                    tr!(
                        "If you are in the recipient's contacts, the gift goes straight to their “Collection”. Otherwise it shows on their “New” tab, and they decide whether to keep it or delete it. You cannot take a gift back."
                    ),
                );
            }
            form.flow.password.summary_tick(ui, &p);
            return false;
        }
        if part == Part::Footer {
            let mut close = false;
            form.flow
                .password
                .field(ui, &p, "workshop-series-password", true);
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
            let ready = why.is_none()
                && op.is_some()
                && form.flow.password.ready(true)
                && form.flow.can_act();
            let label = match form.action {
                Action::Give => format!(tr!("Sign and give {}"), records),
                Action::Issue => format!(tr!("Sign and issue {}"), records),
                Action::Continue => tr!("Sign and continue").to_owned(),
            };
            let mut go = false;
            crate::components::button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                go = button_enabled(ui, a, &label, ButtonKind::Primary, None, ready).clicked();
            });
            if go
                && let Some(op) = op.clone()
                && let Some(command) = form.flow.command(op, true, ())
            {
                cx.send(command);
            }
            return close;
        }
        ui.spacing_mut().item_spacing.y = 8.0;
        if form.action != Action::Continue {
            let of = match form.action {
                Action::Give => free,
                _ => left_to_issue.map(u64::from),
            };
            ui.horizontal(|ui| {
                field_label(ui, &p, tr!("How many"));
                if let Some(of) = of {
                    ui.label(txt(
                        &format!(tr!("of {}"), crate::format::count(of)),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
            });
            let before = (form.how_many.clone(), form.to.clone());
            text_field(
                ui,
                &p,
                "workshop-series-count",
                &mut form.how_many,
                "1",
                FieldKind::Text,
                |_| {},
            );
            form.how_many.retain(|c| c.is_ascii_digit());
            if form.action == Action::Give {
                field_label(ui, &p, tr!("Recipient"));
                text_field(
                    ui,
                    &p,
                    "workshop-series-to",
                    &mut form.to,
                    tr!("@name or identity id"),
                    FieldKind::Text,
                    |_| {},
                );
            }
            if (form.how_many.clone(), form.to.clone()) != before {
                changed = true;
            }
            if let Some((first, last)) = serials {
                let words = match form.action {
                    Action::Give => format!(
                        tr!(
                            "The free copies with the lowest numbers go — no. {}–{}. Those for sale are not touched."
                        ),
                        first, last
                    ),
                    _ => format!(tr!("Numbers no. {}–{} are issued."), first, last),
                };
                ui.label(txt(&words, 12.0, Weight::Regular, p.muted));
            }
        } else {
            ui.label(txt(
                &format!(tr!("Left to write: {}"), records),
                13.0,
                Weight::Regular,
                p.text,
            ));
        }
        if let Some(why) = why.as_deref().filter(|w| !w.is_empty()) {
            notice(ui, a, "warning", p.danger, why);
        }
        super::super::tokens::para(
            ui,
            tr!(
                "One record per copy, up to 4 at once. The window can be closed — the series goes on in the background, its progress shows on the model's page."
            ),
            12.0,
            p.muted,
            None,
        );
        false
    };
    let closed = if gives {
        dialog_with_summary(ui, a, "workshop-give", "nft", &title, 480.0, body)
    } else {
        dialog_with_summary(ui, a, "workshop-issue", "nft", &title, 480.0, body)
    };
    if changed {
        form.changed_at = now;
    }
    if closed && !form.flow.waiting() {
        form.forget();
        state.series = None;
    }
}
