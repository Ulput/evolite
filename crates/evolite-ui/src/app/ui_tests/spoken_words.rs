//! BUG-270 (QA: the pool's row read out as «Пришло privately…»), the whole class (TEAM.md §8d,
//! QA 6): no name a screen reader is given in ru or uk carries a word of the English interface —
//! on every screen a preset opens. A word of the English interface is one of the words of the
//! catalog's keys (`_keys.json`); the names that stay in Latin everywhere (Dash, Core, Platform,
//! InstantSend …) and the demo's own data (names, labels) are not counted.

use super::*;
use crate::cli::PRESETS;
use std::collections::BTreeSet;

/// Words that stay in Latin in every language — names, not words of the interface. Each group
/// says why; nothing the interface says in words is here («app», «pool», «connect» are
/// translated, and a Latin one is a fault the test must see).
const LATIN: &[&str] = &[
    // The product and the network's own names (the glossary keeps them in Latin).
    "dash",
    "core",
    "platform",
    "evolite",
    "evolution",
    "dashpay",
    "dpns",
    "testnet",
    "mainnet",
    "devnet",
    "regtest",
    "orchard",
    // Protocol names a translation keeps: InstantSend, ChainLock, CoinJoin, the asset lock in
    // brackets («перевод в Platform (asset lock)»), BIP/DIP, a spork.
    "instantsend",
    "instantlock",
    "chainlock",
    "coinjoin",
    "asset",
    "lock",
    "bip",
    "dip",
    "spork",
    // Apps and services by name.
    "jembe",
    "yappr",
    "imgur",
    "github",
    "android",
    "windows",
    "macos",
    "linux",
    // Formats, schemes and crypto names, written as they are.
    "nft",
    "nfts",
    "glb",
    "svg",
    "png",
    "jpg",
    "json",
    "pdf",
    "csv",
    "url",
    "uri",
    "https",
    "http",
    "com",
    "spv",
    "ecdsa",
    "bls",
    "xpub",
    "argon",
    "aes",
    "gcm",
    "sqlite",
    "txid",
    "hex",
    // Currency codes, and the units' codes next to an amount and on the unit switch («DASH»,
    // «mDASH», «duffs» — as `i18n::isolate_amounts` takes them; a sentence that names the unit
    // translates it: «даффов за кБ»).
    "usd",
    "eur",
    "duffs",
];

/// Phrases that are names whole: a product's («App Connect») and the demo's people, wallets and
/// labels — taken out of a label before it is read.
const NAMED_PHRASES: &[&str] = &[
    "App Connect",
    "Main",
    "Savings",
    "Travel",
    "Evo Coffee",
    "Workshop rent",
    "Alice Payments",
    "Exchange · withdrawal",
];

/// The demo's people, by name: in a contact's line, never translated.
const DEMO_PEOPLE: &[&str] = &[
    "alice", "chen", "bob", "kate", "carol", "diaz", "satoshi", "nakamoto", "werner", "alina",
];

/// The words of the English interface, lowercased, three letters or more.
fn interface_words() -> BTreeSet<String> {
    let keys: Vec<String> =
        serde_json::from_str(include_str!("../../../locales/_keys.json")).expect("the keys");
    let latin: BTreeSet<&str> = LATIN.iter().copied().collect();
    keys.iter()
        .flat_map(|k| {
            k.split(|c: char| !c.is_ascii_alphabetic())
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .filter(|w| w.len() >= 3 && !latin.contains(w.as_str()))
        .collect()
}

/// The demo's own words — NFTs, tokens, a wallet, a language's own name — taken out of a label
/// before it is read: a person's data is never translated.
const DEMO_PHRASES: &[&str] = &[
    "Coffin Dance",
    "Teddy",
    "Web Shot",
    "Tests with Bob",
    "Workshop Hours",
    "Garden Points",
    "Seasons of the city",
    "English",
];

fn english_in_spoken(lang: &str) {
    let words = interface_words();
    let mut wrong = BTreeSet::new();
    for preset in PRESETS {
        if matches!(*preset, "sync" | "sync-nodes") {
            continue;
        }
        let (_turn, mut kit) = demo_app(preset, lang);
        settle(&mut kit);
        for node in kit.query_all_by(|n| n.label().is_some()) {
            // Any space as a space, the direction marks of a person's words (`own`) out: a
            // name is found as it is written («Evo\u{a0}Coffee» is «Evo Coffee»).
            let mut label: String = node
                .accesskit_node()
                .label()
                .unwrap_or_default()
                .chars()
                .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
                .map(|c| if c.is_whitespace() { ' ' } else { c })
                .collect();
            for phrase in DEMO_PHRASES.iter().chain(NAMED_PHRASES) {
                label = label.replace(phrase, "");
            }
            // Latin words only, outside any longer token (an address, an id, a URL).
            for token in label.split(|c: char| c.is_whitespace()) {
                if token.chars().any(|c| c.is_ascii_digit()) || token.contains('@') {
                    continue;
                }
                for w in token.split(|c: char| !c.is_ascii_alphabetic()) {
                    // A word in capitals is a protocol's (HIGH, MASTER, WIF, DAPI), not a
                    // sentence's.
                    if w.len() > 1 && w.chars().all(|c| c.is_ascii_uppercase()) {
                        continue;
                    }
                    let w = w.to_lowercase();
                    if DEMO_PEOPLE.contains(&w.as_str()) {
                        continue;
                    }
                    if words.contains(&w) {
                        wrong.insert(format!("{preset}: «{w}» in «{label}»"));
                    }
                }
            }
        }
    }
    let wrong: Vec<_> = wrong.into_iter().take(40).collect();
    assert!(wrong.is_empty(), "{lang}:\n{}", wrong.join("\n"));
}

#[test]
fn no_english_interface_word_is_spoken_in_ru() {
    english_in_spoken("ru");
}

#[test]
fn no_english_interface_word_is_spoken_in_uk() {
    english_in_spoken("uk");
}

/// The painted words of the frame, nested shapes included.
fn painted(kit: &Kit) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &kit.output().shapes {
        walk(&c.shape, &mut out);
    }
    out
}

/// A Latin «identity» or «identities» standing as a word (BUG-250's glossary: never in ru or uk).
/// A DPNS name is the person's own («@werner555-identity-number-1…», BUG-265's long names): its
/// words are not the interface's.
fn latin_identity(text: &str) -> bool {
    text.split_whitespace()
        .filter(|token| !token.contains('@'))
        .any(latin_identity_in)
}

fn latin_identity_in(text: &str) -> bool {
    let l = text.to_lowercase();
    ["identity", "identities"].iter().any(|w| {
        l.match_indices(w).any(|(i, m)| {
            !l[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
                && !l[i + m.len()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric)
        })
    })
}

/// BUG-270 (QA: «для identity», «Платит identity · кредиты» in «Add a name»): the glossary's
/// «identity» held where the catalog cannot see it — text put together in code. No preset's
/// screen draws a Latin «identity» in ru or uk.
fn identity_drawn_in_latin(lang: &str) {
    let mut wrong = BTreeSet::new();
    for preset in PRESETS {
        if matches!(*preset, "sync" | "sync-nodes") {
            continue;
        }
        let (_turn, mut kit) = demo_app(preset, lang);
        settle(&mut kit);
        for text in painted(&kit).into_iter().filter(|t| latin_identity(t)) {
            wrong.insert(format!("{preset}: «{text}»"));
        }
    }
    let wrong: Vec<_> = wrong.into_iter().take(40).collect();
    assert!(wrong.is_empty(), "{lang}:\n{}", wrong.join("\n"));
}

#[test]
fn no_latin_identity_is_drawn_in_ru() {
    identity_drawn_in_latin("ru");
}

#[test]
fn no_latin_identity_is_drawn_in_uk() {
    identity_drawn_in_latin("uk");
}

/// BUG-270, QA's case itself: «Add a name» for an identity without a name, in ru — its sentences
/// say «личность», never «identity».
#[test]
fn add_a_name_for_a_nameless_identity_says_it_in_ru() {
    let (_turn, mut kit) = demo_app("add-name", "ru");
    settle(&mut kit);
    show_wallet_edited(
        &mut kit,
        4,
        "an identity without a name",
        |w| {
            if let Some(id) = w.identity.as_mut() {
                id.names.clear();
            }
        },
        |_| true,
    );
    let latin: Vec<String> = painted(&kit)
        .into_iter()
        .chain(
            kit.query_all_by(|n| n.label().is_some())
                .map(|n| n.accesskit_node().label().unwrap_or_default()),
        )
        .filter(|t| latin_identity(t))
        .collect();
    assert!(latin.is_empty(), "{latin:?}");
    assert!(
        has_words(&kit, "для вашей личности") || has_words(&kit, "Личность · кредиты"),
        "the nameless identity's own sentences"
    );
}
