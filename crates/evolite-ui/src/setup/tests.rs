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

/// SEC-8 (Пр3, review S2): `--pack` refuses a program built with `dev-tools` or without the
/// real core (each carries its mark), and packs one without either.
#[test]
fn a_dev_tools_or_demo_only_program_is_not_packed() {
    let mut program = vec![0u8; 4096];
    assert_eq!(not_a_release(&program), None);
    program[1000..1023].copy_from_slice(b"evolite:dev-tools-build");
    assert!(not_a_release(&program).is_some_and(|why| why.contains("dev-tools")));
    let mut program = vec![0u8; 4096];
    program[2000..2023].copy_from_slice(b"evolite:demo-core-build");
    assert!(not_a_release(&program).is_some_and(|why| why.contains("real core")));
    // The app's `main.rs` puts exactly these marks into such builds.
    let app = include_str!("../main.rs");
    for mark in [
        r#"static DEV_TOOLS_MARK: [u8; 23] = *b"evolite:dev-tools-build";"#,
        r#"static DEMO_CORE_MARK: [u8; 23] = *b"evolite:demo-core-build";"#,
    ] {
        assert!(
            app.contains(mark),
            "the app's mark is the one --pack looks for: {mark}"
        );
    }
}

/// Beta list §5а item 10: the removal names both networks' data folders, each with its network,
/// whether or not they exist yet.
#[test]
fn the_removal_names_both_network_folders() {
    let base = std::env::temp_dir().join(format!("evolite-removal-{}", std::process::id()));
    let known = super::plan::Known {
        local: base.join("Local"),
        roaming: base.join("Roaming"),
        programs: base.join("Local").join("Programs"),
    };
    let said = super::ui::removal_folders(&known);
    assert!(
        said.contains(&format!("{} (testnet)", known.testnet_data().display())),
        "{said}"
    );
    assert!(
        said.contains(&format!("{} (mainnet)", known.mainnet_data().display())),
        "{said}"
    );
}

/// Installer review r1: with the picture cache left, the last screen never calls it removed, and
/// with the data gone it does not say evolite keeps nothing more; without, as before.
#[test]
fn the_summary_never_calls_a_left_cache_removed() {
    use super::ui::removed_summary;
    for data_gone in [true, false] {
        let said = removed_summary(data_gone, true, "/data/evolite");
        assert!(!said.contains("picture cache"), "{said}");
        assert!(!said.contains("keeps nothing more"), "{said}");
    }
    assert!(removed_summary(true, false, "").contains("keeps nothing more"));
    assert!(removed_summary(false, false, "/data/evolite").contains("picture cache"));
    assert!(removed_summary(false, true, "/data/evolite").contains("/data/evolite"));
}
