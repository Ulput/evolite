//! PRIV-1: Contacts say honestly what DashPay makes public (docs/design/priv-1-2026-11-07.md).

use super::*;

const HIDE_HINT: &str =
    "Hides the request on this computer only. On the network it stays, visible to everyone.";
const ADD_LINE: &str = "The request, and the fact that you are contacts, are visible to everyone and can't be deleted. The addresses for payments between you are hidden.";
const ACCEPT_LINE: &str = "Your answer makes you contacts: everyone can see it, and it can't be deleted. The addresses for payments between you are hidden.";
const LINK: &str = "Learn more about privacy ›";
const POINTS: [&str; 5] = [
    "Who sent a request to whom, and when, is visible to everyone, forever. A request can't be deleted or withdrawn.",
    "Accepting a request means sending one back, so everyone can see who is in contact with whom. Unanswered requests are visible too.",
    "Anyone can send you a request without your consent. “Hide” removes it from this computer only.",
    "The profile (name, “About”, avatar) and @names are public. The avatar loads from someone else's server, which sees your IP.",
    "The payment keys are encrypted, and each payment goes to a new address: the requests do not link the payments. Blockchain analysis may still link them.",
];

/// Contacts with a request in the list: the demo's.
fn contacts() -> (MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("contacts", "en");
    wait_for(&mut kit, "a request", |kit| {
        kit.query_all_by_role_and_label(Role::Button, "Hide")
            .next()
            .is_some()
    });
    (turn, kit)
}

/// Tab until the focus is on the widget named exactly `name`.
fn tab_exactly(kit: &mut Kit, name: &str) {
    for _ in 0..80 {
        if focused(kit).as_deref() == Some(name) {
            return;
        }
        tab(kit, false);
    }
    panic!("Tab never reached «{name}»");
}

/// AC PRIV-1.3: the screen's own texts claim no more than is true.
#[test]
fn contacts_say_what_is_public_and_promise_no_unlinkable_payments() {
    let (_turn, kit) = contacts();
    for text in [
        "DashPay contacts: you exchanged requests. The requests themselves are public; the payment keys inside them are encrypted. Each payment goes to a new address from the contact's keys. A request is a write to Platform paid for by the identity's credits.",
        "Visible to everyone, forever: who, to whom and when. Inside is a key for paying you, encrypted for the recipient. The identity's credits pay for it.",
        "Each payment goes to a new address from the contact's key: the request does not link the payments. Blockchain analysis may still link them.",
    ] {
        assert!(has_text(&kit, text), "missing: {text}");
    }
    let overclaims = kit
        .query_all_by(|n| {
            n.value()
                .or(n.label())
                .is_some_and(|v| v.contains("encrypted requests") || v.contains("cannot see"))
        })
        .count();
    assert_eq!(overclaims, 0);
    assert!(button(&kit, LINK).rect().is_positive());
}

/// AC PRIV-1.2: «Hide» says it is local only — on keyboard focus too, and to a screen reader.
#[test]
fn hide_says_it_is_local_on_focus_and_to_a_screen_reader() {
    let (_turn, mut kit) = contacts();
    let hide = kit
        .query_all_by_role_and_label(Role::Button, "Hide")
        .next()
        .expect("«Hide»");
    assert_eq!(
        hide.accesskit_node().description().as_deref(),
        Some(HIDE_HINT)
    );
    assert!(!has_text(&kit, HIDE_HINT), "a hint before any focus");
    tab_exactly(&mut kit, "Hide");
    kit.run_steps(2);
    assert!(has_text(&kit, HIDE_HINT), "no hint on keyboard focus");
}

/// AC PRIV-1.1: «Add a contact» says what becomes public, with no error and no hover.
#[test]
fn add_a_contact_says_what_becomes_public() {
    let (_turn, mut kit) = demo_app("add-contact", "en");
    kit.run_steps(3);
    assert!(has_text(&kit, ADD_LINE));
}

/// AC PRIV-1.1: «Accept» opens «Accept the request», which says what the answer makes public;
/// without «password for every payment» its «Accept» sends the answer at once.
#[test]
fn accept_says_what_the_answer_makes_public_before_it_goes() {
    let (_turn, mut kit) = contacts();
    assert!(kit.state().contacts.accepting.is_none());
    kit.query_all_by_role_and_label(Role::Button, "Accept")
        .next()
        .expect("«Accept» of the request")
        .click();
    kit.run_steps(4);
    let id = kit
        .state()
        .contacts
        .accepting
        .clone()
        .expect("«Accept the request» open");
    assert!(has_text(&kit, "Accept the request"));
    assert!(has_text(&kit, ACCEPT_LINE));
    assert!(!has_text(&kit, "Wallet password"));
    // The dialog's own «Accept» is drawn over the page: the last one.
    kit.query_all_by_role_and_label(Role::Button, "Accept")
        .last()
        .expect("the dialog's «Accept»")
        .click();
    // The demo core refuses every spend: its answer coming back is the answer having gone.
    wait_for(&mut kit, "the core's answer", |kit| {
        kit.state().contacts.result.is_some() && !kit.state().contacts.waiting
    });
    assert_eq!(kit.state().contacts.accepting.as_deref(), Some(id.as_str()));
}

/// AC PRIV-1.4: the link opens «Contact privacy» with its five points; Escape and «Got it»
/// close it, and the focus goes back to the link.
#[test]
fn the_privacy_dialog_opens_from_the_link_and_closes_back_to_it() {
    let (_turn, mut kit) = contacts();
    for close in ["Escape", "Got it"] {
        tab_exactly(&mut kit, LINK);
        enter(&mut kit);
        kit.run_steps(2);
        assert!(kit.state().contacts.privacy, "{close}: not opened");
        assert!(has_text(&kit, "Contact privacy"));
        for point in POINTS {
            assert!(has_text(&kit, point), "missing: {point}");
        }
        if close == "Escape" {
            kit.key_press(Key::Escape);
            kit.run_steps(3);
        } else {
            button(&kit, "Got it").click();
            kit.run_steps(3);
        }
        assert!(!kit.state().contacts.privacy, "{close}: still open");
        assert!(!has_text(&kit, POINTS[0]));
        assert_eq!(focused(&kit).as_deref(), Some(LINK), "{close}: focus");
    }
}

/// PRIV-1 r1 must 1 (BUG-42 beside it): with «password for every payment» on, «Accept the
/// request» has the password field under the line about what is public; its «Accept» stays off
/// while the field is empty, and the answer goes with the typed password — the demo core lets it
/// past its password gate (not «password needed», not «wrong password»).
#[test]
fn accept_with_the_password_setting_asks_for_it_and_sends_it() {
    let (_turn, mut kit) = contacts();
    kit.state()
        .core
        .send(evolite_api::Command::SetPasswordEachSend {
            on: true,
            password: None,
        });
    wait_for(&mut kit, "the setting in the snapshot", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .is_some_and(|w| w.password_each_send)
    });
    kit.run_steps(3);
    kit.query_all_by_role_and_label(Role::Button, "Accept")
        .next()
        .expect("«Accept» of the request")
        .click();
    kit.run_steps(4);
    assert!(kit.state().contacts.accepting.is_some(), "the dialog");
    assert!(has_text(&kit, ACCEPT_LINE));
    assert!(has_text(&kit, "Wallet password"), "no password field");
    let dialog_accept = |kit: &Kit| {
        kit.query_all_by_role_and_label(Role::Button, "Accept")
            .last()
            .expect("the dialog's «Accept»")
            .accesskit_node()
            .is_disabled()
    };
    assert!(dialog_accept(&kit), "«Accept» on with no password");
    kit.query_all_by_role_and_label(Role::Button, "Accept")
        .last()
        .expect("the dialog's «Accept»")
        .click();
    kit.run_steps(3);
    assert!(
        !kit.state().contacts.waiting && kit.state().contacts.result.is_none(),
        "an answer went with no password"
    );
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .click();
    kit.run_steps(2);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .type_text("the demo takes any other");
    kit.run_steps(2);
    assert!(!dialog_accept(&kit), "«Accept» off with a password typed");
    kit.query_all_by_role_and_label(Role::Button, "Accept")
        .last()
        .expect("the dialog's «Accept»")
        .click();
    wait_for(&mut kit, "the core's answer", |kit| {
        kit.state().contacts.result.is_some() && !kit.state().contacts.waiting
    });
    let result = kit.state().contacts.result.clone();
    // The demo core's gate: «asks for the password for every send» when none came.
    let gate = |e: &evolite_api::CoreError| match e {
        evolite_api::CoreError::NotAvailable(why) => why.contains("password for every send"),
        evolite_api::CoreError::WrongPassword => true,
        _ => false,
    };
    assert!(
        !matches!(&result, Some(Err(e)) if gate(e)),
        "the answer did not carry the password: {result:?}"
    );
}

/// PRIV-1.5 (r1 should 1): in Arabic the line of «Add a contact» starts at the right — its
/// icon on the right of the words, the line the mirror of the English one — and «Contact
/// privacy» opens from its link with all five points.
#[test]
fn arabic_mirrors_the_public_line_and_shows_the_privacy_dialog() {
    let start = |lang: &str| {
        let (_turn, mut kit) = demo_app("add-contact", lang);
        kit.run_steps(3);
        let line = crate::i18n::t(ADD_LINE).into_owned();
        let rect = kit
            .query_all(By::new().role(Role::Label).value(&line))
            .next()
            .unwrap_or_else(|| panic!("{lang}: the line"))
            .rect();
        (rect, kit.ctx.content_rect().width())
    };
    let (en, width) = start("en");
    let (ar, _) = start("ar");
    eprintln!("en {en:?}, ar {ar:?}, window {width}");
    assert!(
        (en.left() - (width - ar.right())).abs() < 1.0,
        "the Arabic line does not start where the English one ends mirrored: en {en:?}, ar {ar:?}"
    );

    let (_turn, mut kit) = demo_app("contacts", "ar");
    wait_for(&mut kit, "a request", |kit| {
        kit.query_all_by_role(Role::Button)
            .any(|b| b.accesskit_node().label().as_deref() == Some(&crate::i18n::spoken("Hide")))
    });
    let link = crate::i18n::spoken(LINK);
    button(&kit, &link).click();
    kit.run_steps(3);
    assert!(kit.state().contacts.privacy, "not opened");
    for point in POINTS {
        let shown = crate::i18n::t(point).into_owned();
        assert!(has_text(&kit, &shown), "missing: {point}");
    }
}
