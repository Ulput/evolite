//! Branch O: «Forgot the password?» and the way back while a wallet is being added.

use super::*;
use crate::screens::Screen;

/// The space between the «Type «delete»» field and the deletion's buttons.
fn under_the_word(kit: &Kit) -> f32 {
    let field = kit
        .query_all_by_role(Role::TextInput)
        .last()
        .expect("the «delete» field")
        .rect();
    gap(field, button(kit, "Delete from the computer").rect())
}

/// BUG-39 №13: the deletion dialog gets shorter — «Forgot the password?» has no password field
/// (the dialog remembers its last size) — and its buttons still follow the «delete» field, with
/// no empty space between them.
#[test]
fn a_shorter_deletion_dialog_has_no_gap_under_the_word() {
    let (_turn, mut kit) = demo_app("delete", "en");
    kit.run_steps(5);
    let first = under_the_word(&kit);
    kit.state_mut()
        .settings
        .delete
        .as_mut()
        .expect("the deletion dialog")
        .forgot = true;
    kit.run_steps(5);
    let shorter = under_the_word(&kit);
    assert!(
        (shorter - first).abs() < 1.0,
        "{shorter} px under the word, {first} px before"
    );
}

/// BUG-43: adding a wallet, the onboarding's steps have one way back at the top left — its own
/// «‹ Back» — not «Back to “…”» drawn over it.
#[test]
fn adding_a_wallet_the_password_step_has_one_way_back() {
    let (_turn, mut kit) = demo_app("wallet-add", "en");
    wait_for(&mut kit, "the wallet being added", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.adding_wallet)
    });
    kit.state_mut().screen = Screen::Onboarding(crate::screens::onboarding::Flow::Restore);
    kit.run_steps(3);
    if let Some(o) = kit.state_mut().onboarding.as_mut() {
        o.step = crate::screens::onboarding::Step::Password;
    }
    kit.run_steps(3);
    let backs: Vec<String> = kit
        .query_all_by(|n| {
            n.role() == Role::Button
                && n.label()
                    .is_some_and(|l| l == "Back" || l.starts_with("Back to"))
        })
        .map(|n| n.accesskit_node().label().unwrap_or_default())
        .collect();
    assert_eq!(backs, ["Back"], "the ways back at the top");
}
