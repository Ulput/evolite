//! input/text · input/password · input/amount with a label above.

use super::prelude::*;

/// Label above a field: 12 Medium, muted.
pub fn field_label(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(
        egui::RichText::new(crate::i18n::t(text))
            .font(sans(12.0, Weight::Medium))
            .color(p.muted),
    );
}

#[derive(Clone, Copy)]
pub enum FieldKind {
    /// input/text: 14 Regular, padding 12 × 14.
    Text,
    /// Like Text, but the characters are hidden.
    Password,
    /// input/amount: 20 mono Medium, padding 8 (14 on the left), room for a unit and "Max".
    Amount,
}

/// egui's undo history keeps a copy of every state of a field's text in
/// memory; for a password or a phrase word it is dropped after each frame,
/// so the only copy is the form's own, wiped with it.
pub fn forget_typing(ctx: &egui::Context, id: egui::Id) {
    if let Some(mut state) = egui::text_edit::TextEditState::load(ctx, id) {
        state.clear_undoer();
        state.store(ctx, id);
    }
}

/// input/text · input/amount: raised field, 1 px line (the text colour while focused), radius
/// 10. `end` paints extra content on the right (a unit, a button) inside the field. A password
/// field has "Show" / "Hide" at its end: a long password is typed blind twice otherwise.
pub fn text_field(
    ui: &mut Ui,
    p: &Palette,
    id: &str,
    value: &mut String,
    hint: &str,
    kind: FieldKind,
    end: impl FnOnce(&mut Ui),
) -> Response {
    let (font, margin) = match kind {
        FieldKind::Text | FieldKind::Password => {
            (sans(14.0, Weight::Regular), Margin::symmetric(14, 12))
        }
        FieldKind::Amount => (
            mono(20.0, Weight::Medium),
            Margin {
                left: 14,
                right: 8,
                top: 8,
                bottom: 8,
            },
        ),
    };
    let id = ui.make_persistent_id(id);
    let focused = ui.memory(|m| m.has_focus(id));
    let password = matches!(kind, FieldKind::Password);
    let shown_id = id.with("shown");
    let mut shown = password && ui.data(|d| d.get_temp::<bool>(shown_id)).unwrap_or(false);
    Frame::new()
        .fill(p.raised)
        .stroke(Stroke::new(
            1.0,
            if focused { p.text } else { p.field_line },
        ))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    end(ui);
                    if password {
                        let label = if shown { tr!("Hide") } else { tr!("Show") };
                        let toggle = ui
                            .add(
                                egui::Label::new(txt(label, 12.0, Weight::Medium, p.muted))
                                    .sense(Sense::click()),
                            )
                            .on_hover_cursor(CursorIcon::PointingHand);
                        named(&toggle, egui::WidgetType::Button, true, label);
                        if toggle.clicked() {
                            shown = !shown;
                            ui.data_mut(|d| d.insert_temp(shown_id, shown));
                        }
                    }
                    let response = crate::rtl::text_edit(
                        ui,
                        egui::TextEdit::singleline(value)
                            .password(password && !shown)
                            .id(id)
                            .hint_text(
                                egui::RichText::new(crate::i18n::t(hint))
                                    .font(font.clone())
                                    .color(p.muted),
                            )
                            .font(font)
                            .text_color(p.text)
                            .frame(Frame::NONE)
                            .desired_width(f32::INFINITY),
                    );
                    if matches!(kind, FieldKind::Password) {
                        forget_typing(ui.ctx(), id);
                    }
                    response
                })
                .inner
            })
            .inner
        })
        .inner
}

/// input/text over several lines: the same frame as [`text_field`], `rows` lines high at least.
pub fn text_area(
    ui: &mut Ui,
    p: &Palette,
    id: &str,
    value: &mut String,
    hint: &str,
    rows: usize,
) -> Response {
    let font = sans(14.0, Weight::Regular);
    let id = ui.make_persistent_id(id);
    let focused = ui.memory(|m| m.has_focus(id));
    Frame::new()
        .fill(p.raised)
        .stroke(Stroke::new(
            1.0,
            if focused { p.text } else { p.field_line },
        ))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::TextEdit::multiline(value)
                    .id(id)
                    .hint_text(
                        egui::RichText::new(crate::i18n::t(hint))
                            .font(font.clone())
                            .color(p.muted),
                    )
                    .font(font)
                    .text_color(p.text)
                    .frame(Frame::NONE)
                    .desired_rows(rows)
                    .desired_width(f32::INFINITY),
            )
        })
        .inner
}

/// What the amount field reports back: the edit and whether "Max" was pressed.
pub struct AmountResponse {
    pub response: Response,
    pub all: bool,
}

/// input/amount: a DASH amount in mono 20 with the unit and, when `with_all`, a "Max" button
/// that the caller answers by filling in the whole balance.
pub fn amount_field(
    ui: &mut Ui,
    p: &Palette,
    id: &str,
    value: &mut String,
    with_all: bool,
) -> AmountResponse {
    let mut all = false;
    // The placeholder in the chosen unit and format: "0,0000", "0.0000", "0.00", "0".
    let hint = crate::format::input_text(0);
    let response = text_field(ui, p, id, value, &hint, FieldKind::Amount, |ui| {
        if with_all {
            all = super::mini_button(ui, p, tr!("Max")).clicked();
        } else {
            // As tall as the "Max" button, so every amount field is the same height.
            ui.allocate_exact_size(vec2(0.0, 27.0), Sense::hover());
        }
        ui.label(
            egui::RichText::new(crate::i18n::t(crate::format::unit_name()))
                .font(mono(13.0, Weight::Regular))
                .color(p.muted),
        );
    });
    AmountResponse { response, all }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text_edit::TextEditState;

    /// Types "secret" into a focused field over a few frames; `forget` says
    /// whether the field drops its undo history like a password field.
    fn type_secret(forget: bool) -> (String, TextEditState) {
        let ctx = egui::Context::default();
        let id = egui::Id::new("secret");
        let mut value = String::new();
        for frame in 0..4 {
            let typed = frame == 1 || frame == 2;
            let input = egui::RawInput {
                time: Some(frame as f64),
                events: if typed {
                    vec![egui::Event::Text("secret".into())]
                } else {
                    Vec::new()
                },
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                ui.memory_mut(|m| m.request_focus(id));
                ui.add(egui::TextEdit::singleline(&mut value).id(id));
                if forget {
                    forget_typing(ui.ctx(), id);
                }
            });
            output.textures_delta.clear();
        }
        (value, TextEditState::load(&ctx, id).expect("state"))
    }

    #[test]
    fn a_secret_field_keeps_no_undo_history() {
        let (plain, state) = type_secret(false);
        assert_eq!(plain, "secretsecret");
        let undoer = state.undoer();
        assert!(
            undoer.is_in_flux() || undoer.has_undo(&(Default::default(), String::new())),
            "an ordinary field remembers what was typed"
        );

        let (secret, state) = type_secret(true);
        assert_eq!(secret, "secretsecret");
        let undoer = state.undoer();
        assert!(!undoer.is_in_flux());
        assert!(!undoer.has_undo(&(Default::default(), String::new())));
    }
}
