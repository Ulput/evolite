//! Page parts: the screen header (title, optional count, actions on the right) and the mono
//! uppercase label above a group or a day.

use super::prelude::*;

/// Header: 28 Bold title, an optional mono count next to it, actions on the right (drawn right
/// to left, so the primary one goes first). 41 px, as tall as a button.
pub fn page_header(
    ui: &mut Ui,
    a: &mut Assets,
    title: &str,
    count: Option<&str>,
    actions: impl FnOnce(&mut Ui, &mut Assets),
) {
    let p = a.p;
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 41.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.label(txt(title, 28.0, Weight::Bold, p.text));
            if let Some(count) = count {
                ui.label(
                    egui::RichText::new(crate::i18n::t(count))
                        .font(mono(16.0, Weight::Medium))
                        .color(p.muted),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                actions(ui, a);
            });
        },
    );
}

/// "NETWORK", "TODAY": 11 mono Medium, uppercase, letter spacing 0.88.
pub fn section_label(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(
        egui::RichText::new(crate::i18n::t(text).to_uppercase())
            .font(mono(11.0, Weight::Medium))
            .color(p.muted)
            .extra_letter_spacing(0.88),
    );
}
