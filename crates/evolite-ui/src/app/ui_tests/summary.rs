//! BUG-249: a dialog that asks agreement pins its summary (the amount, the fee, the total, the
//! ticks) with the footer; in a window too low for that and a middle of a few lines, everything
//! under the header scrolls as one, and the primary never acts on a summary not seen.

use super::*;
use crate::components::dialog_seen;

const SUMMARY_LINE: &str = "A payment cannot be undone.";

/// Whether «Sign and send» went: the form waits for the core's answer, or has it (the demo core
/// answers at once).
fn sent(kit: &Kit) -> bool {
    let send = &kit.state().send;
    send.waiting > 0 || send.result.is_some()
}

/// «Check the payment» with «Password for every send» on, opened in a window of `size` (closed
/// and opened again there: what a larger window showed does not count as seen).
fn review_at(size: [f32; 2]) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("send-confirm-tall", "en");
    settle(&mut kit);
    kit.set_size(egui::vec2(size[0], size[1]));
    kit.state_mut().send.confirming = false;
    kit.run_steps(2);
    kit.state_mut().send.confirming = true;
    show_wallet_edited(
        &mut kit,
        4,
        "the review at its size",
        |w| w.password_each_send = true,
        |kit| dialog_seen(&kit.ctx).is_some(),
    );
    (turn, kit)
}

/// The architect's condition 1: in the one scroll, Enter in the password with the summary not
/// seen sends nothing and brings the summary into sight; the next Enter sends. A window lower
/// than the app allows makes sure the summary and the field are not both in sight.
#[test]
fn enter_with_the_summary_unseen_shows_it_and_sends_nothing() {
    let (_turn, mut kit) = review_at([800.0, 300.0]);
    assert!(dialog_seen(&kit.ctx).unwrap().one_scroll, "the one scroll");
    // The password typed, the focus in its field (as after typing it).
    kit.state_mut().send.password = evolite_api::Zeroizing::new("demo".to_owned());
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .focus();
    kit.run_steps(3);
    assert!(
        kit.query_by_label("Sign and send")
            .is_some_and(|b| !b.accesskit_node().is_disabled()),
        "«Sign and send» is ready"
    );
    let (_, seen) = dialog_seen(&kit.ctx).unwrap().summary.expect("a summary");
    assert!(!seen, "the summary has not been in sight");
    kit.key_press(egui::Key::Enter);
    kit.run_steps(4);
    assert_eq!(
        kit.state().send.waiting,
        0,
        "nothing sent on the first Enter"
    );
    let dialog = dialog_seen(&kit.ctx).unwrap();
    let (rect, seen) = dialog.summary.expect("a summary");
    let view = dialog.scrolled.expect("it scrolls");
    assert!(seen, "the summary is brought into sight");
    assert!(
        view.contains(rect.left_top() + egui::vec2(1.0, 1.0)),
        "from its top: {rect:?} in {view:?}"
    );
    // Back in the field: the next Enter signs — once the review has been open long enough for
    // its keys (`a11y::SPEND_KEYS_AFTER`, the clock's own time).
    std::thread::sleep(crate::a11y::SPEND_KEYS_AFTER);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .unwrap()
        .focus();
    kit.run_steps(2);
    kit.key_press(egui::Key::Enter);
    kit.run_steps(4);
    assert!(sent(&kit), "the second Enter sends");
}

/// The architect's condition 3: a screen reader reads the summary (the amount, the fee, that it
/// cannot be undone) before «Sign and send», whatever the window.
#[test]
fn the_summary_is_read_before_the_button() {
    for size in [[800.0, 440.0], [1024.0, 768.0]] {
        let (_turn, kit) = review_at(size);
        let order: Vec<String> = kit
            .query_all_by(|_| true)
            .filter_map(|n| {
                let node = n.accesskit_node();
                node.label().or(node.value())
            })
            .collect();
        let at = |t: &str| order.iter().position(|o| o == t);
        let (summary, button) = (at(SUMMARY_LINE), at("Sign and send"));
        assert!(
            summary.is_some() && button.is_some() && summary < button,
            "{size:?}: the summary {summary:?}, the button {button:?}"
        );
    }
}

/// The architect's condition 5 (BUG-122 at an ordinary size): at 1024×768 the footer stays put
/// and the summary with it; only the middle scrolls, if anything does.
#[test]
fn at_an_ordinary_size_the_footer_and_summary_stay_put() {
    let (_turn, kit) = review_at([1024.0, 768.0]);
    let seen = dialog_seen(&kit.ctx).unwrap();
    assert!(!seen.one_scroll, "the footer scrolls at 1024×768");
    let window = kit.ctx.content_rect();
    let footer = seen.footer.expect("a footer");
    assert!(window.contains_rect(footer), "{footer:?} out of {window:?}");
    let (summary, in_sight) = seen.summary.expect("a summary");
    assert!(
        in_sight && window.contains_rect(summary),
        "the summary is out of the window"
    );
    if let Some(view) = seen.scrolled {
        assert!(summary.top() >= view.bottom() - 0.5, "the summary scrolls");
    }
}

/// Review r1 Must: a Danger button that signs («Disconnect» an app) is held as a primary is: in
/// the one scroll its first click sends nothing and brings the summary into sight; the second
/// disconnects.
#[test]
fn disconnect_with_the_summary_unseen_shows_it_and_sends_nothing() {
    let (_turn, mut kit) = demo_app("apps-disconnect", "en");
    settle(&mut kit);
    kit.set_size(egui::vec2(800.0, 300.0));
    // Opened again in the low window: what a larger one showed does not count as seen.
    let asked = kit.state().requests.disconnecting.clone();
    kit.state_mut().requests.disconnecting = None;
    kit.run_steps(2);
    kit.state_mut().requests.disconnecting = asked;
    wait_for(&mut kit, "the dialog", |kit| {
        dialog_seen(&kit.ctx).is_some()
    });
    kit.run_steps(4);
    let dialog = dialog_seen(&kit.ctx).unwrap();
    assert!(dialog.one_scroll, "the one scroll");
    let (_, seen) = dialog.summary.expect("a summary");
    assert!(!seen, "the summary has not been in sight");
    let gone = |kit: &Kit| {
        let r = &kit.state().requests;
        r.disconnect_flow.waiting() || r.disconnect_result.is_some() || r.disconnecting.is_none()
    };
    let disconnect = |kit: &Kit| {
        kit.query_all_by_role_and_label(Role::Button, "Disconnect")
            .last()
            .expect("«Disconnect»")
            .rect()
    };
    // Scrolled to the button (its click is what is held, not a click out of sight).
    kit.query_all_by_role_and_label(Role::Button, "Disconnect")
        .last()
        .unwrap()
        .scroll_to_me();
    kit.run_steps(3);
    let at = disconnect(&kit).center();
    kit.input_mut().events.push(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    kit.input_mut().events.push(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    kit.run_steps(4);
    assert!(!gone(&kit), "nothing sent on the first click");
    let (_, seen) = dialog_seen(&kit.ctx).unwrap().summary.expect("a summary");
    assert!(seen, "the summary is brought into sight");
    // Back to the button: the next click disconnects.
    kit.query_all_by_role_and_label(Role::Button, "Disconnect")
        .last()
        .unwrap()
        .scroll_to_me();
    kit.run_steps(3);
    kit.query_all_by_role_and_label(Role::Button, "Disconnect")
        .last()
        .unwrap()
        .click();
    kit.run_steps(4);
    assert!(gone(&kit), "the second click disconnects");
}
