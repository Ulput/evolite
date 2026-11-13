//! The wallet is open in another evolite window: this one opens nothing — no store, no
//! keys — and says why. Made like Unlock (logo, heading, a column of text, a button);
//! not in Figma.

use super::txt;
use crate::components::{Assets, ButtonKind, button, notice};
use crate::icons::Logo;
use crate::theme::Weight;
use egui::Ui;

pub fn elsewhere(ui: &mut Ui, a: &mut Assets) {
    let p = a.p;
    let column = 420.0;
    ui.add_space(((ui.available_height() - 420.0) / 2.0).max(24.0));
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 24.0;
        ui.add(a.icons.logo(Logo::Stacked, &p, 96.0));
        ui.label(txt(
            tr!("The wallet is already open in another window"),
            24.0,
            Weight::Bold,
            p.text,
        ));
        ui.allocate_ui(egui::vec2(column, 0.0), |ui| {
            ui.spacing_mut().item_spacing.y = 16.0;
            ui.add(
                crate::components::para(
                    tr!("Work in the first evolite window. Two copies on one data folder could damage the history and the phrase's vault, so nothing is open here — neither keys nor data."),
                    14.0,
                    Weight::Regular,
                    p.muted,
                ),
            );
            notice(
                ui,
                a,
                "info",
                p.muted,
                tr!("Close the first window and this one opens the wallet by itself in a few seconds."),
            );
            if button(ui, a, tr!("Close this window"), ButtonKind::Secondary, Some(column)).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    });
}
