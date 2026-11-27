//! BUG-318, the class (BUG-186's rule for every summary): while a write runs, its window's
//! summary shows what it was signed with, not figures reckoned again from a balance the write
//! has already moved. The pool's dialogs are in `pool_moves`; here FEAT-0057 П3's «Credits:
//! now › after» and Send's «Left».

use super::identity_pool::with_wallet;
use super::*;

fn wallet_of(kit: &Kit) -> evolite_api::Wallet {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.clone())
        .expect("a wallet")
}

/// П3 (reproduced): «Accept the request» is signed with the credits it shows; the write runs
/// and the credits drop meanwhile (a balance pass reads the identity charged) — «now › after»
/// stays what it was signed with, the fee not taken off twice. Mutant: `SpendFlow::credits`
/// always `now` → the line follows the dropped credits.
#[test]
fn now_and_after_stay_as_signed_while_the_write_runs() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    wait_for(&mut kit, "a request", |kit| {
        kit.query_all_by_role_and_label(Role::Button, "Accept")
            .next()
            .is_some()
    });
    kit.query_all_by_role_and_label(Role::Button, "Accept")
        .next()
        .expect("«Accept»")
        .click();
    kit.run_steps(4);
    assert!(has_text(&kit, "Accept the request"));
    let w = wallet_of(&kit);
    let fee = w.fees.contact_request;
    let signed = crate::components::credits_after_words(
        crate::components::Credits::of(&w),
        crate::components::Charge::About(fee),
    )
    .0;
    assert!(has_words(&kit, &signed), "before: «{signed}»");
    // Sent, its answer to come; the identity charged meanwhile.
    kit.state_mut()
        .contacts
        .flow
        .set_stage(crate::components::spend_flow::Stage::Waiting {
            ask: 999_999,
            signed: (),
        });
    with_wallet(&mut kit, move |w| w.balances.credits -= fee);
    kit.run_steps(3);
    let charged = crate::components::credits_after_words(
        crate::components::Credits::of(&wallet_of(&kit)),
        crate::components::Charge::About(fee),
    )
    .0;
    assert_ne!(signed, charged, "the credits did drop");
    assert!(has_words(&kit, &signed), "while it runs: still «{signed}»");
    assert!(!has_words(&kit, &charged), "not «{charged}»");
}

/// Send (a guard — it did not reproduce): «Sign and send» lets go of the amount at once
/// (`let_go_of_payment`), so while the payment is on its way the form's «Left» is «—» or the
/// balance with nothing taken off, never less than before the press (the amount off again).
#[test]
fn left_is_not_taken_off_twice_while_a_payment_is_on_its_way() {
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    kit.run_steps(3);
    wait_for(&mut kit, "«Sign and send» enabled", |kit| {
        kit.query_all_by_role_and_label(Role::Button, "Sign and send")
            .last()
            .is_some_and(|b| !b.accesskit_node().is_disabled())
    });
    let amount = crate::format::parse_amount(&kit.state().send.amount).expect("an amount");
    let left_of = |kit: &Kit| {
        let texts = painted_texts(kit);
        texts
            .iter()
            .position(|t| t == "Left")
            .and_then(|i| texts.get(i + 1))
            .cloned()
    };
    let before = left_of(&kit)
        .and_then(|v| crate::format::parse_amount(v.trim_end_matches(" DASH")))
        .expect("«Left» before the press");
    kit.query_all_by_role_and_label(Role::Button, "Sign and send")
        .last()
        .expect("«Sign and send»")
        .click();
    // The frame of the press: the payment is on its way, its answer (the demo refuses) not
    // read yet — a refusal puts the amount back, as it should.
    kit.run_steps(1);
    assert!(kit.state().send.amount.is_empty(), "the form let go of it");
    assert!(amount > 0);
    // «—», or never less than before the press: the amount is not taken off a second time.
    let left = left_of(&kit);
    let after = left
        .as_deref()
        .filter(|v| *v != "—")
        .map(|v| crate::format::parse_amount(v.trim_end_matches(" DASH")));
    assert!(
        after.is_none_or(|a| a.is_some_and(|a| a >= before)),
        "«Left» while it is on its way: {left:?}, before the press {before}"
    );
}
