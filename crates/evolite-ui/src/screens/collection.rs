//! 18 · Collection: the identity's NFTs as the core found them on Platform (Jembe's gifts),
//! with filters and the selected NFT on the right. Nothing is shown that is not there: an
//! empty or still loading list says so. Only the demo core can "Give" (the dialog), beside an
//! inactive "Sell" and "Get a gift" as in Figma; the release, without the demo core, shows none
//! of the three until the real core can do them.

use super::txt;
use crate::art::ArtCache;
use crate::components::{
    Assets, ButtonKind, GridSpec, button, card, chip, divider, inner, kv_row, page_header, pill,
};
#[cfg(demo_core)]
use crate::components::{
    FieldKind, button_enabled, dialog, field_label, password_asked, text_field,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::{NFT_MIN_W, Weight, mono, sans};
use egui::{
    Align, Align2, CornerRadius, CursorIcon, Layout, Margin, Rect, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
#[cfg(demo_core)]
use evolite_api::Spend;
use evolite_api::{Command, CoreError, Duffs, Nft, NftOrigin, NftsStatus};
use std::collections::HashSet;

const FILTERS: [&str; 3] = [tr!("All"), tr!("Jembe · gifts"), tr!("For sale")];

/// Hints on the demo's buttons that are not there yet.
#[cfg(demo_core)]
const GIFT_LATER: &str = tr!("Receiving a gift by link comes in the next version");
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
            ) && nft.issuer == "jembe"
        }
        2 => nft.price.is_some(),
        _ => true,
    }
}

#[derive(Default)]
pub struct CollectionState {
    pub filter: usize,
    pub selected: usize,
    /// The gift dialog is open for the selected NFT.
    pub gifting: bool,
    #[cfg(demo_core)]
    pub recipient: String,
    /// "Password for every payment": the transfer costs credits (BUG-42).
    #[cfg(demo_core)]
    pub password: crate::components::SpendPassword,
    /// The core's answer to the last "Give".
    pub result: Option<Result<(), CoreError>>,
    /// The pictures, drawn by the core at the size of the cards.
    pub art: ArtCache,
    /// Settings → Collection: a picture from an issuer not trusted waits for "Show".
    pub ask_unknown: bool,
    /// Issuers whose pictures are drawn without asking ("Always show from …").
    pub trusted: HashSet<String>,
    /// NFTs whose picture the person asked to see.
    pub shown: HashSet<String>,
}

/// Issuers trusted out of the box: the partner app whose gifts the wallet is made for.
const TRUSTED: [&str; 1] = ["jembe"];

impl CollectionState {
    /// Another wallet opens: the gift dialog, its answer and the pictures asked for go; the
    /// drawn pictures and the settings stay (BUG-45).
    pub fn forget_wallet(&mut self) {
        self.selected = 0;
        self.gifting = false;
        #[cfg(demo_core)]
        self.recipient.clear();
        self.result = None;
        self.shown.clear();
    }

    /// The picture of `nft` is not drawn until the person asks: an issuer nobody trusted may
    /// send anything, and drawing it is the first thing that could go wrong.
    pub fn waits_for_consent(&self, nft: &Nft) -> bool {
        self.ask_unknown
            && !TRUSTED.contains(&nft.issuer.as_str())
            && !self.trusted.contains(&nft.issuer)
            && !self.shown.contains(&nft.id)
    }
}

/// DPNS label: 3–63 letters, digits and inner hyphens, with an optional "@".
#[cfg(any(demo_core, test))]
pub fn is_dash_name(input: &str) -> bool {
    let label = input.trim().trim_start_matches('@');
    (3..=63).contains(&label.len())
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !label.starts_with('-')
        && !label.ends_with('-')
        && !label.contains("--")
}

pub fn collection(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut CollectionState) {
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
    #[cfg(demo_core)]
    page_header(ui, a, tr!("Collection"), count.as_deref(), |ui, a| {
        button_enabled(ui, a, tr!("Get a gift"), ButtonKind::Secondary, None, false)
            .on_disabled_hover_text(crate::i18n::t(GIFT_LATER).into_owned());
    });
    #[cfg(not(demo_core))]
    page_header(ui, a, tr!("Collection"), count.as_deref(), |_, _| {});
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
    if nfts.is_empty() {
        if empty(ui, a, &wallet.nfts_status) {
            cx.send(Command::RefreshNfts);
        }
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (i, label) in FILTERS.iter().enumerate() {
            if pill(ui, &p, label, state.filter == i).clicked() {
                state.filter = i;
            }
        }
    });
    // The saved list is still here and the core asks again by itself: not an error (calm
    // states CS-1); «Try again» asks now (CS-1a).
    if let NftsStatus::Failed(why) = &wallet.nfts_status {
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
        .collect();
    // The panel follows the filter: a hidden NFT gives way to the first one shown.
    if !shown.contains(&state.selected)
        && let Some(first) = shown.first()
    {
        state.selected = *first;
    }

    let mut asked = Vec::new();
    let asks: Vec<bool> = nfts.iter().map(|n| state.waits_for_consent(n)).collect();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let total = ui.available_width();
        let side = total >= 768.0;
        let detail_w = if side { 300.0 } else { total };
        let grid_w = if side { total - detail_w - 24.0 } else { total };
        // Read before the grid: a click there shows in the panel on the next frame.
        let selected = (!shown.is_empty()).then_some(state.selected);
        let mut grid = |ui: &mut Ui, a: &mut Assets| {
            ui.set_width(grid_w);
            ui.spacing_mut().item_spacing.y = 16.0;
            if shown.is_empty() {
                card(&p, Margin::symmetric(20, 32)).show(ui, |ui| {
                    ui.set_width(inner(grid_w, 20));
                    ui.vertical_centered(|ui| {
                        ui.label(txt(tr!("Nothing with this filter"), 14.0, Weight::Regular, p.muted))
                    });
                });
            } else {
                // As many columns of at least an NFT card (195) as fit, up to 6 (responsive
                // spec R4). The cards are of one size: nothing to paint at the row's height.
                let spec = GridSpec {
                    min_w: NFT_MIN_W,
                    gap: vec2(16.0, 16.0),
                    max_cols: 6,
                    even: false,
                };
                crate::components::grid(ui, shown.len(), spec, |ui, k, card_w, _| {
                    let i = shown[k];
                    let (card, show) = nft_card(
                        ui,
                        a,
                        &mut state.art,
                        &mut asked,
                        &nfts[i],
                        (i == state.selected, asks[i]),
                        card_w,
                    );
                    if show {
                        state.shown.insert(nfts[i].id.clone());
                    } else if card.clicked() {
                        state.selected = i;
                    }
                });
            }
            crate::components::notice(
                ui,
                a,
                "info",
                p.muted,
                tr!("Jembe NFTs from Dash Platform: the picture is stored in the document itself — no outside servers — and checked against its hash on this computer."),
            );
        };
        if side {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 24.0;
                ui.vertical(|ui| grid(ui, a));
                if let Some(i) = selected {
                    ui.vertical(|ui| {
                        let (gift, trust) = detail(ui, a, &nfts[i], asks[i], gift_fee, now, detail_w);
                        state.gifting |= gift;
                        if trust {
                            state.trusted.insert(nfts[i].issuer.clone());
                        }
                    });
                }
            });
        } else {
            ui.spacing_mut().item_spacing.y = 24.0;
            grid(ui, a);
            if let Some(i) = selected {
                let (gift, trust) = detail(ui, a, &nfts[i], asks[i], gift_fee, now, detail_w);
                state.gifting |= gift;
                if trust {
                    state.trusted.insert(nfts[i].issuer.clone());
                }
            }
        }
    });
    for command in asked {
        cx.send(command);
    }

    #[cfg(demo_core)]
    if state.gifting {
        gift_dialog(ui, a, cx, state);
    }
}

fn edition(nft: &Nft) -> String {
    match nft.edition {
        // The issuer set no edition size.
        (n, 0) => format!("№ {n}"),
        (n, of) => format!(tr!("No. {n} of {of}"), n = n, of = of),
    }
}

/// No NFTs: why, as the core says, instead of an empty grid. After a failed load, «Try again»;
/// `true` when it was clicked (CS-1a).
fn empty(ui: &mut Ui, a: &mut Assets, status: &NftsStatus) -> bool {
    let mut again = false;
    let p = a.p;
    let (title, detail) = match status {
        NftsStatus::Loading => (tr!("Loading NFTs from Dash Platform…"), String::new()),
        NftsStatus::NoIdentity => (
            tr!("NFTs belong to an identity"),
            tr!("Make an identity — Jembe gifts will come to it.").into(),
        ),
        // The core asks again by itself (BUG-10): the reason and that, not an endless wait.
        NftsStatus::Failed(why) => (tr!("Could not load the NFTs"), failed_detail(why)),
        NftsStatus::Ready => (
            tr!("You have no NFTs yet"),
            tr!("Jembe gifts appear here as soon as someone sends them to your identity.").into(),
        ),
    };
    let width = ui.available_width();
    card(&p, Margin::symmetric(20, 32)).show(ui, |ui| {
        ui.set_width(inner(width, 20));
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(txt(title, 15.0, Weight::SemiBold, p.text));
            if !detail.is_empty() {
                ui.add(egui::Label::new(txt(&detail, 13.0, Weight::Regular, p.muted)).wrap());
            }
            if matches!(status, NftsStatus::Failed(_)) {
                again = button(ui, a, tr!("Try again"), ButtonKind::Secondary, None).clicked();
            }
        });
    });
    again
}

/// A tile: art (square), name 15 SemiBold, edition and origin. Selected — 1.5 px ink border.
/// `(selected, ask)`: `ask` — the picture waits for "Show"; the second value says it was
/// clicked.
fn nft_card(
    ui: &mut Ui,
    a: &mut Assets,
    art_cache: &mut ArtCache,
    asked: &mut Vec<Command>,
    nft: &Nft,
    (selected, ask): (bool, bool),
    width: f32,
) -> (egui::Response, bool) {
    let p = a.p;
    let art = width - 20.0;
    // Room for two rows of the name (design review №24), the edition under them.
    let (rect, resp) = ui.allocate_exact_size(
        vec2(width, 10.0 + art + 10.0 + 2.0 * NAME_ROW + 17.0 + 14.0),
        Sense::click(),
    );
    crate::components::named_choice(&resp, egui::WidgetType::Button, selected, &nft.name);
    let resp = resp.on_hover_text(crate::i18n::t(&nft.name).into_owned());
    let stroke = if selected {
        Stroke::new(1.5, p.ink)
    } else if resp.hovered() {
        Stroke::new(1.0, p.muted)
    } else {
        Stroke::new(1.0, p.line)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(18),
        p.surface,
        stroke,
        StrokeKind::Inside,
    );
    let art_rect = Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(art, art));
    let mut show = false;
    if ask {
        // Nothing is drawn, not even in the sandbox, until the person asks.
        ui.painter()
            .rect_filled(art_rect, CornerRadius::same(12), p.raised);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(art_rect.shrink(16.0))
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
        art_cache.paint(ui, &p, &nft.art, art_rect, &mut |c| asked.push(c));
    }
    let x = rect.left() + 14.0;
    let y = art_rect.bottom() + 10.0;
    let painter = ui.painter();
    let name = painter.layout_job(tile_name(&nft.name, rect.width() - 28.0, p.text));
    let edition_y = y + name.size().y + 2.0;
    painter.galley(pos2(x, y), name, p.text);
    let edition = painter.text(
        pos2(x, edition_y),
        Align2::LEFT_TOP,
        crate::i18n::t(&edition(nft)),
        mono(12.0, Weight::Medium),
        p.muted,
    );
    let origin = match &nft.origin {
        // A gift from an address with no name: "from " alone would stay untranslated.
        NftOrigin::Gift { from, .. } if from.is_empty() => tr!("from an unknown address").into(),
        NftOrigin::Gift { from, .. } => format!(tr!("from {from}"), from = from),
        NftOrigin::Received { .. } => tr!("received").into(),
        NftOrigin::Release => format!(tr!("issue {}"), nft.issuer),
    };
    // The origin takes what the edition leaves and ends in "…" there: "No. 40 of 1000"
    // and "issue jembe" never run into each other.
    let room = (rect.right() - 14.0 - edition.right() - 10.0).max(0.0);
    let mut job = egui::text::LayoutJob::single_section(
        crate::i18n::t(&origin).into_owned(),
        egui::TextFormat::simple(sans(12.0, Weight::Regular), p.muted),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let origin = painter.layout_job(job);
    painter.galley(
        pos2(rect.right() - 14.0 - origin.size().x, edition_y),
        origin,
        p.muted,
    );
    (resp.on_hover_cursor(CursorIcon::PointingHand), show)
}

/// One row of a tile's name, 15 SemiBold.
const NAME_ROW: f32 = 20.0;

/// A tile's name: two rows at most, then «…» (design review №24: one row cut «Кольцо с
/// брилли…»); the whole of it is also on hover and in the details.
fn tile_name(name: &str, width: f32, color: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::single_section(
        crate::i18n::t(name).into_owned(),
        egui::TextFormat::simple(sans(15.0, Weight::SemiBold), color),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: width,
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    job
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

/// `ask`: the picture waits for consent; `gift_fee`: "Give" is offered, at this fee.
/// Returns ("Give", "Always show from …").
fn detail(
    ui: &mut Ui,
    a: &mut Assets,
    nft: &Nft,
    ask: bool,
    gift_fee: Option<Duffs>,
    now: chrono::DateTime<chrono::Local>,
    width: f32,
) -> (bool, bool) {
    let p = a.p;
    #[cfg_attr(not(demo_core), allow(unused_mut))]
    let mut gift = false;
    #[cfg(not(demo_core))]
    let _ = gift_fee;
    let mut trust = false;
    card(&p, Margin::same(20))
        .corner_radius(CornerRadius::same(18))
        .show(ui, |ui| {
            ui.set_width(inner(width, 20));
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(txt(&nft.name, 20.0, Weight::Bold, p.text));
                let kind = match nft.origin {
                    NftOrigin::Gift { .. } | NftOrigin::Received { .. } => {
                        format!(tr!("Gift {}"), capitalized(&nft.issuer))
                    }
                    NftOrigin::Release => format!(tr!("Issue {}"), nft.issuer),
                };
                ui.label(
                    egui::RichText::new(crate::i18n::t(&format!("{kind} · {}", edition(nft))))
                        .font(mono(12.0, Weight::Regular))
                        .color(p.muted),
                );
            });
            ui.add(
                egui::Label::new(
                    txt(&nft.description, 13.0, Weight::Regular, p.muted).line_height(Some(19.5)),
                )
                .wrap(),
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
            kv_row(ui, &p, tr!("Owner"), &nft.owner, value.clone());
            match &nft.origin {
                NftOrigin::Gift { from, at } => {
                    let line = format!("{from} · {}", text::date_short(*at, now));
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
            let price = match nft.price {
                Some(d) => crate::format::amount(d),
                None => tr!("not for sale").into(),
            };
            kv_row(ui, &p, tr!("Price"), &price, value);
            // "Give" and "Sell": the demo's, as in Figma.
            #[cfg(demo_core)]
            {
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
                    ui.add(
                        egui::Label::new(txt(
                            &format!(
                                tr!("Transfer ≈ {} · a gift cannot be burned"),
                                crate::format::amount(fee)
                            ),
                            12.0,
                            Weight::Regular,
                            p.muted,
                        ))
                        .wrap(),
                    );
                }
            }
        });
    (gift, trust)
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
    let closed = dialog(ui, a, "gift-dialog", "nft", &title, 464.0, |ui, a| {
        let mut close = false;
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
        let ask = password_asked(cx.snap);
        state.password.settle(state.result.as_ref());
        let entered = state.password.field(ui, &p, "gift-password", ask);
        crate::components::button_row_spend(ui, |ui| {
            if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                close = true;
            }
            let ready =
                !state.recipient.trim().is_empty() && error.is_none() && state.password.ready(ask);
            if button_enabled(ui, a, tr!("Give"), ButtonKind::Primary, None, ready).clicked()
                || (entered && ready)
            {
                let op = Spend::GiftNft {
                    id: nft.id.clone(),
                    to: state.recipient.trim().to_string(),
                };
                cx.send(state.password.command(op, ask));
                state.result = None;
            }
        });
        match state.result.as_ref().filter(|_| !state.password.is_wrong()) {
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
        close
    });
    if closed {
        state.gifting = false;
        state.recipient.clear();
        state.result = None;
        state.password.forget();
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

#[cfg(test)]
mod tests {
    use super::{failed_detail, is_dash_name};

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
                let job = super::tile_name(name, 160.0 - 28.0, color);
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
            edition: (1, 1),
            owner: String::new(),
            price: None,
        };
        let mut state = super::CollectionState {
            ask_unknown: true,
            ..Default::default()
        };
        assert!(!state.waits_for_consent(&nft("jembe")));
        let stranger = nft("neon-forge");
        assert!(state.waits_for_consent(&stranger));
        // "Show" on this one.
        state.shown.insert(stranger.id.clone());
        assert!(!state.waits_for_consent(&stranger));
        // "Always show from neon-forge".
        let other = Nft {
            id: "nft-other".into(),
            ..nft("neon-forge")
        };
        assert!(state.waits_for_consent(&other));
        state.trusted.insert("neon-forge".into());
        assert!(!state.waits_for_consent(&other));
        // Settings: do not ask.
        state.ask_unknown = false;
        assert!(!state.waits_for_consent(&nft("anyone")));
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
