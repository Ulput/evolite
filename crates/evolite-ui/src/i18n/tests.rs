use super::*;

fn catalog(lang: Lang, json: &str) -> Catalog {
    Catalog::parse(lang, json).expect("a catalog")
}

#[test]
fn exact_strings_and_templates() {
    let mut de = catalog(
        Lang::De,
        r#"{
            "Send": "Senden",
            "Hides in {} s": "Verschwindet in {} s",
            "Not sent: {}": "Nicht gesendet: {}",
            "the network did not answer": "das Netzwerk hat nicht geantwortet",
            "{} → {}": "{1} ← {0}"
        }"#,
    );
    assert_eq!(de.translate("Send").as_deref(), Some("Senden"));
    assert_eq!(
        de.translate("Hides in 12 s").as_deref(),
        Some("Verschwindet in 12 s")
    );
    assert_eq!(
        de.translate("Not sent: the network did not answer")
            .as_deref(),
        Some("Nicht gesendet: das Netzwerk hat nicht geantwortet"),
        "words inside a template are translated too"
    );
    assert_eq!(
        de.translate("a → b").as_deref(),
        Some("b ← a"),
        "places by number"
    );
    assert_eq!(de.translate("No such string"), None);
}

#[test]
fn a_persons_own_words_are_never_translated() {
    let mut de = catalog(
        Lang::De,
        r#"{
            "Main": "Hauptkonto",
            "Open “{}”": "„{}“ öffnen"
        }"#,
    );
    // A wallet named "Main" by its owner keeps that name inside a translated sentence.
    let name = own("Main");
    assert_eq!(
        de.translate(&format!("Open “{name}”")).as_deref(),
        Some(format!("„{name}“ öffnen").as_str())
    );
    assert_eq!(
        de.translate("Open “Main”").as_deref(),
        Some("„Hauptkonto“ öffnen")
    );
    assert_eq!(
        unmarked(Cow::Owned(own("Main"))),
        "Main",
        "the marks are not drawn"
    );
}

#[test]
fn text_from_outside_stays_inside_a_template() {
    let mut de = catalog(Lang::De, r#"{ "Not sent: {}": "Nicht gesendet: {}" }"#);
    assert_eq!(
        de.translate("Not sent: connection refused by the remote peer")
            .as_deref(),
        Some("Nicht gesendet: connection refused by the remote peer"),
        "a library's error has no translation and stays as it is"
    );
}

#[test]
fn plural_forms_by_the_number_found() {
    let json = r#"{ "{} payment|{} payments": { "one": "{} Zahlung", "other": "{} Zahlungen" } }"#;
    let mut de = catalog(Lang::De, json);
    assert_eq!(de.translate("1 payment").as_deref(), Some("1 Zahlung"));
    assert_eq!(
        de.translate("21 payments").as_deref(),
        Some("21 Zahlungen"),
        "by German rules"
    );
    assert_eq!(
        de.translate("1.234 payments").as_deref(),
        Some("1.234 Zahlungen"),
        "the count written with the language's thousands separator"
    );

    let uk = r#"{ "{} payment|{} payments": { "one": "o{}", "few": "f{}", "many": "m{}" } }"#;
    let mut uk = catalog(Lang::Uk, uk);
    for (en, want) in [
        ("1 payment", "o1"),
        ("2 payments", "f2"),
        ("5 payments", "m5"),
        ("21 payments", "o21"),
    ] {
        assert_eq!(uk.translate(en).as_deref(), Some(want), "{en}");
    }

    let ar = r#"{ "{} payment|{} payments": { "zero": "z{}", "one": "o{}", "two": "t{}", "few": "f{}", "many": "m{}", "other": "x{}" } }"#;
    let mut ar = catalog(Lang::Ar, ar);
    for (en, want) in [
        ("0 payments", "z0"),
        ("1 payment", "o1"),
        ("2 payments", "t2"),
        ("5 payments", "f5"),
        ("11 payments", "m11"),
        ("100 payments", "x100"),
    ] {
        assert_eq!(ar.translate(en).as_deref(), Some(want), "{en}");
    }
}

#[test]
fn cldr_categories() {
    use plural::Category::*;
    let ru: Vec<_> = [1, 2, 5, 11, 21, 22, 25]
        .map(|n| plural::category(Lang::Ru, n))
        .into();
    assert_eq!(ru, [One, Few, Many, Many, One, Few, Many]);
    assert_eq!(plural::category(Lang::Fr, 0), One);
    assert_eq!(plural::category(Lang::En, 0), Other);
    assert_eq!(plural::category(Lang::Ja, 1), Other);
}

#[test]
fn the_system_locale_picks_the_language() {
    assert_eq!(for_locale("ru-RU"), Lang::Ru);
    assert_eq!(for_locale("uk_UA.UTF-8"), Lang::Uk);
    assert_eq!(for_locale("zh-CN"), Lang::ZhHans);
    assert_eq!(
        for_locale("zh-Hant-TW"),
        Lang::En,
        "traditional Chinese is not one of ours"
    );
    assert_eq!(for_locale("pt-PT"), Lang::PtBr);
    assert_eq!(for_locale("en-GB"), Lang::En);
    assert_eq!(
        for_locale("bn-BD"),
        Lang::En,
        "Bengali was replaced by uk and vi"
    );
    assert_eq!(for_locale(""), Lang::En);
}

#[test]
fn every_language_has_a_name_and_a_catalog() {
    for lang in Lang::ALL {
        assert_eq!(Lang::from_code(lang.code()), Some(lang));
        assert_ne!(
            lang.native(),
            lang.code(),
            "{} has its own name",
            lang.code()
        );
        if let Some(json) = lang.catalog_json() {
            Catalog::parse(lang, json).unwrap_or_else(|e| panic!("{}: {e}", lang.code()));
        }
    }
}

#[test]
fn english_and_text_without_words_stay_as_they_are() {
    assert_eq!(t_in(Lang::En, "Send"), "Send");
    assert_eq!(t_in(Lang::De, "12,8460"), "12,8460");
    assert_eq!(
        t_in(Lang::De, "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f2c"),
        "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f2c"
    );
}

#[test]
fn an_arabic_word_goes_to_egui_as_it_is_written() {
    // BUG-117: egui shapes and joins a word itself (HarfBuzz) and turns it right to left; one
    // Arabic word goes to it in logical order, with its own letters, not joined forms.
    assert_eq!(visual("سلام"), "سلام");
    assert!(
        !visual("نسيت كلمة المرور؟")
            .chars()
            .any(|c| ('\u{FB50}'..='\u{FEFF}').contains(&c)),
        "no presentation forms"
    );
    // Two words: their order is bidi's, right to left; each word's letters as written.
    assert_eq!(visual("محفظة جديدة"), "جديدة محفظة");
    // Numbers inside Arabic keep their own order.
    assert!(visual("المبلغ 1234 DASH").contains("1234"));
    assert_eq!(visual("Send 1 DASH"), "Send 1 DASH", "no Arabic, untouched");
}

#[test]
fn brackets_in_arabic_face_the_right_way_and_lines_keep_their_place() {
    // "مشفرة (Argon2)": drawn left to right, the brackets still open before "Argon2".
    let shown = visual("مشفرة (Argon2)");
    assert!(shown.contains("(Argon2)"), "{shown:?}");
    let lines = visual("سلام\nمرحبا");
    assert_eq!(lines.matches('\n').count(), 1);
    assert!(
        !lines.starts_with('\n') && !lines.ends_with('\n'),
        "{lines:?}"
    );
    assert_eq!(lines.split('\n').next(), Some(&*visual("سلام")));
}

/// Every key the interface has (locales/_keys.json, made by locales/extract.pl) is in every
/// catalog, with the same number of places, and a plural key has each form its language needs.
#[test]
fn every_catalog_covers_every_key() {
    let keys: Vec<String> =
        serde_json::from_str(include_str!("../../locales/_keys.json")).expect("the key list");
    let mut gaps = Vec::new();
    for lang in Lang::ALL {
        let Some(json) = lang.catalog_json() else {
            continue;
        };
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(json).expect("a catalog");
        let places = |s: &str| s.matches('{').count();
        let mut missing = 0;
        for key in &keys {
            let want = places(key.split('|').next().unwrap_or(key));
            match map.get(key) {
                None => missing += 1,
                Some(serde_json::Value::String(s)) if places(s) != want => {
                    gaps.push(format!("{}: places in {key:?}", lang.code()))
                }
                Some(serde_json::Value::Object(forms)) => {
                    for n in [0, 1, 2, 3, 5, 11, 21, 100] {
                        let name = plural::category(lang, n).name();
                        if !forms.contains_key(name) && !forms.contains_key("other") {
                            gaps.push(format!("{}: no {name} for {key:?}", lang.code()));
                        }
                    }
                }
                _ => {}
            }
        }
        if missing > 0 {
            gaps.push(format!("{}: {missing} keys missing", lang.code()));
        }
    }
    assert!(gaps.is_empty(), "{}", gaps.join("\n"));
}

#[test]
fn pieces_and_plurals_that_count_nothing() {
    let json = r#"{
        "just now": "gerade eben",
        "{} address|{} addresses": { "one": "{} Adresse", "other": "{} Adressen" },
        "the payment goes to a new address": "die Zahlung geht an eine neue Adresse"
    }"#;
    let mut de = catalog(Lang::De, json);
    assert_eq!(
        de.translate("Platform · just now").as_deref(),
        Some("Platform · gerade eben"),
        "each piece between \" · \""
    );
    assert_eq!(
        de.translate("identity 3Ah7 · the payment goes to a new address")
            .as_deref(),
        Some("identity 3Ah7 · die Zahlung geht an eine neue Adresse")
    );
    assert_eq!(
        de.translate("new address"),
        None,
        "a plural form needs a number"
    );
    assert_eq!(de.translate("Platform · untranslated"), None);
}

#[test]
fn a_template_needs_its_words_translated() {
    let json = r#"{
        "{} of {}": "{} von {}",
        "a new address made of the contact's key": "eine neue Adresse aus dem Schlüssel des Kontakts"
    }"#;
    let mut de = catalog(Lang::De, json);
    assert_eq!(de.translate("3 of 8").as_deref(), Some("3 von 8"));
    assert_eq!(
        de.translate("identity 3Ah7 · a new address made of the contact's key")
            .as_deref(),
        Some("identity 3Ah7 · eine neue Adresse aus dem Schlüssel des Kontakts"),
        "not \"{{}} von {{}}\" with English left in its places"
    );
}

#[test]
fn dates_and_numbers_by_language() {
    assert_eq!(
        date_in(Lang::Fr, 14, "octobre", Some(2026)),
        "14 octobre 2026"
    );
    assert_eq!(
        date_in(Lang::En, 14, "October", Some(2026)),
        "October 14, 2026"
    );
    assert_eq!(date_in(Lang::Ja, 14, "10月", Some(2026)), "2026年10月14日");
    assert_eq!(date_in(Lang::De, 3, "Okt.", None), "3. Okt.");
    assert_eq!(separators_in(Lang::En), (',', '.'));
    assert_eq!(separators_in(Lang::De), ('.', ','));
}

#[test]
fn a_key_followed_by_a_full_stop_is_still_translated() {
    let json = r#"{
        "the link is not a picture": "по ссылке не картинка",
        "The picture did not download: {}": "Картинка не скачалась: {}",
        "Wrong password": "パスワードが違います",
        "Done.": "Готово."
    }"#;
    let mut ru = catalog(Lang::Ru, json);
    // "{why}." around a core's reason, inside a template too.
    assert_eq!(
        ru.translate("The picture did not download: the link is not a picture.")
            .as_deref(),
        Some("Картинка не скачалась: по ссылке не картинка.")
    );
    assert_eq!(ru.translate("Done.").as_deref(), Some("Готово."));
    assert_eq!(ru.translate("Done..."), None, "only one full stop is ours");
    assert_eq!(ru.translate("Unknown words."), None);
    let mut ja = catalog(Lang::Ja, json);
    assert_eq!(
        ja.translate("Wrong password.").as_deref(),
        Some("パスワードが違います。"),
        "the language's own full stop"
    );
}

#[test]
fn a_reason_with_a_colon_and_a_full_stop_is_translated_whole() {
    // BUG-35: "{}: {}." fitted the reason by its colon, kept both halves English and won over
    // the key itself with the full stop after it.
    let reason =
        "This is not a Dash address: a typo or an extra character — the checksum does not match";
    let stops: &[char] = &['.', '。', '।'];
    let stopped = format!("{reason}.");
    for lang in Lang::ALL {
        let Some(json) = lang.catalog_json() else {
            continue;
        };
        let mut c = catalog(lang, json);
        let shown = c.translate(&stopped).unwrap_or_default();
        let bare = c.translate(reason).expect("the reason is in the catalog");
        assert_eq!(
            shown.trim_end_matches(stops),
            bare.trim_end_matches(stops),
            "{lang:?}"
        );
    }
    // A template still takes words it has no translation for when nothing else does.
    let mut ru = catalog(Lang::Ru, r#"{ "{}: {}.": "{}: {}." }"#);
    assert_eq!(
        ru.translate("Not saved: some words from a library nobody translated.")
            .as_deref(),
        Some("Not saved: some words from a library nobody translated.")
    );
}

#[test]
fn a_name_inside_arabic_keeps_its_at_sign_in_front() {
    // "Send 0.25 DASH to @alice" (the script check's line): bidi gave the neutral "@" the
    // paragraph's direction and drew "alice@".
    let shown = visual("إرسال ٠٫٢٥ داش إلى @alice");
    assert!(shown.contains("@alice"), "{shown:?}");
    assert!(!shown.contains("alice@"), "{shown:?}");
    // A full stop after the name stays with the sentence; two names, a hyphen inside one.
    let shown = visual("استُلم من @satoshi و @jam-bo.");
    assert!(
        shown.contains("@satoshi") && shown.contains("@jam-bo"),
        "{shown:?}"
    );
    assert!(
        !shown.contains(['\u{2068}', '\u{2069}']),
        "no isolate is drawn: {shown:?}"
    );
    // Already isolated by `own`, and an e-mail-like "a@b" in Latin text: left alone.
    assert_eq!(isolate_names(&own("@alice")), own("@alice"));
    assert_eq!(
        visual("Send to @alice"),
        "Send to @alice",
        "no Arabic, untouched"
    );
}

/// BUG-52: the English text (the keys) quotes with “…”, as English does; «…» is for the
/// languages that use it, in their catalogs.
#[test]
fn english_quotes_with_curly_quotes() {
    let keys: Vec<String> =
        serde_json::from_str(include_str!("../../locales/_keys.json")).expect("the key list");
    let guillemets: Vec<&String> = keys.iter().filter(|k| k.contains(['«', '»'])).collect();
    assert!(guillemets.is_empty(), "«…» in English: {guillemets:#?}");
}

/// BUG-70: an abbreviation's point ends the sentence too — «1 Min.», not «1 Min..»; a point
/// after a word, or a number stays.
#[test]
fn an_abbreviation_at_the_end_takes_no_second_point() {
    let shown = t_in(
        Lang::De,
        "You can try again. If the password is wrong again, the next pause is 1 min.",
    );
    assert!(shown.ends_with("die nächste Pause 1 Min."), "{shown}");
    let mut de = catalog(
        Lang::De,
        r#"{
            "{} min": "{} Min.",
            "Wait {}.": "Warten Sie {}.",
            "{} left.": "{} übrig."
        }"#,
    );
    assert_eq!(
        de.translate("Wait 5 min.").as_deref(),
        Some("Warten Sie 5 Min.")
    );
    assert_eq!(de.translate("Wait 5.").as_deref(), Some("Warten Sie 5."));
    assert_eq!(
        de.translate("5 min left.").as_deref(),
        Some("5 Min. übrig.")
    );
}

/// BUG-117: what the screen shows of `text` in the Arabic interface, left to right: the text as
/// [`visual`] gives it to egui, laid out by epaint (which shapes each run of one font with
/// HarfBuzz) in Inter with the app's Noto Sans Arabic, as `(char, x)` of each glyph by `x`.
/// Joined forms (U+FE80…) are read as their letters.
fn drawn(text: &str) -> Vec<(char, f32)> {
    use egui::{Color32, FontFamily, FontId};
    set(Lang::Ar);
    let shown = visual(text).into_owned();
    let mut fonts = app_fonts();
    let mut view = fonts.with_pixels_per_point(1.0);
    let galley = view.layout_no_wrap(
        shown,
        FontId::new(20.0, FontFamily::Proportional),
        Color32::WHITE,
    );
    set(Lang::En);
    let mut glyphs: Vec<(char, f32)> = galley.rows[0]
        .glyphs
        .iter()
        .map(|g| (letter(g.chr), g.pos.x))
        .collect();
    glyphs.sort_by(|a, b| a.1.total_cmp(&b.1));
    glyphs
}

/// Inter with the app's Noto Sans Arabic behind it, as the app's proportional family.
fn app_fonts() -> egui::epaint::text::Fonts {
    use egui::FontFamily;
    use egui::epaint::text::{FontData, FontDefinitions, Fonts, TextOptions};
    use std::sync::Arc;
    let mut defs = FontDefinitions::empty();
    defs.font_data.insert(
        "latin".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-latin-400.ttf"
        ))),
    );
    defs.font_data.insert(
        "arabic".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/NotoSansArabic-arabic-400.ttf"
        ))),
    );
    defs.families.insert(
        FontFamily::Proportional,
        vec!["latin".into(), "arabic".into()],
    );
    Fonts::new(TextOptions::default(), defs)
}

/// BUG-119: a paragraph is broken in reading order, and only then is each line turned into
/// visual order: the first line holds the first words.
#[test]
fn a_paragraph_is_broken_in_reading_order() {
    set(Lang::Ar);
    let short = |line: &str| line.chars().count() <= 5;
    let lines = t_wrapped("أ ب ت ث ج ح", short);
    let kept = t_wrapped(&format!("أ {} ب", own("x y z")), short);
    let whole = t_wrapped("أ ب ت", short);
    set(Lang::En);
    assert_eq!(lines, "ت ب أ\nح ج ث");
    assert_eq!(kept, "أ\nx y z\nب", "a person's words stay on one line");
    assert_eq!(whole, "ت ب أ", "what fits is left as t gives it");
    assert_eq!(t_wrapped("Two lines", |_| true), "Two lines");
}

/// BUG-119, as epaint draws it: an Arabic paragraph wrapped into three rows reads from the top
/// row's right edge — its first word rightmost there — to the last row's left edge, and the
/// short row is the last one. egui wrapped the line already in visual order, so the paragraph's
/// end came first, in a row of one or two words.
#[test]
fn an_arabic_paragraph_reads_down_from_its_first_row() {
    use egui::text::LayoutJob;
    use egui::{Color32, FontFamily, FontId};
    const TEXT: &str = "أضف جهة اتصال باسمها في المنصة ثم أرسل لها طلبا مدفوعا من رصيد هويتك وانتظر قبولها قبل الدفع";
    let mut fonts = app_fonts();
    let view = std::cell::RefCell::new(fonts.with_pixels_per_point(1.0));
    let font = FontId::new(20.0, FontFamily::Proportional);
    let width_of = |line: &str| {
        view.borrow_mut()
            .layout_no_wrap(line.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    set(Lang::Ar);
    let width = (width_of(&visual(TEXT)) / 2.6).floor();
    let shown = t_wrapped(TEXT, |line| width_of(line) <= width);
    set(Lang::En);
    let galley = view.borrow_mut().layout_job(LayoutJob::simple(
        shown.clone(),
        font.clone(),
        Color32::WHITE,
        width,
    ));
    assert_eq!(galley.rows.len(), 3, "three rows: {shown:?}");
    // Each row's letters by `x`, without the zero-width copies epaint adds after a right-to-left
    // run.
    let row_letters = |i: usize| -> Vec<char> {
        let mut glyphs: Vec<(char, f32)> = galley.rows[i]
            .glyphs
            .iter()
            .filter(|g| g.advance_width > 0.0 && !g.chr.is_whitespace())
            .map(|g| (letter(g.chr), g.pos.x))
            .collect();
        glyphs.sort_by(|a, b| a.1.total_cmp(&b.1));
        glyphs.into_iter().map(|g| g.0).collect()
    };
    let first = row_letters(0);
    let last = row_letters(2);
    assert_eq!(
        first.last(),
        Some(&'أ'),
        "the first word rightmost in the top row: {first:?}"
    );
    assert_eq!(
        last.first(),
        Some(&'ع'),
        "the last word leftmost in the last row: {last:?}"
    );
    let widths: Vec<f32> = galley.rows.iter().map(|r| r.rect().width()).collect();
    assert!(
        widths[2] < widths[0] && widths[2] < widths[1],
        "the last row is the short one: {widths:?}"
    );
    assert!(
        galley.rows.windows(2).all(|r| r[0].pos.y < r[1].pos.y),
        "rows go down"
    );
}

/// The letter a joined form (Arabic Presentation Forms-B, U+FE80…U+FEF4) stands for.
fn letter(c: char) -> char {
    // Each letter's forms in a row from U+FE80: isolated, final, and for joining ones initial
    // and medial.
    const FORMS: [(char, u32); 35] = [
        ('ء', 1),
        ('آ', 2),
        ('أ', 2),
        ('ؤ', 2),
        ('إ', 2),
        ('ئ', 4),
        ('ا', 2),
        ('ب', 4),
        ('ة', 2),
        ('ت', 4),
        ('ث', 4),
        ('ج', 4),
        ('ح', 4),
        ('خ', 4),
        ('د', 2),
        ('ذ', 2),
        ('ر', 2),
        ('ز', 2),
        ('س', 4),
        ('ش', 4),
        ('ص', 4),
        ('ض', 4),
        ('ط', 4),
        ('ظ', 4),
        ('ع', 4),
        ('غ', 4),
        ('ف', 4),
        ('ق', 4),
        ('ك', 4),
        ('ل', 4),
        ('م', 4),
        ('ن', 4),
        ('ه', 4),
        ('و', 2),
        ('ى', 2),
    ];
    let mut at = 0xFE80;
    for (base, n) in FORMS {
        if (at..at + n).contains(&(c as u32)) {
            return base;
        }
        at += n;
    }
    if (0xFEF1..0xFEF5).contains(&(c as u32)) {
        return 'ي';
    }
    c
}

/// Where the letters `word` are drawn, right to left as an Arabic reader goes: each one's `x`.
fn reads_right_to_left(glyphs: &[(char, f32)], word: &str) -> bool {
    let xs: Vec<f32> = word
        .chars()
        .map(|c| {
            glyphs
                .iter()
                .find(|(g, _)| *g == c)
                .unwrap_or_else(|| panic!("«{c}» drawn: {glyphs:?}"))
                .1
        })
        .collect();
    xs.windows(2).all(|w| w[0] > w[1])
}

/// BUG-117: an Arabic word is drawn right to left, its first letter rightmost. egui shapes it
/// (HarfBuzz turns a right-to-left run around), so [`visual`] must not turn it too.
#[test]
fn an_arabic_word_is_drawn_from_the_right() {
    for word in ["محفظة", "إضافة", "كلمة"] {
        let glyphs = drawn(word);
        assert!(
            reads_right_to_left(&glyphs, word),
            "«{word}» drawn as {:?}",
            glyphs.iter().map(|g| g.0).collect::<String>()
        );
    }
}

/// BUG-117: in an Arabic sentence the words follow one another from the right, each read from
/// its right; a Latin word, an amount or an address inside stays left to right.
#[test]
fn mixed_arabic_and_latin_keep_both_directions() {
    let text = "Platform في أرصدة DASH 12.5";
    let glyphs = drawn(text);
    let at = |word: &str| -> f32 {
        let seq: Vec<char> = word.chars().collect();
        let chars: Vec<char> = glyphs.iter().map(|g| g.0).collect();
        let i = chars
            .windows(seq.len())
            .position(|w| w == seq.as_slice())
            .unwrap_or_else(|| panic!("«{word}» left to right in {chars:?}"));
        glyphs[i].1
    };
    // The Latin pieces read left to right as they are.
    let (platform, dash, amount) = (at("Platform"), at("DASH"), at("12.5"));
    for word in ["في", "أرصدة"] {
        assert!(reads_right_to_left(&glyphs, word), "«{word}»: {glyphs:?}");
    }
    // A right-to-left paragraph: «Platform» first, so rightmost; then «في», «أرصدة», and
    // «DASH 12.5» leftmost, one left-to-right piece (a number after a Latin word goes with it).
    let fi = glyphs.iter().find(|g| g.0 == 'ف').unwrap().1;
    let arsida = glyphs.iter().find(|g| g.0 == 'أ').unwrap().1;
    assert!(
        platform > fi && fi > arsida && arsida > amount && amount > dash,
        "{glyphs:?}"
    );

    // An address inside an Arabic sentence: whole, left to right, after «إلى» to its left.
    let address = "yXk3Tq9mD8vJw2Rf";
    let glyphs = drawn(&format!("أرسل إلى {address}"));
    let chars: String = glyphs.iter().map(|g| g.0).collect();
    assert!(chars.contains(address), "{chars:?}");
    // «ل» is in «إلى» too: the first three letters are enough.
    assert!(reads_right_to_left(&glyphs, "أرس"), "{chars:?}");
}

/// BUG-119 (design review): an @name inside Arabic is drawn as it is written, «@carol» left to
/// right with its @ on the left — the neutral @ between Arabic and Latin does not take the
/// paragraph's direction. So in the middle of a line, at its end, and as the template's own
/// place «@{}» filled with a name a person typed (`own`).
#[test]
fn an_at_name_inside_arabic_keeps_its_at_on_the_left() {
    for text in [
        "الهوية 9Pw3 · تم التحقق من @carol بإثبات من Platform".to_owned(),
        // As «Add a contact» shows it, with the identity's short id.
        "الهوية 9Pw3Ex6Rb2…9Ga2Ju6N · تم التحقق من @carol بإثبات من Platform".to_owned(),
        "تم التحقق من @carol".to_owned(),
        format!("تم التحقق من @{} بإثبات", own("carol")),
        format!("@{} · تم التحقق من الاسم", own("carol")),
    ] {
        // Left to right on screen, spaces left out.
        let ink: String = drawn(&text)
            .iter()
            .map(|g| g.0)
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            ink.contains("@carol"),
            "«@carol» left to right in {text:?}: {ink}"
        );
    }
}

/// BUG-125: «Core · 2 of 3 coins» in the Arabic interface reads right to left as «Core · 2 من 3
/// عملات»: «Core» rightmost, as one left-to-right word, then 2, then 3. Without «Core» isolated,
/// the bidi algorithm took «Core · 2» as one left-to-right run, and the 2 stood left of «Core».
#[test]
fn core_in_the_coins_line_is_its_own_left_to_right_run() {
    use egui::{Color32, FontFamily, FontId};
    set(Lang::Ar);
    let shown = t("Core · 2 of 3 coins").into_owned();
    let whole = t("Core · 3 coins").into_owned();
    set(Lang::En);
    assert!(
        !shown.contains(['\u{2068}', '\u{2069}']),
        "no isolate is drawn: {shown:?}"
    );
    let x_of = |text: String| {
        let mut fonts = app_fonts();
        let mut view = fonts.with_pixels_per_point(1.0);
        let galley = view.layout_no_wrap(
            text.clone(),
            FontId::new(20.0, FontFamily::Proportional),
            Color32::WHITE,
        );
        let glyphs: Vec<(char, f32)> = galley.rows[0]
            .glyphs
            .iter()
            .map(|g| (letter(g.chr), g.pos.x))
            .collect();
        move |c: char| {
            glyphs
                .iter()
                .find(|(g, _)| *g == c)
                .map(|(_, x)| *x)
                .unwrap_or_else(|| panic!("no «{c}» in {text:?}"))
        }
    };
    let at = x_of(shown.clone());
    assert!(at('C') < at('e'), "«Core» left to right: {shown:?}");
    assert!(at('2') < at('C'), "2 after «Core», to its left: {shown:?}");
    assert!(at('3') < at('2'), "3 after 2, to its left: {shown:?}");
    let at = x_of(whole.clone());
    assert!(at('3') < at('C'), "3 after «Core», to its left: {whole:?}");
}

/// BUG-125 r1, P3 1: the isolate around «Core» in ar's three coins lines is invisible in an
/// editor and a diff, so the catalog writes it as escapes (U+2068, U+2069); and whatever
/// writes the catalog next, each of the three keeps it.
#[test]
fn the_arabic_coins_lines_keep_core_isolated() {
    let json = include_str!("../../locales/ar.json");
    let catalog: serde_json::Value = serde_json::from_str(json).expect("ar.json");
    for key in [
        "Core · {} of {} coins",
        "Core · {} coins",
        "Core · of {} coins",
    ] {
        let value = catalog[key]
            .as_str()
            .unwrap_or_else(|| panic!("no «{key}»"));
        assert!(
            value.starts_with("\u{2068}Core\u{2069} "),
            "«{key}»: {value:?}"
        );
    }
}
