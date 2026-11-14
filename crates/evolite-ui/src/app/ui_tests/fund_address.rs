//! FEAT-0015 stage 2, AC-T9: «Top up the Platform address» from Core on the demo core — the
//! tile's action (21), the form without a password (22), the review with it (22b), the steps
//! (23), the stopped one and its «Continue» (24, 24b), what it credited (25).

use super::*;
use crate::screens::Screen;

fn enabled(kit: &Kit, name: &str) -> bool {
    kit.query_all_by_role_and_label(Role::Button, name)
        .last()
        .is_some_and(|b| !b.accesskit_node().is_disabled())
}

fn type_password(kit: &mut Kit, password: &str) {
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .click();
    kit.run_steps(2);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .type_text(password);
    kit.run_steps(2);
}

fn platform_address(kit: &Kit) -> evolite_api::Duffs {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map_or(0, |w| w.balances.platform_address)
}

/// 21 → 22: «Top up from Core ›» on the Platform address tile opens the form; the form counts
/// and has no password (BUG-42: the password is on the review).
#[test]
fn the_tile_opens_the_form_without_a_password() {
    let (_turn, mut kit) = demo_app("overview", "en");
    settle(&mut kit);
    assert!(enabled(&kit, "Top up from Core ›"), "the tile's action");
    kit.query_all_by_role_and_label(Role::Button, "Top up from Core ›")
        .next()
        .unwrap()
        .click();
    wait_for(&mut kit, "the form", |kit| {
        has_text(kit, "Top up the Platform address")
    });
    assert!(
        kit.query_all_by_role(Role::PasswordInput).next().is_none(),
        "no password in the form"
    );
    assert!(has_words(&kit, "How much to credit"), "the amount field");
    assert!(enabled(&kit, "Review"), "0,1 DASH is a top-up");
}

/// 22: under the protocol's threshold «Review» is off and the form says the least.
#[test]
fn under_the_threshold_the_form_says_the_least() {
    let (_turn, mut kit) = demo_app("fund-address", "en");
    settle(&mut kit);
    kit.state_mut().fund_address.form.as_mut().unwrap().amount = crate::format::input_text(1_000);
    kit.state_mut()
        .fund_address
        .form
        .as_mut()
        .unwrap()
        .suggested = None;
    kit.run_steps(3);
    assert!(!enabled(&kit, "Review"), "«Review» under the threshold");
    assert!(
        has_words(&kit, "to the address"),
        "«At least … to the address»"
    );
}

/// 22b → 25: the review asks the password; «Sign and top up» waits for it; signed, the demo
/// credits the address and the Overview says so, with «Open in History».
#[test]
fn the_review_asks_the_password_and_the_address_is_credited() {
    let (_turn, mut kit) = demo_app("fund-address-review", "en");
    settle(&mut kit);
    let before = platform_address(&kit);
    assert!(
        kit.query_all_by_role(Role::PasswordInput).next().is_some(),
        "the password on the review"
    );
    assert!(
        !enabled(&kit, "Sign and top up"),
        "not without the password"
    );
    type_password(&mut kit, "the demo takes any other");
    assert!(enabled(&kit, "Sign and top up"));
    kit.query_all_by_role_and_label(Role::Button, "Sign and top up")
        .next()
        .unwrap()
        .click();
    wait_for(&mut kit, "the answer", |kit| {
        matches!(kit.state().fund_address.result, Some((_, Ok(()))))
    });
    wait_for(&mut kit, "the address credited", |kit| {
        platform_address(kit) >= before + 10_000_000
    });
    // The steps (23) were open; hidden, the Overview has the answer (25).
    kit.state_mut().fund_address.steps_open = false;
    kit.run_steps(3);
    assert!(has_words(&kit, "Credited to the Platform address"));
    let history = kit
        .query_all_by_role_and_label(Role::Button, "Open in History")
        .next()
        .expect("«Open in History»");
    history.click();
    wait_for(&mut kit, "History", |kit| {
        kit.state().screen == Screen::Activity
    });
}

/// 23: while it runs, the three steps; «Hide» closes them and the funding goes on.
#[test]
fn the_steps_show_and_hide() {
    let (_turn, mut kit) = demo_app("fund-address-steps", "en");
    settle(&mut kit);
    for step in [
        "Asset lock in Core",
        "Confirmation",
        "Credit to the address",
    ] {
        assert!(has_text(&kit, step), "{step}");
    }
    kit.query_all_by_role_and_label(Role::Button, "Hide")
        .next()
        .unwrap()
        .click();
    kit.run_steps(3);
    assert!(!kit.state().fund_address.steps_open);
    assert!(kit.state().fund_address.running.is_some(), "it goes on");
}

/// 24 → 24b: a stopped one: the banner and «Continue the top-up», the tile's action dim; the
/// dialog asks the password and continues without new money.
#[test]
fn a_stopped_one_is_continued() {
    let (_turn, mut kit) = demo_app("fund-address-unfinished", "en");
    settle(&mut kit);
    assert!(
        !enabled(&kit, "Top up from Core ›"),
        "no new top-up while one is stopped"
    );
    assert!(has_words(&kit, "are already locked in Core"));
    kit.query_all_by_role_and_label(Role::Button, "Continue the top-up")
        .next()
        .expect("«Continue the top-up»")
        .click();
    wait_for(&mut kit, "the dialog", |kit| {
        kit.state().fund_address.resume_open
    });
    // «Password for every send» off in the plain preset: nothing to type.
    let sign = |kit: &Kit| enabled(kit, "Sign and continue");
    wait_for(&mut kit, "«Sign and continue»", sign);
    kit.query_all_by_role_and_label(Role::Button, "Sign and continue")
        .next()
        .unwrap()
        .click();
    wait_for(&mut kit, "the answer", |kit| {
        kit.state().fund_address.result.is_some()
    });
}
