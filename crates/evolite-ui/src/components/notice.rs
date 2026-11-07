//! Explanations: a raised notice, a bordered warning, a one-line "example" note.

use super::prelude::*;

/// A short explanation on a raised background: icon 18 and wrapped 13 px text.
pub fn notice(ui: &mut Ui, a: &mut Assets, icon: &str, icon_color: Color32, text: &str) {
    let p = a.p;
    Frame::new()
        .fill(p.raised)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(a.icons.icon(icon, icon_color, &p, 18.0));
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(crate::i18n::t(text))
                            .font(sans(13.0, Weight::Regular))
                            .color(p.text)
                            .line_height(Some(18.0)),
                    )
                    .wrap(),
                );
            });
        });
}

/// [`notice()`] in parts, each under a small heading (13 SemiBold): what the phrase brings back
/// and what it does not, say (BUG-58).
pub fn notice_parts(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    icon_color: Color32,
    parts: &[(&str, &str)],
) {
    let p = a.p;
    Frame::new()
        .fill(p.raised)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(a.icons.icon(icon, icon_color, &p, 18.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, (heading, text)) in parts.iter().enumerate() {
                        if i > 0 {
                            ui.add_space(8.0);
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(crate::i18n::t(heading))
                                    .font(sans(13.0, Weight::SemiBold))
                                    .color(p.text)
                                    .line_height(Some(18.0)),
                            )
                            .wrap(),
                        );
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(crate::i18n::t(text))
                                    .font(sans(13.0, Weight::Regular))
                                    .color(p.text)
                                    .line_height(Some(18.0)),
                            )
                            .wrap(),
                        );
                    }
                });
            });
        });
}

/// The «attention» tone (calm states CS-1): a risk the person decides on, not an error — red is
/// for errors only. The warning icon in the yellow that holds 3:1 on raised in both themes:
/// `accent` (the light theme's `brand` #C27C00 is 2.9:1 on the cream; in the dark one they are
/// one colour).
pub fn attention_color(p: &Palette) -> Color32 {
    p.accent
}

/// [`notice()`] in the «attention» tone ([`attention_color`]).
pub fn attention_notice(ui: &mut Ui, a: &mut Assets, text: &str) {
    let color = attention_color(&a.p);
    notice(ui, a, "warning", color, text);
}

/// One line in the «attention» tone under a field: the warning icon 14 and 12 px text in the
/// text colour, wrapped.
pub fn attention_line(ui: &mut Ui, a: &mut Assets, text: &str) {
    let p = a.p;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add(a.icons.icon("warning", attention_color(&p), &p, 14.0));
        ui.add(egui::Label::new(txt(text, 12.0, Weight::Regular, p.text)).wrap());
    });
}

/// [`notice()`] for an error: the line in words, and when the network or a library said more,
/// "Details" under it with their words on hover (a click copies them, for a bug report).
pub fn error_notice(ui: &mut Ui, a: &mut Assets, text: &str, details: Option<&str>) {
    let p = a.p;
    notice(ui, a, "warning", p.danger, text);
    if let Some(details) = details {
        details_line(ui, &p, details);
    }
}

/// "Details": muted, the failure's own words on hover, copied on a click.
pub fn details_line(ui: &mut Ui, p: &Palette, details: &str) {
    let link = ui
        .add(
            egui::Label::new(txt(tr!("Details"), 12.0, Weight::Medium, p.muted))
                .sense(Sense::click()),
        )
        .on_hover_text(
            crate::i18n::t(&format!(
                tr!("{details}\n\nClick to copy"),
                details = details
            ))
            .into_owned(),
        );
    if link.clicked() {
        ui.ctx().copy_text(details.to_owned());
    }
}

/// A warning on the page background with a 1 px line: what can go wrong, not an explanation.
pub fn warning(ui: &mut Ui, a: &mut Assets, text: &str) {
    let p = a.p;
    Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.add(a.icons.icon("warning", p.muted, &p, 18.0));
                ui.add(
                    egui::Label::new(
                        txt(text, 13.0, Weight::Regular, p.text).line_height(Some(19.5)),
                    )
                    .wrap(),
                );
            });
        });
}

/// One muted line with an info icon: the screen shows an example, not the wallet (the demo).
#[cfg(demo_core)]
pub fn demo_line(ui: &mut Ui, a: &mut Assets, text: &str) {
    let p = a.p;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add(a.icons.icon("info", p.muted, &p, 16.0));
        ui.label(txt(text, 13.0, Weight::Regular, p.muted));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{DARK, LIGHT};

    /// WCAG relative luminance.
    fn luminance(c: Color32) -> f64 {
        let lin = |v: u8| {
            let s = f64::from(v) / 255.0;
            if s <= 0.039_28 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// CS-1.2: the «attention» icon holds 3:1 on its raised notice and on the page, in both
    /// themes, and it is not the error red.
    #[test]
    fn attention_is_seen_and_is_not_red() {
        for p in [LIGHT, DARK] {
            let c = attention_color(&p);
            assert_ne!(c, p.danger);
            for bg in [p.raised, p.bg] {
                assert!(
                    contrast(c, bg) >= 3.0,
                    "{:?} on {:?}: {}",
                    c,
                    bg,
                    contrast(c, bg)
                );
            }
        }
        // The mockup's brand yellow does not, in the light theme: why `accent` is used.
        assert!(contrast(LIGHT.brand, LIGHT.raised) < 3.0);
    }
}
