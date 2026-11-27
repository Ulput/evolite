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

/// AC-T4: the pool's two windows keep no armed button after a success. BUG-274 (the class: one
/// rule for every dialog that pays): once it is done the window shows its answer and «Done» —
/// no «Shield» or «Unshield» in it at all, on or off; the next move is the window opened again.
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
        // The page has a button of that name too; the window adds its own.
        let named = |kit: &Kit| kit.query_all_by_role_and_label(Role::Button, name).count();
        let armed = |kit: &Kit| {
            kit.query_all_by_role_and_label(Role::Button, name)
                .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
                .is_some_and(|b| !b.accesskit_node().is_disabled())
        };
        let before = named(&kit);
        assert!(armed(&kit), "{preset}: on before");
        kit.state_mut().pool.result = Some((move_, Ok(())));
        kit.run_steps(3);
        assert_eq!(
            named(&kit),
            before - 1,
            "{preset}: the window's own button gone once done"
        );
        assert!(
            kit.query_all_by_role_and_label(Role::Button, "Done")
                .next()
                .is_some(),
            "{preset}: «Done» instead"
        );
        drop(turn);
    }
}

const FROM_POOL: i64 = 100_000;

fn pool_label() -> String {
    format!("Top up by {}", crate::format::amount(FROM_POOL))
}

/// BUG-346: a top-up from the pool sent, the pool's balance dropped by it below a top-up's fee —
/// the window stays the pool's (not «From the Core wallet» with its asset lock words), and once
/// answered it is not Core's form with the amount, a click from a second top-up. Mutant: the
/// source by the order again (`sent_from` not kept) → «From the Core wallet».
#[test]
fn a_top_up_from_the_pool_stays_the_pools_while_it_runs_and_after() {
    let (_turn, mut kit) = demo_app("top-up-pool", "en");
    settle(&mut kit);
    assert!(has_words(&kit, "From the pool"));
    if let Some(form) = &mut kit.state_mut().identity.top_up {
        form.amount = crate::format::input_text(FROM_POOL);
    }
    wait_for(&mut kit, "the button with the amount", |kit| {
        has_text(kit, &pool_label())
    });
    button(&kit, &pool_label()).click();
    kit.run_steps(1);
    assert_eq!(
        kit.state()
            .identity
            .top_up
            .as_ref()
            .and_then(|f| f.sent_from),
        Some(crate::screens::identity::TopUpFrom::Pool),
        "sent from the pool"
    );
    // The notes taken: the pool holds less than a top-up's fee; no mixed coins either.
    let drained = |w: &mut evolite_api::Wallet| {
        w.balances.shielded = w.fees.pool_top_up - 1;
        w.balances.mixed = None;
    };
    show_wallet_edited(&mut kit, 4, "the pool drained", drained, |_| true);
    assert!(has_words(&kit, "From the pool"), "still the pool's window");
    assert!(
        !has_words(&kit, "From the Core wallet"),
        "not Core's form in the middle of a pool move"
    );
    // Answered: it went through (the demo refuses: the answer is the test's). The answer and
    // «Done» — not Core's form with the amount, a second top-up one click away.
    kit.state_mut().identity.top_up_result = Some((FROM_POOL, Ok(())));
    show_wallet_edited(&mut kit, 4, "answered", drained, |_| true);
    assert!(has_words(&kit, "Credits added"), "the answer");
    assert!(
        !has_words(&kit, "From the Core wallet"),
        "not Core's form after it"
    );
    assert!(
        kit.query_all_by(
            |n| n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with("Top up by"))
        )
        .next()
        .is_none(),
        "no «Top up by …» after it"
    );
    assert!(has_text(&kit, "Done"));
}

/// BUG-319 r3: the pool's fee for a top-up is promised as at most — «up to ≈», never a bare «≈»
/// the network may beat (QA: 0.00111 taken of a declared 0.0011436).
#[test]
fn a_top_up_from_the_pool_promises_the_fee_at_most() {
    let (_turn, mut kit) = demo_app("top-up-pool", "en");
    settle(&mut kit);
    if let Some(form) = &mut kit.state_mut().identity.top_up {
        form.amount = crate::format::input_text(FROM_POOL);
    }
    settle(&mut kit);
    let fee = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.fees.pool_top_up)
        .expect("a wallet");
    let up_to = format!("up to ≈ {}", crate::format::amount(fee));
    assert!(has_words(&kit, &up_to), "«{up_to}»");
}
