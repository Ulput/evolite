//! 01 · Welcome: the logo, the motto and the three ways in.

use super::{Screen, txt};
use crate::components::{Assets, ButtonKind, button};
use crate::context::Ctx;
use crate::icons::Logo;
use crate::theme::Weight;
use egui::{Align, Layout, Ui, vec2};

/// 01 · Welcome: everything centred, column 440, gap 28.
pub fn welcome(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    settings: &mut super::settings::SettingsState,
) {
    let p = a.p;
    let column = 440.0;
    let content_h = 536.0;
    // A window too low for the screen scrolls it (BUG-138, as the unlock screen, BUG-127): the
    // third button and the foot stay within reach. Centred in the window's height when it fits.
    let room = ui.available_height();
    let shown = egui::ScrollArea::vertical()
        .id_salt("welcome-screen")
        .auto_shrink([false, false])
        .show(ui, |ui| {
    ui.add_space(((room - content_h) / 2.0).max(24.0));
    let content = ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 28.0;
        // Figma 31: another wallet beside the ones here. Its top bar has the logo and the
        // network already (`App`), so neither is repeated here (designer review №23).
        let adding = cx.snap.adding_wallet;
        if !adding {
            ui.add(a.icons.logo(Logo::Stacked, &p, 119.0));
        }
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
        if adding {
            let footer = format!(tr!("Wallets on this computer: {}"), cx.snap.wallets.len());
            ui.label(txt(&footer, 13.0, Weight::Regular, p.muted));
        } else {
            // The language and the theme can be changed here, before any wallet (BUG-53).
            super::settings::language_and_theme(ui, a, cx.snap.network, settings);
        }
    });
    // Room under the foot when scrolled to the end.
    ui.add_space(24.0);
    content.response.rect.bottom()
        });
    dash_evolution_logo(ui, a, shown.inner);
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
