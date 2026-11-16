//! BUG-202: the words of a recovery phrase are never translated, on any screen that shows them.
//! Twelve BIP39 words are words of the interface too («address», «open»…): in Russian they
//! came out as «адрес», «открыт», and a phrase written down so restores nothing.

use super::*;
use crate::i18n::{Lang, t_in};
use evolite_api::{Event, Phrase, PhraseQuestion};

/// BIP39 words that Russian would translate, repeated up to `n`.
fn interface_words(n: usize) -> Vec<String> {
    let found: Vec<&str> = evolite_core::bip39::words()
        .filter(|w| t_in(Lang::Ru, w) != *w)
        .collect();
    assert!(found.len() >= 4, "{found:?}");
    found
        .iter()
        .cycle()
        .take(n)
        .map(|w| w.to_string())
        .collect()
}

/// The texts painted this frame, nested shapes included.
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

/// Every word in `words` is painted as it is, and named so for a screen reader when `named`;
/// no word's translation is painted or named.
fn words_as_they_are(kit: &Kit, words: &[String], named: bool) {
    let painted = painted(kit);
    for w in words {
        assert!(painted.iter().any(|t| t == w), "«{w}» painted: {painted:?}");
        if named {
            assert!(has_text(kit, w), "«{w}» named");
        }
        let translated = t_in(Lang::Ru, w).into_owned();
        assert!(
            !painted.contains(&translated) && !has_text(kit, &translated),
            "«{w}» shown as «{translated}»"
        );
    }
}

/// O1, «Write the phrase down»: the grid of words.
#[test]
fn the_new_phrase_is_shown_in_its_own_words() {
    let (_turn, mut kit) = demo_app("create", "ru");
    kit.run_steps(5);
    let words = interface_words(12);
    kit.state_mut()
        .onboarding
        .as_mut()
        .expect("the onboarding")
        .on_event(&Event::NewPhrase(Phrase::new(words.clone())));
    kit.run_steps(3);
    words_as_they_are(&kit, &words, false);
}

/// O2, «Check the phrase»: the four choices of each asked word.
#[test]
fn the_check_offers_the_words_in_their_own_words() {
    let (_turn, mut kit) = demo_app("verify", "ru");
    kit.run_steps(5);
    let words = interface_words(12);
    kit.state_mut()
        .onboarding
        .as_mut()
        .expect("the onboarding")
        .on_event(&Event::NewPhrase(Phrase::new(words.clone())));
    kit.run_steps(3);
    // The choices are words of the phrase: each one offered is drawn and named as it is.
    let offered: Vec<String> = words
        .iter()
        .filter(|w| painted(&kit).contains(w))
        .cloned()
        .collect();
    assert!(offered.len() >= 3, "{offered:?}");
    words_as_they_are(&kit, &offered, true);
}

/// O3, the restore: a suggestion for the word being typed.
#[test]
fn a_restore_suggests_the_word_in_its_own_words() {
    let (_turn, mut kit) = demo_app("restore", "ru");
    kit.run_steps(5);
    kit.state_mut()
        .onboarding
        .as_mut()
        .expect("the onboarding")
        .type_restore_word(5, "addr");
    kit.run_steps(3);
    words_as_they_are(&kit, &["address".to_string()], true);
}

/// Settings › Show the phrase: the phrase on one line.
#[test]
fn the_phrase_in_settings_is_shown_in_its_own_words() {
    let (_turn, mut kit) = demo_app("phrase", "ru");
    kit.run_steps(5);
    let words = interface_words(12);
    kit.state_mut()
        .settings
        .on_revealed(Ok(Phrase::new(words.clone())));
    kit.run_steps(3);
    let line = words.join(" ");
    assert!(has_text(&kit, &line), "the phrase as it is");
    // Unmarked, the line is not shown as the catalog would make it.
    let translated = t_in(Lang::Ru, &line).into_owned();
    assert!(
        translated == line || !has_text(&kit, &translated),
        "«{translated}»"
    );
}

/// Settings › Check the phrase: the choices of each asked word.
#[test]
fn the_settings_check_offers_the_words_in_their_own_words() {
    let (_turn, mut kit) = demo_app("check-phrase", "ru");
    wait_for(&mut kit, "the questions", |kit| {
        kit.state()
            .settings
            .check
            .as_ref()
            .is_some_and(|c| c.questions.is_some())
    });
    let words = interface_words(12);
    let questions: Vec<PhraseQuestion> = words
        .chunks(3)
        .enumerate()
        .map(|(q, choices)| PhraseQuestion {
            number: q * 3 + 1,
            choices: choices.to_vec(),
        })
        .collect();
    if let Some(check) = kit.state_mut().settings.check.as_mut() {
        check.picks = vec![None; questions.len()];
        check.questions = Some(questions);
        check.verdict = None;
    }
    kit.run_steps(3);
    words_as_they_are(&kit, &words, true);
}
