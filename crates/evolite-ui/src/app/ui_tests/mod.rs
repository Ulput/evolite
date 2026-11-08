//! The whole app on the demo core, driven like a person drives it: the pointer moves over the
//! words and presses, Tab and Enter go to the window, and widgets are found by what a screen
//! reader hears (AccessKit). Nothing is drawn to pixels and no window opens (egui_kittest
//! without `snapshot`/`wgpu`). docs/architecture/kittest.md, skill `evolite-kittest`.
//!
//! Each test opens the app as `evolite --screen <preset> --lang <code>` opens it, holds the
//! one-app turn ([`demo_app`]) to its end and reads the app's own state where the screen's
//! answer is not in the tree.

mod add_name;
mod amounts;
mod as_known;
mod bugs;
mod calm_states;
mod card;
mod contacts_privacy;
mod core_known;
mod dialog_keys;
mod dialogs;
mod focus;
mod history;
mod identity_pool;
mod layout;
mod loading;
mod login;
mod new_identity;
mod notes;
mod onboarding;
mod pause;
mod send;
mod settings_apps;
mod spend;
mod sync;
mod tab_sweep;
mod wallets;

use super::App;
use crate::cli::Launch;
use egui::Key;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub(super) type Kit = Harness<'static, App>;

/// App tests take turns: the interface language and the capture request are per thread now,
/// but the network and the core and art threads are one per process.
static ONE_APP: Mutex<()> = Mutex::new(());

/// The app as `evolite --screen <preset> --lang <lang>` starts it — the demo core, dark,
/// 1024×768 — once the core's first snapshot is on screen. Keep the turn to the end of the
/// test: `let (_turn, mut kit) = …` (not `_`).
pub(super) fn demo_app(preset: &str, lang: &str) -> (MutexGuard<'static, ()>, Kit) {
    let turn = ONE_APP.lock().unwrap_or_else(|e| e.into_inner());
    let args: Vec<String> = ["--screen", preset, "--lang", lang]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let launch = Launch::from_args(&args);
    assert!(
        matches!(launch.core, crate::cli::CoreChoice::Mock),
        "a preset opens the demo"
    );
    let mut kit = Harness::builder()
        .with_size(egui::vec2(1024.0, 768.0))
        .build_eframe(|cc| App::with_art(cc, launch, evolite_core::ArtOptions::off()));
    // The core answers on its own thread.
    wait_for(&mut kit, "first snapshot from the demo core", |kit| {
        kit.state().snapshot.is_some()
    });
    // The app's fonts take effect a frame after they are installed.
    kit.run_steps(3);
    (turn, kit)
}

/// Frames, without any input, until `done` (at most 10 s): the core answers on its own thread.
pub(super) fn wait_for(kit: &mut Kit, what: &str, done: impl Fn(&Kit) -> bool) {
    let started = Instant::now();
    while !done(kit) {
        assert!(started.elapsed() < Duration::from_secs(10), "no {what}");
        std::thread::sleep(Duration::from_millis(5));
        kit.step();
    }
}

/// The words of a label as drawn: a label node whose text is exactly `words`.
pub(super) fn words<'k>(kit: &'k Kit, words: &'k str) -> egui_kittest::Node<'k> {
    kit.get(By::new().role(Role::Label).value(words))
}

/// The button a screen reader calls `name` (`components::named`).
pub(super) fn button<'k>(kit: &'k Kit, name: &'k str) -> egui_kittest::Node<'k> {
    kit.get_by_role_and_label(Role::Button, name)
}

/// Whether some node says `text`: as its name or as a label's words.
pub(super) fn has_text(kit: &Kit, text: &str) -> bool {
    kit.query_all_by(|n| n.label().as_deref() == Some(text) || n.value().as_deref() == Some(text))
        .next()
        .is_some()
}

/// Whether some node says `part` within its words.
pub(super) fn has_words(kit: &Kit, part: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.contains(part)) || n.value().is_some_and(|v| v.contains(part))
    })
    .next()
    .is_some()
}

/// A click on the words of the widget named `name`, not on its centre or its icon: the label
/// inside it (BUG-17, BUG-18). A row clicked through `a11y::row_click` (BUG-40) has its words
/// beside it in the tree, not inside: then they are the label that says the same.
pub(super) fn click_words_of(kit: &mut Kit, name: &str) {
    let is_label = |n: &egui_kittest::Node<'_>| n.accesskit_node().role() == Role::Label;
    let label = kit
        .query_all_by_label(name)
        .find_map(|node| {
            if is_label(&node) {
                Some(node)
            } else {
                node.children_recursive().find(is_label)
            }
        })
        .unwrap_or_else(|| panic!("«{name}» has no words of its own"));
    label.click();
    kit.run_steps(2);
}

/// The space between two widgets' rectangles, across or down, whichever they are apart in.
pub(super) fn gap(a: egui::Rect, b: egui::Rect) -> f32 {
    [
        b.left() - a.right(),
        a.left() - b.right(),
        b.top() - a.bottom(),
        a.top() - b.bottom(),
    ]
    .into_iter()
    .fold(f32::MIN, f32::max)
}

/// The keyboard focus: `None` when no widget has it, else the AccessKit name of the one that
/// has it (empty for a nameless widget).
pub(super) fn focused(kit: &Kit) -> Option<String> {
    kit.ctx.memory(|m| m.focused())?;
    Some(
        kit.query_by(|n| n.is_focused())
            .and_then(|n| n.accesskit_node().label())
            .unwrap_or_default(),
    )
}

/// One press of Tab (Shift+Tab with `back`) and the frames it takes.
pub(super) fn tab(kit: &mut Kit, back: bool) {
    if back {
        kit.key_press_modifiers(egui::Modifiers::SHIFT, Key::Tab);
    } else {
        kit.key_press(Key::Tab);
    }
    kit.run_steps(2);
}

/// Presses Tab until the focus is on a widget whose name starts with `name` (at most 80 times).
pub(super) fn tab_to(kit: &mut Kit, name: &str) {
    for _ in 0..80 {
        if focused(kit).is_some_and(|f| f.starts_with(name)) {
            return;
        }
        tab(kit, false);
    }
    panic!("Tab never reached «{name}»");
}

/// Enter, and the frames it takes.
pub(super) fn enter(kit: &mut Kit) {
    kit.key_press(Key::Enter);
    kit.run_steps(2);
}
