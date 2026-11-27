//! FEAT-0050 on the demo core: what Simple hides and Advanced shows, by the card's table (A3), at
//! once on a switch (A2); money in what is hidden said in Simple (R-5, A6); the mode in the
//! problem report.

use super::*;
use crate::screens::Screen;

/// `kit` in `advanced` mode, a couple of frames drawn.
fn mode(kit: &mut Kit, advanced: bool) {
    kit.state_mut().settings.set("advanced", advanced);
    kit.run_steps(3);
}

fn button_named(kit: &Kit, name: &str) -> bool {
    kit.query_all_by_role_and_label(Role::Button, name)
        .next()
        .is_some()
}

/// A3, A6: the Overview — the Platform address's tile and «Top up from Core ›» in Advanced; in
/// Simple neither, its money said in one line with the way to «Mode».
#[test]
fn the_platform_address_is_advanced_and_its_money_is_said_in_simple() {
    let (_turn, mut kit) = demo_app("overview", "en");
    wait_for(&mut kit, "the cards", |kit| has_words(kit, "Private pool"));
    let line = "you can manage it in Advanced mode";
    mode(&mut kit, false);
    wait_for(&mut kit, "the line", |kit| has_words(kit, line));
    assert!(has_words(&kit, "The Platform address holds"));
    assert!(
        !button_named(&kit, "Top up from Core ›"),
        "no tile in Simple"
    );
    // «Change the mode» opens Settings › Advanced, where the mode is.
    kit.query_all_by_label("Change the mode")
        .next()
        .expect("the way to the mode")
        .click();
    wait_for(&mut kit, "Settings › Advanced", |kit| {
        kit.state().screen == Screen::Settings && has_text(kit, "Advanced mode")
    });
    kit.state_mut().screen = Screen::Overview;
    mode(&mut kit, true);
    wait_for(&mut kit, "the tile", |kit| {
        button_named(kit, "Top up from Core ›")
    });
    assert!(!has_words(&kit, line), "no line in Advanced");
}

/// A3: Send — the Platform address as a source in Advanced only.
#[test]
fn send_offers_the_platform_address_in_advanced_only() {
    let (_turn, mut kit) = demo_app("send", "en");
    let offered = |kit: &Kit| {
        kit.query_all_by(|n| {
            n.role() == Role::RadioButton
                && n.label().is_some_and(|l| l.contains("Platform address"))
        })
        .next()
        .is_some()
    };
    mode(&mut kit, true);
    wait_for(&mut kit, "the sources", offered);
    mode(&mut kit, false);
    wait_for(&mut kit, "no Platform address", |kit| !offered(kit));
}

/// A3, R-6: Identity — the keys and «+ New identity» in Advanced only; the identity itself in
/// both. A2: its keys open, Simple goes back to the identity at once.
#[test]
fn identity_keys_and_a_new_identity_are_advanced() {
    let (_turn, mut kit) = demo_app("identity", "en");
    let new_one = |kit: &Kit| button_named(kit, "Create a new identity");
    mode(&mut kit, true);
    wait_for(&mut kit, "Advanced", |kit| {
        new_one(kit) && has_text(kit, "Keys")
    });
    mode(&mut kit, false);
    wait_for(&mut kit, "Simple", |kit| {
        !new_one(kit) && !has_text(kit, "Keys")
    });
    assert!(has_words(&kit, "Work as"), "the identity still shown");
    mode(&mut kit, true);
    kit.state_mut().screen = Screen::IdentityKeys;
    kit.run_steps(2);
    mode(&mut kit, false);
    wait_for(&mut kit, "back to the identity", |kit| {
        kit.state().screen == Screen::Identity
    });
}

/// A3: Settings › Network — the own node and «Sync again» in Advanced only.
#[test]
fn the_node_and_a_new_sync_are_advanced() {
    let (_turn, mut kit) = demo_app("settings", "en");
    kit.state_mut().settings.section = crate::screens::settings::Section::Network;
    mode(&mut kit, true);
    wait_for(&mut kit, "Advanced", |kit| {
        has_text(kit, "Own node") && button_named(kit, "Sync again")
    });
    mode(&mut kit, false);
    wait_for(&mut kit, "Simple", |kit| {
        !has_text(kit, "Own node") && !button_named(kit, "Sync again")
    });
    assert!(has_text(&kit, "Core nodes"), "the network itself shown");
}

/// The problem report says the mode (support: where a button is depends on it).
#[test]
fn the_report_says_the_mode() {
    let (_turn, mut kit) = demo_app("report", "en");
    for (advanced, words) in [(false, "Simple mode"), (true, "Advanced mode")] {
        mode(&mut kit, advanced);
        let snapshot = kit.state().snapshot.clone().expect("a snapshot");
        let mut commands = Vec::new();
        let mut go = None;
        let mut cx = crate::context::Ctx::new(&snapshot, &mut commands, &mut go);
        cx.show_advanced(advanced);
        let system = crate::screens::settings::report::system(&cx, true, 1.0);
        let said = system.iter().find(|(label, _)| *label == "Mode");
        assert_eq!(said.map(|(_, v)| v.as_str()), Some(words), "{system:?}");
    }
}
