//! FEAT-0035: the address in «Check the payment» without spaces; a look-alike's differences said
//! in words; the address whole at 800×600 and in Arabic.

use super::*;
use crate::app::dev::LOOKALIKE;

const KNOWN: &str = "yNk2Hc8pWq4mR7vT3sLb9XeJ6uZa5Df1Gh";

/// The open app on `preset` in a window of `size`.
fn app_at(preset: &str, lang: &str, size: egui::Vec2) -> (MutexGuard<'static, ()>, Kit) {
    let turn = ONE_APP.lock().unwrap_or_else(|e| e.into_inner());
    let args: Vec<String> = ["--screen", preset, "--lang", lang]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let launch = Launch::from_args(&args);
    let mut kit = Harness::builder()
        .with_size(size)
        .build_eframe(|cc| App::with_art(cc, launch, evolite_core::ArtOptions::off()));
    wait_for(&mut kit, "first snapshot", |kit| {
        kit.state().snapshot.is_some()
    });
    kit.run_steps(6);
    (turn, kit)
}

/// The label that is the recipient's address, as written: no spaces, every character.
fn address_label<'k>(kit: &'k Kit) -> egui_kittest::Node<'k> {
    kit.query_all_by(|n| {
        n.value().as_deref() == Some(LOOKALIKE) || n.label().as_deref() == Some(LOOKALIKE)
    })
    .next()
    .expect("the recipient's address, solid")
}

/// AC-D1, AC-D2: the recipient's address is one solid string.
#[test]
fn the_recipients_address_is_solid() {
    let (_turn, kit) = demo_app("send-lookalike", "en");
    let _ = address_label(&kit);
    assert!(
        !has_words(&kit, "yNk2 Hc8p"),
        "no groups of four anywhere in the last look"
    );
    // The designer's review: «to address …» does not say the recipient's address twice.
    assert!(!has_words(&kit, "to address"));
}

/// AC-T «not by colour alone»: the warning says both addresses whole and where they differ.
#[test]
fn a_lookalike_is_said_in_words() {
    let (_turn, kit) = demo_app("send-lookalike", "en");
    // A label's words are its value (egui).
    let said = kit
        .query_all_by(|n| {
            n.role() == Role::Label
                && n.value()
                    .is_some_and(|v| v.starts_with("Careful: this address only looks like"))
        })
        .next()
        .expect("the look-alike warning")
        .value()
        .unwrap_or_default();
    assert!(said.contains(KNOWN), "{said}");
    assert!(said.contains(LOOKALIKE), "{said}");
    assert!(said.ends_with("9–11"), "{said}");
}

/// r1 Should 1: in Arabic a screen reader gets the warning in reading order — the catalog's
/// translation with the earlier address in it, not the text as drawn right to left.
#[test]
fn a_lookalike_is_said_in_reading_order_in_arabic() {
    const SENTENCE: &str = "Careful: this address only looks like {} that you paid or saved before — the middle is different. Compare it character by character.";
    let ar: serde_json::Value =
        serde_json::from_str(include_str!("../../../locales/ar.json")).expect("the ar catalog");
    let expected = ar[SENTENCE]
        .as_str()
        .expect("translated")
        .replacen("{}", KNOWN, 1);
    let (_turn, kit) = demo_app("send-lookalike", "ar");
    let said = kit
        .query_all_by(|n| {
            n.role() == Role::Label && n.value().is_some_and(|v| v.contains(LOOKALIKE))
        })
        .find_map(|n| n.value().filter(|v| v.contains(KNOWN)))
        .expect("the look-alike warning");
    assert!(
        said.starts_with(&expected),
        "said: {said}\nexpected to start with: {expected}"
    );
}

/// AC-T «no character lost»: at 800×600 and in Arabic the address is whole and inside the
/// window; it is not mirrored (the same characters in the same order).
#[test]
fn the_address_stays_whole_in_a_narrow_window_and_in_arabic() {
    for lang in ["en", "ar"] {
        let size = egui::vec2(800.0, 600.0);
        let (_turn, kit) = app_at("send-lookalike", lang, size);
        let node = address_label(&kit);
        let rect = node.rect();
        assert!(
            rect.left() >= 0.0 && rect.right() <= size.x,
            "{lang}: {rect:?} outside the window"
        );
    }
}
