//! BUG-177: after «Top up» the window shows what was signed, not the form reckoned again on the
//! new balances — no second top-up of the same amount by one more click.

use super::*;

const AMOUNT: i64 = 10_000_000;

fn top_up_label() -> String {
    format!("Top up by {}", crate::format::amount(AMOUNT))
}

/// The top-up window of 0.1 DASH (the form's own suggestion), signed: the demo refuses spends, so
/// the core's answer is the test's.
fn signed_top_up() -> (MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("top-up", "en");
    wait_for(&mut kit, "the form", |kit| {
        kit.state().identity.top_up.is_some()
    });
    if let Some(form) = &mut kit.state_mut().identity.top_up {
        form.amount = crate::format::input_text(AMOUNT);
    }
    wait_for(&mut kit, "the button with the amount", |kit| {
        has_text(kit, &top_up_label())
    });
    button(&kit, &top_up_label()).click();
    // One frame: the click is taken and the top-up sent; the demo's refusal comes on the next
    // frame, so the answer is the test's (an answer nobody waits for is dropped).
    kit.run_steps(1);
    assert_eq!(kit.state().identity.topping_up(), Some(AMOUNT), "sent once");
    (turn, kit)
}

fn credits(kit: &Kit) -> i64 {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map_or(0, |w| w.balances.credits)
}

/// AC-T1, AC-T2: credited, the new balances in: no «Top up by 0.1000» to press again, no «More
/// than there is», «On the identity after» as signed (not the amount added twice). «Top up
/// again» gives an empty field.
#[test]
fn a_top_up_done_offers_no_second_one_of_the_same_amount() {
    let (_turn, mut kit) = signed_top_up();
    let before = credits(&kit);
    // Credited: the answer, and the credits the next snapshot brings.
    let ask = kit.state().identity.top_up_flow.waiting_ask();
    kit.state_mut().identity.on_top_up_done(ask, &Ok(()));
    // As much as the credits were, plus the top-up: put in again over a later snapshot, never
    // added twice.
    let credited = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map_or(0, |w| w.balances.credits)
        + AMOUNT;
    show_wallet_edited(
        &mut kit,
        3,
        "the credits added",
        |w| w.balances.credits = credited,
        |_| true,
    );
    assert!(has_words(&kit, "Credits added"));
    assert!(
        !has_text(&kit, &top_up_label()),
        "no button that signs the same amount again"
    );
    assert!(!has_text(&kit, "More than there is on Core"));
    let after = crate::format::balance(before + AMOUNT);
    assert!(
        has_words(&kit, &format!("On the identity after: {after}")),
        "as signed: {after}"
    );
    assert!(!button(&kit, "Top up again").accesskit_node().is_disabled());
    button(&kit, "Top up again").click();
    kit.run_steps(3);
    let form = kit.state().identity.top_up.as_ref().expect("the form");
    assert!(form.amount.is_empty(), "an empty field: {:?}", form.amount);
    assert!(form.signed.is_none());
    assert!(
        !has_text(&kit, &top_up_label()),
        "nothing to sign until an amount is typed"
    );
}

/// AC-T3: a top-up that did not go through gives the form back with its amount: trying again is
/// one deliberate click.
#[test]
fn a_failed_top_up_gives_the_form_back() {
    let (_turn, mut kit) = signed_top_up();
    let ask = kit.state().identity.top_up_flow.waiting_ask();
    kit.state_mut()
        .identity
        .on_top_up_done(ask, &Err(evolite_api::CoreError::NotSynced));
    kit.run_steps(3);
    assert!(
        has_text(&kit, &top_up_label()),
        "the form again, its amount kept"
    );
    assert!(!button(&kit, &top_up_label()).accesskit_node().is_disabled());
}

/// AC-T4: the pool's two windows keep no armed button after a success. «Shield» or «Unshield» is
/// off on the same amount once it is done, and a new amount typed makes the next one.
#[test]
fn a_pool_move_done_is_not_offered_again_on_the_same_amount() {
    use crate::screens::shielded::PoolMove;
    for (preset, move_, name) in [
        ("shield-core", PoolMove::Shield, "Shield"),
        ("unshield", PoolMove::Unshield, "Unshield"),
    ] {
        let (turn, mut kit) = demo_app(preset, "en");
        kit.run_steps(10);
        {
            let pool = &mut kit.state_mut().pool;
            match move_ {
                PoolMove::Shield => pool.amount = "0.01".into(),
                PoolMove::Unshield => pool.unshield_amount = "0.01".into(),
            }
        }
        kit.run_steps(3);
        let armed = |kit: &Kit| {
            // The window's own button: the lowest of that name (the page has one too).
            kit.query_all_by_role_and_label(Role::Button, name)
                .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
                .is_some_and(|b| !b.accesskit_node().is_disabled())
        };
        assert!(armed(&kit), "{preset}: on before");
        kit.state_mut().pool.result = Some((move_, Ok(())));
        kit.run_steps(3);
        assert!(!armed(&kit), "{preset}: off once done, on the same amount");
        drop(turn);
    }
}
