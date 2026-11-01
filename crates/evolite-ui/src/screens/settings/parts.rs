//! What every settings section is made of: a heading, groups of rows, the control on the right of a row.

use super::txt;
use crate::components::{
    Assets, ButtonKind, button, card, inner, section_label, segmented, segmented_with_off,
    text_width, toggle,
};
use crate::theme::{Palette, Weight, mono, sans};
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Layout, Margin, Sense, Stroke, StrokeKind,
    Ui, vec2,
};

pub(super) fn heading(ui: &mut Ui, p: &Palette, title: &str, lead: Option<&str>) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(txt(title, 20.0, Weight::Bold, p.text));
        if let Some(lead) = lead {
            ui.add(egui::Label::new(txt(lead, 13.0, Weight::Regular, p.muted)).wrap());
        }
    });
}

/// What sits on the right of a settings row.
pub(super) enum Control<'a> {
    Toggle(&'a mut bool),
    Segmented(&'a [&'a str], &'a mut usize),
    /// [`Control::Segmented`] where an item with `Some(hint)` cannot be chosen.
    SegmentedWithOff(&'a [&'a str], &'a mut usize, &'a [Option<&'a str>]),
    /// A value with a chevron, e.g. "8 nodes ›".
    Value(&'a str, bool),
    /// A value only to read (no chevron, not clickable); `true`: in mono. Empty: nothing.
    Text(&'a str, bool),
    /// A lock and "always": cannot be changed.
    Locked,
    Button(&'a str, ButtonKind),
    /// Several buttons side by side; the index of the one clicked goes to the second field.
    Buttons(&'a [(&'a str, ButtonKind)], &'a mut Option<usize>),
    Danger(&'a str),
}

/// A group: mono uppercase label over a card of rows divided by lines.
pub(super) fn group(
    ui: &mut Ui,
    a: &mut Assets,
    label: &str,
    rows: Vec<(&str, &str, Control)>,
) -> Option<usize> {
    let p = a.p;
    let mut clicked = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        section_label(ui, &p, label);
        let width = ui.available_width();
        card(&p, Margin::ZERO).show(ui, |ui| {
            ui.set_width(inner(width, 0));
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, (title, sub, control)) in rows.into_iter().enumerate() {
                if i > 0 {
                    let (line, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().rect_filled(line, 0, p.line);
                }
                if row(ui, a, title, sub, control) {
                    clicked = Some(i);
                }
            }
        });
    });
    clicked
}

/// One row: title 14 SemiBold and a wrapped 12 px line, the control on the right.
/// Returns true when its button or value was clicked.
fn row(ui: &mut Ui, a: &mut Assets, title: &str, sub: &str, control: Control) -> bool {
    let p = a.p;
    let mut clicked = false;
    egui::Frame::new()
        .inner_margin(Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Several buttons in a narrow window go under the text, not squeeze it.
            if matches!(control, Control::Buttons(..))
                && ui.available_width() - control_width(ui, &control) - 20.0 < 300.0
            {
                ui.spacing_mut().item_spacing.y = 12.0;
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(txt(title, 14.0, Weight::SemiBold, p.text));
                    ui.add(
                        egui::Label::new(
                            txt(sub, 12.0, Weight::Regular, p.muted).line_height(Some(17.4)),
                        )
                        .wrap(),
                    );
                });
                ui.horizontal(|ui| {
                    // Left to right here: the first button first.
                    if let Control::Buttons(buttons, which) = control {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for (i, (label, kind)) in buttons.iter().enumerate() {
                            if button(ui, a, label, *kind, None).clicked() {
                                *which = Some(i);
                                clicked = true;
                            }
                        }
                    }
                });
                return;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 20.0;
                // The control first (right to left) so the text knows how much room is left.
                let text_w = ui.available_width() - control_width(ui, &control) - 20.0;
                ui.vertical(|ui| {
                    ui.set_width(text_w);
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(txt(title, 14.0, Weight::SemiBold, p.text));
                    ui.add(
                        egui::Label::new(
                            txt(sub, 12.0, Weight::Regular, p.muted).line_height(Some(17.4)),
                        )
                        .wrap(),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    clicked = draw_control(ui, a, title, control);
                });
            });
        });
    clicked
}

fn control_width(ui: &Ui, control: &Control) -> f32 {
    match control {
        Control::Toggle(_) => 40.0,
        Control::Segmented(items, _) | Control::SegmentedWithOff(items, _, _) => {
            8.0 + items
                .iter()
                .map(|s| text_width(ui, s, sans(13.0, Weight::SemiBold)) + 24.0 + 4.0)
                .sum::<f32>()
        }
        Control::Value(v, _) => text_width(ui, v, sans(13.0, Weight::Medium)) + 22.0,
        Control::Text(v, _) => text_width(ui, v, sans(13.0, Weight::Medium)),
        Control::Locked => 70.0,
        Control::Button(l, _) | Control::Danger(l) => {
            text_width(ui, l, sans(14.0, Weight::SemiBold)) + 36.0
        }
        Control::Buttons(buttons, _) => buttons
            .iter()
            .map(|(l, _)| text_width(ui, l, sans(14.0, Weight::SemiBold)) + 36.0 + 8.0)
            .sum(),
    }
}

fn draw_control(ui: &mut Ui, a: &mut Assets, title: &str, control: Control) -> bool {
    let p = a.p;
    match control {
        Control::Toggle(on) => {
            if toggle(ui, &p, *on, title).clicked() {
                *on = !*on;
            }
            false
        }
        Control::Segmented(items, selected) => {
            segmented(ui, &p, items, selected, true);
            false
        }
        Control::SegmentedWithOff(items, selected, off) => {
            segmented_with_off(ui, &p, items, selected, true, off);
            false
        }
        Control::Value(value, mono_value) => {
            let resp = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add(a.icons.icon("chevron-right", p.muted, &p, 16.0));
                    let font = if mono_value {
                        mono(13.0, Weight::Medium)
                    } else {
                        sans(13.0, Weight::Medium)
                    };
                    ui.label(
                        egui::RichText::new(crate::i18n::t(value))
                            .font(font)
                            .color(p.text),
                    );
                })
                .response
                .interact(Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            crate::components::named(
                &resp,
                egui::WidgetType::Button,
                true,
                &format!("{} · {}", crate::i18n::t(title), crate::i18n::t(value)),
            );
            resp.clicked()
        }
        Control::Text(value, mono_value) => {
            let font = if mono_value {
                mono(13.0, Weight::Medium)
            } else {
                sans(13.0, Weight::Medium)
            };
            ui.label(
                egui::RichText::new(crate::i18n::t(value))
                    .font(font)
                    .color(p.text),
            );
            false
        }
        Control::Locked => {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(txt(tr!("always"), 13.0, Weight::Medium, p.muted));
            ui.add(a.icons.icon("lock", p.muted, &p, 16.0))
                .on_hover_text(
                    crate::i18n::t(tr!("Signature checks cannot be turned off")).into_owned(),
                );
            false
        }
        Control::Button(label, kind) => button(ui, a, label, kind, None).clicked(),
        Control::Buttons(buttons, which) => {
            ui.spacing_mut().item_spacing.x = 8.0;
            // Right to left: the last first, so they read in order.
            for (i, (label, kind)) in buttons.iter().enumerate().rev() {
                if button(ui, a, label, *kind, None).clicked() {
                    *which = Some(i);
                }
            }
            which.is_some()
        }
        Control::Danger(label) => {
            let font = sans(14.0, Weight::SemiBold);
            let w = text_width(ui, label, font.clone()) + 36.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 41.0), Sense::click());
            crate::components::named(&resp, egui::WidgetType::Button, true, label);
            let fill = if resp.hovered() {
                p.raised
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect(
                rect,
                CornerRadius::same(10),
                fill,
                Stroke::new(1.0, p.danger),
                StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                crate::i18n::t(label),
                font,
                p.danger,
            );
            resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
        }
    }
}
