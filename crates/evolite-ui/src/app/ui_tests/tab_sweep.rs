//! Tab round every screen preset: the focus must never be lost (no widget has it after a Tab)
//! and never land on a widget without a name (a screen reader says nothing there).

use super::*;
use crate::cli::PRESETS;

/// Known problems not fixed yet, `preset: problem`. A fixed one must leave this list (the test
/// says so), a new one must not join it. Empty since BUG-40 and BUG-41: the 15 screens that lost
/// the focus (rows that took their clicks through `ui.horizontal(..).response.interact(..)`,
/// now `a11y::row_click`) and the nameless fields and dialog crosses.
const KNOWN: &[&str] = &[];

/// Problems found on many screens at once, one cause each (one bug each): «a nameless Tab
/// stop (TextInput)». Empty since BUG-41.
const KNOWN_EVERYWHERE: &[&str] = &[];

/// A problem without its place: «sync: focus lost» (the place moves on screens that change).
fn kind(problem: &str) -> &str {
    problem.split(" after ").next().unwrap_or(problem)
}

fn known(problem: &str) -> bool {
    KNOWN.iter().any(|k| kind(k) == kind(problem))
        || KNOWN_EVERYWHERE.iter().any(|k| problem.contains(k))
}

/// At most this many presses of Tab per screen: enough to come round on every preset.
const PRESSES: usize = 70;

/// Where the app is: the screen, the onboarding step, the wallet's state and the dialog on top.
/// The demo core moves some screens on by itself (the sync ends and the wallet opens), and a
/// focus lost there is not Tab's.
fn place(kit: &Kit) -> String {
    let app = kit.state();
    format!(
        "{:?} {:?} {:?} {:?}",
        app.screen,
        app.onboarding.as_ref().map(|o| o.step),
        app.vault,
        kit.ctx.memory(|m| m.top_modal_layer()),
    )
}

/// What goes wrong with Tab on one screen: each kind once, at its first place. The round stops
/// where the screen goes on by itself (see [`place`]).
fn sweep(preset: &str) -> Vec<String> {
    let (_turn, mut kit) = demo_app(preset, "en");
    // An app's request is the demo core's answer to the preset's link, a moment after the
    // start: the round waits for its dialog (it once came in the middle of «exchange»).
    // The sync screen comes after the wallet is made: the demo core opens it a moment later.
    let staged = matches!(preset, "sync" | "sync-nodes");
    if staged {
        wait_for(&mut kit, "the wallet made", |kit| {
            kit.state().vault == Some(evolite_api::VaultState::Unlocked)
        });
    }
    if preset == "connect" {
        wait_for(&mut kit, "the app's request", |kit| {
            kit.ctx.memory(|m| m.top_modal_layer()).is_some()
        });
    }
    // «Check the phrase» opens asking the core for the words: the password field goes when
    // they come, and a round standing on it then loses the focus (review r2, should 3).
    if preset == "check-phrase" {
        wait_for(&mut kit, "the words asked", |kit| {
            kit.state()
                .settings
                .check
                .as_ref()
                .is_some_and(|c| c.questions.is_some())
        });
    }
    // A late answer of the demo core (what the screen asked for when it opened) redraws the
    // screen under the round and may take the focus with it: seen under a loaded check (BUG-120,
    // BUG-48 r2). Then the screen is let settle and its round starts again, once. The staged
    // sync answers all the time; [`place`] stops its round where it moves on.
    let snapshot = kit.state().snapshot.clone();
    let answered = |k: &Kit| !staged && k.state().snapshot != snapshot;
    round(&mut kit, preset, answered).unwrap_or_else(|| {
        settle(&mut kit);
        round(&mut kit, preset, |_| false).expect("a round")
    })
}

/// One Tab round of `preset`; `None` when `moved` said, after a press, that the core answered.
fn round(kit: &mut Kit, preset: &str, moved: impl Fn(&Kit) -> bool) -> Option<Vec<String>> {
    let start = place(kit);
    let mut found: Vec<String> = Vec::new();
    let mut note = |problem: String| {
        let kind = problem
            .split(" after ")
            .next()
            .unwrap_or_default()
            .to_owned();
        if !found.iter().any(|f| f.starts_with(&kind)) {
            found.push(problem);
        }
    };
    let mut first: Option<egui::Id> = None;
    let mut last = String::from("the start");
    for press in 0..PRESSES {
        tab(kit, false);
        if moved(kit) {
            return None;
        }
        if place(kit) != start {
            println!(
                "  ({preset}: went on by itself after «{last}»: {start} → {})",
                place(kit)
            );
            break;
        }
        let id = kit.ctx.memory(|m| m.focused());
        // The focused node's role, when the tree has one.
        let role = kit
            .query_by(|n| n.is_focused())
            .map(|n| format!("{:?}", n.accesskit_node().role()));
        match (focused(kit), role) {
            (None, _) => note(format!("{preset}: focus lost after «{last}»")),
            (Some(_), None) => note(format!(
                "{preset}: focus on a widget missing from the tree after «{last}»"
            )),
            (Some(name), Some(role)) if name.is_empty() => note(format!(
                "{preset}: a nameless Tab stop ({role}) after «{last}»"
            )),
            (Some(name), _) => last = name,
        }
        // Round once: back at the first widget Tab reached.
        match (first, id) {
            (None, Some(id)) => first = Some(id),
            (Some(start), Some(id)) if id == start && press > 0 => break,
            _ => {}
        }
    }
    Some(found)
}

#[test]
fn tab_never_loses_the_focus_on_any_screen() {
    let found: Vec<String> = PRESETS.iter().flat_map(|p| sweep(p)).collect();
    println!("Tab sweep, {} presets:", PRESETS.len());
    for f in &found {
        println!("  {f}");
    }
    let new: Vec<&String> = found.iter().filter(|f| !known(f)).collect();
    let fixed: Vec<&&str> = KNOWN
        .iter()
        .filter(|k| !found.iter().any(|f| kind(f) == kind(k)))
        .chain(
            KNOWN_EVERYWHERE
                .iter()
                .filter(|k| !found.iter().any(|f| f.contains(*k))),
        )
        .collect();
    assert!(
        new.is_empty() && fixed.is_empty(),
        "new Tab problems: {new:#?}\nfixed, take them off KNOWN: {fixed:#?}"
    );
}
