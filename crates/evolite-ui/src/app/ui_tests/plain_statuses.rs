//! FEAT-0051 on the demo core: plain statuses (R-1), the seal and its words for a screen reader
//! (R-2, R-3), ids whole where they fit and shortened in their middle where they do not (R-5),
//! the Overview's line in plain words (R-7).

use super::*;

/// The demo's paid withdrawal: its title names this address.
const TO: &str = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f2c";

fn names(kit: &Kit) -> Vec<String> {
    kit.query_all_by(|n| n.label().is_some())
        .filter_map(|n| n.accesskit_node().label())
        .collect()
}

/// AC-A1: plain — «Completed» on the History's lines that went through; advanced — the Dash
/// terms, InstantLock and ChainLock, in Latin letters.
#[test]
fn history_says_completed_in_plain_words_and_the_terms_for_advanced_users() {
    let (_turn, mut kit) = demo_app("activity", "en");
    wait_for(&mut kit, "the lines", |kit| painted_has(kit, "Completed"));
    assert!(
        !painted_texts(&kit)
            .iter()
            .any(|t| t == "ChainLock" || t == "InstantLock")
    );
    kit.state_mut().settings.set("advanced", true);
    wait_for(&mut kit, "the terms", |kit| {
        painted_texts(kit).iter().any(|t| t == "ChainLock")
            && painted_texts(kit).iter().any(|t| t == "InstantLock")
    });
}

/// AC-A2, AC-A3: a line evolite checked itself says «cryptographically checked» in its name;
/// one still pending does not; the seal is no Tab stop of its own (the Tab sweep holds that for
/// every screen).
#[test]
fn a_checked_line_says_so_and_a_pending_one_does_not() {
    let (_turn, mut kit) = demo_app("activity", "en");
    wait_for(&mut kit, "the lines", |kit| painted_has(kit, "Completed"));
    let all = names(&kit);
    let checked: Vec<&String> = all
        .iter()
        .filter(|n| n.ends_with("cryptographically checked"))
        .collect();
    assert!(!checked.is_empty(), "no checked line: {all:?}");
    // The demo's first line is still pending (in the mempool): no seal, no such words.
    let lines: Vec<&String> = all.iter().filter(|n| n.contains(" DASH")).collect();
    assert!(
        lines
            .iter()
            .any(|n| !n.contains("cryptographically checked")),
        "every line says checked: {lines:?}"
    );
}

/// AC-A5: at 1280 × 800 in English the address in a line's title is whole, no «…».
#[test]
fn a_wide_window_shows_the_id_in_a_line_whole() {
    let (_turn, mut kit) = demo_app("activity", "en");
    kit.set_size(egui::vec2(1280.0, 800.0));
    wait_for(&mut kit, "the withdrawal's line", |kit| {
        painted_texts(kit).iter().any(|t| t.contains(TO))
    });
    let title = painted_texts(&kit)
        .into_iter()
        .find(|t| t.contains(TO))
        .unwrap();
    assert!(!title.contains('…'), "{title}");
}

/// AC-A6: at 800 × 440 in Russian and Japanese the address is shortened in its middle, never
/// shorter than 4…4, on one line; the line's name keeps it whole.
#[test]
fn a_small_window_shortens_the_id_in_its_middle_and_says_it_whole() {
    for lang in ["ru", "ja"] {
        let (_turn, mut kit) = demo_app("activity", lang);
        kit.set_size(egui::vec2(800.0, 440.0));
        let head = &TO[..4];
        let tail = &TO[TO.len() - 4..];
        wait_for(&mut kit, "the withdrawal's line", |kit| {
            painted_texts(kit)
                .iter()
                .any(|t| t.contains(head) && t.contains(tail))
        });
        let title = painted_texts(&kit)
            .into_iter()
            .find(|t| t.contains(head) && t.contains(tail))
            .unwrap();
        if !title.contains(TO) {
            let at = title.find(head).unwrap();
            let shown: String = title[at..].split_whitespace().next().unwrap().to_owned();
            let (front, back) = shown.split_once('…').expect("shortened in its middle");
            assert!(front.chars().count() >= 4, "{lang}: {shown}");
            assert!(
                back.trim_end_matches(['…', ',', '.']).chars().count() >= 4,
                "{lang}: {shown}"
            );
            assert!(TO.starts_with(front), "{lang}: {shown}");
        }
        assert!(
            names(&kit).iter().any(|n| n.contains(TO)),
            "{lang}: the whole id in the line's name"
        );
        if !title.contains(TO) {
            // R-5: and whole under the pointer.
            let at = kit
                .query_all_by(|n| n.label().is_some_and(|l| l.contains(TO)))
                .next()
                .expect("the withdrawal's line")
                .rect()
                .center();
            kit.hover_at(at);
            wait_for(&mut kit, "the whole id under the pointer", |kit| {
                kit.query_all_by(|n| {
                    n.role() == Role::Label
                        && n.value().is_some_and(|v| v.contains(TO))
                })
                .next()
                .is_some()
            });
        }
    }
}

/// AC-A9 (R-7): the Overview's line in plain words; the advanced one with «confirmed».
#[test]
fn the_overview_says_completed_in_plain_words() {
    let (_turn, mut kit) = demo_app("overview", "en");
    wait_for(&mut kit, "the line", |kit| {
        has_words(kit, "All completed · updated")
            || has_words(kit, "Some payments are pending · updated")
    });
    kit.state_mut().settings.set("advanced", true);
    wait_for(&mut kit, "the advanced words", |kit| {
        has_words(kit, "All confirmed · updated")
            || has_words(kit, "Some payments are unconfirmed · updated")
    });
}

/// AC-A4: the card's «Copy» puts the whole id on the ordinary clipboard and says «Copied».
#[test]
fn the_cards_copy_puts_the_whole_address_and_says_copied() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    let copy = "Copy: this wallet's address";
    wait_for(&mut kit, "«Copy»", |kit| {
        kit.query_all_by_label(copy).next().is_some()
    });
    kit.query_all_by_label(copy).next().unwrap().click();
    kit.run_steps(3);
    let copied = kit.output().platform_output.commands.iter().any(|c| {
        matches!(c, egui::OutputCommand::CopyText(t) if t.starts_with('y') && !t.contains('…') && t.len() >= 34)
    });
    assert!(
        copied || has_text(&kit, "Copied"),
        "the whole address copied"
    );
    assert!(
        kit.query_all_by_label("Copied").next().is_some(),
        "«Copied» in its place"
    );
}
