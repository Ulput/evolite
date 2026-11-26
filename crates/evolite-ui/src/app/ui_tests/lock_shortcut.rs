//! FEAT-0057 П8 on the demo core: the lock beside Settings and Ctrl+L (⌘L) lock the open wallet
//! from anywhere, a text field too, and forget what was typed; with no wallet open they do
//! nothing.

use super::*;
use crate::screens::Screen;

fn locked(kit: &Kit) -> bool {
    kit.state().screen == Screen::Unlock
}

fn press_lock(kit: &mut Kit) {
    kit.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
    kit.run_steps(2);
}

/// A8.1: Ctrl+L on the Overview locks.
#[test]
fn ctrl_l_locks_from_the_overview() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.run_steps(3);
    press_lock(&mut kit);
    wait_for(&mut kit, "the unlock screen", locked);
}

/// A8.1: Ctrl+L with the amount field focused locks too, and the amount typed is gone.
#[test]
fn ctrl_l_locks_from_a_field_and_forgets_the_amount() {
    let (_turn, mut kit) = demo_app("send", "en");
    kit.run_steps(3);
    kit.state_mut().send.amount = "0.5".into();
    kit.run_steps(2);
    kit.query_all_by(|n| n.role() == Role::TextInput && n.value().is_some_and(|v| v == "0.5"))
        .next()
        .expect("the amount field")
        .click();
    kit.run_steps(2);
    press_lock(&mut kit);
    wait_for(&mut kit, "the unlock screen", locked);
    assert!(kit.state().send.amount.is_empty(), "the amount forgotten");
}

/// A8.1, A8.3: the lock in the menu, a button named «Lock», locks as «Lock now» does; in the
/// rail (1024) too, and in Arabic.
#[test]
fn the_lock_in_the_menu_locks() {
    for (size, lang) in [
        (egui::vec2(1280.0, 800.0), "en"),
        (egui::vec2(1024.0, 768.0), "en"),
        (egui::vec2(1280.0, 800.0), "ar"),
    ] {
        let (_turn, mut kit) = demo_app("overview", lang);
        kit.set_size(size);
        kit.run_steps(3);
        let name = crate::i18n::spoken("Lock");
        kit.get_by_role_and_label(Role::Button, &name).click();
        wait_for(&mut kit, "the unlock screen", locked);
        assert!(
            kit.query_all_by_role_and_label(Role::Button, &name)
                .next()
                .is_none(),
            "{lang} {size:?}: no lock on the unlock screen"
        );
    }
}

/// A8.4, A8.2: on the Welcome (no wallet open) neither the lock nor Ctrl+L does anything.
#[test]
fn with_no_wallet_open_ctrl_l_does_nothing() {
    let (_turn, mut kit) = demo_app("welcome", "en");
    kit.run_steps(3);
    let before = kit.state().screen;
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Lock")
            .next()
            .is_none(),
        "no lock"
    );
    press_lock(&mut kit);
    kit.run_steps(3);
    assert_eq!(kit.state().screen, before);
}

/// The designer, 2026-11-26 (П8 in the rail): the lock and Settings in one row at the rail's
/// foot, 32 px each and 4 apart, the lock first as read (on the right in Arabic); the rail no
/// higher for it (FEAT-0036 AC-T2 is `low_menu`'s).
#[test]
fn in_the_rail_the_lock_and_settings_share_one_row() {
    for lang in ["en", "ar"] {
        let (_turn, mut kit) = demo_app("overview", lang);
        kit.set_size(egui::vec2(1024.0, 768.0));
        kit.run_steps(3);
        let rect = |name: &str| {
            kit.get_by_role_and_label(Role::Button, &crate::i18n::spoken(name))
                .rect()
        };
        let (lock, settings) = (rect("Lock"), rect("Settings"));
        assert_eq!(lock.size(), egui::vec2(32.0, 32.0), "{lang}: the lock");
        assert_eq!(settings.size(), egui::vec2(32.0, 32.0), "{lang}: Settings");
        assert_eq!(lock.center().y, settings.center().y, "{lang}: one row");
        let (left, right) = if lang == "ar" {
            (settings, lock)
        } else {
            (lock, settings)
        };
        assert_eq!(
            right.left() - left.right(),
            4.0,
            "{lang}: 4 apart, the lock first"
        );
        assert!(
            settings.bottom() <= 768.0 - 20.0 + 0.5,
            "{lang}: the foot where it was"
        );
    }
}

/// r2 (Should 1): in the wide menu Tab reaches the lock, then Settings — as they are read, as in
/// the rail; in Arabic the same order, the frame mirrored.
#[test]
fn in_the_wide_menu_tab_reaches_the_lock_before_settings() {
    for lang in ["en", "ar"] {
        let (_turn, mut kit) = demo_app("overview", lang);
        kit.set_size(egui::vec2(1280.0, 800.0));
        kit.run_steps(3);
        let lock = crate::i18n::spoken(tr!("Lock"));
        let settings = crate::i18n::spoken(tr!("Settings"));
        tab_to(&mut kit, &lock);
        assert_eq!(focused(&kit).as_deref(), Some(lock.as_str()), "{lang}");
        tab(&mut kit, false);
        assert_eq!(
            focused(&kit).as_deref(),
            Some(settings.as_str()),
            "{lang}: after the lock"
        );
    }
}
