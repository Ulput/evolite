//! FEAT-0020 K1: «Keys · @x» lists every key of the identity — the acting ones by number, the
//! disabled ones last — with whose each is; Identity's summary and Settings → Security open it,
//! and it goes back to where it was opened from (AC-T17).

use super::*;
use crate::screens::Screen;

/// The table's row of key `id`: its words read whole start with «{id} · ».
fn row(kit: &Kit, id: u32) -> egui::Rect {
    let start = format!("{id} · ");
    kit.query_all_by(|n| {
        n.role() == Role::Label
            && (n.label().is_some_and(|l| l.starts_with(&start))
                || n.value().is_some_and(|v| v.starts_with(&start)))
    })
    .next()
    .unwrap_or_else(|| panic!("no row of key {id}"))
    .rect()
}

/// The words of key `id`'s row, read whole.
fn row_words(kit: &Kit, id: u32) -> String {
    let start = format!("{id} · ");
    let node = kit
        .query_all_by(|n| {
            n.label().is_some_and(|l| l.starts_with(&start))
                || n.value().is_some_and(|v| v.starts_with(&start))
        })
        .next()
        .unwrap_or_else(|| panic!("no row of key {id}"));
    {
        let n = node.accesskit_node();
        n.label()
            .or(n.value())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}

fn link_to(kit: &Kit, place: &str) -> bool {
    kit.query_by_role_and_label(Role::Link, &format!("Back: {place}"))
        .is_some()
}

/// The demo's identity: its keys one row each, by number, the disabled key 5 after the others;
/// whose each key is and its state in words.
#[test]
fn every_key_has_a_row_and_the_disabled_one_comes_last() {
    let (_turn, mut kit) = demo_app("identity-keys", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    wait_for(&mut kit, "K1", |k| k.state().screen == Screen::IdentityKeys);
    kit.run_steps(3);
    assert!(has_words(&kit, "Keys · "), "the title");
    assert!(has_text(&kit, "All keys"));
    let tops: Vec<f32> = [0, 1, 2, 3, 4, 6, 5]
        .iter()
        .map(|id| row(&kit, *id).top())
        .collect();
    assert!(
        tops.windows(2).all(|w| w[0] < w[1]),
        "by number, the disabled last: {tops:?}"
    );
    assert!(
        row_words(&kit, 0).contains("this wallet's"),
        "{}",
        row_words(&kit, 0)
    );
    assert!(row_words(&kit, 0).ends_with("active"));
    assert!(
        row_words(&kit, 3).contains("app Yappr"),
        "{}",
        row_words(&kit, 3)
    );
    assert!(
        row_words(&kit, 6).contains("Encryption · DashPay"),
        "{}",
        row_words(&kit, 6)
    );
    assert!(
        row_words(&kit, 5).contains("disabled "),
        "{}",
        row_words(&kit, 5)
    );
}

/// Identity's card «Manage keys ›» opens K1; its «‹ Identity» and Esc go back (AC-T17).
#[test]
fn identitys_manage_keys_opens_k1_and_goes_back() {
    let (_turn, mut kit) = demo_app("identity", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    kit.get_by_role_and_label(Role::Button, "Manage keys ›")
        .click();
    wait_for(&mut kit, "K1", |k| k.state().screen == Screen::IdentityKeys);
    kit.run_steps(3);
    assert!(link_to(&kit, "Identity"), "«‹ Identity»");
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    assert_eq!(kit.state().screen, Screen::Identity, "Esc goes back");

    kit.get_by_role_and_label(Role::Button, "Manage keys ›")
        .click();
    wait_for(&mut kit, "K1 again", |k| {
        k.state().screen == Screen::IdentityKeys
    });
    kit.run_steps(3);
    kit.get_by_role_and_label(Role::Link, "Back: Identity")
        .click();
    kit.run_steps(3);
    assert_eq!(kit.state().screen, Screen::Identity, "the link goes back");
}

/// Settings → Security: «Keys @x» opens K1 (AC-D10); «‹ Settings» goes back to Security.
#[test]
fn securitys_keys_row_opens_k1_and_goes_back_to_security() {
    let (_turn, mut kit) = demo_app("security", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    assert!(has_text(&kit, "Identity keys") || has_words(&kit, "IDENTITY KEYS"));
    let name = kit
        .query_all_by(|n| n.label().is_some_and(|l| l.starts_with("Keys @")))
        .next()
        .and_then(|n| n.accesskit_node().label().map(|s| s.to_string()))
        .expect("the row «Keys @x»");
    kit.query_all_by(|n| n.label().is_some_and(|l| l == name.as_str()))
        .next()
        .expect("the row")
        .click();
    kit.run_steps(3);
    wait_for(&mut kit, "K1", |k| k.state().screen == Screen::IdentityKeys);
    kit.run_steps(3);
    assert!(link_to(&kit, "Settings"), "«‹ Settings»");
    kit.get_by_role_and_label(Role::Link, "Back: Settings")
        .click();
    kit.run_steps(3);
    assert_eq!(kit.state().screen, Screen::Settings);
    assert!(
        kit.state().settings.section == crate::screens::settings::Section::Security,
        "the section it was opened from"
    );
}

const WIF: &str = evolite_core::DEMO_KEY_WIF;
const HEX: &str = evolite_core::DEMO_KEY_HEX;

/// The key's value is somewhere in the tree.
fn key_on_screen(kit: &Kit) -> bool {
    has_words(kit, WIF) || has_words(kit, HEX)
}

fn k1(preset: &str) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app(preset, "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    wait_for(&mut kit, "K1", |k| k.state().screen == Screen::IdentityKeys);
    kit.run_steps(3);
    (turn, kit)
}

/// R-10, AC-D2: «⋯» only on this wallet's acting keys — not on an app's (3, 4), another
/// wallet's (6) or the disabled one (5); Esc closes its menu and K1 stays.
#[test]
fn only_this_wallets_acting_keys_offer_the_private_key() {
    let (_turn, mut kit) = k1("identity-keys");
    let more = |kit: &Kit, id: u32| {
        kit.query_by_role_and_label(Role::Button, &format!("Key {id} — actions"))
            .is_some()
    };
    // Every acting key has its menu; the disabled one (5) none.
    for id in [0, 1, 2, 3, 4, 6] {
        assert!(more(&kit, id), "key {id}");
    }
    assert!(!more(&kit, 5), "the disabled key");
    // An app's key (3) is only disabled: its private key is not the phrase's.
    kit.get_by_role_and_label(Role::Button, "Key 3 — actions")
        .click();
    kit.run_steps(3);
    assert!(has_text(&kit, "Disable key…"));
    assert!(
        kit.query_by_role_and_label(Role::Button, "Show the private key")
            .is_none()
    );
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    kit.get_by_role_and_label(Role::Button, "Key 2 — actions")
        .click();
    kit.run_steps(3);
    assert!(button(&kit, "Show the private key").rect().width() > 0.0);
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    assert!(
        kit.query_by_role_and_label(Role::Button, "Show the private key")
            .is_none(),
        "the menu closed"
    );
    assert_eq!(kit.state().screen, Screen::IdentityKeys, "K1 stays");
}

/// AC-T13, T16: «Show the key» waits for «I understand» and the password; a wrong password
/// says so and shows nothing; the right one shows the demo's test key, WIF and hex, with
/// «Copy» buttons that carry no value (AC-T15).
#[test]
fn the_key_shows_after_i_understand_and_the_password() {
    let (_turn, mut kit) = k1("identity-keys");
    kit.get_by_role_and_label(Role::Button, "Key 2 — actions")
        .click();
    kit.run_steps(3);
    button(&kit, "Show the private key").click();
    kit.run_steps(3);
    assert!(has_words(&kit, "Whoever knows this key can act as"));
    let show = |kit: &Kit| !button(kit, "Show the key").accesskit_node().is_disabled();
    kit.get_by_role(Role::PasswordInput)
        .type_text(evolite_core::DEMO_WRONG_PASSWORD);
    kit.run_steps(3);
    assert!(!show(&kit), "not before «I understand»");
    kit.get_by(|n| {
        n.label()
            .is_some_and(|l| l.starts_with("I understand: this key"))
    })
    .click();
    kit.run_steps(3);
    assert!(show(&kit));
    button(&kit, "Show the key").click();
    wait_for(&mut kit, "Wrong password", |k| {
        has_text(k, "Wrong password")
    });
    assert!(!key_on_screen(&kit));
    // The right one (the demo takes any other).
    kit.state_mut()
        .identity_keys
        .reveal
        .as_mut()
        .unwrap()
        .password = zeroize::Zeroizing::new("demo".into());
    kit.run_steps(2);
    button(&kit, "Show the key").click();
    wait_for(&mut kit, "the key", key_on_screen);
    assert!(has_words(&kit, WIF) && has_words(&kit, HEX));
    for copy in ["Copy WIF", "Copy hex"] {
        assert!(button(&kit, copy).rect().width() > 0.0, "{copy}");
    }
    assert!(
        kit.query_all_by(|n| n.role() == Role::Button
            && n.label()
                .is_some_and(|l| l.contains(WIF) || l.contains(HEX)))
            .next()
            .is_none(),
        "no button carries the value"
    );
    assert!(
        kit.state()
            .identity_keys
            .reveal
            .as_ref()
            .is_some_and(|r| r.password.is_empty()),
        "the password wiped once the key is shown"
    );
}

/// AC-T13: a shown key goes with a lock — out of the tree and out of K1's state.
#[test]
fn a_lock_takes_the_shown_key() {
    let (_turn, mut kit) = k1("identity-key-shown");
    wait_for(&mut kit, "the key", key_on_screen);
    show_edited(
        &mut kit,
        1,
        "the key gone on the lock",
        |app| {
            if let Some(s) = app.snapshot.as_mut() {
                s.vault = evolite_api::VaultState::Locked;
            }
        },
        |k| k.state().identity_keys.reveal.is_none(),
    );
    kit.run_steps(3);
    assert!(!key_on_screen(&kit));
}

/// AC-T13: leaving K1 takes the shown key; «Hide» does too.
#[test]
fn leaving_k1_or_hide_takes_the_shown_key() {
    let (_turn, mut kit) = k1("identity-key-shown");
    wait_for(&mut kit, "the key", key_on_screen);
    button(&kit, "Hide").click();
    kit.run_steps(3);
    assert!(kit.state().identity_keys.reveal.is_none() && !key_on_screen(&kit));

    kit.state_mut().identity_keys.reveal_for_preset();
    wait_for(&mut kit, "the key again", key_on_screen);
    kit.state_mut().screen = Screen::Overview;
    kit.run_steps(3);
    assert!(kit.state().identity_keys.reveal.is_none(), "off K1");
    assert!(!key_on_screen(&kit));
}

/// The demo identity's key `id`, as K1 has it now.
fn key_now(kit: &Kit, id: u32) -> Option<evolite_api::IdentityKey> {
    kit.state()
        .snapshot
        .as_ref()?
        .wallet
        .as_ref()?
        .identity
        .as_ref()?
        .keys
        .iter()
        .find(|k| k.id == id)
        .cloned()
}

/// Types `password` into the open change dialog and presses its main button `act`.
fn confirm(kit: &mut Kit, password: &str, act: &str) {
    kit.get_by_role(Role::PasswordInput).type_text(password);
    kit.run_steps(3);
    button(kit, act).click();
    kit.run_steps(2);
}

/// K3, R-3, R-4, R-6: «Disable key…» says what stops working and that it is for good; a wrong
/// password keeps the dialog; the right one disables the key and K1 says the keys changed.
#[test]
fn a_key_is_disabled_after_the_password() {
    let (_turn, mut kit) = k1("identity-keys");
    open_disable(&mut kit, 1);
    assert!(has_text(&kit, "Disable key 1?"));
    assert!(has_words(
        &kit,
        "Signing in to apps and sites connected with this key"
    ));
    assert!(has_words(&kit, "Disabling is for good"));
    assert!(has_words(&kit, "roughly"), "the fee, roughly (AC-T7)");
    confirm(&mut kit, "demo", "Disable key");
    wait_for(&mut kit, "key 1 disabled", |k| {
        key_now(k, 1).is_some_and(|k| k.disabled_at.is_some())
    });
    wait_for(&mut kit, "the dialog closed", |k| {
        k.state().identity_keys.change.is_none()
    });
    assert!(has_words(&kit, "The keys changed"));
}

/// R-6: a wrong password says so and keeps the dialog; the key stays.
#[test]
fn a_wrong_password_keeps_the_key_and_the_dialog() {
    let (_turn, mut kit) = k1("identity-keys");
    open_disable(&mut kit, 1);
    confirm(&mut kit, evolite_core::DEMO_WRONG_PASSWORD, "Disable key");
    wait_for(&mut kit, "wrong password", |k| {
        has_words(k, "Wrong password — the keys did not change.")
    });
    assert!(key_now(&kit, 1).unwrap().disabled_at.is_none());
    assert!(kit.state().identity_keys.change.is_some());
}

/// Opens «Disable key…» from key `id`'s menu.
fn open_disable(kit: &mut Kit, id: u32) {
    kit.get_by_role_and_label(Role::Button, &format!("Key {id} — actions"))
        .click();
    kit.run_steps(3);
    button(kit, "Disable key…").click();
    kit.run_steps(3);
}

/// K4, R-2, AC-T3: the main key is replaced one for one — the new one at max+1, the old one
/// disabled in the same change.
#[test]
fn the_main_key_is_replaced_one_for_one() {
    let (_turn, mut kit) = k1("identity-keys");
    button(&kit, "Replace the main key").click();
    kit.run_steps(3);
    assert!(has_words(&kit, "0 · main · MASTER"));
    assert!(has_words(&kit, "7 · main · MASTER · from the phrase"));
    confirm(&mut kit, "demo", "Replace key");
    wait_for(&mut kit, "a new main key", |k| {
        key_now(k, 7).is_some_and(|k| k.level == evolite_api::KeyLevel::Master)
    });
    assert!(key_now(&kit, 0).unwrap().disabled_at.is_some());
}

/// K3b, АРХ-2: DashPay's encryption key is not disabled alone — its menu offers replacing the
/// pair, and that dialog says what it means for contacts.
#[test]
fn dashpays_key_is_only_replaced_as_a_pair() {
    let (_turn, mut kit) = k1("identity-keys");
    kit.get_by_role_and_label(Role::Button, "Key 6 — actions")
        .click();
    kit.run_steps(3);
    assert!(!has_text(&kit, "Disable key…"));
    button(&kit, "Replace the DashPay pair…").click();
    kit.run_steps(3);
    assert!(has_text(&kit, "Replace the DashPay pair"));
    assert!(has_words(
        &kit,
        "You will not be able to pay contacts whose requests point to the old key"
    ));
}

/// K5, AC-D6: «+ Add a key» offers the three ready purposes; one the identity has is dimmed
/// with why; the chosen one is added with the next number.
#[test]
fn a_key_is_added_for_a_ready_purpose() {
    let (_turn, mut kit) = k1("identity-keys");
    button(&kit, "+ Add a key").click();
    kit.run_steps(3);
    assert!(has_words(&kit, "with the next number — 7."));
    let add = |kit: &Kit| !button(kit, "Add key").accesskit_node().is_disabled();
    let choose = |kit: &mut Kit, name: &str| {
        kit.get_by_role_and_label(Role::RadioButton, name).click();
        kit.run_steps(3);
    };
    kit.get_by_role(Role::PasswordInput).type_text("demo");
    kit.run_steps(3);
    assert!(!add(&kit), "nothing chosen yet");
    // The identity has a key for tokens (2): that card is dimmed and not chosen.
    choose(&mut kit, "For tokens");
    assert!(!add(&kit), "tokens: it has key 2");
    choose(&mut kit, "For credit transfers");
    assert!(add(&kit));
    button(&kit, "Add key").click();
    wait_for(&mut kit, "key 7 for transfers", |k| {
        key_now(k, 7).is_some_and(|k| k.purpose == evolite_api::KeyPurpose::Transfers)
    });
}

/// АРХ-4: each visit to K1 reads the identity again — «checking…» until the answer, then
/// when; leaving K1 forgets it, so the next visit asks again.
#[test]
fn k1_reads_the_identity_again_on_each_visit() {
    let (_turn, mut kit) = k1("identity-keys");
    wait_for(&mut kit, "checked", |k| has_words(k, "checked just now"));
    kit.state_mut().screen = Screen::Identity;
    kit.run_steps(3);
    assert!(!has_words(&kit, "checked just now"));
    kit.state_mut().screen = Screen::IdentityKeys;
    kit.run_steps(1);
    assert!(
        has_words(&kit, "checking…") || has_words(&kit, "checked just now"),
        "asked again"
    );
    wait_for(&mut kit, "checked again", |k| {
        has_words(k, "checked just now")
    });
}

/// K7, AC-D8: an identity without keys DashPay can use gets «Enable DashPay…» on Contacts in
/// place of the list; it opens K1 with «For DashPay» chosen, and K1 shows the same card.
#[test]
fn contacts_enable_dashpay_opens_k1_with_the_pair_chosen() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    let strip = |app: &mut crate::app::App| {
        if let Some(id) = app
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
            .and_then(|w| w.identity.as_mut())
        {
            id.keys
                .retain(|k| k.purpose != evolite_api::KeyPurpose::Encryption);
        }
    };
    show_edited(&mut kit, 2, "the card", strip, |k| {
        has_words(k, "DashPay contacts for")
    });
    assert!(
        !has_words(&kit, "Contacts · "),
        "the card in place of the list"
    );
    button(&kit, "Enable DashPay…").click();
    wait_for(&mut kit, "K1", |k| k.state().screen == Screen::IdentityKeys);
    kit.run_steps(3);
    let chosen = kit
        .state()
        .identity_keys
        .change
        .as_ref()
        .and_then(|c| c.chosen);
    assert_eq!(chosen, Some(evolite_api::key_plan::KeyRequest::DashPay));
    assert!(has_text(&kit, "Add a key"));
}

/// AC-T18: K1 with no place to go back to (a cold start on its preset) goes to Identity, its
/// part — not Overview.
#[test]
fn k1_without_a_place_goes_back_to_identity() {
    let (_turn, mut kit) = k1("identity-keys");
    assert!(link_to(&kit, "Identity"), "«‹ Identity»");
    kit.get_by_role_and_label(Role::Link, "Back: Identity")
        .click();
    kit.run_steps(3);
    assert_eq!(kit.state().screen, Screen::Identity);
}

/// AC-T19: another wallet's unbound encryption key, numbered before ours would be, is not
/// DashPay's: its row says «unbound», DashPay's mark is on it, its menu disables it alone; K7
/// says contacts do not work and, with «I understand», adds our pair and disables it.
#[test]
fn another_wallets_encryption_key_is_not_dashpays() {
    let (_turn, mut kit) = k1("identity-keys");
    // The demo's key 6 made another wallet's: unbound, alone, so DashPay takes it.
    let theirs = |app: &mut crate::app::App| {
        if let Some(key) = app
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
            .and_then(|w| w.identity.as_mut())
            .and_then(|i| i.keys.iter_mut().find(|k| k.id == 6))
        {
            key.owner = evolite_api::KeyOwner::Other;
        }
    };
    show_edited(&mut kit, 2, "key 6 another wallet's", theirs, |k| {
        row_words(k, 6).contains("another wallet")
    });
    assert!(
        row_words(&kit, 6).contains("Encryption · unbound"),
        "{}",
        row_words(&kit, 6)
    );
    kit.get_by_role_and_label(Role::Button, "Key 6 — actions")
        .click();
    kit.run_steps(3);
    assert!(has_text(&kit, "Disable key…"));
    assert!(!has_text(&kit, "Replace the DashPay pair…"));
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    assert!(has_words(&kit, "so contacts do not work in evolite"));
    button(&kit, "Enable DashPay…").click();
    kit.run_steps(3);
    assert!(has_words(
        &kit,
        "Now that is key 6 of another wallet or an app"
    ));
    kit.get_by_role(Role::PasswordInput).type_text("demo");
    kit.run_steps(3);
    let add = |kit: &Kit| !button(kit, "Add key").accesskit_node().is_disabled();
    assert!(!add(&kit), "not before «I understand»");
    kit.get_by(|n| {
        n.label()
            .is_some_and(|l| l.starts_with("I understand: a disabled key"))
    })
    .click();
    kit.run_steps(3);
    assert!(add(&kit));
    button(&kit, "Add key").click();
    wait_for(&mut kit, "our pair, key 6 off", |k| {
        key_now(k, 6).is_some_and(|k| k.disabled_at.is_some()) && key_now(k, 8).is_some()
    });
}

/// The designer's Must 1: K3's «What stops working» is said in the interface's language, here
/// Russian for the HIGH key 1 (it stayed English behind a bullet).
#[test]
fn what_stops_is_translated() {
    let (_turn, mut kit) = demo_app("key-disable", "ru");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    wait_for(&mut kit, "K3", |k| {
        has_words(k, "Вход в приложения и на сайты, подключённые этим ключом")
    });
}

/// The designer's ar must (AC-D9): the identity's @name inside an Arabic sentence stays one
/// left-to-right run, «@» before the name — K7's card title and K1's title (as BUG-118, BUG-119).
#[test]
fn in_arabic_the_name_keeps_its_at_before_it() {
    let (_turn, mut kit) = demo_app("identity-keys-no-encryption", "ar");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    settle(&mut kit);
    wait_for(&mut kit, "K7", |k| has_words(k, "DashPay"));
    let texts: Vec<String> = kit
        .query_all_by(|n| {
            n.value().is_some_and(|v| v.contains("satoshi"))
                || n.label().is_some_and(|l| l.contains("satoshi"))
        })
        .filter_map(|n| {
            let n = n.accesskit_node();
            n.value().or(n.label()).map(|s| s.to_string())
        })
        .collect();
    assert!(!texts.is_empty(), "the name is on the screen");
    for t in &texts {
        assert!(!t.contains("satoshi@"), "«@» after the name: {t:?}");
    }
    assert!(texts.iter().any(|t| t.contains("@satoshi")), "{texts:?}");
}

/// The shown identity's snapshot, to change what the demo core said.
fn identity_mut(kit: &mut Kit) -> &mut evolite_api::Identity {
    kit.state_mut()
        .snapshot
        .as_mut()
        .and_then(|s| s.wallet.as_mut())
        .and_then(|w| w.identity.as_mut())
        .expect("an identity")
}

fn set_credits(kit: &mut Kit, credits: evolite_api::Duffs) {
    if let Some(w) = kit
        .state_mut()
        .snapshot
        .as_mut()
        .and_then(|s| s.wallet.as_mut())
    {
        w.balances.credits = credits;
        w.balances.credits_checked = true;
    }
}

/// BUG-183 AC-T4: K5 for transfers says ≈ 0.00024, K3b's pair ≈ 0.0008; with 0.0005 of
/// credits the pair is refused before the password.
#[test]
fn a_key_change_says_its_fee_by_the_keys_it_adds() {
    let (_turn, mut kit) = k1("identity-keys");
    button(&kit, "+ Add a key").click();
    kit.run_steps(3);
    kit.get_by_role_and_label(Role::RadioButton, "For credit transfers")
        .click();
    kit.run_steps(3);
    assert!(has_words(
        &kit,
        &format!("≈ {}", crate::format::amount(24_000))
    ));
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    kit.get_by_role_and_label(Role::Button, "Key 6 — actions")
        .click();
    kit.run_steps(3);
    button(&kit, "Replace the DashPay pair…").click();
    kit.run_steps(3);
    assert!(has_words(
        &kit,
        &format!("≈ {}", crate::format::amount(80_000))
    ));
    set_credits(&mut kit, 50_000);
    kit.get_by_role(Role::PasswordInput).type_text("demo");
    kit.run_steps(3);
    assert!(has_words(&kit, "Not enough credits: ≈"));
    assert!(
        button(&kit, "Replace the pair")
            .accesskit_node()
            .is_disabled()
    );
}

/// BUG-184 AC-T3: after its send the answer was «could not find out», and the snapshot shows
/// key 1 disabled: the dialog says done, not «no key 1 among the acting ones».
#[test]
fn a_disable_the_snapshot_shows_after_an_unclear_answer_is_done() {
    let (_turn, mut kit) = k1("identity-keys");
    open_disable(&mut kit, 1);
    if let Some(change) = &mut kit.state_mut().identity_keys.change {
        change.sent_once = true;
        change.result = Some(Err(evolite_api::CoreError::Failed("unclear".into())));
    }
    if let Some(key) = identity_mut(&mut kit).keys.iter_mut().find(|k| k.id == 1) {
        key.disabled_at = Some(1_793_000_000);
    }
    kit.run_steps(3);
    assert!(
        kit.state().identity_keys.change.is_none(),
        "the dialog closed"
    );
    assert!(!has_words(&kit, "among the identity's acting keys"));
    assert!(has_words(&kit, "The keys changed"));
}

/// BUG-184 AC-T3 (FEAT-0020 AC-T2 kept): before any send the plan's refusal is still said.
#[test]
fn before_a_send_the_plans_refusal_is_said() {
    let (_turn, mut kit) = k1("identity-keys");
    open_disable(&mut kit, 1);
    if let Some(key) = identity_mut(&mut kit).keys.iter_mut().find(|k| k.id == 1) {
        key.disabled_at = Some(1_793_000_000);
    }
    kit.run_steps(3);
    assert!(
        kit.state().identity_keys.change.is_some(),
        "still open, with the refusal"
    );
}

/// BUG-184 AC-T4: an added key the snapshot shows after an unclear answer is done, not «it has
/// one already».
#[test]
fn an_added_key_the_snapshot_shows_after_an_unclear_answer_is_done() {
    let (_turn, mut kit) = k1("identity-keys");
    button(&kit, "+ Add a key").click();
    kit.run_steps(3);
    kit.get_by_role_and_label(Role::RadioButton, "For credit transfers")
        .click();
    kit.run_steps(3);
    if let Some(change) = &mut kit.state_mut().identity_keys.change {
        change.sent_once = true;
        change.result = Some(Err(evolite_api::CoreError::Failed("unclear".into())));
    }
    let mut transfer = identity_mut(&mut kit).keys[0].clone();
    transfer.id = 7;
    transfer.purpose = evolite_api::KeyPurpose::Transfers;
    transfer.level = evolite_api::KeyLevel::Critical;
    transfer.disabled_at = None;
    identity_mut(&mut kit).keys.push(transfer);
    kit.run_steps(3);
    assert!(
        kit.state().identity_keys.change.is_none(),
        "the dialog closed"
    );
    assert!(has_words(&kit, "The keys changed"));
}

/// Says the core answered the open change dialog after its send, with `result`.
fn answered(kit: &mut Kit, result: Result<(), evolite_api::CoreError>) {
    if let Some(change) = &mut kit.state_mut().identity_keys.change {
        change.sent_once = true;
        change.waiting = false;
        change.result = Some(result);
    }
    kit.run_steps(3);
}

/// BUG-200: once the change was sent and answered, K3, K3b, K4 and K5 offer no second change
/// whatever the answer said — only «View the keys» and «Close». A plan worked out again on the
/// new keys («−9, +10») with its button on was a second paid change one click away.
#[test]
fn an_answered_change_offers_no_second_one() {
    let (_turn, mut kit) = k1("identity-keys");
    let failed = || Err(evolite_api::CoreError::Failed("unclear".into()));
    let none_of = |kit: &Kit| {
        ["Disable key", "Replace key", "Replace the pair", "Add key"]
            .iter()
            .all(|act| kit.query_by_role_and_label(Role::Button, act).is_none())
    };
    // K3: disable key 1.
    open_disable(&mut kit, 1);
    answered(&mut kit, failed());
    assert!(none_of(&kit), "K3 offers no change after the answer");
    assert!(has_text(&kit, "View the keys") && has_text(&kit, "Close"));
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    // K4: the main key.
    button(&kit, "Replace the main key").click();
    kit.run_steps(3);
    answered(&mut kit, failed());
    assert!(none_of(&kit), "K4 offers no change after the answer");
    button(&kit, "View the keys").click();
    kit.run_steps(3);
    assert!(
        kit.state().identity_keys.change.is_none(),
        "«View the keys» shows K1"
    );
    // K3b: DashPay's pair.
    kit.get_by_role_and_label(Role::Button, "Key 6 — actions")
        .click();
    kit.run_steps(3);
    button(&kit, "Replace the DashPay pair…").click();
    kit.run_steps(3);
    answered(&mut kit, failed());
    assert!(none_of(&kit), "K3b offers no change after the answer");
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    // K5: a new key.
    button(&kit, "+ Add a key").click();
    kit.run_steps(3);
    answered(&mut kit, failed());
    assert!(none_of(&kit), "K5 offers no change after the answer");
}

/// BUG-200: a wrong password sent nothing — the form stays for the right one (R-6 kept).
#[test]
fn a_wrong_password_is_no_send() {
    let (_turn, mut kit) = k1("identity-keys");
    open_disable(&mut kit, 1);
    confirm(&mut kit, evolite_core::DEMO_WRONG_PASSWORD, "Disable key");
    wait_for(&mut kit, "wrong password", |k| {
        has_words(k, "Wrong password — the keys did not change.")
    });
    assert!(
        kit.query_by_role_and_label(Role::Button, "Disable key")
            .is_some()
    );
}
