//! The installer's catalogs against its code.

use super::*;

/// Every text the installer marks with `tr!`, read from its own sources.
fn marked() -> std::collections::BTreeSet<String> {
    let sources = [
        include_str!("main.rs"),
        include_str!("ui.rs"),
        include_str!("work.rs"),
        include_str!("system.rs"),
    ];
    let mut out = std::collections::BTreeSet::new();
    for src in sources {
        let mut rest = src;
        while let Some(at) = rest.find("tr!(") {
            // `include_str!(…)` ends in the same letters.
            let own = !rest[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_');
            // rustfmt may put the literal on the next line.
            rest = rest[at + 4..].trim_start();
            let Some(text) = rest.strip_prefix('"') else {
                continue;
            };
            // The first quote not escaped closes the literal.
            let mut escaped = false;
            let end = text
                .char_indices()
                .find(|&(_, c)| {
                    let close = c == '"' && !escaped;
                    escaped = c == '\\' && !escaped;
                    close
                })
                .map(|(i, _)| i)
                .expect("a closed tr!");
            if own {
                out.insert(text[..end].replace("\\\"", "\""));
            }
            rest = &text[end..];
        }
    }
    // Units of the sizes ([`plan::bytes`]), translated as templates.
    out.insert("{} MB".into());
    out.insert("{} GB".into());
    out
}

/// The catalogs read as evolite's do: a text and a template come back translated.
#[test]
fn the_catalogs_translate() {
    let json: serde_json::Value = serde_json::from_str(catalog(Lang::Ru).unwrap()).unwrap();
    let mut ru = i18n::Catalog::parse(Lang::Ru, catalog(Lang::Ru).unwrap()).unwrap();
    assert_eq!(ru.exact("Back"), json["Back"].as_str());
    let step = json["Step {} of {}"]
        .as_str()
        .unwrap()
        .replacen("{}", "2", 1)
        .replacen("{}", "4", 1);
    assert_eq!(ru.translate("Step 2 of 4"), Some(step));
}

/// Each language has every text, with as many blanks as the English: the values go into them
/// in order.
#[test]
fn every_language_has_every_text() {
    let marked = marked();
    assert!(marked.len() > 70, "{}", marked.len());
    for lang in Lang::ALL.into_iter().filter(|l| *l != Lang::En) {
        let json: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(catalog(lang).unwrap()).expect("a catalog");
        for key in &marked {
            let Some(value) = json.get(key).and_then(|v| v.as_str()) else {
                panic!("{}: no translation of {key:?}", lang.code());
            };
            assert_eq!(
                value.matches("{}").count(),
                key.matches("{}").count(),
                "{}: {key:?}",
                lang.code()
            );
        }
        for key in json.keys() {
            assert!(
                marked.contains(key),
                "{}: {key:?} is not in the code",
                lang.code()
            );
        }
    }
}
