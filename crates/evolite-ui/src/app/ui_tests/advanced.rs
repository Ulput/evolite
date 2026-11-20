//! FEAT-0040…0046, the shared «Features for advanced users» switch in Settings › Advanced, on the
//! demo core: off for a new user; turned on and off with no password and no restart; the core is
//! told, and told again when it has the switch otherwise (after the lock, which turns it off there).

use super::*;

fn core_has(kit: &Kit, on: bool) -> bool {
    kit.state()
        .snapshot
        .as_ref()
        .is_some_and(|s| s.advanced_features == on)
}

/// AC-A-X1, X2: the section and its switch, off; on, the core has it; off again, at once.
#[test]
fn the_switch_turns_on_and_off_with_no_password() {
    let (_turn, mut kit) = demo_app("advanced", "en");
    wait_for(&mut kit, "the section", |kit| {
        has_text(kit, "Features for advanced users")
    });
    assert!(has_words(&kit, "Most people do not need them."));
    assert!(has_words(
        &kit,
        "money and wallets already made are not touched"
    ));
    assert!(
        !kit.state().settings.is_on("advanced"),
        "off for a new user"
    );
    assert!(core_has(&kit, false));
    kit.get_by_role_and_label(Role::CheckBox, "Features for advanced users")
        .click();
    kit.run_steps(2);
    assert!(kit.state().settings.is_on("advanced"), "no password asked");
    wait_for(&mut kit, "the core told", |kit| core_has(kit, true));
    kit.get_by_role_and_label(Role::CheckBox, "Features for advanced users")
        .click();
    wait_for(&mut kit, "off again", |kit| core_has(kit, false));
}

/// The core has it off while the app has it on (as after the lock): the app says it again, once.
#[test]
fn the_core_is_told_again_when_it_has_it_off() {
    let (_turn, mut kit) = demo_app("advanced", "en");
    kit.state_mut().settings.set("advanced", true);
    wait_for(&mut kit, "the core told", |kit| core_has(kit, true));
    kit.state_mut()
        .core
        .send(evolite_api::Command::AdvancedFeatures { on: false });
    wait_for(&mut kit, "said again", |kit| core_has(kit, true));
}
