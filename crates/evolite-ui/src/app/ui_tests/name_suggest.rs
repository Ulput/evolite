//! FEAT-0039: the names suggested under «To» and «Name in DPNS» — what they offer and in what
//! order, the keys, that a choice only fills the field, and a network that does not answer.

use super::*;
use crate::screens::Screen;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};

/// The top text field on the screen («To» on Send) or the one holding `value`: clicked, so it
/// has the focus.
fn click_field(kit: &mut Kit, value: Option<&str>) {
    let field = kit
        .query_all_by_role(Role::TextInput)
        .filter(|n| value.is_none_or(|v| n.value().as_deref() == Some(v)))
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("a text field");
    field.click();
    kit.run_steps(2);
}

/// Types `text` into the field that has the focus, one character a frame — as fast as a person
/// never types, well within the pause before the network is asked.
fn type_slowly(kit: &mut Kit, text: &str) {
    for c in text.chars() {
        kit.query_by(|n| n.is_focused())
            .expect("a field with the focus")
            .type_text(&c.to_string());
        kit.run_steps(1);
    }
    // The list follows the field a frame later.
    kit.run_steps(2);
}

/// The rows of the open list, as a screen reader names them.
fn options(kit: &Kit) -> Vec<String> {
    kit.query_all_by_role(Role::ListBoxOption)
        .map(|n| n.accesskit_node().label().unwrap_or_default())
        .collect()
}

/// AC-A2, AC-A6: «@ali» in «To» — the contact first with its tag, then the network's names,
/// shorter first; the list is a listbox of options with their full words, and says its count.
#[test]
fn a_name_typed_in_to_suggests_the_contact_first_then_the_network() {
    let (_turn, mut kit) = demo_app("send-book", "en");
    click_field(&mut kit, None);
    type_slowly(&mut kit, "@ali");
    wait_for(&mut kit, "the network's names", |kit| {
        options(kit).len() == 3
    });
    assert_eq!(
        options(&kit),
        [
            "@alice, Alice Chen, In contacts",
            "@alina, Alina",
            "@alice-pay, Alice Payments",
        ]
    );
    let list = kit.get_by_role(Role::ListBox);
    assert_eq!(
        list.accesskit_node().label().as_deref(),
        Some("3 suggestions")
    );
    assert!(
        !has_words(&kit, "Searching the network"),
        "the network answered: no line under the rows"
    );
    assert!(
        focused(&kit).is_some_and(|f| f.is_empty() || !f.starts_with('@')),
        "the focus stays in the field"
    );
}

/// AC-A3, AC-T7: ↓ ↓ Enter takes the second row — its @name goes into «To» and is checked as
/// if typed; nothing is paid.
#[test]
fn the_second_suggestion_fills_the_field_and_is_checked_as_typed() {
    let (_turn, mut kit) = demo_app("send-book", "en");
    click_field(&mut kit, None);
    type_slowly(&mut kit, "@ali");
    wait_for(&mut kit, "the network's names", |kit| {
        options(kit).len() == 3
    });
    kit.key_press(egui::Key::ArrowDown);
    kit.key_press(egui::Key::ArrowDown);
    kit.run_steps(2);
    let selected: Vec<_> = kit
        .query_all_by_role(Role::ListBoxOption)
        .filter(|n| n.accesskit_node().is_selected() == Some(true))
        .map(|n| n.accesskit_node().label().unwrap_or_default())
        .collect();
    assert_eq!(selected, ["@alina, Alina"]);
    enter(&mut kit);
    assert_eq!(kit.state().send.recipient, "@alina");
    assert!(options(&kit).is_empty(), "the list closed");
    wait_for(
        &mut kit,
        "the name checked",
        |kit| matches!(&kit.state().send.resolution, Some(Ok(found)) if found.name == "alina"),
    );
    let send = &kit.state().send;
    assert!(
        !send.confirming && send.waiting == 0 && send.result.is_none(),
        "a choice pays nothing"
    );
}

/// AC-A5: «@alice» typed letter by letter asks the network once.
#[test]
fn typing_a_name_quickly_asks_the_network_once() {
    let (_turn, mut kit) = demo_app("send-book", "en");
    click_field(&mut kit, None);
    type_slowly(&mut kit, "@alice");
    wait_for(&mut kit, "the answer", |kit| {
        options(kit)
            .first()
            .is_some_and(|o| o.starts_with("@alice,"))
            && !has_words(kit, "Searching the network")
    });
    assert_eq!(kit.state().send.names.sent, 1);
    // A longer beginning of an answer that was not full: no new ask.
    type_slowly(&mut kit, "-");
    kit.run_steps(2);
    std::thread::sleep(crate::components::name_suggest::PAUSE * 2);
    kit.run_steps(2);
    assert_eq!(options(&kit), ["@alice-pay, Alice Payments"]);
    assert_eq!(kit.state().send.names.sent, 1);
}

/// AC-A6: Esc closes the list and keeps the name; the next Esc is the screen's.
#[test]
fn esc_closes_the_list_and_keeps_what_was_typed() {
    let (_turn, mut kit) = demo_app("send-book", "en");
    click_field(&mut kit, None);
    type_slowly(&mut kit, "@ali");
    wait_for(&mut kit, "the list", |kit| !options(kit).is_empty());
    kit.key_press(egui::Key::Escape);
    kit.run_steps(2);
    assert!(options(&kit).is_empty());
    assert_eq!(kit.state().send.recipient, "@ali");
    assert_eq!(
        kit.state().screen,
        Screen::Send,
        "the first Esc was the list's"
    );
}

/// AC-A4, AC-T6: the network does not answer — one's own rows at once, and after 5 s the list
/// says so; the name can still be typed whole and checked.
#[test]
fn without_an_answer_the_contacts_stay_and_the_list_says_so() {
    let (_turn, mut kit) = demo_app("names-offline", "en");
    click_field(&mut kit, None);
    type_slowly(&mut kit, "@ali");
    assert_eq!(options(&kit), ["@alice, Alice Chen, In contacts"]);
    assert!(has_words(&kit, "Searching the network…"));
    wait_for(&mut kit, "the network's silence said", |kit| {
        has_words(
            kit,
            "The network did not answer — only your contacts are shown",
        )
    });
    assert_eq!(options(&kit), ["@alice, Alice Chen, In contacts"]);
    type_slowly(&mut kit, "ce");
    wait_for(
        &mut kit,
        "the name checked",
        |kit| matches!(&kit.state().send.resolution, Some(Ok(found)) if found.name == "alice"),
    );
}

/// AC-A7: «Add a contact» offers none of one's own identities; a contact is tagged and Enter
/// on it opens its card instead of a new request.
#[test]
fn add_contact_offers_no_own_identity_and_opens_a_contact() {
    let (_turn, mut kit) = demo_app("add-contact", "en");
    click_field(&mut kit, Some("@carol"));
    for _ in 0.."@carol".len() {
        kit.key_press(egui::Key::Backspace);
    }
    kit.run_steps(2);
    type_slowly(&mut kit, "sat");
    wait_for(&mut kit, "the answer", |kit| {
        has_words(kit, "No names starting with")
    });
    assert!(
        options(&kit).is_empty(),
        "satoshi and satoshi-lab are the wallet's own"
    );
    for _ in 0..3 {
        kit.key_press(egui::Key::Backspace);
    }
    type_slowly(&mut kit, "ali");
    wait_for(&mut kit, "the network's names", |kit| {
        options(kit).len() == 3
    });
    assert_eq!(options(&kit)[0], "@alice, Alice Chen, In contacts");
    kit.key_press(egui::Key::ArrowDown);
    enter(&mut kit);
    let contacts = &kit.state().contacts;
    assert!(contacts.adding.is_none(), "no new request");
    assert_eq!(
        contacts.profile.as_deref(),
        Some("3Ah7Kp2Qx9Lm4Vt8Rn5Wc1Jd6Yb2Fs7Ge4Hu3Mz9Lp1"),
        "@alice's card"
    );
}

/// AC-A10, AC-D7, frame N7: «Strangers' pictures in name suggestions» stands right under
/// «Hide amounts» in Security, off; switched on, the core is told, and off again at once.
#[test]
fn the_strangers_pictures_switch_is_under_hide_amounts_and_reaches_the_core() {
    let (_turn, mut kit) = demo_app("security", "en");
    let switch = |kit: &Kit, name: &str| {
        kit.query_all_by_label(name)
            .find(|n| n.accesskit_node().role() != Role::Label)
            .unwrap_or_else(|| panic!("the switch «{name}»"))
            .rect()
    };
    let hide = switch(&kit, "Hide amounts");
    let pictures = switch(&kit, "Strangers' pictures in name suggestions");
    let fresh = switch(&kit, "A new address after each payment");
    assert!(hide.top() < pictures.top() && pictures.top() < fresh.top());
    assert!(
        !kit.state().settings.is_on("stranger-pictures"),
        "off by default"
    );
    assert!(has_words(&kit, "those servers see your IP"));
    let click = |kit: &mut Kit| {
        kit.query_all_by_label("Strangers' pictures in name suggestions")
            .find(|n| n.accesskit_node().role() != Role::Label)
            .expect("the switch")
            .click();
        kit.run_steps(3);
    };
    click(&mut kit);
    assert!(kit.state().settings.is_on("stranger-pictures"));
    assert_eq!(
        kit.state().core_told.map(|t| t.2),
        Some(true),
        "the core is told"
    );
    click(&mut kit);
    assert_eq!(
        kit.state().core_told.map(|t| t.2),
        Some(false),
        "off at once"
    );
}
