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
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    // Painted words say nothing to a screen reader: the row reads «label: value».
    let spoken = format!(
        "{}: {}",
        crate::i18n::spoken(label),
        crate::i18n::spoken(value)
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
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
///
/// Side by side they are one height (design-system §4): the shorter card grows to the taller
/// one's height, measured the frame before. The cell gets how much its card is short, `extra`,
/// and puts it at the end of the card's content with [`grow`]; one under the other it is 0.
/// Returns the two columns' rectangles (nothing when one is under the other).
pub fn two_columns(
    ui: &mut Ui,
    id: &str,
    wide: bool,
    mut cell: impl FnMut(&mut Ui, bool, f32),
) -> [Rect; 2] {
    if !wide {
        cell(ui, true, 0.0);
        cell(ui, false, 0.0);
        return [Rect::NOTHING; 2];
    }
    let id = ui.id().with(id).with("two-columns");
    // Each card's own height, without what it grew by.
    let natural: [f32; 2] = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let tallest = natural[0].max(natural[1]);
    let extra = natural.map(|h| (tallest - h).max(0.0));
    let mut rects = [Rect::NOTHING; 2];
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        for (i, first) in [true, false].into_iter().enumerate() {
            rects[i] = ui.vertical(|ui| cell(ui, first, extra[i])).response.rect;
        }
    });
    let now = [0, 1].map(|i| rects[i].height() - extra[i]);
    if (0..2).any(|i| (now[i] - natural[i]).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(id, now));
        ui.ctx().request_repaint();
    }
    rects
}

/// Makes a card's content `extra` taller (see [`two_columns`]); called last inside the card.
pub fn grow(ui: &mut Ui, extra: f32) {
    if extra > 0.0 {
        let h = ui.min_rect().height() + extra;
        ui.set_min_height(h);
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
    list_card_with(ui, a, width, (title, note), |_, _| {}, rows);
}

/// [`list_card`] with something more at the right end of its title row, after the note
/// (`right`, drawn right to left): the eye of «Earlier addresses» (FEAT-0007).
pub fn list_card_with(
    ui: &mut Ui,
    a: &mut Assets,
    width: f32,
    (title, note): (&str, &str),
    right: impl FnOnce(&mut Ui, &mut Assets),
    rows: impl FnOnce(&mut Ui, &mut Assets),
) {
    let p = a.p;
    card(&p, Margin::same(20)).show(ui, |ui| {
        ui.set_width(inner(width, 20));
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.horizontal(|ui| {
            ui.label(txt(title, 15.0, Weight::SemiBold, p.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                right(ui, a);
                ui.label(txt(note, 13.0, Weight::Medium, p.muted));
            });
        });
        ui.add_space(10.0);
        rows(ui, a);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Design review 2026-11-05 №7: two cards of a row side by side are one height, the shorter
    /// one grown to the taller; one under the other nothing grows.
    #[test]
    fn two_cards_side_by_side_are_one_height() {
        let ctx = egui::Context::default();
        let p = crate::theme::DARK;
        let mut rects = [Rect::NOTHING; 2];
        for _ in 0..4 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                rects = two_columns(ui, "test", true, |ui, first, extra| {
                    card(&p, Margin::same(24)).show(ui, |ui| {
                        ui.set_width(200.0);
                        let tall = if first { 40.0 } else { 120.0 };
                        ui.allocate_exact_size(vec2(200.0, tall), Sense::hover());
                        grow(ui, extra);
                    });
                });
            });
            out.textures_delta.clear();
        }
        assert_eq!(rects[0].height(), rects[1].height(), "{rects:?}");
        assert_eq!(
            rects[1].height(),
            120.0 + 2.0 * 24.0 + 2.0,
            "the taller one keeps its own"
        );
        assert_eq!(rects[0].left() + 250.0 + 20.0, rects[1].left(), "gap 20");
    }
}
