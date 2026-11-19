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
/// and has no password (BUG-42: the password is on the review). BUG-172 (AC-D1, AC-D2, frame
/// F1): the amount opens empty — no sum is put into what gets signed — «Total from Core» says
/// «—» and «Review» is off; an amount typed turns it on.
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
    let form = kit.state().fund_address.form.as_ref().unwrap();
    assert!(
        form.amount.is_empty() && form.suggested.is_none(),
        "no amount put in"
    );
    assert!(!enabled(&kit, "Review"), "nothing to review yet");
    // The rows' values are painted, not in the tree: read them off the frame.
    fn painted(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| painted(s, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for c in &kit.output().shapes {
        painted(&c.shape, &mut texts);
    }
    assert!(
        texts.iter().any(|t| t == "—"),
        "«Total from Core» waits for an amount: {texts:?}"
    );
    kit.state_mut().fund_address.form.as_mut().unwrap().amount =
        crate::format::input_text(10_000_000);
    kit.run_steps(3);
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

/// BUG-160: once a stopped one's «Continue» failed, it no longer holds a new top-up back: the
/// tile's action is active again, and the stopped one is still offered.
#[test]
fn a_failed_continue_does_not_dim_the_tile() {
    let (_turn, mut kit) = demo_app("fund-address-unfinished", "en");
    settle(&mut kit);
    assert!(!enabled(&kit, "Top up from Core ›"), "held back first");
    kit.state_mut().dev.fund_unfinished_preset = false;
    show_wallet_edited(
        &mut kit,
        2,
        "the stopped one not holding back",
        |w| {
            if let Some(u) = w.unfinished_address_top_up.as_mut() {
                u.holds_back = false;
            }
        },
        |_| true,
    );
    assert!(
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .is_some_and(|w| w.unfinished_address_top_up.is_some()),
        "still stopped"
    );
    assert!(
        has_words(&kit, "are already locked in Core"),
        "still offered"
    );
    assert!(enabled(&kit, "Top up from Core ›"), "a new one may start");
}

/// BUG-171 AC-T2, AC-T3: «Continue» of a lock found after a restore says where it goes — the
/// address shown now, there being no record of the one signed for — and promises the lock less
/// what Platform keeps without an InstantLock.
#[test]
fn continuing_a_recovered_lock_says_where_and_how_much() {
    let (_turn, mut kit) = demo_app("fund-address-recovered", "en");
    kit.set_size(egui::vec2(1024.0, 1000.0));
    settle(&mut kit);
    wait_for(&mut kit, "the dialog", |kit| {
        kit.state().fund_address.resume_open
    });
    kit.run_steps(2);
    assert!(has_text(&kit, "To · this wallet's Platform address"));
    assert!(
        has_words(&kit, crate::app::dev::RECOVERED_TO),
        "the address it goes to"
    );
    let at_least =
        crate::format::amount(10_012_000 - evolite_api::address_fund::ADDRESS_FUND_ON_TOP);
    assert!(has_words(&kit, &at_least), "at least {at_least}");
}

/// BUG-171 AC-T4: «Credited» says what came, as the core proved it, not what was planned.
#[test]
fn credited_says_what_came() {
    let (_turn, mut kit) = demo_app("overview", "en");
    settle(&mut kit);
    let state = &mut kit.state_mut().fund_address;
    state.running = Some(100_000);
    state.on_funded(97_064);
    state.on_done(&Ok(()));
    assert_eq!(state.result, Some((97_064, Ok(()))));
    // Without the proved amount: the plan's.
    state.running = Some(100_000);
    state.on_done(&Ok(()));
    assert_eq!(state.result, Some((100_000, Ok(()))));
}
