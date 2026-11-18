//! BUG-228 (option C) and SEC-1: no word of a recovery phrase, a private key or a password
//! shown reaches the accessibility tree — any program reads it there (UI Automation) — unless
//! the person turns «Read the words to a screen reader» on, for one showing of that screen. Then
//! each word is a node of its own. Any frame without the screen turns it off again.

use super::*;
use crate::screens::Screen;
use evolite_api::{Command, Event, Phrase, PhraseQuestion};

/// Words no screen says by itself.
fn phrase() -> Vec<String> {
    [
        "zebra", "walnut", "umbrella", "tunnel", "stove", "rocket", "puzzle", "orbit", "noodle",
        "mango", "kiwi", "jungle",
    ]
    .map(String::from)
    .to_vec()
}

const TOGGLE: &str = "Read the words to a screen reader";
const ONE_SHOWING: &str = "for this showing only";

/// Every label, value and description in the tree that holds one of `words` (a whole word).
fn secrets_in_tree(kit: &Kit, words: &[String]) -> Vec<String> {
    let holds = |text: &str| {
        text.split(|c: char| !c.is_alphanumeric())
            .any(|t| words.iter().any(|w| w == t))
    };
    kit.query_all_by(|_| true)
        .flat_map(|n| {
            {
                let node = n.accesskit_node();
                [node.label(), node.value(), node.description()]
            }
            .into_iter()
            .flatten()
            .filter(|t| holds(t))
            .collect::<Vec<_>>()
        })
        .collect()
}

/// With reading on: each word in the tree, one per node, a node with no other.
fn one_word_per_node(kit: &Kit, words: &[String]) {
    let found = secrets_in_tree(kit, words);
    for w in words {
        assert!(found.iter().any(|t| t == w), "«{w}» read: {found:?}");
    }
    assert!(
        found.iter().all(|t| words.contains(t)),
        "one word per node: {found:?}"
    );
}

fn none_in_tree(kit: &Kit, words: &[String], screen: &str) {
    let found = secrets_in_tree(kit, words);
    assert!(found.is_empty(), "{screen}: {found:?}");
}

fn reading_on(kit: &mut Kit) {
    // Clicked until it is on, and said on screen: a loaded run may take more than a few frames,
    // and a dialog still settling may miss a click (the BUG-228 check).
    let started = Instant::now();
    while !has_words(kit, ONE_SHOWING) {
        assert!(started.elapsed() < PATIENCE, "reading not on");
        kit.query_by_role_and_label(Role::CheckBox, TOGGLE)
            .expect("the switch")
            .click();
        kit.run_steps(2);
    }
    // Settled: the switch redoes its frame (review r1, M2); two more do no harm.
    kit.run_steps(2);
}

fn new_phrase(kit: &mut Kit, words: &[String]) {
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
    kit.run_steps(3);
}

/// O1, the new phrase: painted, none in the tree; with reading on, one per node.
#[test]
fn the_new_phrase_reaches_the_tree_only_when_read() {
    let (_turn, mut kit) = demo_app("create", "en");
    kit.run_steps(5);
    let words = phrase();
    new_phrase(&mut kit, &words);
    assert!(painted_has(&kit, "zebra"), "on screen");
    none_in_tree(&kit, &words, "O1");
    assert!(!has_words(&kit, ONE_SHOWING));
    reading_on(&mut kit);
    one_word_per_node(&kit, &words);
    // Leaving the step ends the showing: back on it, reading is off.
    if let Some(o) = kit.state_mut().onboarding.as_mut() {
        o.step = crate::screens::onboarding::Step::Verify;
    }
    kit.run_steps(3);
    if let Some(o) = kit.state_mut().onboarding.as_mut() {
        o.step = crate::screens::onboarding::Step::Phrase;
    }
    kit.run_steps(3);
    none_in_tree(&kit, &words, "O1 shown again");
    assert!(!has_words(&kit, ONE_SHOWING));
}

/// O2, the check: the choices are named by their place; with reading on, by their word.
#[test]
fn the_check_choices_reach_the_tree_only_when_read() {
    let (_turn, mut kit) = demo_app("verify", "en");
    kit.run_steps(5);
    let words = phrase();
    new_phrase(&mut kit, &words);
    none_in_tree(&kit, &words, "O2");
    assert!(has_text(&kit, "Choice 1"));
    reading_on(&mut kit);
    let offered: Vec<String> = words
        .iter()
        .filter(|w| has_text(&kit, w))
        .cloned()
        .collect();
    assert!(offered.len() >= 3, "{offered:?}");
}

/// O3, the restore with typing shown: the fields give no value, the suggestion no word.
#[test]
fn the_restore_fields_reach_the_tree_only_when_read() {
    let (_turn, mut kit) = demo_app("restore", "en");
    kit.run_steps(5);
    let typed = ["abandon", "ability", "able"].map(String::from).to_vec();
    if let Some(o) = kit.state_mut().onboarding.as_mut() {
        for (i, w) in typed.iter().enumerate() {
            o.type_restore_word(i, w);
        }
        o.type_restore_word(5, "zebr");
    }
    kit.run_steps(3);
    assert!(painted_has(&kit, "ability"), "on screen");
    let mut secret = typed.clone();
    secret.extend(["zebr".to_string(), "zebra".to_string()]);
    none_in_tree(&kit, &secret, "O3");
    reading_on(&mut kit);
    assert!(has_text(&kit, "zebra"), "the suggestion by its word");
    assert!(
        kit.query_all_by(|n| n.value().as_deref() == Some("ability"))
            .next()
            .is_some(),
        "a field by its word"
    );
}

fn settings_phrase(kit: &mut Kit, words: &[String]) {
    kit.state_mut().settings.phrase_dialog = true;
    kit.state_mut()
        .settings
        .on_revealed(Ok(Phrase::new(words.to_vec())));
    kit.run_steps(3);
}

/// Settings › Show the phrase: painted, named without its words; with reading on, one per node.
#[test]
fn the_phrase_in_settings_reaches_the_tree_only_when_read() {
    let (_turn, mut kit) = demo_app("phrase", "en");
    kit.run_steps(5);
    let words = phrase();
    settings_phrase(&mut kit, &words);
    assert!(painted_has(&kit, "zebra walnut"), "on screen");
    none_in_tree(&kit, &words, "Show the phrase");
    assert!(has_words(&kit, "Recovery phrase, 12 words"));
    reading_on(&mut kit);
    one_word_per_node(&kit, &words);
}

/// Settings › Check the phrase: the pills are named by their place unless reading is on.
#[test]
fn the_settings_check_reaches_the_tree_only_when_read() {
    let (_turn, mut kit) = demo_app("check-phrase", "en");
    wait_for(&mut kit, "the questions", |kit| {
        kit.state()
            .settings
            .check
            .as_ref()
            .is_some_and(|c| c.questions.is_some())
    });
    let words = phrase();
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
    none_in_tree(&kit, &words, "Check the phrase");
    reading_on(&mut kit);
    one_word_per_node(&kit, &words);
}

/// SEC-1: a private key shown: painted, named without it; with reading on, in the tree.
#[test]
fn a_shown_key_reaches_the_tree_only_when_read() {
    let (_turn, mut kit) = demo_app("identity-key-shown", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    let key = [
        evolite_core::DEMO_KEY_WIF.to_string(),
        evolite_core::DEMO_KEY_HEX.to_string(),
    ];
    wait_for(&mut kit, "the key", |kit| {
        key.iter().all(|k| painted_has(kit, k))
    });
    none_in_tree(&kit, &key, "the key, WIF and hex");
    reading_on(&mut kit);
    for k in &key {
        assert!(has_words(&kit, k), "{k} read");
    }
}

/// What the app gave the clipboard in the last frame, by egui (not `copy_secret`).
fn copied(kit: &Kit) -> Vec<String> {
    kit.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

/// The node named `label` selected with the mouse across, then Ctrl+C: what was copied.
fn copied_by_selecting(kit: &mut Kit, label: &str) -> Vec<String> {
    let rect = kit
        .query_by_label(label)
        .unwrap_or_else(|| panic!("«{label}» in the tree"))
        .rect();
    let (from, to) = (
        rect.left_center() + egui::vec2(1.0, 0.0),
        rect.right_center() - egui::vec2(1.0, 0.0),
    );
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    kit.event(egui::Event::PointerMoved(from));
    kit.event(button(from, true));
    kit.step();
    kit.event(egui::Event::PointerMoved(to));
    kit.step();
    kit.event(button(to, false));
    kit.step();
    kit.event(egui::Event::Copy);
    kit.step();
    copied(kit)
}

/// Review r1, M1: with reading on, the words and the key are in the tree but not selectable:
/// Ctrl+C would put them on the ordinary clipboard, past `copy_secret` (its history and cloud
/// exclusion, its 60 s wipe).
#[test]
fn the_words_read_cannot_be_copied_by_selecting_them() {
    let (_turn, mut kit) = demo_app("phrase", "en");
    kit.run_steps(5);
    let words = phrase();
    settings_phrase(&mut kit, &words);
    reading_on(&mut kit);
    let got = copied_by_selecting(&mut kit, "zebra");
    assert!(got.is_empty(), "a word of the phrase copied: {got:?}");
    drop((kit, _turn));

    let (_turn, mut kit) = demo_app("identity-key-shown", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    let wif = evolite_core::DEMO_KEY_WIF;
    wait_for(&mut kit, "the key", |kit| painted_has(kit, wif));
    reading_on(&mut kit);
    for key in [wif, evolite_core::DEMO_KEY_HEX] {
        let got = copied_by_selecting(&mut kit, key);
        assert!(got.is_empty(), "the key copied: {got:?}");
    }
}

/// Review r1, M2: turning reading off takes the words out of the tree in the same frame — the
/// next one may not come before the mouse moves. So does «Hide» on the new phrase.
#[test]
fn reading_off_and_hide_take_the_words_out_in_the_same_frame() {
    let (_turn, mut kit) = demo_app("phrase", "en");
    kit.run_steps(5);
    let words = phrase();
    settings_phrase(&mut kit, &words);
    reading_on(&mut kit);
    one_word_per_node(&kit, &words);
    let started = Instant::now();
    // A click a loaded run misses changes nothing: clicked again. One frame each.
    while has_words(&kit, ONE_SHOWING) {
        assert!(started.elapsed() < PATIENCE, "reading not off");
        kit.query_by_role_and_label(Role::CheckBox, TOGGLE)
            .expect("the switch")
            .click();
        kit.step();
    }
    none_in_tree(&kit, &words, "Show the phrase, the frame reading went off");
    drop((kit, _turn));

    let (_turn, mut kit) = demo_app("create", "en");
    kit.run_steps(5);
    new_phrase(&mut kit, &words);
    reading_on(&mut kit);
    one_word_per_node(&kit, &words);
    let hidden = |kit: &Kit| {
        kit.state()
            .onboarding
            .as_ref()
            .is_some_and(|o| o.phrase_hidden())
    };
    let started = Instant::now();
    while !hidden(&kit) {
        assert!(started.elapsed() < PATIENCE, "not hidden");
        kit.query_all_by_role_and_label(Role::Button, "Hide")
            .last()
            .expect("«Hide»")
            .click();
        kit.step();
    }
    none_in_tree(&kit, &words, "O1, the frame it was hidden");
}

/// SEC-1: a password shown with «Show» stays out of the tree.
#[test]
fn a_password_shown_stays_out_of_the_tree() {
    let (_turn, mut kit) = demo_app("phrase", "en");
    kit.run_steps(5);
    kit.state_mut().settings.password = "zebra-walnut-7".to_string().into();
    kit.run_steps(2);
    // The field's «Show» (the page's «Show the phrase» row is behind the dialog).
    kit.query_all_by_role_and_label(Role::Button, "Show")
        .last()
        .expect("«Show»")
        .click();
    kit.run_steps(3);
    assert!(painted_has(&kit, "zebra-walnut-7"), "shown");
    none_in_tree(&kit, &["zebra".to_string()], "the password shown");
    drop((kit, _turn));

    // The sign-in screen's own field (review r1, S1): its «Show» masks the same way.
    let (_turn, mut kit) = demo_app("unlock", "en");
    kit.run_steps(3);
    kit.state_mut().unlock.password = "zebra-walnut-7".to_string().into();
    kit.run_steps(2);
    kit.query_all_by_role_and_label(Role::Button, "Show")
        .last()
        .expect("«Show»")
        .click();
    kit.run_steps(3);
    assert!(painted_has(&kit, "zebra-walnut-7"), "shown at sign-in");
    none_in_tree(
        &kit,
        &["zebra".to_string()],
        "the password shown at sign-in",
    );
}

/// The customer: reading is on for one showing only. Closing, the 60 s timer, a lock and
/// another wallet each turn it off, and it comes back off.
#[test]
fn reading_ends_with_the_showing() {
    let (_turn, mut kit) = demo_app("phrase", "en");
    kit.run_steps(5);
    let words = phrase();
    let again = |kit: &mut Kit, what: &str| {
        settings_phrase(kit, &words);
        none_in_tree(kit, &words, what);
        assert!(!has_words(kit, ONE_SHOWING), "{what}: off");
    };
    // Closed and opened again.
    settings_phrase(&mut kit, &words);
    reading_on(&mut kit);
    kit.state_mut().settings.phrase_dialog = false;
    kit.run_steps(3);
    again(&mut kit, "closed");
    // Hidden by the timer.
    reading_on(&mut kit);
    kit.state_mut().settings.revealed_at =
        Some(std::time::Instant::now() - std::time::Duration::from_secs(61));
    kit.run_steps(3);
    assert!(!painted_has(&kit, "zebra"), "hidden");
    again(&mut kit, "hidden by the timer");
    // Left for another screen.
    reading_on(&mut kit);
    kit.state_mut().screen = Screen::Overview;
    kit.run_steps(3);
    kit.state_mut().screen = Screen::Settings;
    again(&mut kit, "left");
    // A lock.
    reading_on(&mut kit);
    show_edited(
        &mut kit,
        2,
        "locked",
        |app| {
            if let Some(s) = app.snapshot.as_mut() {
                s.vault = evolite_api::VaultState::Locked;
            }
        },
        |_| true,
    );
    kit.run_steps(3);
    again(&mut kit, "locked");
    // Another wallet.
    reading_on(&mut kit);
    kit.state_mut().core.send(Command::SwitchWallet {
        id: "5e6f7a8b".into(),
        password: Some(evolite_api::Password::new("savings-password".into())),
    });
    wait_for(&mut kit, "Savings open", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallets.iter().find(|w| w.open))
            .is_some_and(|w| w.id == "5e6f7a8b")
    });
    kit.state_mut().screen = Screen::Settings;
    again(&mut kit, "another wallet");
}
