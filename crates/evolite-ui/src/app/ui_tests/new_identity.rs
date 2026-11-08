//! FEAT-0014: one more identity — «+ New identity» last in «Work as», why it is off, the form
//! with no name, «Check the new identity», the banner after it, and the DashPay-on-Android line
//! (AC-T9, AC-T10, AC-D1–D11).

use super::*;
use crate::screens::Screen;
use evolite_api::Wallet;

const NEW: &str = "Create a new identity";

/// Changes the wallet the app shows, and draws, a few times over: a snapshot of the demo core
/// in between would undo it.
fn with_wallet(kit: &mut Kit, change: impl Fn(&mut Wallet)) {
    for _ in 0..3 {
        if let Some(wallet) = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
        {
            change(wallet);
        }
        kit.run_steps(1);
    }
}

/// A label or a name with `text` in it.
fn shows(kit: &Kit, text: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.contains(text)) || n.value().is_some_and(|v| v.contains(text))
    })
    .next()
    .is_some()
}

/// The button whose name starts with `start`, if on.
fn on(kit: &Kit, start: &str) -> Option<bool> {
    kit.query_all_by_role(Role::Button)
        .find(|b| {
            b.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with(start))
        })
        .map(|b| !b.accesskit_node().is_disabled())
}

/// AC-T10, AC-D1: «+ New identity» stands with one identity too, named «Create a new identity»
/// for a screen reader, and opens the form called «New identity».
#[test]
fn the_new_identity_pill_stands_with_one_identity_and_opens_the_form() {
    let (_turn, mut kit) = demo_app("identity", "en");
    with_wallet(&mut kit, |w| w.identities.truncate(1));
    assert!(has_text(&kit, "Work as"), "«Work as» with one identity");
    assert_eq!(on(&kit, NEW), Some(true), "«{NEW}» on");
    button(&kit, NEW).click();
    kit.run_steps(3);
    assert_eq!(kit.state().screen, Screen::CreateIdentity);
    assert!(has_text(&kit, "New identity"));
    assert!(has_text(&kit, "1 · Name (optional)"));
}

/// AC-T10, AC-D2 (R-4): with a creation cut short the pill is off and the reason is in words
/// under the row, without hovering.
#[test]
fn the_pill_is_off_with_its_reason_while_a_creation_is_unfinished() {
    let (_turn, mut kit) = demo_app("identity-unfinished", "en");
    kit.run_steps(3);
    assert_eq!(on(&kit, NEW), Some(false), "«{NEW}» off");
    assert!(has_text(
        &kit,
        "Finish creating the identity first: “Continue” is on its card"
    ));
    button(&kit, NEW).click();
    kit.run_steps(3);
    assert_eq!(
        kit.state().screen,
        Screen::Identity,
        "an off pill goes nowhere"
    );
}

/// R-4: while «Create» waits for the core the pill is off too, and says the creation is under
/// way — the snapshot does not show it yet.
#[test]
fn the_pill_is_off_while_a_creation_is_under_way() {
    let (_turn, mut kit) = demo_app("identity", "en");
    kit.run_steps(3);
    assert_eq!(on(&kit, NEW), Some(true), "«{NEW}» on before");
    kit.state_mut().create_identity.waiting = true;
    kit.run_steps(2);
    assert_eq!(on(&kit, NEW), Some(false), "«{NEW}» off while creating");
    assert!(has_text(
        &kit,
        "Creating: asset lock, confirmation, identity and name — about a minute. Do not close evolite."
    ));
    kit.state_mut().create_identity.waiting = false;
    kit.run_steps(2);
    assert_eq!(on(&kit, NEW), Some(true), "«{NEW}» on once answered");
}

/// R-4 (review r1 must 2): a stopped top-up does not hold a new identity back, and the pill
/// sends nobody to continue it — werner555's stopped 0.5 DASH top-up must not be continued.
#[test]
fn a_stopped_top_up_leaves_the_pill_on() {
    let (_turn, mut kit) = demo_app("identity", "en");
    with_wallet(&mut kit, |w| {
        w.unfinished_top_up = Some(evolite_api::UnfinishedTopUp {
            locked: 50_000_000,
            awaiting_chain_lock: false,
        });
    });
    assert_eq!(on(&kit, NEW), Some(true), "«{NEW}» on");
    assert!(
        !has_text(&kit, "First finish the stopped top-up"),
        "no word of the top-up under the pills"
    );
    button(&kit, NEW).click();
    kit.run_steps(3);
    assert_eq!(kit.state().screen, Screen::CreateIdentity);
}

/// AC-T10: Tab comes to the pills and the arrows go along them to «+ New identity».
#[test]
fn arrows_go_along_work_as_to_the_new_identity() {
    let (_turn, mut kit) = demo_app("identity", "en");
    let first = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| {
            w.identities
                .iter()
                .map(|i| format!("@{}", i.name.clone().unwrap_or_default()))
                .collect::<Vec<_>>()
        })
        .expect("the demo's identities");
    tab_to(&mut kit, &first[0]);
    for _ in 1..first.len() {
        kit.key_press(Key::ArrowRight);
        kit.run_steps(2);
    }
    assert_eq!(
        focused(&kit).as_deref(),
        Some(first[first.len() - 1].as_str())
    );
    kit.key_press(Key::ArrowRight);
    kit.run_steps(2);
    assert_eq!(focused(&kit).as_deref(), Some(NEW));
}

/// AC-T10, AC-D3, frame 4: an empty name is no mistake — the line says what it means, the name
/// costs nothing, the fourth step is optional, and «Create» is on.
#[test]
fn the_form_without_a_name_says_what_it_means_and_creates() {
    let (_turn, mut kit) = demo_app("create-identity-nameless", "en");
    wait_for(&mut kit, "«Create for …» on", |kit| {
        on(kit, "Create for ") == Some(true)
    });
    assert!(has_text(
        &kit,
        "Without a name nobody finds it by @name or adds it to DashPay contacts. You can add a name later on the Identity screen."
    ));
    assert!(has_text(&kit, "Name registration: no name · 0"));
    assert!(has_text(&kit, "Name, if given"));
    assert!(has_text(&kit, "2 · Top-up"));
    assert!(has_text(&kit, "Pay from"));
    assert!(
        kit.query_all_by_role(Role::RadioButton)
            .any(|r| r.accesskit_node().label().as_deref() == Some("From Core")),
        "«From Core», chosen"
    );
}

/// KICKOFF, review r1 should 6 (mutation B9): the first identity needs a name — only the UI
/// holds this rule, the core takes an empty one. With no identity yet an empty name is not
/// «no name»: no line saying what that means, nothing at 0 for the name, «Create» off.
#[test]
fn the_first_identity_needs_a_name() {
    let (_turn, mut kit) = demo_app("create-identity-nameless", "en");
    wait_for(&mut kit, "«Create for …» on", |kit| {
        on(kit, "Create for ") == Some(true)
    });
    with_wallet(&mut kit, |w| {
        w.identity = None;
        w.identities.clear();
    });
    assert!(
        has_text(&kit, "Create an identity"),
        "the first one's title"
    );
    assert!(!has_text(
        &kit,
        "Without a name nobody finds it by @name or adds it to DashPay contacts. You can add a name later on the Identity screen."
    ));
    assert!(!has_text(&kit, "Name registration: no name · 0"));
    assert_eq!(on(&kit, "Create for "), Some(false), "«Create» off");
}

/// Review r1 must 1: without a name the form asks for what the core will take. At testnet's
/// prices 0.0015 DASH is enough with a name (at least 135 000 duffs) and too little without one
/// (200 000): the hint says the nameless least and «Create» is off.
#[test]
fn a_nameless_identity_asks_for_its_own_least_top_up() {
    let (_turn, mut kit) = demo_app("create-identity-nameless", "en");
    wait_for(&mut kit, "«Create for …» on", |kit| {
        on(kit, "Create for ") == Some(true)
    });
    let mut kept = false;
    for _ in 0..50 {
        kit.state_mut().create_identity.amount = "0.0015".to_owned();
        if let Some(w) = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
        {
            w.fees.name_regular = 65_000;
            w.fees.identity_create = 41_000;
            w.fees.identity_lock_min = 241_000;
        }
        kit.run_steps(1);
        kept = kit
            .state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .is_some_and(|w| w.fees.identity_lock_min == 241_000);
        if kept {
            break;
        }
    }
    assert!(kept, "the fees did not stay");
    assert!(has_text(
        &kit,
        &format!(
            "At least {} — Dash Platform takes no smaller asset lock",
            crate::format::amount(200_000)
        )
    ));
    assert_eq!(on(&kit, "Create for "), Some(false), "«Create» off");
}

/// AC-D7 (frame 7): «Check the new identity» lists the credits, the name and the creation and
/// their total from Core.
#[test]
fn the_review_lists_what_is_locked_and_its_total() {
    let (_turn, mut kit) = demo_app("create-identity-confirm", "en");
    wait_for(&mut kit, "«Sign and create» on", |kit| {
        on(kit, "Sign and create") == Some(true)
    });
    let fees = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.fees.clone())
        .expect("fees");
    let total = 30_000_000 + fees.name_regular + fees.identity_create;
    assert!(has_text(&kit, "Check the new identity"));
    assert!(shows(
        &kit,
        &format!("Total from Core: {}", crate::format::amount(total))
    ));
    assert!(shows(&kit, &crate::format::amount(total)), "the total");
    assert!(shows(&kit, "@evolite-2026"));
}

/// AC-T10, AC-D4, AC-D5, AC-T9 (analyst-44): created without a name, the new identity is the
/// chosen one on Identity with its banner, the DashPay-on-Android line (its slot is not 0), and
/// «×» closes the banner.
#[test]
fn a_new_identity_is_shown_with_its_banner_which_closes() {
    let (_turn, mut kit) = demo_app("create-identity-nameless", "en");
    wait_for(&mut kit, "«Create for …» on", |kit| {
        on(kit, "Create for ") == Some(true)
    });
    kit.query_all_by_role(Role::Button)
        .find(|b| {
            b.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Create for "))
        })
        .expect("«Create for …»")
        .click();
    kit.run_steps(3);
    assert!(has_text(&kit, "Check the new identity"));
    button(&kit, "Sign and create").click();
    wait_for(&mut kit, "Identity with the banner", |kit| {
        kit.state().screen == Screen::Identity && has_text(kit, "Identity created")
    });
    assert!(
        shows(&kit, "(no name)"),
        "the banner names it by its short id"
    );
    assert!(has_text(
        &kit,
        "DashPay on Android shows only the first identity of this phrase: this one is not visible there."
    ));
    assert!(has_text(
        &kit,
        "No names yet. With a name the identity can be found by @name and added to contacts."
    ));
    button(&kit, "Close").click();
    kit.run_steps(3);
    assert!(!has_text(&kit, "Identity created"));
}

/// R-9 (AC-T9): the DashPay-on-Android line only on an identity that is not slot 0.
#[test]
fn the_dashpay_android_line_is_only_off_slot_zero() {
    let line = "DashPay on Android shows only the first identity of this phrase: this one is not visible there.";
    let (_turn, mut kit) = demo_app("identity", "en");
    with_wallet(&mut kit, |w| {
        if let Some(i) = w.identity.as_mut() {
            i.slot = 0;
        }
    });
    assert!(!has_text(&kit, line), "slot 0");
    with_wallet(&mut kit, |w| {
        if let Some(i) = w.identity.as_mut() {
            i.slot = 1;
        }
    });
    assert!(has_text(&kit, line), "slot 1");
}

/// AC-D8 (frames 9 and 10): too little on Core says both sums in place of the InstantSend hint;
/// Core not checked now says it is being checked — and «Create» is off in both.
#[test]
fn too_little_or_unchecked_core_keeps_create_off() {
    let (_turn, mut kit) = demo_app("create-identity-short", "en");
    wait_for(&mut kit, "«At least ≈ … is needed»", |kit| {
        shows(kit, "is needed, Core has")
    });
    assert_eq!(on(&kit, "Create for "), Some(false));
    drop(kit);
    drop(_turn);
    let (_turn, mut kit) = demo_app("create-identity-unchecked", "en");
    wait_for(&mut kit, "«still being checked»", |kit| {
        has_text(
            kit,
            "The Core balance is still being checked: you can create an identity once the sync is done.",
        )
    });
    assert_eq!(on(&kit, "Create for "), Some(false));
}

/// Frame 8 (AC-D9): while the core creates, its steps stand over the form; «Hide» closes the
/// window only — the creation still waits, and «What happens» beside the form goes on.
#[test]
fn the_steps_stand_over_the_form_while_creating_and_hide() {
    let (_turn, mut kit) = demo_app("create-identity-nameless", "en");
    kit.run_steps(3);
    assert!(
        !has_text(&kit, "Identity creation"),
        "no steps before «Create»"
    );
    kit.state_mut().create_identity.waiting = true;
    kit.state_mut().create_identity.step = Some(2);
    kit.run_steps(3);
    assert!(has_text(&kit, "Identity creation"), "the steps' window");
    button(&kit, "Hide").click();
    kit.run_steps(3);
    assert!(!has_text(&kit, "Identity creation"), "hidden");
    assert!(kit.state().create_identity.waiting, "the creation goes on");
    assert!(has_text(&kit, "What happens"));
}
