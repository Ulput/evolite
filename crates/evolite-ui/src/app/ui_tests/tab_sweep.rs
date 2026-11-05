//! Tab round every screen preset: the focus must never be lost (no widget has it after a Tab)
//! and never land on a widget without a name (a screen reader says nothing there).

use super::*;
use crate::cli::PRESETS;

/// What the sweep found on 2026-11-05, to be filed by QA and fixed one by one: `preset: problem`.
/// A fixed one must leave this list (the test says so), a new one must not join it.
///
/// The focus is lost where a widget takes its clicks through `ui.horizontal(..).response
/// .interact(..)`, under the id of its own `Ui` — egui takes the focus back from that id, as it
/// did from the settings rows (BUG-18 r3): `components::text_action`, `controls.rs` checkbox
/// (not proven one by one).
const KNOWN: &[&str] = &[
    "create: focus lost after «24 words»",
    "restore: focus lost after «24 words»",
    "dashpay: focus lost after «Back»",
    "sync: focus lost after «1 hour»",
    // «Edit profile» is a `text_action` (№15), which loses the focus until BUG-40.
    "identity: focus lost after «@satoshi-lab»",
    "identity-unfinished: focus lost after «Continue creating»",
    "top-up-stopped: focus lost after «Continue the top-up»",
    "top-up-awaiting: focus lost after «Check again»",
    "apps: focus lost after «Disconnect»",
    "sign: focus lost after «the start»",
    "report: focus lost after «Every time»",
    "delete: focus lost after «the start»",
    "forgot: focus lost after «the start»",
    "wallet-add: focus lost after «Import from DashPay»",
];

/// Nameless stops found on many screens at once, one cause each (filed as one bug each):
/// every text field (`components::text_field`: its label above is not its AccessKit name), and
/// a dialog's × (`components::dialog`: an image button without a name), the first stop of
/// every dialog.
const KNOWN_EVERYWHERE: &[&str] = &[
    "a nameless Tab stop (TextInput)",
    "a nameless Tab stop (PasswordInput)",
    "a nameless Tab stop (MultilineTextInput)",
    "a nameless Tab stop (Button) after «the start»",
];

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

/// What goes wrong with Tab on one screen: each kind once, at its first place.
fn sweep(preset: &str) -> Vec<String> {
    let (_turn, mut kit) = demo_app(preset, "en");
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
        tab(&mut kit, false);
        let id = kit.ctx.memory(|m| m.focused());
        // The focused node's role, when the tree has one.
        let role = kit
            .query_by(|n| n.is_focused())
            .map(|n| format!("{:?}", n.accesskit_node().role()));
        match (focused(&kit), role) {
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
    found
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
