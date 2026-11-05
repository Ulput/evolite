//! BUG-42 (R-27 AC-T4): the last step of every spend has the "Wallet password" field when the
//! wallet asks for it for every send, and none when it does not.

use super::*;
use evolite_api::Command;

/// The last step of each spend (the step whose button spends): its preset, and the button to
/// press when the preset stops one step before it. `shield` is not here: its preset shows the
/// proof part-way, when nothing is asked any more; `shield-core` is the same step.
const SPENDS: [(&str, &str, Option<&str>); 17] = [
    ("Send", "send-confirm", None),
    ("AddContact (Send)", "send-contact", None),
    ("TopUpIdentity", "top-up", None),
    ("ResumeTopUp", "top-up-stopped", None),
    ("ResumeTopUp (ChainLock)", "top-up-awaiting", None),
    ("CreateIdentity", "create-identity", None),
    (
        "ResumeIdentity",
        "identity-unfinished",
        Some("Continue creating"),
    ),
    ("AddName", "add-name", None),
    ("Shield", "shield-core", None),
    ("Unshield", "unshield", None),
    ("TopUpFromPool", "unshield-identity", None),
    ("AddContact", "add-contact", None),
    ("AcceptContact", "contacts", Some("Accept")),
    ("SaveProfile", "edit-profile", None),
    ("ApproveApp (key)", "connect", None),
    ("ApproveApp (document)", "sign", None),
    ("GiftNft", "gift", None),
];

fn asks(kit: &Kit) -> bool {
    has_text(kit, "Wallet password")
}

/// The wallet's "password for every send", turned on in the demo core as Settings → Security
/// turns it on.
fn ask_every_time(kit: &mut Kit) {
    kit.state().core.send(Command::SetPasswordEachSend {
        on: true,
        password: None,
    });
    wait_for(kit, "the setting in the snapshot", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .is_some_and(|w| w.password_each_send)
    });
    kit.run_steps(3);
}

/// The spend's last step: the preset, then the button it stops before.
fn last_step(preset: &str, press: Option<&str>, on: bool) -> (MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app(preset, "en");
    if on {
        ask_every_time(&mut kit);
    }
    kit.run_steps(3);
    if let Some(name) = press {
        kit.query_all_by_role_and_label(Role::Button, name)
            .next()
            .unwrap_or_else(|| panic!("{preset}: no «{name}»"))
            .click();
        kit.run_steps(4);
    }
    (turn, kit)
}

#[test]
fn every_spend_asks_for_the_password_only_when_the_wallet_wants_it() {
    let mut wrong = Vec::new();
    for (spend, preset, press) in SPENDS {
        let (turn, kit) = last_step(preset, press, false);
        if asks(&kit) {
            wrong.push(format!("{spend} ({preset}): asked with the setting off"));
        }
        drop(kit);
        drop(turn);
        let (_turn, kit) = last_step(preset, press, true);
        if !asks(&kit) {
            wrong.push(format!("{spend} ({preset}): not asked with the setting on"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A wrong password in a spend's last step: nothing happens, and the line under the field
/// names the operation (the analyst: «Неверный пароль — <операция> не выполнена.»).
#[test]
fn a_wrong_password_says_which_operation_did_not_happen() {
    let (_turn, mut kit) = last_step("add-name", None, true);
    // The field has no name yet (BUG-41): the only password field of the step.
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .click();
    kit.run_steps(2);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .type_text(evolite_core::DEMO_WRONG_PASSWORD);
    kit.run_steps(2);
    kit.query_all_by_role_and_label(Role::Button, "Register")
        .next()
        .expect("no «Register»")
        .click();
    let said = crate::components::wrong_text(Some("name"));
    wait_for(&mut kit, "the wrong-password line", |kit| {
        has_text(kit, said)
    });
}
