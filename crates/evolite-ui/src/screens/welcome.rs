//! 01 · Welcome: the logo, the motto and the three ways in.

use super::{Screen, txt};
use crate::components::{Assets, ButtonKind, button, network_chip};
use crate::context::Ctx;
use crate::icons::Logo;
use crate::theme::Weight;
use egui::{Align, Layout, Ui, vec2};

/// 01 · Welcome: everything centred, column 440, gap 28.
pub fn welcome(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let p = a.p;
    let column = 440.0;
    let content_h = 536.0;
    ui.add_space(((ui.available_height() - content_h) / 2.0).max(24.0));
    let content = ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 28.0;
        ui.add(a.icons.logo(Logo::Stacked, &p, 119.0));
        //, Figma 31: another wallet beside the ones here.
        let adding = cx.snap.adding_wallet;
        let (title, lead) = if adding {
            let names: Vec<String> = cx
                .snap
                .wallets
                .iter()
                .map(|w| format!("«{}»", crate::i18n::own(&w.name)))
                .collect();
            let stay = match names.len() {
                0 => String::new(),
                1 => format!(tr!(" {} stays — switch in the menu on the left."), names[0]),
                n => format!(
                    tr!(" {} and {} stay — switch between them in the menu on the left."),
                    names[..n - 1].join(", "),
                    names[n - 1]
                ),
            };
            (
                tr!("A new wallet on this computer").to_owned(),
                format!(tr!("It will have its own phrase, password and history.{stay}"), stay = stay),
            )
        } else {
            (
                tr!("Evolution you can verify.").to_owned(),
                tr!("A light wallet for Dash Core and Dash Platform. Only you hold the keys; network data is verified on this computer.").to_owned(),
            )
        };
        ui.label(txt(&title, 20.0, Weight::Medium, p.text));
        ui.allocate_ui(vec2(column, 80.0), |ui| {
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.label(txt(&lead, 15.0, Weight::Regular, p.muted).line_height(Some(22.5)));
            });
        });
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.add_space(-10.0);
        use super::onboarding::Flow;
        for (label, kind, flow) in [
            (tr!("Create a wallet"), ButtonKind::Primary, Flow::Create),
            (tr!("Restore from phrase"), ButtonKind::Secondary, Flow::Restore),
            (tr!("Import from DashPay"), ButtonKind::Secondary, Flow::DashPay),
        ] {
            if button(ui, a, label, kind, Some(320.0)).clicked() {
                cx.go(Screen::Onboarding(flow));
            }
        }
        ui.add_space(40.0);
        // The language this screen is in, by its own name: "Russian" was written here whatever
        // the language.
        let lang = crate::i18n::own(crate::i18n::current().native());
        let footer = if adding {
            format!(tr!("Wallets on this computer: {}"), cx.snap.wallets.len())
        } else if p.dark {
            format!(tr!("{} · dark theme"), lang)
        } else {
            format!(tr!("{} · light theme"), lang)
        };
        ui.allocate_ui_with_layout(vec2(320.0, 25.0), Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            network_chip(ui, a, cx.snap.network);
            ui.label(txt(&footer, 13.0, Weight::Regular, p.muted));
        });
    });
    dash_evolution_logo(ui, a, content.response.rect.bottom());
}

/// The Dash Evolution logo, alone and centred at the foot of the welcome and unlock screens:
/// the network the wallet stands on, under its own logo, never in the way. 40 px high (≈ 96
/// wide: its "Dash Evolution" lettering stays readable), 28 px from the bottom; left out when
/// the window is too low for it, under the screen's content that ends at `content_bottom`.
pub(crate) fn dash_evolution_logo(ui: &mut Ui, a: &mut Assets, content_bottom: f32) {
    let p = a.p;
    let area = ui.max_rect();
    let height = 40.0;
    let logo = a.icons.dash_evolution(&p, height);
    let size = logo.size().unwrap_or(vec2(height * 2.4, height));
    let bottom = area.bottom() - 28.0;
    // Below what the screen drew, with room to spare.
    if bottom - height < content_bottom + 16.0 {
        return;
    }
    let rect =
        egui::Rect::from_center_size(egui::pos2(area.center().x, bottom - height / 2.0), size);
    logo.paint_at(ui, rect);
    let resp = ui.interact(rect, ui.id().with("dash-evolution"), egui::Sense::hover());
    crate::components::named(&resp, egui::WidgetType::Image, true, "Dash Evolution");
}
