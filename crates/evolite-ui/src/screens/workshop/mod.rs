//! FEAT-0038 «Workshop»: the main page (W1), a new collection (W2, its review W3 and steps) and
//! a collection's page (W4). Shown only while «Show the Workshop» is on (W0, AC-A1).
//!
//! Everything here is on behalf of the identity shown and paid with its credits; the reads are
//! the core's proved Workshop reading ([`Command::ReadWorkshop`]), asked again when the identity
//! changes, and the registration goes through the module V1 commands ([`Command::CheckContract`],
//! [`evolite_api::Spend::RegisterContract`]).

mod collection_page;
mod model_page;
mod new_collection;
pub mod new_model;

use std::collections::HashMap;

use crate::components::{Assets, ButtonKind, button, button_enabled, page_header, section_label};
use crate::context::Ctx;
use crate::screens::txt;
use crate::theme::Weight;
use egui::{Align, Layout, Sense, Ui, vec2};
use evolite_api::workshop::{Collection, ModelCounts, WorkshopAsk, WorkshopRead};
use evolite_api::{Command, CoreError};

/// BUG-266: the first pause after a «wait for the sync» answer: not a failure, the sync may end
/// a moment later; then the shared rule's growing pause (`evolite_api::retry`)…
const RETRY_NOT_SYNCED: f64 = 3.0;

/// …never longer than this (W7 r2): «not synced» is no failure — a longer pause outlived the
/// sync by minutes, BUG-266 again.
const RETRY_NOT_SYNCED_MOST: f64 = 30.0;

/// W7 r2: whether the sync moved to another stage since this form last looked (`seen`): a
/// «not synced» answer is then asked again at once.
pub(crate) fn sync_moved(
    seen: &mut Option<evolite_api::SyncStage>,
    now: evolite_api::SyncStage,
) -> bool {
    seen.replace(now).is_some_and(|before| before != now)
}

/// The rule of «not synced»: 3 s first, growing, at most 30 s.
fn not_synced_rule() -> evolite_api::retry::Retry {
    evolite_api::retry::Retry::with_first(RETRY_NOT_SYNCED).capped(RETRY_NOT_SYNCED_MOST)
}

/// The answer a form has for what it shows now, as BUG-266 takes it: `None` — none yet (asked,
/// or about to be); `Some(true)` — «not synced»; `Some(false)` — another answer.
pub(crate) fn not_synced<T>(answer: Option<&Result<T, CoreError>>) -> Option<bool> {
    answer.map(|a| matches!(a, Err(CoreError::NotSynced)))
}

/// BUG-266 on the shared rule (TEAM.md §8d-5): a form's check answered «not synced» is asked
/// again — after 3 s, then 6, 12… at most 30 s, and at once when the sync moves to another
/// stage (W7 r2) — the sync or Platform may be ready a moment later, and the form asked only
/// when it changed, so the error stayed after the sync. Whether to drop the answer and ask anew.
pub(super) fn retry_not_synced(
    ui: &Ui,
    now: f64,
    answer: Option<bool>,
    sync_moved: bool,
    retry: &mut evolite_api::retry::Retry,
) -> bool {
    let due = not_synced_retry_due(now, answer, sync_moved, retry);
    if retry.is_failed() {
        let wait = (retry.next_at - now).max(0.1);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(wait));
    }
    due
}

/// [`retry_not_synced`] without the frame it asks for (a screen drawn again anyway).
pub(crate) fn not_synced_retry_due(
    now: f64,
    answer: Option<bool>,
    sync_moved: bool,
    retry: &mut evolite_api::retry::Retry,
) -> bool {
    match answer {
        // Answered otherwise: the count starts again.
        Some(false) => {
            *retry = not_synced_rule();
            false
        }
        // Asked, not answered yet.
        None => false,
        Some(true) => {
            if retry.first == 0.0 {
                *retry = not_synced_rule();
            }
            // This answer is to the ask that went: a failure of it, once.
            if retry.asked == 0 || retry.asking {
                retry.asked = retry.asked.max(1);
                retry.asking = true;
                retry.failed(now);
            }
            // The sync went on: its answer may be another now.
            if sync_moved {
                retry.again(now);
            }
            retry.ask(now)
        }
    }
}

#[cfg(feature = "dev-tools")]
pub use model_page::Action as SeriesAction;
pub use model_page::SeriesForm;
pub use new_collection::NewCollection;
pub use new_model::ModelForm;

/// Which page of the Workshop is shown.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Page {
    /// W1.
    #[default]
    Home,
    /// W2 (W3 over it, then the steps).
    New,
    /// W4: the collection of this contract (base58).
    Collection(String),
    /// W5 (its review, then W6): a new model in the collection of this contract.
    NewModel(String),
    /// W7: a model (its id) of the collection of this contract, its numbers and series.
    Model { contract: String, model: String },
}

/// The Workshop's screen state: the page, and what the core read for the identity shown.
#[derive(Default)]
pub struct WorkshopState {
    pub page: Page,
    /// The identity the reads below are for; another one shown starts them again.
    identity: Option<String>,
    /// One's collections: `None` not read yet; asked once per identity and after a registration.
    collections: Option<Result<Vec<Collection>, CoreError>>,
    collections_asked: bool,
    /// A model's numbers (W4), by (contract, model).
    counts: HashMap<(String, String), ModelCounts>,
    counts_asked: HashMap<(String, String), Retry>,
    /// W7: one's copies of a model, every page («Your numbers») or those for sale («Prices»),
    /// by (contract, model, for sale).
    instances: HashMap<(String, String, bool), InstancesRead>,
    /// W7 «Given», by (contract, model); asked once a visit.
    given: HashMap<(String, String), evolite_api::workshop::Given>,
    pub(crate) given_asked: HashMap<(String, String), Retry>,
    /// BUG-314: the identity's sales (DocumentHistory's `purchase`s), asked once a visit:
    /// «Sold» and the sales' History lines are the core's reading of them. A model's counts are
    /// asked once they answered, and again after a first good answer.
    sales_asked: Retry,
    /// The sales answered (well or not): the counts may go.
    sales_answered: bool,
    /// The sales were read well.
    sales_read: bool,
    /// BUG-314 r1: when they were last read well (the frame's time): a visit to a model page
    /// reads them again once [`SALES_AGAIN`] passed since.
    sales_at: Option<f64>,
    /// The pass a model page was last drawn in: one drawn now, not in the pass before, is a
    /// visit.
    model_seen_pass: Option<u64>,
    /// The last frame's time: an answer that failed is timed by it (the reads' pauses).
    now: f64,
    /// W7 r1: «Retry» pressed: the failed reads go again on the next frame.
    retry_reads: bool,
    pub new: NewCollection,
    /// W5: the new model's form.
    pub model: ModelForm,
    /// W8 «Give», «Issue more», W12b «Continue»: the series window, when open.
    pub series: Option<SeriesForm>,
    /// W4: «About the collection» open.
    about: bool,
    /// `--screen workshop-*-actions` (W9 frames at 800): the model page kept scrolled to its
    /// actions, «Prices» under them.
    #[cfg(feature = "dev-tools")]
    pub scroll_to_actions: bool,
}

/// BUG-314 r1 (the coordinator, 2026-11-25): the sales are read again on a visit to a model
/// page at most this often (seconds).
pub(super) const SALES_AGAIN: f64 = 60.0;

impl WorkshopState {
    /// [`evolite_api::Event::Workshop`]: kept when it is for what was asked.
    pub fn on_read(&mut self, ask: &WorkshopAsk, result: &Result<WorkshopRead, CoreError>) {
        match (ask, result) {
            (WorkshopAsk::Collections, Ok(WorkshopRead::Collections(all))) => {
                self.collections = Some(Ok(all.clone()));
            }
            (WorkshopAsk::Collections, Err(e)) => self.collections = Some(Err(e.clone())),
            (WorkshopAsk::Counts { .. }, Ok(WorkshopRead::Counts(counts))) => {
                let key = (counts.contract.clone(), counts.model.clone());
                if let Some(r) = self.counts_asked.get_mut(&key) {
                    r.answered();
                }
                self.counts.insert(key, counts.clone());
            }
            // W7 r1 Must: a failed count says so; asked again on «Retry» or after
            // [`RETRY_READ`], never every frame.
            (WorkshopAsk::Counts { contract, model }, Err(_)) => {
                if let Some(r) = self
                    .counts_asked
                    .get_mut(&(contract.clone(), model.clone()))
                {
                    r.failed(self.now);
                }
            }
            (
                WorkshopAsk::Instances {
                    contract,
                    model,
                    for_sale,
                    ..
                },
                result,
            ) => {
                let read = self
                    .instances
                    .entry((contract.clone(), model.clone(), *for_sale))
                    .or_default();
                match result {
                    Ok(WorkshopRead::Instances(page)) => {
                        read.list.extend(page.instances.iter().cloned());
                        read.next = page.next.clone();
                        read.done = page.next.is_none();
                        // The next page is a new read.
                        read.retry = Retry::default();
                    }
                    _ => {
                        read.retry.failed(self.now);
                        read.asks -= 1;
                    }
                }
            }
            (WorkshopAsk::Given { .. }, Ok(WorkshopRead::Given(given))) => {
                let key = (given.contract.clone(), given.model.clone());
                if let Some(r) = self.given_asked.get_mut(&key) {
                    r.answered();
                }
                self.given.insert(key, given.clone());
            }
            // W7 r1 Must: as a count.
            (WorkshopAsk::Given { contract, model }, Err(_)) => {
                if let Some(r) = self.given_asked.get_mut(&(contract.clone(), model.clone())) {
                    r.failed(self.now);
                }
            }
            // BUG-314: the counts read before the sales (the collection's page, or after a failed
            // read of them) are asked again: «Sold» is known now.
            (WorkshopAsk::Sales, result) if result.is_ok() => {
                self.sales_asked.answered();
                if self.sales_at.is_none() {
                    self.counts.clear();
                    self.counts_asked.clear();
                } else if let Ok(WorkshopRead::Sales(new)) = result {
                    // BUG-314 r1: read again on a visit, the core from its cursor — only the
                    // sales since; the models they sold are counted again.
                    self.recount_sold(new);
                }
                self.sales_answered = true;
                self.sales_read = true;
                self.sales_at = Some(self.now);
            }
            (WorkshopAsk::Sales, Err(_)) => {
                self.sales_asked.failed(self.now);
                self.sales_answered = true;
            }
            _ => {}
        }
    }

    /// BUG-314 in tests: the model's «Sold» as its counts were read — `None`: no counts yet,
    /// `Some(None)`: read with «Sold» unknown.
    #[cfg(all(test, demo_core, feature = "dev-tools"))]
    pub(crate) fn sold_read(&self, contract: &str, model: &str) -> Option<Option<u32>> {
        self.counts
            .get(&(contract.to_owned(), model.to_owned()))
            .map(|c| c.sold)
    }

    /// BUG-314 r1: the counts of the models `new` sold are read again; a sale whose model is
    /// gone may have been any one's: all are.
    fn recount_sold(&mut self, new: &[evolite_api::workshop::Sale]) {
        if new.iter().any(|s| s.model_id.is_none()) {
            self.counts.clear();
            self.counts_asked.clear();
            return;
        }
        for sale in new {
            let key = (
                sale.contract.clone(),
                sale.model_id.clone().unwrap_or_default(),
            );
            self.counts.remove(&key);
            self.counts_asked.remove(&key);
        }
    }

    /// BUG-314 r1: a model page drawn in pass `pass` at `now`. Entered (not drawn in the pass
    /// before) once [`SALES_AGAIN`] passed since the sales were read, they are asked again —
    /// the core reads from its cursor, so only what came since.
    pub(super) fn on_model_page(&mut self, pass: u64, now: f64) {
        let entered = self.model_seen_pass.is_none_or(|p| p + 1 < pass);
        self.model_seen_pass = Some(pass);
        if entered && self.sales_read && self.sales_at.is_some_and(|t| now - t >= SALES_AGAIN) {
            self.sales_read = false;
            self.sales_asked = Retry::default();
        }
    }

    /// W9 r1 in tests: the models' counts forgotten, as before their first read — asked again
    /// by the next frame, drawn in it without them.
    #[cfg(all(test, demo_core, feature = "dev-tools"))]
    pub(crate) fn forget_counts(&mut self) {
        self.counts.clear();
        self.counts_asked.clear();
    }

    /// BUG-314 r1 in tests: when the sales were last read well.
    #[cfg(all(test, demo_core, feature = "dev-tools"))]
    pub(crate) fn sales_read_at(&self) -> Option<f64> {
        self.sales_at
    }

    /// BUG-314: the sales are asked again (another identity).
    fn forget_sales(&mut self) {
        self.sales_asked = Retry::default();
        self.sales_answered = false;
        self.sales_read = false;
        self.sales_at = None;
    }

    /// BUG-314: the identity's sales asked when they are not read (and their pause is over);
    /// whether the model's counts may be asked now — after the sales answered.
    pub(super) fn ask_sales(&mut self, cx: &mut Ctx, now: f64, again: bool) -> bool {
        if !self.sales_read {
            if again {
                self.sales_asked.again(now);
            }
            if self.sales_asked.ask(now) {
                cx.send(Command::ReadWorkshop {
                    ask: WorkshopAsk::Sales,
                });
            }
        }
        self.sales_answered
    }

    /// The collections to be read again (a registration, «Try again»).
    fn read_again(&mut self) {
        self.collections = None;
        self.collections_asked = false;
    }

    /// The identity shown changed (or the wallet): its reads start again, the form is kept
    /// only while nothing was signed.
    fn follow(&mut self, identity: Option<&str>) {
        if self.identity.as_deref() == identity {
            return;
        }
        // The first one seen is not a change: the page and the form stay.
        let first = self.identity.is_none();
        self.identity = identity.map(str::to_owned);
        if first {
            return;
        }
        self.read_again();
        self.counts.clear();
        self.counts_asked.clear();
        self.instances.clear();
        self.given.clear();
        self.given_asked.clear();
        self.forget_sales();
        self.about = false;
        if !self.new.signed() {
            self.new.forget();
        }
        if !self.model.signed() {
            self.model.forget();
        }
        if let Some(form) = &mut self.series {
            form.forget();
        }
        self.series = None;
        if matches!(
            self.page,
            Page::Collection(_) | Page::NewModel(_) | Page::Model { .. }
        ) {
            self.page = Page::Home;
        }
    }

    /// Lock, a wallet switch: everything read and typed goes.
    pub fn forget_wallet(&mut self) {
        self.new.forget();
        self.model.forget();
        if let Some(form) = &mut self.series {
            form.forget();
        }
        *self = Self::default();
    }
}

impl crate::screen_state::ScreenState for WorkshopState {
    #[cfg(all(test, demo_core, feature = "dev-tools"))]
    fn flows(&mut self) -> Vec<&mut dyn crate::components::spend_flow::AnyFlow> {
        let mut flows: Vec<&mut dyn crate::components::spend_flow::AnyFlow> =
            vec![&mut self.new.flow, &mut self.model.flow];
        if let Some(form) = &mut self.series {
            flows.push(&mut form.flow);
        }
        flows
    }

    /// Lock, a wallet or an identity switch: the form, its password and what was read go (the
    /// core still finishes a registration signed, and the list shows it when read again).
    fn forget(&mut self, _why: crate::screen_state::Why) {
        self.forget_wallet();
    }
}

/// The Workshop, whichever page is shown. `art`: the NFT pictures' cache, shared with the
/// Collection.
pub fn workshop(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut WorkshopState,
    art: &mut crate::art::ArtCache,
) {
    let wallet = cx.wallet().clone();
    state.follow(wallet.identity.as_ref().map(|i| i.id.as_str()));
    if wallet.identity.is_some() && !state.collections_asked {
        state.collections_asked = true;
        cx.send(Command::ReadWorkshop {
            ask: WorkshopAsk::Collections,
        });
    }
    // BUG-277: the Workshop's pages scroll — on none of them could the end of a long page be
    // reached («Prices» and «Recent» under the window at 1024 and 800). A scroll of its own a
    // page: each opens at its top.
    let page = state.page.clone();
    egui::ScrollArea::vertical()
        .id_salt(("workshop-page", format!("{page:?}")))
        .auto_shrink(false)
        .show(ui, |ui| match page {
            Page::Home => home(ui, a, cx, state),
            Page::New => new_collection::page(ui, a, cx, state),
            Page::Collection(contract) => collection_page::page(ui, a, cx, state, art, &contract),
            Page::NewModel(_) => new_model::page(ui, a, cx, state, art),
            Page::Model { contract, model } => {
                model_page::page(ui, a, cx, state, art, &contract, &model)
            }
        });
}

/// «Credits @satoshi: 1.2000 DASH», or «—» while they are not read.
fn credits_line(wallet: &evolite_api::Wallet) -> String {
    let amount = if wallet.balances.credits_read {
        crate::format::amount(wallet.balances.credits)
    } else {
        "—".into()
    };
    format!(
        tr!("Credits {name}: {amount}"),
        name = crate::i18n::own(&super::tokens::shown_name(wallet)),
        amount = amount
    )
}

/// W1: what can be made, the identity and its credits, one's collections.
fn home(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut WorkshopState) {
    let p = a.p;
    let wallet = cx.wallet().clone();
    ui.spacing_mut().item_spacing.y = 16.0;
    page_header(ui, a, tr!("Workshop"), None, |_, _| {});
    ui.allocate_ui(vec2(ui.available_width().min(720.0), 0.0), |ui| {
        super::tokens::para(
            ui,
            tr!(
                "Issue your own NFTs and tokens. Everything made here is written to Dash Platform on behalf of your identity and paid with its credits."
            ),
            14.0,
            p.muted,
            None,
        );
    });
    // AC-A2, BUG-262: without an identity, only why and the way to one — the empty state of
    // every screen that needs one.
    if crate::screens::no_identity::empty_state(
        ui,
        a,
        cx,
        crate::screens::no_identity::Needs::Workshop,
    ) {
        return;
    }
    identity_row(ui, a, cx, &wallet);
    section_label(ui, &p, tr!("Create"));
    let width = ui.available_width();
    if create_card(
        ui,
        a,
        width,
        "nft",
        tr!("NFT collection"),
        tr!(
            "A contract by the NFT evolite v1 standard: models with a picture and numbered copies that can be given."
        ),
        tr!("Create NFT"),
        no_sign_in_key(&wallet),
    ) {
        state.new.open();
        state.page = Page::New;
    }
    // FEAT-0019 stage 3 is not built yet: its card stays, its button waits for it.
    create_card(
        ui,
        a,
        width,
        "token",
        tr!("Token"),
        tr!(
            "Your own Dash Platform token: its name, how much to issue and who may issue more, burn, freeze. Three ready rule sets or your own."
        ),
        tr!("Create a token"),
        Some(tr!("Creating tokens comes in the next version")),
    );
    collections(ui, a, cx, state);
}

/// The identity chips (when there are several) and the shown one's credits on the right.
fn identity_row(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, wallet: &evolite_api::Wallet) {
    // BUG-265: the one picker, wrapped within the page, so the buttons below stay in reach.
    let credits = credits_line(wallet);
    let picked =
        crate::components::identity_picker(ui, a, wallet, tr!("Identity"), None, Some(&credits));
    if let Some(id) = picked.chosen {
        cx.send(Command::ChooseIdentity { id });
    }
}

/// BUG-298: the Workshop's writes sign with the identity's HIGH key (the instances') —
/// without one, said in place of the button, not as an error after the password.
pub(crate) fn no_sign_in_key(wallet: &evolite_api::Wallet) -> Option<&'static str> {
    let id = wallet.identity.as_ref()?;
    (!evolite_api::key_plan::has(&id.keys, evolite_api::key_plan::KeyRequest::SignIn)).then_some(
        tr!("The identity has no sign-in key (HIGH): add one in Keys."),
    )
}

/// A «Create» card: its icon, name, what it is and its button on the right (W1). `later`: the
/// button is inactive, with this said under it. Whether the button was pressed.
#[allow(clippy::too_many_arguments)]
fn create_card(
    ui: &mut Ui,
    a: &mut Assets,
    width: f32,
    icon: &str,
    title: &str,
    about: &str,
    action: &str,
    later: Option<&str>,
) -> bool {
    let p = a.p;
    let mut pressed = false;
    crate::components::card(&p, egui::Margin::same(20)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 20));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let (tile, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            ui.painter()
                .rect_filled(tile, egui::CornerRadius::same(10), p.raised);
            a.icons.icon(icon, p.text, &p, 22.0).paint_at(
                ui,
                egui::Rect::from_center_size(tile.center(), vec2(22.0, 22.0)),
            );
            let button_w = crate::components::button_width(ui, action);
            let text_w = (ui.available_width() - button_w - 16.0).max(120.0);
            ui.vertical(|ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(txt(title, 16.0, Weight::SemiBold, p.text));
                super::tokens::para(ui, about, 13.0, p.muted, Some(text_w));
                if let Some(later) = later {
                    ui.label(txt(later, 12.0, Weight::Regular, p.muted));
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let kind = ButtonKind::Secondary;
                pressed = button_enabled(ui, a, action, kind, None, later.is_none()).clicked();
            });
        });
    });
    pressed
}

/// «MY COLLECTIONS»: one row per collection of the shown identity, opening its page (W4).
fn collections(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut WorkshopState) {
    let p = a.p;
    match &state.collections {
        None => {
            section_label(ui, &p, tr!("My collections"));
            ui.label(txt(
                tr!("Reading your collections…"),
                13.0,
                Weight::Regular,
                p.muted,
            ));
        }
        Some(Err(e)) => {
            section_label(ui, &p, tr!("My collections"));
            crate::components::error_notice(
                ui,
                a,
                &crate::text::error_text(e),
                crate::text::error_details(e).as_deref(),
            );
            if button(ui, a, tr!("Try again"), ButtonKind::Secondary, None).clicked() {
                state.read_again();
            }
        }
        Some(Ok(all)) => {
            let all = all.clone();
            let search = cx.wallet().own_collections;
            // Nothing found and nothing to say: no section.
            if all.is_empty() && search == evolite_api::workshop::OwnSearch::Done {
                return;
            }
            section_label(
                ui,
                &p,
                &if all.is_empty() {
                    crate::i18n::t(tr!("My collections")).into_owned()
                } else {
                    format!(
                        "{} · {}",
                        crate::i18n::t(tr!("My collections")),
                        crate::format::count(all.len() as u64)
                    )
                },
            );
            for c in &all {
                if collection_row(ui, a, c).clicked() {
                    state.page = Page::Collection(c.contract.clone());
                    state.about = false;
                }
            }
            own_search(ui, a, state, search);
        }
    }
}

/// BUG-271 r1 (TEAM.md §8d-2): while the search for one's own collections on the network is not
/// done, the list says so — more may come; a failed search says so with «Retry» (the core tries
/// it again by itself after a pause).
fn own_search(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut WorkshopState,
    search: evolite_api::workshop::OwnSearch,
) {
    use evolite_api::workshop::OwnSearch;
    let p = a.p;
    match search {
        OwnSearch::Done => {}
        OwnSearch::Searching => {
            ui.add(crate::components::para(
                tr!("Looking for your collections…"),
                13.0,
                Weight::Regular,
                p.muted,
            ));
        }
        OwnSearch::Failed => {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.add(crate::components::para(
                    tr!("Could not find your collections"),
                    13.0,
                    Weight::Medium,
                    p.danger,
                ));
                // The list is asked again: the core starts the failed search with it.
                if crate::components::text_action(ui, a, "sync", tr!("Retry")).clicked() {
                    state.read_again();
                }
            });
        }
    }
}

/// One collection's name, its kind and its models; the whole row opens it.
fn collection_row(ui: &mut Ui, a: &mut Assets, c: &Collection) -> egui::Response {
    let p = a.p;
    let width = ui.available_width();
    let name = collection_name(c);
    let models = crate::text::plural(c.models.len() as u32, tr!("model"), tr!("models"));
    let row = crate::components::card(&p, egui::Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(crate::components::inner(width, 20));
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    // BUG-258: room for the chevron; a long name and its tags wrap.
                    ui.set_width((ui.available_width() - 30.0).max(120.0));
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(txt(
                            &crate::i18n::own(&name),
                            15.0,
                            Weight::SemiBold,
                            p.text,
                        ));
                        for tag in kind_tags(c) {
                            tag_chip(ui, &p, tag);
                        }
                    });
                    ui.label(txt(&models, 12.0, Weight::Regular, p.muted));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(a.icons.icon("chevron-right", p.muted, &p, 18.0));
                });
            });
        })
        .response;
    let resp = crate::a11y::row_click(ui, &row, &format!("workshop-collection-{}", c.contract));
    // BUG-270: the person's own words, as drawn — never translated («Seasons of the city» was
    // read «Seasons из the city»).
    crate::components::named(
        &resp,
        egui::WidgetType::Button,
        true,
        &crate::i18n::own(&name),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// BUG-272 (TEAM.md §8d, В3): a fee or a sum the core estimates — «≈ 0.1941 DASH»; while its
/// answer is still coming, «Calculating…» (the check took 20 s after a restart); «—» when
/// nothing is asked.
pub(super) fn estimate(sum: Option<evolite_api::Duffs>, counting: bool) -> String {
    match sum {
        Some(d) => format!("≈ {}", crate::format::amount(d)),
        None if counting => tr!("Calculating…").to_owned(),
        None => "—".to_owned(),
    }
}

use evolite_api::retry::Retry;

/// W7 r1 Must (TEAM.md §8d-4, -5): whether the page's read of `key` goes now, by the shared rule
/// (`evolite_api::retry`): never asked; or failed and its pause over, or `again` («Retry»).
pub(super) fn ask_now<K: std::hash::Hash + Eq + Clone>(
    asked: &mut HashMap<K, Retry>,
    key: &K,
    now: f64,
    again: bool,
) -> bool {
    let r = asked.entry(key.clone()).or_default();
    if again {
        r.again(now);
    }
    r.ask(now)
}

/// W7: one's copies of a model as read so far, page after page.
#[derive(Default)]
struct InstancesRead {
    list: Vec<evolite_api::workshop::Instance>,
    /// Where the next page starts; `None` with `done`.
    next: Option<String>,
    asks: u32,
    done: bool,
    /// The read of the next page, by the shared rule.
    retry: Retry,
}

/// At most this many asks of a model's copies (5 network pages each): «Your numbers» and
/// «Prices» of 10 000 copies at 100 a page; past it they say what was read.
const INSTANCE_ASKS: u32 = 20;

impl WorkshopState {
    /// W7: one's copies of a model (`for_sale`: those for sale only), asked page by page until
    /// all are read; what is read so far, and whether it is all.
    fn instances(
        &mut self,
        cx: &mut Ctx,
        contract: &str,
        model: &str,
        for_sale: bool,
        now: f64,
        again: bool,
    ) -> (Vec<evolite_api::workshop::Instance>, bool) {
        let read = self
            .instances
            .entry((contract.to_owned(), model.to_owned(), for_sale))
            .or_default();
        if again {
            read.retry.again(now);
        }
        if !read.done && read.asks < INSTANCE_ASKS && read.retry.ask(now) {
            read.asks += 1;
            cx.send(Command::ReadWorkshop {
                ask: WorkshopAsk::Instances {
                    contract: contract.to_owned(),
                    model: model.to_owned(),
                    for_sale,
                    after: read.next.clone(),
                },
            });
        }
        (read.list.clone(), read.done)
    }

    /// W7 r1: some read of the model's page failed and waits for «Retry» or its time.
    fn page_failed(&self, contract: &str, model: &str) -> bool {
        let key = (contract.to_owned(), model.to_owned());
        let failed =
            |m: &HashMap<(String, String), Retry>| m.get(&key).is_some_and(|r| r.is_failed());
        failed(&self.counts_asked)
            || failed(&self.given_asked)
            || self.sales_asked.is_failed()
            || [false, true].iter().any(|s| {
                self.instances
                    .get(&(contract.to_owned(), model.to_owned(), *s))
                    .is_some_and(|r| r.retry.is_failed())
            })
    }
}

/// The collection's name: its contract's description, or its short id when it has none.
fn collection_name(c: &Collection) -> String {
    if c.description.trim().is_empty() {
        crate::text::whole(&c.contract)
    } else {
        c.description.clone()
    }
}

/// «closed · immutable» (W1, W4): a collection by the profile is `readonly`; open ones are not
/// made here, but may be read.
fn kind_tags(c: &Collection) -> [&'static str; 2] {
    [
        // Their own keys: «closed» alone is a neuter elsewhere (ru «закрыто»).
        if c.open {
            tr!("open collection")
        } else {
            tr!("closed collection")
        },
        tr!("immutable"),
    ]
}

/// A small grey tag (W1, W4): 11 Medium on `raised`, radius 6.
fn tag_chip(ui: &mut Ui, p: &crate::theme::Palette, label: &str) {
    let font = crate::theme::sans(11.0, Weight::Medium);
    let w = 16.0 + crate::components::text_width(ui, label, font.clone());
    let (rect, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(6), p.raised);
    crate::components::paint_text(
        ui,
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        p.muted,
    );
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    /// BUG-266 on the shared rule: «not synced» is asked again 3 s after it came, then 6 s after
    /// the next one; never while asking; another answer starts the count again.
    #[test]
    fn a_not_synced_answer_is_asked_again_after_its_time() {
        let mut r = evolite_api::retry::Retry::default();
        let due = |now, answer, r: &mut _| not_synced_retry_due(now, answer, false, r);
        assert!(!due(10.0, Some(true), &mut r), "first seen");
        assert!(!due(12.9, Some(true), &mut r));
        assert!(due(13.0, Some(true), &mut r), "after 3 s");
        assert!(!due(13.5, None, &mut r), "asking");
        assert!(!due(14.0, Some(true), &mut r), "again «not synced»");
        assert!(!due(19.9, Some(true), &mut r));
        assert!(due(20.0, Some(true), &mut r), "after 6 s");
        assert!(!due(21.0, Some(false), &mut r), "answered");
        assert_eq!(r.failures, 0);
    }

    /// W7 r2 (the architect's Should, BUG-266's complaint): «not synced» for minutes — the sync
    /// then ends — the price is asked again within 30 s, and at once when the sync's stage
    /// moved.
    #[test]
    fn after_the_sync_the_price_comes_within_30_s() {
        let mut r = evolite_api::retry::Retry::default();
        // Ten minutes of «not synced», asked as often as the rule lets.
        let mut now = 0.0;
        let mut last_ask = 0.0;
        while now < 600.0 {
            if not_synced_retry_due(now, Some(true), false, &mut r) {
                assert!(now - last_ask <= 30.0 + 0.5, "a gap of {}", now - last_ask);
                last_ask = now;
                // The answer to that ask: «not synced» again.
                not_synced_retry_due(now, None, false, &mut r);
            }
            now += 0.5;
        }
        // The sync ends now: the next ask is at most 30 s away…
        assert!(r.next_at - now <= 30.0);
        // …and at once with the stage that moved.
        let mut seen = Some(evolite_api::SyncStage::Platform);
        let moved = sync_moved(&mut seen, evolite_api::SyncStage::Done);
        assert!(moved);
        assert!(not_synced_retry_due(now, Some(true), moved, &mut r));
        assert!(!sync_moved(&mut seen, evolite_api::SyncStage::Done), "once");
    }
}
