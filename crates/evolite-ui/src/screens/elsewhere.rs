//! The wallet is open in another evolite window: this one opens nothing — no store, no
//! keys — and says why. Made like Unlock (logo, heading, a column of text, a button);
//! not in Figma.

use super::txt;
use crate::components::{Assets, ButtonKind, button, notice};
use crate::icons::Logo;
use crate::theme::Weight;
use egui::Ui;

/// `switched`: this run was started by a switch to `here` from `back` (FEAT-0037): the folder is
/// that network open in another window, said so (AC-A6), with a way back. True when «Back to …»
/// was clicked.
pub fn elsewhere(
    ui: &mut Ui,
    a: &mut Assets,
    switched: Option<(evolite_api::Network, evolite_api::Network)>,
) -> bool {
    let p = a.p;
    let mut back = false;
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
            if let Some((here, other)) = switched {
                let name = |n| match n {
                    evolite_api::Network::Testnet => "Testnet",
                    evolite_api::Network::Mainnet => "Mainnet",
                };
                ui.add(crate::components::para(
                    &format!(
                        tr!("{} is already open in another evolite window. Switch to that window or go back to {}."),
                        name(here),
                        name(other).to_lowercase()
                    ),
                    14.0,
                    Weight::Regular,
                    p.muted,
                ));
                let label = format!(tr!("Back to {}"), name(other).to_lowercase());
                back = button(ui, a, &label, ButtonKind::Primary, Some(column)).clicked();
                return;
            }
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
    back
}
