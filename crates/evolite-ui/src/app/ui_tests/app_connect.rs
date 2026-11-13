//! FEAT-0023 round 4: App Connect v2 on screen, on the demo core (docs/design/
//! app-connect-v2-2026-11-11.md): the request, the list's states, renewing, removing and the
//! refusals.

use super::*;
use evolite_api::Command;

/// The demo's Yappr request, opened as a pasted link would open it.
fn open_request(kit: &mut Kit) {
    kit.state_mut().core.send(Command::OpenAppLink {
        text: "dashpay://connect?v=2&n=t&r=AQ".into(),
    });
    wait_for(kit, "the request dialog", |kit| {
        kit.state().requests.open.is_some()
    });
    kit.run_steps(3);
}

fn type_password(kit: &mut Kit, password: &str) {
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .click();
    kit.run_steps(2);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .type_text(password);
    kit.run_steps(2);
}

fn enabled(kit: &Kit, name: &str) -> bool {
    kit.query_all_by_role_and_label(Role::Button, name)
        .last()
        .is_some_and(|b| !b.accesskit_node().is_disabled())
}

/// AC-D1–D3: the request says who the app is in its own words, what it may and may not do, the
/// limit and period it asks (selected), the warning about older apps before the password, and
/// what it costs; «Connect» waits for the password, which is asked even with the setting off.
#[test]
fn the_request_says_what_the_app_gets_and_waits_for_the_password() {
    let (_turn, mut kit) = demo_app("apps", "en");
    assert!(
        !kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .unwrap()
            .password_each_send,
        "the setting is off in the demo"
    );
    open_request(&mut kit);
    for words in [
        "Connect an app",
        "what the app calls itself",
        "The app will be able to",
        "Write to the Yappr contracts in your name",
        "Change your DashPay profile",
        "Transfer credits and DASH",
        "See the phrase and other keys",
        "Spending limit, DASH",
        "This is permanent for",
        "Adding keys",
    ] {
        assert!(has_words(&kit, words), "«{words}»");
    }
    assert!(
        !has_words(&kit, "I understand"),
        "no checkbox before the password"
    );
    assert!(!enabled(&kit, "Connect"), "not before the password");
    assert!(has_text(&kit, "Enter the password"));
    type_password(&mut kit, "any");
    assert!(enabled(&kit, "Connect"));
}

/// A1, after: «Connect» sends the choices; the list says the app is connected and shows it.
#[test]
fn connecting_puts_the_app_in_the_list() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.state_mut()
        .snapshot
        .as_mut()
        .and_then(|s| s.wallet.as_mut())
        .unwrap()
        .apps
        .clear();
    open_request(&mut kit);
    type_password(&mut kit, "any");
    button(&kit, "Connect").click();
    wait_for(&mut kit, "the app connected", |kit| {
        kit.state().requests.connected.is_some()
    });
    kit.run_steps(3);
    assert!(kit.state().requests.open.is_none(), "the dialog closed");
    assert!(has_words(&kit, "Yappr connected: limit 0.51"), "the banner");
    assert!(has_text(&kit, "Active"));
    assert!(has_words(&kit, "Spent"));
}

/// The optional key can be left off: the grant has none for it, the primary key all the limit.
#[test]
fn an_optional_key_left_off_is_not_granted() {
    let (_turn, mut kit) = demo_app("apps", "en");
    open_request(&mut kit);
    kit.query_by_role_and_label(Role::CheckBox, "Change your DashPay profile")
        .expect("the optional key's checkbox")
        .click();
    kit.run_steps(2);
    let state = &kit.state().requests;
    let request = state.open.clone().unwrap();
    let grant =
        crate::screens::connect2::grant(&request, &state.choice, chrono::Local::now().timestamp())
            .expect("a grant");
    assert_eq!(grant.keys[1], None);
    assert_eq!(grant.keys[0].map(|(b, _)| b), Some(51_000_000));
}

/// L1/L2: active, used up (with what to do), expired, and the disconnected ones apart with
/// «Remove from the list» only; never red.
#[test]
fn the_list_shows_each_connections_state() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.run_steps(3);
    for words in [
        "Connected · 3",
        "Active",
        "Limit used up",
        "Expired on",
        "The app can no longer write anything in your name.",
        "Disconnected · 1",
        "disconnected on",
    ] {
        assert!(has_words(&kit, words), "«{words}»");
    }
    assert_eq!(
        kit.query_all_by_role_and_label(Role::Button, "Remove from the list")
            .count(),
        1,
        "only the disconnected one"
    );
    assert_eq!(
        kit.query_all_by_role_and_label(Role::Button, "Renew")
            .count(),
        3
    );
}

/// D2: removing a disconnected app asks once, with no password, and it leaves the list.
#[test]
fn a_disconnected_app_leaves_the_list_without_a_password() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.run_steps(3);
    // The fourth card, below the fold at the test window's height: by AccessKit, as a reader.
    button(&kit, "Remove from the list").click_accesskit();
    kit.run_steps(3);
    assert!(has_text(&kit, "Remove from the list?"));
    assert!(kit.query_all_by_role(Role::PasswordInput).next().is_none());
    kit.query_all_by_role_and_label(Role::Button, "Remove from the list")
        .last()
        .expect("the dialog's button")
        .click();
    wait_for(&mut kit, "the app removed", |kit| {
        !has_words(kit, "Disconnected · 1")
    });
}

/// R1: «Renew» says a new sign-in renews it, with the limit as it stands, and offers the link.
#[test]
fn renewing_explains_the_new_sign_in() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.run_steps(3);
    kit.query_all_by_role_and_label(Role::Button, "Renew")
        .next()
        .expect("«Renew»")
        .click();
    kit.run_steps(3);
    assert!(has_text(&kit, "Renew the Yappr connection"));
    assert!(has_words(&kit, "it takes a new sign-in"));
    kit.query_all_by_role_and_label(Role::Button, "Paste a link")
        .last()
        .expect("«Paste a link»")
        .click();
    kit.run_steps(3);
    assert!(kit.state().requests.pasting, "the paste dialog opens");
}

/// E1: a v1 link is refused in its own dialog, with its version and that nothing was spent.
#[test]
fn a_v1_link_is_refused_with_its_version() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.state_mut().core.send(Command::OpenAppLink {
        text: "dash-key:abc?n=t&v=1".into(),
    });
    wait_for(&mut kit, "the refusal", |kit| {
        kit.state().requests.refused.is_some()
    });
    kit.run_steps(3);
    assert!(has_text(&kit, "Cannot connect"));
    assert!(has_words(&kit, "(v=1)"));
    assert!(has_words(&kit, "Nothing was spent."));
    button(&kit, "Got it").click();
    kit.run_steps(2);
    assert!(kit.state().requests.refused.is_none());
}

/// The rect of the first node whose label or value starts with `words`.
fn rect_of(kit: &Kit, words: &str) -> egui::Rect {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.starts_with(words))
            || n.value().is_some_and(|v| v.starts_with(words))
    })
    .next()
    .unwrap_or_else(|| panic!("«{words}»"))
    .rect()
}

/// The room between the line starting with `under` and the nearest title above it starting with
/// `title` (the list behind the dialog has the same titles).
fn gap_above(kit: &Kit, under: &str, title: &str) -> f32 {
    let line = rect_of(kit, under);
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.starts_with(title))
            || n.value().is_some_and(|v| v.starts_with(title))
    })
    .map(|n| n.rect())
    .filter(|r| r.bottom() <= line.top() + 0.5 && (r.left() - line.left()).abs() < 60.0)
    .map(|r| line.top() - r.bottom())
    .min_by(f32::total_cmp)
    .unwrap_or_else(|| panic!("«{title}» above «{under}»"))
}

/// A1, the designer's r4 note 3: the line under an optional key (with its checkbox) stands as
/// close to its title as the line under the primary key does.
#[test]
fn the_line_under_each_key_is_as_close_to_its_title() {
    let (_turn, mut kit) = demo_app("apps", "en");
    open_request(&mut kit);
    let primary = gap_above(&kit, "fees come from", "Write to");
    let optional = gap_above(&kit, "up to ", "Change your DashPay profile");
    assert!(
        (primary - optional).abs() <= 2.0,
        "under the primary key {primary} px, under the optional one {optional} px"
    );
}

/// L1, the designer's r4 note 2: a state chip has its icon before its words, as every status
/// in the app (the card's right-to-left row must not turn it round).
#[test]
fn a_state_chip_has_its_icon_before_its_words() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.run_steps(3);
    let words = rect_of(&kit, "Active");
    let icon = kit
        .query_all_by(|n| n.role() == Role::Image)
        .map(|n| n.rect())
        .find(|r| {
            (r.center().y - words.center().y).abs() < 6.0 && (r.left() - words.right()).abs() < 30.0
                || (r.center().y - words.center().y).abs() < 6.0
                    && (words.left() - r.right()).abs() < 30.0
        })
        .expect("the chip's icon beside «Active»");
    assert!(
        icon.right() <= words.left(),
        "the icon at {icon:?}, the words at {words:?}"
    );
}
