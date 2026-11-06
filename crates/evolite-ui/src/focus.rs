//! A ring around what the keyboard is on (WCAG 2.4.7 "Focus visible"). The buttons, pills,
//! toggles and rows are painted by hand and egui draws no focus for them; rather than in every
//! component, this egui plugin draws it once a frame around the focused widget. Only while the
//! keyboard moved last (Tab, arrows, Enter or Space, a shortcut): a click puts the ring away,
//! as on the web.

use egui::{Context, Event, Id, Key, LayerId, Order, RawInput, Stroke, StrokeKind, Ui};

fn shown_id() -> Id {
    Id::new("focus-ring-shown")
}

/// Shows the ring around the focus from this frame on, as if the keyboard had moved it: a
/// destructive dialog opens with its focus on «Cancel» and shows what Enter will do (the
/// designer's rule b′). A click puts it away again.
pub fn show_ring(ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(shown_id(), true));
}

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
                // Enter or Space on a button, or a shortcut: the keyboard moved last too, so a
                // dialog opened so shows its opening focus (the designer, 2026-11-06).
                Event::Key {
                    key: Key::Enter | Key::Space,
                    pressed: true,
                    ..
                } => self.keyboard = true,
                Event::Key {
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.command || modifiers.ctrl || modifiers.alt => self.keyboard = true,
                Event::PointerButton { pressed: true, .. } | Event::Touch { .. } => {
                    self.keyboard = false;
                }
                _ => {}
            }
        }
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        if ui.ctx().data_mut(|d| d.remove_temp::<bool>(shown_id())) == Some(true) {
            self.keyboard = true;
        }
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

    /// The designer's rule b′: a destructive dialog opens with the ring on «Cancel», no Tab.
    #[test]
    fn a_ring_shown_on_purpose_needs_no_tab() {
        let ctx = Context::default();
        ctx.add_plugin(FocusRing::default());
        let focus_first = |ring: bool| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let (_, first) = ui.allocate_exact_size(vec2(80.0, 30.0), egui::Sense::click());
                first.request_focus();
                if ring {
                    show_ring(ui.ctx());
                }
            });
            out.textures_delta.clear();
            out.shapes
        };
        focus_first(false);
        assert_eq!(rings(&focus_first(false)), 0, "focused by the app: no ring");
        assert_eq!(
            rings(&focus_first(true)),
            1,
            "shown on purpose: the ring at once"
        );
        assert_eq!(rings(&focus_first(false)), 1, "and it stays until a click");
    }

    /// The designer, 2026-11-06: an ordinary dialog shows the ring around its opening focus
    /// when the keyboard moved last — also by Enter or Space on a button, or a shortcut, not
    /// only by Tab. After a click it does not.
    #[test]
    fn enter_space_or_a_shortcut_after_a_click_show_the_ring() {
        let key = |key: Key, modifiers: egui::Modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let click = Event::PointerButton {
            pos: pos2(300.0, 250.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        };
        for (what, event) in [
            ("Enter", key(Key::Enter, Default::default())),
            ("Space", key(Key::Space, Default::default())),
            ("Ctrl+N", key(Key::N, egui::Modifiers::COMMAND)),
        ] {
            let ctx = Context::default();
            ctx.add_plugin(FocusRing::default());
            let focus_first = |events: Vec<Event>| {
                let input = RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))),
                    events,
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    let (_, first) = ui.allocate_exact_size(vec2(80.0, 30.0), egui::Sense::click());
                    first.request_focus();
                });
                out.textures_delta.clear();
                out.shapes
            };
            focus_first(vec![]);
            assert_eq!(
                rings(&focus_first(vec![click.clone()])),
                0,
                "a click: no ring"
            );
            assert_eq!(rings(&focus_first(vec![event])), 1, "{what}: the ring");
        }
    }
}
