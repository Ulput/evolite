//! Where a dialog's focus starts and what Enter does in it: the designer's rule b′ (2026-11-06),
//! owned by `components::dialog` and `components::button_row` for every dialog.
//!
//! All of these failed on dev 4d500ed: no dialog placed its focus when it opened.

use super::*;

/// Whether the focus ring (`focus::FocusRing`) is drawn around `rect` in the last frame.
fn ring_around(kit: &Kit, rect: egui::Rect) -> bool {
    let ring = rect.expand(2.0);
    kit.output().shapes.iter().any(|c| {
        matches!(&c.shape, egui::Shape::Rect(r)
            if r.stroke.width == 2.0
                && (r.rect.min - ring.min).length() < 0.5
                && (r.rect.max - ring.max).length() < 0.5)
    })
}

/// Whether some label's words contain `part` (the demo core's answer to an action it cannot do).
fn says_part(kit: &Kit, part: &str) -> bool {
    kit.query_all_by(|n| n.value().is_some_and(|v| v.contains(part)))
        .next()
        .is_some()
}

/// Types `text` into whatever has the focus, as the keyboard does.
fn type_keys(kit: &mut Kit, text: &str) {
    kit.event(egui::Event::Text(text.to_owned()));
    kit.run_steps(2);
}

/// The rect of the dialog's own button called `name` (drawn over the page: the last one).
fn last_button(kit: &Kit, name: &str) -> egui::Rect {
    kit.query_all_by_role_and_label(Role::Button, name)
        .last()
        .map(|b| b.rect())
        .unwrap_or_else(|| panic!("no «{name}»"))
}

/// An ordinary dialog without fields opens with the focus on its primary, and Enter does it:
/// «Copy» of a private address.
#[test]
fn an_ordinary_dialog_without_fields_opens_on_its_primary() {
    let (_turn, mut kit) = demo_app("pool-address", "en");
    wait_for(&mut kit, "the address dialog", |kit| {
        kit.query_all_by_label("Copy").next().is_some()
    });
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Copy"),
        "the focus on opening"
    );
    enter(&mut kit);
    wait_for(&mut kit, "«Copied» after Enter", |kit| {
        kit.query_all_by_label("Copied").next().is_some()
    });
}

/// An ordinary dialog with fields opens in its first field, so typing goes there at once, and
/// Enter in it presses the primary: the wallet's new name is saved.
#[test]
fn an_ordinary_dialog_with_fields_opens_in_its_first_field() {
    let (_turn, mut kit) = demo_app("change-password", "en");
    kit.run_steps(2);
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Current password"),
        "«Change password»: the first of its three fields"
    );
    drop(kit);
    drop(_turn);

    let (_turn, mut kit) = demo_app("wallet-rename", "en");
    kit.run_steps(2);
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Title"),
        "the focus on opening"
    );
    type_keys(&mut kit, " 2");
    assert!(
        kit.state()
            .settings
            .rename
            .as_ref()
            .is_some_and(|f| f.name.ends_with(" 2")),
        "typing went into the field"
    );
    enter(&mut kit);
    let rename = kit.state().settings.rename.as_ref();
    assert!(
        rename.is_none_or(|f| f.waiting),
        "Enter in the field did not save the name"
    );
}

/// A destructive dialog without fields opens on «Cancel» with its ring shown at once (no Tab):
/// Enter closes it and nothing is done. The action itself goes only from itself: Tab to it and
/// Enter. «Sync again» restarts the app.
#[test]
fn a_destructive_dialog_without_fields_opens_on_cancel() {
    let (_turn, mut kit) = demo_app("resync", "en");
    wait_for(&mut kit, "the dialog", |kit| {
        kit.query_all_by_label("Sync again").next().is_some()
    });
    kit.run_steps(2);
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Cancel"),
        "the focus on opening"
    );
    let cancel = last_button(&kit, "Cancel");
    assert!(
        ring_around(&kit, cancel),
        "the ring on «Cancel» without a Tab"
    );
    enter(&mut kit);
    assert!(
        kit.state().settings.resync.is_none(),
        "Enter closes the dialog"
    );
    drop(kit);
    drop(_turn);

    let (_turn, mut kit) = demo_app("resync", "en");
    wait_for(&mut kit, "the dialog", |kit| {
        kit.query_all_by_label("Sync again").next().is_some()
    });
    kit.run_steps(2);
    tab(&mut kit, false);
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Sync again"),
        "Tab from «Cancel»"
    );
    enter(&mut kit);
    // The demo core has no chain to sync again and says so: the command went.
    wait_for(&mut kit, "the core's answer after Tab and Enter", |kit| {
        says_part(kit, "nothing to sync")
    });
}

/// The same when the dialog is opened by a click: Apps → «Disconnect» opens on «Cancel», and
/// Enter closes it without disabling the app's keys.
#[test]
fn disconnecting_an_app_opens_on_cancel() {
    let (_turn, mut kit) = demo_app("apps", "en");
    wait_for(&mut kit, "a connected app", |kit| {
        kit.query_all_by_label("Disconnect").next().is_some()
    });
    kit.query_all_by_role_and_label(Role::Button, "Disconnect")
        .next()
        .expect("«Disconnect»")
        .click();
    kit.run_steps(3);
    assert!(
        kit.state().requests.disconnecting.is_some(),
        "the dialog is open"
    );
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Cancel"),
        "the focus on opening"
    );
    assert!(
        ring_around(&kit, last_button(&kit, "Cancel")),
        "the ring on «Cancel» after a click too"
    );
    enter(&mut kit);
    let requests = &kit.state().requests;
    assert!(
        requests.disconnecting.is_none() && !requests.disconnect_waiting,
        "Enter closes the dialog and disconnects nothing"
    );
}

/// Enter cannot delete a wallet. The dialog opens in its first field; Enter there goes on to the
/// next field, and in the last one — with everything asked for filled in and «Delete from the
/// computer» enabled — it does nothing. Only Tab to the button and Enter on it deletes.
#[test]
fn enter_cannot_delete_a_wallet() {
    let (_turn, mut kit) = demo_app("delete", "en");
    kit.run_steps(2);
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Password"),
        "the focus on opening"
    );
    type_keys(&mut kit, "password");
    enter(&mut kit);
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Type “delete”"),
        "Enter in the password goes on to the next field"
    );
    type_keys(&mut kit, "delete");
    words(&kit, "The phrase is written down and checked").click();
    kit.run_steps(2);
    // Back into the last field by a click, as a person would, and Enter there.
    kit.get_by_role_and_label(Role::TextInput, "Type “delete”")
        .click();
    kit.run_steps(2);
    assert!(
        !button(&kit, "Delete from the computer")
            .accesskit_node()
            .is_disabled(),
        "everything asked is there"
    );
    for _ in 0..3 {
        enter(&mut kit);
    }
    // The demo core answers a deletion at once ("nothing to delete"): give it the time.
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        kit.step();
    }
    assert!(
        !says_part(&kit, "nothing to delete")
            && kit.query_all_by_label("Deleting…").next().is_none()
            && kit.state().settings.delete.is_some(),
        "Enter in a field deleted the wallet"
    );
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Type “delete”"),
        "Enter in the last field leaves the focus in it"
    );
    assert!(
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.wallets.iter().any(|w| w.open)),
        "the wallet is still there"
    );
    // The button itself still does it.
    tab_to(&mut kit, "Delete from the computer");
    enter(&mut kit);
    wait_for(&mut kit, "the deletion asked after Tab and Enter", |kit| {
        says_part(kit, "nothing to delete")
    });
}

/// Whether «Sign and send» went: the form waits for the core's answer, or has it.
fn sent(kit: &Kit) -> bool {
    let send = &kit.state().send;
    send.waiting > 0 || send.result.is_some()
}

/// Whether the dialog's enabled button `name` is drawn.
fn enabled_button(kit: &Kit, name: &str) -> bool {
    kit.query_all_by_role_and_label(Role::Button, name)
        .last()
        .is_some_and(|b| !b.accesskit_node().is_disabled())
}

/// The designer, 2026-11-06: a double Enter in «Check the payment» sends nothing. «Check and
/// send» by the keyboard and Enter again at once: the dialog stays, its focus on «Sign and send»
/// with the ring, and nothing is signed; nor by the key held down (its auto-repeat) after the
/// wait. A fresh Enter once the amount and the address are read signs.
#[test]
fn a_double_enter_in_check_the_payment_sends_nothing() {
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    wait_for(&mut kit, "the last look", |kit| {
        enabled_button(kit, "Sign and send")
    });
    // Back to the form, then from it by the keyboard, as a person in a hurry.
    kit.query_all_by_role_and_label(Role::Button, "Cancel")
        .last()
        .expect("«Cancel»")
        .click();
    kit.run_steps(3);
    assert!(!kit.state().send.confirming, "the form again");
    tab_to(&mut kit, "Check and send");
    enter(&mut kit);
    assert!(kit.state().send.confirming, "Enter opened the last look");
    wait_for(&mut kit, "«Sign and send» enabled", |kit| {
        enabled_button(kit, "Sign and send")
    });
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Sign and send"),
        "the focus on opening"
    );
    enter(&mut kit);
    assert!(
        !sent(&kit) && kit.state().send.confirming,
        "the second Enter at once signed"
    );
    assert!(
        ring_around(&kit, last_button(&kit, "Sign and send")),
        "the ring on «Sign and send»"
    );

    // Enter held down on «Check and send»: the press began before the dialog opened, and what
    // comes after the wait is only the key's auto-repeat.
    kit.query_all_by_role_and_label(Role::Button, "Cancel")
        .last()
        .expect("«Cancel»")
        .click();
    kit.run_steps(3);
    tab_to(&mut kit, "Check and send");
    let enter_key = |pressed: bool| egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Default::default(),
    };
    kit.event(enter_key(true));
    kit.run_steps(2);
    assert!(kit.state().send.confirming, "Enter opened the last look");
    wait_for(&mut kit, "«Sign and send» enabled", |kit| {
        enabled_button(kit, "Sign and send")
    });
    std::thread::sleep(crate::a11y::SPEND_KEYS_AFTER + std::time::Duration::from_millis(100));
    // egui marks a press of a key already down as its repeat.
    for _ in 0..3 {
        kit.event(enter_key(true));
        kit.run_steps(1);
    }
    kit.event(enter_key(false));
    kit.run_steps(2);
    assert!(
        !sent(&kit) && kit.state().send.confirming,
        "the key held down signed"
    );

    // A fresh Enter after the wait: the amount and the address are read.
    enter(&mut kit);
    wait_for(&mut kit, "the payment after a fresh Enter", sent);
}

/// The other way into a spend dialog: it opens with the focus on its primary and the ring shown
/// at once, though no key was pressed, and a click signs without waiting.
#[test]
fn check_the_payment_shows_its_ring_and_a_click_signs_at_once() {
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    wait_for(&mut kit, "the last look", |kit| {
        enabled_button(kit, "Sign and send")
    });
    kit.run_steps(2);
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Sign and send"),
        "the focus on opening"
    );
    assert!(
        ring_around(&kit, last_button(&kit, "Sign and send")),
        "the ring without a key"
    );
    kit.query_all_by_role_and_label(Role::Button, "Sign and send")
        .last()
        .expect("«Sign and send»")
        .click();
    kit.run_steps(2);
    wait_for(&mut kit, "the payment after a click", sent);
}

/// A sign-in is asked by a site, not by the person (rule b′): Enter in the password field
/// never connects, however ready the dialog is; only «Connect» itself does (FEAT-0023 round 4).
#[test]
fn enter_cannot_allow_an_app_to_sign_in() {
    use crate::screens::app_requests::Consent;
    let (_turn, mut kit) = demo_app("connect", "en");
    wait_for(&mut kit, "the sign-in request", |kit| {
        kit.state().requests.open.is_some()
    });
    kit.run_steps(3);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .click();
    kit.run_steps(2);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .type_text("any");
    kit.run_steps(2);
    assert!(enabled_button(&kit, "Connect"), "ready");
    for _ in 0..2 {
        enter(&mut kit);
    }
    assert!(
        kit.state().requests.consent == Consent::Asking && kit.state().requests.open.is_some(),
        "Enter allowed the sign-in"
    );
    kit.query_all_by_role_and_label(Role::Button, "Connect")
        .last()
        .expect("«Connect»")
        .click();
    wait_for(&mut kit, "the sign-in done", |kit| {
        kit.state().requests.connected.is_some()
    });
}
