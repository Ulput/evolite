//! FEAT-0040: the fee of a Core payment for advanced users — «Fee» under «Coins» in Send.

use super::*;
use evolite_api::fee_choice::FeeChoice;

const ADDRESS: &str = "yXdxCB2vAX6wPHeaQqyB6XJT9eSZGJx3qB";

/// Send with an address typed, the advanced features as `on`.
fn send_to(to: &str, on: bool) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("send", "en");
    kit.state_mut().settings.set("advanced", on);
    kit.state_mut().send.recipient = to.into();
    kit.state_mut().send.amount = "0.1".into();
    kit.run_steps(4);
    (turn, kit)
}

/// AC-A1, UC-6: off, no «Fee» row and «Normal» kept; on, the row with «Normal» chosen.
#[test]
fn the_fee_row_is_there_only_with_the_advanced_features() {
    let (_turn, kit) = send_to(ADDRESS, false);
    assert!(!has_text(&kit, "Custom"), "no row while off");
    drop(kit);
    drop(_turn);
    let (_turn, kit) = send_to(ADDRESS, true);
    assert!(has_text(&kit, "Custom"), "the row");
    assert_eq!(kit.state().settings.fee, FeeChoice::Normal);
}

/// UC-2, R-12: «Higher» chosen is the wallet's — kept in its settings, the line under it says
/// the rate.
#[test]
fn higher_is_chosen_and_kept_with_the_wallet() {
    let (_turn, mut kit) = send_to(ADDRESS, true);
    kit.query_all_by_label("Higher")
        .find(|n| n.accesskit_node().role() != Role::Label)
        .expect("«Higher»")
        .click();
    kit.run_steps(3);
    wait_for(&mut kit, "«Higher» kept", |kit| {
        kit.state().settings.fee == FeeChoice::Higher
    });
    assert!(has_words(&kit, "twice the normal"));
    let prefs = kit.state().settings.prefs(&Default::default());
    assert_eq!(prefs.fee_rate, Some(2_000));
}

/// AC-A3, R-9: «Custom» typed under the least says why and holds «Check the payment».
#[test]
fn a_custom_rate_under_the_least_holds_the_payment() {
    let (_turn, mut kit) = send_to(ADDRESS, true);
    kit.state_mut().settings.fee = FeeChoice::Custom(5_000);
    kit.state_mut().send.fee_custom = "500".into();
    kit.run_steps(4);
    assert!(has_words(&kit, "At least 1,000 duffs per kB"));
}

/// AC-A7, R-8: to a contact the row is shown, not active, and says why.
#[test]
fn a_payment_to_a_contact_keeps_the_normal_fee() {
    let (_turn, kit) = send_to("@alice", true);
    assert!(has_words(
        &kit,
        "A payment to a contact is built with the normal fee"
    ));
    assert!(!has_text(&kit, "Custom"), "no choice to a contact");
}
