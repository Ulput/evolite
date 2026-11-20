//! Р1 (design REL-verify, review r1): the update check on the demo core, which answers with a fixed
//! release and never asks the network — About says what is out with its links and the keys, Settings
//! has a dot (said to a screen reader), the switch in Privacy turns it all off, and a security fix
//! is said on the Overview, closable.

use super::*;

const NEWER: &str = "evolite 1.0.1 is out";

fn update_known(kit: &Kit) -> bool {
    kit.state()
        .snapshot
        .as_ref()
        .is_some_and(|s| s.update.as_ref().is_some_and(|u| u.newer))
}

/// On by default: About says the new version, opens its release page, and names the keys.
#[test]
fn about_says_a_newer_release_with_its_links() {
    let (_turn, mut kit) = demo_app("about", "en");
    wait_for(&mut kit, "the demo's release", update_known);
    wait_for(&mut kit, NEWER, |kit| has_words(kit, NEWER));
    assert!(has_words(&kit, "Check the installer before you install it"));
    assert!(
        has_words(&kit, "Published with the first signed release"),
        "no keys yet"
    );
    assert!(has_words(&kit, "a signed answer"), "when it was checked");
    let settings = button(&kit, "Settings");
    assert!(
        settings
            .accesskit_node()
            .data()
            .description()
            .is_some_and(|d| d.contains("new version")),
        "the dot, said"
    );
    let before = crate::links::opened().len();
    button(&kit, "Release page").click();
    kit.run_steps(2);
    assert_eq!(
        crate::links::opened()[before..],
        ["https://github.com/Ulput/evolite/releases/tag/v1.0.1".to_owned()]
    );
}

/// Off in Privacy: nothing is asked, what was learnt goes, and About says the check is off.
#[test]
fn the_switch_in_privacy_turns_it_off() {
    let (_turn, mut kit) = demo_app("security", "en");
    wait_for(&mut kit, "the demo's release", update_known);
    assert!(has_text(&kit, "Check for a new version"));
    assert!(has_words(&kit, "GitHub sees your IP"));
    kit.state_mut().settings.set("update-check", false);
    wait_for(&mut kit, "the release forgotten", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.update.is_none())
    });
}

/// A security fix: said on the Overview; «Close» takes it away for this run.
#[test]
fn a_security_fix_is_said_on_the_overview() {
    let (_turn, mut kit) = demo_app("overview", "en");
    wait_for(&mut kit, "the demo's release", update_known);
    let critical = |app: &mut App| {
        if let Some(u) = app.snapshot.as_mut().and_then(|s| s.update.as_mut()) {
            u.critical = true;
        }
    };
    let said = "evolite 1.0.1 fixes a security problem";
    show_edited(&mut kit, 2, said, critical, |kit| has_words(kit, said));
    let before = crate::links::opened().len();
    button(&kit, "How to check the installer").click();
    kit.run_steps(1);
    assert_eq!(
        crate::links::opened()[before..],
        ["https://github.com/Ulput/evolite/blob/main/docs/verify.md".to_owned()]
    );
    kit.query_all_by_role_and_label(Role::Button, "Close")
        .next()
        .expect("the notice's «×»")
        .click();
    draw_edited(&mut kit, 3, critical);
    assert!(!has_words(&kit, said), "closed for this run");
}
