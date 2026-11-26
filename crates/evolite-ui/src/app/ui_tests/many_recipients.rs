//! FEAT-0056: one Core payment to several recipients on the demo core — «+ Recipient» only
//! with the advanced features, the rows and «All», «Paste a list», the last look with one
//! password, and the one line in History.

use super::*;
use crate::screens::Screen;
use crate::screens::send::many::Row;

const A: &str = "yXdxCB2vAX6wPHeaQqyB6XJT9eSZGJx3qB";
/// Tall enough for the whole form: a click lands only on what is on screen.
const TALL: egui::Vec2 = egui::vec2(1024.0, 1800.0);
const B: &str = "yMXp7gU4uQ4mVQ4V1V3H6cPCXxWfNrsW3d";

fn send_screen(advanced: bool, size: egui::Vec2) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("send", "en");
    kit.set_size(size);
    kit.state_mut().settings.set("advanced", advanced);
    kit.state_mut().send.recipient = A.into();
    kit.state_mut().send.amount = "0.1".into();
    kit.run_steps(4);
    (turn, kit)
}

/// An established contact of the demo, as `@name`.
fn contact(kit: &Kit) -> String {
    let w = kit
        .state()
        .snapshot
        .as_ref()
        .unwrap()
        .wallet
        .as_ref()
        .unwrap();
    let c = w
        .contacts
        .iter()
        .find(|c| c.state == evolite_api::ContactState::Established && c.name.is_some())
        .expect("an established contact");
    format!("@{}", c.name.as_deref().unwrap())
}

fn rows(kit: &mut Kit, rows: Vec<Row>) {
    kit.state_mut().send.many.as_mut().expect("rows").rows = rows;
    kit.run_steps(4);
}

fn row(to: &str, amount: &str) -> Row {
    Row {
        to: to.into(),
        amount: amount.into(),
        all: false,
    }
}

fn core(kit: &Kit) -> evolite_api::Duffs {
    kit.state()
        .snapshot
        .as_ref()
        .unwrap()
        .wallet
        .as_ref()
        .unwrap()
        .balances
        .core
}

fn enabled(kit: &Kit, name: &str) -> bool {
    kit.query_all_by_role_and_label(Role::Button, name)
        .next()
        .is_some_and(|b| !b.accesskit_node().is_disabled())
}

/// A8 (R-9, Н2): «+ Recipient» only with the advanced features on; off, the form is as it was.
/// Mutant: `allowed` always true → red.
#[test]
fn another_recipient_is_offered_only_to_advanced_users() {
    let (_turn, kit) = send_screen(false, egui::vec2(1024.0, 768.0));
    assert!(
        kit.query_by_label("+ Recipient").is_none(),
        "off: one recipient"
    );
    drop(kit);
    drop(_turn);
    let (_turn, kit) = send_screen(true, TALL);
    assert!(kit.query_by_label("+ Recipient").is_some());
}

/// A1, A3, A7: two addresses and a contact — the last look asks each recipient's address (the
/// contact's from its key), nothing is spent before «Sign and send to 3 recipients»; then one
/// line «Sent to 3 recipients» in History with all three, and Core less the total and the fee.
#[test]
fn three_recipients_are_one_payment_and_one_line() {
    let (_turn, mut kit) = send_screen(true, TALL);
    button(&kit, "+ Recipient").click();
    kit.run_steps(3);
    let name = contact(&kit);
    rows(
        &mut kit,
        vec![row(A, "0.1"), row(&name, "0.2"), row(B, "0.3")],
    );
    assert!(has_words(&kit, "To recipients"));
    let before = core(&kit);
    button(&kit, "Check and send").click();
    wait_for(&mut kit, "the recipients checked", |k| {
        k.state()
            .send
            .many
            .as_ref()
            .is_some_and(|m| m.previews.len() == 3 && m.previews.values().all(Result::is_ok))
    });
    kit.run_steps(3);
    assert_eq!(core(&kit), before, "A3: nothing spent by the look");
    if kit.query_by_role(Role::PasswordInput).is_some() {
        kit.get_by_role(Role::PasswordInput).click();
        kit.run_steps(2);
        kit.get_by_role(Role::PasswordInput).type_text("demo");
        kit.run_steps(3);
    }
    if kit
        .query_by_label("I checked the amount and where it goes")
        .is_some()
    {
        kit.get_by_label("I checked the amount and where it goes")
            .click();
        kit.run_steps(2);
    }
    button(&kit, "Sign and send to 3 recipients").click();
    wait_for(&mut kit, "the answer", |k| {
        matches!(k.state().send.result, Some(Ok(_)))
    });
    assert!(
        kit.state().send.many.is_none(),
        "the rows went with the payment"
    );
    let w = kit
        .state()
        .snapshot
        .as_ref()
        .unwrap()
        .wallet
        .as_ref()
        .unwrap();
    let line = &w.activity[0];
    assert_eq!(crate::text::activity_title(line), "Sent to 3 recipients");
    assert_eq!(crate::text::sent_to(line).len(), 3);
    assert_eq!(core(&kit), before + line.amount);
}

/// A2 (UC-2): «All» on the second row gives it the rest after the others and the fee; the other
/// rows' «All» is off while it is on. Mutant: «All» allowed twice → red.
#[test]
fn all_goes_to_one_recipient_only() {
    let (_turn, mut kit) = send_screen(true, TALL);
    button(&kit, "+ Recipient").click();
    kit.run_steps(3);
    let mut second = row(B, "");
    second.all = true;
    rows(&mut kit, vec![row(A, "0.1"), second]);
    let buttons: Vec<bool> = kit
        .query_all_by_role_and_label(Role::Button, "All (the rest)")
        .map(|b| b.accesskit_node().is_disabled())
        .collect();
    assert_eq!(
        buttons,
        vec![true, false],
        "only the row with «All» may take it off"
    );
    let w = kit
        .state()
        .snapshot
        .as_ref()
        .unwrap()
        .wallet
        .as_ref()
        .unwrap();
    let (core, fees) = (w.balances.core, w.fees.clone());
    let rest = core - 10_000_000 - (fees.core_payment + evolite_api::extra_outputs_fee(1));
    assert!(
        has_words(&kit, &crate::format::amount(rest)),
        "the rest, shown"
    );
}

/// A4 (UC-3): a list of five lines, one wrong — four rows more, and the wrong one by its
/// number; A5: at 50 rows «+ Recipient» is off.
#[test]
fn a_pasted_list_adds_rows_and_fifty_is_the_limit() {
    let (_turn, mut kit) = send_screen(true, TALL);
    button(&kit, "+ Recipient").click();
    kit.run_steps(3);
    rows(&mut kit, vec![row(A, "0.1"), row("", "")]);
    button(&kit, "Paste a list").click();
    kit.run_steps(3);
    if let Some(many) = kit.state_mut().send.many.as_mut() {
        many.paste = Some(format!(
            "{B}, 0.01\n{A}; 0.02\n{B}\t0.03\n{A}, lots\n{B}, 0.04"
        ));
    }
    kit.run_steps(2);
    button(&kit, "Parse").click();
    kit.run_steps(4);
    let many = kit.state().send.many.as_ref().unwrap();
    assert_eq!(many.rows.len(), 5, "the first and four from the list");
    assert!(has_words(&kit, "Line 4: "));
    rows(&mut kit, (0..50).map(|_| row(A, "0.001")).collect());
    assert!(!enabled(&kit, "+ Recipient"), "no 51st");
}

/// §8d (A9): at 1024 × 768 and 800 × 440 the rows, «+ Recipient» and «Check and send» are
/// within the window's width, and the last look's sign button is on screen.
#[test]
fn the_rows_and_the_last_look_fit_small_windows() {
    for size in [egui::vec2(1024.0, 768.0), egui::vec2(800.0, 440.0)] {
        let (_turn, mut kit) = send_screen(true, size);
        kit.state_mut().send.add_recipient();
        kit.run_steps(3);
        rows(&mut kit, vec![row(A, "0.1"), row(B, "0.2"), row(A, "0.3")]);
        for name in ["+ Recipient", "Check and send"] {
            let r = button(&kit, name).rect();
            assert!(r.right() <= size.x, "{name} at {size:?}: {r:?}");
        }
        assert!(has_words(&kit, &format!("{} is in the list twice", A)));
        // The look as «Check and send» opens it (that button may be below the window).
        for to in [A, B] {
            kit.state_mut()
                .core
                .send(evolite_api::Command::PreviewPayment { to: to.into() });
        }
        kit.state_mut().send.confirming = true;
        wait_for(&mut kit, "the look", |k| {
            k.state()
                .send
                .many
                .as_ref()
                .is_some_and(|m| m.previews.len() == 2)
        });
        kit.run_steps(3);
        let sign = button(&kit, "Sign and send to 3 recipients").rect();
        assert!(
            sign.right() <= size.x && sign.bottom() <= size.y,
            "the sign button at {size:?}: {sign:?}"
        );
        assert!(kit.state().screen == Screen::Send);
    }
}

const PRIVACY: &str = "Every recipient sees the whole transaction";

/// FEAT-0056 R-12, AC-A10: a contact and an address — the muted line over the totals; the contact
/// taken out, or a contact alone, no line; at 1024 and 800 × 440 the line and «Check and send»
/// stay within the window. Mutant: the line for any list → red.
#[test]
fn a_payment_with_a_contact_among_several_says_what_recipients_see() {
    for size in [egui::vec2(1024.0, 768.0), egui::vec2(800.0, 440.0)] {
        let (_turn, mut kit) = send_screen(true, size);
        kit.state_mut().send.add_recipient();
        kit.run_steps(3);
        let name = contact(&kit);
        rows(&mut kit, vec![row(A, "0.1"), row(&name, "0.2")]);
        assert!(
            has_words(&kit, PRIVACY),
            "a contact and an address at {size:?}"
        );
        let line = kit
            .query_all_by(|n| n.value().is_some_and(|v| v.starts_with(PRIVACY)))
            .next()
            .expect("the line")
            .rect();
        let check = button(&kit, "Check and send").rect();
        assert!(
            line.right() <= size.x && check.right() <= size.x,
            "{line:?} {check:?} at {size:?}"
        );
        rows(&mut kit, vec![row(A, "0.1"), row(B, "0.2")]);
        assert!(!has_words(&kit, PRIVACY), "addresses only");
        rows(&mut kit, vec![row(&name, "0.2"), row("", "")]);
        assert!(!has_words(&kit, PRIVACY), "one contact alone");
    }
}
