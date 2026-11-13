//! Right to left (Arabic): the whole interface mirrored in one place instead of
//! on every screen. The screens are laid out left to right as always; this egui plugin turns
//! the finished frame over around the window's vertical axis and the pointer with it, so the
//! menu stands on the right, titles start at the right edge, a row's amount goes to its left
//! end and "›" points left.
//!
//! What is not turned over, only moved to its mirrored place:
//! - text: a line keeps its letters (Arabic is already in visual order, see
//!   [`crate::i18n::visual`]); lines of a paragraph align to the right, and an Arabic paragraph
//!   wrapped by egui (in visual order, from its end) is read top-down from its start;
//! - pictures (icons, the logo, avatars, NFT): send ↗ and receive ↙ stay as in Figma — only
//!   "chevron-left/right" swap, see [`crate::icons`];
//! - [`keep`] places: a QR code (a mirrored one may not scan), a text field's text with its
//!   cursor.

use egui::epaint::{CornerRadius, Mesh, Shape, TextShape, TextureId};
use egui::{Context, Event, FullOutput, Id, Pos2, RawInput, Rect, Ui, vec2};

/// The plugin; added once at start, it does nothing while the language reads left to right.
pub struct Mirror {
    /// Mirroring this frame (decided with its input, so input and picture agree).
    on: bool,
    /// The window's left + right edge, in points: x becomes `axis − x`.
    axis: f32,
    /// Whether the interface reads right to left; a test asks without switching the language.
    rtl: fn() -> bool,
}

impl Default for Mirror {
    fn default() -> Self {
        Self {
            on: false,
            axis: 0.0,
            rtl: crate::i18n::rtl,
        }
    }
}

fn keep_id() -> Id {
    Id::new("rtl-keep")
}

fn on_id() -> Id {
    Id::new("rtl-on")
}

/// Keeps `rect` left to right inside: it moves to its mirrored place as a whole.
pub fn keep(ctx: &Context, rect: Rect) {
    if ctx.data(|d| d.get_temp::<bool>(on_id())) == Some(true) {
        ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<Rect>>(keep_id()).push(rect));
    }
}

/// A text edit whose cursor stays by its text when mirrored: the text moves as a whole, and the
/// cursor (a separate line egui draws) must move with it rather than be reflected.
pub fn text_edit(ui: &mut Ui, edit: egui::TextEdit<'_>) -> egui::Response {
    let out = edit.show(ui);
    let text = Rect::from_min_size(out.galley_pos, out.galley.size());
    // The cursor is a little wider and taller than the line it stands in.
    keep(ui.ctx(), text.expand2(vec2(3.0, 4.0)));
    // Its label above is its name for a screen reader (BUG-41).
    crate::a11y::take_field_name(ui, &out.response.response);
    // In a dialog: the first field takes the focus when it opens, Enter goes on from it.
    crate::a11y::field_drawn(ui, &out.response.response);
    out.response.response
}

impl egui::Plugin for Mirror {
    fn debug_name(&self) -> &'static str {
        "rtl"
    }

    fn input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        self.on = (self.rtl)();
        ctx.data_mut(|d| d.insert_temp(on_id(), self.on));
        let screen = input.screen_rect.unwrap_or_else(|| ctx.viewport_rect());
        self.axis = screen.min.x + screen.max.x;
        if !self.on {
            return;
        }
        let axis = self.axis;
        for event in &mut input.events {
            match event {
                Event::PointerMoved(pos)
                | Event::PointerButton { pos, .. }
                | Event::Touch { pos, .. } => pos.x = axis - pos.x,
                Event::MouseMoved(delta) | Event::MouseWheel { delta, .. } => delta.x = -delta.x,
                _ => {}
            }
        }
    }

    fn output_hook(&mut self, ctx: &Context, output: &mut FullOutput) {
        let kept = ctx
            .data_mut(|d| d.remove_temp::<Vec<Rect>>(keep_id()))
            .unwrap_or_default();
        if !self.on {
            return;
        }
        let m = Mirroring {
            axis: self.axis,
            kept,
        };
        for clipped in &mut output.shapes {
            clipped.clip_rect = m.rect(clipped.clip_rect);
            m.shape(&mut clipped.shape);
        }
        if let Some(ime) = &mut output.platform_output.ime {
            ime.rect = m.rect(ime.rect);
            ime.cursor_rect = m.rect(ime.cursor_rect);
        }
        if let Some(update) = &mut output.platform_output.accesskit_update {
            let axis = f64::from(self.axis);
            for (_, node) in &mut update.nodes {
                if let Some(b) = node.bounds() {
                    node.set_bounds(egui::accesskit::Rect {
                        x0: axis - b.x1,
                        x1: axis - b.x0,
                        ..b
                    });
                }
            }
        }
    }
}

struct Mirroring {
    axis: f32,
    kept: Vec<Rect>,
}

impl Mirroring {
    fn x(&self, x: f32) -> f32 {
        self.axis - x
    }

    fn pos(&self, p: &mut Pos2) {
        p.x = self.x(p.x);
    }

    fn rect(&self, r: Rect) -> Rect {
        Rect::from_x_y_ranges(self.x(r.max.x)..=self.x(r.min.x), r.y_range())
    }

    /// How far `r` moves to stand where its mirror image would, without turning over.
    fn shift(&self, r: Rect) -> egui::Vec2 {
        vec2(self.axis - r.max.x - r.min.x, 0.0)
    }

    fn shape(&self, shape: &mut Shape) {
        if let Shape::Vec(shapes) = shape {
            for s in shapes {
                self.shape(s);
            }
            return;
        }
        let bounds = shape.visual_bounding_rect();
        if bounds.is_finite()
            && let Some(kept) = self
                .kept
                .iter()
                .find(|k| k.expand(1.0).contains_rect(bounds))
        {
            shape.translate(self.shift(*kept));
            return;
        }
        match shape {
            Shape::Noop | Shape::Vec(_) | Shape::Callback(_) => {}
            Shape::Circle(c) => self.pos(&mut c.center),
            Shape::Ellipse(e) => self.pos(&mut e.center),
            Shape::LineSegment { points, .. } => points.iter_mut().for_each(|p| self.pos(p)),
            Shape::Path(path) => path.points.iter_mut().for_each(|p| self.pos(p)),
            Shape::QuadraticBezier(b) => b.points.iter_mut().for_each(|p| self.pos(p)),
            Shape::CubicBezier(b) => b.points.iter_mut().for_each(|p| self.pos(p)),
            Shape::Rect(r) => {
                r.rect = self.rect(r.rect);
                let c = r.corner_radius;
                r.corner_radius = CornerRadius {
                    nw: c.ne,
                    ne: c.nw,
                    sw: c.se,
                    se: c.sw,
                };
            }
            Shape::Text(text) => self.text(text),
            Shape::Mesh(mesh) => self.mesh(std::sync::Arc::make_mut(mesh)),
        }
    }

    /// Plain colour (gradients, hand-drawn marks) is reflected; a picture only moves.
    fn mesh(&self, mesh: &mut Mesh) {
        if mesh.texture_id == TextureId::default() {
            mesh.vertices.iter_mut().for_each(|v| self.pos(&mut v.pos));
        } else {
            mesh.translate(self.shift(mesh.calc_bounds()));
        }
    }

    fn text(&self, text: &mut TextShape) {
        let r = text.galley.rect;
        // One line goes where its ink's mirror image is. An Arabic word's glyphs can stand
        // outside the galley's rect (its rect narrower than its ink): placed by the rect, the ink
        // lost its inset on the far side — «عرض» 4 px from a field's frame (BUG-123).
        let (left, right) = match ink_x(&text.galley) {
            Some(ink) if text.galley.rows.len() < 2 => ink,
            _ => (r.min.x, r.max.x),
        };
        text.pos.x = self.axis - text.pos.x - left - right;
        if text.angle != 0.0 || text.galley.rows.len() < 2 {
            return;
        }
        let galley = std::sync::Arc::make_mut(&mut text.galley);
        // Every line to the right edge of the block.
        for row in &mut galley.rows {
            row.pos.x = r.min.x + r.max.x - row.pos.x - row.size.x;
        }
        // An Arabic paragraph: egui wrapped its visual order, so its first rows hold its end.
        let mut start = 0;
        for i in 0..galley.rows.len() {
            if galley.rows[i].ends_with_newline || i + 1 == galley.rows.len() {
                read_from_the_top(&mut galley.rows[start..=i]);
                start = i + 1;
            }
        }
    }
}

/// Where a galley's glyphs are drawn, left and right, in its own coordinates; `None` without
/// any (an empty line). Zero-width marks are left out.
fn ink_x(galley: &egui::Galley) -> Option<(f32, f32)> {
    galley
        .rows
        .iter()
        .flat_map(|row| {
            row.row
                .glyphs
                .iter()
                .filter(|g| g.advance_width > 0.0)
                .map(move |g| (row.pos.x + g.pos.x, row.pos.x + g.pos.x + g.advance_width))
        })
        .reduce(|(l, r), (gl, gr)| (l.min(gl), r.max(gr)))
}

/// Puts the rows of one wrapped right-to-left paragraph in reading order, top to bottom.
fn read_from_the_top(rows: &mut [egui::epaint::text::PlacedRow]) {
    let rtl = rows
        .iter()
        .any(|r| r.glyphs.iter().any(|g| crate::i18n::is_rtl_char(g.chr)));
    if rows.len() < 2 || !rtl {
        return;
    }
    let tops: Vec<f32> = rows.iter().map(|r| r.pos.y).collect();
    for (row, top) in rows.iter_mut().rev().zip(tops) {
        row.pos.y = top;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::{RectShape, Stroke, StrokeKind};
    use egui::{Color32, pos2};

    fn mirroring() -> Mirroring {
        Mirroring {
            axis: 1000.0,
            kept: vec![Rect::from_min_max(pos2(100.0, 0.0), pos2(300.0, 50.0))],
        }
    }

    #[test]
    fn a_rect_goes_to_the_other_side_with_its_corners() {
        let mut shape = Shape::Rect(RectShape::new(
            Rect::from_min_max(pos2(10.0, 5.0), pos2(110.0, 25.0)),
            CornerRadius {
                nw: 8,
                ne: 0,
                sw: 4,
                se: 0,
            },
            Color32::RED,
            Stroke::NONE,
            StrokeKind::Inside,
        ));
        mirroring().shape(&mut shape);
        let Shape::Rect(r) = shape else {
            panic!("a rect")
        };
        assert_eq!(
            r.rect,
            Rect::from_min_max(pos2(890.0, 5.0), pos2(990.0, 25.0))
        );
        assert_eq!((r.corner_radius.ne, r.corner_radius.se), (8, 4));
    }

    #[test]
    fn a_chevron_drawn_as_a_path_points_the_other_way() {
        let mut shape = Shape::line_segment([pos2(500.0, 0.0), pos2(510.0, 10.0)], Stroke::NONE);
        mirroring().shape(&mut shape);
        let Shape::LineSegment { points, .. } = shape else {
            panic!("a line")
        };
        assert_eq!(points, [pos2(500.0, 0.0), pos2(490.0, 10.0)]);
    }

    #[test]
    fn a_kept_place_moves_as_a_whole() {
        // A QR module at the left of a kept card stays at the card's left.
        let mut shape = Shape::rect_filled(
            Rect::from_min_size(pos2(110.0, 10.0), vec2(5.0, 5.0)),
            0,
            Color32::BLACK,
        );
        mirroring().shape(&mut shape);
        let Shape::Rect(r) = shape else {
            panic!("a rect")
        };
        // The card 100..300 stands at 700..900; the module 10 px into it.
        assert_eq!(r.rect.min, pos2(710.0, 10.0));
    }

    /// A frame of 800 × 600 points in a context that reads right to left.
    fn mirrored() -> Context {
        let ctx = Context::default();
        ctx.add_plugin(Mirror {
            rtl: || true,
            ..Default::default()
        });
        ctx
    }

    fn input(events: Vec<Event>) -> RawInput {
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            events,
            ..Default::default()
        }
    }

    /// One frame; what it painted, after the mirror.
    fn frame(
        ctx: &Context,
        events: Vec<Event>,
        ui: impl FnMut(&mut Ui),
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut out = ctx.run_ui(input(events), ui);
        out.textures_delta.clear();
        out.shapes
    }

    #[test]
    fn a_click_on_the_right_lands_on_what_is_laid_out_on_the_left() {
        let ctx = mirrored();
        let mut clicked = false;
        let at = pos2(800.0 - 20.0, 20.0);
        // An area is measured unseen on its first frame; then the pointer comes, presses, lets go.
        let frames = [
            vec![],
            vec![Event::PointerMoved(at)],
            vec![Event::PointerMoved(at)],
            vec![Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
            vec![Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ];
        for events in frames {
            frame(&ctx, events, |ui| {
                egui::Area::new(Id::new("button"))
                    .fixed_pos(Pos2::ZERO)
                    .show(ui.ctx(), |ui| {
                        let (rect, resp) =
                            ui.allocate_exact_size(vec2(40.0, 40.0), egui::Sense::click());
                        assert_eq!(rect.min, Pos2::ZERO);
                        clicked |= resp.clicked();
                    });
            });
        }
        assert!(clicked);
    }

    #[test]
    fn a_field_cursor_stays_at_the_end_of_its_text() {
        let ctx = mirrored();
        let id = Id::new("field");
        let mut text = String::from("@alice");
        let mut shapes = vec![];
        for _ in 0..2 {
            shapes = frame(&ctx, vec![], |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let mut state =
                        egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(
                            egui::text::CCursor::new(6),
                        )));
                    state.store(ui.ctx(), id);
                    ui.memory_mut(|m| m.request_focus(id));
                    text_edit(ui, egui::TextEdit::singleline(&mut text).id(id));
                });
            });
        }
        let text_rect = shapes
            .iter()
            .find_map(|c| match &c.shape {
                Shape::Text(t) if t.galley.job.text == "@alice" => {
                    Some(t.galley.rect.translate(t.pos.to_vec2()))
                }
                _ => None,
            })
            .expect("the field's text");
        let cursor_x = shapes
            .iter()
            .find_map(|c| match c.shape {
                Shape::LineSegment { points: [a, b], .. } if a.x == b.x && b.y - a.y > 8.0 => {
                    Some(a.x)
                }
                _ => None,
            })
            .expect("the cursor");
        // Right to left the text stands at the right of the field, read as typed, and the
        // cursor after its last letter — not reflected to its first.
        assert!(text_rect.min.x > 400.0, "{text_rect:?}");
        assert!(
            (cursor_x - text_rect.max.x).abs() < 3.0,
            "{cursor_x} {text_rect:?}"
        );
    }

    #[test]
    fn the_window_without_bounds_stays_unbounded() {
        let m = mirroring();
        assert_eq!(m.rect(Rect::EVERYTHING), Rect::EVERYTHING);
        assert_eq!(
            m.rect(Rect::from_min_max(pos2(0.0, 0.0), pos2(250.0, 9.0))),
            Rect::from_min_max(pos2(750.0, 0.0), pos2(1000.0, 9.0))
        );
    }
}
