//! O4 · Restore from phrase (typed or pasted) and O5 · what the DashPay import found.

use super::txt;
use super::{Flow, Onboarding, Step, actions, heading};
use crate::components::{Assets, card, inner, notice, segmented, text_action};
use crate::context::Ctx;
use crate::theme::{Weight, mono, sans};
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Key, Layout, Margin, Rect, Sense, Stroke, Ui,
    pos2, vec2,
};
use evolite_api::{Command, Phrase};
use evolite_core::bip39;
use zeroize::Zeroize;

/// O4 · Restore from phrase (also the first step of the DashPay import).
pub(super) fn restore(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, o: &mut Onboarding) {
    let p = a.p;
    let (title, lead) = if o.flow == Flow::DashPay {
        (
            tr!("Import from DashPay"),
            tr!(
                "Enter the phrase from DashPay (12 words). evolite finds the addresses, identity, name and contacts on DashPay's paths."
            ),
        )
    } else {
        (
            tr!("Restore from phrase"),
            tr!(
                "Enter the 12 or 24 words in order. You can paste the whole phrase — evolite spreads it over the fields."
            ),
        )
    };
    heading(ui, &p, title, lead);
    // As when creating: how many fields there are is chosen, not guessed from a paste.
    // DashPay's phrase is always 12 words.
    if o.flow != Flow::DashPay {
        phrase_length(ui, o, &p);
    }

    take_pasted(o);
    // The fields emptied: no line left from an earlier paste (SEC-2 review r1, could).
    if o.restore_words.iter().all(|w| w.is_empty()) {
        o.paste_wiped = false;
    } else if crate::secret_words::window_left(ui.ctx()) {
        // Э4 (SEC-7b): the window left, the typed words hide; «Show the words» brings them back.
        o.restore_hidden = true;
    }

    let width = ui.available_width();
    let w = (width - 16.0) / 3.0;
    // BUG-228: what is typed reaches the tree only with reading on, for this showing.
    let read = o.restore_reading.showing(ui.ctx());
    let reader = crate::secret_words::screen_reader_running();
    if reader {
        crate::secret_words::toggle(ui, a, &mut o.restore_reading);
    }
    word_grid(ui, o, &p, w);

    // Suggestions for the field in focus.
    suggestions(ui, o, &p, read);
    if !reader {
        crate::secret_words::toggle(ui, a, &mut o.restore_reading);
    }

    paste_row(ui, a, o);
    if o.paste_error {
        ui.label(txt(
            tr!("The clipboard holds no phrase of 12 or 24 words"),
            12.0,
            Weight::Regular,
            p.danger,
        ));
    }
    // SEC-2 B4: the phrase pasted is not left on the clipboard.
    if o.paste_wiped {
        ui.label(txt(
            tr!("The phrase was taken off the clipboard."),
            12.0,
            Weight::Regular,
            p.muted,
        ));
    }

    let words: Vec<String> = o
        .restore_words
        .iter()
        .map(|w| w.trim().to_lowercase())
        .collect();
    let filled = words.iter().all(|w| !w.is_empty());
    let refs: Vec<&str> = words.iter().map(String::as_str).collect();
    let result = bip39::check(&refs);
    phrase_problem(ui, &p, &result, &words, filled);
    notice(
        ui,
        a,
        "info",
        p.muted,
        tr!(
            "Words are checked against the BIP39 word list, the checksum after the last word. evolite shows the addresses, identities and names it found on the next step."
        ),
    );
    if let Some(error) = &o.error {
        ui.label(txt(error, 12.0, Weight::Regular, p.danger));
    }
    // FEAT-0041 R-1: always there, the switch on or off (analyst-158); not in the import from
    // DashPay, whose Android app has none.
    if o.flow != Flow::DashPay {
        super::bip39::block(ui, a, o, false);
    }
    let ready = result.is_ok() && !o.waiting && super::bip39::ready(o, false);
    // An import from DashPay reads Platform first: up to a few minutes on a first sync.
    let label = if o.waiting && o.flow == Flow::DashPay {
        tr!("Looking for your DashPay wallet…")
    } else if o.waiting {
        tr!("Checking…")
    } else {
        tr!("Continue")
    };
    let pressed = actions(ui, a, None, label, ready).1 || (o.submit_on_open && ready);
    if pressed {
        // The phrase goes to the core and is wiped here at once.
        let passphrase = super::bip39::typed(o).filter(|_| o.flow != Flow::DashPay);
        cx.send(Command::UsePhrase {
            phrase: Phrase::with_passphrase(words, passphrase.unwrap_or_default()),
            dashpay: o.flow == Flow::DashPay,
        });
        for w in o.restore_words.iter_mut() {
            w.zeroize();
        }
        o.waiting = true;
        o.error = None;
        o.submit_on_open = false;
    }
}

/// «Phrase length»: how many fields there are is chosen, not guessed from a paste.
fn phrase_length(ui: &mut Ui, o: &mut Onboarding, p: &crate::theme::Palette) {
    let p = *p;
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 36.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(txt(tr!("Phrase length"), 13.0, Weight::Medium, p.muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut long = usize::from(o.restore_words.len() == 24);
                segmented(
                    ui,
                    &p,
                    &[tr!("12 words"), tr!("24 words")],
                    &mut long,
                    false,
                );
                set_length(o, if long == 1 { 24 } else { 12 });
            });
        },
    );
}

/// The word fields, three in a row; the last one focused is remembered for the suggestions.
fn word_grid(ui: &mut Ui, o: &mut Onboarding, p: &crate::theme::Palette, w: f32) {
    let mut focused = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let count = o.restore_words.len();
        for row in 0..count / 3 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for c in 0..3 {
                    word_cell(ui, o, p, row * 3 + c, count, w, &mut focused);
                }
            });
        }
    });
    if focused.is_some() {
        o.focused = focused;
    }
}

/// One word's field: its number, its text, and what Enter and Tab do there.
fn word_cell(
    ui: &mut Ui,
    o: &mut Onboarding,
    p: &crate::theme::Palette,
    i: usize,
    count: usize,
    w: f32,
    focused: &mut Option<usize>,
) {
    let p = *p;
    let word = o.restore_words[i].trim().to_lowercase();
    let id = ui.make_persistent_id(("restore-word", i));
    let has_focus = ui.memory(|m| m.has_focus(id));
    // While typing (in the field or the last one typed in), a start of a word
    // is fine; once another field is taken, only a whole word of the list is.
    let typing = has_focus || o.focused == Some(i);
    let unknown = !word.is_empty()
        && if typing {
            bip39::suggest(&word, 1).is_empty()
        } else {
            bip39::index(&word).is_none()
        };
    if has_focus {
        *focused = Some(i);
    }
    let stroke = if unknown {
        p.danger
    } else if has_focus {
        p.ink
    } else {
        p.field_line
    };
    let fill = if word.is_empty() && !has_focus {
        p.bg
    } else {
        p.raised
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(w - 26.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(
                    egui::RichText::new(crate::i18n::t(&format!("{:02}", i + 1)))
                        .font(mono(12.0, Weight::Regular))
                        .color(p.muted),
                );
                crate::a11y::name_next_field(ui, &format!(tr!("Word no. {}"), i + 1));
                // SEC-2 B3: the words are not copied or cut out of the fields.
                if has_focus {
                    let pasted = ui.input_mut(|input| {
                        input
                            .events
                            .retain(|e| !matches!(e, egui::Event::Copy | egui::Event::Cut));
                        pasted_phrase(&input.events)
                    });
                    // SEC-2 B4: a whole phrase pasted by Ctrl+V leaves the clipboard: the text
                    // the paste brought, as the clipboard holds it, not the field after it
                    // (review r1, S1).
                    if let Some(text) = pasted {
                        o.paste_wiped = crate::clipboard::wipe_pasted(&text);
                    }
                }
                crate::rtl::text_edit(
                    ui,
                    egui::TextEdit::singleline(&mut o.restore_words[i])
                        .id(id)
                        .password(o.restore_hidden)
                        .font(mono(14.0, Weight::Medium))
                        .text_color(p.text)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                crate::components::forget_typing(ui.ctx(), id);
                if !o.restore_reading.is_on() {
                    crate::secret_words::mute(ui.ctx(), id);
                }
                // SEC-3: the words reach no IME and no text suggestions.
                if ui.memory(|m| m.has_focus(id)) {
                    crate::secret_input::focused(ui.ctx());
                }
            });
            // Enter or Tab takes the first suggestion; Enter also goes on to
            // the next field (Tab does so by itself).
            let (enter, tab) =
                ui.input(|inp| (inp.key_pressed(Key::Enter), inp.key_pressed(Key::Tab)));
            if has_focus && (enter || tab) {
                if let Some(first) = completion(&o.restore_words[i]) {
                    o.restore_words[i] = first.to_string();
                }
                if enter && i + 1 < count {
                    ui.memory_mut(|m| {
                        m.request_focus(ui.make_persistent_id(("restore-word", i + 1)))
                    });
                }
            }
        });
}

/// Suggestions for the field in focus.
fn suggestions(ui: &mut Ui, o: &mut Onboarding, p: &crate::theme::Palette, read: bool) {
    let p = *p;
    if let Some(i) = o.focused.filter(|i| *i < o.restore_words.len()) {
        let prefix = o.restore_words[i].trim().to_lowercase();
        let hints = bip39::suggest(&prefix, 4);
        if !hints.is_empty() && !hints.contains(&prefix.as_str()) {
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 28.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(txt(
                        &format!(
                            tr!("Suggestions for no. {} (Enter takes the first):"),
                            i + 1
                        ),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    for (n, hint) in hints.into_iter().enumerate() {
                        let font = mono(13.0, Weight::Medium);
                        let tw = ui
                            .painter()
                            .layout_no_wrap(hint.to_string(), font.clone(), Color32::WHITE)
                            .size()
                            .x;
                        let (rect, resp) =
                            ui.allocate_exact_size(vec2(tw + 20.0, 28.0), Sense::click());
                        crate::components::named(
                            &resp,
                            egui::WidgetType::Button,
                            true,
                            &if read {
                                crate::i18n::phrase_word(hint)
                            } else {
                                format!(tr!("Suggestion {}"), n + 1)
                            },
                        );
                        let fill = if resp.hovered() { p.line } else { p.raised };
                        ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            crate::i18n::t(&crate::i18n::phrase_word(hint)),
                            font,
                            p.text,
                        );
                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            o.restore_words[i] = hint.to_string();
                            if i + 1 < o.restore_words.len() {
                                ui.memory_mut(|m| {
                                    m.request_focus(ui.make_persistent_id(("restore-word", i + 1)))
                                });
                            }
                        }
                    }
                },
            );
        }
    }
}

/// «Paste from clipboard» and «Show / Hide the words».
fn paste_row(ui: &mut Ui, a: &mut Assets, o: &mut Onboarding) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        if text_action(ui, a, "paste", tr!("Paste from clipboard")).clicked() {
            let pasted = zeroize::Zeroizing::new(
                arboard::Clipboard::new()
                    .and_then(|mut c| c.get_text())
                    .unwrap_or_default(),
            );
            let parts: Vec<String> = pasted.split_whitespace().map(str::to_lowercase).collect();
            o.paste_error = !(parts.len() == 12 || parts.len() == 24);
            // Said only of this paste (review r1, could).
            o.paste_wiped = false;
            if !o.paste_error {
                spread(o, 0, &parts);
                // A whole phrase on screen at once is easy to see over a shoulder.
                o.restore_hidden = true;
                // SEC-2 B4: taken, it leaves the clipboard.
                o.paste_wiped = crate::clipboard::wipe_pasted(&pasted);
            }
        }
        let (icon, label) = if o.restore_hidden {
            ("eye", tr!("Show the words"))
        } else {
            ("eye-off", tr!("Hide the words"))
        };
        if text_action(ui, a, icon, label).clicked() {
            o.restore_hidden = !o.restore_hidden;
        }
    });
}

/// What is wrong with the phrase, once that can be said.
fn phrase_problem(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    result: &Result<(), bip39::PhraseError>,
    words: &[String],
    filled: bool,
) {
    let p = *p;
    match (&result, filled) {
        // A word that cannot start any word of the list, or, once every field is filled, any
        // word that is not one (an unfinished "aban" too).
        (Err(bip39::PhraseError::UnknownWord(i)), filled)
            if !words[*i].is_empty() && (filled || bip39::suggest(&words[*i], 1).is_empty()) =>
        {
            ui.label(txt(
                &format!(tr!("Word no. {} is not in the BIP39 word list"), i + 1),
                12.0,
                Weight::Regular,
                p.danger,
            ));
        }
        (Err(bip39::PhraseError::Checksum), true) => {
            ui.add(crate::components::para(
                tr!("The checksum does not match: check the order and spelling of the words"),
                12.0,
                Weight::Regular,
                p.danger,
            ));
        }
        _ => {}
    }
}

/// The first word of the list that `typed` starts, unless it is a whole word already.
fn completion(typed: &str) -> Option<&'static str> {
    let prefix = typed.trim().to_lowercase();
    if prefix.is_empty() || bip39::index(&prefix).is_some() {
        return None;
    }
    bip39::suggest(&prefix, 1).first().copied()
}

/// A whole phrase typed or pasted into one field (Ctrl+V) is spread over the fields; a whole
/// phrase at once is hidden, as "Paste from clipboard" hides it: easy to see over a
/// shoulder.
fn take_pasted(o: &mut Onboarding) {
    for i in 0..o.restore_words.len() {
        let parts: Vec<String> = o.restore_words[i]
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        if parts.len() > 1 {
            spread(o, i, &parts);
            if parts.len() >= 12 {
                o.restore_hidden = true;
            }
            break;
        }
    }
}

/// SEC-2 B4 (review r1, S1): what a paste brings into the focused field, exactly as the
/// clipboard holds it, when it is a whole phrase (12 parts or more): that is what is wiped from
/// the clipboard, whatever the field held before or makes of line breaks.
fn pasted_phrase(events: &[egui::Event]) -> Option<zeroize::Zeroizing<String>> {
    events.iter().find_map(|e| match e {
        egui::Event::Paste(text) if text.split_whitespace().count() >= 12 => {
            Some(zeroize::Zeroizing::new(text.clone()))
        }
        _ => None,
    })
}

/// 12 or 24 fields; the words of the fields taken away are wiped first.
fn set_length(o: &mut Onboarding, n: usize) {
    if o.restore_words.len() == n {
        return;
    }
    for word in o.restore_words.iter_mut().skip(n) {
        word.zeroize();
    }
    o.restore_words.resize(n, String::new());
}

/// Put `parts` into the fields from `start`, growing to 24 fields when needed.
fn spread(o: &mut Onboarding, start: usize, parts: &[String]) {
    let total = start + parts.len();
    let size = if total > 12 { 24 } else { 12 };
    o.restore_words.resize(size, String::new());
    for (k, part) in parts.iter().enumerate().take(size - start) {
        o.restore_words[start + k] = part.clone();
    }
}

/// O5 · We found your DashPay wallet.
pub(super) fn found(ui: &mut Ui, a: &mut Assets, o: &mut Onboarding) {
    let p = a.p;
    heading(
        ui,
        &p,
        tr!("We found your DashPay wallet"),
        tr!("The phrase fits. evolite scanned the network on DashPay's paths and found:"),
    );
    let width = ui.available_width();
    let Some(scan) = o.scan.clone() else {
        o.step = Step::Restore;
        return;
    };
    let dash = |d: i64| crate::format::balance(d);
    // Core balances come with the wallet's own sync: the real core says so instead of a 0.
    let (core_line, core_value) = if scan.core_known {
        (
            format!(
                tr!("{} addresses with history · BIP44"),
                scan.core_addresses_used
            ),
            Some(dash(scan.core)),
        )
    } else {
        (
            tr!("BIP44 · balances appear after the sync").to_string(),
            None,
        )
    };
    let mut rows: Vec<(&str, String, String, Option<String>, bool)> = vec![(
        "core",
        tr!("Core wallet").into(),
        core_line,
        core_value,
        false,
    )];
    // FEAT-0030 R1: coins after CoinJoin, apart from Core and spendable from «Mixed».
    if let Some((addresses, duffs)) = scan.coinjoin {
        rows.push((
            "mixed",
            tr!("Mixed coins").into(),
            format!(
                "{} · {}",
                crate::text::plural(addresses, tr!("address"), tr!("addresses")),
                tr!("CoinJoin account")
            ),
            Some(dash(duffs)),
            false,
        ));
    }
    if let Some(id) = &scan.identity_id {
        rows.push((
            "identity",
            // BUG-270: translated, as every row's title («Identity 8rd3…» stayed in Latin).
            format!(tr!("Identity {}"), crate::text::whole(id)),
            format!(tr!("DIP-13 keys · {} keys"), scan.identity_keys),
            Some(dash(scan.identity_credits)),
            false,
        ));
    }
    if let Some(name) = &scan.name {
        rows.push((
            "username",
            format!(tr!("Name @{name}"), name = name),
            tr!("DPNS · confirmed").into(),
            None,
            true,
        ));
    }
    if scan.contacts > 0 {
        rows.push((
            "contacts",
            tr!("DashPay contacts").into(),
            format!(tr!("{} contacts · DIP-15"), scan.contacts),
            None,
            false,
        ));
    }
    // A person knows the wallet by its balances: hidden by the setting, the eye shows them
    // (FEAT-0007, the designer's decision for the found wallet).
    // In a row of its own: a right-to-left layout straight in the page takes all the height
    // left, and the found wallet went below the window's edge (BUG-133).
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            crate::components::amount_toggle(ui, a, false);
        });
    });
    card(&p, Margin::ZERO).show(ui, |ui| {
        ui.set_width(inner(width, 0));
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, (icon, title, sub, value, verified)) in rows.iter().enumerate() {
            if i > 0 {
                let (line, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(line, 0, p.line);
            }
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
            let cy = rect.center().y;
            let boxr = Rect::from_min_size(pos2(rect.left() + 18.0, cy - 18.0), vec2(36.0, 36.0));
            crate::components::paint_icon_box(ui, a, boxr, 10, icon, 18.0, p.text);
            let x = boxr.right() + 14.0;
            // BUG-348: the identity's id ends before the amount — shortened in its middle to
            // what is left of the row («8rd3…J38R» at the least), as in the identity lists.
            let title = match (*icon, &scan.identity_id) {
                ("identity", Some(id)) => {
                    // r1 nit: the width of the very text painted at the right — `t(v)`, laid out
                    // as it is drawn below.
                    let value_w = value.as_ref().map_or(0.0, |v| {
                        ui.painter()
                            .layout_no_wrap(
                                crate::i18n::t(v).into_owned(),
                                mono(14.0, Weight::Medium),
                                p.text,
                            )
                            .size()
                            .x
                    });
                    let room = rect.right() - 20.0 - value_w - 16.0 - x;
                    let short = crate::components::fit_id(id, &|s: &str| {
                        crate::components::text_width(
                            ui,
                            &format!(tr!("Identity {}"), s),
                            sans(14.0, Weight::SemiBold),
                        ) <= room
                    });
                    format!(tr!("Identity {}"), crate::text::whole(&short))
                }
                _ => title.clone(),
            };
            crate::components::paint_text(
                ui,
                pos2(x, cy - 1.0),
                Align2::LEFT_BOTTOM,
                &title,
                sans(14.0, Weight::SemiBold),
                p.text,
            );
            crate::components::paint_text(
                ui,
                pos2(x, cy + 2.0),
                Align2::LEFT_TOP,
                sub,
                sans(12.0, Weight::Regular),
                p.muted,
            );
            if let Some(v) = value {
                ui.painter().text(
                    pos2(rect.right() - 20.0, cy),
                    Align2::RIGHT_CENTER,
                    crate::i18n::t(v),
                    mono(14.0, Weight::Medium),
                    p.text,
                );
            }
            if *verified {
                a.icons.icon("confirmed", p.accent, &p, 18.0).paint_at(
                    ui,
                    Rect::from_center_size(pos2(rect.right() - 29.0, cy), vec2(18.0, 18.0)),
                );
            }
        }
    });
    // Nothing to agree to any more: the coins are not lost, only apart (FEAT-0030 R1).
    if scan.coinjoin.is_some() {
        notice(
            ui,
            a,
            "info",
            p.muted,
            tr!(
                "Mixed coins will be shown apart, on the “Mixed” tile, and you can spend them. evolite does not mix coins yet."
            ),
        );
    }
    if actions(ui, a, None, tr!("Set the password"), true).1 {
        o.next();
    }
}

#[cfg(test)]
mod tests {
    use super::{Flow, Onboarding, completion, pasted_phrase, set_length, take_pasted};

    #[test]
    fn the_length_switch_adds_and_takes_away_fields() {
        let mut o = Onboarding::new(Flow::Restore);
        assert_eq!(o.restore_words.len(), 12);
        set_length(&mut o, 24);
        o.restore_words[0] = "orbit".into();
        o.restore_words[20] = "canvas".into();
        assert_eq!(o.restore_words.len(), 24);
        // Back to 12: the first words stay, the fields past 12 go.
        set_length(&mut o, 12);
        assert_eq!(o.restore_words.len(), 12);
        assert_eq!(o.restore_words[0], "orbit");
    }

    #[test]
    fn enter_completes_a_start_but_not_a_whole_word() {
        assert_eq!(completion("fro"), Some("frog"));
        assert_eq!(completion(" ABAN "), Some("abandon"));
        assert_eq!(completion("frog"), None);
        assert_eq!(completion("qqq"), None);
        assert_eq!(completion(""), None);
    }

    #[test]
    fn a_phrase_pasted_into_a_field_is_spread_and_hidden() {
        let mut o = Onboarding::new(Flow::Restore);
        // Two words typed with a space: spread, still shown.
        o.restore_words[0] = "orbit canvas".into();
        take_pasted(&mut o);
        assert_eq!(
            (o.restore_words[1].as_str(), o.restore_hidden),
            ("canvas", false)
        );
        // A whole phrase by Ctrl+V into one field: spread over the fields and hidden.
        o.restore_words[0] = ["abandon"; 11].join(" ") + " about";
        take_pasted(&mut o);
        assert_eq!(o.restore_words[11], "about");
        assert!(o.restore_hidden);
    }

    /// SEC-2 review r1, S1: Ctrl+V of a phrase ending in a line break into a field that held
    /// «a»: the text wiped is the paste's own, not the field's; fewer than 12 parts and other
    /// events are not a phrase.
    #[test]
    fn the_text_a_paste_brought_is_the_one_wiped() {
        let phrase = ["abandon"; 11].join(" ") + " about\n";
        let events = [
            egui::Event::Text("a".into()),
            egui::Event::Paste(phrase.clone()),
        ];
        assert_eq!(
            pasted_phrase(&events).as_deref().map(String::as_str),
            Some(phrase.as_str())
        );
        let short = [egui::Event::Paste(["abandon"; 11].join(" "))];
        assert_eq!(pasted_phrase(&short), None);
        assert_eq!(pasted_phrase(&[egui::Event::Copy]), None);
    }
}
