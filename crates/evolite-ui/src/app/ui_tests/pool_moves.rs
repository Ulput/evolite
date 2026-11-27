//! The pool's dialogs while a move runs (BUG-101): what the core has already done for the move
//! is not a reason to reproach it. Progress and the answer are set on the app's own state: the
//! demo core has no asset lock to remember and answers at once.

use super::identity_pool::with_wallet;
use super::*;
use crate::screens::shielded::PoolMove;
use evolite_api::CoreError;

const STOPPED: &str = "A stopped shielding";
const POOL_TOO_LITTLE: &str = "More than there is in the pool";
const CORE_RANGE: &str = "the Platform fee comes out of the amount";
const LOCKED: i64 = 50_000_000;

/// The dialog after `state` and one more snapshot with the wallet as `change` leaves it.
fn in_state(
    kit: &mut Kit,
    change: impl Fn(&mut evolite_api::Wallet),
    state: impl FnOnce(&mut crate::screens::shielded::PoolState),
) {
    state(&mut kit.state_mut().pool);
    with_wallet(kit, change);
    kit.run_steps(2);
}

/// BUG-101 (a): the shield the person has just started has its lock remembered by the core
/// as soon as the coins are locked. Only when it has really stopped is it called stopped.
#[test]
fn a_shield_at_work_is_not_called_stopped() {
    let (_turn, mut kit) = demo_app("shield-core", "en");
    let locked = |w: &mut evolite_api::Wallet| w.pool.unfinished_shield = Some(LOCKED);
    in_state(&mut kit, locked, |_| {});
    assert!(has_words(&kit, STOPPED), "not running: it is offered");
    in_state(&mut kit, locked, |p| p.progress = Some(0.4));
    assert!(!has_words(&kit, STOPPED), "called stopped while it runs");
    in_state(&mut kit, locked, |p| {
        p.progress = None;
        p.result = Some((PoolMove::Shield, Err(CoreError::NotAvailable("No".into()))));
    });
    assert!(has_words(&kit, STOPPED), "failed: it is really stopped");
    in_state(&mut kit, locked, |p| {
        p.result = Some((PoolMove::Shield, Ok(())))
    });
    assert!(
        !has_words(&kit, STOPPED),
        "done, and the core has not said so yet"
    );
}

/// BUG-101 (a): «Continue» of a stopped shield: the lock is still what moves, the amount field
/// does not come back, and there is no «stopped» over its progress.
#[test]
fn a_continued_shield_keeps_its_amount_and_says_no_stopped() {
    let (_turn, mut kit) = demo_app("shield-core", "en");
    let locked = |w: &mut evolite_api::Wallet| w.pool.unfinished_shield = Some(LOCKED);
    in_state(&mut kit, locked, |p| {
        p.progress = Some(0.4);
        p.continuing = true;
        p.amount.clear();
    });
    assert!(!has_words(&kit, STOPPED));
    let fee = kit.state().snapshot.as_ref().unwrap();
    let fee = fee.wallet.as_ref().unwrap().fees.shield_from_core;
    let arrives = format!(
        "Arrives in the pool: {}",
        crate::format::amount(LOCKED - fee)
    );
    assert!(has_text(&kit, &arrives), "the locked amount still counted");
}

/// BUG-101 (b), the shield from Core: the locked coins have left Core, what was typed is more
/// than Core has now. Said while typing, not while the proof is built and not after it.
#[test]
fn a_shield_at_work_is_not_told_it_is_too_much() {
    let (_turn, mut kit) = demo_app("shield-core", "en");
    let spent = |w: &mut evolite_api::Wallet| {
        w.balances.core = 1_000;
        w.balances.core_as_of = None;
    };
    in_state(&mut kit, spent, |_| {});
    assert!(has_words(&kit, CORE_RANGE), "typing: it is said");
    in_state(&mut kit, spent, |p| p.progress = Some(0.4));
    assert!(!has_words(&kit, CORE_RANGE), "said while it runs");
    in_state(&mut kit, spent, |p| {
        p.progress = None;
        p.result = Some((PoolMove::Shield, Ok(())));
    });
    assert!(!has_words(&kit, CORE_RANGE), "said after it succeeded");
}

/// BUG-101 (b): the note being spent has left the pool's balance while the proof is built.
#[test]
fn an_unshield_at_work_is_not_told_it_is_too_much() {
    let (_turn, mut kit) = demo_app("unshield", "en");
    let spent = |w: &mut evolite_api::Wallet| w.balances.shielded = 1_000;
    in_state(&mut kit, spent, |_| {});
    assert!(has_words(&kit, POOL_TOO_LITTLE), "typing: it is said");
    in_state(&mut kit, spent, |p| p.progress = Some(0.4));
    assert!(!has_words(&kit, POOL_TOO_LITTLE), "said while it runs");
    in_state(&mut kit, spent, |p| {
        p.progress = None;
        p.result = Some((PoolMove::Unshield, Ok(())));
    });
    assert!(!has_words(&kit, POOL_TOO_LITTLE), "said after it succeeded");

    // BUG-274: after it, the answer and «Done» — no amount to type over the success; «Done»
    // closes the window and the old answer goes with it.
    assert!(
        kit.query_all_by_role(Role::TextInput).next().is_none(),
        "no amount field over the success"
    );
    kit.query_all_by_role_and_label(Role::Button, "Done")
        .next()
        .expect("«Done»")
        .click();
    kit.run_steps(3);
    assert!(kit.state().pool.result.is_none(), "the old answer is gone");
}

/// BUG-101: the core's answer ends «Continue» too: the next shield is one started by the person.
#[test]
fn the_answer_ends_a_continued_shield() {
    use evolite_api::{ShieldSource, Spend};
    let (_turn, mut kit) = demo_app("shield", "en");
    kit.state_mut().pool.continuing = true;
    kit.state_mut().pool.progress = Some(0.4);
    let op = Spend::Shield {
        from: ShieldSource::Credits,
        amount: 1_000,
    };
    let command = kit
        .state_mut()
        .pool
        .flow
        .command(op, false, crate::screens::shielded::PoolMove::Shield)
        .unwrap();
    kit.state_mut().core.send(command);
    wait_for(&mut kit, "the core's answer", |kit| {
        kit.state().pool.result.is_some()
    });
    assert!(!kit.state().pool.continuing);
    assert!(kit.state().pool.progress.is_none());
}

/// BUG-101, review: an unshield or a pool top-up closed mid-proof then succeeds: its answer is not
/// this shield's. The stopped shield is still offered with «Continue», and closing its dialog
/// leaves the other move's answer alone.
#[test]
fn another_moves_success_leaves_a_stopped_shield_offered() {
    use evolite_api::Spend;
    for op in [
        Spend::Unshield { amount: 1_000 },
        // FEAT-0049 r1: «Unshield» to Core is answered as an unshield, not as a payment.
        Spend::UnshieldToCore { amount: 1_000 },
        Spend::TopUpFromPool { amount: 1_000 },
    ] {
        let (_turn, mut kit) = demo_app("shielded", "en");
        let locked = |w: &mut evolite_api::Wallet| w.pool.unfinished_shield = Some(LOCKED);
        in_state(&mut kit, locked, |p| {
            p.from = 2;
            p.progress = Some(0.4);
        });
        // Sent as the dialog sends it: the answer is taken by its number.
        let command = kit
            .state_mut()
            .pool
            .flow
            .command(op, false, PoolMove::Unshield)
            .unwrap();
        kit.state_mut().core.send(command);
        wait_for(&mut kit, "the core's answer", |kit| {
            kit.state().pool.result.is_some()
        });
        // The demo refuses these: its answer shows whose it is, the success is put in its place.
        let answer = kit.state().pool.result.as_ref().map(|(of, _)| *of);
        assert_eq!(
            answer,
            Some(PoolMove::Unshield),
            "the answer is the move's own"
        );
        in_state(&mut kit, locked, |p| {
            p.result = Some((PoolMove::Unshield, Ok(())));
            p.shielding = true;
        });
        assert!(has_words(&kit, STOPPED), "its success hid the stopped one");
        assert!(!has_text(&kit, "The funds are in the pool."));
        assert!(
            !button(&kit, "Continue").accesskit_node().is_disabled(),
            "«Continue» is offered"
        );
        button(&kit, "Cancel").click();
        kit.run_steps(3);
        assert!(!kit.state().pool.shielding, "the dialog closed");
        assert_eq!(
            kit.state().pool.result,
            Some((PoolMove::Unshield, Ok(()))),
            "closing another dialog leaves it"
        );
    }
}

/// BUG-101, review, the other way: a shield's success does not stop the unshield's amount being
/// judged.
#[test]
fn a_shields_success_is_not_an_unshields() {
    let (_turn, mut kit) = demo_app("unshield", "en");
    let spent = |w: &mut evolite_api::Wallet| w.balances.shielded = 1_000;
    in_state(&mut kit, spent, |p| {
        p.result = Some((PoolMove::Shield, Ok(())));
    });
    assert!(has_words(&kit, POOL_TOO_LITTLE));
}

fn wallet_of(kit: &Kit) -> evolite_api::Wallet {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.clone())
        .expect("a wallet")
}

/// A move sent and still running as the dialog leaves it: its answer not come.
fn still_running(p: &mut crate::screens::shielded::PoolState, move_: PoolMove, signed: Signed) {
    p.flow
        .set_stage(crate::components::spend_flow::Stage::Waiting {
            ask: 999_999,
            signed: move_,
        });
    p.progress = Some(0.07);
    p.signed = signed;
}

type Signed = Option<(Option<evolite_api::Duffs>, Option<evolite_api::Duffs>)>;

/// BUG-318: while a shield from Core runs, «Left on Core» is what it was signed with — the coins
/// have left the balance for the asset lock, and the amount is not taken off a second time.
/// «Shield» keeps the figures it was signed with. Mutant: the signed figures not used → the
/// amount twice.
#[test]
fn a_running_shield_shows_what_it_was_signed_with() {
    let (_turn, mut kit) = demo_app("shield", "en");
    let w = wallet_of(&kit);
    let amount = (w.balances.core / 4 / 10_000) * 10_000;
    let left = w.balances.core - amount - w.fees.core_payment;
    let arrives = amount - w.fees.shield_from_core;
    in_state(
        &mut kit,
        |_| {},
        |p| {
            // The preset shows a shield at work: none here, so «Shield» can be pressed.
            p.progress = None;
            p.from = 2;
            p.amount = crate::format::input_text(amount);
        },
    );
    let said = format!("≈ {}", crate::format::balance(left));
    assert!(has_words(&kit, &said), "before: «{said}»");
    // The dialog's «Shield» (the screen has its own behind it).
    kit.query_all_by_role_and_label(Role::Button, "Shield")
        .last()
        .expect("the dialog's «Shield»")
        .click();
    kit.run_steps(1);
    assert_eq!(
        kit.state().pool.signed,
        Some((Some(left), Some(arrives))),
        "signed with what the summary said"
    );
    // The coins in the lock: the balance less the amount and Core's fee, the answer to come.
    let locked = move |w: &mut evolite_api::Wallet| w.balances.core = left;
    in_state(&mut kit, locked, |p| {
        still_running(p, PoolMove::Shield, Some((Some(left), Some(arrives))));
    });
    assert!(has_words(&kit, &said), "while it runs: still «{said}»");
    let twice = format!(
        "≈ {}",
        crate::format::balance(left - amount - w.fees.core_payment)
    );
    assert!(!has_words(&kit, &twice), "not «{twice}»");
}

/// BUG-318, the same rule: while an unshield runs, «Left in the pool» is what it was signed
/// with, not the pool's balance that has dropped already.
#[test]
fn a_running_unshield_shows_what_it_was_signed_with() {
    let (_turn, mut kit) = demo_app("unshield", "en");
    let w = wallet_of(&kit);
    let pool = w.balances.shielded;
    let amount = (pool / 4 / 10_000) * 10_000;
    let fee = crate::screens::shielded::unshield_fee(&w.fees, kit.state().pool.unshield_to);
    let left = crate::format::left_after(pool, amount, fee);
    in_state(
        &mut kit,
        |_| {},
        |p| {
            p.unshield_amount = crate::format::input_text(amount);
        },
    );
    let said = crate::format::balance(left);
    assert!(has_words(&kit, &said), "before: «{said}»");
    let spent = move |w: &mut evolite_api::Wallet| w.balances.shielded = left;
    in_state(&mut kit, spent, |p| {
        still_running(p, PoolMove::Unshield, Some((Some(left), Some(amount))));
    });
    assert!(has_words(&kit, &said), "while it runs: still «{said}»");
    let twice = crate::format::balance(crate::format::left_after(left, amount, fee));
    assert!(!has_words(&kit, &twice), "not «{twice}»");
}
