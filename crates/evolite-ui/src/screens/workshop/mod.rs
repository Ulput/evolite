//! FEAT-0038 «Workshop»: the main page (W1), a new collection (W2, its review W3 and steps) and
//! a collection's page (W4). Shown only while «Show the Workshop» is on (W0, AC-A1).
//!
//! Everything here is on behalf of the identity shown and paid with its credits; the reads are
//! the core's proved Workshop reading ([`Command::ReadWorkshop`]), asked again when the identity
//! changes, and the registration goes through the module V1 commands ([`Command::CheckContract`],
//! [`evolite_api::Spend::RegisterContract`]).

mod collection_page;
mod new_collection;
pub mod new_model;

use std::collections::{HashMap, HashSet};

use super::*;
use crate::components::{Assets, ButtonKind, button, button_enabled, page_header, section_label};
use crate::context::Ctx;
use crate::screens::txt;
use crate::theme::Weight;
use egui::{Align, Layout, Sense, Ui, vec2};
use evolite_api::workshop::{Collection, ModelCounts, WorkshopAsk, WorkshopRead};
use evolite_api::{Command, CoreError};

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
    counts_asked: HashSet<(String, String)>,
    pub new: NewCollection,
    /// W5: the new model's form.
    pub model: ModelForm,
    /// W4: «About the collection» open.
    about: bool,
}

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
                self.counts.insert(key, counts.clone());
            }
            // A failed count stays «—»; asked again on the page's next visit.
            (WorkshopAsk::Counts { contract, model }, Err(_)) => {
                self.counts_asked.remove(&(contract.clone(), model.clone()));
            }
            _ => {}
        }
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
        self.about = false;
        if !self.new.signed() {
            self.new.forget();
        }
        if !self.model.signed() {
            self.model.forget();
        }
        if matches!(self.page, Page::Collection(_) | Page::NewModel(_)) {
            self.page = Page::Home;
        }
    }

    /// Lock, a wallet switch: everything read and typed goes.
    pub fn forget_wallet(&mut self) {
        self.new.forget();
        self.model.forget();
        *self = Self::default();
    }
}

impl crate::screen_state::ScreenState for WorkshopState {
    #[cfg(all(test, demo_core, feature = "dev-tools"))]
    fn flows(&mut self) -> Vec<&mut dyn crate::components::spend_flow::AnyFlow> {
        vec![&mut self.new.flow, &mut self.model.flow]
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
    match state.page.clone() {
        Page::Home => home(ui, a, cx, state),
        Page::New => new_collection::page(ui, a, cx, state),
        Page::Collection(contract) => collection_page::page(ui, a, cx, state, art, &contract),
        Page::NewModel(_) => new_model::page(ui, a, cx, state, art),
    }
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
    // AC-A2: without an identity, only why and the way to one.
    if wallet.identity.is_none() {
        crate::components::notice(
            ui,
            a,
            "identity",
            p.accent,
            tr!("The Workshop needs an identity with credits."),
        );
        if button(ui, a, tr!("Create an identity"), ButtonKind::Primary, None).clicked() {
            cx.go(Screen::CreateIdentity);
        }
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
        None,
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
    let p = a.p;
    let shown = wallet.identity.as_ref().map(|i| i.id.clone());
    let mut chosen = None;
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 28.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(txt(tr!("Identity"), 13.0, Weight::Regular, p.muted));
            for c in &wallet.identities {
                let name = c
                    .name
                    .as_ref()
                    .map(|n| format!("@{n}"))
                    .unwrap_or_else(|| super::tokens::state::short(&c.id));
                let on = Some(&c.id) == shown.as_ref();
                if super::tokens::identity_chip(ui, &p, &crate::i18n::own(&name), on).clicked()
                    && !on
                {
                    chosen = Some(c.id.clone());
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(txt(&credits_line(wallet), 13.0, Weight::Regular, p.muted));
            });
        },
    );
    if let Some(id) = chosen {
        cx.send(Command::ChooseIdentity { id });
    }
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
        Some(Ok(all)) if all.is_empty() => {}
        Some(Ok(all)) => {
            let all = all.clone();
            section_label(
                ui,
                &p,
                &format!(
                    "{} · {}",
                    crate::i18n::t(tr!("My collections")),
                    crate::format::count(all.len() as u64)
                ),
            );
            for c in &all {
                if collection_row(ui, a, c).clicked() {
                    state.page = Page::Collection(c.contract.clone());
                    state.about = false;
                }
            }
        }
    }
    let _ = cx;
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
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.horizontal(|ui| {
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
    crate::components::named(&resp, egui::WidgetType::Button, true, &name);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The collection's name: its contract's description, or its short id when it has none.
fn collection_name(c: &Collection) -> String {
    if c.description.trim().is_empty() {
        super::tokens::state::short(&c.contract)
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
