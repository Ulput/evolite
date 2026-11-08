//! BUG-60, BUG-61: what the sync says while it runs — a number only for a stage the core can
//! count, and a name for the step after the masternode lists.

use super::{Kit, demo_app, has_text, tab_to};
use crate::screens::sidebar::RailMark;
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
    full_menu(&mut kit);
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
    full_menu(&mut kit);
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

/// A window wide enough for the full menu: the card with its words (the rail has the mark
/// alone, BUG-75).
fn full_menu(kit: &mut Kit) {
    kit.set_size(egui::vec2(1280.0, 800.0));
    kit.run_steps(2);
}

/// What the rail's mark showed in the last frame and whether it asked for a frame soon.
fn rail_mark(kit: &Kit) -> (RailMark, bool) {
    crate::screens::sidebar::rail_mark_shown(&kit.ctx).expect("the rail's mark")
}

/// The system's animations on or off, as the app read them (for the next 3 s).
fn system_motion(kit: &mut Kit, on: bool) {
    kit.state_mut().system_motion = Some((std::time::Instant::now(), on));
}

/// What the rail's sync status says to a screen reader.
fn rail_status(kit: &Kit) -> String {
    kit.query_all_by_role(egui::accesskit::Role::Status)
        .next()
        .and_then(|n| n.accesskit_node().value())
        .unwrap_or_else(|| panic!("no sync status in the rail"))
}

/// BUG-75 AC-D1, AC-D2, AC-D5: in the rail (1024 wide) the block moves while sync goes and asks
/// for frames; the reader hears «Sync · stage · percent · …»; once done the full logo, still,
/// with no more frames.
#[test]
fn the_rail_mark_moves_while_it_syncs_and_stands_full_when_done() {
    let (_turn, mut kit) = demo_app("overview", "en");
    system_motion(&mut kit, true);
    let moving = |k: &Kit| {
        matches!(
            rail_mark(k),
            (
                RailMark::Block {
                    dim: false,
                    stopped: false,
                    ..
                },
                true
            )
        )
    };
    shows(
        &mut kit,
        SyncStage::Filters,
        Some(0.64),
        "the block moving",
        moving,
    );
    assert_eq!(
        rail_status(&kit),
        "Sync · Block filters · 64 % · 3 Core nodes"
    );
    // A frame of the harness is 0.25 s of the app's time.
    let before = rail_mark(&kit).0;
    shows(
        &mut kit,
        SyncStage::Filters,
        Some(0.64),
        "the block moved on",
        |k| moving(k) && rail_mark(k).0 != before,
    );
    // Done: the block goes home in 400 ms, then the full logo and no frames.
    shows(&mut kit, SyncStage::Done, Some(1.0), "done", |_| true);
    assert!(
        matches!(rail_mark(&kit), (RailMark::Block { dim: false, .. }, true)),
        "on its way home first: {:?}",
        rail_mark(&kit)
    );
    let mut frames = 0;
    while rail_mark(&kit) != (RailMark::Done, false) {
        frames += 1;
        assert!(frames <= 3, "not done in 400 ms: {:?}", rail_mark(&kit));
        shows(&mut kit, SyncStage::Done, Some(1.0), "done", |_| true);
    }
    // Still from now on: no more frames asked for.
    shows(&mut kit, SyncStage::Done, Some(1.0), "done", |_| true);
    assert_eq!(rail_mark(&kit), (RailMark::Done, false));
    assert!(
        rail_status(&kit).starts_with("Synced · block "),
        "{}",
        rail_status(&kit)
    );
}

/// BUG-75 AC-D4: the system's animations off — nothing moves and no frames are asked for: «e»
/// at 55 % and the block by the stage's percent; at the end the full logo at once.
#[test]
fn with_the_systems_animations_off_the_rail_mark_stands_still() {
    let (_turn, mut kit) = demo_app("overview", "en");
    system_motion(&mut kit, false);
    let still = (
        RailMark::Block {
            dim: true,
            x: crate::screens::sidebar::block_still(Some(0.64)),
            alpha: 1.0,
            stopped: false,
        },
        false,
    );
    shows(
        &mut kit,
        SyncStage::Filters,
        Some(0.64),
        "still at 60 %",
        |k| rail_mark(k) == still,
    );
    shows(&mut kit, SyncStage::Done, Some(1.0), "done at once", |k| {
        rail_mark(k) == (RailMark::Done, false)
    });
}

/// BUG-75 AC-D3: a stop — the danger block halfway, still; the reader hears why.
#[test]
fn a_stopped_sync_shows_in_the_rail() {
    let (_turn, mut kit) = demo_app("overview", "en");
    system_motion(&mut kit, true);
    for _ in 0..50 {
        let sync = &mut kit.state_mut().snapshot.as_mut().expect("a snapshot").sync;
        sync.stage = SyncStage::Filters;
        sync.error = Some("No Core nodes answer".into());
        kit.run_steps(1);
        if matches!(
            rail_mark(&kit),
            (RailMark::Block { stopped: true, .. }, false)
        ) {
            break;
        }
    }
    assert!(matches!(
        rail_mark(&kit),
        (RailMark::Block { stopped: true, .. }, false)
    ));
    assert_eq!(
        rail_status(&kit),
        "The sync stopped · No Core nodes answer · resync in Settings → Network and nodes"
    );
}

/// BUG-75 AC-D6: the full menu is as it was — the card with its words, no mark of the rail.
#[test]
fn the_full_menu_keeps_its_sync_card() {
    let (_turn, mut kit) = demo_app("overview", "en");
    full_menu(&mut kit);
    kit.ctx
        .data_mut(|d| d.remove::<(RailMark, bool)>(egui::Id::new("evolite-rail-mark")));
    shows(&mut kit, SyncStage::Filters, Some(0.25), "the card", |k| {
        card(k) == "Block filters · 25 % · 3 Core nodes"
    });
    assert!(
        crate::screens::sidebar::rail_mark_shown(&kit.ctx).is_none(),
        "no rail mark in the full menu"
    );
    assert_eq!(
        kit.query_all_by_role(egui::accesskit::Role::Status).count(),
        0,
        "the card is a label, as before"
    );
}

/// BUG-75 AC-D5: the rail's mark is a Tab stop, and the keyboard's focus shows its words as
/// the pointer does.
#[test]
fn the_keyboard_on_the_rail_mark_shows_its_words() {
    let (_turn, mut kit) = demo_app("overview", "en");
    assert!(!has_text(&kit, "Synced"), "no words before the focus comes");
    tab_to(&mut kit, "Synced · block ");
    kit.run_steps(2);
    assert!(has_text(&kit, "Synced"), "the tooltip's first line");
}

/// What the rail's sync status is to a screen reader as a live region.
fn rail_live(kit: &Kit) -> Option<egui::accesskit::Live> {
    kit.query_all_by_role(egui::accesskit::Role::Status)
        .next()
        .map(|n| n.accesskit_node().live())
}

/// Whether the sidebar asked egui for a frame in the frames just drawn (egui keeps the file of
/// each call to `request_repaint*`).
fn the_sidebar_asks_for_frames(kit: &Kit) -> bool {
    kit.ctx
        .repaint_causes()
        .iter()
        .any(|c| c.file.ends_with("sidebar.rs"))
}

/// BUG-75 r1 should 1: the frames themselves, not only the flag — while the block moves the
/// sidebar asks egui for them; once done, home and quiet, it asks for none.
#[test]
fn once_done_the_rail_mark_asks_for_no_frames() {
    let (_turn, mut kit) = demo_app("overview", "en");
    system_motion(&mut kit, true);
    shows(
        &mut kit,
        SyncStage::Filters,
        Some(0.64),
        "the block moving",
        the_sidebar_asks_for_frames,
    );
    // Done: 400 ms home, and the status quiet after its second (a frame of the harness is
    // 0.25 s of the app's time).
    shows(&mut kit, SyncStage::Done, Some(1.0), "done", |_| true);
    let mut frames = 0;
    while rail_mark(&kit) != (RailMark::Done, false)
        || rail_live(&kit) != Some(egui::accesskit::Live::Off)
    {
        frames += 1;
        assert!(frames <= 8, "not home and quiet: {:?}", rail_mark(&kit));
        shows(&mut kit, SyncStage::Done, Some(1.0), "done", |_| true);
    }
    for _ in 0..3 {
        shows(&mut kit, SyncStage::Done, Some(1.0), "done", |_| true);
        assert!(
            !the_sidebar_asks_for_frames(&kit),
            "frames asked for once done: {:?}",
            kit.ctx.repaint_causes()
        );
    }
}

/// BUG-75 r1 should 2: the status is polite for a second after a new stage, the end and a stop,
/// and off after it — the percent moving on is not announced.
#[test]
fn the_rail_status_is_polite_for_a_second_after_a_change() {
    use egui::accesskit::Live;
    let (_turn, mut kit) = demo_app("overview", "en");
    system_motion(&mut kit, false);
    let polite = |k: &Kit| rail_live(k) == Some(Live::Polite);
    // Quiet after a second (a frame of the harness is 0.25 s), the percent moving meanwhile.
    let goes_quiet = |kit: &mut Kit, stage: SyncStage, what: &str| {
        for (i, progress) in [0.30, 0.35, 0.40, 0.45, 0.50, 0.55].into_iter().enumerate() {
            shows(kit, stage, Some(progress), what, |_| true);
            if rail_live(kit) == Some(Live::Off) {
                assert!(i >= 2, "{what}: quiet before its second: frame {i}");
                return;
            }
        }
        panic!("{what}: still polite after a second: {:?}", rail_live(kit));
    };
    shows(
        &mut kit,
        SyncStage::Filters,
        Some(0.25),
        "polite at a new stage",
        |k| polite(k) && rail_status(k).starts_with("Sync · Block filters"),
    );
    goes_quiet(&mut kit, SyncStage::Filters, "the stage");
    shows(
        &mut kit,
        SyncStage::Done,
        Some(1.0),
        "polite at the end",
        |k| polite(k) && rail_status(k).starts_with("Synced"),
    );
    goes_quiet(&mut kit, SyncStage::Done, "the end");
    // A stop: the same stage with an error.
    for _ in 0..50 {
        let sync = &mut kit.state_mut().snapshot.as_mut().expect("a snapshot").sync;
        sync.stage = SyncStage::Filters;
        sync.error = Some("No Core nodes answer".into());
        kit.run_steps(1);
        if rail_status(&kit).starts_with("The sync stopped") {
            break;
        }
    }
    assert!(polite(&kit), "polite at a stop: {:?}", rail_live(&kit));
    let mut frames = 0;
    while rail_live(&kit) != Some(Live::Off) {
        frames += 1;
        assert!(frames <= 6, "the stop: still polite after a second");
        let sync = &mut kit.state_mut().snapshot.as_mut().expect("a snapshot").sync;
        sync.stage = SyncStage::Filters;
        sync.error = Some("No Core nodes answer".into());
        kit.run_steps(1);
    }
    assert!(frames >= 2, "the stop: quiet before its second");
}

/// BUG-94: a folder the wallet's database refuses is said in full, in the interface's language,
/// and no resync is offered: it would not help.
#[test]
fn a_refused_folder_is_said_in_words() {
    let text = "Sync cannot start: other users of this computer can write to the folder /home/a. Run chmod go-w '/home/a' in a terminal, and it starts by itself within a minute.";
    let (_turn, mut kit) = demo_app("overview", "en");
    system_motion(&mut kit, true);
    for _ in 0..50 {
        let sync = &mut kit.state_mut().snapshot.as_mut().expect("a snapshot").sync;
        sync.error = Some("database ancestor /home/a is group/other-writable".into());
        sync.folder_refused = Some(text.into());
        kit.run_steps(1);
        if rail_status(&kit).starts_with("The sync stopped") {
            break;
        }
    }
    assert_eq!(rail_status(&kit), format!("The sync stopped · {text}"));
    assert!(
        kit.query_by_label(text).is_some(),
        "the band over the screen"
    );
    let ru = crate::i18n::t_in(crate::i18n::Lang::Ru, text);
    assert!(
        ru.starts_with("Синхронизация не может начаться") && ru.contains("chmod go-w '/home/a'"),
        "{ru}"
    );
}
