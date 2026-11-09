//! Where a screen scrolls: the wheel beside the content (R2a).

use super::*;
use crate::screens::Screen;
use evolite_api::{Command, Password, VaultState};

/// Wider than the content's 1200 with the menu and the margins (1528), low enough to scroll.
const WIDE: egui::Vec2 = egui::vec2(1800.0, 460.0);
const MENU: f32 = 248.0;
/// Inside the left margin, between the menu and the 1200-wide content.
const LEFT_MARGIN: f32 = 350.0;
/// The outermost points of the right margin.
const RIGHT_EDGE: f32 = 1790.0;

/// Where everything right of the menu stands, top to bottom.
fn screen_places(kit: &Kit) -> Vec<f32> {
    kit.query_all(By::new().role(Role::Label))
        .filter(|n| n.rect().left() >= MENU)
        .map(|n| n.rect().top())
        .collect()
}

/// One notch of `by` points with the pointer at `x`, half way down the window.
fn wheel_at(kit: &mut Kit, x: f32, by: f32) {
    kit.hover_at(egui::pos2(x, 300.0));
    kit.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, by),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    kit.run_steps(10);
}

/// R2a: in a window wider than 1528 the margins are as much the screen as its column: the wheel
/// over them scrolls it, on every screen, also those that keep their own scroll area.
#[test]
fn the_wheel_over_the_side_margins_scrolls_the_screen() {
    for preset in ["overview", "send", "activity", "identity", "shielded"] {
        let (_turn, mut kit) = demo_app(preset, "en");
        kit.set_size(WIDE);
        kit.run_steps(4);
        let top = screen_places(&kit);
        for x in [LEFT_MARGIN, RIGHT_EDGE] {
            wheel_at(&mut kit, x, -120.0);
            assert_ne!(
                screen_places(&kit),
                top,
                "{preset}: the wheel at x = {x} did nothing"
            );
            wheel_at(&mut kit, x, 120.0);
            assert_eq!(screen_places(&kit), top, "{preset}: back at the top");
        }
    }
}

/// R2a: the margins are the screen's only outside a dialog and outside the menu.
#[test]
fn the_wheel_elsewhere_leaves_the_screen_where_it_is() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(WIDE);
    kit.run_steps(4);
    let top = screen_places(&kit);
    wheel_at(&mut kit, 100.0, -120.0);
    assert_eq!(screen_places(&kit), top, "over the menu");
    drop(kit);
    drop(_turn);

    let (_turn, mut kit) = demo_app("shield", "en");
    kit.set_size(WIDE);
    kit.run_steps(4);
    let top = screen_places(&kit);
    wheel_at(&mut kit, LEFT_MARGIN, -120.0);
    assert_eq!(screen_places(&kit), top, "under the dialog's backdrop");
}

/// A window low enough for every screen to scroll.
const LOW: egui::Vec2 = egui::vec2(1024.0, 460.0);

/// Overview scrolled down: where the words now stand are not where they stood.
fn scroll_overview_down(kit: &mut Kit) {
    kit.set_size(LOW);
    kit.run_steps(4);
    let top = screen_places(kit);
    wheel_at(kit, 500.0, -250.0);
    assert_ne!(screen_places(kit), top, "the Overview did not scroll");
}

/// A screen opened in a new app that starts from `memory`, as eframe starts from the file of the
/// last run (`persistence`); the places of its words at the top of it.
fn places_in_a_new_app(preset: &str, memory: Option<egui::Memory>) -> Vec<f32> {
    let _turn = ONE_APP.lock().unwrap_or_else(|e| e.into_inner());
    let args: Vec<String> = ["--screen", preset, "--lang", "en"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let launch = Launch::from_args(&args);
    let mut kit = Harness::builder().with_size(LOW).build_eframe(|cc| {
        if let Some(memory) = memory {
            cc.egui_ctx.memory_mut(|m| *m = memory);
        }
        App::with_art(cc, launch, evolite_core::ArtOptions::off())
    });
    wait_for(&mut kit, "first snapshot", |kit| {
        kit.state().snapshot.is_some()
    });
    kit.run_steps(6);
    screen_places(&kit)
}

/// BUG-102: eframe keeps egui's memory, scroll offsets too, in a file between runs, and the
/// numbers of the screen visits start over: the first visit of this run met the offset of the
/// first visit of the last one, whatever screen that was.
#[test]
fn a_new_run_opens_every_screen_at_the_top() {
    let memory = {
        let (_turn, mut kit) = demo_app("overview", "en");
        scroll_overview_down(&mut kit);
        kit.ctx.memory(|m| m.clone())
    };
    for preset in ["overview", "identity"] {
        assert_eq!(
            places_in_a_new_app(preset, Some(memory.clone())),
            places_in_a_new_app(preset, None),
            "{preset}: opened where the last run left a screen"
        );
    }
}

/// BUG-102: locked with the Overview scrolled, it opens at the top again: the lock is the end
/// of the visit, though the screen after it is the same one. The demo core does not lock, so
/// the app's snapshot is locked here; a snapshot from the core may come first.
#[test]
fn the_overview_after_a_lock_opens_at_the_top() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(LOW);
    kit.run_steps(4);
    let top = screen_places(&kit);
    scroll_overview_down(&mut kit);
    let wallet = kit.state().snapshot.as_ref().and_then(|s| s.wallet.clone());
    for _ in 0..10 {
        if let Some(s) = kit.state_mut().snapshot.as_mut() {
            s.vault = VaultState::Locked;
            s.wallet = None;
        }
        kit.run_steps(1);
        if kit.state().screen == Screen::Unlock {
            break;
        }
    }
    assert_eq!(kit.state().screen, Screen::Unlock);
    for _ in 0..10 {
        if let Some(s) = kit.state_mut().snapshot.as_mut() {
            s.vault = VaultState::Unlocked;
            s.wallet = wallet.clone();
        }
        kit.run_steps(1);
        if kit.state().screen == Screen::Overview {
            break;
        }
    }
    assert_eq!(kit.state().screen, Screen::Overview);
    kit.run_steps(4);
    assert_eq!(screen_places(&kit), top);
}

/// BUG-102: another wallet opened over a scrolled Overview opens at the top.
#[test]
fn another_wallet_opens_at_the_top() {
    let (_turn, mut kit) = demo_app("overview", "en");
    wait_for(&mut kit, "the wallets", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.wallets.iter().any(|w| w.open))
    });
    kit.set_size(LOW);
    kit.run_steps(4);
    let top = screen_places(&kit);
    scroll_overview_down(&mut kit);
    kit.state_mut().core.send(Command::SwitchWallet {
        id: "5e6f7a8b".into(),
        password: Some(Password::new("savings-password".into())),
    });
    wait_for(&mut kit, "the other wallet", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.wallets.iter().any(|w| w.open && w.id == "5e6f7a8b"))
    });
    kit.run_steps(6);
    assert_eq!(screen_places(&kit), top);
}
