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

/// BUG-252: while the wallet is locked the app says nothing — the core keeps it off; at the unlock
/// it is said once and the core has it on. (The demo has no lock: the locked snapshot is drawn.)
#[test]
fn a_locked_wallet_is_told_nothing_and_the_unlock_tells_it() {
    let (_turn, mut kit) = demo_app("advanced", "en");
    kit.state_mut().settings.set("advanced", true);
    wait_for(&mut kit, "the core told", |kit| core_has(kit, true));
    // The lock turns it off in the core.
    kit.state_mut()
        .core
        .send(evolite_api::Command::AdvancedFeatures { on: false });
    let locked = |app: &mut App| {
        if let Some(s) = app.snapshot.as_mut() {
            s.vault = evolite_api::VaultState::Locked;
            s.advanced_allowed = false;
        }
    };
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(1500) {
        draw_edited(&mut kit, 1, locked);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(core_has(&kit, false), "nothing said while locked");
    assert!(
        kit.state().settings.is_on("advanced"),
        "the switch on screen stays on"
    );
    // The unlock: the snapshot says so (the demo does not send one by itself).
    show_edited(
        &mut kit,
        2,
        "said at the unlock",
        |app| {
            if let Some(s) = app.snapshot.as_mut() {
                s.vault = evolite_api::VaultState::Unlocked;
                s.advanced_allowed = true;
            }
        },
        |kit| core_has(kit, true),
    );
}
