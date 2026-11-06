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

/// The height of a [`button`], and so of a row of them.
pub const BUTTON_HEIGHT: f32 = 41.0;

/// A dialog's row of buttons: on the right, as high as its buttons. `add` draws them left to
/// right, in the order they are read: the secondary first («Cancel» when the dialog acts,
/// «Close» when it only shows), the primary last, at the right end — one order in every dialog
/// (designer review №8). Tab goes the same way, Cancel → primary (the designer, 2026-11-06:
/// egui's Tab follows the order widgets are drawn in), and in Arabic the mirrored frame turns
/// both over together. Enter takes the primary from a field, as before.
///
/// Right-aligned but drawn left to right: the row starts its width before its right edge, the
/// width the buttons took last frame; the first frame, or one whose buttons changed width, is
/// drawn again before it is shown (as `egui::Grid` does). A fixed height, not `with_layout`:
/// that one takes all the height left and centres the buttons in it, and a dialog keeps its
/// size and its top (BUG-39 №3, №13), so whatever comes under the row would grow the dialog
/// every frame.
pub fn button_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let id = ui.auto_id_with("button-row");
    let (_, row) = ui.allocate_space(vec2(ui.available_width(), BUTTON_HEIGHT));
    let width = ui.data(|d| d.get_temp::<f32>(id));
    let left = row.right() - width.unwrap_or(row.width());
    let mut buttons = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(left, row.top()), row.max))
            .layout(Layout::left_to_right(Align::Center)),
    );
    buttons.spacing_mut().item_spacing.x = 10.0;
    let inner = add(&mut buttons);
    let now = buttons.min_rect().width();
    if width.is_none_or(|w| (w - now).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(id, now));
        ui.ctx()
            .request_discard("a row of buttons changed its width");
    }
    inner
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
    let (rect, resp) = ui.allocate_exact_size(vec2(w, BUTTON_HEIGHT), sense);
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
    let row = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // The words are part of the action: a selectable label would take their clicks.
            ui.style_mut().interaction.selectable_labels = false;
            ui.add(a.icons.icon(icon, p.muted, &p, 16.0));
            ui.label(txt(label, 13.0, Weight::Medium, p.muted));
        })
        .response;
    let resp = crate::a11y::row_click(ui, &row, "text-action");
    named(&resp, egui::WidgetType::Button, true, label);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// What [`amount_toggle`] asks of the app: «Hide amounts» switched over (FEAT-0007).
const AMOUNTS_FLIP: &str = "amounts-flip";

/// amount-toggle (FEAT-0007): the «eye» beside a screen's main amount. It shows the action —
/// «Hide the amounts» while they are seen, «Show the amounts» while hidden — and a click, Enter
/// or Space switches the one «Hide amounts» of Settings → Security, for the whole app (the app
/// takes it with [`take_amounts_flip`]). `compact`: the icon alone, 28 × 28, its words in the
/// tooltip (the wallet menu, «Earlier addresses»); else icon 16 + 12 Medium, padding 8 × 6.
pub fn amount_toggle(ui: &mut Ui, a: &mut Assets, compact: bool) -> Response {
    let p = a.p;
    let hidden = crate::format::display().hide;
    let (icon, label) = if hidden {
        ("eye", tr!("Show the amounts"))
    } else {
        ("eye-off", tr!("Hide the amounts"))
    };
    let font = sans(12.0, Weight::Medium);
    let size = if compact {
        vec2(28.0, 28.0)
    } else {
        vec2(
            8.0 + 16.0 + 6.0 + text_width(ui, label, font.clone()) + 8.0,
            28.0,
        )
    };
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    named(&resp, egui::WidgetType::Button, true, label);
    let over = resp.hovered() || resp.has_focus();
    let color = if over { p.text } else { p.muted };
    if over {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), p.raised);
    }
    let icon_left = if compact {
        rect.center().x - 8.0
    } else {
        rect.left() + 8.0
    };
    let icon_rect = Rect::from_min_size(pos2(icon_left, rect.center().y - 8.0), vec2(16.0, 16.0));
    a.icons.icon(icon, color, &p, 16.0).paint_at(ui, icon_rect);
    if !compact {
        paint_text(
            ui,
            pos2(icon_rect.right() + 6.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            font,
            color,
        );
    }
    // The icon alone says its words on hover and on Tab focus.
    let resp = if compact {
        let words = crate::i18n::t(label).into_owned();
        if resp.has_focus() && !resp.hovered() {
            resp.show_tooltip_text(words.clone());
        }
        resp.on_hover_text(words)
    } else {
        resp
    };
    if resp.clicked() {
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new(AMOUNTS_FLIP), true));
        ui.ctx().request_repaint();
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Whether an [`amount_toggle`] was pressed since the last call; the app switches the setting.
pub fn take_amounts_flip(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new(AMOUNTS_FLIP)))
        .unwrap_or(false)
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
