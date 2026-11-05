//! Not in the design: a check of how egui draws Arabic and Hindi.

use super::{Screen, txt};
use crate::components::{Assets, ButtonKind, button, card, page_header};
use crate::context::Ctx;
use crate::theme::Weight;
use egui::{Margin, Ui};

/// Not in the design: how egui draws Arabic (right to left, joined letters) and Hindi
/// (conjuncts, vowel signs). The "expected" line is an image of the correct rendering
/// would be needed for a fair comparison, so the screen shows the raw text and what to look for.
pub fn probe(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let p = a.p;
    ui.spacing_mut().item_spacing.y = 18.0;
    page_header(ui, a, tr!("Scripts check"), None, |ui, a| {
        if button(ui, a, tr!("Back"), ButtonKind::Secondary, None).clicked() {
            cx.go(Screen::Welcome);
        }
    });
    ui.label(txt(
        tr!("evolite promises 15 languages, among them Arabic (right to left) and Hindi. Below are interface strings drawn by egui."),
        14.0,
        Weight::Regular,
        p.muted,
    ));
    for (lang, sample, check) in [
        (
            tr!("Arabic"),
            "إرسال ٠٫٢٥ داش إلى @alice",
            tr!("The letters should join, the line reads right to left, @alice left to right."),
        ),
        (
            tr!("Hindi"),
            "भेजें — पुष्टि हो गई",
            tr!(
                "“ष्टि” and “र्ई” should form ligatures, with vowel signs above and below the letters."
            ),
        ),
        (
            tr!("Russian"),
            tr!("Send 0.25 DASH to @alice"),
            tr!("For comparison: Cyrillic from Inter."),
        ),
    ] {
        card(&p, Margin::same(20)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(txt(lang, 13.0, Weight::Medium, p.muted));
            ui.label(txt(sample, 28.0, Weight::Regular, p.text));
            ui.label(txt(check, 12.0, Weight::Regular, p.muted));
        });
    }
}
