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
        5_000_000
            + evolite_api::address_plan::duffs_up(
                evolite_api::address_plan::AddressMove::Transfer.fee(1)
            ),
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
    // Core, «Mixed» (the demo has mixed coins, FEAT-0030), the credits.
    assert_eq!(kit.state().send.source, 2, "from the credits");
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
    // Core, «Mixed», credits, the pool, the Platform address (FEAT-0030).
    assert_eq!(kit.state().send.source, 4);
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

/// QA pass 21 (2): to the wallet's own Platform address the line says so — not «Someone else's
/// address». Typed in capitals too, as bech32m allows (QA21 r1 P3 1).
#[test]
fn a_transfer_to_the_wallets_own_platform_address_says_it_is_ours() {
    let (_turn, mut kit) = demo_app("send-from-address", "en");
    let own = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.platform_address.clone())
        .expect("the demo's Platform address");
    for typed in [own.clone(), own.to_ascii_uppercase()] {
        kit.state_mut().send.recipient = typed.clone();
        kit.run_steps(3);
        wait_for(&mut kit, "the line", |kit| {
            has_words(kit, "Transfer to a Platform address")
        });
        assert!(!has_words(&kit, "Someone else's address"), "{typed}");
        assert!(has_words(&kit, "this wallet's address"), "{typed}");
    }
}

/// BUG-143, then FEAT-0015 stage 2 (frame 14): from Core, the wallet's own Platform address is
/// not «Someone else's» — it is a top-up of that address: «Check and top up», which opens the
/// funding's review (22b) with the amount. Typed in capitals too (QA21 r1 P3 1).
#[test]
fn from_core_the_wallets_own_platform_address_is_not_someone_elses() {
    let (_turn, mut kit) = demo_app("core-to-platform-address", "en");
    // Tall enough for the plate, the fee line and «Check and top up» together.
    kit.set_size(egui::vec2(1024.0, 1100.0));
    settle(&mut kit);
    let own = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.platform_address.clone())
        .expect("the demo's Platform address");
    for typed in [own.clone(), own.to_ascii_uppercase()] {
        kit.state_mut().send.recipient = typed.clone();
        kit.state_mut().send.amount = crate::format::input_text(10_000_000);
        kit.run_steps(3);
        assert_eq!(kit.state().send.source, 0, "from Core");
        assert!(!has_words(&kit, "Someone else's"), "{typed}");
        assert!(
            !has_words(&kit, "you can't send to it from Core directly"),
            "{typed}: no refusal"
        );
        wait_for(&mut kit, "«Check and top up»", |kit| {
            kit.query_all_by_role_and_label(Role::Button, "Check and top up")
                .next()
                .is_some_and(|b| !b.accesskit_node().is_disabled())
        });
    }
    button(&kit, "Check and top up").click();
    wait_for(&mut kit, "the funding's review", |kit| {
        kit.state()
            .fund_address
            .form
            .as_ref()
            .is_some_and(|f| f.review)
    });
    // Drawn from the frame after the click (`App`).
    kit.run_steps(3);
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Sign and top up")
            .next()
            .is_some(),
        "22b over Send"
    );
}

/// BUG-141, QA pass 23, review r1 should 1: to the demo's own Receive address — itself funded,
/// as in QA's wallet — from the Platform address. More than the other address holds is «More
/// than there is in this balance» before the password, and the fee row says one input's fee,
/// not the recipient's balance; «Max» gives all the other address holds less the plan's fee.
#[test]
fn to_its_own_funded_address_the_fee_is_the_plans_and_max_is_the_others() {
    use evolite_api::address_plan::{AddressMove, duffs_up};
    let (_turn, mut kit) = demo_app("send-from-address", "en");
    settle(&mut kit);
    let (own, other) = {
        let w = kit
            .state()
            .snapshot
            .as_ref()
            .unwrap()
            .wallet
            .as_ref()
            .unwrap();
        assert_eq!(
            w.platform_input_addresses[0], w.platform_address,
            "Receive's is funded"
        );
        (
            w.platform_address.clone(),
            (w.platform_inputs[1] / 1_000) as i64,
        )
    };
    let one_input = format!(
        "≈ {}",
        crate::format::amount(duffs_up(AddressMove::Transfer.fee(1)))
    );
    kit.state_mut().send.recipient = own;
    kit.state_mut().send.amount = crate::format::input_text(other + 1_000_000);
    kit.state_mut().send.max = false;
    wait_for(&mut kit, "«more than there is»", |kit| {
        has_words(kit, "More than there is in this balance")
    });
    assert!(
        has_words(&kit, &one_input),
        "the fee row says one input's fee {one_input}"
    );
    assert!(
        !kit.query_all_by_role_and_label(Role::Button, "Check and send")
            .next()
            .is_some_and(|b| !b.accesskit_node().is_disabled()),
        "nothing to sign"
    );
    button(&kit, "Max").click();
    wait_for(&mut kit, "«Max»", |kit| kit.state().send.max);
    kit.run_steps(3);
    let max = other - duffs_up(AddressMove::Transfer.fee(1));
    assert_eq!(
        kit.state().send.amount,
        crate::format::input_text(max),
        "all the other address holds less the plan's fee"
    );
    assert!(has_words(&kit, &one_input), "the fee row in «Max»");
    assert!(!has_words(&kit, "More than there is in this balance"));
}
