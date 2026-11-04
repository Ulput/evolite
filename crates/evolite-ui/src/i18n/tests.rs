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
fn arabic_is_joined_and_put_in_visual_order() {
    // "سلام" (salam): four letters, joined; drawn left to right they come last first.
    let shown = visual("سلام");
    assert_ne!(shown, "سلام", "joined forms and reversed");
    assert!(
        shown.chars().all(|c| !matches!(c, 'س' | 'ل' | 'ا' | 'م')),
        "presentation forms: {shown:?}"
    );
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
