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
/// Side by side they are one height (design-system §4), in the very frame they are drawn: each
/// cell draws its card with [`RowCell::card`], which lays the content out and leaves a place
/// for the background; when the row is laid out and the tallest card is known, every
/// background is painted at that height (the architect's «background last», responsive
/// review 2026-11-06). Nothing is kept between frames and no other frame is asked for, so
/// nothing can flicker (BUG-62). The room a shorter card gets is at its end, under its content.
/// Returns the two columns' rectangles, both of the row's height (nothing when one is under
/// the other).
pub fn two_columns(
    ui: &mut Ui,
    wide: bool,
    mut cell: impl FnMut(&mut Ui, bool, &mut RowCell),
) -> [Rect; 2] {
    if !wide {
        for first in [true, false] {
            cell(ui, first, &mut RowCell::alone());
        }
        return [Rect::NOTHING; 2];
    }
    let mut cells = [RowCell::in_row(), RowCell::in_row()];
    let mut rects = [Rect::NOTHING; 2];
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        for (i, first) in [true, false].into_iter().enumerate() {
            rects[i] = ui
                .vertical(|ui| cell(ui, first, &mut cells[i]))
                .response
                .rect;
        }
    });
    let bottom = rects[0].bottom().max(rects[1].bottom());
    for (rect, cell) in rects.iter_mut().zip(&cells) {
        rect.max.y = bottom;
        cell.paint(ui, bottom);
    }
    rects
}

/// One cell of [`two_columns`]: draws the cell's card, so that the row can paint its
/// background at the row's height.
pub struct RowCell {
    in_row: bool,
    /// The place left for the background, the card's frame and its outer rectangle as laid out.
    card: Option<(egui::layers::ShapeIdx, Frame, Rect)>,
}

impl RowCell {
    fn alone() -> Self {
        Self {
            in_row: false,
            card: None,
        }
    }

    fn in_row() -> Self {
        Self {
            in_row: true,
            card: None,
        }
    }

    /// `frame.show(ui, add)`; in a row, the background waits for the row's height.
    pub fn card<R>(&mut self, ui: &mut Ui, frame: Frame, add: impl FnOnce(&mut Ui) -> R) -> R {
        if !self.in_row {
            return frame.show(ui, add).inner;
        }
        let bg = ui.painter().add(egui::Shape::Noop);
        // The same margins and border width, nothing painted: the frame itself comes later.
        let mut bare = frame;
        bare.fill = Color32::TRANSPARENT;
        bare.stroke.color = Color32::TRANSPARENT;
        bare.shadow = Default::default();
        let shown = bare.show(ui, add);
        self.card = Some((bg, frame, shown.response.rect));
        shown.inner
    }

    /// Paints the card's background down to `bottom`, the bottom of the row.
    fn paint(&self, ui: &Ui, bottom: f32) {
        if let Some((bg, frame, mut outer)) = self.card {
            outer.max.y = bottom;
            let content = outer - frame.total_margin();
            ui.painter().set(bg, frame.paint(content));
        }
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
    list_card_framed(
        ui,
        a,
        &mut RowCell::alone(),
        width,
        (title, note),
        right,
        rows,
    );
}

/// [`list_card`] as a cell of [`two_columns`].
pub fn list_card_in(
    ui: &mut Ui,
    a: &mut Assets,
    cell: &mut RowCell,
    width: f32,
    (title, note): (&str, &str),
    rows: impl FnOnce(&mut Ui, &mut Assets),
) {
    list_card_framed(ui, a, cell, width, (title, note), |_, _| {}, rows);
}

fn list_card_framed(
    ui: &mut Ui,
    a: &mut Assets,
    cell: &mut RowCell,
    width: f32,
    (title, note): (&str, &str),
    right: impl FnOnce(&mut Ui, &mut Assets),
    rows: impl FnOnce(&mut Ui, &mut Assets),
) {
    let p = a.p;
    cell.card(ui, card(&p, Margin::same(20)), |ui| {
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

    /// The card backgrounds painted in a frame: rectangles filled with the card colour.
    fn backgrounds(out: &egui::FullOutput, fill: Color32) -> Vec<Rect> {
        fn walk(shape: &egui::Shape, fill: Color32, found: &mut Vec<Rect>) {
            match shape {
                egui::Shape::Rect(r) if r.fill == fill => found.push(r.rect),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, fill, found)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        for clipped in &out.shapes {
            walk(&clipped.shape, fill, &mut found);
        }
        found
    }

    /// Two cards of 3 and 4 rows, as on Identity («Profile» and «Credits») and in the pool.
    fn row(ctx: &egui::Context, wide: bool) -> ([Rect; 2], Vec<Rect>) {
        let p = crate::theme::DARK;
        let mut rects = [Rect::NOTHING; 2];
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            rects = two_columns(ui, wide, |ui, first, cell| {
                cell.card(ui, card(&p, Margin::same(24)), |ui| {
                    ui.set_width(200.0);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    for _ in 0..if first { 3 } else { 4 } {
                        ui.allocate_exact_size(vec2(200.0, 30.0), Sense::hover());
                    }
                });
            });
        });
        out.textures_delta.clear();
        (rects, backgrounds(&out, p.surface))
    }

    /// The taller card: 4 rows of 30 with 3 gaps of 10, the margins and the line.
    const TALL: f32 = 4.0 * 30.0 + 3.0 * 10.0 + 2.0 * 24.0 + 2.0;

    /// Design review 2026-11-05 №7: two cards of a row side by side are one height, the shorter
    /// one grown to the taller — painted so in the very first frame (BUG-62).
    #[test]
    fn two_cards_side_by_side_are_one_height_in_the_first_frame() {
        let ctx = egui::Context::default();
        let (rects, painted) = row(&ctx, true);
        assert_eq!(painted.len(), 2, "{painted:?}");
        for r in &painted {
            assert_eq!(r.height(), TALL, "both backgrounds: {painted:?}");
            assert_eq!(r.top(), painted[0].top(), "one top: {painted:?}");
        }
        assert_eq!(rects.map(|r| r.height()), [TALL, TALL], "{rects:?}");
        assert_eq!(rects[0].left() + 250.0 + 20.0, rects[1].left(), "gap 20");
    }

    /// BUG-62: the cards of a row stand still, frame after frame, and no frame asks for
    /// another — before, the heights were measured the frame before and the two cards took
    /// turns growing, asking for a repaint each time: the tiles flickered.
    #[test]
    fn two_cards_side_by_side_keep_their_height_frame_after_frame() {
        let ctx = egui::Context::default();
        let first = row(&ctx, true).1;
        for frame in 1..8 {
            assert_eq!(row(&ctx, true).1, first, "frame {frame}");
        }
        assert!(!ctx.has_requested_repaint(), "no frame asked for another");
    }

    /// One under the other each card keeps its own height.
    #[test]
    fn one_under_the_other_nothing_grows() {
        let ctx = egui::Context::default();
        let painted = row(&ctx, false).1;
        let heights: Vec<f32> = painted.iter().map(|r| r.height()).collect();
        assert_eq!(heights, [TALL - 40.0, TALL], "{painted:?}");
    }
}
