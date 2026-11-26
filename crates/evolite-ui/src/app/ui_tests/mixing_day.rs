//! FEAT-0052 on the demo core: yesterday's day of mixing (`activity_mixing::example_day`, put in
//! by the `activity-mixing*` and `overview-mixing` presets) as one line in History and on the
//! Overview, its card with the list, and an operation's card opened from it with «Back».

use super::*;
use crate::screens::activity_mixing::EXAMPLE_IDS;
use egui::accesskit::Role;

/// The buttons whose names start with `start`.
fn buttons_starting(kit: &Kit, start: &str) -> Vec<String> {
    kit.query_all_by(|n| {
        n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with(start))
    })
    .filter_map(|n| n.accesskit_node().label())
    .collect()
}

/// The example day's line: its name (R-9).
fn day_line(kit: &Kit) -> Option<String> {
    buttons_starting(kit, "Mixing, 6 operations, ")
        .into_iter()
        .next()
}

/// History on the preset, once the day is in it.
fn history(preset: &str) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app(preset, "en");
    wait_for(&mut kit, "the day of mixing", |kit| day_line(kit).is_some());
    settle(&mut kit);
    (turn, kit)
}

/// A1, R-9, A7: the day's six operations are one line, said as one sentence, «Mixing coins» in
/// the plain mode; none of its rounds is a line of its own; the payment from the mixed coins is.
#[test]
fn a_day_of_mixing_is_one_line_said_as_one_sentence() {
    let (_turn, kit) = history("activity-mixing");
    let name = day_line(&kit).expect("the line");
    let fees = format!(", fees {}, ", crate::format::signed_amount(-10_374));
    assert!(name.contains(&fees), "{name}");
    assert!(
        name.ends_with(", Completed, cryptographically checked"),
        "{name}"
    );
    assert_eq!(buttons_starting(&kit, "Mixing, 6 operations").len(), 1);
    assert!(painted_has(&kit, "Mixing coins"));
    assert!(
        !painted_has(&kit, "Mixing (CoinJoin)"),
        "no CoinJoin in plain"
    );
    assert!(
        !painted_has(&kit, "A mixing round: the balance did not change"),
        "no round a line of its own"
    );
    let paid = crate::text::signed_amount(-10_002_200);
    assert!(
        buttons_starting(&kit, "Sent")
            .iter()
            .any(|l| l.contains(&paid)),
        "the payment from mixed coins its own line"
    );
}

/// A7: the advanced mode names it «Mixing (CoinJoin)».
#[test]
fn the_advanced_mode_says_coinjoin() {
    let (_turn, kit) = history("activity-mixing-advanced");
    assert!(painted_has(&kit, "Mixing (CoinJoin)"));
    assert!(!painted_has(&kit, "Mixing coins"));
}

/// A4: one operation not confirmed — the day's line is pending, with no seal.
#[test]
fn a_day_with_one_pending_is_pending() {
    let (_turn, mut kit) = history("activity-mixing");
    let pending = EXAMPLE_IDS + 7;
    show_wallet_edited(
        &mut kit,
        3,
        "a round pending",
        |w| {
            if let Some(a) = w.activity.iter_mut().find(|a| a.id == pending) {
                a.proof = evolite_api::Proof::Mempool;
            }
        },
        |kit| day_line(kit).is_some_and(|n| n.contains(", Pending")),
    );
    let name = day_line(&kit).expect("the line");
    assert!(!name.contains("cryptographically checked"), "{name}");
}

/// A2, UC-3, A8: the line opens the day's card — what mixing does, the summary and its six
/// operations, «Round» and «Preparing coins» in the plain mode, reached by Tab; one of them opens
/// its own card, whose «Back» comes back to the day's.
#[test]
fn the_day_opens_its_list_and_each_its_card_with_back() {
    let (_turn, mut kit) = history("activity-mixing");
    let name = day_line(&kit).expect("the line");
    button(&kit, &name).click();
    wait_for(&mut kit, "the day's card", |kit| {
        has_words(kit, "Mixing does not change the balance")
    });
    settle(&mut kit);
    for words in ["Mixing rounds", "Coins prepared", "Into mixing", "Fees"] {
        assert!(painted_has(&kit, words), "«{words}»");
    }
    assert!(
        painted_has(&kit, &crate::format::amount(5 * 10_000_100)),
        "what went into mixing"
    );
    assert_eq!(buttons_starting(&kit, "Round").len(), 5, "the rounds");
    assert_eq!(buttons_starting(&kit, "Preparing coins").len(), 1);
    tab_to(&mut kit, "Preparing coins");
    let prep = focused(&kit).expect("focused");
    button(&kit, &prep).click();
    wait_for(&mut kit, "the preparation's card", |kit| {
        kit.query_by_label("Back").is_some()
    });
    settle(&mut kit);
    assert!(
        kit.query_by_label("Mixing rounds").is_none() && !painted_has(&kit, "Mixing rounds"),
        "its card in place of the day's"
    );
    button(&kit, "Back").click();
    wait_for(&mut kit, "the day's card again", |kit| {
        painted_has(kit, "Mixing rounds")
    });
    // «Close» under it (its cross is «Close» too: either closes it).
    kit.query_all_by_role_and_label(Role::Button, "Close")
        .next()
        .expect("«Close»")
        .click();
    kit.run_steps(3);
    assert!(!painted_has(&kit, "Mixing rounds"), "closed");
}

/// The painted texts that start with `start`, and whether egui cut each with «…».
fn painted_cut(kit: &Kit, start: &str) -> Vec<bool> {
    fn walk(shape: &egui::Shape, start: &str, out: &mut Vec<bool>) {
        match shape {
            egui::Shape::Text(t) if t.galley.text().starts_with(start) => out.push(t.galley.elided),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, start, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &kit.output().shapes {
        walk(&c.shape, start, &mut out);
    }
    out
}

/// The designer (2026-11-26, §8d-2): in the day's card the kind of an operation is whole —
/// «Preparing coins», «Подготовка монет» — its status under its fee, not in a column; a round
/// that took nothing says «no fee».
#[test]
fn the_days_list_says_each_kind_whole_and_no_fee() {
    for (lang, prep, none) in [
        ("en", "Preparing coins", "no fee"),
        ("ru", "Подготовка монет", "без комиссии"),
    ] {
        let (turn, mut kit) = demo_app("activity-mixing-day", lang);
        wait_for(&mut kit, "the day's card", |kit| {
            !painted_cut(kit, prep).is_empty()
        });
        settle(&mut kit);
        assert_eq!(painted_cut(&kit, prep), vec![false], "{lang}: whole");
        assert!(painted_has(&kit, none), "{lang}: «{none}»");
        drop(kit);
        drop(turn);
    }
}

/// UC-3: the preset opens the collateral's card from the day; its cross closes both.
#[test]
fn the_cross_of_a_card_from_the_day_closes_both() {
    let (_turn, mut kit) = demo_app("activity-mixing-card", "en");
    wait_for(&mut kit, "the card with «Back»", |kit| {
        kit.query_by_label("Back").is_some()
    });
    settle(&mut kit);
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    assert!(kit.query_by_label("Back").is_none());
    assert!(
        !painted_has(&kit, "Mixing rounds"),
        "the day's card closed too"
    );
}

/// A5, R-6: a search by the txid of one of the day's operations shows its line alone.
#[test]
fn a_search_shows_the_operation_found_on_its_own() {
    let (_turn, mut kit) = history("activity-mixing");
    kit.state_mut().history.query = format!("{:016x}", EXAMPLE_IDS + 1);
    wait_for(&mut kit, "the preparation alone", |kit| {
        day_line(kit).is_none()
            && has_words(kit, "Between your own: coins split into denominations")
    });
    assert!(
        buttons_starting(&kit, "Mixing, ").is_empty(),
        "no day's line"
    );
}

/// R-5: «Mixing» under «Type» shows the days; «Transfer to yourself» none of them.
#[test]
fn the_mixing_filter_shows_the_days_and_only_it() {
    use crate::screens::activity_filters::kind;
    let (_turn, mut kit) = history("activity-mixing");
    kit.state_mut().history.filters.kinds = kind::MIXING;
    wait_for(&mut kit, "only the days", |kit| {
        buttons_starting(kit, "Sent").is_empty()
    });
    assert!(day_line(&kit).is_some());
    kit.state_mut().history.filters.kinds = kind::SELF_TRANSFER;
    wait_for(&mut kit, "no day", |kit| {
        buttons_starting(kit, "Mixing, ").is_empty()
    });
}

/// A9: the Overview's latest operations have the day as one line too.
#[test]
fn the_overview_has_the_day_as_one_line() {
    let (_turn, mut kit) = demo_app("overview-mixing", "en");
    // The demo's newer lines left out, so the day is among the latest five.
    show_wallet_edited(
        &mut kit,
        3,
        "only the example day",
        |w| w.activity.retain(|a| a.id > EXAMPLE_IDS),
        |kit| day_line(kit).is_some(),
    );
    assert_eq!(buttons_starting(&kit, "Mixing, ").len(), 1);
    assert!(
        !buttons_starting(&kit, "Sent").is_empty(),
        "the payment its own"
    );
    // R-11 r2 (AC-A10): no day headings here — the day as the rows beside it say it.
    wait_for(&mut kit, "yesterday's day", |kit| {
        painted_has(kit, "6 operations · yesterday, 10:00–12:05")
    });
    // The same day moved to today.
    let yesterday: std::collections::HashMap<u64, evolite_api::Timestamp> =
        crate::screens::activity_mixing::example_day(chrono::Local::now())
            .into_iter()
            .map(|a| (a.id, a.at))
            .collect();
    show_wallet_edited(
        &mut kit,
        3,
        "the day today",
        |w| {
            w.activity.retain(|a| a.id > EXAMPLE_IDS);
            for a in &mut w.activity {
                if let Some(at) = yesterday.get(&a.id) {
                    a.at = at + 86_400;
                }
            }
        },
        |kit| painted_has(kit, "6 operations · today, 10:00–12:05"),
    );
}

/// R-11, AC-A10: the day's line says its times — the preparation at 10:00 to the last round at
/// 12:05 — not the date of the heading over it, whole, in ru, en and ar at 1024 and 800.
#[test]
fn the_days_line_says_its_times_at_both_sizes() {
    fn painted(kit: &Kit) -> Vec<(String, bool)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, bool)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.galley.elided)),
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
    for (lang, words) in [
        ("en", Some("6 operations · 10:00–12:05")),
        ("ru", Some("6 операций · 10:00–12:05")),
        ("ar", None),
    ] {
        for size in [egui::vec2(1024.0, 768.0), egui::vec2(800.0, 600.0)] {
            let (turn, mut kit) = demo_app("activity-mixing", lang);
            kit.set_size(size);
            let line = |kit: &Kit| {
                painted(kit)
                    .into_iter()
                    .find(|(t, _)| t.contains("10:00") && t.contains("12:05"))
            };
            wait_for(&mut kit, "the day's line", |kit| line(kit).is_some());
            settle(&mut kit);
            let (text, cut) = line(&kit).expect("the line");
            assert!(!cut, "{lang} {size:?}: whole — {text}");
            if let Some(words) = words {
                assert!(text.contains(words), "{lang} {size:?}: {text}");
            }
            // Under its day's heading: no day of its own (R-11).
            for day in ["yesterday", "вчера", "today", "сегодня"] {
                assert!(!text.contains(day), "{lang} {size:?}: {text}");
            }
            drop(kit);
            drop(turn);
        }
    }
}
