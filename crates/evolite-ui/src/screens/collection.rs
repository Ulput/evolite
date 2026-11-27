//! 18 · Collection: the NFTs the wallet's identities hold (FEAT-0055) in collections of the
//! evolite standard (FEAT-0054: no partner contract), as the core found them on Platform, with
//! filters and the selected NFT on the right. Nothing is shown that is not there: an empty or
//! still searched list says so. Only the demo core can "Give" (the dialog), beside an inactive
//! "Sell"; the release, without the demo core, shows neither until the real core can do them.

use super::txt;
use crate::art::{ArtCache, Consent};
use crate::components::{
    Assets, ButtonKind, button, card, chip, divider, inner, kv_row, page_header, pill,
};
#[cfg(demo_core)]
use crate::components::{
    FieldKind, Part, button_enabled, dialog_with_summary, field_label, password_asked, text_field,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::{Weight, mono, sans};
use egui::{
    Align, CornerRadius, CursorIcon, Layout, Margin, Rect, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
#[cfg(demo_core)]
use evolite_api::Spend;
use evolite_api::{Command, CoreError, Duffs, Nft, NftOrigin, NftsStatus};
use std::collections::HashSet;

/// The filters of the Collection tab (BUG-188 §5): none chosen is all of it; 1 and 2 are
/// what `matches` takes.
const FILTERS: [&str; 2] = [tr!("Received"), tr!("For sale")];

/// BUG-188: a group's actions go through the Workshop's series windows, not built yet.
const SERIES_LATER: &str = tr!("Actions on several copies come in the next version");

/// Hints on the demo's buttons that are not there yet.
#[cfg(demo_core)]
#[cfg(demo_core)]
const SELL_LATER: &str = tr!("Selling NFTs comes in the next version");
#[cfg(demo_core)]
const TRANSFER_LATER: &str = tr!("NFT transfer comes in the next version");

fn matches(filter: usize, nft: &Nft) -> bool {
    match filter {
        1 => {
            matches!(
                nft.origin,
                NftOrigin::Gift { .. } | NftOrigin::Received { .. }
            )
        }
        2 => nft.price.is_some(),
        _ => true,
    }
}

#[derive(Default)]
pub struct CollectionState {
    pub filter: usize,
    /// FEAT-0055: only this identity's NFTs (its id); `None`: every identity's.
    pub only: Option<String>,
    pub selected: usize,
    /// BUG-188: the card is open as a window (content narrower than [`SIDE_FROM`]).
    pub open: bool,
    /// The page of a group's copies in its card, 20 a page.
    pub page: usize,
    /// The gift dialog is open for the selected NFT.
    pub gifting: bool,
    #[cfg(demo_core)]
    pub recipient: String,
    /// "Give" (TECH-spend-flow): the password for every payment — the transfer costs credits
    /// (BUG-42) — sent once, its own answer only.
    #[cfg(demo_core)]
    pub flow: crate::components::spend_flow::SpendFlow<()>,
    /// The core's answer to the last "Give".
    pub result: Option<Result<(), CoreError>>,
    /// FEAT-0038 W10 (UC-13): the series window on one copy — «Give…», «Sell…», «Change the
    /// price…», «Take off sale…» — and the model's name for it.
    pub series: Option<super::workshop::SeriesForm>,
    pub series_model: String,
    /// The copy the window is about, as the card showed it.
    pub series_copy: Option<SeriesCopy>,
    /// `--screen collection-pin…`: the demo's pin's card, or this window on it, once the list
    /// has it.
    #[cfg(feature = "dev-tools")]
    pub preset_pin: Option<Option<CopyAct>>,
    /// The pictures, drawn by the core at the size of the cards.
    pub art: ArtCache,
    /// Settings → Collection: a picture from an issuer not trusted waits for "Show".
    pub ask_unknown: bool,
    /// Issuers whose pictures are drawn without asking ("Always show from …").
    pub trusted: HashSet<String>,
    /// NFTs whose picture the person asked to see.
    pub shown: HashSet<String>,
}

/// Issuers trusted out of the box: the demo's partner issuer only (FEAT-0054 r1). The real core
/// has none: a picture is drawn at once from a source the person chose ([`Nft::chosen`]) or an
/// issuer they trusted ([`evolite_api::Wallet::trusted_issuers`]).
#[cfg(demo_core)]
const TRUSTED: [&str; 1] = ["jembe"];
#[cfg(not(demo_core))]
const TRUSTED: [&str; 0] = [];

impl CollectionState {
    /// Another wallet opens: the gift dialog, its answer and the pictures asked for go; the
    /// drawn pictures and the settings stay (BUG-45).
    /// The core's answer to «Give», taken only by the dialog that asked it (`ask`). A wrong
    /// password is said under its field.
    pub fn on_gift_done(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) {
        #[cfg(demo_core)]
        if self.flow.on_done(ask, result)
            && let Some(((), result)) = self.flow.take_result()
        {
            self.result = Some(result);
        }
        #[cfg(not(demo_core))]
        let _ = (ask, result);
    }

    pub fn forget_wallet(&mut self) {
        self.only = None;
        self.selected = 0;
        self.open = false;
        self.page = 0;
        self.gifting = false;
        #[cfg(demo_core)]
        self.recipient.clear();
        self.result = None;
        self.shown.clear();
        if let Some(form) = &mut self.series {
            form.forget();
        }
        self.series = None;
        self.series_copy = None;
    }

    /// The picture of `nft` is not drawn until the person asks: an issuer nobody trusted may
    /// send anything, and drawing it is the first thing that could go wrong. FEAT-0054 r1: drawn
    /// at once from a source the person chose (their own collection, one added by id, a
    /// purchase) or an issuer the wallet keeps as trusted (`trusted`).
    pub fn waits_for_consent(&self, nft: &Nft, trusted: &[String]) -> bool {
        self.ask_unknown
            && !nft.chosen
            && !TRUSTED.contains(&nft.issuer.as_str())
            && !trusted.contains(&nft.issuer)
            && !self.trusted.contains(&nft.issuer)
            && !self.shown.contains(&nft.id)
    }
}

/// DPNS label: 3–63 letters, digits and inner hyphens, with an optional "@". Also the
/// Contacts search's «Find @… in DPNS» (FEAT-0057 П4), so in every build.
pub fn is_dash_name(input: &str) -> bool {
    let label = input.trim().trim_start_matches('@');
    (3..=63).contains(&label.len())
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !label.starts_with('-')
        && !label.ends_with('-')
        && !label.contains("--")
}

pub fn collection(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut CollectionState) {
    // BUG-262: no identity — its empty state; NFTs are held by an identity.
    ui.spacing_mut().item_spacing.y = 20.0;
    if super::no_identity::page(
        ui,
        a,
        cx,
        tr!("Collection"),
        super::no_identity::Needs::Collection,
    ) {
        return;
    }
    let p = a.p;
    let wallet = cx.wallet();
    let nfts = &wallet.nfts;
    // Giving away works only in the demo; the real core does not transfer NFTs yet.
    #[cfg(demo_core)]
    let gift_fee = cx.snap.demo.then_some(wallet.fees.nft_transfer);
    #[cfg(not(demo_core))]
    let gift_fee: Option<Duffs> = None;
    let now = cx.now;
    ui.spacing_mut().item_spacing.y = 20.0;
    // The count only for NFTs there are: none while the list loads, not «0» beside «Loading…»
    // (BUG-74 №3, the designer's improvement 1 rule 9).
    let count = (!nfts.is_empty()).then(|| nfts.len().to_string());
    page_header(ui, a, tr!("Collection"), count.as_deref(), |_, _| {});
    // BUG-277 (the class): everything under the header scrolls — ten identities' chips and the
    // filters over the grid leave it no room at 800 × 600 otherwise.
    // The page's width before its scroll bar: the columns and the side card are counted on it
    // (BUG-188), as when only the grid scrolled.
    let page_w = ui.available_width();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // Before the list: avatars ask too, and there may be no NFT here at all (BUG-95).
        if let Some(why) = state.art.off.or(a.avatars.off) {
            crate::components::notice(
                ui,
                a,
                "warning",
                p.muted,
                crate::art::pictures_off_text(why),
            );
        }
        // FEAT-0055: every identity's NFTs together; with several identities, «All» and each one
        // to show only its own (the one picker, BUG-265). An identity gone shows all again. No
        // filter while the wallet has no NFT at all: nothing to tell apart.
        if state
            .only
            .as_ref()
            .is_some_and(|only| !wallet.identities.iter().any(|i| &i.id == only))
        {
            state.only = None;
        }
        if wallet.identities.len() > 1
            && (!nfts.is_empty() || state.only.is_some())
            && let Some(only) = crate::components::identity_filter(
                ui,
                a,
                wallet,
                tr!("Identity"),
                state.only.as_deref(),
            )
        {
            state.only = only;
        }
        // r1: under one identity's chip its own list's state — another identity's failed read is
        // not this one's; under «All» every list's together.
        let status = state
            .only
            .as_ref()
            .and_then(|only| {
                wallet
                    .nfts_statuses
                    .iter()
                    .find(|(id, _)| id == only)
                    .map(|(_, s)| s)
            })
            .unwrap_or(&wallet.nfts_status);
        let owned = |n: &Nft| state.only.as_ref().is_none_or(|only| &n.owner_id == only);
        if !nfts.iter().any(owned) {
            if empty(ui, a, status) {
                cx.send(Command::RefreshNfts);
            }
            return;
        }
        // BUG-188 §5: the tab «Collection» (always chosen while «New» is not built), a divider,
        // then the filters, each on and off again; none on is all of the Collection.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if pill(ui, &p, tr!("Collection"), true).clicked() {
                state.filter = 0;
            }
            let (bar, _) = ui.allocate_exact_size(vec2(9.0, 20.0), Sense::hover());
            ui.painter()
                .vline(bar.center().x, bar.y_range(), Stroke::new(1.0, p.line));
            for (k, label) in FILTERS.iter().enumerate() {
                let filter = k + 1;
                if pill(ui, &p, label, state.filter == filter).clicked() {
                    state.filter = if state.filter == filter { 0 } else { filter };
                }
            }
        });
        // The saved list is still here and the core asks again by itself: not an error (calm
        // states CS-1); «Try again» asks now (CS-1a).
        if let NftsStatus::Failed(why) = status {
            crate::components::notice(
                ui,
                a,
                "info",
                p.muted,
                &format!(
                    tr!(
                        "The list may be out of date: it did not refresh from Dash Platform ({}). Your saved NFTs are still here."
                    ),
                    why
                ),
            );
            if button(ui, a, tr!("Try again"), ButtonKind::Secondary, None).clicked() {
                cx.send(Command::RefreshNfts);
            }
        }

        let shown: Vec<usize> = (0..nfts.len())
            .filter(|i| matches(state.filter, &nfts[*i]))
            .filter(|i| owned(&nfts[*i]))
            .collect();
        // The panel follows the filter: a hidden NFT gives way to the first one shown.
        if !shown.contains(&state.selected)
            && let Some(first) = shown.first()
        {
            state.selected = *first;
        }

        let mut asked = Vec::new();
        let asks: Vec<bool> = nfts
            .iter()
            .map(|n| state.waits_for_consent(n, &wallet.trusted_issuers))
            .collect();
        // BUG-188: copies of one model are one tile «×N».
        let groups = groups(nfts, &shown);
        let selected = groups
            .iter()
            .position(|g| g.contains(&state.selected))
            .map(|k| groups[k].clone());
        // BUG-308 r1: a group of the shown identity's own model says where it can be given now.
        let mine = selected
            .as_ref()
            .is_some_and(|g| is_shown_identitys(wallet, &nfts[g[0]].issuer));
        // The card beside the grid only while the grid keeps four columns: content of 952 and
        // wider (a window of 1280); narrower, it opens as a window over the grid (BUG-188).
        let total = page_w;
        let side = total >= SIDE_FROM;
        let grid_w = if side {
            total - SIDE_W - SIDE_GAP
        } else {
            total
        };
        let mut clicked = false;
        // W10: the series window a card asked for, and its copy.
        let mut opened: Option<(CopyAct, usize)> = None;
        ui.scope(|ui| {
            let grid = |ui: &mut Ui,
                        a: &mut Assets,
                        state: &mut CollectionState,
                        asked: &mut Vec<Command>|
             -> bool {
                let mut clicked = false;
                // The scroll bar may take its 10 from the width: the columns are counted on the
                // width the spec gives, the tiles share what is left (BUG-188).
                let inner_w = ui.available_width().min(grid_w);
                ui.set_width(inner_w);
                ui.spacing_mut().item_spacing.y = 16.0;
                if groups.is_empty() {
                    card(&p, Margin::symmetric(20, 32)).show(ui, |ui| {
                        ui.set_width(inner(inner_w, 20));
                        ui.vertical_centered(|ui| {
                            ui.label(txt(tr!("Nothing with this filter"), 14.0, Weight::Regular, p.muted))
                        });
                    });
                } else {
                    let (cols, _) = tile_grid(grid_w);
                    let tile_w = tile_width(inner_w, cols);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = TILE_GAP;
                        for row in groups.chunks(cols) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = TILE_GAP;
                                for group in row {
                                    let first = group[0];
                                    let (tile, show) = nft_card(
                                        ui,
                                        a,
                                        &mut state.art,
                                        asked,
                                        nfts,
                                        group,
                                        (group.contains(&state.selected), asks[first]),
                                        tile_w,
                                    );
                                    if show {
                                        state.shown.insert(nfts[first].id.clone());
                                    } else if tile.clicked() {
                                        if !group.contains(&state.selected) {
                                            state.page = 0;
                                        }
                                        state.selected = first;
                                        clicked = true;
                                    }
                                }
                            });
                        }
                    });
                }
                crate::components::notice(
                    ui,
                    a,
                    "info",
                    p.muted,
                    tr!("NFTs of evolite collections on Dash Platform: the picture is stored in the documents themselves — no outside servers — and drawn by its hash on this computer."),
                );
                clicked
            };
            if side {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = SIDE_GAP;
                    ui.vertical(|ui| clicked = grid(ui, a, state, &mut asked));
                    if let Some(group) = &selected {
                        ui.vertical(|ui| {
                            card(&p, Margin::same(20))
                                .corner_radius(CornerRadius::same(18))
                                .show(ui, |ui| {
                                    ui.set_width(inner(SIDE_W, 20));
                                    let picked = card_body(
                                        ui,
                                        a,
                                        (&mut state.art, &mut asked),
                                        (nfts, group, &asks),
                                        (gift_fee, now, &mut state.page),
                                        (&wallet.series, true, mine),
                                    );
                                    state.gifting |= picked.gift;
                                    opened = opened.or(picked.series.map(|act| (act, group[0])));
                                    if let Some(issuer) = picked.trust {
                                        // r1: kept by the wallet, not only this run.
                                        asked.push(Command::TrustIssuer {
                                            issuer: issuer.clone(),
                                        });
                                        state.trusted.insert(issuer);
                                    }
                                });
                        });
                    }
                });
            } else {
                clicked = grid(ui, a, state, &mut asked);
            }
        });
        // Narrower than 952: the card is a window of 520, opened by a click on a tile (C4).
        state.open = !side && (state.open || clicked);
        if state.open
            && let Some(group) = &selected
        {
            let nft = &nfts[group[0]];
            let title = if group.len() > 1 {
                format!(tr!("{name} · ×{n}"), name = nft.name, n = group.len())
            } else {
                nft.name.clone()
            };
            let mut picked = Picked::default();
            // W10: one copy's series buttons in the window's footer.
            let footer = match (group.len(), offer(nft, nfts, &wallet.series)) {
                (1, Offer::Series(why)) => Some(why),
                _ => None,
            };
            let mut from_footer = None;
            let close = crate::components::dialog_with_header_and_footer(
                ui,
                a,
                "nft-card",
                CARD_W,
                |ui, _| {
                    ui.label(txt(&title, 20.0, Weight::Bold, p.text));
                },
                |ui, a, part| match part {
                    crate::components::Part::Footer => {
                        if let Some(why) = footer {
                            from_footer = copy_actions(ui, a, nft, why, false);
                        }
                        from_footer.is_some()
                    }
                    crate::components::Part::Summary => false,
                    crate::components::Part::Middle => {
                        picked = card_body(
                            ui,
                            a,
                            (&mut state.art, &mut asked),
                            (nfts, group, &asks),
                            (gift_fee, now, &mut state.page),
                            (&wallet.series, false, mine),
                        );
                        picked.gift
                    }
                },
            );
            picked.series = picked.series.or(from_footer);
            opened = opened.or(picked.series.map(|act| (act, group[0])));
            state.gifting |= picked.gift;
            if let Some(issuer) = picked.trust {
                // r1: kept by the wallet, not only this run.
                asked.push(Command::TrustIssuer {
                    issuer: issuer.clone(),
                });
                state.trusted.insert(issuer);
            }
            // One window at a time (BUG-188 §5): a series window takes the card's place.
            if close || picked.gift || picked.series.is_some() {
                state.open = false;
            }
        }
        // `--screen collection-pin…`: the pin's card or one of its windows, once it is listed.
        #[cfg(feature = "dev-tools")]
        if let Some(act) = state.preset_pin
            && let Some(i) = nfts.iter().position(|n| n.id == "demo-pin-1")
        {
            state.preset_pin = None;
            state.selected = i;
            match act {
                None => state.open = true,
                Some(act) => {
                    open_series(state, &nfts[i], act);
                    if let Some(form) = &mut state.series {
                        form.preset_fill();
                    }
                }
            }
        }
        if let Some((act, i)) = opened {
            open_series(state, &nfts[i], act);
        }
        for command in asked {
            cx.send(command);
        }

        #[cfg(demo_core)]
        if state.gifting {
            gift_dialog(ui, a, cx, state);
        }
    });
    series_window(ui, a, cx, state);
}

/// W10: what a card's button opens on its one copy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CopyAct {
    Give,
    Sell,
    ChangePrice,
    TakeOff,
}

/// W10: the copy a series window is about — its owner, collection, model, number and price
/// as the card showed them.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesCopy {
    id: String,
    target: evolite_api::series::SeriesTarget,
    copy: super::workshop::OneCopy,
}

/// W10: what the card of one copy offers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Offer {
    /// Not a copy of a Workshop model (no model or no number): the card as before.
    Before,
    /// The series windows on this copy; `Some(why)`: shown, not active, with why.
    Series(Option<&'static str>),
}

/// W10 (UC-13): the series windows are offered on a copy of a Workshop model with a number
/// when its identity holds no other copy of that model — the core takes the free copies with the
/// lowest numbers or those at a price from the highest, so the copy on the card is the one
/// written only then (R-10); and not while a series of that identity runs (R-8, AC-A8).
fn offer(nft: &Nft, nfts: &[Nft], series: &[evolite_api::series::SeriesState]) -> Offer {
    let (Some(model), Some(_)) = (nft.model.as_deref(), nft.edition.0) else {
        return Offer::Before;
    };
    let copies = nfts
        .iter()
        .filter(|n| {
            n.owner_id == nft.owner_id
                && n.contract == nft.contract
                && n.model.as_deref() == Some(model)
        })
        .count();
    if copies > 1 {
        return Offer::Series(Some(SERIES_LATER));
    }
    if series
        .iter()
        .any(|s| s.running && s.target.identity == nft.owner_id)
    {
        return Offer::Series(Some(tr!("A series is running — wait for it or stop it")));
    }
    Offer::Series(None)
}

/// W10: the window `act` asks for on `nft`, the copy of its card.
fn open_series(state: &mut CollectionState, nft: &Nft, act: CopyAct) {
    use super::workshop::{OneCopy, SeriesAction, SeriesForm};
    let (Some(model), Some(serial)) = (nft.model.clone(), nft.edition.0) else {
        return;
    };
    let target = evolite_api::series::SeriesTarget {
        identity: nft.owner_id.clone(),
        contract: nft.contract.clone(),
        model,
    };
    let copy = OneCopy {
        serial,
        price: nft.price,
    };
    let action = match (act, nft.price) {
        (CopyAct::Give, _) => SeriesAction::Give,
        (CopyAct::Sell, _) => SeriesAction::Sell,
        (CopyAct::ChangePrice, Some(from)) => SeriesAction::Reprice { from },
        (CopyAct::TakeOff, Some(_)) => SeriesAction::Unprice,
        // Not for sale: nothing to change or take off.
        (CopyAct::ChangePrice | CopyAct::TakeOff, None) => return,
    };
    state.series = Some(SeriesForm::one(action, target.clone(), copy));
    state.series_model = nft.name.clone();
    state.series_copy = Some(SeriesCopy {
        id: nft.id.clone(),
        target,
        copy,
    });
}

/// W10: the open series window on one copy; when its series is signed and answered well, the
/// Collection is read again (the price, a copy given away).
fn series_window(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut CollectionState) {
    let Some(one) = state.series_copy.clone() else {
        return;
    };
    let OneCopy { serial, price } = one.copy;
    use super::workshop::OneCopy;
    // What the window counts from: the one copy, free or at its price.
    let for_sale: Vec<evolite_api::workshop::Instance> = price
        .map(|price| evolite_api::workshop::Instance {
            id: one.id.clone(),
            serial,
            price: Some(price),
        })
        .into_iter()
        .collect();
    let have = super::workshop::SeriesHave {
        free: Some(u64::from(price.is_none())),
        left_to_issue: None,
        for_sale: &for_sale,
        for_sale_read: true,
    };
    let name = state.series_model.clone();
    if super::workshop::series_dialog(ui, a, cx, &mut state.series, &name, &have) {
        cx.send(Command::RefreshNfts);
    }
    if state.series.is_none() {
        state.series_copy = None;
    }
}

fn edition(nft: &Nft) -> String {
    match nft.edition {
        // FEAT-0054 r1: the document has no number — none shown.
        (None, _) => String::new(),
        // The issuer set no edition size.
        (Some(n), 0) => format!("№ {n}"),
        (Some(n), of) => format!(tr!("No. {n} of {of}"), n = n, of = of),
    }
}

/// No NFTs: why, as the core says, instead of an empty grid. After a failed load, «Try again»;
/// `true` when it was clicked (CS-1a).
fn empty(ui: &mut Ui, a: &mut Assets, status: &NftsStatus) -> bool {
    let mut again = false;
    let p = a.p;
    let (title, detail) = match status {
        // TEAM.md §8d-4: «still searching», not «none».
        NftsStatus::Loading => (tr!("Looking for your NFTs…"), String::new()),
        NftsStatus::NoIdentity => (
            tr!("NFTs belong to an identity"),
            tr!("Make an identity — the NFTs it gets will show here.").into(),
        ),
        // The core asks again by itself (BUG-10): the reason and that, not an endless wait.
        NftsStatus::Failed(why) => (tr!("Could not load the NFTs"), failed_detail(why)),
        NftsStatus::Ready => (
            tr!("You have no NFTs yet"),
            tr!(
                "NFTs of evolite collections show here when your identities make, get or buy them."
            )
            .into(),
        ),
    };
    let width = ui.available_width();
    card(&p, Margin::symmetric(20, 32)).show(ui, |ui| {
        ui.set_width(inner(width, 20));
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(txt(title, 15.0, Weight::SemiBold, p.text));
            if !detail.is_empty() {
                ui.add(crate::components::para(
                    &detail,
                    13.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
            if matches!(status, NftsStatus::Failed(_)) {
                again = button(ui, a, tr!("Try again"), ButtonKind::Secondary, None).clicked();
            }
        });
    });
    again
}

/// BUG-188: the narrowest and widest a tile may be, and the gap between tiles.
pub(crate) const TILE_MIN: f32 = 148.0;
pub(crate) const TILE_MAX: f32 = 196.0;
pub(crate) const TILE_GAP: f32 = 12.0;
/// The most columns, however wide.
const MAX_COLS: usize = 8;
/// The card beside the grid: its width, the gap before it, and the width of content from which
/// it is there at all (four columns still fit beside it: a window of 1280).
pub(crate) const SIDE_W: f32 = 300.0;
const SIDE_GAP: f32 = 24.0;
pub(crate) const SIDE_FROM: f32 = 952.0;
/// The card as a window, narrower than [`SIDE_FROM`] (C4).
const CARD_W: f32 = 520.0;
/// The copies a group's card lists on one page.
const PAGE: usize = 20;

/// BUG-188 §2: the columns of a grid `width` wide and the width of each tile:
/// `floor((W + 12) / 160)` columns, at most 8, one more while a tile would be wider than 196.
/// A few tiles are not stretched over the row: the width is the same whatever their number.
pub(crate) fn tile_grid(width: f32) -> (usize, f32) {
    let mut cols =
        (((width + TILE_GAP) / (TILE_MIN + TILE_GAP)).floor() as usize).clamp(1, MAX_COLS);
    while tile_width(width, cols) > TILE_MAX && cols < MAX_COLS {
        cols += 1;
    }
    (cols, tile_width(width, cols))
}

/// The width of each of `cols` tiles in `width`, [`TILE_GAP`] between them.
fn tile_width(width: f32, cols: usize) -> f32 {
    ((width - TILE_GAP * (cols as f32 - 1.0)) / cols as f32).max(0.0)
}

/// BUG-188 §4: the shown NFTs (`shown`, indices into `nfts` in their order) by model — one
/// contract and one picture — each group where its first copy stands.
pub(crate) fn groups(nfts: &[Nft], shown: &[usize]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for &i in shown {
        let same = |g: &&mut Vec<usize>| {
            let first = &nfts[g[0]];
            first.contract == nfts[i].contract && first.art == nfts[i].art
        };
        match groups.iter_mut().find(same) {
            Some(group) => group.push(i),
            None => groups.push(vec![i]),
        }
    }
    groups
}

/// A group's numbers on its tile: «№ 12 · 40 · 77», a run «№ 40–43»; one copy — «No. 3 of 100».
pub(crate) fn numbers(nfts: &[Nft], group: &[usize]) -> String {
    if let [one] = group {
        return edition(&nfts[*one]);
    }
    let mut n: Vec<u32> = group.iter().filter_map(|&i| nfts[i].edition.0).collect();
    n.sort_unstable();
    n.dedup();
    let run = n.windows(2).all(|w| w[1] == w[0] + 1);
    match (n.first(), n.last()) {
        (Some(a), Some(b)) if run && n.len() > 2 => format!("№ {a}–{b}"),
        _ => {
            let list: Vec<String> = n.iter().map(u32::to_string).collect();
            format!("№ {}", list.join(" · "))
        }
    }
}

/// Where a copy came from, in a few words: «from @alice», «issue jembe».
fn origin(nft: &Nft) -> String {
    match &nft.origin {
        // A gift from an address with no name: "from " alone would stay untranslated.
        NftOrigin::Gift { from, .. } if from.is_empty() => tr!("from an unknown address").into(),
        NftOrigin::Gift { from, .. } => format!(tr!("from {from}"), from = from),
        NftOrigin::Received { .. } => tr!("received").into(),
        NftOrigin::Release => format!(tr!("issue {}"), nft.issuer),
    }
}

/// A tile's third line: «2 for sale of 4» in the accent when any copy is for sale, else where
/// the first came from.
fn third_line(nfts: &[Nft], group: &[usize]) -> (String, bool) {
    let on_sale = group.iter().filter(|&&i| nfts[i].price.is_some()).count() as u32;
    if on_sale == 0 {
        return (origin(&nfts[group[0]]), false);
    }
    let line = text::plural_in(on_sale, tr!("{} for sale of {}"), tr!("{} for sale of {}"))
        .replacen("{}", &crate::format::count(group.len() as u64), 1);
    (line, true)
}

/// One row of a tile's name, 14 SemiBold.
const NAME_ROW: f32 = 19.0;

/// A tile (BUG-188 §3): the picture, square, under it the name in two rows at most, the
/// numbers and a third line, each of one row ending in «…»; «×N» on the picture's corner for
/// a group. All tiles are of one height. Selected — 1.5 px ink border. `(selected, ask)`:
/// `ask` — the picture waits for "Show"; the second value says it was clicked.
#[allow(clippy::too_many_arguments)]
fn nft_card(
    ui: &mut Ui,
    a: &mut Assets,
    art_cache: &mut ArtCache,
    asked: &mut Vec<Command>,
    nfts: &[Nft],
    group: &[usize],
    (selected, ask): (bool, bool),
    width: f32,
) -> (egui::Response, bool) {
    let p = a.p;
    let nft = &nfts[group[0]];
    let art = width - 16.0;
    let height = 8.0 + art + 10.0 + 2.0 * NAME_ROW + 4.0 + 16.0 + 16.0 + 10.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let label = if group.len() > 1 {
        format!(tr!("{name} · ×{n}"), name = nft.name, n = group.len())
    } else {
        nft.name.clone()
    };
    crate::components::named_choice(&resp, egui::WidgetType::Button, selected, &label);
    let resp = resp.on_hover_text(crate::i18n::t(&label).into_owned());
    let stroke = if selected {
        Stroke::new(1.5, p.ink)
    } else if resp.hovered() {
        Stroke::new(1.0, p.muted)
    } else {
        Stroke::new(1.0, p.line)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(14),
        p.surface,
        stroke,
        StrokeKind::Inside,
    );
    let art_rect = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(art, art));
    // «×N» on the picture's top right corner (the frame mirrors it in Arabic).
    let badge = (group.len() > 1).then(|| {
        let galley = ui.painter().layout_no_wrap(
            format!(tr!("×{}"), group.len()),
            sans(12.0, Weight::SemiBold),
            p.text,
        );
        let size = galley.size() + vec2(14.0, 6.0);
        let rect = Rect::from_min_size(
            pos2(art_rect.right() - 8.0 - size.x, art_rect.top() + 8.0),
            size,
        );
        (rect, galley)
    });
    let mut show = false;
    if ask {
        // Nothing is drawn, not even in the sandbox, until the person asks. FEAT-0054 r1 Must 3:
        // the words and «Show» under the «×N» badge, never drawn over it.
        ui.painter()
            .rect_filled(art_rect, CornerRadius::same(10), p.raised);
        let mut room = art_rect.shrink(12.0);
        if let Some((badge, _)) = &badge {
            room.min.y = room.min.y.max(badge.bottom() + 6.0);
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(room)
                .layout(Layout::top_down(Align::Center).with_main_align(Align::Center)),
        );
        child.spacing_mut().item_spacing.y = 10.0;
        child.label(txt(
            &format!(tr!("Picture from {}"), nft.issuer),
            12.0,
            Weight::Regular,
            p.muted,
        ));
        show = button(&mut child, a, tr!("Show"), ButtonKind::Secondary, None).clicked();
    } else {
        // Drawn by the core's sandboxed renderer at the size of the card, not decoded here.
        art_cache.paint(ui, &p, &nft.art, art_rect, Consent::waits(ask), &mut |c| {
            asked.push(c)
        });
    }
    let painter = ui.painter();
    if let Some((badge, galley)) = badge {
        painter.rect_filled(badge, CornerRadius::same(8), p.surface.gamma_multiply(0.92));
        painter.galley(badge.min + vec2(7.0, 3.0), galley, p.text);
    }
    let x = rect.left() + 8.0;
    let room = rect.width() - 16.0;
    let mut y = art_rect.bottom() + 10.0;
    let name = painter.layout_job(tile_name(painter.ctx(), &nft.name, room, p.text));
    painter.galley(pos2(x, y), name, p.text);
    y += 2.0 * NAME_ROW + 4.0;
    let line = |text: &str, font: egui::FontId, color: egui::Color32, y: f32| {
        // Arabic cut at a word in reading order, its start kept (BUG-118); egui cuts the rest.
        let mut job = egui::text::LayoutJob::single_section(
            crate::components::cut_to(painter.ctx(), text, &font, room, 1),
            egui::TextFormat::simple(font, color),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        painter.galley(pos2(x, y), painter.layout_job(job), color);
    };
    line(
        &crate::i18n::t(&numbers(nfts, group)),
        crate::theme::figures(11.0, Weight::Regular),
        p.muted,
        y,
    );
    let (third, on_sale) = third_line(nfts, group);
    line(
        &crate::i18n::t(&third),
        sans(11.0, Weight::Medium),
        if on_sale { p.accent } else { p.muted },
        y + 16.0,
    );
    (resp.on_hover_cursor(CursorIcon::PointingHand), show)
}

/// A tile's name: two rows at most, then «…» (design review №24: one row cut «Кольцо с
/// брилли…»); the whole of it is also on hover and in the details.
fn tile_name(
    ctx: &egui::Context,
    name: &str,
    width: f32,
    color: egui::Color32,
) -> egui::text::LayoutJob {
    let font = sans(14.0, Weight::SemiBold);
    // In Arabic: two lines in reading order, the second cut at a word (BUG-118, BUG-119).
    let mut job = egui::text::LayoutJob::single_section(
        crate::components::cut_to(ctx, name, &font, width, 2),
        egui::TextFormat::simple(font, color),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: width,
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    job
}

/// What the card's buttons asked for.
#[derive(Default)]
struct Picked {
    /// «Give» (the demo's, one copy).
    gift: bool,
    /// W10: a series window on the one copy.
    series: Option<CopyAct>,
    /// «Always show from …»: this issuer.
    trust: Option<String>,
}

/// The card of the selected tile: one copy as before ([`detail`]), a group as BUG-188 §4.
/// `panel`: beside the grid (300), else in the window (520).
fn card_body(
    ui: &mut Ui,
    a: &mut Assets,
    (art_cache, asked): (&mut ArtCache, &mut Vec<Command>),
    (nfts, group, asks): (&[Nft], &[usize], &[bool]),
    (gift_fee, now, page): (Option<Duffs>, chrono::DateTime<chrono::Local>, &mut usize),
    (series, panel, mine): (&[evolite_api::series::SeriesState], bool, bool),
) -> Picked {
    let first = group[0];
    if group.len() == 1 {
        let offer = offer(&nfts[first], nfts, series);
        let (gift, trust, act) = detail(
            ui,
            a,
            &nfts[first],
            asks[first],
            (gift_fee, offer),
            now,
            panel,
        );
        return Picked {
            gift,
            series: act,
            trust: trust.then(|| nfts[first].issuer.clone()),
        };
    }
    // BUG-282: the group's picture as its tile's — waiting for consent while its copy does
    // («Always show from …» here too).
    let trust = group_card(
        ui,
        a,
        (art_cache, asked),
        (nfts, group, asks),
        (now, page),
        (panel, mine),
    );
    Picked {
        gift: false,
        series: None,
        trust: trust.then(|| nfts[first].issuer.clone()),
    }
}

/// BUG-188 §4: the card of copies of one model — what each is, its price, the actions on N.
fn group_card(
    ui: &mut Ui,
    a: &mut Assets,
    (art_cache, asked): (&mut ArtCache, &mut Vec<Command>),
    (nfts, group, asks): (&[Nft], &[usize], &[bool]),
    (now, page): (chrono::DateTime<chrono::Local>, &mut usize),
    (panel, mine): (bool, bool),
) -> bool {
    let p = a.p;
    let mut trust = false;
    // BUG-309: the copies by their number, those without one last.
    let mut group = group.to_vec();
    group.sort_by_key(|&i| (nfts[i].edition.0.is_none(), nfts[i].edition.0));
    let group = &group[..];
    // BUG-282: the picture shown is the first copy's: it waits while that one does.
    let ask = asks[group[0]];
    let nft = &nfts[group[0]];
    let n = group.len();
    ui.spacing_mut().item_spacing.y = 12.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (pic, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
        art_cache.paint(ui, &p, &nft.art, pic, Consent::waits(ask), &mut |c| {
            asked.push(c)
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            if panel {
                ui.add(crate::components::para(
                    &format!(tr!("{name} · ×{n}"), name = nft.name, n = n),
                    18.0,
                    Weight::Bold,
                    p.text,
                ));
            }
            let kind = match nft.origin {
                NftOrigin::Gift { .. } | NftOrigin::Received { .. } => {
                    format!(tr!("Gift {}"), capitalized(&nft.issuer))
                }
                NftOrigin::Release => format!(tr!("Issue {}"), nft.issuer),
            };
            let model = match nft.edition.1 {
                0 => kind,
                of => format!(tr!("{kind} · model of {of}"), kind = kind, of = of),
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(crate::i18n::t(&model))
                        .font(crate::theme::figures(12.0, Weight::Regular))
                        .color(p.muted),
                )
                .wrap(),
            );
        });
    });
    // BUG-282: a picture that waits for consent — the issuer trusted from here, as on one copy's
    // card (the tile has its «Show»).
    if ask {
        trust = button(
            ui,
            a,
            &format!(tr!("Always show from {}"), nft.issuer),
            ButtonKind::Ghost,
            None,
        )
        .on_hover_text(
            crate::i18n::t(tr!(
                "Pictures from this issuer will be drawn without asking"
            ))
            .into_owned(),
        )
        .clicked();
    }
    let prices: Vec<Duffs> = group.iter().filter_map(|&i| nfts[i].price).collect();
    let summary = copies_summary(n, &prices);
    ui.add(crate::components::para(
        &crate::i18n::t(&summary),
        14.0,
        Weight::Medium,
        p.text,
    ));
    ui.label(txt(tr!("COPIES"), 11.0, Weight::SemiBold, p.muted));
    // More than 20: pages, their numbers above the list.
    let pages = n.div_ceil(PAGE);
    *page = (*page).min(pages - 1);
    if pages > 1 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for k in 0..pages {
                if pill(ui, &p, &(k + 1).to_string(), *page == k).clicked() {
                    *page = k;
                }
            }
        });
    }
    let all_verified = group.iter().all(|&i| nfts[i].art_verified);
    let width = ui.available_width();
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 4))
        .show(ui, |ui| {
            ui.set_width(width - 28.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            let shown = &group[*page * PAGE..(*page * PAGE + PAGE).min(n)];
            for (row, &i) in shown.iter().enumerate() {
                if row > 0 {
                    divider(ui, &p);
                }
                copy_row(ui, &p, &nfts[i], (now, all_verified), panel);
            }
        });
    if all_verified {
        chip(
            ui,
            a,
            "confirmed",
            tr!("The picture matches its hash"),
            true,
        );
    }
    // The actions on N go through the Workshop's series windows (W8, W9, W9b), not here yet.
    let actions = [
        (tr!("Give…"), ButtonKind::Primary),
        (tr!("Put up for sale…"), ButtonKind::Secondary),
        (tr!("Take off sale…"), ButtonKind::Secondary),
    ];
    let later = crate::i18n::t(SERIES_LATER).into_owned();
    // BUG-308: not there yet, and seen to be so at every width — the buttons dimmed (in the
    // window they were as bright as working ones on its ground), and under them what is true
    // now, not what they will do.
    let action = |ui: &mut Ui, a: &mut Assets, (label, kind): (&str, ButtonKind), w| {
        crate::components::button_enabled(ui, a, label, kind, w, false)
            .on_disabled_hover_text(later.clone());
    };
    ui.scope(|ui| {
        ui.multiply_opacity(DIM);
        if panel {
            let w = ui.available_width();
            for each in actions {
                action(ui, a, each, Some(w));
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for each in actions {
                    action(ui, a, each, None);
                }
                action(ui, a, (tr!("Delete…"), ButtonKind::DangerOutline), None);
            });
        }
    });
    if panel {
        divider(ui, &p);
        ui.scope(|ui| {
            ui.multiply_opacity(DIM);
            action(ui, a, (tr!("Delete…"), ButtonKind::DangerOutline), None);
        });
    }
    ui.add(crate::components::para(
        &later,
        12.0,
        Weight::Regular,
        p.muted,
    ));
    // r1 (Should): where it can be done now — one copy of the identity's own model, on its
    // page in the Workshop (W9). Not for another's model.
    if mine {
        ui.add(crate::components::para(
            tr!("You can give or put up for sale one copy on the model's page in the Workshop."),
            12.0,
            Weight::Regular,
            p.muted,
        ));
    }
    trust
}

/// BUG-308: how much of itself a button not there yet shows, on any ground.
const DIM: f32 = 0.45;

/// One copy in a group's card: «№ 12 · from @alice · 13 Nov.» and its price or «not for sale»;
/// in the panel shorter, «№ 12 · @alice» and the price alone. A picture that does not match its
/// hash says so in its own row when the others do.
fn copy_row(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    nft: &Nft,
    (now, all_verified): (chrono::DateTime<chrono::Local>, bool),
    panel: bool,
) {
    let number = nft.edition.0.map(|n| format!("№ {n}")).unwrap_or_default();
    // BUG-309: an issuer without a name is its id — whole where it fits, else shortened in its
    // middle to the row (FEAT-0051 R-5), in the panel too.
    let issuer_id = (matches!(nft.origin, NftOrigin::Release) && is_identity_id(&nft.issuer))
        .then_some(nft.issuer.as_str());
    let left_with = |issuer: &str| {
        let who = match &nft.origin {
            NftOrigin::Gift { from, .. } if !from.is_empty() => {
                if panel {
                    from.clone()
                } else {
                    format!(tr!("from {from}"), from = from)
                }
            }
            NftOrigin::Release => format!(tr!("issue {}"), issuer),
            _ => origin(nft),
        };
        let when = match &nft.origin {
            NftOrigin::Gift { at, .. } => Some(*at),
            NftOrigin::Received { at } => *at,
            NftOrigin::Release => None,
        };
        let mut left = if number.is_empty() {
            crate::i18n::t(&who).into_owned()
        } else {
            format!("{number} · {}", crate::i18n::t(&who))
        };
        if let (false, Some(at)) = (panel, when) {
            left = format!("{left} · {}", text::date_short(at, now));
        }
        left
    };
    let (right, color) = match (nft.price, panel) {
        (Some(d), true) => (crate::format::amount(d), p.accent),
        (Some(d), false) => (
            format!(tr!("for sale · {}"), crate::format::amount(d)),
            p.accent,
        ),
        (None, _) => (tr!("not for sale").into(), p.muted),
    };
    ui.add_space(9.0);
    ui.horizontal(|ui| {
        // BUG-309: the state keeps its words and 12 from them; the row's start is cut with «…».
        let right_w = crate::components::text_width(ui, &right, sans(12.0, Weight::Medium));
        let room = (ui.available_width() - right_w - 12.0).max(0.0);
        let font = crate::theme::figures(12.0, Weight::Regular);
        let whole = left_with(&nft.issuer);
        let left = match issuer_id {
            Some(id) => left_with(&crate::components::fit_id(id, &|cut: &str| {
                crate::components::text_width(ui, &left_with(cut), font.clone()) <= room
            })),
            None => whole.clone(),
        };
        // r1: the whole row under the pointer only when it is not all there — one hint, ours.
        let cut = left != whole || crate::components::text_width(ui, &left, font.clone()) > room;
        ui.allocate_ui_with_layout(
            vec2(room, 16.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_max_width(room);
                let row = ui.add(
                    egui::Label::new(
                        egui::RichText::new(&left)
                            .font(crate::theme::figures(12.0, Weight::Regular))
                            .color(p.text),
                    )
                    .truncate()
                    .show_tooltip_when_elided(false),
                );
                if cut {
                    row.on_hover_text(whole);
                }
            },
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(txt(&right, 12.0, Weight::Medium, color));
        });
    });
    if !all_verified && !nft.art_verified {
        ui.label(txt(
            tr!("the picture does not match"),
            12.0,
            Weight::Medium,
            p.danger,
        ));
    }
    ui.add_space(9.0);
}

/// BUG-309: whether an issuer is an identity's id rather than a name. The core gives the
/// collection owner's id (base58 of 32 bytes: 43 or 44 letters of its alphabet, no «@»); only the
/// demo gives names. r1: an id by what it is — base58's alphabet has no 0, O, I or l, and a DPNS
/// label of 43 such letters with no hyphen is not likely, where «40 letters and digits» was.
fn is_identity_id(issuer: &str) -> bool {
    (43..=44).contains(&issuer.len())
        && issuer
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && !matches!(c, '0' | 'O' | 'I' | 'l'))
}

/// BUG-308 r1: whether `issuer` is the identity the wallet works as — its id, or its name (the
/// demo's issuers are names).
fn is_shown_identitys(wallet: &evolite_api::Wallet, issuer: &str) -> bool {
    wallet.identity.as_ref().is_some_and(|i| {
        i.id == issuer
            || i.name()
                .is_some_and(|n| n == issuer.trim_start_matches('@'))
    })
}

/// The selected NFT. Returns true when "Give" was clicked.
/// "Jembe" from "jembe": the issuer as a name in a sentence.
fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

/// `ask`: the picture waits for consent; `gift_fee`: "Give" is offered, at this fee; `offer`:
/// W10's series windows on this copy, or the card as before.
/// Returns ("Give", "Always show from …", the series window asked for).
fn detail(
    ui: &mut Ui,
    a: &mut Assets,
    nft: &Nft,
    ask: bool,
    (gift_fee, offer): (Option<Duffs>, Offer),
    now: chrono::DateTime<chrono::Local>,
    title: bool,
) -> (bool, bool, Option<CopyAct>) {
    let p = a.p;
    let mut act = None;
    #[cfg_attr(not(demo_core), allow(unused_mut))]
    let mut gift = false;
    #[cfg(not(demo_core))]
    let _ = gift_fee;
    let mut trust = false;
    // BUG-188: the body alone; the panel puts it in a card, the window under its title.
    {
        {
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                if title {
                    ui.label(txt(&nft.name, 20.0, Weight::Bold, p.text));
                }
                let kind = match nft.origin {
                    NftOrigin::Gift { .. } | NftOrigin::Received { .. } => {
                        format!(tr!("Gift {}"), capitalized(&nft.issuer))
                    }
                    NftOrigin::Release => format!(tr!("Issue {}"), nft.issuer),
                };
                ui.label(
                    egui::RichText::new(crate::i18n::t(&match edition(nft) {
                        // r1: no number, no « · ».
                        number if number.is_empty() => kind,
                        number => format!("{kind} · {number}"),
                    }))
                    .font(crate::theme::figures(12.0, Weight::Regular))
                    .color(p.muted),
                );
            });
            ui.add(
                crate::components::para(&nft.description, 13.0, Weight::Regular, p.muted)
                    .line_height(19.5),
            );
            if nft.art_verified {
                chip(
                    ui,
                    a,
                    "confirmed",
                    tr!("The picture matches its hash"),
                    true,
                )
                .on_hover_text(
                    crate::i18n::t(tr!(
                        "The picture's hash is in the NFT document and checked on this computer"
                    ))
                    .into_owned(),
                );
            } else {
                chip(
                    ui,
                    a,
                    "warning",
                    tr!("The picture does not match its hash"),
                    false,
                );
            }
            if ask {
                trust = button(
                    ui,
                    a,
                    &format!(tr!("Always show from {}"), nft.issuer),
                    ButtonKind::Ghost,
                    None,
                )
                .on_hover_text(
                    crate::i18n::t(tr!(
                        "Pictures from this issuer will be drawn without asking"
                    ))
                    .into_owned(),
                )
                .clicked();
            }
            divider(ui, &p);
            let value = sans(13.0, Weight::Medium);
            // W10: «for sale · 0.5000 DASH» in the accent, as the tiles say it. The first row (the
            // designer's check): the buttons under the table are about it, and at 800 × 440 the
            // rows below scroll.
            match (nft.price, &offer) {
                (Some(d), Offer::Series(_)) => crate::components::kv_row_in(
                    ui,
                    &p,
                    tr!("Price"),
                    &format!(tr!("for sale · {}"), crate::format::amount(d)),
                    value.clone(),
                    p.accent,
                ),
                (price, _) => {
                    let price = match price {
                        Some(d) => crate::format::amount(d),
                        None => tr!("not for sale").into(),
                    };
                    kv_row(ui, &p, tr!("Price"), &price, value.clone());
                }
            }
            kv_row(ui, &p, tr!("Owner"), &nft.owner, value.clone());
            match &nft.origin {
                NftOrigin::Gift { from, at } => {
                    let line = text::with_date(from, &text::date_short(*at, now));
                    kv_row(ui, &p, tr!("Gift from"), &line, value.clone());
                }
                // Platform may not keep when it changed hands: then no date is made up.
                NftOrigin::Received { at } => {
                    kv_row(
                        ui,
                        &p,
                        tr!("Obtained"),
                        &at.map_or(tr!("as a gift").into(), |at| text::date_short(at, now)),
                        value.clone(),
                    );
                }
                NftOrigin::Release => {}
            }
            kv_row(ui, &p, tr!("Issued by"), &nft.issuer, value.clone());
            kv_row(
                ui,
                &p,
                tr!("Contract"),
                &nft.contract,
                mono(13.0, Weight::Medium),
            );
            // In the panel under the card; the window has them in its footer (BUG-122: never
            // scrolled away).
            if let (Offer::Series(why), true) = (offer, title) {
                act = copy_actions(ui, a, nft, why, title);
            }
            // "Give" and "Sell": the demo's, as in Figma — on a copy W10 does not cover.
            #[cfg(demo_core)]
            if offer == Offer::Before {
                divider(ui, &p);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let w = (ui.available_width() - 8.0) / 2.0;
                    gift = button_enabled(
                        ui,
                        a,
                        tr!("Give"),
                        ButtonKind::Primary,
                        Some(w),
                        gift_fee.is_some(),
                    )
                    .on_disabled_hover_text(crate::i18n::t(TRANSFER_LATER).into_owned())
                    .clicked();
                    button_enabled(ui, a, tr!("Sell"), ButtonKind::Secondary, Some(w), false)
                        .on_disabled_hover_text(crate::i18n::t(SELL_LATER).into_owned());
                });
                if let Some(fee) = gift_fee {
                    ui.add(crate::components::para(
                        &format!(
                            tr!("Transfer ≈ {} · a gift cannot be burned"),
                            crate::format::amount(fee)
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
            }
        }
    }
    (gift, trust, act)
}

/// W10 (the frame W10 on BUG-188): «Give…», then «Change the price…» and «Take off sale…» for a
/// copy for sale or «Sell…»; the rules of a direct purchase under them (AC-A11). `why`: they
/// are shown, not active, with why. `panel`: one under another, the panel's width.
fn copy_actions(
    ui: &mut Ui,
    a: &mut Assets,
    nft: &Nft,
    why: Option<&'static str>,
    panel: bool,
) -> Option<CopyAct> {
    let p = a.p;
    let mut buttons = vec![(tr!("Give…"), ButtonKind::Primary, CopyAct::Give)];
    if nft.price.is_some() {
        buttons.push((
            tr!("Change the price…"),
            ButtonKind::Secondary,
            CopyAct::ChangePrice,
        ));
        buttons.push((
            tr!("Take off sale…"),
            ButtonKind::Secondary,
            CopyAct::TakeOff,
        ));
    } else {
        buttons.push((tr!("Sell…"), ButtonKind::Secondary, CopyAct::Sell));
    }
    let mut act = None;
    let mut press = |ui: &mut Ui, a: &mut Assets, (label, kind, what), width| {
        let pressed = crate::components::button_enabled(ui, a, label, kind, width, why.is_none());
        let pressed = match why {
            Some(why) => pressed.on_disabled_hover_text(crate::i18n::t(why).into_owned()),
            None => pressed,
        };
        if pressed.clicked() {
            act = Some(what);
        }
    };
    if panel {
        let width = ui.available_width();
        for each in buttons {
            press(ui, a, each, Some(width));
        }
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for each in buttons {
                press(ui, a, each, None);
            }
        });
    }
    ui.add(crate::components::para(
        tr!(
            "Anyone can buy it — at once, without your confirmation. A gift takes the price off; after a sale the new owner sets the price."
        ),
        12.0,
        Weight::Regular,
        p.muted,
    ));
    act
}

/// "Give": the recipient's Dash name and the fee; the core does the transfer (the demo's).
#[cfg(demo_core)]
fn gift_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut CollectionState) {
    let p = a.p;
    let wallet = cx.wallet();
    let Some(nft) = wallet.nfts.get(state.selected) else {
        state.gifting = false;
        return;
    };
    let fee = wallet.fees.nft_transfer;
    let title = format!(tr!("Give “{}”"), crate::i18n::own(&nft.name));
    // TECH-paid-once-gaps (BUG-274's rule): given — its answer and «Done», no second «Give» of
    // the same NFT.
    let given = matches!(state.result, Some(Ok(())));
    let closed = dialog_with_summary(ui, a, "gift-dialog", "nft", &title, 464.0, |ui, a, part| {
        let mut close = false;
        if given {
            return match part {
                Part::Summary => false,
                Part::Footer => {
                    crate::components::button_row(ui, |ui| {
                        close = button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked();
                    });
                    close
                }
                Part::Middle => {
                    crate::components::notice(
                        ui,
                        a,
                        "confirmed",
                        p.accent,
                        tr!("NFT transferred."),
                    );
                    false
                }
            };
        }
        let name = state
            .recipient
            .trim()
            .trim_start_matches('@')
            .to_lowercase();
        let error = if state.recipient.trim().is_empty() {
            None
        } else if !is_dash_name(&state.recipient) {
            Some(tr!(
                "This is not a Dash name: 3–63 letters, digits and hyphens"
            ))
        } else if name == nft.owner.trim_start_matches('@') {
            Some(tr!("This NFT is already yours"))
        } else {
            None
        };
        // BUG-249: what is agreed to, pinned over the password: that the NFT cannot be taken
        // back, and the fee.
        if part == Part::Summary {
            crate::components::notice(
                ui,
                a,
                "info",
                p.muted,
                tr!("The NFT goes to the recipient's identity. It cannot be taken back."),
            );
            kv_row(
                ui,
                &p,
                tr!("Platform fee"),
                &format!("≈ {}", crate::format::amount(fee)),
                mono(13.0, Weight::Medium),
            );
            // FEAT-0004 FR-7: the large-spend tick with what it agrees to.
            state.flow.password.summary_tick(ui, &p);
            return false;
        }
        if part == Part::Footer {
            // The footer (BUG-122): always in the window.
            let ask = password_asked(cx.snap);
            let entered = state.flow.password.field(ui, &p, "gift-password", ask);
            crate::components::button_row_spend(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                let ready = !state.recipient.trim().is_empty()
                    && error.is_none()
                    && state.flow.can_act()
                    && state.flow.password.ready(ask);
                if button_enabled(ui, a, tr!("Give"), ButtonKind::Primary, None, ready).clicked()
                    || (entered && ready)
                {
                    let op = Spend::GiftNft {
                        id: nft.id.clone(),
                        to: state.recipient.trim().to_string(),
                    };
                    if let Some(command) = state.flow.command(op, ask, ()) {
                        cx.send(command);
                        state.result = None;
                    }
                }
            });
            match state.result.as_ref() {
                Some(Ok(())) => {
                    crate::components::notice(ui, a, "confirmed", p.accent, tr!("NFT transferred."))
                }
                Some(Err(CoreError::NotAvailable(why))) => crate::components::notice(
                    ui,
                    a,
                    "info",
                    p.muted,
                    &format!(tr!("{why}. The NFT is still yours."), why = why),
                ),
                Some(Err(e)) => crate::components::error_notice(
                    ui,
                    a,
                    &crate::text::failed(tr!("Not transferred"), e),
                    crate::text::error_details(e).as_deref(),
                ),
                None => {}
            }
            return close;
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field_label(ui, &p, tr!("Recipient"));
            text_field(
                ui,
                &p,
                "gift-recipient",
                &mut state.recipient,
                "@name",
                FieldKind::Text,
                |_| {},
            );
            match error {
                Some(e) => ui.label(txt(e, 12.0, Weight::Regular, p.danger)),
                None => ui.label(txt(
                    tr!("The recipient's name in Dash, e.g. @alice"),
                    12.0,
                    Weight::Regular,
                    p.muted,
                )),
            };
        });
        false
    });
    if closed {
        state.gifting = false;
        state.recipient.clear();
        state.result = None;
        state.flow.forget();
    }
}

/// Under "Could not load the NFTs": the core's reason and that it asks again. One template:
/// the catalog translates the reason in its place and ends the sentence as its language does.
fn failed_detail(why: &str) -> String {
    format!(
        tr!("{}. evolite asks Dash Platform again every minute."),
        why.trim_end_matches('.')
    )
}

impl crate::screen_state::ScreenState for CollectionState {
    #[cfg(all(test, demo_core, feature = "dev-tools"))]
    fn flows(&mut self) -> Vec<&mut dyn crate::components::spend_flow::AnyFlow> {
        vec![&mut self.flow]
    }

    fn forget(&mut self, why: crate::screen_state::Why) {
        #[cfg(demo_core)]
        self.flow.forget();
        if why == crate::screen_state::Why::WalletSwitch {
            self.forget_wallet();
        }
    }
}

/// «You have 3 · 1 for sale at 0.5000 DASH · 2 not for sale» of `n` copies at `prices` (those
/// for sale). Batch 74 (BUG-307's class): each piece in the interface language before they are
/// joined — the line translated whole was split at « · », and «1 for sale at …» was taken by
/// another key's template and stayed half English.
pub(super) fn copies_summary(n: usize, prices: &[Duffs]) -> String {
    let lang = crate::i18n::current();
    let piece = |s: String| crate::i18n::t_in(lang, &s).into_owned();
    let mut parts = vec![piece(text::plural_in(
        n as u32,
        tr!("You have {}"),
        tr!("You have {}"),
    ))];
    if let Some(low) = prices.iter().min() {
        let sale = text::plural_in(
            prices.len() as u32,
            tr!("{} for sale at {}"),
            tr!("{} for sale at {}"),
        )
        .replacen("{}", &crate::format::amount(*low), 1);
        parts.push(piece(sale));
    }
    let unsold = n.saturating_sub(prices.len()) as u32;
    if unsold > 0 {
        parts.push(piece(text::plural_in(
            unsold,
            tr!("{} not for sale"),
            tr!("{} not for sale"),
        )));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::{
        copies_summary, failed_detail, groups, is_dash_name, numbers, third_line, tile_grid,
    };
    use evolite_api::{Nft, NftOrigin};

    /// BUG-188 §2: the designer's table — 800 (676): 4 × 160, 1024 (868): 5 × 164, 1280 with
    /// the panel (628): 4 × 148, 1440 (788): 5 × 148, 1600 and wider (876): 5 × 166. At most 8
    /// columns; a tile never wider than 196 while a column can be added.
    #[test]
    fn the_grid_follows_the_designers_table() {
        for (width, cols, tile) in [
            (676.0, 4, 160.0),
            (868.0, 5, 164.0),
            (628.0, 4, 148.0),
            (788.0, 5, 148.0),
            (876.0, 5, 166.0),
        ] {
            let (c, w) = tile_grid(width);
            assert_eq!(c, cols, "{width}");
            assert!((w - tile).abs() < 0.5, "{width}: {w}");
        }
        assert_eq!(tile_grid(5000.0).0, 8, "no more than 8 columns");
        // One column of 300 would be wider than 196: a second, though narrower than 148.
        assert_eq!(tile_grid(300.0).0, 2);
        for width in (300..1200).step_by(7) {
            let (cols, w) = tile_grid(width as f32);
            assert!(w <= 196.0 || cols == 8, "{width}: {cols} × {w}");
        }
    }

    fn nft(id: &str, art: &str, edition: u32, price: Option<i64>) -> Nft {
        Nft {
            id: id.into(),
            name: art.into(),
            description: String::new(),
            art: art.into(),
            art_verified: true,
            origin: NftOrigin::Release,
            issuer: "jembe".into(),
            contract: "jembe-nft".into(),
            model: None,
            edition: (Some(edition), 500),
            owner: "@me".into(),
            owner_id: "me".into(),
            chosen: false,
            price,
        }
    }

    /// FEAT-0038 W10: the series windows on a copy of a Workshop model with a number, held alone
    /// by its identity; inactive with why when the identity holds another copy of that model
    /// (the core would take the lowest free one, maybe not this) or runs a series; the card as
    /// before for a copy with no model or no number. Mutant: the other copies not counted → red.
    #[test]
    fn the_series_windows_are_offered_on_a_copy_held_alone() {
        use super::{Offer, SERIES_LATER, offer};
        use evolite_api::series::{SeriesKind, SeriesState, SeriesTarget};
        let of_model = |id: &str, owner: &str| Nft {
            model: Some("pin".into()),
            owner_id: owner.into(),
            ..nft(id, "pin", 1, Some(50_000_000))
        };
        let one = of_model("a", "me");
        assert_eq!(
            offer(&one, std::slice::from_ref(&one), &[]),
            Offer::Series(None)
        );
        // Another identity's copy of the model is not this one's.
        let theirs = of_model("b", "other");
        assert_eq!(
            offer(&one, &[one.clone(), theirs], &[]),
            Offer::Series(None)
        );
        let second = of_model("c", "me");
        assert_eq!(
            offer(&one, &[one.clone(), second], &[]),
            Offer::Series(Some(SERIES_LATER))
        );
        let running = SeriesState {
            id: 1,
            target: SeriesTarget {
                identity: "me".into(),
                contract: "jembe-nft".into(),
                model: "other-model".into(),
            },
            kind: SeriesKind::Mint { count: 3 },
            total: 3,
            done: 1,
            refused: 0,
            unclear: 0,
            running: true,
            stopping: false,
            per_write: 1,
        };
        assert!(matches!(
            offer(&one, std::slice::from_ref(&one), &[running]),
            Offer::Series(Some(why)) if why.starts_with("A series is running")
        ));
        let no_model = nft("d", "jembe", 1, None);
        assert_eq!(
            offer(&no_model, std::slice::from_ref(&no_model), &[]),
            Offer::Before
        );
        let no_number = Nft {
            edition: (None, 0),
            ..one.clone()
        };
        assert_eq!(
            offer(&no_number, std::slice::from_ref(&no_number), &[]),
            Offer::Before
        );
    }

    /// BUG-188 §4: copies of one contract and one picture are one group, where its first copy
    /// stands; another contract with the same picture is another model.
    #[test]
    fn copies_of_one_model_are_one_group_at_its_first_copy() {
        let mut other = nft("o", "drum", 9, None);
        other.contract = "neon-forge".into();
        let nfts = vec![
            nft("a", "drum", 12, None),
            nft("b", "lock", 3, None),
            nft("c", "drum", 40, Some(5)),
            other,
            nft("d", "drum", 77, None),
        ];
        let all: Vec<usize> = (0..nfts.len()).collect();
        assert_eq!(groups(&nfts, &all), vec![vec![0, 2, 4], vec![1], vec![3]]);
        // A filter groups what it shows.
        assert_eq!(groups(&nfts, &[1, 2]), vec![vec![1], vec![2]]);
    }

    /// BUG-188 §3: a group's numbers — a list, a run, one copy's edition.
    #[test]
    fn a_groups_numbers_are_a_list_or_a_run() {
        let nfts: Vec<Nft> = [12, 77, 40, 41, 42, 43, 3]
            .iter()
            .map(|&n| nft(&n.to_string(), "x", n, None))
            .collect();
        assert_eq!(numbers(&nfts, &[0, 2, 1]), "№ 12 · 40 · 77");
        assert_eq!(numbers(&nfts, &[2, 3, 4, 5]), "№ 40–43");
        assert_eq!(numbers(&nfts, &[2, 3]), "№ 40 · 41", "two are a list");
        assert_eq!(numbers(&nfts, &[6]), "No. 3 of 500");
    }

    /// BUG-188 §3: a tile's third line — «1 for sale of 3» in the accent while a copy is for
    /// sale (in Russian «продаётся 1 из 3», «продаются 2 из 4»), else where the first came from.
    #[test]
    fn a_tiles_third_line_says_how_many_are_for_sale() {
        let nfts = vec![
            nft("a", "x", 1, None),
            nft("b", "x", 2, Some(5)),
            nft("c", "x", 3, Some(5)),
            nft("d", "x", 4, None),
        ];
        assert_eq!(
            third_line(&nfts, &[0, 1, 3]),
            ("1 for sale of 3".into(), true)
        );
        assert_eq!(
            third_line(&nfts, &[0, 1, 2, 3]),
            ("2 for sale of 4".into(), true)
        );
        assert_eq!(third_line(&nfts, &[0, 3]), ("issue jembe".into(), false));
        let ru = |text: &str| crate::i18n::t_in(crate::i18n::Lang::Ru, text).into_owned();
        assert_eq!(ru("1 for sale of 3"), "продаётся 1 из 3");
        assert_eq!(ru("2 for sale of 4"), "продаются 2 из 4");
        assert_eq!(
            ru("You have 3 · 1 for sale at 0.5000 DASH · 2 not for sale"),
            "У вас 3 · продаётся 1 по 0.5000 DASH · не продаются 2"
        );
        // Batch 74: the screen's line, each piece in Russian before they are joined.
        crate::i18n::set(crate::i18n::Lang::Ru);
        let line = copies_summary(3, &[50_000_000]);
        // The amount as Russian writes it, with its decimal comma.
        let price = crate::format::amount(50_000_000);
        crate::i18n::set(crate::i18n::Lang::En);
        assert_eq!(
            line,
            format!("У вас 3 · продаётся 1 по {price} · не продаются 2")
        );
    }

    /// BUG-10: a failed list says why and that it is asked again.
    #[test]
    fn a_failed_list_says_why_and_that_it_comes_again() {
        let text = failed_detail("Dash Platform did not answer within a minute");
        assert!(text.contains("did not answer"), "{text}");
        assert!(text.contains("again every minute"), "{text}");
        // Both halves in the language, the stop as its catalog writes it (BUG-15).
        let ru = crate::i18n::t_in(crate::i18n::Lang::Ru, &text);
        assert_eq!(
            ru,
            "Dash Platform не ответила за минуту. evolite спрашивает Dash Platform снова каждую минуту."
        );
        assert_eq!(failed_detail("No answer."), failed_detail("No answer"));
    }

    /// Design review №24: «Кольцо с бриллиантом» in the narrowest tile (160 px) is whole on two
    /// rows; one row, as before, cut it.
    #[test]
    fn a_tiles_name_takes_two_rows_before_it_is_cut() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::i18n::Lang::Ru, false);
        let name = "Кольцо с бриллиантом";
        let (mut two, mut one) = (None, None);
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let color = egui::Color32::WHITE;
                let job = super::tile_name(ui.ctx(), name, 160.0 - 28.0, color);
                let mut cut = job.clone();
                cut.wrap = egui::text::TextWrapping::truncate_at_width(160.0 - 28.0);
                two = Some(ui.painter().layout_job(job));
                one = Some(ui.painter().layout_job(cut));
            });
            out.textures_delta.clear();
        }
        let (two, one) = (two.unwrap(), one.unwrap());
        assert!(one.elided, "one row cuts it");
        assert!(!two.elided, "two rows hold it");
        assert_eq!(two.rows.len(), 2);
        assert!(two.size().y <= 2.0 * super::NAME_ROW, "{}", two.size().y);
    }

    /// BUG-118 (4): an Arabic name too long for a tile's two rows keeps its start: the second
    /// row ends with «…» after whole words, in reading order. egui's cut ended the second row at
    /// its left — the start of that row's words in Arabic.
    #[test]
    fn a_long_arabic_tiles_name_keeps_its_start_on_two_rows() {
        const NAME: &str = "خاتم ذهبي مرصع بالألماس من مجموعة الربيع النادرة جدا";
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::i18n::Lang::Ar, false);
        crate::i18n::set(crate::i18n::Lang::Ar);
        let mut laid = None;
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let job = super::tile_name(ui.ctx(), NAME, 160.0 - 28.0, egui::Color32::WHITE);
                laid = Some(ui.painter().layout_job(job));
            });
            out.textures_delta.clear();
        }
        crate::i18n::set(crate::i18n::Lang::En);
        let laid = laid.unwrap();
        let text = &laid.job.text;
        let rows: Vec<&str> = text.split('\n').collect();
        assert_eq!(rows.len(), 2, "{text:?}");
        assert!(rows[0].contains("خاتم"), "the first word on top: {text:?}");
        assert!(
            rows[1].contains('…') && !text.contains("جدا"),
            "the second row cut after whole words: {text:?}"
        );
        assert!(!laid.elided, "nothing left for egui to cut: {text:?}");
    }

    /// BUG-282 (the class): every picture the Collection paints says whether it may be drawn,
    /// computed from the consent check — the tile's and the group card's alike; none passes
    /// «given» as a constant. Mutant: the group card's paint without it is red.
    #[test]
    fn every_picture_painted_says_its_consent() {
        let file = include_str!("collection.rs");
        let body = &file[..file.find("#[cfg(test)]").unwrap_or(file.len())];
        let calls: Vec<&str> = body
            .match_indices("art_cache.paint(")
            .map(|(at, _)| &body[at..at + body[at..].find(");").unwrap_or(0)])
            .collect();
        assert_eq!(calls.len(), 2, "the tile and the group card");
        for call in &calls {
            assert!(call.contains("Consent::waits("), "{call}");
        }
        assert!(
            !body.contains("Consent::Given") && !body.contains("Consent::Own"),
            "no constant consent"
        );
        // r1 (Should): every other picture of the app says its consent too — the Workshop's
        // are the person's own, `Own`; `paint_or_tell` as `paint`.
        for (file, text) in [
            (
                "workshop/model_page.rs",
                include_str!("workshop/model_page.rs"),
            ),
            (
                "workshop/collection_page.rs",
                include_str!("workshop/collection_page.rs"),
            ),
            (
                "workshop/new_model.rs",
                include_str!("workshop/new_model.rs"),
            ),
        ] {
            for (at, _) in text
                .match_indices(".paint(ui")
                .chain(text.match_indices(".paint_or_tell("))
            {
                let call = &text[at..at + text[at..].find(";").unwrap_or(0)];
                assert!(call.contains("Consent::"), "{file}: {call}");
            }
        }
    }

    #[test]
    fn a_strangers_picture_waits_until_asked_for() {
        use evolite_api::{Nft, NftOrigin};
        let nft = |issuer: &str| Nft {
            id: format!("nft-{issuer}"),
            name: String::new(),
            description: String::new(),
            art: String::new(),
            art_verified: true,
            origin: NftOrigin::Release,
            issuer: issuer.into(),
            contract: String::new(),
            model: None,
            edition: (Some(1), 1),
            owner: String::new(),
            owner_id: String::new(),
            chosen: false,
            price: None,
        };
        let mut state = super::CollectionState {
            ask_unknown: true,
            ..Default::default()
        };
        // The demo's partner issuer is trusted by the demo core only (FEAT-0054 r1); the real
        // core trusts no issuer out of the box.
        assert_eq!(
            state.waits_for_consent(&nft("jembe"), &[]),
            !cfg!(demo_core)
        );
        let stranger = nft("neon-forge");
        assert!(state.waits_for_consent(&stranger, &[]));
        // "Show" on this one.
        state.shown.insert(stranger.id.clone());
        assert!(!state.waits_for_consent(&stranger, &[]));
        // "Always show from neon-forge".
        let other = Nft {
            id: "nft-other".into(),
            ..nft("neon-forge")
        };
        assert!(state.waits_for_consent(&other, &[]));
        state.trusted.insert("neon-forge".into());
        assert!(!state.waits_for_consent(&other, &[]));
        // FEAT-0054 r1: a source the person chose (own collection, added by id, a purchase) is
        // drawn at once; a transfer from an issuer not chosen waits; an issuer the wallet keeps
        // as trusted is drawn, after a restart too (it comes in the snapshot).
        let mine = Nft {
            chosen: true,
            ..nft("my-own-collection")
        };
        assert!(!state.waits_for_consent(&mine, &[]));
        let sent = nft("someone-else");
        assert!(state.waits_for_consent(&sent, &[]));
        assert!(!state.waits_for_consent(&sent, &["someone-else".to_string()]));
        // Settings: do not ask.
        state.ask_unknown = false;
        assert!(!state.waits_for_consent(&nft("anyone"), &[]));
    }

    #[test]
    fn checks_dash_names() {
        for good in ["@alice", "bob", "nakamoto-dev", "a1b"] {
            assert!(is_dash_name(good), "{good}");
        }
        for bad in ["", "@a", "ab", "-ab", "ab-", "a--b", "名前", "a b c"] {
            assert!(!is_dash_name(bad), "{bad}");
        }
    }
}
