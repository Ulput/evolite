//! FEAT-0046: «Broadcast again» on the card of a payment the network has not confirmed
//! (`--screen activity-card-stuck`: line 3 of the demo, until it is broadcast again).

use super::*;
use evolite_api::{Command, Proof};

fn line3(kit: &Kit) -> evolite_api::Activity {
    kit.state()
        .snapshot
        .as_ref()
        .unwrap()
        .wallet
        .as_ref()
        .unwrap()
        .activity
        .iter()
        .find(|a| a.id == 3)
        .unwrap()
        .clone()
}

fn advanced_on(kit: &mut Kit) {
    kit.state_mut().settings.set("advanced", true);
    kit.state()
        .core
        .send(Command::AdvancedFeatures { on: true });
    wait_for(kit, "the switch in the core", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.advanced_features)
    });
}

/// UC-4, AC-A9 (the interface's half): with the switch off the card says what it means, with no
/// button and no word of one.
#[test]
fn off_the_card_explains_without_the_button() {
    let (_turn, mut kit) = demo_app("activity-card-stuck", "en");
    wait_for(&mut kit, "the explanation", |kit| {
        has_words(kit, "most likely the nodes did not take it")
    });
    assert!(!has_text(&kit, "Broadcast again"));
    assert!(!has_words(&kit, "You can broadcast it again"));
    assert!(!has_words(&kit, "every 10 minutes"));
}

/// UC-1, AC-A1, AC-A6 (R-3a, R-6, R-10): on, the button is there with the words; pressed, the
/// payment waits again with the time it was broadcast, and the button waits a minute.
#[test]
fn broadcast_again_sends_and_then_waits_a_minute() {
    let (_turn, mut kit) = demo_app("activity-card-stuck", "en");
    advanced_on(&mut kit);
    wait_for(&mut kit, "the button", |kit| {
        has_text(kit, "Broadcast again")
    });
    assert!(has_words(&kit, "every 10 minutes while it is open"));
    assert!(has_words(&kit, "You can broadcast it again."));
    button(&kit, "Broadcast again").click();
    wait_for(&mut kit, "broadcast", |kit| {
        line3(kit).detail.is_some_and(|d| d.resent_at.is_some())
    });
    assert_eq!(line3(&kit).proof, Proof::Mempool, "waits again (R-8)");
    wait_for(&mut kit, "its time", |kit| {
        has_words(kit, "Broadcast again ")
    });
    // The line waits again: no button until it is not confirmed again.
    assert!(!has_text(&kit, "Broadcast again"));
}

/// R-6 (AC-A6) and R-7: not confirmed again soon after — the button waits out its minute, and
/// the words say the coins may be spent elsewhere.
#[test]
fn still_not_confirmed_after_it_says_why_and_waits() {
    let (_turn, mut kit) = demo_app("activity-card-stuck", "en");
    advanced_on(&mut kit);
    wait_for(&mut kit, "the button", |kit| {
        has_text(kit, "Broadcast again")
    });
    button(&kit, "Broadcast again").click();
    wait_for(&mut kit, "broadcast", |kit| {
        line3(kit).detail.is_some_and(|d| d.resent_at.is_some())
    });
    show_edited(
        &mut kit,
        3,
        "not confirmed again",
        |app| {
            if let Some(a) = app
                .snapshot
                .as_mut()
                .and_then(|s| s.wallet.as_mut())
                .and_then(|w| w.activity.iter_mut().find(|a| a.id == 3))
            {
                a.proof = Proof::Unconfirmed;
            }
        },
        |kit| has_words(kit, "with this phrase in another wallet"),
    );
    assert!(has_words(&kit, "You can again in"), "the minute (R-6)");
}

/// AC-A5 (R-5): a payment the core does not mark as one to broadcast again — here one coming
/// in — shows the words, never the button.
#[test]
fn only_a_payment_of_ours_has_the_button() {
    let (_turn, mut kit) = demo_app("activity-card-stuck", "en");
    advanced_on(&mut kit);
    show_edited(
        &mut kit,
        3,
        "not one of ours",
        |app| {
            if let Some(d) = app
                .snapshot
                .as_mut()
                .and_then(|s| s.wallet.as_mut())
                .and_then(|w| w.activity.iter_mut().find(|a| a.id == 3))
                .and_then(|a| a.detail.as_mut())
            {
                d.resendable = false;
            }
        },
        |kit| has_words(kit, "most likely the nodes did not take it"),
    );
    assert!(!has_text(&kit, "Broadcast again"));
}
