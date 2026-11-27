//! FEAT-0050 UC-1: «What suits you better?» — asked once per app, on its very first run, before
//! «Create / Restore»: two cards, Simple chosen; «Continue» keeps the choice for every wallet. An
//! update from a version without it is not asked (UC-3): its «advanced» switch is the mode.

use crate::components::{Assets, ButtonKind, button, option_card};
use crate::screens::settings::SettingsState;
use crate::screens::txt;
use crate::screens::welcome::{LOW_TOP, login_logo, low_window};
use crate::theme::Weight;
use egui::{Ui, vec2};

/// Whether this run asks: the very first one — no wallet here or on another network, not adding
/// one — and the app's settings with no mode yet.
pub fn to_ask(
    settings: &SettingsState,
    wallets: usize,
    other_wallets: usize,
    adding: bool,
) -> bool {
    wallets == 0 && other_wallets == 0 && !adding && settings.mode_to_ask()
}

/// The question in place of the Welcome's buttons; the choice goes to `settings`.
pub fn ask(
    ui: &mut Ui,
    a: &mut Assets,
    settings: &mut SettingsState,
    network: evolite_api::Network,
) {
    let p = a.p;
    let low = low_window(ui);
    let picked_id = egui::Id::new("first-run-mode");
    let mut advanced = ui.data(|d| d.get_temp::<bool>(picked_id)).unwrap_or(false);
    egui::ScrollArea::vertical()
        .id_salt("first-run-mode-screen")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let room = ui.available_height();
            ui.add_space(if low { LOW_TOP } else { ((room - 520.0) / 2.0).max(24.0) });
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 20.0;
                login_logo(ui, a, low, 119.0);
                ui.label(txt(tr!("What suits you better?"), 20.0, Weight::Medium, p.text));
                ui.allocate_ui(vec2(440.0, 0.0), |ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    for (on, icon, title, detail) in [
                        (
                            false,
                            "home",
                            tr!("Simple mode"),
                            tr!("Send, receive, contacts and the private pool — without extra terms."),
                        ),
                        (
                            true,
                            "settings",
                            tr!("Advanced mode"),
                            tr!(
                                "Everything in Simple, plus the Platform address, keys, fees, the node and the Workshop. Dash terms as they are."
                            ),
                        ),
                    ] {
                        if option_card(ui, a, icon, title, detail, advanced == on).clicked() {
                            advanced = on;
                        }
                    }
                });
                ui.label(txt(
                    tr!("You can switch at any time. Your money and data stay as they are."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
                if button(ui, a, tr!("Continue"), ButtonKind::Primary, Some(320.0)).clicked() {
                    settings.choose_mode(advanced);
                }
                // The language and the theme here too, as on the Welcome (BUG-53).
                ui.add_space(20.0);
                crate::screens::settings::language_and_theme(ui, a, network, settings);
            });
            ui.add_space(24.0);
        });
    ui.data_mut(|d| d.insert_temp(picked_id, advanced));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::settings::Section;

    /// FEAT-0050 A1, A5: asked on the very first run only — not with a wallet here or on another
    /// network, not while adding one, not once answered, not after an update (its switch set,
    /// on or off: that is the mode).
    #[test]
    fn the_mode_is_asked_once_and_never_after_an_update() {
        let fresh = SettingsState::new(true, Section::Network, false);
        assert!(to_ask(&fresh, 0, 0, false), "the very first run");
        assert!(!to_ask(&fresh, 1, 0, false), "a wallet here");
        assert!(
            !to_ask(&fresh, 0, 1, false),
            "a wallet on the other network"
        );
        assert!(!to_ask(&fresh, 0, 0, true), "adding a wallet");
        let mut answered = SettingsState::new(true, Section::Network, false);
        answered.choose_mode(true);
        assert!(!to_ask(&answered, 0, 0, false), "answered");
        assert!(answered.is_on("advanced"), "Advanced chosen");
        for was in [true, false] {
            let mut updated = SettingsState::new(true, Section::Network, false);
            updated.set("advanced", was);
            assert!(
                !to_ask(&updated, 0, 0, false),
                "an update with the switch {was}"
            );
            assert_eq!(updated.is_on("advanced"), was, "the mode is the switch");
        }
        // A10: a wallet seen once — after the last one is deleted, not asked.
        let mut had = SettingsState::new(true, Section::Network, false);
        had.mode_known();
        assert!(!to_ask(&had, 0, 0, false), "after the last wallet went");
        assert!(!had.is_on("advanced"), "Simple, as the switch was");
    }
}
