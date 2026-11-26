//! The whole app on the demo core, driven like a person drives it: the pointer moves over the
//! words and presses, Tab and Enter go to the window, and widgets are found by what a screen
//! reader hears (AccessKit). Nothing is drawn to pixels and no window opens (egui_kittest
//! without `snapshot`/`wgpu`). docs/architecture/kittest.md, skill `evolite-kittest`.
//!
//! Each test opens the app as `evolite --screen <preset> --lang <code>` opens it, holds the
//! one-app turn ([`demo_app`]) to its end and reads the app's own state where the screen's
//! answer is not in the tree.

mod add_collection;
mod add_name;
mod address_solid;
mod addresses;
mod advanced;
mod alignment;
mod amount_marks;
mod amounts;
mod app_connect;
mod as_known;
mod backup_file;
mod broken_channel;
mod bugs;
mod calm_states;
mod card;
mod coins;
mod collection_filter;
mod collection_grid;
mod contact_card;
mod contact_resend;
mod contacts_privacy;
mod contested_names;
mod core_known;
mod credits_after;
mod dialog_keys;
mod dialog_scroll;
mod dialogs;
mod fee_choice;
mod fiat;
mod focus;
mod fund_address;
mod history;
mod identity_from_pool;
mod identity_keys;
mod identity_picker;
mod identity_pool;
mod imgur;
mod layout;
mod loading;
mod lock_coins;
mod lock_shortcut;
mod login;
mod low_login;
mod low_menu;
mod many_recipients;
mod message;
mod missing_keys;
mod mixed;
mod mixed_to_platform;
mod mono_addresses;
mod name_suggest;
mod name_waiting;
mod network_switch;
mod new_identity;
mod no_identity;
mod notes;
mod onboarding;
mod paid_once;
mod passphrase;
mod password_reveal;
mod pause;
mod phrase_tree;
mod phrase_words;
mod plain_statuses;
mod pool_levels;
mod pool_moves;
mod pool_progress;
mod qr;
mod resend;
mod scrolling;
mod secret_input;
mod send;
mod send_back;
mod set_aside;
mod settings_apps;
mod smallest_window;
mod spend;
mod spend_flows;
mod spoken_words;
mod summary;
mod sync;
mod tab_sweep;
mod tax_category;
mod token_actions;
mod token_manage;
mod token_windows;
mod tokens;
mod top_up_twice;
mod update_check;
mod wallets;
mod workshop;
mod xpub;

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

/// The thread that took [`ONE_APP`] last (through [`take_turn`]).
static TURN_HOLDER: Mutex<Option<std::thread::ThreadId>> = Mutex::new(None);

/// This test's turn at the app. A test that still holds its turn and asks for another would
/// wait for itself forever (BUG-307's test hung three hours): that panics at once, saying so.
/// Another test's turn is waited for as long as it takes — a turn ends with its test, which
/// gives up after [`PATIENCE`] at each wait.
pub(super) fn take_turn() -> MutexGuard<'static, ()> {
    let me = std::thread::current().id();
    let mut mine_since: Option<Instant> = None;
    loop {
        match ONE_APP.try_lock() {
            Ok(turn) => return held_by(turn, me),
            Err(std::sync::TryLockError::Poisoned(e)) => return held_by(e.into_inner(), me),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        let holder = *TURN_HOLDER.lock().unwrap_or_else(|e| e.into_inner());
        // A turn just passed on may not have its holder written yet: «mine» only when it stays.
        mine_since = match (holder == Some(me), mine_since) {
            (true, None) => Some(Instant::now()),
            (true, since) => since,
            (false, _) => None,
        };
        assert!(
            !mine_since.is_some_and(|t| t.elapsed() > Duration::from_secs(1)),
            "ONE_APP is still held: drop the previous app (its _turn) before opening another"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn held_by(turn: MutexGuard<'static, ()>, me: std::thread::ThreadId) -> MutexGuard<'static, ()> {
    *TURN_HOLDER.lock().unwrap_or_else(|e| e.into_inner()) = Some(me);
    turn
}

/// The app as `evolite --screen <preset> --lang <lang>` starts it — the demo core, dark,
/// 1024×768 — once the core's first snapshot is on screen. Keep the turn to the end of the
/// test: `let (_turn, mut kit) = …` (not `_`).
pub(super) fn demo_app(preset: &str, lang: &str) -> (MutexGuard<'static, ()>, Kit) {
    let turn = take_turn();
    let args: Vec<String> = ["--screen", preset, "--lang", lang]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // This machine's last network chosen is not the test's (FEAT-0037).
    let launch = Launch::from_args_with(&args, |_| None);
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

/// How long a test waits for the demo core, which answers on its own thread. Generous: a check
/// with several builds on the machine can keep that thread off the CPU for seconds (BUG-120).
/// Waits are bounded by time, never by a count of frames: under load a frame takes longer, and
/// a frame without load passes before the core has had a turn.
pub(super) const PATIENCE: Duration = Duration::from_secs(20);

/// Frames, without any input, until `done` (at most [`PATIENCE`]): the core answers on its own
/// thread.
pub(super) fn wait_for(kit: &mut Kit, what: &str, done: impl Fn(&Kit) -> bool) {
    let started = Instant::now();
    while !done(kit) {
        assert!(started.elapsed() < PATIENCE, "no {what}");
        std::thread::sleep(Duration::from_millis(5));
        kit.step();
    }
}

/// Frames until the demo core has sent nothing new for half a second (at most [`PATIENCE`]):
/// what a screen asked for when it opened has come and is drawn. For a test that reads the
/// screen first and compares it later (BUG-120). Not for the staged first sync, which never
/// stops before it is done.
pub(super) fn settle(kit: &mut Kit) {
    const QUIET: Duration = Duration::from_millis(500);
    let started = Instant::now();
    let mut quiet_since = Instant::now();
    let mut last = kit.state().snapshot.clone();
    while quiet_since.elapsed() < QUIET {
        assert!(
            started.elapsed() < PATIENCE,
            "the demo core never went quiet"
        );
        std::thread::sleep(Duration::from_millis(5));
        kit.step();
        if kit.state().snapshot != last {
            last = kit.state().snapshot.clone();
            quiet_since = Instant::now();
        }
    }
    // Drawn from the last snapshot and laid out.
    kit.run_steps(2);
}

/// Puts `edit` into the app (most often its snapshot) and draws, until `frames` frames in a row
/// were drawn from the edited snapshot and the last of them shows `seen` (at most
/// [`PATIENCE`]); `edit` is put in again before each frame.
///
/// A snapshot from the demo core waiting when a frame starts puts the core's own values back over
/// the test's. So the answers that came meanwhile are taken first, as a frame takes them, and
/// only then `edit` is put in: a snapshot can beat it only in the instant between the two, and
/// then the frame does not count (BUG-120). Checks made after the call see a frame drawn from
/// the edit.
pub(super) fn show_edited(
    kit: &mut Kit,
    frames: usize,
    what: &str,
    edit: impl Fn(&mut App),
    seen: impl Fn(&Kit) -> bool,
) {
    let started = Instant::now();
    let mut kept = 0;
    loop {
        let ctx = kit.ctx.clone();
        kit.state_mut().take_events(&ctx);
        edit(kit.state_mut());
        let edited = kit.state().snapshot.clone();
        kit.step();
        kept = if kit.state().snapshot == edited {
            kept + 1
        } else {
            0
        };
        if kept >= frames && seen(kit) {
            return;
        }
        assert!(started.elapsed() < PATIENCE, "not shown: {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// `frames` frames, each drawn with `edit` put in after the core's answers that came — the
/// way [`show_edited`] puts it in, for a test that counts its frames or acts between them
/// (TECH-kittest-snapshot-guard).
pub(super) fn draw_edited(kit: &mut Kit, frames: usize, edit: impl Fn(&mut App)) {
    for _ in 0..frames {
        let ctx = kit.ctx.clone();
        kit.state_mut().take_events(&ctx);
        edit(kit.state_mut());
        kit.step();
    }
}

/// The tests' own edits of the snapshot that need not go through [`show_edited`],
/// [`show_wallet_edited`] or [`draw_edited`] — `(file, function, why)`. Empty: every edit does.
const DIRECT_SNAPSHOT_EDITS: &[(&str, &str, &str)] = &[];

/// TECH-kittest-snapshot-guard: a test edits the snapshot only through the helpers above. An
/// edit made once and drawn later is undone by a snapshot the demo core sends meanwhile, and
/// under a loaded check that failed tests at random (TECH-flaky-kittests, r1 and r2). A new
/// direct edit (`kit.state_mut().snapshot…`) fails here with its file and function; one that
/// must stay goes into [`DIRECT_SNAPSHOT_EDITS`] with its reason, and a listed one gone fails too.
#[test]
fn snapshot_edits_go_through_the_helpers() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/ui_tests");
    let needle: String = ["state_mut()", ".snapshot"].concat();
    let binding: String = ["=kit.", "state_mut();"].concat();
    let mut found: Vec<(String, String)> = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("the tests' folder")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .filter(|p| p.file_name().is_some_and(|n| n != "mod.rs"))
        .collect();
    files.sort();
    for path in files {
        let text = std::fs::read_to_string(&path).expect("a test file");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // Without the spaces and line breaks, with where each kept character stood.
        let (compact, at): (String, Vec<usize>) = text
            .char_indices()
            .filter(|(_, c)| !c.is_whitespace())
            .map(|(i, c)| (c, i))
            .unzip();
        let starts: Vec<usize> = compact.char_indices().map(|(i, _)| i).collect();
        let place = |k: usize| {
            let char_no = starts.binary_search(&k).expect("a char boundary");
            enclosing_fn(&text[..at[char_no]])
        };
        for (k, _) in compact.match_indices(&needle) {
            found.push((name.clone(), place(k)));
        }
        // Through a binding (review r1 M1): `let app = kit.state_mut();`, then `app.snapshot` in
        // the same function.
        for (k, _) in compact.match_indices(&binding) {
            let Some(var) = compact[..k]
                .rsplit_once("let")
                .map(|(_, v)| v.trim_start_matches("mut").to_owned())
                .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_alphanumeric() || c == '_'))
            else {
                continue;
            };
            let func = place(k);
            let uses = format!("{var}.snapshot");
            let edits = compact[k..]
                .match_indices(&uses)
                .any(|(j, _)| place(k + j) == func);
            if edits {
                found.push((name.clone(), func));
            }
        }
    }
    let listed: Vec<(String, String)> = DIRECT_SNAPSHOT_EDITS
        .iter()
        .map(|(f, n, _)| (f.to_string(), n.to_string()))
        .collect();
    let new: Vec<_> = found.iter().filter(|f| !listed.contains(f)).collect();
    assert!(
        new.is_empty(),
        "snapshot edited directly — use show_edited / show_wallet_edited / draw_edited, or list it with its reason: {new:?}"
    );
    let gone: Vec<_> = listed.iter().filter(|l| !found.contains(l)).collect();
    assert!(gone.is_empty(), "listed but no longer there: {gone:?}");
}

/// The name of the last `fn` declared in `before`: the function a position is in.
fn enclosing_fn(before: &str) -> String {
    before
        .match_indices("fn ")
        .filter(|(i, _)| *i == 0 || before[..*i].ends_with(char::is_whitespace))
        .last()
        .map(|(i, _)| {
            before[i + 3..]
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .unwrap_or_default()
}

/// [`show_edited`] for the wallet in the snapshot (none: nothing to edit).
pub(super) fn show_wallet_edited(
    kit: &mut Kit,
    frames: usize,
    what: &str,
    edit: impl Fn(&mut evolite_api::Wallet),
    seen: impl Fn(&Kit) -> bool,
) {
    show_edited(
        kit,
        frames,
        what,
        |app| {
            if let Some(w) = app.snapshot.as_mut().and_then(|s| s.wallet.as_mut()) {
                edit(w);
            }
        },
        seen,
    );
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

/// The texts painted this frame, nested shapes included: what is on screen, tree or not
/// (BUG-228: secret words are painted, not put in the tree).
pub(super) fn painted_texts(kit: &Kit) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &kit.output().shapes {
        walk(&c.shape, &mut out);
    }
    out
}

/// Whether `part` is painted on screen this frame.
pub(super) fn painted_has(kit: &Kit, part: &str) -> bool {
    painted_texts(kit).iter().any(|t| says(t, part))
}

/// Whether `text` holds `part` as written, or with its no-break spaces read as plain ones:
/// they keep a part on one line (W9's tiles), the words are the same. A test that asks for the
/// no-break space itself («Evo\u{a0}Coffee») still finds it.
fn says(text: &str, part: &str) -> bool {
    text.contains(part) || (text.contains('\u{a0}') && text.replace('\u{a0}', " ").contains(part))
}

/// Whether some node says `part` within its words.
pub(super) fn has_words(kit: &Kit, part: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| says(&l, part)) || n.value().is_some_and(|v| says(&v, part))
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

/// BUG-307: a test asking for a second turn while it holds the first is told so, not left
/// waiting for itself.
#[test]
fn a_second_turn_in_one_test_panics_saying_why() {
    let _turn = take_turn();
    let again = std::panic::catch_unwind(take_turn).expect_err("a second turn");
    let why = again
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| again.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(why.contains("drop the previous app (its _turn)"), "{why}");
}
