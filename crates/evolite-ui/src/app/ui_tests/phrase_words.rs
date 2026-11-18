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

/// The core's phrase first, then `words` instead: given before it, the core's would replace
/// them (a loaded run was slow enough for that, batch 26).
fn give_phrase(kit: &mut Kit, words: &[String]) {
    wait_for(kit, "the core's phrase", |k| {
        k.state()
            .onboarding
            .as_ref()
            .is_some_and(|o| o.has_phrase())
    });
    kit.state_mut()
        .onboarding
        .as_mut()
        .expect("the onboarding")
        .on_event(&Event::NewPhrase(Phrase::new(words.to_vec())));
}

/// Waits until `n` of `words` are painted: a fixed number of frames is not enough in a loaded
/// run (batch 26, BUG-187).
fn wait_painted(kit: &mut Kit, words: &[String], n: usize) {
    wait_for(kit, "the words painted", |k| {
        let painted = painted(k);
        words.iter().filter(|w| painted.contains(w)).count() >= n
    });
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
    let words = interface_words(12);
    give_phrase(&mut kit, &words);
    wait_painted(&mut kit, &words, words.len());
    words_as_they_are(&kit, &words, false);
}

/// O2, «Check the phrase»: the four choices of each asked word.
#[test]
fn the_check_offers_the_words_in_their_own_words() {
    let (_turn, mut kit) = demo_app("verify", "ru");
    let words = interface_words(12);
    give_phrase(&mut kit, &words);
    wait_painted(&mut kit, &words, 3);
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
    wait_painted(&mut kit, &["address".to_string()], 1);
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
    let line = words.join(" ");
    wait_for(&mut kit, "the phrase", |k| has_text(k, &line));
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
    wait_painted(&mut kit, &words, words.len());
    words_as_they_are(&kit, &words, true);
}
