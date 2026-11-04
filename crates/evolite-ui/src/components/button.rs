//! button/primary · secondary · ghost, the small "Max" button and text actions with an icon.

use super::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    /// Red, white text: what cannot be undone ("Delete from the computer").
    Danger,
}

/// button/primary · secondary · ghost: 14 SemiBold, padding 11 × 18, radius 10.
pub fn button(
    ui: &mut Ui,
    a: &mut Assets,
    label: &str,
    kind: ButtonKind,
    width: Option<f32>,
) -> Response {
    button_enabled(ui, a, label, kind, width, true)
}

/// A button that can be disabled: dimmed, no hover, no clicks.
pub fn button_enabled(
    ui: &mut Ui,
    a: &mut Assets,
    label: &str,
    kind: ButtonKind,
    width: Option<f32>,
    enabled: bool,
) -> Response {
    let p = a.p;
    let font = sans(14.0, Weight::SemiBold);
    let w = width.unwrap_or_else(|| text_width(ui, label, font.clone()) + 36.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 41.0), sense);
    named(&resp, egui::WidgetType::Button, enabled, label);
    let hovered = enabled && resp.hovered();
    let (fill, stroke, color) = match kind {
        ButtonKind::Primary => (
            if hovered {
                p.ink.gamma_multiply(0.9)
            } else {
                p.ink
            },
            Color32::TRANSPARENT,
            p.on_ink,
        ),
        ButtonKind::Secondary => (if hovered { p.raised } else { p.surface }, p.line, p.text),
        ButtonKind::Ghost => (
            if hovered {
                p.raised
            } else {
                Color32::TRANSPARENT
            },
            Color32::TRANSPARENT,
            p.text,
        ),
        ButtonKind::Danger => (
            if hovered {
                p.danger_fill.gamma_multiply(0.9)
            } else {
                p.danger_fill
            },
            Color32::TRANSPARENT,
            Color32::WHITE,
        ),
    };
    let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(10),
        dim(fill),
        Stroke::new(1.0, dim(stroke)),
        StrokeKind::Inside,
    );
    paint_text(
        ui,
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        font,
        dim(color),
    );
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// A small button inside a field ("Max"): surface, 1 px line, radius 8, 12 SemiBold.
pub fn mini_button(ui: &mut Ui, p: &Palette, label: &str) -> Response {
    let font = sans(12.0, Weight::SemiBold);
    let (rect, resp) = ui.allocate_exact_size(
        vec2(text_width(ui, label, font.clone()) + 20.0, 27.0),
        Sense::click(),
    );
    named(&resp, egui::WidgetType::Button, true, label);
    let fill = if resp.hovered() { p.raised } else { p.surface };
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        fill,
        Stroke::new(1.0, p.line),
        StrokeKind::Inside,
    );
    paint_text(
        ui,
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        font,
        p.text,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// A text action with a 16 px icon: "Hide", "Copy for 60 s", "About the nodes".
pub fn text_action(ui: &mut Ui, a: &mut Assets, icon: &str, label: &str) -> Response {
    let p = a.p;
    let resp = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // The words are part of the action: a selectable label would take their clicks.
            ui.style_mut().interaction.selectable_labels = false;
            ui.add(a.icons.icon(icon, p.muted, &p, 16.0));
            ui.label(txt(label, 13.0, Weight::Medium, p.muted));
        })
        .response
        .interact(Sense::click());
    named(&resp, egui::WidgetType::Button, true, label);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};

    /// Presses and lets go at `x` (from the action's left edge) over a few frames; whether the
    /// action took the click.
    fn click_text_action_at(x: f32) -> bool {
        let ctx = egui::Context::default();
        // The app's fonts; they take effect from the next frame, so the first one draws nothing.
        crate::theme::install_fonts(&ctx, crate::i18n::Lang::En, false);
        let mut a = Assets {
            p: crate::theme::DARK,
            icons: crate::icons::Icons::load(),
            avatars: Default::default(),
        };
        let mut clicked = false;
        let mut rect = Rect::NOTHING;
        let press = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        for frame in 0..6 {
            let at = pos2(rect.left() + x, rect.center().y);
            let events = match frame {
                0 | 1 => vec![],
                2 | 3 => vec![Event::PointerMoved(at)],
                4 => vec![press(at, true)],
                _ => vec![press(at, false)],
            };
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                if frame == 0 {
                    return;
                }
                let resp = text_action(ui, &mut a, "chevron-left", "Back to “Main”");
                rect = resp.rect;
                clicked |= resp.clicked();
            });
            out.textures_delta.clear();
        }
        clicked
    }

    #[test]
    fn a_text_action_answers_a_click_on_its_words_too() {
        assert!(click_text_action_at(8.0), "on the icon");
        assert!(click_text_action_at(60.0), "on the words");
    }
}
