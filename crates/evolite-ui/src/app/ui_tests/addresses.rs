//! FEAT-0015 stage 1: paying from the Platform addresses, on the demo core — the fourth source,
//! what the recipient makes of it, what it refuses before the last look, and a payment through.

use super::*;
use evolite_api::ActivityKind;

/// Whether «Check and send» takes a click.
fn ready(kit: &Kit) -> bool {
    !button(kit, "Check and send").accesskit_node().is_disabled()
}

/// AC-D1, AC-T8 (R-10): «Platform address» is a source of its own, says it is public, and a
/// transfer to a Platform address says what it is; its fee is an estimate («≈», BUG-108).
#[test]
fn the_platform_address_is_a_source_that_says_it_is_public() {
    let (_turn, mut kit) = demo_app("send-from-address", "en");
    wait_for(&mut kit, "the form", |kit| {
        has_words(kit, "visible to everyone")
    });
    assert!(has_words(&kit, "Transfer to a Platform address"));
    assert!(has_words(&kit, "Platform fee: ≈"), "the fee is an estimate");
    assert!(ready(&kit), "a transfer can go");
}

/// AC-T1 (R-3): to someone else's Core address, refused before the last look, with the way
/// out; the button stays grey.
#[test]
fn a_foreign_core_address_is_refused_before_the_last_look() {
    let (_turn, mut kit) = demo_app("address-withdraw-foreign", "en");
    wait_for(&mut kit, "the refusal", |kit| {
        has_words(kit, "only to this wallet's addresses")
    });
    assert!(has_words(&kit, "choose Core as the source"));
    assert!(!ready(&kit));
}

/// UC-4 (analyst-47): a withdrawal whose fee is above a tenth of the amount warns, and says
/// how much; it may still go.
#[test]
fn a_withdrawal_that_costs_much_of_its_amount_warns() {
    let (_turn, mut kit) = demo_app("address-withdraw-warning", "en");
    wait_for(&mut kit, "the warning", |kit| {
        has_words(kit, "A withdrawal to Core costs")
    });
    assert!(has_words(&kit, "20%"), "0.004 of 0.02");
    assert!(has_words(&kit, "Withdrawal fee: ≈"));
    assert!(ready(&kit));
}

/// A payment from the addresses goes through on the demo core: the last look says it is
/// public, «Sign and send» pays, the history has the line and the addresses hold less.
#[test]
fn a_payment_from_the_addresses_goes_through() {
    let (_turn, mut kit) = demo_app("address-confirm", "en");
    let sign = crate::i18n::spoken("Sign and send");
    wait_for(&mut kit, "the last look", |kit| {
        kit.query_by_label(&sign).is_some()
    });
    assert!(has_words(&kit, "visible to everyone on Platform"));
    let held = |kit: &Kit| {
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .map_or(0, |w| w.balances.platform_address)
    };
    let before = held(&kit);
    kit.get_by_label(&sign).click();
    wait_for(&mut kit, "the line in the history", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .is_some_and(|w| matches!(w.activity[0].kind, ActivityKind::AddressSent { .. }))
    });
    assert_eq!(
        before - held(&kit),
        5_000_000 + 6_500,
        "the amount and the fee"
    );
}

/// AC-T9, AC-D15 (frames 06, 14a): with nothing to spend on the Platform address, its card is
/// no source; Tab reaches it, and Enter fills the address instead: from the credits, to the
/// wallet's own Platform address.
#[test]
fn an_empty_platform_address_card_fills_the_address() {
    let (_turn, mut kit) = demo_app("send", "en");
    let empty = |kit: &mut Kit| {
        if let Some(w) = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
        {
            w.platform_inputs = vec![1_000];
            w.balances.platform_address = 1;
        }
    };
    empty(&mut kit);
    kit.run_steps(2);
    let own = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.platform_address.clone())
        .unwrap_or_default();
    tab_to(&mut kit, "Platform address");
    empty(&mut kit);
    enter(&mut kit);
    assert_eq!(kit.state().send.recipient, own, "its own address in «To»");
    assert_eq!(kit.state().send.source, 1, "from the credits");
}

/// AC-D14 (frame 15): from Core to a Platform address there is no direct way; the form says
/// so, keeps the button grey and offers the credits or the Platform address, the recipient
/// kept.
#[test]
fn core_to_a_platform_address_offers_the_ways_there() {
    let (_turn, mut kit) = demo_app("core-to-platform-address", "en");
    wait_for(&mut kit, "the plate", |kit| {
        has_words(kit, "you can't send to it from Core directly")
    });
    assert!(!ready(&kit));
    let to = kit.state().send.recipient.clone();
    button(&kit, "Send from the Platform address ›").click();
    kit.run_steps(3);
    assert_eq!(kit.state().send.source, 3);
    assert_eq!(kit.state().send.recipient, to);
}

/// R-1а (the designer's strings of 2026-11-10): from the Platform address, a name that is not
/// one of the wallet's identities is said once under the recipient; nothing to pay, the reason
/// under the button.
#[test]
fn a_foreign_name_from_the_platform_address_says_what_to_enter() {
    let (_turn, mut kit) = demo_app("address-top-up-foreign", "en");
    wait_for(&mut kit, "the notice", |kit| {
        has_words(kit, "you can only top up your own identity")
    });
    assert!(has_words(
        &kit,
        "Enter your own identity or a Platform address"
    ));
    assert!(!has_words(&kit, "the owner's identity gets the credits"));
    assert!(has_words(&kit, "Platform fee: —"));
    assert!(!ready(&kit));
}
