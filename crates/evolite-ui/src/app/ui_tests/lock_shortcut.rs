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
        (egui::vec2(1024.0, 900.0), "en"),
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

/// BUG-331 (the designer): in a rail with room for it (1024 × 900) the lock stands above
/// Settings, 36 × 36 each, 8 apart, in the rail's middle; Tab reaches the lock, then Settings.
#[test]
fn in_a_tall_rail_the_lock_stands_above_settings() {
    for lang in ["en", "ar"] {
        let (_turn, mut kit) = demo_app("overview", lang);
        kit.set_size(egui::vec2(1024.0, 900.0));
        kit.run_steps(4);
        let lock_name = crate::i18n::spoken(tr!("Lock"));
        let settings_name = crate::i18n::spoken(tr!("Settings"));
        let rect = |kit: &Kit, name: &str| kit.get_by_role_and_label(Role::Button, name).rect();
        let (lock, settings) = (rect(&kit, &lock_name), rect(&kit, &settings_name));
        assert_eq!(lock.size(), egui::vec2(36.0, 36.0), "{lang}: the lock");
        assert_eq!(settings.size(), egui::vec2(36.0, 36.0), "{lang}: Settings");
        assert_eq!(
            lock.center().x,
            settings.center().x,
            "{lang}: one above the other"
        );
        assert_eq!(
            settings.top() - lock.bottom(),
            8.0,
            "{lang}: 8 apart, the lock above"
        );
        tab_to(&mut kit, &lock_name);
        tab(&mut kit, false);
        assert_eq!(
            focused(&kit).as_deref(),
            Some(settings_name.as_str()),
            "{lang}: Tab from the lock to Settings"
        );
    }
}

/// BUG-331: in 800 × 600 the rail has no room for the lock — none there, the ten items with no
/// fade (FEAT-0036 AC-T2) — and the wallet menu's «Lock · Ctrl+L» locks; Ctrl+L too.
#[test]
fn in_a_low_rail_the_lock_is_in_the_wallet_menu() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(egui::vec2(800.0, 600.0));
    kit.run_steps(4);
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Lock")
            .next()
            .is_none(),
        "no lock in the rail"
    );
    assert_eq!(
        crate::screens::sidebar::menu_more_below(&kit.ctx),
        Some(false),
        "the ten items with no fade"
    );
    kit.state_mut().wallet_menu = true;
    kit.run_steps(3);
    kit.get_by_role_and_label(Role::Button, "Lock").click();
    wait_for(&mut kit, "the unlock screen", locked);
}

/// BUG-331: in 800 × 600, with no lock in the rail, Ctrl+L locks.
#[test]
fn in_a_low_rail_ctrl_l_locks() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(egui::vec2(800.0, 600.0));
    kit.run_steps(3);
    press_lock(&mut kit);
    wait_for(&mut kit, "the unlock screen", locked);
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

/// BUG-331 r1 (Should): the rail's lock is there on the frame drawn first — no measure kept yet,
/// as at the start or after the unlock — not one frame later with the sync card jumping 44.
/// Mutant: without the frame drawn again, the first frame has no lock (red).
#[test]
fn the_rails_lock_is_there_on_the_first_frame() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(egui::vec2(1024.0, 900.0));
    kit.run_steps(3);
    let lock = crate::i18n::spoken(tr!("Lock"));
    let at = kit.get_by_role_and_label(Role::Button, &lock).rect();
    kit.ctx
        .data_mut(|d| d.remove_temp::<f32>(crate::screens::sidebar::nav_height_id()));
    kit.step();
    let first = kit
        .query_all_by_role_and_label(Role::Button, &lock)
        .next()
        .map(|b| b.rect());
    assert_eq!(first, Some(at), "the lock in its place on the first frame");
}
