//! Appearance and language: theme cards, language, units.

use super::parts::{Control, group, heading};
use super::{CURRENCIES, SettingsState, ThemeChoice};
use crate::components::Assets;
use crate::theme::{Palette, Weight, sans};
use egui::{
    Align2, CornerRadius, CursorIcon, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use evolite_api::FiatRate;

pub(super) fn appearance(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut SettingsState,
    fiat: Option<&FiatRate>,
) {
    let p = a.p;
    heading(ui, &p, tr!("Appearance and language"), None);
    let width = ui.available_width();
    let w = (width - 2.0 * 16.0) / 3.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for (choice, label) in [
            (ThemeChoice::Dark, tr!("Dark")),
            (ThemeChoice::Light, tr!("Light")),
            (ThemeChoice::System, tr!("As in the system")),
        ] {
            if theme_card(ui, a, choice, label, state.theme == choice, w).clicked() {
                state.theme = choice;
            }
        }
    });
    let mut rate = *state.toggle("fiat-rate");
    let mut unit = state.unit;
    let mut style = state.number_style;
    let mut currency = state.fiat_currency;
    let unit_note = [
        "1 DASH = 100 000 000 duffs",
        "1 mDASH = 0,001 DASH",
        "1 duff = 0,00000001 DASH",
    ][unit];
    let (g, d) = crate::i18n::separators();
    let by_language = format!(
        "{}: 1{g}234{d}5678",
        crate::i18n::t(tr!("As in the interface language"))
    );
    let style_note = [
        by_language.as_str(),
        tr!("Space between thousands, comma before decimals"),
        tr!("Comma between thousands, point before decimals"),
    ][style];
    let rate_note = match fiat {
        Some(f) if f.currency == CURRENCIES[currency] => format!(
            tr!("{} · updated {}"),
            match evolite_core::network::current() {
                evolite_api::Network::Testnet => format!(
                    tr!("1 DASH ≈ {} {} · mainnet rate, testnet coins are worth nothing"),
                    crate::format::rate_number(f.price),
                    f.currency
                ),
                evolite_api::Network::Mainnet => format!(
                    "1 DASH ≈ {} {}",
                    crate::format::rate_number(f.price),
                    f.currency
                ),
            },
            crate::text::time_of_day(f.at, chrono::Local::now())
        ),
        _ => tr!("Asking rates.ctx.com for the rate…").into(),
    };
    let lang = crate::i18n::current().native();
    let mut rows = vec![
        (
            tr!("Interface language"),
            tr!("15 languages · Arabic right to left"),
            Control::Value(lang, false),
        ),
        (
            tr!("Number format"),
            style_note,
            Control::Segmented(
                &[tr!("As in the language"), "1 234,56", "1,234.56"],
                &mut style,
            ),
        ),
        (
            tr!("Amount unit"),
            unit_note,
            Control::Segmented(&["DASH", "mDASH", "duffs"], &mut unit),
        ),
        (
            tr!("Fiat rate"),
            tr!(
                "The rate comes from rates.ctx.com (as in DashPay): the service sees your IP. Every 30 minutes"
            ),
            Control::Toggle(&mut rate),
        ),
    ];
    if *state.toggle("fiat-rate") {
        rows.push((
            tr!("Currency"),
            &rate_note,
            Control::Segmented(&CURRENCIES, &mut currency),
        ));
    }
    if group(ui, a, tr!("Language and format"), rows) == Some(0) {
        state.lang_picker = true;
    }
    state.unit = unit;
    state.number_style = style;
    state.fiat_currency = currency;
    *state.toggle("fiat-rate") = rate;

    let system = if crate::motion::system_animations() {
        tr!("As in the system: animation is on now")
    } else {
        tr!("As in the system: animation is off now")
    };
    let mut motion = state.nft_motion;
    let mut ask = *state.toggle("nft-ask-unknown");
    group(
        ui,
        a,
        tr!("Collection"),
        vec![
            (
                tr!("NFT animation"),
                system,
                Control::Segmented(
                    &[tr!("As in the system"), tr!("On"), tr!("Off")],
                    &mut motion,
                ),
            ),
            (
                tr!("Ask before showing"),
                tr!("A picture from an unknown issuer is drawn only when you ask"),
                Control::Toggle(&mut ask),
            ),
        ],
    );
    state.nft_motion = motion;
    *state.toggle("nft-ask-unknown") = ask;
}

/// "Interface language": the fifteen languages by their own names, the current one marked; a
/// choice applies at once and is kept in `settings.json`.
pub(super) fn lang_dialog(ui: &mut Ui, a: &mut Assets, state: &mut SettingsState) {
    use crate::i18n;
    let p = a.p;
    let now = i18n::current();
    crate::theme::want_name_fonts();
    let closed = crate::components::dialog(
        ui,
        a,
        "lang-picker",
        "language",
        tr!("Interface language"),
        420.0,
        |ui, a| {
            let mut close = false;
            ui.spacing_mut().item_spacing.y = 2.0;
            for lang in picker_order() {
                let on = lang == now;
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                crate::components::named_choice(
                    &resp,
                    egui::WidgetType::RadioButton,
                    on,
                    lang.native(),
                );
                if on || resp.hovered() {
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(8), p.raised);
                }
                // A language's own name is never translated.
                ui.painter().text(
                    pos2(rect.left() + 12.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    lang.native(),
                    crate::theme::name_font(
                        lang,
                        14.0,
                        if on {
                            Weight::SemiBold
                        } else {
                            Weight::Regular
                        },
                    ),
                    p.text,
                );
                if on {
                    a.icons.icon("check", p.text, &p, 16.0).paint_at(
                        ui,
                        Rect::from_center_size(
                            pos2(rect.right() - 18.0, rect.center().y),
                            vec2(16.0, 16.0),
                        ),
                    );
                }
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    state.lang = Some(lang);
                    i18n::set(lang);
                    close = true;
                }
            }
            close
        },
    );
    if closed {
        state.lang_picker = false;
    }
}

/// A theme preview: a little window in that theme's colours, a tick on the chosen one.
fn theme_card(
    ui: &mut Ui,
    a: &mut Assets,
    choice: ThemeChoice,
    label: &str,
    on: bool,
    width: f32,
) -> Response {
    use crate::theme::{DARK, LIGHT};
    let p = a.p;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 136.0), Sense::click());
    crate::components::named_choice(&resp, egui::WidgetType::RadioButton, on, label);
    let stroke = if on {
        Stroke::new(2.0, p.ink)
    } else if resp.hovered() {
        Stroke::new(1.0, p.muted)
    } else {
        Stroke::new(1.0, p.line)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(14),
        p.surface,
        stroke,
        StrokeKind::Inside,
    );
    let preview = Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(width - 20.0, 84.0));
    let painter = ui.painter();
    let mini = |painter: &egui::Painter, area: Rect, t: &Palette, clip: Rect| {
        let painter = painter.with_clip_rect(clip);
        painter.rect_filled(area, CornerRadius::same(8), t.bg);
        let side = Rect::from_min_size(area.min + vec2(8.0, 8.0), vec2(36.0, area.height() - 16.0));
        painter.rect_filled(side, CornerRadius::same(4), t.surface);
        let x = side.right() + 10.0;
        painter.rect_filled(
            Rect::from_min_size(pos2(x, area.top() + 8.0), vec2(60.0, 7.0)),
            2,
            t.text,
        );
        painter.rect_filled(
            Rect::from_min_size(pos2(x, area.top() + 22.0), vec2(90.0, 6.0)),
            2,
            t.raised,
        );
        painter.rect_filled(
            Rect::from_min_size(pos2(x, area.top() + 34.0), vec2(70.0, 6.0)),
            2,
            t.raised,
        );
        painter.rect_filled(
            Rect::from_min_size(pos2(x, area.top() + 48.0), vec2(10.0, 10.0)),
            2,
            t.accent,
        );
    };
    match choice {
        ThemeChoice::Dark => mini(painter, preview, &DARK, preview),
        ThemeChoice::Light => mini(painter, preview, &LIGHT, preview),
        ThemeChoice::System => {
            let (left, right) = preview.split_left_right_at_fraction(0.5);
            painter.rect_filled(preview, CornerRadius::same(8), LIGHT.bg);
            painter
                .with_clip_rect(left)
                .rect_filled(preview, CornerRadius::same(8), DARK.bg);
            let _ = right;
        }
    }
    let y = preview.bottom() + 21.0;
    let icon = match (on, choice) {
        (true, _) => "check",
        (false, ThemeChoice::Dark) => "theme-dark",
        (false, _) => "theme-light",
    };
    a.icons
        .icon(icon, if on { p.text } else { p.muted }, &p, 16.0)
        .paint_at(
            ui,
            Rect::from_min_size(pos2(rect.left() + 14.0, y - 8.0), vec2(16.0, 16.0)),
        );
    ui.painter().text(
        pos2(rect.left() + 38.0, y),
        Align2::LEFT_CENTER,
        crate::i18n::t(label),
        sans(13.0, if on { Weight::SemiBold } else { Weight::Medium }),
        if on { p.text } else { p.muted },
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// The foot of the welcome and unlock screens (Figma 28, 01): the network, then the language
/// and the theme, each a link — before the password is typed (BUG-53). The language opens the
/// same "Interface language" as Settings; the theme switches between dark and light. Both are
/// the app's, not a wallet's: they are written to `settings.json` with no wallet open.
pub(crate) fn language_and_theme(
    ui: &mut Ui,
    a: &mut Assets,
    network: evolite_api::Network,
    state: &mut SettingsState,
) {
    let p = a.p;
    let lang = crate::i18n::own(crate::i18n::current().native());
    let theme = if p.dark {
        tr!("dark theme")
    } else {
        tr!("light theme")
    };
    let font = sans(13.0, Weight::Regular);
    let w = crate::components::text_width(ui, &lang, font.clone())
        + crate::components::text_width(ui, " · ", font.clone())
        + crate::components::text_width(ui, theme, font)
        + 12.0
        + 90.0;
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        ui.spacing_mut().item_spacing.x = 0.0;
        crate::components::network_chip(ui, a, network);
        ui.add_space(12.0);
        if foot_link(ui, &p, &lang).clicked() {
            state.lang_picker = true;
        }
        ui.label(crate::screens::txt(" · ", 13.0, Weight::Regular, p.muted));
        if foot_link(ui, &p, theme).clicked() {
            state.theme = if p.dark {
                ThemeChoice::Light
            } else {
                ThemeChoice::Dark
            };
        }
    });
    if state.lang_picker {
        lang_dialog(ui, a, state);
    }
}

/// A muted word of the foot that can be clicked: underlined under the pointer, a link for
/// screen readers.
fn foot_link(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let resp = ui
        .add(
            egui::Label::new(crate::screens::txt(text, 13.0, Weight::Regular, p.muted))
                .sense(Sense::click()),
        )
        .on_hover_cursor(CursorIcon::PointingHand);
    if resp.hovered() {
        let r = resp.rect;
        ui.painter()
            .hline(r.x_range(), r.bottom() - 1.0, Stroke::new(1.0, p.muted));
    }
    crate::components::named(&resp, egui::WidgetType::Link, true, text);
    resp
}

/// The languages in the picker: alphabetically by their own names (designer review №21), so
/// the list has one order whatever the interface language.
fn picker_order() -> Vec<crate::i18n::Lang> {
    let mut langs = crate::i18n::Lang::ALL.to_vec();
    langs.sort_by_key(|l| l.native().to_lowercase());
    langs
}

#[cfg(test)]
mod foot_tests {
    use super::*;

    #[test]
    fn the_picker_lists_every_language_once_by_its_own_name() {
        let order = picker_order();
        assert_eq!(order.len(), crate::i18n::Lang::ALL.len());
        let names: Vec<String> = order.iter().map(|l| l.native().to_lowercase()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        for lang in crate::i18n::Lang::ALL {
            assert_eq!(order.iter().filter(|l| **l == lang).count(), 1);
        }
    }
}
