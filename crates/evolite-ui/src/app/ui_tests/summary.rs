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

/// FEAT-0038 W0–W4 r2 Must 2: «Check the collection» signs 10+ DASH, so its summary (the fee,
/// the credits after, that it cannot be undone, the large spend's tick) is pinned with the
/// footer. Its middle is a few lines: at the smallest window the summary is in sight as soon as
/// it opens; below it (no window the app allows, `theme::MIN_WINDOW`) it may scroll, and then the
/// primary is held until it is in sight (BUG-249 r4: «in sight now») — either way the first
/// Enter never acts on it unseen — and a screen reader reads it before «Sign and create».
#[test]
fn the_collection_review_pins_its_summary() {
    for size in [[800.0, 440.0], [800.0, 200.0]] {
        let (_turn, mut kit) = demo_app("workshop-review", "en");
        kit.set_size(egui::vec2(size[0], size[1]));
        wait_for(&mut kit, "the review", |kit| {
            dialog_seen(&kit.ctx).is_some() && has_text(kit, "Check the collection")
        });
        kit.run_steps(4);
        let dialog = dialog_seen(&kit.ctx).unwrap();
        let (_, seen) = dialog.summary.expect("a pinned summary");
        if size[1] >= crate::theme::MIN_WINDOW[1] {
            assert!(seen, "{size:?}: the summary is in sight when it opens");
        } else {
            assert!(
                seen || dialog.one_scroll,
                "{size:?}: in sight, or scrolling with the primary held until it is"
            );
        }
        let order: Vec<String> = kit
            .query_all_by(|_| true)
            .filter_map(|n| {
                let node = n.accesskit_node();
                node.label().or(node.value())
            })
            .collect();
        let at = |t: &str| order.iter().position(|o| o.starts_with(t));
        let (fee, after, button) = (
            at("Registration"),
            at("Credits after"),
            at("Sign and create"),
        );
        assert!(
            fee.is_some() && after.is_some() && button.is_some() && fee < button && after < button,
            "{size:?}: {fee:?} {after:?} before {button:?}"
        );
    }
}

/// BUG-249 r4, QA on the real core (dev 284687bf, (c)): the identity top-up at 800 × 440 — the
/// summary scrolled into sight by the wheel, then out of it again — and the first «Top up by …»
/// sent at once: «seen once» let it go. The primary acts only while the summary is in sight
/// now; else its press brings the summary in, and the next press acts.
#[test]
fn the_top_up_with_its_summary_scrolled_away_first_brings_it_in() {
    let (_turn, mut kit) = demo_app("top-up", "en");
    settle(&mut kit);
    kit.set_size(egui::vec2(800.0, 440.0));
    show_wallet_edited(
        &mut kit,
        6,
        "the top-up at 800 × 440, the password asked",
        |w| w.password_each_send = true,
        |kit| dialog_seen(&kit.ctx).is_some_and(|d| d.one_scroll),
    );
    // Signed: the flow waits or has its answer, or the form shows what it signed (the demo
    // core answers at once).
    let sent = |kit: &Kit| {
        let id = &kit.state().identity;
        id.top_up_flow.signed().is_some()
            || id.top_up_flow.result().is_some()
            || id.top_up.as_ref().is_some_and(|f| f.signed.is_some())
            || id.top_up_result.is_some()
            || has_words(kit, "Wrong password")
    };
    let wheel = |kit: &mut Kit, dy: f32| {
        let centre = dialog_seen(&kit.ctx).unwrap().scrolled.unwrap().center();
        for _ in 0..12 {
            kit.hover_at(centre);
            kit.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, dy),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            });
            kit.run_steps(3);
        }
    };
    let in_sight = |kit: &Kit| {
        let d = dialog_seen(&kit.ctx).unwrap();
        let (summary, _) = d.summary.expect("a summary");
        d.scrolled
            .is_some_and(|v| v.contains(summary.left_top() + egui::vec2(1.0, 1.0)))
    };
    // (b): the wheel brings the summary in, and takes it away again.
    wheel(&mut kit, -120.0);
    assert!(in_sight(&kit), "the wheel shows the summary");
    wheel(&mut kit, 120.0);
    assert!(!in_sight(&kit), "scrolled away again");
    // (c): the password typed, the first press.
    kit.state_mut()
        .identity
        .top_up_flow
        .password
        .type_in("demo");
    std::thread::sleep(crate::a11y::SPEND_KEYS_AFTER);
    kit.run_steps(3);
    let primary = |kit: &Kit| {
        kit.query_all_by_role(Role::Button)
            .find(|b| {
                b.accesskit_node()
                    .label()
                    .is_some_and(|l| l.starts_with("Top up by "))
            })
            .expect("«Top up by …»")
            .rect()
            .center()
    };
    let click = |kit: &mut Kit, at: egui::Pos2| {
        kit.hover_at(at);
        kit.run_steps(1);
        for pressed in [true, false] {
            kit.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        kit.run_steps(6);
    };
    let at = primary(&kit);
    click(&mut kit, at);
    assert!(!sent(&kit), "the first press sends nothing");
    assert!(in_sight(&kit), "it brings the summary in");
    let at = primary(&kit);
    click(&mut kit, at);
    assert!(sent(&kit), "the second press sends");
}

/// BUG-249 r4, QA (⚠️): after the wheel, the first click on the password field left the focus in
/// the amount, and the password went into it («0,001wrong-pass-64»). A click on the pinned field
/// takes the focus, whatever the scroll did before.
#[test]
fn after_the_wheel_a_click_on_the_password_types_into_it() {
    let (_turn, mut kit) = demo_app("top-up", "en");
    settle(&mut kit);
    kit.set_size(egui::vec2(800.0, 440.0));
    show_wallet_edited(
        &mut kit,
        6,
        "the top-up at 800 × 440, the password asked",
        |w| w.password_each_send = true,
        |kit| dialog_seen(&kit.ctx).is_some_and(|d| d.one_scroll),
    );
    let amount = |kit: &Kit| kit.state().identity.top_up.as_ref().unwrap().amount.clone();
    let before = amount(&kit);
    // The focus in the amount, as after typing it; then the wheel over the middle.
    kit.query_all_by_role(Role::TextInput)
        .next()
        .expect("the amount")
        .focus();
    kit.run_steps(2);
    let centre = dialog_seen(&kit.ctx).unwrap().scrolled.unwrap().center();
    for dy in [-120.0, -120.0, 120.0] {
        kit.hover_at(centre);
        kit.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, dy),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        kit.run_steps(1);
    }
    // At once, the scroll still moving: a click on the password field.
    let at = kit
        .query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .rect()
        .center();
    kit.hover_at(at);
    for pressed in [true, false] {
        kit.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
    }
    kit.run_steps(2);
    kit.event(egui::Event::Text("wrong-pass".into()));
    kit.run_steps(3);
    assert_eq!(amount(&kit), before, "nothing typed into the amount");
}

/// BUG-249 r4, QA (⚠️): after a wrong password the form comes back (its amount field again), and
/// the next click on the password field must take the focus — QA's typed into the amount, and
/// the Enter after it sent nothing.
#[test]
fn after_a_wrong_password_a_click_on_the_password_types_into_it() {
    let (_turn, mut kit) = demo_app("top-up", "en");
    settle(&mut kit);
    show_wallet_edited(
        &mut kit,
        4,
        "the top-up, the password asked",
        |w| w.password_each_send = true,
        |kit| dialog_seen(&kit.ctx).is_some(),
    );
    let amount = |kit: &Kit| kit.state().identity.top_up.as_ref().unwrap().amount.clone();
    let before = amount(&kit);
    // Signed: the form shows what was signed (its amount field gone) while the core answers;
    // then the core's «wrong password» gives it back (the demo core takes any password, so its
    // answer is put in as the app puts it).
    kit.state_mut().identity.top_up.as_mut().unwrap().signed =
        Some(crate::screens::identity::SignedTopUp {
            amount: 10_000_000,
            core_before: 0,
            credits_before: 0,
            credits_checked: true,
            fee: 0,
            on_top: 0,
        });
    kit.run_steps(4);
    kit.state_mut().identity.top_up.as_mut().unwrap().signed = None;
    kit.state_mut().identity.top_up_flow.password.on_wrong();
    kit.run_steps(4);
    let at = kit
        .query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .rect()
        .center();
    kit.hover_at(at);
    kit.run_steps(1);
    for pressed in [true, false] {
        kit.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
    }
    kit.run_steps(2);
    kit.event(egui::Event::Text("again".into()));
    kit.run_steps(3);
    assert_eq!(amount(&kit), before, "nothing typed into the amount");
}

/// BUG-249 r4, QA on qa-f62: «Top up the identity» opened only on the third click of the
/// Identity screen's «Top up». One click opens it, and it stays open — the click that opened
/// it is not a click outside it.
#[test]
fn one_click_on_top_up_opens_the_dialog_and_it_stays() {
    for size in [egui::vec2(1280.0, 800.0), egui::vec2(800.0, 440.0)] {
        let (_turn, mut kit) = demo_app("identity", "en");
        kit.set_size(size);
        settle(&mut kit);
        assert!(kit.state().identity.top_up.is_none(), "closed at first");
        kit.query_all_by_role_and_label(Role::Button, "Top up")
            .next()
            .expect("«Top up»")
            .click();
        kit.run_steps(6);
        assert!(
            kit.state().identity.top_up.is_some(),
            "{size:?}: one click opens it"
        );
        assert!(
            dialog_seen(&kit.ctx).is_some_and(|d| d.name == "top-up"),
            "{size:?}: and it is drawn"
        );
    }
}
