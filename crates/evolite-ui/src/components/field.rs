//! input/text · input/password · input/amount with a label above.

use super::prelude::*;

/// Label above a field: 12 Medium, muted.
pub fn field_label(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(
        egui::RichText::new(crate::i18n::t(text))
            .font(sans(12.0, Weight::Medium))
            .color(p.muted),
    );
    // It is also the field's name for a screen reader (BUG-41).
    crate::a11y::name_next_field(ui, text);
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
            let resp = ui.add(
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
            );
            crate::a11y::take_field_name(ui, &resp);
            crate::a11y::field_drawn(ui, &resp);
            resp
        })
        .inner
}

/// What the amount field reports back: the edit, whether "Max" was pressed and whether the
/// unit was switched.
pub struct AmountResponse {
    pub response: Response,
    pub all: bool,
    pub switched: bool,
}

/// The unit switch of an amount field (FEAT-0008, `field/unit-switch`): the amount's unit or
/// the currency of «Fiat rate». `off`: why the currency cannot be chosen now.
pub struct UnitChoice<'a> {
    pub currency: &'a str,
    pub in_currency: &'a mut bool,
    pub off: Option<&'a str>,
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
    amount_field_in(ui, p, id, value, with_all, None)
}

/// [`amount_field`] with, when `unit` is given, the switch between the unit and the currency
/// in place of the unit's name; the placeholder is then in the unit chosen.
pub fn amount_field_in(
    ui: &mut Ui,
    p: &Palette,
    id: &str,
    value: &mut String,
    with_all: bool,
    unit: Option<UnitChoice<'_>>,
) -> AmountResponse {
    let (mut all, mut switched) = (false, false);
    // The placeholder in the chosen unit and format: "0,0000", "0.0000", "0.00", "0".
    let hint = match &unit {
        Some(u) if *u.in_currency => crate::format::fiat_input_text(0),
        _ => crate::format::input_text(0),
    };
    let response = text_field(ui, p, id, value, &hint, FieldKind::Amount, |ui| {
        if with_all {
            all = super::mini_button(ui, p, tr!("Max")).clicked();
        } else {
            // As tall as the "Max" button, so every amount field is the same height.
            ui.allocate_exact_size(vec2(0.0, 27.0), Sense::hover());
        }
        match unit {
            Some(unit) => switched = unit_switch(ui, p, unit),
            None => {
                ui.label(
                    egui::RichText::new(crate::i18n::t(crate::format::unit_name()))
                        .font(mono(13.0, Weight::Regular))
                        .color(p.muted),
                );
            }
        }
    });
    AmountResponse {
        response,
        all,
        switched,
    }
}

/// `field/unit-switch`: «DASH | USD», 24 high on `raised`, the chosen one on `ink`. Each half
/// is a radio button by name for a screen reader, reached by Tab; the arrows switch. Whether
/// it switched.
fn unit_switch(ui: &mut Ui, p: &Palette, unit: UnitChoice<'_>) -> bool {
    let font = sans(12.0, Weight::SemiBold);
    let names = [crate::format::unit_name(), unit.currency];
    let widths = names.map(|n| text_width(ui, n, font.clone()) + 16.0);
    let (rect, _) = ui.allocate_exact_size(vec2(widths[0] + widths[1] + 4.0, 24.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(7), p.raised);
    let mut x = rect.left() + 2.0;
    let mut switched = false;
    let mut responses = Vec::new();
    for (i, (name, w)) in names.iter().zip(widths).enumerate() {
        let item = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(w, 20.0));
        x += w;
        let off = i == 1 && unit.off.is_some();
        let on = *unit.in_currency == (i == 1);
        let sense = if off { Sense::hover() } else { Sense::click() };
        let resp = ui.interact(item, ui.id().with(("unit-switch", i)), sense);
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::RadioButton, !off, on, *name)
        });
        if on {
            ui.painter().rect_filled(item, CornerRadius::same(6), p.ink);
        }
        let color = match (on, off) {
            (true, _) => p.on_ink,
            (false, true) => p.muted.gamma_multiply(0.45),
            (false, false) => p.muted,
        };
        // Codes and unit names are not translated and not mirrored (AC-T11).
        ui.painter().text(
            item.center(),
            Align2::CENTER_CENTER,
            *name,
            font.clone(),
            color,
        );
        let resp = match unit.off.filter(|_| off) {
            Some(why) => resp
                .on_hover_cursor(CursorIcon::NotAllowed)
                .on_hover_text(crate::i18n::t(why).into_owned()),
            None => resp.on_hover_cursor(CursorIcon::PointingHand),
        };
        if resp.clicked() && !on {
            switched = true;
        }
        responses.push(resp);
    }
    let arrows = responses.iter().any(Response::has_focus)
        && ui
            .input(|i| i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowRight));
    if arrows && (unit.off.is_none() || *unit.in_currency) {
        switched = true;
    }
    if switched {
        *unit.in_currency = !*unit.in_currency;
    }
    switched
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
