//! FEAT-0048: an identity created privately, from the pool — the default on both levels (R-1):
//! nothing moves on Core (A1), the lines on what is private before the password (A8), the first
//! step into the pool when it holds too little (A2), Core only on the advanced level and under
//! its warning (A5), and for everyone when the pool is unavailable (A6), the plates of a creation
//! sent and not known yet or refused (UC-3, UC-4), and the invitation said as not private.

use super::*;
use crate::screens::Screen;

fn balances(kit: &Kit) -> (i64, i64, usize) {
    let w = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .expect("a wallet");
    (w.balances.core, w.balances.shielded, w.identities.len())
}

/// A1, A8: «Create privately» → the review says what is private and what is seen before the
/// password → «Sign and create»: an identity more, the exit (0.03) gone from the pool, Core as it
/// was.
#[test]
fn an_identity_is_made_from_the_pool_and_nothing_moves_on_core() {
    let (_turn, mut kit) = demo_app("create-identity-pool", "en");
    settle(&mut kit);
    let (core, pool, identities) = balances(&kit);
    assert!(
        has_text(&kit, "2 · From the pool"),
        "the pool path by default"
    );
    button(&kit, "Create privately").click();
    kit.run_steps(4);
    assert!(
        has_words(
            &kit,
            "Private: nobody can link this identity to your Core coins"
        ),
        "A8: the private line in the review"
    );
    assert!(has_words(
        &kit,
        "Your name, contacts and profile are visible to everyone"
    ));
    assert!(!has_words(&kit, "anonym"), "R-5: never «anonymous»");
    button(&kit, "Sign and create").click();
    wait_for(&mut kit, "the identity from the pool", |kit| {
        balances(kit).2 > identities
    });
    let (core_after, pool_after, _) = balances(&kit);
    assert_eq!(core_after, core, "A1: nothing moved on Core");
    assert_eq!(pool - pool_after, 3_000_000, "the 0.03 exit left the pool");
}

/// R-2 (И5): 0.03 for an ordinary name or none, 0.25 for a contested one; the nearest the
/// network offers when it does not offer that one.
#[test]
fn the_exit_is_0_03_or_0_25_for_a_contested_name() {
    let mut fees = crate::screens::shielded::tests::fees();
    fees.pool_identity_exits = vec![3_000_000, 10_000_000, 25_000_000, 50_000_000];
    let exit = crate::screens::create_identity::exit_for;
    assert_eq!(exit(&fees, false), 3_000_000);
    assert_eq!(exit(&fees, true), 25_000_000);
    fees.pool_identity_exits = vec![10_000_000, 50_000_000];
    assert_eq!(exit(&fees, false), 10_000_000, "the nearest above");
    assert_eq!(exit(&fees, true), 50_000_000);
}

/// A2 (UC-2, R-6): too little in the pool — the first step puts 0.1 into it, with the pool's own
/// «Shield» window over this screen; «Create privately» stays off until it is in.
#[test]
fn too_little_in_the_pool_puts_0_1_in_first() {
    let (_turn, mut kit) = demo_app("create-identity-pool-short", "en");
    settle(&mut kit);
    assert!(has_text(&kit, "First put DASH into the pool"));
    assert!(has_words(
        &kit,
        "It is visible that you put 0.1000 DASH into the pool."
    ));
    let create = kit
        .query_all_by_role_and_label(Role::Button, "Create privately")
        .next()
        .expect("«Create privately»");
    assert!(create.accesskit_node().is_disabled(), "off while short");
    button(&kit, "Put 0.1000 DASH into the pool").click();
    kit.run_steps(4);
    let pool = &kit.state().pool;
    assert!(pool.shielding, "the «Shield» window is up");
    assert_eq!(pool.from, 2, "from Core");
    assert_eq!(crate::format::parse_amount(&pool.amount), Some(10_000_000));
    assert_eq!(kit.state().screen, Screen::CreateIdentity, "still here");
}

/// BUG-315 (R-6, r1 Must): what «First put DASH into the pool» offers — 0.1 when Core can send it
/// and it covers the shortfall, all Core can send when that still covers it, and Core short too
/// said with how much more; a Core not checked yet is not taken for short. The model of the
/// «Shield» window: Core spends the amount + its payment, the pool gets the amount − the fee.
#[test]
fn into_the_pool_never_offers_what_core_does_not_have() {
    use crate::screens::create_identity::pool::{Core, CoreShort, INTO_POOL, into_pool};
    let core = |has| Core::Known {
        has,
        core_payment: 1_000,
        shield_fee: 100_000,
    };
    let short = |has, more| Err(CoreShort { has, more });
    assert_eq!(into_pool(2_000_000, core(50_000_000)), Ok(INTO_POOL));
    assert_eq!(
        into_pool(2_000_000, core(INTO_POOL + 1_000)),
        Ok(INTO_POOL),
        "0.1 and Core's own payment"
    );
    assert_eq!(
        into_pool(2_000_000, core(INTO_POOL + 999)),
        Ok(INTO_POOL - 1),
        "one duff short of 0.1: all Core can send"
    );
    assert_eq!(
        into_pool(9_950_000, core(50_000_000)),
        Ok(49_999_000),
        "0.1 less the fee does not cover the shortfall: more than 0.1"
    );
    assert_eq!(
        into_pool(2_000_000, core(5_000_000)),
        Ok(4_999_000),
        "all Core can send, less its own payment"
    );
    // Exactly enough: what arrives (2 100 000 − 100 000) is the shortfall.
    assert_eq!(into_pool(2_000_000, core(2_101_000)), Ok(2_100_000));
    assert_eq!(
        into_pool(2_000_000, core(2_100_999)),
        short(2_100_999, 1),
        "one duff less: short by one"
    );
    // r1 Must: the edge the fee on top did not see — what would go in covers the shortfall,
    // what arrives (less the fee) does not.
    assert_eq!(into_pool(2_000_500, core(2_101_000)), short(2_101_000, 500));
    assert_eq!(
        into_pool(2_000_000, core(1_600_000)),
        short(1_600_000, 501_000),
        "short by, the fee and the payment counted"
    );
    assert_eq!(
        into_pool(2_000_000, core(0)),
        short(0, 2_101_000),
        "nothing in Core: the shortfall, the fee and the payment"
    );
    assert_eq!(
        into_pool(2_000_000, Core::NotKnown),
        Ok(INTO_POOL),
        "not known: not short"
    );
}

/// The wallet's Core as checked in this sync, `core` in it.
fn core_of(core: evolite_api::Duffs) -> impl Fn(&mut evolite_api::Wallet) {
    move |w| {
        w.balances.core = core;
        w.balances.core_as_of = None;
    }
}

/// BUG-315 (QA: 0.02 in Core, «Put 0.1000 DASH into the pool» offered): Core short too — said,
/// with how much more, no «Put … into the pool», and «Receive» opens Receive.
#[test]
fn core_short_too_says_how_much_more_and_offers_receive() {
    let (_turn, mut kit) = demo_app("create-identity-pool-short", "en");
    settle(&mut kit);
    show_wallet_edited(&mut kit, 2, "0.01 in Core", core_of(1_000_000), |_| true);
    assert!(has_words(&kit, "Core has only "), "Core's shortfall said");
    assert!(has_words(&kit, "more is needed. Receive DASH to Core"));
    // r1 Must: «… more» is what Core must still receive — the pool's shortfall, the fee taken
    // out of the amount on its way and Core's own payment, less what Core has.
    let w = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.clone())
        .expect("a wallet");
    let exit = crate::screens::create_identity::pool::exit_for(&w.fees, false);
    let short = exit + w.fees.pool_identity_create - w.balances.shielded;
    let more = short + w.fees.shield_from_core + w.fees.core_payment - 1_000_000;
    assert!(
        has_words(
            &kit,
            &format!("{} more is needed", crate::format::amount(more))
        ),
        "«{} more»",
        crate::format::amount(more)
    );
    assert!(
        !has_words(&kit, "It is visible that you put"),
        "nothing to put in"
    );
    assert!(
        kit.query_all_by_role(Role::Button).all(|b| !b
            .accesskit_node()
            .label()
            .unwrap_or_default()
            .starts_with("Put ")),
        "no «Put … into the pool»"
    );
    // The plate's «Receive», not the menu's or the top bar's (which open Receive too): the
    // lowest one on the page.
    kit.query_all_by_role_and_label(Role::Button, "Receive")
        .filter(|b| b.accesskit_node().bounding_box().is_some())
        .max_by(|a, b| {
            let y = |n: &egui_kittest::Node<'_>| {
                n.accesskit_node().bounding_box().map_or(0.0, |r| r.y0)
            };
            y(a).total_cmp(&y(b))
        })
        .expect("«Receive» on the plate")
        .click();
    show_wallet_edited(&mut kit, 2, "the click", core_of(1_000_000), |_| true);
    assert_eq!(kit.state().screen, Screen::Receive);
}

/// BUG-315: Core has less than 0.1 but enough — all it has goes in, never more.
#[test]
fn core_with_less_than_0_1_puts_in_what_it_has() {
    let (_turn, mut kit) = demo_app("create-identity-pool-short", "en");
    settle(&mut kit);
    show_wallet_edited(&mut kit, 2, "0.06 in Core", core_of(6_000_000), |_| true);
    // r1 Must: all Core can send — what it has less its own payment; the fee comes out on the way.
    let payment = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.fees.core_payment)
        .expect("a wallet");
    let put = crate::format::balance(6_000_000 - payment);
    assert!(!has_words(&kit, "Core has only "), "Core is enough");
    button(&kit, &format!("Put {put} into the pool")).click();
    show_wallet_edited(&mut kit, 4, "the click", core_of(6_000_000), |_| true);
    assert_eq!(
        crate::format::parse_amount(&kit.state().pool.amount),
        Some(6_000_000 - payment)
    );
}

/// R-1, A5: from Core is not offered on the ordinary level; on the advanced one it is a choice
/// of its own, its warning over the Core form, and the way back.
#[test]
fn from_core_only_on_the_advanced_level_and_under_its_warning() {
    {
        let (_turn, mut kit) = demo_app("create-identity-pool", "en");
        settle(&mut kit);
        assert!(!has_text(&kit, "Create from Core (not private)"));
    }
    let (_turn, mut kit) = demo_app("create-identity-core", "en");
    settle(&mut kit);
    assert!(has_words(
        &kit,
        "This identity will be linked to your Core coins for good"
    ));
    assert!(has_text(&kit, "2 · Top-up"), "the Core form under it");
    button(&kit, "Create privately instead").click();
    kit.run_steps(4);
    assert!(has_text(&kit, "2 · From the pool"));
    assert!(has_text(&kit, "Create from Core (not private)"));
}

/// A6 (UC-6): the pool unavailable — the Core form for everyone, under the warning, with when
/// private creation comes back.
#[test]
fn the_pool_unavailable_falls_back_to_core_with_the_warning() {
    let (_turn, mut kit) = demo_app("create-identity-pool-off", "en");
    settle(&mut kit);
    assert!(has_words(
        &kit,
        "This identity will be linked to your Core coins for good"
    ));
    assert!(has_text(
        &kit,
        "Private creation becomes available once the pool works."
    ));
    assert!(has_text(&kit, "2 · Top-up"));
}

/// BUG-245 (option B): the first pool check said, minutes long; nothing to create meanwhile.
#[test]
fn the_first_pool_check_is_said_with_its_minutes() {
    let (_turn, mut kit) = demo_app("create-identity-pool-connecting", "en");
    settle(&mut kit);
    assert!(has_words(
        &kit,
        "The first pool check can take up to ~10 minutes"
    ));
    let create = kit
        .query_all_by_role_and_label(Role::Button, "Create privately")
        .next()
        .expect("«Create privately»");
    assert!(create.accesskit_node().is_disabled());
}

/// UC-3 (BUG-243): sent and not known yet — never «failed», nothing to send again; UC-4: refused,
/// the money and the address it went to.
#[test]
fn a_creation_sent_from_the_pool_is_waited_for_or_said_refused() {
    let (_turn, mut kit) = demo_app("create-identity-pool-checking", "en");
    settle(&mut kit);
    assert!(has_text(
        &kit,
        "The identity from the pool is being checked"
    ));
    assert!(has_words(&kit, "Nothing is sent again"));
    assert!(!has_words(&kit, "failed"));
    assert!(!has_text(&kit, "Create privately"), "nothing new meanwhile");
    drop(kit);
    drop(_turn);
    let (_turn2, mut kit) = demo_app("create-identity-pool-refused", "en");
    settle(&mut kit);
    assert!(has_words(
        &kit,
        "The identity was not created: 0.02742313 DASH went to your Platform address."
    ));
    assert!(has_words(&kit, "tdashevo1qz7mv8w5"));
}

/// UC-4, A4: «Shield again» on the refusal plate opens its window — what goes back into the
/// pool, the password, `Spend::ShieldRefund` — and once it went through, its answer and «Done»,
/// no second «Shield again» in it (BUG-274).
#[test]
fn a_refused_identitys_money_goes_back_into_the_pool_in_one_action() {
    use crate::screens::shielded::PoolMove;
    let (_turn, mut kit) = demo_app("create-identity-pool-refused", "en");
    settle(&mut kit);
    let again = |kit: &Kit| {
        kit.query_all_by_role_and_label(Role::Button, "Shield again")
            .count()
    };
    assert_eq!(again(&kit), 1, "the plate's one action");
    button(&kit, "Shield again").click();
    settle(&mut kit);
    let seen = crate::components::dialog_seen(&kit.ctx).expect("its window");
    assert_eq!(seen.name, "shield-refund");
    assert!(seen.spends, "a spend: its password and row");
    assert!(has_words(&kit, "Back into the pool"), "what goes back");
    assert!(has_words(&kit, "≈ 0.02742313 DASH"));
    assert!(has_words(&kit, "Nothing moves on Core"));
    if kit.query_all_by_role(Role::PasswordInput).next().is_some() {
        kit.query_all_by_role(Role::PasswordInput)
            .next()
            .unwrap()
            .type_text("the demo takes any other");
        kit.run_steps(2);
    }
    // The window's own button: the lowest of that name (the plate has one under it).
    kit.query_all_by_role_and_label(Role::Button, "Shield again")
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("the window's button")
        .click();
    wait_for(&mut kit, "the demo's answer", |kit| {
        kit.state().pool.result.is_some()
    });
    settle(&mut kit);
    assert!(
        has_words(&kit, "Nothing was returned"),
        "the demo's refusal said in the window"
    );
    // Gone through: the answer and «Done», no second «Shield again» in the window.
    kit.state_mut().pool.result = Some((PoolMove::Shield, Ok(())));
    settle(&mut kit);
    assert!(has_words(&kit, "The money is back in the pool."));
    assert_eq!(
        again(&kit),
        0,
        "none: the window says «Done», the plate hides its own until it goes (r1 Should 1)"
    );
    button(&kit, "Done").click();
    settle(&mut kit);
    assert!(kit.state().create_identity.pool.refund.is_none(), "closed");
    assert!(kit.state().pool.result.is_none(), "its answer forgotten");
}

/// r1: a creation from the pool proved never to reach the network: said so, and a new one may
/// start (the money is in the pool).
#[test]
fn one_that_never_reached_the_network_lets_a_new_creation_start() {
    let (_turn, mut kit) = demo_app("create-identity-pool-not-delivered", "en");
    settle(&mut kit);
    assert!(has_words(&kit, "It did not reach the network"));
    assert!(!has_text(
        &kit,
        "The identity from the pool is being checked"
    ));
    assert!(
        has_text(&kit, "2 · From the pool"),
        "the form, not the plate alone"
    );
    assert!(has_text(&kit, "Create privately"));
}

/// FEAT-0047: «I have an invitation» is a choice of its own, said as not private.
#[test]
fn an_invitation_is_said_as_not_private() {
    let (_turn, mut kit) = demo_app("create-identity-pool", "en");
    settle(&mut kit);
    button(&kit, "I have an invitation").click();
    kit.run_steps(4);
    assert!(has_words(
        &kit,
        "An identity from an invitation is not private"
    ));
    button(&kit, "Back").click();
    kit.run_steps(4);
    assert!(has_text(&kit, "2 · From the pool"));
}

/// The lead's T-1: the core's progress is a timer to 0.95 — the steps say «Building the private
/// proof» before it, «Waiting for Platform» from it, never a bar stuck at 95 %.
#[test]
fn the_steps_wait_for_platform_once_the_proof_is_sent() {
    let step = crate::screens::create_identity::pool_step;
    assert_eq!(step(false, Some(0.5)), None, "nothing before it is sent");
    assert_eq!(step(true, None), Some(1), "the proof first");
    assert_eq!(step(true, Some(0.94)), Some(1));
    assert_eq!(step(true, Some(0.95)), Some(2), "then Platform");
    let (_turn, mut kit) = demo_app("create-identity-pool", "en");
    settle(&mut kit);
    assert!(has_text(&kit, "Building the private proof"));
    assert!(has_text(&kit, "Waiting for Platform"));
    assert!(!has_words(&kit, "95"), "no percentage");
}
