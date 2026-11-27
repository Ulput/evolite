//! BUG-286, BUG-298: the keys other wallets need, on the demo core — the last sign-in key
//! said before the password (and replaced by default), App Connect refused before its dialog
//! without one, and the warning on Contacts, Identity and the keys screen with «Add the missing
//! keys» (one update) and «Not now».

use super::*;
use crate::screens::Screen;
use evolite_api::Command;
use evolite_api::key_plan::KeyRequest;

fn keys_screen() -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("identity-keys", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    wait_for(&mut kit, "K1", |k| k.state().screen == Screen::IdentityKeys);
    kit.run_steps(3);
    (turn, kit)
}

fn identity(kit: &Kit) -> evolite_api::Identity {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .and_then(|w| w.identity.clone())
        .expect("an identity")
}

/// An acting sign-in key of this wallet other than key 1.
fn new_sign_in(kit: &Kit) -> Option<u32> {
    identity(kit)
        .keys
        .iter()
        .find(|k| {
            k.id != 1
                && k.disabled_at.is_none()
                && k.purpose == evolite_api::KeyPurpose::Documents
                && k.level == evolite_api::KeyLevel::High
                && k.owner == evolite_api::KeyOwner::This
        })
        .map(|k| k.id)
}

fn key_1_disabled(kit: &Kit) -> bool {
    identity(kit)
        .keys
        .iter()
        .any(|k| k.id == 1 && k.disabled_at.is_some())
}

/// The button whose name starts with `words` (its fee follows).
fn button_starting<'k>(kit: &'k Kit, words: &'k str) -> Option<egui_kittest::Node<'k>> {
    kit.query_all_by(move |n| {
        n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with(words))
    })
    .next()
}

fn open_disable_1(kit: &mut Kit) {
    kit.get_by_role_and_label(Role::Button, "Key 1 — actions")
        .click();
    kit.run_steps(3);
    button(kit, "Disable key…").click();
    kit.run_steps(3);
}

fn confirm(kit: &mut Kit, act: &str) {
    // The button pressed before (the warning's) is gone: the field is clicked, as a person does.
    kit.get_by_role(Role::PasswordInput).click();
    kit.run_steps(2);
    kit.get_by_role(Role::PasswordInput).type_text("demo");
    kit.run_steps(3);
    button(kit, act).click();
    kit.run_steps(2);
}

/// BUG-298 п. 2: disabling the last sign-in key is said before any password, and «Replace with a
/// new one» (the default) disables it and adds a new one in the same update. Mutant: no warning
/// (`warns_last_sign_in` false) → the password is asked at once.
#[test]
fn the_last_sign_in_key_is_said_and_replaced_by_default() {
    let (_turn, mut kit) = keys_screen();
    open_disable_1(&mut kit);
    assert!(has_words(&kit, "This is the last sign-in key."));
    assert!(has_words(&kit, "Dash Desktop Wallet"));
    assert!(
        kit.query_by_role(Role::PasswordInput).is_none(),
        "no password before the choice"
    );
    button(&kit, "Replace with a new one").click();
    kit.run_steps(3);
    assert!(has_text(&kit, "Replace the sign-in key"));
    assert!(has_words(&kit, "sign-in · HIGH · from the phrase"));
    confirm(&mut kit, "Replace key");
    wait_for(&mut kit, "key 1 replaced", |k| {
        key_1_disabled(k) && new_sign_in(k).is_some()
    });
}

/// BUG-298 п. 1 and п. 3: «Disable anyway» goes; then App Connect is refused before its dialog
/// (not after the password and the key's fee), and the keys screen's warning adds the sign-in
/// key back in one update. Mutant: the demo's (the core's) check of `NoSignInKey` gone → the
/// request's dialog opens.
#[test]
fn without_a_sign_in_key_app_connect_is_refused_and_the_key_is_added_back() {
    let (_turn, mut kit) = keys_screen();
    open_disable_1(&mut kit);
    button(&kit, "Disable anyway").click();
    kit.run_steps(3);
    assert!(has_words(&kit, "Disabling is for good"));
    confirm(&mut kit, "Disable key");
    wait_for(&mut kit, "key 1 disabled", key_1_disabled);
    wait_for(&mut kit, "the dialog closed", |k| {
        k.state().identity_keys.change.is_none()
    });

    // A link is opened on Apps (pasted there), where its dialogs are drawn.
    kit.query_all_by_role_and_label(Role::Button, "Apps")
        .next()
        .expect("the rail's Apps")
        .click();
    wait_for(&mut kit, "Apps", |k| k.state().screen == Screen::Apps);
    kit.state_mut().core.send(Command::OpenAppLink {
        text: "dashpay://connect?v=2&n=t&r=AQ".into(),
        ask: None,
    });
    wait_for(&mut kit, "the refusal", |k| {
        k.state().requests.refused.is_some()
    });
    kit.run_steps(3);
    assert!(kit.state().requests.open.is_none(), "no request's dialog");
    assert!(has_text(&kit, "Cannot connect"));
    assert!(has_words(&kit, "no sign-in key (HIGH): add one in Keys"));
    button(&kit, "Got it").click();
    kit.run_steps(3);
    kit.state_mut().screen = Screen::IdentityKeys;
    kit.run_steps(5);

    assert!(has_words(&kit, "The identity has no sign-in key (HIGH)"));
    assert!(
        button_starting(&kit, "Not now").is_none(),
        "the keys screen always says it"
    );
    button_starting(&kit, "Add the missing keys")
        .expect("the warning's button")
        .click();
    kit.run_steps(3);
    assert!(has_text(&kit, "Add the missing keys"));
    assert!(has_words(&kit, "For signing in"));
    confirm(&mut kit, "Add keys");
    wait_for(&mut kit, "the sign-in key added", |k| {
        new_sign_in(k).is_some()
    });
    wait_for(&mut kit, "the warning gone", |k| {
        !has_words(k, "The identity has no sign-in key (HIGH)")
    });
}

/// The demo identity as one another wallet made: no decryption key.
fn without_decryption(app: &mut crate::app::App) {
    if let Some(id) = app
        .snapshot
        .as_mut()
        .and_then(|s| s.wallet.as_mut())
        .and_then(|w| w.identity.as_mut())
    {
        id.keys.retain(|k| !k.decrypts);
    }
}

const NO_DECRYPTION: &str = "the identity has no DashPay decryption key";

/// BUG-286 п. 2(а), §8d: on Contacts and Identity, at 1024 and at 800 × 440, the warning and its
/// button are on screen, within the window; with the decryption key, none. «Not now» hides it
/// and is the wallet's setting, by identity. Mutant: `missing_keys` empty → no warning.
#[test]
fn a_missing_decryption_key_is_said_on_contacts_and_identity() {
    for preset in ["contacts", "identity"] {
        for size in [egui::vec2(1024.0, 768.0), egui::vec2(800.0, 440.0)] {
            let (_turn, mut kit) = demo_app(preset, "en");
            kit.set_size(size);
            settle(&mut kit);
            assert!(
                !has_words(&kit, NO_DECRYPTION),
                "{preset}: with key 7, none"
            );
            show_edited(&mut kit, 2, "the warning", without_decryption, |k| {
                has_words(k, NO_DECRYPTION)
            });
            let add = button_starting(&kit, "Add the missing keys")
                .expect("the button")
                .rect();
            assert!(
                add.right() <= size.x && add.bottom() <= size.y,
                "{preset} at {size:?}: {add:?} off the window"
            );
            button(&kit, "Not now").click();
            show_edited(&mut kit, 2, "hidden", without_decryption, |k| {
                !has_words(k, NO_DECRYPTION)
            });
            let me = identity(&kit).id;
            let settings = &kit.state().settings;
            assert!(settings.keys_not_now.contains_key(&me));
            let trusted = std::collections::HashSet::new();
            assert!(
                settings.prefs(&trusted).keys_not_now.contains_key(&me),
                "saved"
            );
        }
    }
}

/// BUG-286, BUG-298: «Add the missing keys» on Contacts opens the keys screen with one dialog
/// of what is missing, not K5's choice.
#[test]
fn the_missing_keys_open_one_dialog_from_contacts() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    show_edited(&mut kit, 2, "the warning", without_decryption, |k| {
        has_words(k, NO_DECRYPTION)
    });
    button_starting(&kit, "Add the missing keys")
        .expect("the button")
        .click();
    show_edited(&mut kit, 2, "K1", without_decryption, |k| {
        k.state().screen == Screen::IdentityKeys
    });
    let change = kit
        .state()
        .identity_keys
        .change
        .as_ref()
        .expect("the dialog");
    assert_eq!(
        change.kind,
        crate::screens::identity::key_change::ChangeKind::AddMissing
    );
    assert_eq!(change.missing, vec![KeyRequest::DashPayDecryption]);
    assert!(has_text(&kit, "Add the missing keys"));
    assert!(has_words(&kit, "For DashPay decryption"));
}

/// FEAT-0050 r1, question 3: in Simple mode «Add the missing keys» opens the keys screen with
/// its dialog too — the keys screen is otherwise Advanced only, and without the keys signing in
/// to apps and NFTs break. Mutant: Simple sends every key change back → red.
#[test]
fn in_simple_mode_the_missing_keys_still_open_their_dialog() {
    for preset in ["contacts", "identity"] {
        let (_turn, mut kit) = demo_app(preset, "en");
        kit.set_size(egui::vec2(1280.0, 1000.0));
        kit.state_mut().settings.set("advanced", false);
        settle(&mut kit);
        show_edited(&mut kit, 2, "the warning", without_decryption, |k| {
            has_words(k, NO_DECRYPTION)
        });
        button_starting(&kit, "Add the missing keys")
            .expect("the button")
            .click();
        show_edited(&mut kit, 2, "the keys screen", without_decryption, |k| {
            k.state().screen == Screen::IdentityKeys
        });
        // Several frames on: Simple does not send it back.
        for _ in 0..5 {
            draw_edited(&mut kit, 1, without_decryption);
        }
        assert_eq!(kit.state().screen, Screen::IdentityKeys, "{preset}");
        assert!(
            !kit.state().settings.is_on("advanced"),
            "{preset}: still Simple"
        );
        let change = kit
            .state()
            .identity_keys
            .change
            .as_ref()
            .expect("the dialog");
        assert_eq!(
            change.kind,
            crate::screens::identity::key_change::ChangeKind::AddMissing,
            "{preset}"
        );
        assert!(has_words(&kit, "For DashPay decryption"), "{preset}");
    }
}

/// BUG-298 п. 1: without a sign-in key the Workshop says so in place of «Create NFT», not as an
/// error after the password. Mutant: `no_sign_in_key` none → the button on.
#[test]
fn without_a_sign_in_key_the_workshop_says_it_before_creating() {
    let (_turn, mut kit) = demo_app("workshop", "en");
    settle(&mut kit);
    assert!(!button(&kit, "Create NFT").accesskit_node().is_disabled());
    let no_sign_in = |app: &mut crate::app::App| {
        if let Some(key) = app
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
            .and_then(|w| w.identity.as_mut())
            .and_then(|i| i.keys.iter_mut().find(|k| k.id == 1))
        {
            key.disabled_at = Some(1_762_300_800);
        }
    };
    show_edited(&mut kit, 2, "the line", no_sign_in, |k| {
        has_words(k, "no sign-in key (HIGH): add one in Keys")
    });
    assert!(button(&kit, "Create NFT").accesskit_node().is_disabled());
}

/// BUG-286 r1 Should 1: «Not now» covers what was missing then. The decryption key's warning
/// put off, then the sign-in key goes too: the warning is back on Contacts, saying it.
/// Mutant: «Not now» by identity alone (`not_now_covers` true for any entry) → it stays hidden.
#[test]
fn not_now_for_one_key_does_not_hide_another_missing_later() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    show_edited(&mut kit, 2, "the warning", without_decryption, |k| {
        has_words(k, NO_DECRYPTION)
    });
    button(&kit, "Not now").click();
    show_edited(&mut kit, 2, "hidden", without_decryption, |k| {
        !has_words(k, NO_DECRYPTION)
    });
    let without_both = |app: &mut crate::app::App| {
        without_decryption(app);
        if let Some(key) = app
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
            .and_then(|w| w.identity.as_mut())
            .and_then(|i| i.keys.iter_mut().find(|k| k.id == 1))
        {
            key.disabled_at = Some(1_762_300_800);
        }
    };
    show_edited(
        &mut kit,
        2,
        "back with the sign-in key",
        without_both,
        |k| has_words(k, "The identity has no sign-in key (HIGH)") && has_words(k, NO_DECRYPTION),
    );
}

/// BUG-286 r1 Should 2: the demo core's identity without a sign-in key (key 1 disabled by its
/// own update) and a `dash-key:` v2 link: refused with `NoSignInKey` before any dialog, and
/// nothing is spent — the credits and the keys as they were. Mutant: the demo's check gone →
/// the request's dialog opens.
#[test]
fn a_dash_key_link_without_a_sign_in_key_is_refused_before_anything() {
    let (_turn, mut kit) = demo_app("apps", "en");
    settle(&mut kit);
    let id = identity(&kit).id;
    kit.state_mut().core.send(Command::Spend {
        op: evolite_api::Spend::UpdateKeys {
            identity: id,
            add: vec![],
            disable: vec![1],
        },
        password: Some(evolite_api::Password::new("demo".into())),
        ask: 0,
        large_confirmed: false,
    });
    wait_for(&mut kit, "key 1 disabled", key_1_disabled);
    let credits = |k: &Kit| {
        k.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .map(|w| w.balances.credits)
    };
    let (credits_before, keys_before) = (credits(&kit), identity(&kit).keys);
    kit.state_mut().core.send(Command::OpenAppLink {
        text: "dash-key:3mJr7AoUXx2Wqd?n=t&v=2".into(),
        ask: None,
    });
    wait_for(&mut kit, "the refusal", |k| {
        k.state().requests.refused.is_some()
    });
    assert_eq!(
        kit.state().requests.refused,
        Some(evolite_api::AppLinkRefusal::NoSignInKey)
    );
    kit.run_steps(5);
    assert!(kit.state().requests.open.is_none(), "no request's dialog");
    assert!(has_words(&kit, "no sign-in key (HIGH): add one in Keys"));
    assert_eq!(credits(&kit), credits_before, "nothing spent");
    assert_eq!(identity(&kit).keys, keys_before, "no key added");
}
