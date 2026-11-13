//! BUG-42 (R-27 AC-T4): the last step of every spend has the "Wallet password" field when the
//! wallet asks for it for every send, and none when it does not.

use super::*;
use evolite_api::Command;
use std::time::{Duration, Instant};

/// The last step of each spend (the step whose button spends): its preset, and the button to
/// press when the preset stops one step before it. `shield` is not here: its preset shows the
/// proof part-way, when nothing is asked any more; `shield-core` is the same step.
const SPENDS: [(&str, &str, Option<&str>); 16] = [
    ("Send", "send-confirm", None),
    ("AddContact (Send)", "send-contact", None),
    ("TopUpIdentity", "top-up", None),
    ("ResumeTopUp", "top-up-stopped", None),
    ("ResumeTopUp (ChainLock)", "top-up-awaiting", None),
    ("CreateIdentity", "create-identity-confirm", None),
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
    ("DisconnectApp", "apps", Some("Disconnect")),
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
    let said = crate::components::wrong_text(Some(evolite_api::DoneWhat::Name));
    wait_for(&mut kit, "the wrong-password line", |kit| {
        has_text(kit, said)
    });
}

/// Types `password` into the last look's field and presses «Sign and send».
fn sign_with(kit: &mut Kit, password: &str) {
    let field = |kit: &Kit| {
        kit.query_all_by_role(Role::PasswordInput)
            .next()
            .expect("the password field")
            .click();
    };
    field(kit);
    kit.run_steps(2);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .type_text(password);
    kit.run_steps(2);
    wait_for(kit, "«Sign and send» enabled", |kit| {
        kit.query_all_by_role_and_label(Role::Button, "Sign and send")
            .last()
            .is_some_and(|b| !b.accesskit_node().is_disabled())
    });
    kit.query_all_by_role_and_label(Role::Button, "Sign and send")
        .last()
        .expect("«Sign and send»")
        .click();
    kit.run_steps(2);
}

const WRONG_SAID: &str = "Wrong password — the payment is not signed.";

/// BUG-92: from any source a wrong password is answered in the last look, at its field: the
/// dialog does not close, nothing went into the form's result, and the amount is still there.
/// Before the fix the dialog closed and the line stood below the window's edge.
#[test]
fn a_wrong_password_stays_in_the_last_look_with_the_error_at_the_field() {
    for source in 0..3 {
        let (turn, mut kit) = demo_app("send-confirm", "en");
        ask_every_time(&mut kit);
        kit.state_mut().send.source = source;
        kit.run_steps(3);
        sign_with(&mut kit, evolite_core::DEMO_WRONG_PASSWORD);
        wait_for(&mut kit, "the core's answer", |kit| {
            kit.state().send.waiting == 0
                && (kit.state().send.password_wrong || kit.state().send.result.is_some())
        });
        kit.run_steps(3);
        let send = &kit.state().send;
        assert!(send.confirming, "source {source}: the dialog closed");
        assert!(send.result.is_none(), "source {source}: {:?}", send.result);
        assert!(!send.amount.is_empty(), "source {source}: the amount gone");
        assert!(has_text(&kit, WRONG_SAID), "source {source}: the line");
        drop(kit);
        drop(turn);
    }
}

/// BUG-92: the password is corrected in place, the line goes as it changes, and the right one
/// closes the dialog; the demo refuses the payment then, on the form.
#[test]
fn a_corrected_password_goes_on_from_the_same_dialog() {
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    ask_every_time(&mut kit);
    sign_with(&mut kit, evolite_core::DEMO_WRONG_PASSWORD);
    wait_for(&mut kit, "the wrong-password line", |kit| {
        has_text(kit, WRONG_SAID)
    });
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .click();
    kit.run_steps(2);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .type_text("x");
    kit.run_steps(3);
    assert!(!has_text(&kit, WRONG_SAID), "the line goes as it changes");
    kit.query_all_by_role_and_label(Role::Button, "Sign and send")
        .last()
        .expect("«Sign and send»")
        .click();
    wait_for(&mut kit, "the dialog closed", |kit| {
        !kit.state().send.confirming
    });
    assert!(kit.state().send.password.is_empty(), "wiped once answered");
    assert!(
        kit.state().send.result.is_some(),
        "the core's answer is on the form"
    );
}

/// BUG-92: while the core has not answered the dialog says so and offers neither signing again
/// nor «Cancel»; a slow answer does not hold it for ever.
#[test]
fn the_last_look_waits_for_the_answer_and_not_for_ever() {
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    ask_every_time(&mut kit);
    kit.state_mut().send.signing = Some(Instant::now());
    kit.run_steps(3);
    assert!(kit.state().send.confirming, "held open");
    let sending = kit
        .query_all_by_role_and_label(Role::Button, "Sending…")
        .last()
        .expect("«Sending…»");
    assert!(sending.accesskit_node().is_disabled());
    assert_eq!(
        kit.query_all_by_role_and_label(Role::Button, "Cancel")
            .count(),
        1,
        "only the form's own «Cancel»"
    );
    kit.state_mut().send.signing = Some(Instant::now() - Duration::from_secs(60));
    kit.run_steps(3);
    let send = &kit.state().send;
    assert!(!send.confirming && send.signing.is_none(), "closed");
    assert!(send.password.is_empty(), "the password wiped");
}

/// BUG-92: with the dialog closed, a refusal is brought into view: in a low window the line
/// under the buttons is inside it. Before the fix it stood past the bottom edge.
#[test]
fn a_refused_payment_comes_into_view_on_the_form() {
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    kit.set_size(egui::vec2(1024.0, 480.0));
    kit.run_steps(3);
    wait_for(&mut kit, "«Sign and send» enabled", |kit| {
        kit.query_all_by_role_and_label(Role::Button, "Sign and send")
            .last()
            .is_some_and(|b| !b.accesskit_node().is_disabled())
    });
    kit.query_all_by_role_and_label(Role::Button, "Sign and send")
        .last()
        .expect("«Sign and send»")
        .click();
    wait_for(&mut kit, "the refusal", |kit| {
        kit.state().send.result.is_some()
    });
    kit.run_steps(30);
    let line = kit
        .query_by(|n| {
            n.value()
                .is_some_and(|v| v.contains("The payment went nowhere"))
        })
        .expect("the refusal on the form")
        .rect();
    assert!(line.bottom() <= 480.0, "{line:?} past the window");
}

/// R-10 r1 must 2: every payment signed gets the next number, and the core's answer says it back.
#[test]
fn each_payment_signed_has_its_own_number() {
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    ask_every_time(&mut kit);
    assert_eq!(kit.state().send.last_payment, 0);
    sign_with(&mut kit, "right");
    assert_eq!(kit.state().send.last_payment, 1);
    wait_for(&mut kit, "the answer to that number", |kit| {
        kit.state().send.waiting == 0
    });
}

/// BUG-92 r1, must 1: with an earlier payment
/// still unanswered the dialog does not wait: it closes at once, as before BUG-92, and that
/// payment's «wrong password», arriving now, goes to the form. Waiting would have left «not
/// signed» and «Sign and send» on screen with this payment already queued: a double send.
#[test]
fn with_another_payment_unanswered_the_last_look_does_not_wait() {
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    ask_every_time(&mut kit);
    kit.state_mut().send.waiting = 1;
    sign_with(&mut kit, "right");
    {
        let send = &kit.state().send;
        assert!(!send.confirming, "the dialog closed at once");
        assert!(send.signing.is_none(), "nothing is waited for");
        assert!(send.password.is_empty(), "the password wiped");
    }
    kit.state_mut()
        .send
        .on_sent(0, &Err(evolite_api::CoreError::WrongPassword));
    kit.run_steps(3);
    assert!(!kit.state().send.confirming, "no dialog comes back");
    assert!(!kit.state().send.password_wrong, "nothing said at a field");
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Sign and send")
            .next()
            .is_none(),
        "nothing offers to sign again"
    );
}

/// The payment `kit` is about to sign, already in the history as «Sending».
fn with_the_payment_in_history(kit: &mut Kit) {
    const ROW: u64 = 0x9292_9292;
    let amount = crate::format::parse_amount(&kit.state().send.amount).expect("an amount");
    show_wallet_edited(
        kit,
        3,
        "the wallet edited",
        |wallet| {
            if !wallet.activity.iter().any(|a| a.id == ROW) {
                let mut row = wallet.activity[0].clone();
                row.id = ROW;
                row.kind = evolite_api::ActivityKind::Sent {
                    to: "@alice".into(),
                };
                row.at = chrono::Utc::now().timestamp();
                row.amount = -amount;
                row.proof = evolite_api::Proof::Sending;
                wallet.activity.insert(0, row);
            }
        },
        |_| true,
    );
}

/// BUG-92 r1, should 1: a real repeat is warned of in the last look; the payment being signed,
/// whose own «Sending» row is already in the history, is not a repeat of itself.
#[test]
fn the_last_look_does_not_call_its_own_payment_a_repeat() {
    const REPEAT: &str = "is already in the history";
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    with_the_payment_in_history(&mut kit);
    assert!(has_words(&kit, REPEAT), "a real repeat is warned of");
    kit.state_mut().send.signing = Some(Instant::now());
    with_the_payment_in_history(&mut kit);
    assert!(!has_words(&kit, REPEAT), "its own row is no repeat");
}

/// Whether «Check and send» is drawn enabled.
fn can_check_and_send(kit: &Kit) -> bool {
    !button(kit, "Check and send").accesskit_node().is_disabled()
}

/// BUG-92 r2, must 2: payment A unanswered, B signed and queued behind it, then A's «wrong
/// password». The form must not offer to send again —
/// no amount back, «Check and send» off — and A's refusal is still said, in sight. With «Max»
/// too, which would fill the amount again.
#[test]
fn a_refusal_with_another_payment_out_does_not_offer_to_send_again() {
    for max in [false, true] {
        let (turn, mut kit) = demo_app("send", "en");
        wait_for(&mut kit, "the form ready to send", can_check_and_send);
        {
            let send = &mut kit.state_mut().send;
            // As the last look leaves it after signing B, with A still out.
            send.signed = std::mem::take(&mut send.amount);
            send.suggested = None;
            send.max = max;
            send.last_payment = 2;
            send.waiting = 2;
        }
        kit.run_steps(3);
        kit.state_mut()
            .send
            .on_sent(1, &Err(evolite_api::CoreError::WrongPassword));
        kit.run_steps(30);
        assert!(!can_check_and_send(&kit), "max {max}: the button is live");
        let send = &kit.state().send;
        assert!(send.amount.is_empty(), "max {max}: «{}»", send.amount);
        assert_eq!(send.waiting, 1, "max {max}");
        assert!(!send.signed.is_empty(), "max {max}: B's amount kept");
        let line = kit
            .query_all_by(|n| n.value().as_deref() == Some(WRONG_SAID))
            .next()
            .unwrap_or_else(|| panic!("max {max}: A's refusal is not said"))
            .rect();
        assert!(
            line.bottom() <= 768.0,
            "max {max}: {line:?} past the window"
        );
        drop(kit);
        drop(turn);
    }
}
