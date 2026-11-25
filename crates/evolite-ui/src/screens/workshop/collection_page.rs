//! W4 «Collection» (FEAT-0038 UC-4): the collection's name, its kind and id, «About the
//! collection» and «New model», and its models as tiles on the grid of the Collection
//! (BUG-188: 164 wide, 12 apart, the picture 148): name, «issued {m} of {n}», «you have {k}».
//! The numbers are the core's proved counts ([`WorkshopAsk::Counts`]), «—» until they come.

use super::*;
use crate::components::{Part, dialog_with_footer, kv_row_fit};
use evolite_api::workshop::{Count, Model};

/// The tile's width and gap, and the picture's side (BUG-188).
const TILE: f32 = 164.0;
const GAP: f32 = 12.0;
const PICTURE: f32 = 148.0;

pub(super) fn page(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut WorkshopState,
    art: &mut crate::art::ArtCache,
    contract: &str,
) {
    let p = a.p;
    ui.spacing_mut().item_spacing.y = 16.0;
    if crate::components::back_link(ui, a, tr!("Workshop")).clicked() {
        state.page = Page::Home;
        state.about = false;
        return;
    }
    let found = match &state.collections {
        Some(Ok(all)) => all.iter().find(|c| c.contract == contract).cloned(),
        _ => None,
    };
    let Some(c) = found else {
        // Just made: the list is read again and shows it in a moment.
        page_header(ui, a, tr!("Collection"), None, |_, _| {});
        let words = match &state.collections {
            Some(Err(e)) => crate::text::error_text(e),
            Some(Ok(_)) => {
                tr!("The collection is not on the network yet. It shows in a moment.").into()
            }
            None => tr!("Reading your collections…").into(),
        };
        ui.label(txt(&words, 13.0, Weight::Regular, p.muted));
        if state.collections.is_some()
            && button(ui, a, tr!("Refresh"), ButtonKind::Secondary, None).clicked()
        {
            state.read_again();
        }
        return;
    };
    let name = collection_name(&c);
    let mut about = false;
    let mut new_model = false;
    page_header(ui, a, &crate::i18n::own(&name), None, |ui, a| {
        new_model = button(ui, a, tr!("New model"), ButtonKind::Primary, None).clicked();
        about = button(
            ui,
            a,
            tr!("About the collection"),
            ButtonKind::Secondary,
            None,
        )
        .clicked();
    });
    if about {
        state.about = true;
    }
    if new_model {
        state.model = ModelForm::new(&c.contract);
        state.page = Page::NewModel(c.contract.clone());
        return;
    }
    let who = super::super::tokens::shown_name(cx.wallet());
    // BUG-258: the tags and «id · by» go on to the next line at the medium window.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for tag in kind_tags(&c) {
            tag_chip(ui, &p, tag);
        }
        ui.label(txt("NFT evolite v1", 11.0, Weight::Medium, p.accent));
        ui.label(txt(
            &format!(
                tr!("id {id} · by {name}"),
                id = crate::text::whole(&c.contract),
                name = crate::i18n::own(&who)
            ),
            12.0,
            Weight::Regular,
            p.muted,
        ));
    });
    if c.models.is_empty() {
        ui.add_space(8.0);
        ui.label(txt(
            tr!("No models yet. A model is a picture with its name and supply; its copies are issued from it."),
            13.0,
            Weight::Regular,
            p.muted,
        ));
    } else {
        // Each model's numbers, asked once per visit.
        for m in &c.models {
            let key = (c.contract.clone(), m.id.clone());
            let now = ui.input(|i| i.time);
            if super::ask_now(&mut state.counts_asked, &key, now, false) {
                cx.send(Command::ReadWorkshop {
                    ask: WorkshopAsk::Counts {
                        contract: c.contract.clone(),
                        model: m.id.clone(),
                    },
                });
            }
        }
        let width = ui.available_width();
        let cols = (((width + GAP) / (TILE + GAP)).floor() as usize).max(1);
        let mut opened = None;
        for row in c.models.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for m in row {
                    let counts = state.counts.get(&(c.contract.clone(), m.id.clone()));
                    if model_tile(ui, a, cx, art, m, counts) {
                        opened = Some(m.id.clone());
                    }
                }
            });
        }
        if let Some(model) = opened {
            state.page = Page::Model {
                contract: c.contract.clone(),
                model,
            };
            return;
        }
    }
    if state.about && about_window(ui, a, &c, &name) {
        state.about = false;
    }
}

/// A number the core counted, or «—».
fn count(c: Option<Count>) -> String {
    match c {
        Some(Count::Proved(n)) => crate::format::count(n),
        Some(Count::Counted {
            count, complete, ..
        }) => {
            if complete {
                crate::format::count(count)
            } else {
                format!(tr!("at least {}"), crate::format::count(count))
            }
        }
        None => "—".into(),
    }
}

/// One model: its picture, name, «issued {m} of {n}», «you have {k}».
fn model_tile(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    art: &mut crate::art::ArtCache,
    m: &Model,
    counts: Option<&ModelCounts>,
) -> bool {
    let p = a.p;
    // W7: the tile opens the model's page.
    let (rect, resp) = ui.allocate_exact_size(vec2(TILE, PICTURE + 74.0), Sense::click());
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(12), p.surface);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(12),
        egui::Stroke::new(1.0, p.line),
        egui::StrokeKind::Inside,
    );
    let picture = egui::Rect::from_min_size(
        rect.min + vec2((TILE - PICTURE) / 2.0, 8.0),
        vec2(PICTURE, PICTURE),
    );
    if m.art.is_empty() {
        ui.painter()
            .rect_filled(picture, egui::CornerRadius::same(10), p.raised);
        a.icons.icon("image", p.muted, &p, 28.0).paint_at(
            ui,
            egui::Rect::from_center_size(picture.center(), vec2(28.0, 28.0)),
        );
    } else {
        let mut send = |c: Command| cx.send(c);
        art.paint(ui, &p, &m.art, picture, &mut send);
    }
    let issued = counts
        .and_then(|c| c.issued)
        .map_or("—".to_owned(), |n| crate::format::count(n as u64));
    let supply = if m.supply == 0 {
        "—".to_owned()
    } else {
        crate::format::count(m.supply as u64)
    };
    let lines = [
        (crate::i18n::own(&m.name), 14.0, Weight::SemiBold, p.text),
        (
            format!(tr!("issued {m} of {n}"), m = issued, n = supply),
            11.0,
            Weight::Regular,
            p.text,
        ),
        (
            format!(tr!("you have {}"), count(counts.and_then(|c| c.mine))),
            11.0,
            Weight::Regular,
            p.muted,
        ),
    ];
    let mut y = picture.bottom() + 10.0;
    for (words, size, weight, color) in lines {
        let font = crate::theme::sans(size, weight);
        let shown = crate::i18n::t(&words).into_owned();
        let fitted = crate::components::cut_to(ui.ctx(), &shown, &font, TILE - 16.0, 1);
        crate::components::paint_text(
            ui,
            egui::pos2(rect.left() + 8.0, y),
            egui::Align2::LEFT_TOP,
            &fitted,
            font,
            color,
        );
        y += size + 6.0;
    }
    // BUG-270: the person's own words, never translated.
    crate::components::named(
        &resp,
        egui::WidgetType::Button,
        true,
        &crate::i18n::own(&m.name),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// «About the collection»: what it is, its id, its rules. Whether it closed.
fn about_window(ui: &mut Ui, a: &mut Assets, c: &Collection, name: &str) -> bool {
    dialog_with_footer(
        ui,
        a,
        "workshop-about-collection",
        "nft",
        tr!("About the collection"),
        480.0,
        |ui, a, part| {
            let p = a.p;
            if part == Part::Footer {
                let mut close = false;
                crate::components::button_row(ui, |ui| {
                    close = button(ui, a, tr!("Close"), ButtonKind::Primary, None).clicked();
                });
                return close;
            }
            let value = crate::theme::mono(13.0, Weight::Medium);
            ui.spacing_mut().item_spacing.y = 10.0;
            kv_row_fit(
                ui,
                &p,
                tr!("Collection name"),
                &crate::i18n::own(name),
                value.clone(),
            );
            kv_row_fit(
                ui,
                &p,
                tr!("Contract"),
                &crate::text::whole(&c.contract),
                value.clone(),
            );
            kv_row_fit(
                ui,
                &p,
                tr!("Models"),
                &crate::format::count(c.models.len() as u64),
                value.clone(),
            );
            kv_row_fit(ui, &p, tr!("Standard"), "NFT evolite v1", value.clone());
            super::super::tokens::para(
                ui,
                tr!(
                    "A collection cannot be changed or deleted once made — only new models and copies can be issued in it."
                ),
                12.0,
                p.muted,
                None,
            );
            false
        },
    )
}
