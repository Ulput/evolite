//! BUG-154: every address, hash, id and key is drawn in the monospace font. In Inter a
//! lowercase «x» between digits reads as «×» (`tdash1qq7x4m…` looked like `…7×4…`), and the
//! person checks an address before signing. The sweep looks at what each preset really draws:
//! every run of text that is an address, a txid, an id or a key — whole or shortened — must be
//! in a monospace family.

use super::*;

/// Whether `family` is one of the monospace families (`theme::mono`).
fn is_mono(family: &egui::FontFamily) -> bool {
    match family {
        egui::FontFamily::Monospace => true,
        egui::FontFamily::Name(name) => name.starts_with("mono"),
        egui::FontFamily::Proportional => false,
    }
}

use crate::components::{address_like, address_runs as runs};

/// Every text shape of the last frame.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<std::sync::Arc<egui::Galley>> {
    fn walk(shape: &egui::Shape, out: &mut Vec<std::sync::Arc<egui::Galley>>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// The runs drawn in a proportional font on the screen as it is now: «run» in «its text».
fn not_mono(kit: &Kit) -> Vec<String> {
    let mut wrong = Vec::new();
    for galley in texts(&kit.output().shapes) {
        let job = &galley.job;
        for run in runs(&job.text) {
            let proportional = job.sections.iter().any(|section| {
                usize::from(section.byte_range.start) < run.end
                    && run.start < usize::from(section.byte_range.end)
                    && !is_mono(&section.format.font_id.family)
            });
            if proportional {
                wrong.push(format!("«{}» in «{}»", &job.text[run.clone()], job.text));
            }
        }
    }
    wrong.sort();
    wrong.dedup();
    wrong
}

/// What the sweep takes as an address, and what it does not.
#[test]
fn the_sweep_knows_an_address_when_it_sees_one() {
    for yes in [
        "tdash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq",
        "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f",
        "yNBw…Reb4",
        "tdash1qqmz…aznp",
        "9LXd7BxEFCQQmRf2LVawaJAzTW79VSiacB5eRZQ59HwW",
        "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9",
    ] {
        assert!(address_like(yes), "{yes}");
    }
    for no in [
        "DASH",
        "0,1000",
        "evolite",
        "InstantSend",
        "Platform",
        "…",
        "12…",
        "ChainLock",
        "1024",
    ] {
        assert!(!address_like(no), "{no}");
    }
    assert_eq!(runs("To tdash1qq7x4m · 0,1 DASH").len(), 1);
}

/// The sweep over every preset (en): no address, hash, id or key in a proportional font.
#[test]
fn every_address_on_every_screen_is_in_the_monospace_font() {
    let mut wrong = Vec::new();
    for preset in crate::cli::PRESETS {
        let (_turn, mut kit) = demo_app(preset, "en");
        // A few frames for the demo core's answers: some presets (the sync) never go quiet.
        for _ in 0..12 {
            kit.step();
            std::thread::sleep(Duration::from_millis(10));
        }
        for found in not_mono(&kit) {
            wrong.push(format!("{preset}: {found}"));
        }
    }
    wrong.dedup();
    assert!(
        wrong.is_empty(),
        "{} addresses in a proportional font:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The same in Arabic, on the screens with the most addresses: the words are drawn in visual
/// order there, an address still in the monospace font.
#[test]
fn in_arabic_too_an_address_is_in_the_monospace_font() {
    let mut wrong = Vec::new();
    for preset in [
        "send-confirm",
        "send-book",
        "activity-card-withdrawal-paid",
        "activity-card-top-up",
        "tokens-review",
        "dashpay",
        "contacts",
    ] {
        let (_turn, mut kit) = demo_app(preset, "ar");
        for _ in 0..12 {
            kit.step();
            std::thread::sleep(Duration::from_millis(10));
        }
        for found in not_mono(&kit) {
            wrong.push(format!("{preset}: {found}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Words drawn in a monospace font in a line that also holds an address or an @name: «word» in
/// «its text». Amounts, the recovery phrase and section headings are mono on their own lines by
/// design; the words around an id or a name are not (the designer, BUG-154: «@alice · Alice
/// Chen» and «to an address from the contact's key: …» were all in mono).
fn mono_words(kit: &Kit) -> Vec<String> {
    let mut wrong = Vec::new();
    for galley in texts(&kit.output().shapes) {
        let job = &galley.job;
        let ids = runs(&job.text);
        // An @name starts a word (an e-mail is not one); a link or a payment URI is code.
        let named = job.text.starts_with('@') || job.text.contains(" @");
        let code = job.text.contains("://") || job.text.starts_with("dash:");
        if (ids.is_empty() && !named) || code {
            continue;
        }
        // Each run of letters with where it starts, in bytes.
        let mut words = Vec::new();
        let mut start = None;
        for (i, c) in job
            .text
            .char_indices()
            .chain(std::iter::once((job.text.len(), ' ')))
        {
            match (start, c.is_alphabetic()) {
                (None, true) => start = Some(i),
                (Some(s), false) => {
                    words.push(s..i);
                    start = None;
                }
                _ => {}
            }
        }
        for word in words {
            let text = &job.text[word.clone()];
            let code = text.len() <= 5 && text.chars().all(|c| c.is_uppercase());
            let in_id = ids
                .iter()
                .any(|r| r.start <= word.start && word.end <= r.end);
            let mono = job.sections.iter().any(|s| {
                usize::from(s.byte_range.start) <= word.start
                    && word.end <= usize::from(s.byte_range.end)
                    && is_mono(&s.format.font_id.family)
            });
            if mono && text.chars().count() >= 4 && !code && !in_id {
                wrong.push(format!("«{text}» in «{}»", job.text));
            }
        }
    }
    wrong.sort();
    wrong.dedup();
    wrong
}

/// The other half of the sweep: no words in the monospace font beside an id or a name, every
/// preset (en).
#[test]
fn no_words_beside_an_id_are_in_the_monospace_font() {
    let mut wrong = Vec::new();
    for preset in crate::cli::PRESETS {
        let (_turn, mut kit) = demo_app(preset, "en");
        for _ in 0..12 {
            kit.step();
            std::thread::sleep(Duration::from_millis(10));
        }
        for found in mono_words(&kit) {
            wrong.push(format!("{preset}: {found}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} words in the monospace font:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// BUG-159: an e-mail, a version (0.1.0) and a short commit hash (7–12 hex) are code too, in the
/// monospace font — and a date, an amount with its thousands, an ordinary word or number are not.
#[test]
fn an_email_a_version_and_a_short_hash_are_code() {
    let only = |text: &str| -> Vec<String> {
        runs(text).into_iter().map(|r| text[r].to_owned()).collect()
    };
    assert_eq!(
        only("To: ulput@protonmail.com · subject"),
        ["ulput@protonmail.com"]
    );
    assert_eq!(only("evolite 0.1.0 (a97a98e)"), ["0.1.0", "a97a98e"]);
    assert_eq!(only("platform 575d961, v1.2.13."), ["575d961", "v1.2.13"]);
    assert_eq!(only("rust-dashcore aca0202"), ["aca0202"]);
    for no in [
        "14.11.2026",
        "1.200.000 DASH",
        "3.5 DASH",
        "facade and decade",
        "1234567 credits",
        "@alice and @bob",
        "e.g. this",
        "Nov 14, 14:59",
        "0,1000 DASH",
        "ChainLock up to ~2.5 min",
    ] {
        assert_eq!(only(no), Vec::<String>::new(), "{no}");
    }
}
