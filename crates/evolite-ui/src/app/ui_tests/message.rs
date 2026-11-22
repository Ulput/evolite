//! FEAT-0044: «Message» — sign with this wallet's address, check a signature (the demo signs with
//! dashcore's published vector, never a wallet's key).

use super::*;
use crate::screens::message::{Tab, is_open, type_in};
use evolite_api::Command;

fn advanced_on(kit: &mut Kit) {
    kit.state_mut().settings.set("advanced", true);
    kit.state()
        .core
        .send(Command::AdvancedFeatures { on: true });
    wait_for(kit, "the switch in the core", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.advanced_features)
    });
}

/// UC-5, AC-A11: with the advanced switch off neither entry is there; on, «Receive» has «Sign a
/// message» and it opens the window.
#[test]
fn the_message_window_is_behind_the_advanced_switch() {
    let (_turn, mut kit) = demo_app("receive", "en");
    kit.set_size(egui::vec2(1024.0, 1100.0));
    kit.run_steps(3);
    assert!(!has_text(&kit, "Sign a message"));
    advanced_on(&mut kit);
    wait_for(&mut kit, "the entry", |kit| has_text(kit, "Sign a message"));
    button(&kit, "Sign a message").click();
    kit.run_steps(3);
    assert!(is_open(&kit.ctx));
    assert!(
        has_words(&kit, "Sign only text you understand"),
        "R-5 first"
    );
}

/// UC-1, AC-A2, AC-A4: no signature without the password; a wrong one says so; the right one
/// gives the signature to copy; another address than this wallet's is refused.
#[test]
fn a_message_is_signed_after_the_password() {
    let (_turn, mut kit) = demo_app("receive", "en");
    kit.set_size(egui::vec2(1024.0, 1100.0));
    advanced_on(&mut kit);
    wait_for(&mut kit, "the entry", |kit| has_text(kit, "Sign a message"));
    button(&kit, "Sign a message").click();
    kit.run_steps(3);
    let ctx = kit.ctx.clone();
    type_in(&ctx, Tab::Sign, (None, Some("I own this address"), None));
    kit.run_steps(2);
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Sign")
            .last()
            .is_some_and(|b| b.accesskit_node().is_disabled()),
        "no password, no signing"
    );
    type_in(
        &ctx,
        Tab::Sign,
        (None, None, Some(evolite_core::DEMO_WRONG_PASSWORD)),
    );
    kit.run_steps(2);
    kit.query_all_by_role_and_label(Role::Button, "Sign")
        .last()
        .unwrap()
        .click();
    wait_for(&mut kit, "the wrong password", |kit| {
        has_words(kit, "Wrong password")
    });
    type_in(&ctx, Tab::Sign, (None, None, Some("main-wallet-26")));
    kit.run_steps(2);
    kit.query_all_by_role_and_label(Role::Button, "Sign")
        .last()
        .unwrap()
        .click();
    wait_for(&mut kit, "the signature", |kit| {
        has_words(kit, evolite_core::message::DEMO_SIGNATURE)
    });
    assert!(has_text(&kit, "Copy the signature"));
    // Another address: refused before the password.
    type_in(
        &ctx,
        Tab::Sign,
        (
            Some("yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5a9f2c"),
            Some("x"),
            Some("main-wallet-26"),
        ),
    );
    kit.run_steps(2);
    kit.query_all_by_role_and_label(Role::Button, "Sign")
        .last()
        .unwrap()
        .click();
    wait_for(&mut kit, "the refusal", |kit| {
        has_words(kit, "This is not an address of this wallet")
    });
}

/// UC-3, AC-A3, AC-A7 (R-8): a check answers as it is typed, without a password — valid; with a
/// space more in the message, «does not match» and the line about matching to the character;
/// and what is no signature says so.
#[test]
fn a_signature_is_checked_as_it_is_typed() {
    let (_turn, mut kit) = demo_app("receive", "en");
    kit.set_size(egui::vec2(1024.0, 1100.0));
    advanced_on(&mut kit);
    wait_for(&mut kit, "the entry", |kit| has_text(kit, "Sign a message"));
    button(&kit, "Sign a message").click();
    kit.run_steps(3);
    let ctx = kit.ctx.clone();
    let address = "XjM5kXg6x5BqvxSTL6zWwW4mN8TfNGpxUA";
    type_in(
        &ctx,
        Tab::Check,
        (
            Some(address),
            Some(evolite_core::message::DEMO_MESSAGE),
            Some(evolite_core::message::DEMO_SIGNATURE),
        ),
    );
    kit.run_steps(3);
    assert!(has_words(&kit, "The signature is valid"));
    type_in(
        &ctx,
        Tab::Check,
        (
            None,
            Some(&format!("{} ", evolite_core::message::DEMO_MESSAGE)),
            None,
        ),
    );
    kit.run_steps(3);
    assert!(has_words(&kit, "The signature does not match this address"));
    assert!(has_words(&kit, "spaces and line breaks included"));
    type_in(&ctx, Tab::Check, (None, None, Some("hello")));
    kit.run_steps(3);
    assert!(has_words(&kit, "This is not a signature"));
    // AC-A10: the lock closes it (what the app does on a lock).
    crate::screens::message::close(&ctx);
    kit.run_steps(2);
    assert!(!is_open(&kit.ctx));
}
