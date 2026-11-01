//! A ring around what the keyboard is on (WCAG 2.4.7 "Focus visible"). The buttons, pills,
//! toggles and rows are painted by hand and egui draws no focus for them; rather than in every
//! component, this egui plugin draws it once a frame around the focused widget. Only while the
//! keyboard moves the focus (Tab, arrows): a click puts the ring away, as on the web.

use egui::{Context, Event, Id, Key, LayerId, Order, RawInput, Stroke, StrokeKind, Ui};

#[derive(Default)]
pub struct FocusRing {
    /// The last move was the keyboard's.
    keyboard: bool,
}

impl egui::Plugin for FocusRing {
    fn debug_name(&self) -> &'static str {
        "focus-ring"
    }

    fn input_hook(&mut self, _ctx: &Context, input: &mut RawInput) {
        for event in &input.events {
            match event {
                Event::Key {
                    key: Key::Tab | Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight,
                    pressed: true,
                    ..
                } => self.keyboard = true,
                Event::PointerButton { pressed: true, .. } | Event::Touch { .. } => {
                    self.keyboard = false;
                }
                _ => {}
            }
        }
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        if !self.keyboard {
            return;
        }
        let ctx = ui.ctx();
        let Some(id) = ctx.memory(|m| m.focused()) else {
            return;
        };
        // A text field shows its focus itself: the frame's line and the cursor.
        if egui::text_edit::TextEditState::load(ctx, id).is_some() {
            return;
        }
        let Some(resp) = ctx.read_response(id) else {
            return;
        };
        // The theme's accent (`theme::apply_visuals` puts it in the hyperlink colour).
        let color = ui.visuals().hyperlink_color;
        ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("focus-ring")))
            .rect_stroke(
                resp.rect.expand(2.0),
                12,
                Stroke::new(2.0, color),
                StrokeKind::Outside,
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, Shape, pos2, vec2};

    fn frame(ctx: &Context, events: Vec<Event>) -> Vec<egui::epaint::ClippedShape> {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            ui.allocate_exact_size(vec2(80.0, 30.0), egui::Sense::click());
            ui.allocate_exact_size(vec2(80.0, 30.0), egui::Sense::click());
        });
        out.textures_delta.clear();
        out.shapes
    }

    fn tab() -> Event {
        Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    fn rings(shapes: &[egui::epaint::ClippedShape]) -> usize {
        shapes
            .iter()
            .filter(|c| matches!(&c.shape, Shape::Rect(r) if r.stroke.width == 2.0))
            .count()
    }

    #[test]
    fn tab_shows_a_ring_and_a_click_puts_it_away() {
        let ctx = Context::default();
        ctx.add_plugin(FocusRing::default());
        frame(&ctx, vec![]);
        frame(&ctx, vec![tab()]);
        assert_eq!(rings(&frame(&ctx, vec![])), 1, "Tab: the ring is on");
        let click = Event::PointerButton {
            pos: pos2(300.0, 250.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        };
        assert_eq!(rings(&frame(&ctx, vec![click])), 0, "a click: no ring");
    }
}
