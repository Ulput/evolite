//! SPV Б2 and item 3: an asset lock its coins cannot make is refused in the form, before the
//! button, in the core's own words (AC-T3 of Б2, AC-T3.3 of item 3).

use super::*;

fn enabled(kit: &Kit, name: &str) -> bool {
    !button(kit, name).accesskit_node().is_disabled()
}

/// Б2 AC-T3: a top-up of 0.011 DASH from 300 coins of 0.0001 needs more than 90 inputs: said
/// under the amount, «Top up» off.
#[test]
fn a_top_up_from_too_many_small_coins_is_refused_before_the_button() {
    let (_turn, mut kit) = demo_app("top-up-many-coins", "en");
    wait_for(&mut kit, "the refusal", |kit| {
        has_words(kit, "This amount needs more than 90 coins")
    });
    assert!(has_words(&kit, "send “Max” from Core to your own address"));
    let label = format!("Top up by {}", crate::format::amount(1_100_000));
    assert!(!enabled(&kit, &label), "no «{label}» to press");
}

/// The same in Russian, the analyst's words.
#[test]
fn the_refusal_is_said_in_the_language() {
    let (_turn, mut kit) = demo_app("top-up-many-coins", "ru");
    wait_for(&mut kit, "the refusal", |kit| {
        has_words(kit, "Для этой суммы нужно больше 90 монет")
    });
    assert!(has_words(&kit, "отправьте «Всё» с Core на свой адрес"));
}

/// Item 3 AC-T3.3: the final coins (0.01 DASH) cannot make 0.1, the others are not confirmed
/// yet: wait for a block, «Top up» off.
#[test]
fn a_top_up_from_coins_not_final_yet_says_wait_for_a_block() {
    let (_turn, mut kit) = demo_app("top-up-not-final", "en");
    wait_for(&mut kit, "the refusal", |kit| {
        has_words(kit, "not confirmed by the network yet — wait for a block")
    });
    let label = format!("Top up by {}", crate::format::amount(10_000_000));
    assert!(!enabled(&kit, &label), "no «{label}» to press");
}

/// Б2 AC-T3: the new identity's form says the same before «Check».
#[test]
fn a_new_identity_from_too_many_small_coins_is_refused_before_check() {
    let (_turn, mut kit) = demo_app("create-identity-many-coins", "en");
    wait_for(&mut kit, "the refusal", |kit| {
        has_words(kit, "This amount needs more than 90 coins")
    });
    assert!(!kit.state().create_identity.confirming);
}

/// A usual top-up (the demo's own coins, no limit known) says nothing of the coins.
#[test]
fn a_usual_top_up_says_nothing_of_the_coins() {
    let (_turn, mut kit) = demo_app("top-up", "en");
    wait_for(&mut kit, "the form", |kit| has_words(kit, "Top up by"));
    assert!(!has_words(&kit, "coins — the network will not take"));
    assert!(!has_words(&kit, "wait for a block"));
}
