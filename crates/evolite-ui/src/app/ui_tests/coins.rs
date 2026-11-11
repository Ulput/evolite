//! FEAT-0013: «Coins» on Send and its dialog, on the demo core's coins
//! (`evolite_demo::demo::demo_coins`: 9 coins, one dust, one not confirmed, one held by a
//! payment on its way, one from @alice).

use super::*;

fn shows(kit: &Kit, text: &str) -> bool {
    has_words(kit, text)
}

/// The demo's answer to the preset's `ListCoins` is in.
fn coins_in(kit: &mut Kit) {
    wait_for(kit, "the coins", |k| k.state().send.coins.coins.is_some());
    kit.run_steps(2);
}

/// F1, R-1, R-2: from Core the row says «automatic»; from another source there is none.
#[test]
fn the_coins_row_is_for_core_only_and_automatic_by_default() {
    let (_turn, mut kit) = demo_app("send-coins", "en");
    kit.run_steps(3);
    assert!(shows(&kit, "Coins: automatic"));
    kit.state_mut().send.source = 1;
    kit.run_steps(2);
    assert!(!shows(&kit, "Coins: automatic"), "credits: no row");
}

/// F3, R-1, AC-A6: to an @name the row is off, with the reason; a choice made goes (R-9).
#[test]
fn a_payment_to_a_name_lets_the_wallet_choose() {
    let (_turn, mut kit) = demo_app("send-coins-name", "en");
    kit.state_mut().send.coins.chosen = ["x:0".to_owned()].into();
    kit.run_steps(3);
    assert!(shows(
        &kit,
        "The wallet chooses the coins for a payment to a contact."
    ));
    assert!(kit.state().send.coins.chosen.is_empty());
}

/// CC1, R-13: ticked coins and their sum; dust hidden and counted; «Apply» takes the ticks.
#[test]
fn ticked_coins_are_applied_and_dust_is_hidden() {
    let (_turn, mut kit) = demo_app("coins-manual", "en");
    coins_in(&mut kit);
    assert!(shows(&kit, "Chosen 2 of 9"));
    assert!(shows(&kit, "1 hidden"), "the 0.0005 coin");
    button(&kit, "Apply").click();
    kit.run_steps(3);
    let send = &kit.state().send;
    assert_eq!(send.coins.chosen.len(), 2);
    assert!(!send.coins.dialog);
}

/// CC3, R-5: too little for the amount and the fee — how much is short, «Apply» off.
#[test]
fn a_set_too_small_says_how_much_is_short() {
    let (_turn, mut kit) = demo_app("coins-not-enough", "en");
    coins_in(&mut kit);
    assert!(shows(&kit, "short"));
    assert!(shows(&kit, "Tick more coins or lower the amount."));
    assert!(button(&kit, "Apply").accesskit_node().is_disabled());
}

/// CC4, R-10: a ticked coin no longer spendable is said, and «Apply» waits.
#[test]
fn a_ticked_coin_gone_is_said_and_apply_waits() {
    let (_turn, mut kit) = demo_app("coins-spent", "en");
    coins_in(&mut kit);
    // Named by its address, as the rows are; the row is marked and shown first (the designer).
    assert!(shows(
        &kit,
        "The coin yMrD…wvXQ is already spent, perhaps by a payment from another device"
    ));
    assert!(shows(&kit, "already spent"));
    assert!(button(&kit, "Apply").accesskit_node().is_disabled());
}

/// The designer's review: the address count by the language's plural, the payment only with an
/// amount; the linked-address lines in the language on screen.
#[test]
fn the_counts_and_the_notes_are_in_the_language_on_screen() {
    let (_turn, mut kit) = demo_app("coins-manual", "de");
    coins_in(&mut kit);
    assert!(shows(&kit, "2 Adressen · Zahlung"));
    kit.state_mut().send.coins.ticked = [format!("{}:1", "04".repeat(32))].into();
    kit.state_mut().send.amount.clear();
    kit.run_steps(3);
    assert!(shows(&kit, "1 Adresse"));
    assert!(!shows(&kit, "Adresse · Zahlung"), "no amount, no payment");
    drop(kit);
    drop(_turn);
    let (_turn, mut kit) = demo_app("send-coins-confirm", "ru");
    coins_in(&mut kit);
    assert!(shows(&kit, "Эти 2 адреса будут связаны в сети"));
    assert!(shows(&kit, "Одна монета пришла от @alice"));
}

/// P1, R-8: the last look says «k of n», the coins taken and not, and the linked addresses.
#[test]
fn the_last_look_says_which_coins_go() {
    let (_turn, mut kit) = demo_app("send-coins-confirm", "en");
    coins_in(&mut kit);
    // 0.12 DASH from 0.1 (@alice), 0.0405 and 0.0005: the two largest.
    assert!(shows(&kit, "Core · 2 of 3 coins"));
    assert!(shows(&kit, "Inputs: 2 of 3"));
    assert!(shows(&kit, "not needed, stays in the wallet"));
    assert!(shows(
        &kit,
        "These 2 addresses will be linked on the network"
    ));
    assert!(shows(&kit, "One coin came from @alice"));
}

/// R-7: «Max» with chosen coins is their sum less the fee of spending them all.
#[test]
fn max_spends_the_chosen_coins_less_their_fee() {
    let (_turn, mut kit) = demo_app("send-coins-chosen", "en");
    coins_in(&mut kit);
    button(&kit, "Max").click();
    kit.run_steps(3);
    let fees = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.fees.clone())
        .unwrap();
    let max = 10_000_000 + 4_050_000 + 50_000 - fees.core_fee_for_inputs(3);
    assert_eq!(
        crate::format::parse_amount(&kit.state().send.amount),
        Some(max)
    );
}

/// AC-D7: «Hide amounts» hides the coins' amounts in the list too.
#[test]
fn hidden_amounts_hide_the_coins() {
    let (_turn, mut kit) = demo_app("coins-manual", "en");
    kit.state_mut().settings.turn_on("hide-amounts");
    coins_in(&mut kit);
    assert!(shows(&kit, "•••• DASH"));
    assert!(!shows(&kit, "5.0000 DASH"));
}
