//! FEAT-0019 T-2: «Claim» and «Buy» on the demo core — where the buttons are, the refusals
//! before the password, a price that grew (P2), and the demo never pretending to spend.

use super::*;

fn buttons(kit: &Kit, name: &str) -> usize {
    kit.query_all_by_role_and_label(Role::Button, name).count()
}

/// AC-D1, R-15, R-21: «Claim» and «Buy» only on the row that has a proved claim and a price
/// (the demo's Evo Coffee), and no other.
#[test]
fn claim_and_buy_only_where_possible() {
    let (_turn, mut kit) = demo_app("tokens", "en");
    kit.run_steps(3);
    assert_eq!(buttons(&kit, "Claim"), 1);
    assert_eq!(buttons(&kit, "Buy"), 1);
    assert!(has_text(
        &kit,
        "A distribution interval has passed since the last claim — there may be a payout."
    ));
}

/// AC-A9, AC-T12: «Buy» opens P1; under the first tier is refused before the password with the
/// minimum; a count in a tier gives the total in credits and the tier under the field.
#[test]
fn a_purchase_under_the_minimum_is_refused_before_the_password() {
    let (_turn, mut kit) = demo_app("tokens", "en");
    kit.run_steps(3);
    button(&kit, "Buy").click();
    wait_for(&mut kit, "P1", |kit| has_text(kit, "Buy Evo Coffee"));
    if let Some(buy) = &mut kit.state_mut().tokens.buying {
        buy.count = "0.5".into();
    }
    kit.run_steps(3);
    assert!(
        has_text(&kit, "At least 1.00"),
        "the minimum, before the password"
    );
    assert!(button(&kit, "Sign and buy").accesskit_node().is_disabled());
    if let Some(buy) = &mut kit.state_mut().tokens.buying {
        buy.count = "15".into();
    }
    kit.run_steps(3);
    assert!(
        has_words(&kit, "Total: 0.0105 DASH in credits"),
        "15 × 0.0007"
    );
    assert!(
        has_text(&kit, "From 10.00 — 0.0007 DASH each"),
        "the tier it falls in"
    );
    assert!(!button(&kit, "Sign and buy").accesskit_node().is_disabled());
}

/// P2: a price that grew says the new one and the old, and that nothing was taken.
#[test]
fn a_price_that_grew_says_both_prices() {
    let (_turn, mut kit) = demo_app("tokens-buy-changed", "en");
    kit.run_steps(3);
    assert!(has_text(
        &kit,
        "The price changed: now 0.0008 DASH (was 0.0006 DASH). Nothing was taken. Check and confirm again."
    ));
}

/// K1: what a claim pays at least, how often, and the fee; the demo core never pretends to claim.
#[test]
fn a_claim_says_what_it_pays_and_the_demo_does_not_pretend() {
    let (_turn, mut kit) = demo_app("tokens", "en");
    kit.run_steps(3);
    button(&kit, "Claim").click();
    wait_for(&mut kit, "K1", |kit| {
        has_text(kit, "Claim the distribution · Evo Coffee")
    });
    assert!(has_words(&kit, "Distribution: perpetual · every 24 hours"));
    assert!(has_words(&kit, "Accrued: at least 24.00"));
    button(&kit, "Sign and claim").click();
    wait_for(&mut kit, "the demo's refusal", |kit| {
        has_words(kit, "Not claimed")
    });
}

/// K1u: a claim whose amount cannot be counted before says so.
#[test]
fn a_claim_that_cannot_be_counted_says_so() {
    let (_turn, mut kit) = demo_app("tokens-claim-unknown", "en");
    kit.run_steps(3);
    assert!(has_words(&kit, "Accrued: becomes known after the claim"));
}

/// AC-A13: «Claim» and «Buy» are reached by Tab and open with Enter.
#[test]
fn claim_and_buy_work_from_the_keyboard() {
    let (_turn, mut kit) = demo_app("tokens", "en");
    kit.run_steps(3);
    tab_to(&mut kit, "Buy");
    assert_eq!(focused(&kit).as_deref(), Some("Buy"));
    kit.key_press(egui::Key::Enter);
    wait_for(&mut kit, "P1", |kit| has_text(kit, "Buy Evo Coffee"));
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    tab_to(&mut kit, "Claim");
    assert_eq!(focused(&kit).as_deref(), Some("Claim"));
}
