//! FEAT-0048 step 2: fully mixed coins to Platform on the screens — a new identity (path M: A9,
//! A10, A11, И6), «Shield» (UC-8: A16, A18) and the identity's top-up (UC-7: A12, A13). The demo
//! answers «not enough» to every plan; the plans are put in as the core would give them.

use super::*;
use evolite_api::{Duffs, MixedLockQuote, MixedTo};

fn quote(amount: Duffs, nominal: Duffs, confirm: bool, exactly: Option<Duffs>) -> MixedLockQuote {
    MixedLockQuote {
        amount,
        nominal,
        exact: nominal == amount,
        excess: nominal - amount,
        confirm,
        coins: 3,
        fee: 2_260,
        exactly,
    }
}

/// The checkbox whose words start with `words`.
fn tick<'k>(kit: &'k Kit, words: &str) -> egui_kittest::Node<'k> {
    kit.query_all_by_role(Role::CheckBox)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with(words))
        })
        .expect("the tick")
}

/// A11, A9 (И8): the pool short and mixed coins enough → from mixed coins at once, no «First put
/// DASH into the pool»; the exact set, «exactly, no excess»; «Sign and create» sends
/// `CreateIdentityFromMixed` (the demo's own refusal of it comes back).
#[test]
fn the_pool_short_mixed_coins_pay_the_identity_at_once() {
    let (_turn, mut kit) = demo_app("create-identity-pool-short", "en");
    kit.state_mut().create_identity.pool.mixed.set(
        MixedTo::Identity,
        3_000_000,
        Ok(quote(3_000_000, 3_000_000, false, None)),
    );
    settle(&mut kit);
    assert!(
        has_text(&kit, "2 · From mixed coins"),
        "mixed coins by default"
    );
    assert!(has_words(&kit, "exactly, no excess"));
    assert!(has_words(
        &kit,
        "Private: the identity is paid with mixed coins"
    ));
    assert!(
        !has_words(&kit, "First put DASH into the pool"),
        "И8: no step into the pool"
    );
    button(&kit, "Create privately").click();
    settle(&mut kit);
    if kit.query_all_by_role(Role::PasswordInput).next().is_some() {
        kit.query_all_by_role(Role::PasswordInput)
            .next()
            .unwrap()
            .type_text("the demo takes any other");
        kit.run_steps(2);
    }
    assert!(has_words(&kit, "From mixed coins"), "the review's plan");
    button(&kit, "Sign and create").click();
    wait_for(&mut kit, "the demo's answer", |kit| {
        kit.state().create_identity.result.is_some()
    });
    settle(&mut kit);
    assert!(
        has_words(&kit, "there are no fully mixed coins"),
        "CreateIdentityFromMixed went: the demo's refusal of it"
    );
}

/// И6: the pool holds enough → the pool first; «From mixed coins» the second choice, a press away.
#[test]
fn with_enough_in_the_pool_it_comes_first_and_mixed_coins_second() {
    let (_turn, mut kit) = demo_app("create-identity-pool", "en");
    kit.state_mut().create_identity.pool.mixed.set(
        MixedTo::Identity,
        3_000_000,
        Ok(quote(3_000_000, 3_000_000, false, None)),
    );
    settle(&mut kit);
    assert!(has_text(&kit, "2 · From the pool"));
    button(&kit, "From mixed coins").click();
    settle(&mut kit);
    assert!(has_text(&kit, "2 · From mixed coins"));
    button(&kit, "From the pool").click();
    settle(&mut kit);
    assert!(has_text(&kit, "2 · From the pool"));
}

/// A10 (И7): no exact set — the excess goes to the identity's credits, and above 0.01 DASH
/// «Create privately» waits for its tick.
#[test]
fn an_excess_over_a_hundredth_waits_for_its_yes() {
    let (_turn, mut kit) = demo_app("create-identity-pool-short", "en");
    kit.state_mut().create_identity.pool.mixed.set(
        MixedTo::Identity,
        3_000_000,
        Ok(quote(3_000_000, 10_000_000, true, None)),
    );
    settle(&mut kit);
    let words = "Another 0.0700 DASH goes to the identity's credits";
    assert!(has_words(&kit, words));
    let ready = |kit: &Kit| {
        kit.state()
            .create_identity
            .pool
            .mixed
            .ready(MixedTo::Identity, 3_000_000)
    };
    assert!(!ready(&kit), "not without the yes");
    assert!(
        button(&kit, "Create privately")
            .accesskit_node()
            .is_disabled(),
        "«Create privately» waits"
    );
    tick(&kit, words).click();
    settle(&mut kit);
    assert!(ready(&kit), "the yes given");
    assert_eq!(
        kit.state()
            .create_identity
            .pool
            .mixed
            .accept_excess(MixedTo::Identity, 3_000_000),
        7_000_000,
        "the spend carries that very excess"
    );
}

/// UC-8, A16, A18: «Shield» takes «From mixed coins»; an amount the coins cannot make exactly
/// offers «Exactly from denominations: {}» and «Take»; what more goes into the pool is said.
#[test]
fn shield_takes_mixed_coins_and_offers_the_exact_amount() {
    let (_turn, mut kit) = demo_app("shield-core", "en");
    settle(&mut kit);
    kit.state_mut().pool.sources_open = true;
    settle(&mut kit);
    kit.query_all_by(|n| n.label().is_some_and(|l| l.starts_with("From mixed coins")))
        .next()
        .expect("«From mixed coins» among the sources")
        .click();
    settle(&mut kit);
    assert!(kit.state().pool.from_mixed, "chosen");
    kit.state_mut().pool.amount = "0.05".into();
    kit.state_mut().pool.mixed.set(
        MixedTo::Pool,
        5_000_000,
        Ok(quote(5_000_000, 10_000_000, true, Some(3_000_000))),
    );
    settle(&mut kit);
    assert!(has_words(&kit, "Exactly from denominations: 0.0300 DASH"));
    assert!(has_words(&kit, "Another 0.0500 DASH goes into the pool"));
    button(&kit, "Take").click();
    settle(&mut kit);
    assert_eq!(
        crate::format::parse_amount(&kit.state().pool.amount),
        Some(3_000_000),
        "«Take» puts the exact amount in"
    );
}

/// UC-7, A12: the top-up's source by the order — the pool, then mixed coins, then Core — and by
/// hand.
#[test]
fn the_top_up_comes_from_the_pool_then_mixed_coins_then_core() {
    let (_turn, mut kit) = demo_app("top-up-pool", "en");
    settle(&mut kit);
    assert!(has_words(&kit, "From the pool"), "the pool first");
    show_wallet_edited(
        &mut kit,
        3,
        "the pool empty",
        |w| w.balances.shielded = 0,
        |_| true,
    );
    assert!(has_words(&kit, "From mixed coins"), "then mixed coins");
    show_wallet_edited(
        &mut kit,
        3,
        "no mixed coins either",
        |w| {
            w.balances.shielded = 0;
            w.balances.mixed = None;
        },
        |_| true,
    );
    assert!(has_words(&kit, "From the Core wallet"), "then Core");
    settle(&mut kit);
    kit.state_mut().identity.top_up.as_mut().unwrap().from =
        Some(crate::screens::identity::TopUpFrom::Core);
    settle(&mut kit);
    assert!(has_words(&kit, "From the Core wallet"), "chosen by hand");
}

/// A13: a mixed top-up of 0.05 the coins cannot make exactly: «Exactly from denominations:
/// 0.03» and «Take»; without «Take» the excess goes to the credits, with its yes.
#[test]
fn a_mixed_top_up_offers_the_exact_amount() {
    let (_turn, mut kit) = demo_app("top-up-mixed", "en");
    settle(&mut kit);
    assert!(has_words(&kit, "Exactly from denominations: 0.0300 DASH"));
    assert!(has_words(
        &kit,
        "Another 0.0500 DASH goes to the identity's credits"
    ));
    button(&kit, "Take").click();
    settle(&mut kit);
    let form = kit.state().identity.top_up.as_ref().expect("the window");
    assert_eq!(crate::format::parse_amount(&form.amount), Some(3_000_000));
}
