//! W7 «Model» (FEAT-0038 UC-6), W8 «Give» (UC-8) and «Issue more», W9 «Put up for sale», «Change
//! the price» (UC-7) and «Take off sale» (UC-9), the series' plate (W12, W12b) on the model's page. Numbers, not a row per copy (AC-D12): how many one has, which
//! are for sale at what price, how many were sold; the actions go as one series — how many,
//! one «Check» pinned over the password (BUG-249), one signature, one write per copy.

use super::*;
use crate::components::spend_flow::SpendFlow;
use crate::components::{
    FieldKind, Part, dialog_with_summary, dialog_with_summary_under, error_notice, field_label,
    kv_row_fit, notice, text_field,
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
    /// W9 S2: free copies on sale at one price, the lowest numbers first.
    Sell,
    /// W9 S4: the copies at the price `from`, all of them, at a new price.
    Reprice { from: Duffs },
    /// W9 S5: copies off sale, at the price chosen in «Which» or at any, the highest first.
    Unprice,
}

/// Platform's `$price` is in credits: 1 duff = 1 000 credits.
/// A price in duffs as Platform's credits (1 duff = 1 000); `None` for one that is not a price
/// (below 0, or past what credits hold) — no plan then, never a price of 0 (W9 r1, §8d).
fn credits_of(duffs: Duffs) -> Option<evolite_api::series::Credits> {
    u64::try_from(duffs).ok()?.checked_mul(1_000)
}

/// The series window: how many, to whom, its plan, its password.
pub struct SeriesForm {
    pub action: Action,
    target: SeriesTarget,
    how_many: String,
    to: String,
    /// W9: the price of a copy as typed, in the unit of the Settings.
    price: String,
    /// W9 S5 «Which»: the price taken off, or every price.
    which: Option<Duffs>,
    /// A stopped series «Continue» resumes, with what it has left at its write's estimate.
    resume: Option<(u64, u32, Duffs)>,
    /// The series resumed gives (else it issues): its window is the one of its kind.
    resume_gives: bool,
    asked: Option<SeriesKind>,
    changed_at: f64,
    plan: Option<(SeriesKind, Result<SeriesPlan, CoreError>)>,
    /// BUG-266: when a «not synced» plan was first seen; it is asked again.
    not_synced: evolite_api::retry::Retry,
    /// W7 r2: the sync's stage when the form last looked.
    sync_seen: Option<evolite_api::SyncStage>,
    pub(crate) flow: SpendFlow<()>,
}

impl SeriesForm {
    pub fn new(action: Action, target: SeriesTarget) -> Self {
        Self {
            action,
            target,
            // BUG-259: empty, its hint «1» — a «1» typed into ahead of the cursor made «12».
            how_many: String::new(),
            to: String::new(),
            price: String::new(),
            which: None,
            resume: None,
            resume_gives: false,
            asked: None,
            changed_at: 0.0,
            plan: None,
            not_synced: Default::default(),
            sync_seen: None,
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

    /// W9 S5 from a row of «Prices»: `count` copies at `price` (or at any, `None`).
    pub fn take_off(target: SeriesTarget, price: Option<Duffs>, count: u32) -> Self {
        let mut form = Self::new(Action::Unprice, target);
        form.which = price;
        form.how_many = count.to_string();
        form
    }

    /// `--screen workshop-sell` / `-reprice` / `-unprice`: the windows of W9 filled as in the
    /// designer's frames, asked of the core at once.
    #[cfg(feature = "dev-tools")]
    pub fn preset_price(action: Action, target: SeriesTarget) -> Self {
        let mut form = match action {
            Action::Unprice => Self::take_off(target, Some(100_000_000), 2),
            _ => Self::new(action, target),
        };
        if action == Action::Sell {
            form.how_many = "3".into();
        }
        if action != Action::Unprice {
            form.price = format!("0{}5", crate::format::number_marks().1);
        }
        form.changed_at = -SETTLE_SECS;
        form
    }

    /// The price typed, in duffs, when it is one above 0 (AC-A11: 0 or nothing asks nothing).
    fn typed_price(&self) -> Option<Duffs> {
        crate::format::parse_amount(&self.price).filter(|d| *d > 0)
    }

    /// W9 S4: the price typed is the one the copies have now («The same price»).
    fn same_price(&self) -> bool {
        matches!(self.action, Action::Reprice { from } if self.typed_price() == Some(from))
    }

    /// W12b: «Continue» the stopped `series`.
    pub fn resume(series: &SeriesState) -> Self {
        let left = series.total.saturating_sub(series.done + series.refused);
        let mut form = Self::new(Action::Continue, series.target.clone());
        form.resume = Some((series.id, left, series.per_write));
        form.resume_gives = matches!(series.kind, SeriesKind::Gift { .. });
        form
    }

    /// The number typed in «How many», 0 while there is none.
    fn typed_count(&self) -> u32 {
        // A count shown: none typed is none to sign.
        self.how_many
            .trim()
            .parse()
            .ok()
            .filter(|n| *n > 0)
            .unwrap_or(0)
    }

    /// The series the form makes now; `None` while «How many», «To» or the price is not filled.
    fn kind(&self) -> Option<SeriesKind> {
        // S4 takes the whole group: no «How many».
        if let Action::Reprice { from } = self.action {
            let price = self.typed_price().filter(|_| !self.same_price())?;
            return Some(SeriesKind::Reprice {
                from: credits_of(from)?,
                price: credits_of(price)?,
            });
        }
        let count: u32 = self.how_many.trim().parse().ok().filter(|n| *n > 0)?;
        match self.action {
            Action::Sell => Some(SeriesKind::Price {
                count,
                price: credits_of(self.typed_price()?)?,
            }),
            // A price that is not one asks nothing — not «at any price».
            Action::Unprice => Some(SeriesKind::Unprice {
                count,
                price: match self.which {
                    Some(d) => Some(credits_of(d)?),
                    None => None,
                },
            }),
            Action::Reprice { .. } => None,
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
        SeriesRefusal::NoneAtPrice => tr!("None of your copies is for sale at this price").into(),
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
    // W7-2: «‹ {collection}» — back to it, by its name.
    let collection = match &state.collections {
        Some(Ok(all)) => all.iter().find(|c| c.contract == contract).cloned(),
        _ => None,
    };
    let back = collection
        .as_ref()
        .map(|c| crate::i18n::own(&super::collection_name(c)))
        .unwrap_or_else(|| tr!("Collection").into());
    if crate::components::back_link(ui, a, &back).clicked() {
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
    // W7 r1 Must: a read that failed is said and asked again on «Retry» or after 30 s.
    let now = ui.input(|i| i.time);
    state.now = now;
    let again = std::mem::take(&mut state.retry_reads);
    state.on_model_page(ui.ctx().cumulative_pass_nr(), now);
    // BUG-314: «Sold» is the sales the core read: they first, then the counts.
    if state.ask_sales(cx, now, again) && super::ask_now(&mut state.counts_asked, &key, now, again)
    {
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
    state.on_series(&wallet.series);
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

    // W7 (the customer's mockup of 2026-11-22): the picture half the width beside the name, the
    // rules, the supply and its bar, the actions and the numbers; prices, the recent events and
    // the note below. Narrower than [`WIDE`]: one column, the picture the whole width.
    let width = ui.available_width();
    let wide = width >= WIDE;
    let (mine_copies, mine_all) = state.instances(cx, contract, model_id, false, now, again);
    let (sale_copies, sale_all) = state.instances(cx, contract, model_id, true, now, again);
    let about_here: Vec<evolite_api::Activity> = wallet
        .activity
        .iter()
        .filter(|act| about_model(act, &model.name))
        .cloned()
        .collect();
    let who = super::super::tokens::shown_name(&wallet);
    if super::ask_now(&mut state.given_asked, &key, now, again) {
        cx.send(Command::ReadWorkshop {
            ask: WorkshopAsk::Given {
                contract: contract.to_owned(),
                model: model_id.to_owned(),
            },
        });
    }
    let given = state.given.get(&key).cloned();
    let numbers = Numbers {
        mine,
        for_sale: counts.as_ref().map(|_| for_sale),
        free,
        sold: counts.as_ref().and_then(|c| c.sold),
        // BUG-276: from the same sales as the count (their prices), on every copy — not History,
        // which has only the sales this copy saw.
        sold_for: counts.as_ref().and_then(|c| c.sold_for),
        who,
        given,
        now: cx.now,
        prices: counts
            .as_ref()
            .map(|c| c.prices.clone())
            .unwrap_or_default(),
    };
    let left_to_issue = issued.map(|i| model.supply.saturating_sub(i));
    let no_sign_in = super::no_sign_in_key(&wallet);
    let why_not_give = if running {
        Some(tr!("A series is running — wait for it or stop it"))
    } else if no_sign_in.is_some() {
        no_sign_in
    } else if free == Some(0) {
        Some(tr!("You have no free copies"))
    } else {
        None
    };
    let why_not_issue = if running {
        Some(tr!("A series is running — wait for it or stop it"))
    } else if no_sign_in.is_some() {
        no_sign_in
    } else if left_to_issue == Some(0) {
        Some(tr!("The whole supply is issued"))
    } else {
        None
    };
    let (mut give, mut issue, mut sell) = (false, false, false);
    let mut right = |ui: &mut Ui, a: &mut Assets, state: &mut WorkshopState| {
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.add(crate::components::para(
            &crate::i18n::own(&model.name),
            28.0,
            Weight::Bold,
            p.text,
        ));
        if !model.description.is_empty() {
            ui.add(crate::components::para(
                &crate::i18n::own(&model.description),
                14.0,
                Weight::Regular,
                p.text,
            ));
        }
        rule_chips(ui, a, collection.as_ref());
        supply(ui, a, model.supply, issued);
        // W9 S1: «Put up for sale…» first and the main one; «Give…» and «Issue more» beside it.
        // No «Take off sale…» of its own: a row of «Prices» takes off at its price.
        let actions_row = ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(12.0, 8.0);
            sell |= button_enabled(
                ui,
                a,
                tr!("Put up for sale…"),
                ButtonKind::Primary,
                None,
                why_not_give.is_none(),
            )
            .clicked();
            give |= button_enabled(
                ui,
                a,
                tr!("Give…"),
                ButtonKind::Secondary,
                None,
                why_not_give.is_none(),
            )
            .clicked();
            issue |= button_enabled(
                ui,
                a,
                tr!("Issue more"),
                ButtonKind::Secondary,
                None,
                why_not_issue.is_none(),
            )
            .clicked();
        });
        #[cfg(feature = "dev-tools")]
        if state.scroll_to_actions {
            ui.scroll_to_rect(actions_row.response.rect, Some(egui::Align::Min));
        }
        #[cfg(not(feature = "dev-tools"))]
        let _ = (actions_row, state);
        for why in [why_not_give, why_not_issue].into_iter().flatten().take(1) {
            ui.add(crate::components::para(why, 12.0, Weight::Regular, p.muted));
        }
        if wide {
            tiles(ui, a, &numbers);
            your_numbers(ui, a, &mine_copies, mine_all, mine);
        }
    };
    // The series' plate (W12, W12b); W9 S3 (the designer, 2026-11-25): the whole width under
    // «‹ collection», above the picture — after signing, the first thing seen at every width.
    for s in &series_here {
        series_plate(ui, a, cx, state, s);
    }
    if wide {
        let side = ((width - GAP) / 2.0).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            picture(ui, a, cx, art, &model, rect);
            ui.vertical(|ui| {
                ui.set_width(width - side - GAP);
                right(ui, a, state);
            });
        });
    } else {
        let (rect, _) = ui.allocate_exact_size(vec2(width, NARROW_PICTURE), Sense::hover());
        picture(ui, a, cx, art, &model, rect);
        ui.vertical(|ui| {
            ui.set_width(width);
            right(ui, a, state);
        });
        tiles(ui, a, &numbers);
        your_numbers(ui, a, &mine_copies, mine_all, mine);
    }
    if state.page_failed(contract, model_id) {
        // Said, not shown as «—» as while reading; asked again by itself after its pause.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(
                evolite_api::retry::FIRST,
            ));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.add(crate::components::para(
                tr!("Could not read"),
                13.0,
                Weight::Medium,
                p.danger,
            ));
            if crate::components::text_action(ui, a, "sync", tr!("Retry")).clicked() {
                state.retry_reads = true;
            }
        });
    }
    let asked = prices(ui, a, &sale_copies, sale_all, running);
    recent(ui, a, cx, &model.name, &about_here);
    ui.add(crate::components::para(
        tr!(
            "evolite picks the copy numbers itself: for a sale or a gift, the free ones with the lowest numbers. No need to pick them one by one."
        ),
        12.0,
        Weight::Regular,
        p.muted,
    ));
    // W7-7: the contract's id whole, at the advanced level only (FEAT-0051 R-4).
    if cx.advanced() {
        contract_id(ui, a, contract);
    }
    if give {
        state.series = Some(SeriesForm::new(Action::Give, target.clone()));
    }
    if sell {
        state.series = Some(SeriesForm::new(Action::Sell, target.clone()));
    }
    match asked {
        Some(PriceAct::Change(from)) => {
            state.series = Some(SeriesForm::new(Action::Reprice { from }, target.clone()));
        }
        Some(PriceAct::TakeOff(price, count)) => {
            state.series = Some(SeriesForm::take_off(target.clone(), Some(price), count));
        }
        None => {}
    }
    if issue {
        state.series = Some(SeriesForm::new(Action::Issue, target.clone()));
    }
    if state.series.is_some() {
        let have = Have {
            free,
            left_to_issue,
            for_sale: &sale_copies,
            for_sale_read: sale_all,
        };
        series_window(ui, a, cx, state, &model.name, &have);
    }
}

/// W7: content at least this wide has two columns, the picture beside the rest. BUG-277: the
/// mockup has two at 1024 (the content 868 wide there, a column of 422 beside the picture) and
/// one at 800 (644); 900 gave one column at 1024.
const WIDE: f32 = 760.0;
/// Between the picture and its column.
const GAP: f32 = 24.0;
/// The picture's height in one column.
const NARROW_PICTURE: f32 = 340.0;
/// A number tile's height: its label, number and two lines of parts (de and uk fit).
const TILE: f32 = 112.0;
/// «Your numbers» and a price's line: at most this many ranges before «N more ranges».
const RANGES_SHOWN: usize = 24;
/// «Recent»: at most this many events.
const RECENT: usize = 5;

/// The numbers the tiles show.
struct Numbers {
    mine: Option<u64>,
    for_sale: Option<u64>,
    free: Option<u64>,
    sold: Option<u32>,
    sold_for: Option<Duffs>,
    who: String,
    /// «Given» as read (the architect, 2026-11-22); `None` before.
    given: Option<evolite_api::workshop::Given>,
    now: chrono::DateTime<chrono::Local>,
    prices: Vec<(Duffs, u32)>,
}

/// A History line about this model (W7-6): its sales, and its series (issues, gifts, sales put
/// up, taken off: BUG-307).
fn about_model(act: &evolite_api::Activity, name: &str) -> bool {
    match &act.kind {
        evolite_api::ActivityKind::NftSold { model: Some(m), .. }
        | evolite_api::ActivityKind::NftReceived { model: Some(m), .. }
        | evolite_api::ActivityKind::NftSeries { model: Some(m), .. } => m == name,
        _ => false,
    }
}

/// W7-1: the picture whole in its square (or the width), on `raised`; while it is drawn, when it
/// could not be, or when the model has none, said in words — never «No picture».
fn picture(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    art: &mut crate::art::ArtCache,
    model: &Model,
    rect: egui::Rect,
) {
    let p = a.p;
    crate::art::picture_mark(ui, rect);
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(16), p.raised);
    let (words, retry) = if model.art.is_empty() {
        (tr!("Without a picture").to_owned(), false)
    } else {
        let mut send = |c: Command| cx.send(c);
        match art.paint_or_tell(ui, &model.art, rect, &mut send) {
            crate::art::Drawn::Ready => return,
            crate::art::Drawn::Waiting => (tr!("The picture is loading…").to_owned(), false),
            crate::art::Drawn::Failed(why) => (
                format!(tr!("The picture could not be shown: {}."), failure(&why)),
                true,
            ),
        }
    };
    let inner = rect.shrink(24.0);
    let mut again = false;
    // BUG-277: a child of its own, not `scope_builder`: that moved the page's cursor to the end
    // of these words, inside the picture — in one column the name, the rules and the supply
    // were drawn in the frame, in two the column beside it started 24 px early (BUG-276).
    // Clipped to the picture: its words never leave it.
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    {
        let ui = &mut child;
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.add_space((inner.height() / 2.0 - 48.0).max(0.0));
        ui.add(a.icons.icon("image", p.muted, &p, 32.0));
        ui.add(crate::components::para(
            &words,
            13.0,
            Weight::Regular,
            p.muted,
        ));
        if retry {
            again = crate::components::text_action(ui, a, "sync", tr!("Try again")).clicked();
        }
    }
    if again {
        art.try_again(&model.art);
    }
}

/// Why a picture was not drawn, in a few words (W7-1): the network, a damaged file, the size.
fn failure(why: &str) -> &'static str {
    let lower = why.to_lowercase();
    if lower.is_empty() || lower.contains("no picture") {
        tr!("the network did not answer")
    } else if lower.contains("size") || lower.contains("frames") || lower.contains("limit") {
        tr!("the picture is larger than allowed")
    } else {
        tr!("the picture file is damaged")
    }
}

/// W7-3: the collection's rules as chips, only those it has; not pressed.
fn rule_chips(ui: &mut Ui, a: &mut Assets, collection: Option<&Collection>) {
    let p = a.p;
    let Some(c) = collection else {
        return;
    };
    let mut rules = vec![tr!("transferable"), tr!("for sale")];
    if !c.open {
        rules.push(tr!("only the author issues"));
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for rule in rules {
            let font = crate::theme::sans(12.0, Weight::Medium);
            let w = 24.0 + crate::components::text_width(ui, rule, font.clone());
            let (rect, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(13), p.raised);
            crate::components::paint_text(
                ui,
                rect.center(),
                egui::Align2::CENTER_CENTER,
                rule,
                font,
                p.text,
            );
        }
    });
}

/// W7-4: «supply 100 · numbers no. 1–40», the bar of what is issued and «issued 40 of 100».
fn supply(ui: &mut Ui, a: &mut Assets, supply: u32, issued: Option<u32>) {
    let p = a.p;
    let count = |n: u32| crate::format::count(n as u64);
    let line = match issued {
        // BUG-325: one issued is «number no. 1», not «no. 1–1».
        Some(1) => format!(tr!("supply {} · number no. {}"), count(supply), count(1)),
        Some(n) if n > 0 => format!(
            tr!("supply {} · numbers no. {}–{}"),
            count(supply),
            count(1),
            count(n)
        ),
        _ => format!(tr!("supply {}"), count(supply)),
    };
    ui.add(crate::components::para(
        &line,
        13.0,
        Weight::Medium,
        p.muted,
    ));
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    ui.painter()
        .rect_filled(bar, egui::CornerRadius::same(3), p.raised);
    if let Some(n) = issued {
        let mut done = bar;
        done.set_width(bar.width() * (n as f32 / supply.max(1) as f32).clamp(0.0, 1.0));
        ui.painter()
            .rect_filled(done, egui::CornerRadius::same(3), p.accent);
    }
    let of = issued.map_or("—".to_owned(), count);
    ui.label(txt(
        &format!(tr!("issued {} of {}"), of, count(supply)),
        12.0,
        Weight::Regular,
        p.muted,
    ));
}

/// The four numbers, 2 × 2 (W7): you have, for sale, given, sold — each with its parts.
fn tiles(ui: &mut Ui, a: &mut Assets, n: &Numbers) {
    let p = a.p;
    let shown = |v: Option<u64>| v.map_or("—".to_owned(), crate::format::count);
    let width = ui.available_width();
    let w = ((width - 12.0) / 2.0).floor();
    let mine_parts = match (n.free, n.for_sale) {
        (Some(free), Some(sale)) => Some(unbroken(&format!(
            tr!("not for sale {} · for sale {}"),
            crate::format::count(free),
            crate::format::count(sale)
        ))),
        _ => None,
    };
    let sale_parts = (!n.prices.is_empty()).then(|| {
        n.prices
            .iter()
            .map(|(price, k)| {
                unbroken(&format!(
                    tr!("at {} — {}"),
                    crate::format::amount(*price),
                    crate::format::count(*k as u64)
                ))
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    let sold_parts = n.sold.filter(|s| *s > 0).zip(n.sold_for).map(|(_, sum)| {
        unbroken(&format!(
            tr!("for {} · credits to {}"),
            crate::format::amount(sum),
            crate::i18n::own(&n.who)
        ))
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
        ui.horizontal(|ui| {
            tile(
                ui,
                a,
                w,
                tr!("You have"),
                &shown(n.mine),
                p.text,
                mine_parts,
            );
            tile(
                ui,
                a,
                w,
                tr!("For sale"),
                &shown(n.for_sale),
                p.accent,
                sale_parts,
            );
        });
        ui.horizontal(|ui| {
            let (given, given_parts) = given_tile(n);
            tile(ui, a, w, tr!("Given"), &given, p.text, given_parts);
            tile(
                ui,
                a,
                w,
                tr!("Sold"),
                &shown(n.sold.map(u64::from)),
                p.text,
                sold_parts,
            );
        });
    });
}

/// W9 (the designer, 2026-11-25): a tile's parts, translated, each kept whole — its spaces
/// no-break, so a line breaks only at « · », never in the middle of «for sale 4».
fn unbroken(parts: &str) -> String {
    crate::i18n::t_in(crate::i18n::current(), parts)
        .split(" · ")
        .map(|part| part.replace(' ', "\u{a0}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// «Given N» (or «at least N» past the pages read) and «last: {when}, @name»; «No gifts yet»;
/// «only from this computer» for a collection without transfer history; «—» while not read.
fn given_tile(n: &Numbers) -> (String, Option<String>) {
    let Some(g) = &n.given else {
        return ("—".to_owned(), None);
    };
    let number = match g.count {
        Count::Proved(k) => crate::format::count(k),
        Count::Counted {
            count, complete, ..
        } if !complete => format!(tr!("at least {}"), crate::format::count(count)),
        Count::Counted { count, .. } => crate::format::count(count),
    };
    let mut parts = match &g.last {
        Some((at, to)) => last_gift(*at, to, n.now),
        None => tr!("No gifts yet").to_owned(),
    };
    if g.this_computer_only {
        parts = format!("{parts} · {}", tr!("only from this computer"));
    }
    (number, Some(parts))
}

/// «last: today, 13:04, @name». BUG-324: «today, 13:04» in the interface language before it
/// goes in, kept as one piece — left a place of «last: {}, {}», the line split at its first
/// comma and «today» stayed English («сегодня» in «Recent» beside it).
pub(super) fn last_gift(
    at: evolite_api::Timestamp,
    to: &str,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    let to = if to.starts_with('@') {
        crate::i18n::own(to)
    } else {
        crate::text::whole(to)
    };
    let when = crate::text::when(at, now);
    let when = crate::i18n::t_in(crate::i18n::current(), &when);
    format!(tr!("last: {}, {}"), crate::i18n::own(&when), to)
}

/// BUG-325: the numbers a gift or an issue takes — «no. 2» for one, «no. 2–5» for a run (not
/// «no. 2–2»), and r1 Should: a list where they have gaps — a gift leaves those for sale, so it
/// may take 2, 4, 5, which «no. 2–5» would say with the 3 for sale among them. `None` for none.
pub(super) fn serials_words(give: bool, serials: &[u32]) -> Option<String> {
    let mut sorted = serials.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let (&first, &last) = (sorted.first()?, sorted.last()?);
    let n = |v: u32| crate::format::count(v as u64);
    let run = (last - first) as usize + 1 == sorted.len();
    Some(match (give, sorted.len() == 1, run) {
        (true, true, _) => format!(
            tr!(
                "The free copy with the lowest number goes — no. {}. Those for sale are not touched."
            ),
            n(first)
        ),
        (true, false, true) => format!(
            tr!(
                "The free copies with the lowest numbers go — no. {}–{}. Those for sale are not touched."
            ),
            n(first),
            n(last)
        ),
        (true, false, false) => format!(
            tr!(
                "The free copies with the lowest numbers go — no. {}. Those for sale are not touched."
            ),
            listed(&sorted)
        ),
        (false, true, _) => format!(tr!("Number no. {} is issued."), n(first)),
        (false, false, true) => format!(tr!("Numbers no. {}–{} are issued."), n(first), n(last)),
        (false, false, false) => format!(tr!("Numbers no. {} are issued."), listed(&sorted)),
    })
}

/// `sorted` numbers with gaps as a list: «2, 4, 5» — a run of three or more as one range
/// («2, 4–7»), at most [`RANGES_SHOWN`] parts, then «N more ranges».
fn listed(sorted: &[u32]) -> String {
    let n = |v: u32| crate::format::count(v as u64);
    let mut parts = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let mut j = i;
        while j + 1 < sorted.len() && sorted[j + 1] == sorted[j] + 1 {
            j += 1;
        }
        if j - i >= 2 {
            parts.push(format!("{}–{}", n(sorted[i]), n(sorted[j])));
        } else {
            parts.extend(sorted[i..=j].iter().map(|v| n(*v)));
        }
        i = j + 1;
    }
    let mut line = parts
        .iter()
        .take(RANGES_SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if parts.len() > RANGES_SHOWN {
        let more = (parts.len() - RANGES_SHOWN) as u32;
        line = format!(
            "{line} {}",
            crate::text::plural(more, tr!("more range"), tr!("more ranges"))
        );
    }
    line
}

/// One number tile: its label, the number and up to two lines of its parts; one height.
fn tile(
    ui: &mut Ui,
    a: &mut Assets,
    width: f32,
    label: &str,
    value: &str,
    color: egui::Color32,
    parts: Option<String>,
) {
    let p = a.p;
    egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.line))
        .corner_radius(14)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(width - 32.0);
            ui.set_min_height(TILE - 32.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add(crate::components::para(
                    label,
                    12.0,
                    Weight::Medium,
                    p.muted,
                ));
                ui.label(txt(value, 22.0, Weight::SemiBold, color));
                if let Some(parts) = parts {
                    ui.add(crate::components::para(
                        &parts,
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
            });
        });
}

/// `serials` as ranges, in order: «5–8», «12».
fn ranges(serials: &[u32]) -> Vec<String> {
    let mut sorted = serials.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let n = |v: u32| crate::format::count(v as u64);
    let mut out = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let first = sorted[i];
        let mut last = first;
        while i + 1 < sorted.len() && sorted[i + 1] == last + 1 {
            i += 1;
            last = sorted[i];
        }
        out.push(if first == last {
            n(first)
        } else {
            format!("{}–{}", n(first), n(last))
        });
        i += 1;
    }
    out
}

/// «no. 5–8, 14–40», at most `shown` ranges, then «N more ranges».
fn numbers_line(serials: &[u32], shown: usize) -> String {
    let all = ranges(serials);
    let listed: Vec<String> = all.iter().take(shown).cloned().collect();
    let mut line = format!(tr!("no. {}"), listed.join(", "));
    if all.len() > shown {
        let more = (all.len() - shown) as u32;
        line = format!(
            "{line} {}",
            crate::text::plural(more, tr!("more range"), tr!("more ranges"))
        );
    }
    line
}

/// W7-5: «Your numbers: no. 5–8, 14–40» — no row for each copy (AC-D12).
fn your_numbers(
    ui: &mut Ui,
    a: &mut Assets,
    copies: &[evolite_api::workshop::Instance],
    all: bool,
    mine: Option<u64>,
) {
    let p = a.p;
    if copies.is_empty() || mine == Some(0) {
        return;
    }
    let serials: Vec<u32> = copies.iter().map(|c| c.serial).collect();
    let mut line = numbers_line(&serials, RANGES_SHOWN);
    if !all {
        line.push_str(" …");
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(txt(tr!("Your numbers:"), 13.0, Weight::Regular, p.muted));
        ui.add(
            crate::components::para(&line, 13.0, Weight::Regular, p.text)
                .font(crate::theme::mono(13.0, Weight::Regular)),
        );
    });
}

/// What a series window signs: its records, one write's estimate and the total, the spend, and
/// why it cannot go (`Some("")`: no plan yet).
type Signed = (u32, Option<(Duffs, Duffs)>, Option<Spend>, Option<String>);

/// What a row of «Prices» asked for (W9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PriceAct {
    /// S4 «Change the price» of the copies at this price.
    Change(Duffs),
    /// S5 «Take off {n}»: this many at this price.
    TakeOff(Duffs, u32),
}

/// What the series windows count from: the free copies, how many can still be issued, the
/// copies for sale (by price: S4's numbers, S5's «Which»).
struct Have<'a> {
    free: Option<u64>,
    left_to_issue: Option<u32>,
    for_sale: &'a [evolite_api::workshop::Instance],
    /// All of `for_sale` read: before it, S5's «Which» keeps the price it was opened for.
    for_sale_read: bool,
}

/// The copies for sale by price, cheapest first, with their numbers.
fn by_price(
    for_sale: &[evolite_api::workshop::Instance],
) -> std::collections::BTreeMap<Duffs, Vec<u32>> {
    let mut by: std::collections::BTreeMap<Duffs, Vec<u32>> = Default::default();
    for c in for_sale {
        if let Some(price) = c.price {
            by.entry(price).or_default().push(c.serial);
        }
    }
    by
}

/// «PRICES»: a line for each price — the price and «4 copies · no. 5–8», then W9's «Change the
/// price» and «Take off 4»; «Nothing is for sale yet» when nothing is. While a series runs the
/// actions wait for it (AC-A8).
fn prices(
    ui: &mut Ui,
    a: &mut Assets,
    for_sale: &[evolite_api::workshop::Instance],
    all: bool,
    running: bool,
) -> Option<PriceAct> {
    let p = a.p;
    let mut asked = None;
    crate::components::section_label(ui, &p, tr!("Prices"));
    let by_price = by_price(for_sale);
    let width = ui.available_width();
    crate::components::card(&p, egui::Margin::symmetric(16, 14)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 16));
        ui.spacing_mut().item_spacing.y = 10.0;
        if by_price.is_empty() {
            let words = if all {
                tr!("Nothing is for sale yet")
            } else {
                tr!("Reading…")
            };
            ui.label(txt(words, 13.0, Weight::Regular, p.muted));
            return;
        }
        for (price, serials) in &by_price {
            let n = serials.len() as u32;
            let take_off = format!(tr!("Take off {}"), n);
            let amount = crate::format::amount(*price);
            let copies = crate::text::plural(n, tr!("copy"), tr!("copies"));
            let what = format!("{copies} · {}", numbers_line(serials, RANGES_SHOWN));
            let actions = price_action_width(ui, tr!("Change the price"))
                + 16.0
                + price_action_width(ui, &take_off);
            // W9 S1 (the designer, 2026-11-25): one line, the actions on its right, while it has
            // room for all of it; else the row wraps and they go under.
            let one_line = crate::components::text_width(
                ui,
                &amount,
                crate::theme::mono(13.0, Weight::Medium),
            ) + crate::components::text_width(
                ui,
                &what,
                crate::theme::sans(13.0, Weight::Regular),
            ) + actions
                + 2.0 * 24.0
                <= ui.available_width();
            // The price's actions, right to left in the room they get.
            let mut act = |ui: &mut Ui, a: &mut Assets| {
                ui.spacing_mut().item_spacing.x = 16.0;
                if price_action(ui, a, &take_off, !running).clicked() && !running {
                    asked = Some(PriceAct::TakeOff(*price, n));
                }
                if price_action(ui, a, tr!("Change the price"), !running).clicked() && !running {
                    asked = Some(PriceAct::Change(*price));
                }
            };
            let price_words = |ui: &mut Ui| {
                ui.label(crate::components::fig_txt(
                    &amount,
                    crate::theme::mono(13.0, Weight::Medium),
                    p.accent,
                ));
            };
            if one_line {
                // A plain row: a wrapping one puts the actions under even with room for them.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 24.0;
                    price_words(ui);
                    ui.label(txt(&what, 13.0, Weight::Regular, p.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        act(ui, a)
                    });
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(24.0, 6.0);
                    price_words(ui);
                    ui.add(crate::components::para(
                        &what,
                        13.0,
                        Weight::Regular,
                        p.text,
                    ));
                });
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width().max(actions), 20.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| act(ui, a),
                );
            }
        }
    });
    asked
}

/// The width of a [`price_action`].
fn price_action_width(ui: &Ui, label: &str) -> f32 {
    crate::components::text_width(ui, label, crate::theme::sans(14.0, Weight::SemiBold))
}

/// A text action of «Prices» (W9 S1): 14 SemiBold `text`, a button to a screen reader; `muted`
/// while a series runs, with the reason on hover (AC-A8) — its click is not taken then.
fn price_action(ui: &mut Ui, a: &mut Assets, label: &str, on: bool) -> egui::Response {
    let p = a.p;
    let colour = if on { p.text } else { p.muted };
    let row = ui
        .scope(|ui| {
            // The words are the action: a selectable label would take their clicks.
            ui.style_mut().interaction.selectable_labels = false;
            ui.label(txt(label, 14.0, Weight::SemiBold, colour));
        })
        .response;
    let resp = crate::a11y::row_click(ui, &row, label);
    crate::components::named(&resp, egui::WidgetType::Button, on, label);
    if on {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp.on_hover_text(
            crate::i18n::t(tr!("A series is running — wait for it or stop it")).into_owned(),
        )
    }
}

/// «RECENT» (W7-6): up to five of the model's History lines, newest first, with «All history ›»
/// to History searching for the model; «No events yet» when there are none.
fn recent(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, name: &str, lines: &[evolite_api::Activity]) {
    let p = a.p;
    let mut all = false;
    ui.horizontal(|ui| {
        crate::components::section_label(ui, &p, tr!("Recent"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            all = history_link(ui, a, tr!("All history ›")).clicked();
        });
    });
    if all {
        cx.show_history(name.to_owned());
    }
    if lines.is_empty() {
        ui.label(txt(tr!("No events yet"), 13.0, Weight::Regular, p.muted));
        return;
    }
    let mut sorted: Vec<&evolite_api::Activity> = lines.iter().collect();
    sorted.sort_by_key(|act| std::cmp::Reverse(act.at));
    for act in sorted.into_iter().take(RECENT) {
        let when = crate::text::when(act.at, cx.now);
        ui.horizontal(|ui| {
            let when_w =
                crate::components::text_width(ui, &when, crate::theme::sans(12.0, Weight::Regular));
            let room = (ui.available_width() - when_w - 16.0).max(80.0);
            ui.allocate_ui(vec2(room, 0.0), |ui| {
                ui.add(crate::components::para(
                    &crate::text::activity_title(act),
                    13.0,
                    Weight::Regular,
                    p.text,
                ));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(txt(&when, 12.0, Weight::Regular, p.muted));
            });
        });
    }
}

/// «All history ›»: 13 SemiBold in `accent`, a link (W7-6).
fn history_link(ui: &mut Ui, a: &mut Assets, label: &str) -> egui::Response {
    let p = a.p;
    let row = ui
        .scope(|ui| {
            // The words are the link: a selectable label would take their clicks.
            ui.style_mut().interaction.selectable_labels = false;
            ui.label(txt(label, 13.0, Weight::SemiBold, p.accent));
        })
        .response;
    let resp = crate::a11y::row_click(ui, &row, "all-history");
    crate::components::named(&resp, egui::WidgetType::Link, true, label);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// W7-7: «Contract id», the id whole in mono (wrapping when narrow) and «Copy».
fn contract_id(ui: &mut Ui, a: &mut Assets, contract: &str) {
    let p = a.p;
    let width = ui.available_width();
    crate::components::card(&p, egui::Margin::same(16)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 16));
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            ui.label(txt(tr!("Contract id"), 12.0, Weight::Medium, p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::components::text_action(ui, a, "copy", tr!("Copy")).clicked() {
                    crate::clipboard::copy(ui.ctx(), contract.to_owned());
                }
            });
        });
        ui.add(
            crate::components::para(contract, 12.0, Weight::Regular, p.text)
                .font(crate::theme::mono(12.0, Weight::Regular)),
        );
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
        SeriesKind::Price { .. } => format!(tr!("Put up for sale {} of {}"), s.done, s.total),
        SeriesKind::Reprice { .. } => format!(tr!("Price changed: {} of {}"), s.done, s.total),
        SeriesKind::Unprice { .. } => format!(tr!("Taken off sale {} of {}"), s.done, s.total),
        _ => format!(tr!("Done {} of {}"), s.done, s.total),
    };
    let left = s.total.saturating_sub(s.done + s.refused);
    // W9 S3: every record in — a tick, and what a buyer sees now; nothing to sign again.
    let all_in = !s.running && s.done == s.total && s.total > 0;
    let at = match &s.kind {
        SeriesKind::Price { price, .. } | SeriesKind::Reprice { price, .. } => Some(
            crate::format::amount(Duffs::try_from(price / 1_000).unwrap_or(Duffs::MAX)),
        ),
        _ => None,
    };
    // W9 S3: nothing left to write — «Close» on the right of the title's row.
    let close_here = !s.running && left == 0;
    let width = ui.available_width();
    crate::components::card(&p, egui::Margin::same(16)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 16));
        ui.spacing_mut().item_spacing.y = 8.0;
        // W9 S3 (the designer, 2026-11-25): the title and its line one block, 5 apart; «Close»
        // on its right, in the middle of its height.
        let head = |ui: &mut Ui, a: &mut Assets| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if all_in {
                        ui.add(a.icons.icon("confirmed", p.accent, &p, 16.0));
                    }
                    ui.label(txt(&what, 14.0, Weight::SemiBold, p.text));
                });
                if let Some(at) = at.as_ref().filter(|_| all_in) {
                    ui.add(crate::components::para(
                        &format!(
                            tr!(
                                "All records are on the network. They can be bought at {} per copy."
                            ),
                            at
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
            });
        };
        if close_here {
            // The block at its own height, narrower by the button; the button in the room on its
            // right, in the middle of the block's height.
            let full = ui.available_width();
            let close_w = crate::components::text_width(
                ui,
                tr!("Close"),
                crate::theme::sans(14.0, Weight::SemiBold),
            ) + 40.0;
            let block = ui
                .allocate_ui(vec2((full - close_w - 12.0).max(0.0), 0.0), |ui| head(ui, a))
                .response
                .rect;
            let room = egui::Rect::from_min_max(
                egui::pos2(block.left() + full - close_w, block.top()),
                egui::pos2(block.left() + full, block.bottom()),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(room)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
                        cx.send(Command::CloseSeries { id: s.id });
                    }
                },
            );
        } else {
            head(ui, a);
        }
        if !all_in {
            crate::components::progress_bar(ui, &p, s.done as f32 / s.total.max(1) as f32);
        }
        if s.refused > 0 {
            ui.label(txt(
                &format!(tr!("refused: {}"), s.refused),
                12.0,
                Weight::Regular,
                p.danger,
            ));
        }
        // BUG-258: the words and the button go on to the next line at the medium window.
        if !close_here {
            ui.horizontal_wrapped(|ui| {
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
        }
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

/// W8 «Give», «Issue more», W9 «Put up for sale», «Change the price», «Take off sale», W12b
/// «Continue»: how many (to whom, at what price), the plan, the summary pinned over the password
/// (BUG-249; AC-A6 «N records × ≈ one · ≈ all», the credits now and after), «Sign and …».
fn series_window(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut WorkshopState,
    model: &str,
    have: &Have,
) {
    let Some(form) = state.series.as_mut() else {
        return;
    };
    // Signed and answered well: the plate takes over.
    if matches!(form.flow.result(), Some(Ok(()))) {
        state.series = None;
        state.counts.clear();
        state.counts_asked.clear();
        state.instances.clear();
        return;
    }
    let now = ui.input(|i| i.time);
    // BUG-266: «wait for the sync» is not the last word: asked again a moment later.
    let answer = super::not_synced(form.planned());
    let moved = super::sync_moved(&mut form.sync_seen, cx.snap.sync.stage);
    if super::retry_not_synced(ui, now, answer, moved, &mut form.not_synced) {
        form.asked = None;
        form.plan = None;
    }
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
    let who = super::super::tokens::shown_name(&wallet);
    let name = crate::i18n::own(model);
    let by_price = by_price(have.for_sale);
    let on_sale: u32 = by_price.values().map(|s| s.len() as u32).sum();
    // S5 «Which»: the price chosen and how many are at it (every price: all on sale).
    let which_count = match form.which {
        Some(price) => by_price.get(&price).map_or(0, |s| s.len() as u32),
        None => on_sale,
    };
    let title = match form.action {
        Action::Give => format!(tr!("Give “{}”"), name),
        Action::Issue => format!(tr!("Issue more “{}”"), name),
        Action::Continue => format!(tr!("Continue “{}”"), name),
        Action::Sell => tr!("Put up for sale").to_owned(),
        Action::Reprice { .. } => tr!("Change the price").to_owned(),
        Action::Unprice => tr!("Take off sale").to_owned(),
    };
    // W9: what the window is about, under its title.
    let under = match form.action {
        // W9 r1: the free ones not read yet — not «free 0».
        Action::Sell => Some(match have.free {
            Some(free) => format!(tr!("“{}” · free {}"), name, crate::format::count(free)),
            None => format!("“{name}”"),
        }),
        Action::Reprice { from } => {
            let group = by_price.get(&from).cloned().unwrap_or_default();
            Some(format!(
                tr!("“{}” · {} at {} · {}"),
                name,
                crate::text::plural(group.len() as u32, tr!("copy"), tr!("copies")),
                crate::format::amount(from),
                numbers_line(&group, RANGES_SHOWN)
            ))
        }
        Action::Unprice => Some(format!(
            tr!("“{}” · for sale {}"),
            name,
            crate::format::count(u64::from(on_sale))
        )),
        _ => None,
    };
    // One window per kind of series (BUG-249 r3: an id the guard reads): «Continue» opens the
    // one of the series it resumes.
    let gives = match form.action {
        Action::Give => true,
        Action::Continue => form.resume_gives,
        _ => false,
    };
    // What is signed: the plan's records, one write's estimate and the total, or a stopped
    // series' rest.
    let planned = form.planned().cloned();
    let (records, fees, op, why): Signed = match (form.action, &planned, form.resume) {
        (Action::Continue, _, Some((id, left, per_write))) => {
            let fee = per_write.saturating_mul(left as Duffs);
            (
                left,
                Some((per_write, fee)),
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
            (
                plan.serials.len() as u32,
                Some((plan.per_write, plan.total)),
                op,
                why,
            )
        }
        (_, Some(Err(e)), _) => (0, None, None, Some(crate::text::error_text(e))),
        // BUG-259: the plan of what is typed not here yet — the button says the number typed,
        // not «0» until the field loses focus.
        _ => (form.typed_count(), None, None, Some(String::new())),
    };
    if let Some(op) = &op {
        // FEAT-0004 FR-7: the large spend's tick from the start, in the summary.
        form.flow.password.watch(Some(op));
    }
    let action = form.action;
    let serials: Vec<u32> = planned
        .as_ref()
        .and_then(|p| p.as_ref().ok())
        .map(|p| p.serials.clone())
        .unwrap_or_default();
    let mut changed = false;
    let body = |ui: &mut Ui, a: &mut Assets, part: Part| {
        let p = a.p;
        let value = crate::theme::mono(13.0, Weight::Medium);
        // W9 (the designer, 2026-11-25): words and amounts of the form in Inter SemiBold, as
        // «If all are bought»; mono only for the fee and the credits.
        let words = crate::theme::sans(13.0, Weight::SemiBold);
        if part == Part::Summary {
            ui.spacing_mut().item_spacing.y = 10.0;
            if let Some((per_write, total)) = fees {
                // W9 (the designer, 2026-11-25): «3 records × ≈ one» on the left, «≈ all» on the
                // right. The count's words translated first: the row's label is not a key.
                let records_words = crate::text::plural(records, tr!("record"), tr!("records"));
                let records_words = crate::i18n::t_in(crate::i18n::current(), &records_words);
                kv_row_fit(
                    ui,
                    &p,
                    &format!("{records_words} × ≈ {}", crate::format::amount(per_write)),
                    // BUG-259: the margin's estimate — «at most».
                    &super::at_most(Some(total), false),
                    value.clone(),
                );
                if form.action == Action::Unprice {
                    ui.add(crate::components::para(
                        tr!(
                            "A price can only be removed by passing the item to yourself, so the fee is the same as for a gift."
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
                // FEAT-0057 П3 (R-3.1): the one line «Credits: now › after» for the series.
                crate::components::credits_after(
                    ui,
                    a,
                    &who,
                    crate::components::Credits::of(&wallet),
                    crate::components::Charge::AtMost(total),
                );
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
                Action::Sell => format!(tr!("Sign and put up {}"), records),
                Action::Reprice { .. } => format!(tr!("Sign and change {}"), records),
                Action::Unprice => format!(tr!("Sign and take off {}"), records),
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
        let before = (
            form.how_many.clone(),
            form.to.clone(),
            form.price.clone(),
            form.which,
        );
        match form.action {
            Action::Continue => {
                ui.label(txt(
                    &format!(tr!("Left to write: {}"), records),
                    13.0,
                    Weight::Regular,
                    p.text,
                ));
            }
            Action::Give | Action::Issue => {
                let of = match form.action {
                    Action::Give => have.free,
                    _ => have.left_to_issue.map(u64::from),
                };
                how_many(ui, a, form, of, false);
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
                if let Some(words) = serials_words(form.action == Action::Give, &serials) {
                    ui.label(txt(&words, 12.0, Weight::Regular, p.muted));
                }
            }
            Action::Sell => {
                how_many(ui, a, form, have.free, true);
                price_field(ui, &p, form, tr!("Price per copy"));
                let count: u32 = form.how_many.trim().parse().unwrap_or(0);
                if let Some(price) = form.typed_price().filter(|_| count > 0) {
                    kv_row_fit(
                        ui,
                        &p,
                        tr!("You put up"),
                        // Batch 74: its own key — a bare «{} at {}» took «1 for sale at …» of
                        // the Collection's line when it was translated piece by piece.
                        &crate::text::plural_in(
                            count,
                            tr!("{} copy at {}"),
                            tr!("{} copies at {}"),
                        )
                        .replacen("{}", &crate::format::amount(price), 1),
                        words.clone(),
                    );
                    if_all_bought(ui, &p, price.saturating_mul(count as Duffs));
                }
                if !serials.is_empty() {
                    ui.label(txt(
                        &format!(
                            tr!("evolite takes the free copies with the lowest numbers — here {}."),
                            numbers_line(&serials, RANGES_SHOWN)
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
                notice(
                    ui,
                    a,
                    "info",
                    p.muted,
                    tr!(
                        "Anyone can buy them — at once, without your confirmation (a direct purchase). A gift or a transfer takes the price off. After a sale the copy is the new owner's, and they set its price."
                    ),
                );
            }
            Action::Reprice { from } => {
                let group = by_price.get(&from).map_or(0, |s| s.len() as u32);
                price_field(ui, &p, form, tr!("New price per copy"));
                if form.same_price() {
                    ui.label(txt(tr!("The same price"), 12.0, Weight::Regular, p.muted));
                }
                let per_copy = |d: Duffs| format!(tr!("{} per copy"), crate::format::amount(d));
                crate::components::kv_row_in(
                    ui,
                    &p,
                    tr!("Now"),
                    &per_copy(from),
                    words.clone(),
                    p.muted,
                );
                if let Some(price) = form.typed_price() {
                    kv_row_fit(ui, &p, tr!("Will be"), &per_copy(price), words.clone());
                    if_all_bought(ui, &p, price.saturating_mul(group as Duffs));
                }
                ui.label(txt(
                    &format!(
                        tr!(
                            "The price changes on the same {} copies; their numbers stay the same. Until the record is on the network, a buyer sees the old price."
                        ),
                        group
                    ),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
            Action::Unprice => {
                field_label(ui, &p, tr!("Which"));
                let mut choices: Vec<(Option<Duffs>, String)> = by_price
                    .iter()
                    .map(|(price, s)| {
                        (
                            Some(*price),
                            format!(tr!("at {} · {}"), crate::format::amount(*price), s.len()),
                        )
                    })
                    .collect();
                choices.push((None, format!(tr!("all {}"), on_sale)));
                let labels: Vec<&str> = choices.iter().map(|(_, l)| l.as_str()).collect();
                let mut at = choices
                    .iter()
                    .position(|(price, _)| *price == form.which)
                    .unwrap_or(choices.len() - 1);
                crate::components::segmented(ui, &p, &labels, &mut at, true);
                // Not before the copies for sale are read: the price «Take off {n}» opened with,
                // and its count, are not lost to «all 0» of a list still coming.
                if have.for_sale_read && choices[at].0 != form.which {
                    form.which = choices[at].0;
                    // A new choice takes all of its copies, as its row of «Prices» does.
                    let n = match form.which {
                        Some(price) => by_price.get(&price).map_or(0, |s| s.len()),
                        None => on_sale as usize,
                    };
                    form.how_many = n.to_string();
                }
                how_many(ui, a, form, Some(u64::from(which_count)), false);
                ui.label(txt(
                    tr!(
                        "Copies at this price are taken off, from the highest numbers. They stay yours, the price is taken off them."
                    ),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
        }
        if (
            form.how_many.clone(),
            form.to.clone(),
            form.price.clone(),
            form.which,
        ) != before
        {
            changed = true;
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
    let closed = match (action, under.as_deref()) {
        (Action::Sell, Some(under)) => {
            dialog_with_summary_under(ui, a, "workshop-sell", "send", (&title, under), 480.0, body)
        }
        (Action::Reprice { .. }, Some(under)) => dialog_with_summary_under(
            ui,
            a,
            "workshop-reprice",
            "edit",
            (&title, under),
            480.0,
            body,
        ),
        (Action::Unprice, Some(under)) => dialog_with_summary_under(
            ui,
            a,
            "workshop-unprice",
            "send",
            (&title, under),
            480.0,
            body,
        ),
        _ if gives => dialog_with_summary(ui, a, "workshop-give", "nft", &title, 480.0, body),
        _ => dialog_with_summary(ui, a, "workshop-issue", "nft", &title, 480.0, body),
    };
    if changed {
        form.changed_at = now;
    }
    if closed && !form.flow.waiting() {
        form.forget();
        state.series = None;
    }
}

/// «How many», «of {n}», its field; W9 S2 with «All {n}» at the field's end.
fn how_many(ui: &mut Ui, a: &mut Assets, form: &mut SeriesForm, of: Option<u64>, all: bool) {
    let p = a.p;
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
    let mut take_all = false;
    text_field(
        ui,
        &p,
        "workshop-series-count",
        &mut form.how_many,
        "1",
        FieldKind::Text,
        |ui| {
            if let Some(of) = of.filter(|n| all && *n > 0) {
                take_all = crate::components::mini_button(
                    ui,
                    &p,
                    &format!(tr!("All {}"), crate::format::count(of)),
                )
                .clicked();
            }
        },
    );
    if take_all && let Some(of) = of {
        form.how_many = of.to_string();
    }
    form.how_many.retain(|c| c.is_ascii_digit());
}

/// W9: a copy's price in the unit of the Settings, the unit at the field's end.
fn price_field(ui: &mut Ui, p: &crate::theme::Palette, form: &mut SeriesForm, label: &str) {
    field_label(ui, p, label);
    text_field(
        ui,
        p,
        "workshop-series-price",
        &mut form.price,
        "0",
        FieldKind::Amount,
        |ui| {
            ui.label(txt(
                crate::format::unit_name(),
                13.0,
                Weight::Regular,
                p.muted,
            ));
        },
    );
}

/// «If all are bought · {} in credits»: SemiBold `text` — the accent is for «proved» (W9 S2).
fn if_all_bought(ui: &mut Ui, p: &crate::theme::Palette, sum: Duffs) {
    kv_row_fit(
        ui,
        p,
        tr!("If all are bought"),
        &format!(tr!("{} in credits"), crate::format::amount(sum)),
        crate::theme::sans(13.0, Weight::SemiBold),
    );
}
