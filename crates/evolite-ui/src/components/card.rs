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
    kv_row_in(ui, p, label, value, value_font, p.text);
}

/// [`kv_row`] with the value in `color` (red for credits not added, BUG-57).
pub fn kv_row_in(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    value: &str,
    value_font: FontId,
    color: egui::Color32,
) {
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
        color,
    );
}

/// [`kv_row`] with the icon of [`amount_toggle`] right after the label (BUG-73: «Payments» of
/// a contact, «From the Core wallet» in a dialog): as high as a [`kv_row`], the toggle taking
/// only its icon's 16 px in the row (FEAT-0034), the value on the right.
pub fn kv_row_eye(ui: &mut Ui, a: &mut Assets, label: &str, value: &str, value_font: FontId) {
    let color = a.p.text;
    kv_row_eye_in(ui, a, label, value, value_font, color);
}

/// [`kv_row_eye`] with the value in `color` (a Core amount not checked now is muted, BUG-80).
pub fn kv_row_eye_in(
    ui: &mut Ui,
    a: &mut Assets,
    label: &str,
    value: &str,
    value_font: FontId,
    color: egui::Color32,
) {
    let p = a.p;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(txt(label, 13.0, Weight::Regular, p.muted));
        amount_toggle(ui, a, true);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                egui::RichText::new(crate::i18n::t(value))
                    .font(value_font)
                    .color(color),
            );
        });
    });
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
    let rects = cards_row(ui, 0..2, 20.0, |ui, i, c| cell(ui, i == 0, c));
    [rects[0], rects[1]]
}

/// What [`grid`] lays out: cells of at least `min_w` (a minimum of the responsive spec,
/// `theme.rs`), `gap` between them across and down.
#[derive(Clone, Copy, Debug)]
pub struct GridSpec {
    pub min_w: f32,
    pub gap: egui::Vec2,
    /// At most this many columns, however wide.
    pub max_cols: usize,
    /// Only a count of columns that divides the cells: four tiles stand 4, 2 by 2 or one
    /// under another, never 3 and 1.
    pub even: bool,
}

/// How many columns of [`GridSpec`] `spec` hold `n` cells in `width`.
pub fn grid_columns(width: f32, n: usize, spec: GridSpec) -> usize {
    let most = n.min(spec.max_cols).max(1);
    let fit = (((width + spec.gap.x) / (spec.min_w + spec.gap.x)).floor() as usize).clamp(1, most);
    if spec.even {
        (1..=fit).rev().find(|c| n.is_multiple_of(*c)).unwrap_or(1)
    } else {
        fit
    }
}

/// The columns of [`grid_columns`] and the width of each cell in `width`: what [`grid`] lays out
/// in that width, for a screen that measures its cells before drawing them (one amount size for
/// a row of Overview's tiles, one height for the pool's actions).
pub fn grid_layout(width: f32, n: usize, spec: GridSpec) -> (usize, f32) {
    let cols = grid_columns(width, n, spec);
    let cell_w = ((width - spec.gap.x * (cols - 1) as f32) / cols as f32).max(0.0);
    (cols, cell_w)
}

/// The width of each of `n` equal columns in `width` (8 px between them), whole pixels: all
/// `n` in one row when each holds `least`, otherwise fewer per row (BUG-81), one under another
/// at the least. «From» on Send and on «Create an identity»; unlike [`grid_layout`] the least
/// is what the widest cell's words need, not a minimum of the responsive spec.
pub fn from_columns(width: f32, n: usize, least: f32) -> f32 {
    let column = |k: usize| ((width - 8.0 * (k as f32 - 1.0)) / k as f32).floor();
    (1..=n.max(1))
        .rev()
        .map(column)
        .find(|&c| c >= least)
        .unwrap_or_else(|| column(1))
}

/// `n` cells in as many columns of at least `spec.min_w` as fit the available width
/// ([`grid_layout`]), all of one width (`cell` is told it, with the cell's index). The cards of
/// a row are one height in the very frame they are drawn, as in [`two_columns`]: each cell draws
/// its card with [`RowCell::card`], and the backgrounds are painted last, at the tallest one's
/// height (the responsive spec, R4). Returns the number of columns.
pub fn grid(
    ui: &mut Ui,
    n: usize,
    spec: GridSpec,
    mut cell: impl FnMut(&mut Ui, usize, f32, &mut RowCell),
) -> usize {
    let (cols, cell_w) = grid_layout(ui.available_width(), n, spec);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = spec.gap.y;
        for start in (0..n).step_by(cols) {
            let end = (start + cols).min(n);
            if cols == 1 {
                cell(ui, start, cell_w, &mut RowCell::alone());
            } else {
                cards_row(ui, start..end, spec.gap.x, |ui, i, c| {
                    cell(ui, i, cell_w, c)
                });
            }
        }
    });
    cols
}

/// One row of cards side by side, `gap` apart, one height: each cell's background painted
/// last, down to the row's bottom. The rectangles of the cells, all of the row's height.
fn cards_row(
    ui: &mut Ui,
    cells: std::ops::Range<usize>,
    gap: f32,
    mut cell: impl FnMut(&mut Ui, usize, &mut RowCell),
) -> Vec<Rect> {
    let mut drawn: Vec<(Rect, RowCell)> = Vec::with_capacity(cells.len());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for i in cells {
            let mut c = RowCell::in_row();
            let rect = ui.vertical(|ui| cell(ui, i, &mut c)).response.rect;
            drawn.push((rect, c));
        }
    });
    let bottom = drawn
        .iter()
        .map(|(r, _)| r.bottom())
        .fold(f32::NEG_INFINITY, f32::max);
    drawn
        .into_iter()
        .map(|(mut rect, c)| {
            rect.max.y = bottom;
            c.paint(ui, bottom);
            rect
        })
        .collect()
}

/// One cell of [`two_columns`] or of [`grid`]: draws the cell's card, so that the row can paint its
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
        // One card per cell: a second would leave the first one's background unpainted.
        debug_assert!(self.card.is_none(), "a cell of a row draws one card");
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

    /// BUG-81: three columns while each holds the widest words, then two, then one; a column
    /// and the 8 px between them never take more than the panel.
    #[test]
    fn the_sources_take_as_many_columns_as_hold_their_words() {
        assert_eq!(from_columns(474.0, 3, 141.0), 152.0);
        assert_eq!(from_columns(474.0, 3, 152.0), 152.0);
        assert_eq!(from_columns(474.0, 3, 153.0), 233.0);
        assert_eq!(from_columns(474.0, 3, 234.0), 474.0);
        assert_eq!(from_columns(474.0, 3, 600.0), 474.0);
        assert_eq!(from_columns(474.0, 0, 10.0), 474.0);
        for least in [100.0, 152.5, 200.0, 300.0] {
            let c = from_columns(474.0, 3, least);
            let per_row = ((474.0 + 8.0) / (c + 8.0)).floor().min(3.0);
            assert!(per_row * c + 8.0 * (per_row - 1.0) <= 474.0, "{least}: {c}");
        }
    }

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

    const TILES: GridSpec = GridSpec {
        min_w: 200.0,
        gap: egui::Vec2::new(16.0, 16.0),
        max_cols: 4,
        even: true,
    };

    /// Responsive spec R4: as many columns of the least width as fit, at most the cells or
    /// `max_cols`; four tiles stand 4, 2 by 2 or one under another, never 3 and 1.
    #[test]
    fn a_grid_takes_as_many_columns_as_fit() {
        assert_eq!(grid_columns(848.0, 4, TILES), 4, "4 x 200 + 3 x 16");
        assert_eq!(grid_columns(847.0, 4, TILES), 2, "3 fit: not 3 and 1");
        assert_eq!(grid_columns(416.0, 4, TILES), 2);
        assert_eq!(grid_columns(415.0, 4, TILES), 1);
        assert_eq!(grid_columns(5000.0, 4, TILES), 4, "max_cols");
        assert_eq!(grid_columns(10.0, 4, TILES), 1, "one at the least");
        let apps = GridSpec {
            min_w: 240.0,
            even: false,
            max_cols: 3,
            ..TILES
        };
        assert_eq!(grid_columns(752.0, 3, apps), 3);
        assert_eq!(grid_columns(751.0, 3, apps), 2, "2 and 1 is fine here");
        assert_eq!(grid_columns(495.0, 3, apps), 1);
        assert_eq!(grid_columns(5000.0, 5, apps), 3, "no more than max_cols");
        // NFT cards of 195 (the designer): three in Collection's 618 at 1280 (628 less the scroll bar).
        let nfts = GridSpec {
            min_w: crate::theme::NFT_MIN_W,
            max_cols: 6,
            even: false,
            ..TILES
        };
        assert_eq!(grid_columns(618.0, 5, nfts), 3, "3 x 195 + 2 x 16 = 617");
        assert_eq!(grid_columns(616.0, 5, nfts), 2);
        assert_eq!(
            grid_columns(5000.0, 2, nfts),
            2,
            "no more columns than cells"
        );
        assert_eq!(grid_columns(5000.0, 0, nfts), 1);
    }

    /// Responsive spec R4: the cards of each row of a grid are one height, the row's tallest,
    /// in the very first frame; another row keeps its own.
    #[test]
    fn the_cards_of_a_grid_row_are_one_height_in_the_first_frame() {
        let p = crate::theme::DARK;
        let ctx = egui::Context::default();
        let (mut cols, mut widths) = (0, Vec::new());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.allocate_ui(vec2(700.0, 4000.0), |ui| {
                let spec = GridSpec {
                    even: false,
                    ..TILES
                };
                cols = grid(ui, 5, spec, |ui, i, w, cell| {
                    widths.push(w);
                    cell.card(ui, card(&p, Margin::same(24)), |ui| {
                        ui.set_width(inner(w, 24));
                        ui.spacing_mut().item_spacing.y = 10.0;
                        for _ in 0..=i {
                            ui.allocate_exact_size(vec2(10.0, 30.0), Sense::hover());
                        }
                    });
                });
            });
        });
        out.textures_delta.clear();
        assert_eq!(cols, 3, "3 x 200 + 2 x 16 in 700");
        assert!(
            widths
                .iter()
                .all(|w| (w - (700.0 - 32.0) / 3.0).abs() < 0.01),
            "{widths:?}"
        );
        let painted = backgrounds(&out, p.surface);
        assert_eq!(painted.len(), 5, "{painted:?}");
        let tall = |rows: f32| rows * 30.0 + (rows - 1.0) * 10.0 + 2.0 * 24.0 + 2.0;
        let heights: Vec<f32> = painted.iter().map(|r| r.height()).collect();
        assert_eq!(
            heights,
            [tall(3.0), tall(3.0), tall(3.0), tall(5.0), tall(5.0)]
        );
        assert_eq!(painted[3].top(), painted[0].bottom() + 16.0, "gap 16 down");
    }
}
