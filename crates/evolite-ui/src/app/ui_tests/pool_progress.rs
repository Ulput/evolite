//! BUG-317: a top-up from the pool sent from «Top up the identity» — its progress is that
//! window's, not the pool's: «Shield into the private pool» is never left at «Building the
//! proof 95 %» with nothing awaited, and its moves stay free.

use super::*;
use crate::components::dialog_seen;

/// A real click (pointer events) at `at`.
fn click_at(kit: &mut Kit, at: egui::Pos2) {
    kit.hover_at(at);
    kit.run_steps(1);
    for pressed in [true, false] {
        kit.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
    }
    kit.run_steps(2);
}

fn type_password(kit: &mut Kit) {
    for _ in 0..2 {
        if let Some(field) = kit.query_all_by_role(Role::PasswordInput).next() {
            field.click();
        }
        kit.run_steps(2);
    }
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .type_text("the demo takes any other");
    kit.run_steps(2);
}

#[test]
fn a_top_up_from_the_pool_leaves_no_progress_on_the_pool() {
    let (_turn, mut kit) = demo_app("top-up-pool", "en");
    settle(&mut kit);
    kit.state_mut().identity.top_up.as_mut().unwrap().amount = crate::format::input_text(1_000_000);
    settle(&mut kit);
    kit.state_mut().pretend_paid = true;
    kit.state_mut().last_done = None;
    // The top-up's act, then its review's with the password: as a person presses them.
    for _ in 0..3 {
        if kit.state().identity.top_up_flow.waiting() {
            break;
        }
        if kit.query_all_by_role(Role::PasswordInput).next().is_some() {
            type_password(&mut kit);
        }
        let ready = dialog_seen(&kit.ctx).and_then(|d| d.spend_ready);
        let Some(at) = ready else {
            settle(&mut kit);
            continue;
        };
        click_at(&mut kit, at.center());
        settle(&mut kit);
    }
    assert!(
        kit.state().identity.top_up_flow.waiting(),
        "the top-up from the pool is sent"
    );
    // While the core reports the pool's progress, it is the top-up's, not the pool's.
    wait_for(&mut kit, "a progress reported", |kit| {
        kit.state().last_done.is_some() || kit.state().identity.top_up_flow.waiting()
    });
    kit.run_steps(20);
    assert_eq!(kit.state().pool.progress, None, "no bar over the pool");
    wait_for(&mut kit, "the top-up's answer", |kit| {
        kit.state().last_done.is_some()
    });
    settle(&mut kit);
    assert_eq!(kit.state().pool.progress, None, "nothing left behind");
    assert!(!kit.state().pool.flow.waiting());
}

/// The class: a pool move's progress with nothing of the pool's awaited is not taken, and a
/// move's end clears what is left (the lock too).
#[test]
fn the_pools_progress_is_only_its_own_moves() {
    let mut pool = crate::screens::shielded::PoolState::default();
    pool.on_progress(0.5);
    assert_eq!(pool.progress, None, "nothing awaited: not the pool's");
    pool.progress = Some(0.95);
    pool.settle_progress();
    assert_eq!(pool.progress, None, "an end with nothing awaited clears it");
}
