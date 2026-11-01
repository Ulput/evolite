//! Cards and what goes in them: card frame, list card, divider, label–value row, two columns.

use super::prelude::*;

/// card/*: surface, 1 px line, radius 16.
pub fn card(p: &Palette, margin: Margin) -> Frame {
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(margin)
}

/// A label on the left and a value on the right: 13 px, the value Medium (mono for amounts).
pub fn kv_row(ui: &mut Ui, p: &Palette, label: &str, value: &str, value_font: FontId) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    paint_text(
        ui,
        rect.left_center(),
        Align2::LEFT_CENTER,
        label,
        sans(13.0, Weight::Regular),
        p.muted,
    );
    paint_text(
        ui,
        rect.right_center(),
        Align2::RIGHT_CENTER,
        value,
        value_font,
        p.text,
    );
}

/// A horizontal 1 px line across the available width.
pub fn divider(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0, p.line);
}

/// The inner width of a card with a 1 px border and `margin` on each side.
pub fn inner(width: f32, margin: i8) -> f32 {
    width - 2.0 * margin as f32 - 2.0
}

/// Two cards side by side (gap 20; `first` tells which one), or one under the other.
pub fn two_columns(ui: &mut Ui, wide: bool, mut cell: impl FnMut(&mut Ui, bool)) {
    if wide {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            ui.vertical(|ui| cell(ui, true));
            ui.vertical(|ui| cell(ui, false));
        });
    } else {
        cell(ui, true);
        cell(ui, false);
    }
}

/// A list card: 15 SemiBold title with a note on the right, rows divided by 1 px lines.
pub fn list_card(
    ui: &mut Ui,
    a: &mut Assets,
    width: f32,
    title: &str,
    note: &str,
    rows: impl FnOnce(&mut Ui, &mut Assets),
) {
    let p = a.p;
    card(&p, Margin::same(20)).show(ui, |ui| {
        ui.set_width(inner(width, 20));
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.horizontal(|ui| {
            ui.label(txt(title, 15.0, Weight::SemiBold, p.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(txt(note, 13.0, Weight::Medium, p.muted));
            });
        });
        ui.add_space(10.0);
        rows(ui, a);
    });
}
