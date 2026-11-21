//! FEAT-0042: «Account public key (xpub)» on Settings › Backup (the demo core gives BIP32's
//! published test vector, never a wallet's key).

use super::*;
use evolite_api::Command;

fn advanced_on(kit: &mut Kit) {
    kit.state_mut().settings.set("advanced", true);
    kit.state()
        .core
        .send(Command::AdvancedFeatures { on: true });
    wait_for(kit, "the switch in the core", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.advanced_features)
    });
}

fn press_show(kit: &mut Kit) {
    // The dialog's «Show» is the last one (the page's are behind it).
    kit.query_all_by_role_and_label(Role::Button, "Show")
        .last()
        .expect("«Show»")
        .click();
    kit.run_steps(3);
}

/// AC-A8 (UC-4): with the advanced switch off there is no «Public key» group.
#[test]
fn the_public_key_is_behind_the_advanced_switch() {
    let (_turn, mut kit) = demo_app("backup", "en");
    kit.set_size(egui::vec2(1024.0, 1600.0));
    kit.run_steps(3);
    assert!(!has_text(&kit, "Account public key (xpub)"));
    advanced_on(&mut kit);
    kit.run_steps(3);
    assert!(has_text(&kit, "Account public key (xpub)"));
}

/// UC-1, UC-2, AC-A2, AC-A6: the warning before the password; a wrong one says so and shows
/// nothing; the right one shows the key whole with its path and «only Core» — and «Copy»
/// copies it; the lock closes the window and the key goes.
#[test]
fn the_key_comes_after_the_password_and_goes_with_the_lock() {
    let (_turn, mut kit) = demo_app("backup", "en");
    kit.set_size(egui::vec2(1024.0, 1600.0));
    advanced_on(&mut kit);
    kit.state_mut().settings.xpub_dialog = true;
    kit.run_steps(3);
    assert!(has_words(&kit, "It cannot spend."), "the warning first");
    assert!(!has_words(&kit, "tpub"), "no key before the password");
    kit.state_mut().settings.xpub_password =
        zeroize::Zeroizing::new(evolite_core::DEMO_WRONG_PASSWORD.into());
    press_show(&mut kit);
    wait_for(&mut kit, "the wrong password", |kit| {
        has_words(kit, "Wrong password")
    });
    assert!(!has_words(&kit, "tpub"));
    kit.state_mut().settings.xpub_password = zeroize::Zeroizing::new("main-wallet-26".into());
    press_show(&mut kit);
    wait_for(&mut kit, "the key", |kit| {
        has_words(
            kit,
            "tpubD6NzVbkrYhZ4XgiXtGrdW5XDAPFCL9h7we1vwNCpn8tGbBcgfVYjXyhWo4E1xkh56hjod1RhGjxbaTLV3X4FyWuejifB9jusQ46QzG87VKY",
        )
    });
    assert!(has_words(&kit, "m/44'/1'/0' · testnet"));
    assert!(has_words(&kit, "Core payments of account 0 only"));
    assert!(
        kit.state().settings.xpub_password.is_empty(),
        "the password gone"
    );
    button(&kit, "Copy").click();
    kit.run_steps(2);
    assert!(has_text(&kit, "Copied"));
    // The lock: what the app does on it (`keys_gone` → `forget_secrets`; the demo core has no
    // lock of its own).
    kit.state_mut().settings.forget_secrets();
    kit.run_steps(3);
    assert!(!kit.state().settings.xpub_dialog && kit.state().settings.xpub.is_none());
    assert!(!has_words(&kit, "tpubD6Nz"), "the key gone");
}
