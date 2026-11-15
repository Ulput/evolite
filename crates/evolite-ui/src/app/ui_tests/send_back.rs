//! BUG-169: «Cancel» on Send goes back to where Send was entered from, with that screen as it
//! was; the «‹ {place}» link above the title names it; Esc without a dialog and the mouse's
//! «back» go there too (АРХ-1, AC-T3…T7, T9).

use super::*;
use crate::screens::Screen;

fn on(kit: &Kit, screen: Screen) -> bool {
    kit.state().screen == screen
}

/// The link above Send's title says it goes back to `place`.
fn link_to(kit: &Kit, place: &str) -> bool {
    kit.query_by_role_and_label(Role::Link, &format!("Back: {place}"))
        .is_some()
}

/// The last-drawn button named `name`: the dialog's over the page's.
fn click_last(kit: &mut Kit, name: &str) {
    kit.query_all_by_role_and_label(Role::Button, name)
        .last()
        .unwrap_or_else(|| panic!("no «{name}»"))
        .click();
    kit.run_steps(3);
}

/// Send's «Cancel» (the form's, the last one drawn), once Send is ready for it: the core has
/// gone quiet (the identity's refresh, BUG-175), the link is drawn and «Cancel» is on. Then the
/// frames until the screen is `to` (bounded by time, not by frames: under the full suite's load
/// three frames were not always enough).
fn cancel_to(kit: &mut Kit, to: Screen, what: &str) {
    settle(kit);
    wait_for(kit, &format!("{what}: «Cancel» ready"), |k| {
        k.query_all_by_role(Role::Link).any(|l| {
            l.accesskit_node()
                .label()
                .is_some_and(|n| n.starts_with("Back: "))
        }) && k
            .query_all_by_role_and_label(Role::Button, "Cancel")
            .last()
            .is_some_and(|b| !b.accesskit_node().is_disabled())
    });
    click_last(kit, "Cancel");
    wait_for(kit, &format!("{what}: back on {to:?}"), |k| on(k, to));
}

/// From the screen `preset` opens, `enter` reaches Send; the link names `place`; «Cancel» leads
/// back to the same screen, the form empty; then `kept` holds of the screen's own state.
fn round_trip(preset: &str, enter: impl Fn(&mut Kit), place: &str, kept: impl Fn(&Kit) -> bool) {
    let (_turn, mut kit) = demo_app(preset, "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    let from = kit.state().screen;
    assert_ne!(from, Screen::Send, "{preset}");
    enter(&mut kit);
    wait_for(&mut kit, "Send", |k| on(k, Screen::Send));
    wait_for(&mut kit, &format!("{preset}: «‹ {place}»"), |k| {
        link_to(k, place)
    });
    cancel_to(
        &mut kit,
        from,
        &format!("{preset}: back where it came from"),
    );
    assert!(
        kit.state().send.recipient.is_empty() && kit.state().send.amount.is_empty(),
        "{preset}: the form cleared (AC-T6)"
    );
    assert!(kept(&kit), "{preset}: the screen as it was");
}

/// AC-T4, entry 2: Overview's «Send».
#[test]
fn overview_send_cancels_back_to_overview() {
    round_trip("overview", |k| click_last(k, "Send"), "Overview", |_| true);
}

/// AC-T4, entry 3: Overview's «Withdraw to Core».
#[test]
fn overview_withdraw_cancels_back_to_overview() {
    round_trip(
        "overview",
        |k| click_last(k, "Withdraw to Core"),
        "Overview",
        |_| true,
    );
}

/// AC-T4, entry 4: a contact's «Send» in the list.
#[test]
fn a_contacts_send_cancels_back_to_contacts() {
    round_trip(
        "contacts",
        |k| {
            k.query_all_by_role_and_label(Role::Button, "Send")
                .find(|b| {
                    b.accesskit_node()
                        .bounding_box()
                        .is_some_and(|r| r.x0 > 300.0)
                })
                .expect("a contact's «Send»")
                .click();
            k.run_steps(3);
        },
        "Contacts",
        |_| true,
    );
}

/// AC-T4, entry 5: «Send» in a contact's profile — back, the profile is open again.
#[test]
fn a_contact_profiles_send_cancels_back_to_the_profile() {
    round_trip(
        "contact-profile",
        |k| click_last(k, "Send"),
        "Contacts",
        |k| k.state().contacts.profile.is_some(),
    );
}

/// AC-T4, entries 6 and 7: Identity's «Withdraw» and «Transfer».
#[test]
fn identitys_withdraw_and_transfer_cancel_back_to_identity() {
    for action in ["Withdraw", "Transfer"] {
        round_trip("identity", |k| click_last(k, action), "Identity", |_| true);
    }
}

/// AC-T4, entry 8: the private pool's «Send privately».
#[test]
fn the_pools_send_cancels_back_to_the_pool() {
    round_trip(
        "shielded",
        |k| {
            wait_for(k, "the pool connected", |k| {
                k.query_all_by_label("Send privately").next().is_some()
            });
            click_last(k, "Send privately");
        },
        "Private pool",
        |_| true,
    );
}

/// AC-T4, entry 9: an operation's card in History, «Send again» — back, the card is open again.
#[test]
fn send_again_cancels_back_to_the_card() {
    round_trip(
        "activity-card",
        |k| click_last(k, "Send again"),
        "History",
        |k| {
            k.query_all_by_role_and_label(Role::Button, "Send again")
                .next()
                .is_some()
        },
    );
}

/// AC-T4: History's filters are kept on the way to Send and back (the menu's «Send»).
#[test]
fn historys_filters_are_kept_there_and_back() {
    let (_turn, mut kit) = demo_app("activity-filters", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    let filters = kit.state().history.filters;
    assert_ne!(filters, Default::default());
    click_last(&mut kit, "Send");
    wait_for(&mut kit, "Send", |k| on(k, Screen::Send));
    wait_for(&mut kit, "filters: «‹ History»", |k| {
        link_to(k, "History")
    });
    cancel_to(&mut kit, Screen::Activity, "filters");
    assert_eq!(kit.state().history.filters, filters);
}

/// AC-T5: the menu's «Send» goes back to the screen open before it (R-2); started on Send,
/// there is none — «‹ Overview», and «Cancel» leads there (R-3).
#[test]
fn from_the_menu_back_to_the_last_screen_and_cold_to_overview() {
    round_trip("identity", |k| click_last(k, "Send"), "Identity", |_| true);
    let (_turn, mut kit) = demo_app("send", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    wait_for(&mut kit, "cold: «‹ Overview»", |k| {
        link_to(k, "Overview")
    });
    cancel_to(&mut kit, Screen::Overview, "cold");
}

/// AC-T6: leaving Send by the menu keeps what was typed; coming back by the menu finds it.
#[test]
fn leaving_by_the_menu_keeps_the_form() {
    let (_turn, mut kit) = demo_app("send", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    kit.state_mut().send.recipient = "@alice".into();
    click_last(&mut kit, "History");
    assert!(on(&kit, Screen::Activity));
    click_last(&mut kit, "Send");
    wait_for(&mut kit, "Send", |k| on(k, Screen::Send));
    assert_eq!(kit.state().send.recipient, "@alice");
    assert!(link_to(&kit, "History"));
}

/// AC-T3: a lock forgets the place: after it, Send goes back to Overview.
#[test]
fn a_lock_forgets_the_place() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    click_last(&mut kit, "Send");
    wait_for(&mut kit, "Send", |k| on(k, Screen::Send));
    assert!(kit.state().back.is_some());
    show_edited(
        &mut kit,
        1,
        "the place forgotten on the lock",
        |app| {
            if let Some(s) = app.snapshot.as_mut() {
                s.vault = evolite_api::VaultState::Locked;
            }
        },
        |k| k.state().back.is_none(),
    );
}

/// AC-T7: Esc on Send without a dialog, and the mouse's «back», go back as «Cancel» does; Esc
/// with «Check the payment» open only closes it.
#[test]
fn esc_and_the_mouses_back_go_back_but_not_from_a_dialog() {
    for back in [
        egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: egui::pos2(600.0, 400.0),
            button: egui::PointerButton::Extra1,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ] {
        let (_turn, mut kit) = demo_app("identity", "en");
        settle(&mut kit);
        click_last(&mut kit, "Send");
        wait_for(&mut kit, "Send", |k| on(k, Screen::Send));
        kit.state_mut().send.recipient = "@alice".into();
        kit.run_steps(2);
        settle(&mut kit);
        kit.input_mut().events.push(back.clone());
        wait_for(&mut kit, "back to Identity", |k| on(k, Screen::Identity));
        assert!(kit.state().send.recipient.is_empty(), "{back:?}: cleared");
    }
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    wait_for(&mut kit, "the last look", |k| k.state().send.confirming);
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    assert!(on(&kit, Screen::Send), "Esc closed only the dialog");
    assert!(!kit.state().send.confirming);
}

/// AC-T9: the link is the screen's first Tab stop, a link named «Back: {place}»; Enter on it
/// goes back.
#[test]
fn the_link_is_the_first_tab_stop() {
    let (_turn, mut kit) = demo_app("identity", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    click_last(&mut kit, "Send");
    wait_for(&mut kit, "Send", |k| on(k, Screen::Send));
    kit.run_steps(3);
    tab(&mut kit, false);
    assert_eq!(focused(&kit).as_deref(), Some("Back: Identity"));
    enter(&mut kit);
    kit.run_steps(2);
    assert!(on(&kit, Screen::Identity));
}

/// AC-T9: in Arabic the link names the place in Arabic and stands at the right.
#[test]
fn in_arabic_the_link_stands_at_the_right() {
    let (_turn, mut kit) = demo_app("send", "ar");
    settle(&mut kit);
    let name = crate::i18n::spoken("Back: Overview");
    let link = kit
        .query_by_role_and_label(Role::Link, &name)
        .unwrap_or_else(|| panic!("«{name}»"));
    let r = link.accesskit_node().bounding_box().expect("drawn");
    assert!(
        r.x0 > kit.ctx.content_rect().center().x as f64,
        "at the right: {r:?}"
    );
}

/// AC-T8 (S4, the customer: «Да, как на S4»): after a payment is sent, «Done» stands in the
/// place of «Check and send» and «Cancel» is hidden; typing brings the usual buttons back;
/// «Done» goes back where «Cancel» would (here entered from Identity's menu). The demo core never
/// sends: the form is put as the core's «sent» leaves it — cleared, the answer on it.
#[test]
fn done_after_a_payment_goes_back() {
    let (_turn, mut kit) = demo_app("identity", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    click_last(&mut kit, "Send");
    wait_for(&mut kit, "Send", |k| on(k, Screen::Send));
    // Sent: the form is cleared (BUG-39 №7), the answer on it.
    kit.state_mut().send.recipient.clear();
    kit.state_mut().send.amount.clear();
    kit.state_mut().send.result = Some(Ok(evolite_api::SentPayment {
        txid: "ab".repeat(32),
        amount: 10_000_000,
        fee: 227,
        relayed: true,
    }));
    kit.run_steps(3);
    let done = |k: &Kit| {
        k.query_all_by_role_and_label(Role::Button, "Done")
            .next()
            .is_some()
    };
    let usual = |k: &Kit| {
        k.query_all_by_role_and_label(Role::Button, "Check and send")
            .next()
            .is_some()
    };
    wait_for(&mut kit, "«Done»", done);
    assert!(!usual(&kit), "in the place of «Check and send»");
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Cancel")
            .next()
            .is_none(),
        "«Cancel» hidden"
    );
    kit.state_mut().send.recipient = "@alice".into();
    kit.run_steps(2);
    assert!(
        usual(&kit) && !done(&kit),
        "typing brings the usual buttons"
    );
    kit.state_mut().send.recipient.clear();
    kit.run_steps(2);
    click_last(&mut kit, "Done");
    assert!(on(&kit, Screen::Identity));
}

/// The menu's item `name` (drawn before the screen: the first so named).
fn menu(kit: &mut Kit, name: &str) {
    kit.query_all_by_role_and_label(Role::Button, name)
        .next()
        .unwrap_or_else(|| panic!("no «{name}» in the menu"))
        .click();
    kit.run_steps(3);
}

/// r1 Should 1: a contact's profile → «Send» → the menu's «History» → the menu's «Contacts»:
/// the return was given up, so the list, not the profile left open for it.
#[test]
fn a_profile_left_for_a_return_closes_when_send_is_left_otherwise() {
    let (_turn, mut kit) = demo_app("contact-profile", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    assert!(kit.state().contacts.profile.is_some());
    click_last(&mut kit, "Send");
    wait_for(&mut kit, "Send", |k| on(k, Screen::Send));
    assert!(
        kit.state().contacts.profile.is_some(),
        "kept for the return"
    );
    menu(&mut kit, "History");
    assert!(on(&kit, Screen::Activity));
    assert!(kit.state().contacts.profile.is_none(), "closed: given up");
    menu(&mut kit, "Contacts");
    assert!(on(&kit, Screen::Contacts) && kit.state().contacts.profile.is_none());
}

/// r1 Should 1: an operation's card → «Send again» → the menu's «Contacts» → the menu's
/// «History»: the card left open for the return is closed.
#[test]
fn a_card_left_for_a_return_closes_when_send_is_left_otherwise() {
    let (_turn, mut kit) = demo_app("activity-card", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    let ctx = kit.ctx.clone();
    assert!(crate::screens::activity_card::card_open(&ctx));
    click_last(&mut kit, "Send again");
    wait_for(&mut kit, "Send", |k| on(k, Screen::Send));
    assert!(
        crate::screens::activity_card::card_open(&ctx),
        "kept for the return"
    );
    menu(&mut kit, "Contacts");
    assert!(on(&kit, Screen::Contacts));
    assert!(
        !crate::screens::activity_card::card_open(&ctx),
        "closed: given up"
    );
}

/// `--screen send-done` (BUG-169 S4, for the design check): Send just after a payment — «Done»,
/// no «Cancel».
#[test]
fn the_after_send_preset_shows_done() {
    let (_turn, mut kit) = demo_app("send-done", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    assert!(on(&kit, Screen::Send));
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Done")
            .next()
            .is_some()
    );
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Cancel")
            .next()
            .is_none()
    );
}
