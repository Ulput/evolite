//! The note to a payment: its field, its counter, how the last look shows it.

use super::*;

/// N3 under the note field (FEAT-0003, FR-8).
pub const NOTE_PRIVACY: &str = tr!(
    "Kept only on this computer, encrypted with the history. It never goes to the network and the recipient does not see it. Restoring from the recovery phrase does not bring it back."
);

/// "N / 140": the characters of a note as a person sees them; numbers only, no words, and no
/// thousands to group (AC-T10).
pub fn note_counter(note: &str) -> String {
    format!(
        "{} / {}",
        evolite_core::note_text::graphemes(note),
        evolite_api::NOTE_MAX_GRAPHEMES
    )
}

/// "Note · only for you" (Figma 03, AC-D1): optional, up to 140 characters with the counter
/// on the right (red at the limit), cut with N4 when more was pasted, and the privacy line.
pub(super) fn note_field(ui: &mut Ui, a: &mut Assets, form: &mut SendForm) {
    let p = a.p;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let counter = note_counter(&form.note);
        let full =
            evolite_core::note_text::graphemes(&form.note) >= evolite_api::NOTE_MAX_GRAPHEMES;
        ui.horizontal(|ui| {
            field_label(ui, &p, tr!("Note · only for you"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(&counter)
                        .font(mono(12.0, Weight::Regular))
                        .color(if full { p.danger } else { p.muted }),
                );
            });
        });
        crate::a11y::name_next_field(ui, tr!("Note · only for you"));
        let mut typed = form.note.clone();
        let resp = crate::components::text_area(
            ui,
            &p,
            "send-note",
            &mut typed,
            tr!("What the payment is for"),
            1,
        );
        if resp.changed() {
            form.type_note(&typed);
            form.result = None;
        }
        if form.note_cut {
            // N4; the number comes first in the key (`plural`), the language puts it where it
            // goes.
            let most = evolite_api::NOTE_MAX_GRAPHEMES as u32;
            let cut = crate::text::plural(
                most,
                tr!("character is all a note holds: the rest was cut off."),
                tr!("characters is all a note holds: the rest was cut off."),
            );
            ui.add(egui::Label::new(txt(&cut, 12.0, Weight::Regular, p.danger)).wrap());
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(a.icons.icon("lock", p.muted, &p, 14.0));
            ui.add(egui::Label::new(txt(NOTE_PRIVACY, 12.0, Weight::Regular, p.muted)).wrap());
        });
    });
}

/// «Note» in the summary (AC-D1, Figma 08): the last row, the note in one line cut with «…», in
/// its own direction.
pub(super) fn summary_note_row(ui: &mut Ui, p: &crate::theme::Palette, note: &str) {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), 16.0), egui::Sense::hover());
    // Painted words say nothing to a screen reader: the row reads «Note: …», as `kv_row`.
    let spoken = format!("{}: {}", crate::i18n::spoken(tr!("Note")), note);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
    let label = crate::components::paint_text(
        ui,
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        tr!("Note"),
        sans(13.0, Weight::Regular),
        p.muted,
    );
    let room = rect.right() - label.right() - 16.0;
    if room <= 0.0 {
        return;
    }
    let mut job = egui::text::LayoutJob::single_section(
        crate::i18n::t(&crate::i18n::own(note)).into_owned(),
        egui::TextFormat::simple(sans(13.0, Weight::Medium), p.text),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: room,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(job);
    let at = egui::pos2(
        rect.right() - galley.size().x,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(at, galley, p.text);
}

/// The note in «Check the payment» (AC-D2): `raised`, r12, after the total and before «A
/// payment cannot be undone.»; its heading with the `document` icon, the note in the person's
/// own direction, and N15. A note a `dash:` link brought (FEAT-0006 FR-7) is headed «Note from
/// the link» and says it is the QR's text, not a checked name of the recipient.
pub(super) fn confirm_note(ui: &mut Ui, a: &mut Assets, note: &str, from_link: bool) {
    let p = a.p;
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(a.icons.icon("document", p.muted, &p, 14.0));
                let heading = if from_link {
                    tr!("Note from the link")
                } else {
                    tr!("Note · only for you")
                };
                ui.label(txt(heading, 12.0, Weight::Medium, p.muted));
            });
            ui.add(
                egui::Label::new(txt(&crate::i18n::own(note), 14.0, Weight::Regular, p.text))
                    .wrap(),
            );
            if from_link {
                ui.add(
                    egui::Label::new(txt(
                        tr!("This is text from the QR code, not a verified recipient name."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ))
                    .wrap(),
                );
            }
            ui.add(
                egui::Label::new(txt(
                    tr!("Saved to the payment on this computer; it does not go to the network."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ))
                .wrap(),
            );
        });
}
