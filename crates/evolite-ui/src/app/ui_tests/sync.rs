//! BUG-60, BUG-61: what the sync says while it runs — a number only for a stage the core can
//! count, and a name for the step after the masternode lists.

use super::{Kit, demo_app, has_text};
use egui_kittest::kittest::{NodeT, Queryable};
use evolite_api::SyncStage;

/// Puts the sync at `stage`, `progress` into what the app shows, draws, and checks `seen` on that
/// same frame — again (at most 50 frames) while it does not hold: a snapshot of the demo core
/// (the staged first sync sends one each time it moves) may come first and put its own back.
fn shows(
    kit: &mut Kit,
    stage: SyncStage,
    progress: Option<f32>,
    what: &str,
    seen: impl Fn(&Kit) -> bool,
) {
    for _ in 0..50 {
        let sync = &mut kit.state_mut().snapshot.as_mut().expect("a snapshot").sync;
        sync.stage = stage;
        sync.progress = progress;
        sync.core_peers = 3;
        sync.error = None;
        sync.minutes_left = None;
        kit.run_steps(1);
        let kept = kit
            .state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.sync.stage == stage && s.sync.progress == progress);
        if kept && seen(kit) {
            return;
        }
    }
    panic!("not shown: {what}");
}

/// What the sync card in the sidebar says to a screen reader (a label: its words are its value): all of it, as painted.
fn card(kit: &Kit) -> String {
    kit.query_all_by(|n| {
        n.value()
            .is_some_and(|l| l.ends_with("Core nodes") || l.ends_with("узлов Core"))
    })
    .next()
    .and_then(|n| n.accesskit_node().value())
    .unwrap_or_else(|| panic!("no sync card"))
}

/// BUG-60: a stage the core counts says how far it is; one it cannot count (the masternode
/// lists move only when they are done) says no number — not «0 %», not «100 %».
#[test]
fn the_sync_card_gives_a_percent_only_for_what_is_counted() {
    let (_turn, mut kit) = demo_app("overview", "en");
    for (stage, progress, said) in [
        (
            SyncStage::Headers,
            Some(0.47),
            "Block headers · 47 % · 3 Core nodes",
        ),
        (
            SyncStage::Masternodes,
            None,
            "Masternode lists · 3 Core nodes",
        ),
        (
            SyncStage::Connecting,
            None,
            "Looking for network nodes · 3 Core nodes",
        ),
    ] {
        shows(&mut kit, stage, progress, said, |k| card(k) == said);
    }
}

/// BUG-61: after the lists come, the card names the step — the quorums are checked — in the
/// interface language, with no «100 %» standing.
#[test]
fn after_the_masternode_lists_the_card_names_the_quorum_check() {
    let (_turn, mut kit) = demo_app("overview", "ru");
    shows(
        &mut kit,
        SyncStage::Quorums,
        None,
        "the quorum check",
        |k| card(k).starts_with("Проверка кворумов мастернод · "),
    );
    assert!(!card(&kit).contains('%'), "{}", card(&kit));
}

/// BUG-60 on the first sync (onboarding O6): the same — no number and no bar standing for a
/// stage with nothing to count, the number for one that has it. The demo core moves this sync
/// on its own, so each check is made on the frame that shows what the test put in.
#[test]
fn the_first_sync_gives_a_percent_only_for_what_is_counted() {
    let (_turn, mut kit) = demo_app("sync", "en");
    let percent_on_quorums = |k: &Kit| {
        k.query_all_by(|n| {
            n.value()
                .is_some_and(|v| v.starts_with("Checking masternode quorums ·"))
        })
        .any(|_| true)
    };
    shows(
        &mut kit,
        SyncStage::Quorums,
        None,
        "the quorum check",
        |k| has_text(k, "Checking masternode quorums"),
    );
    assert!(!percent_on_quorums(&kit), "no percent on the quorum check");
    shows(
        &mut kit,
        SyncStage::Filters,
        Some(0.25),
        "the filters at 25 %",
        |k| has_text(k, "Block filters · 25 %"),
    );
}
