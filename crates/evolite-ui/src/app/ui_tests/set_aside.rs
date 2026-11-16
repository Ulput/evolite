//! BUG-204, BUG-212 (the analyst's AC-A1…A6): a banner on the screen of each file of the wallet
//! that was set aside — the file's name and no folder, «Try again» that leaves it when the file
//! still does not read, «Start this part over…» with what comes back from the network, «Show in
//! folder» and «Report a problem».

use super::*;
use crate::screens::Screen;

/// AC-A1: each file's banner on its screen; the journal also as a line on Overview.
#[test]
fn each_file_set_aside_has_its_banner_on_its_screen() {
    for (preset, title) in [
        (
            "history-set-aside",
            "Part of the Platform history could not be read",
        ),
        (
            "top-ups-set-aside",
            "The records of credit top-ups could not be read",
        ),
        ("names-set-aside", "Your name requests could not be read"),
        (
            "apps-set-aside",
            "The list of connected apps could not be read",
        ),
        (
            "contacts-set-aside",
            "Your contacts' payment addresses could not be read",
        ),
        (
            "address-book-set-aside",
            "The address book could not be read",
        ),
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        wait_for(&mut kit, title, |kit| has_words(kit, title));
        assert!(has_words(
            &kit,
            "Money and data on the network are not affected"
        ));
        button(&kit, "Try again");
        button(&kit, "Show in folder");
        button(&kit, "Start this part over…");
    }
    let (_turn, mut kit) = demo_app("overview-set-aside", "en");
    wait_for(&mut kit, "the line over Latest operations", |kit| {
        kit.query_by_role_and_label(Role::Button, "Part of the history could not be read ›")
            .is_some()
    });
    button(&kit, "Part of the history could not be read ›").click();
    kit.run_steps(3);
    assert_eq!(kit.state().screen, Screen::Activity);
    assert!(has_words(
        &kit,
        "Part of the Platform history could not be read"
    ));
}

/// AC-A6: the banner names the file set aside and never its folder.
#[test]
fn the_banner_names_the_file_not_the_folder() {
    let (_turn, mut kit) = demo_app("history-set-aside", "en");
    wait_for(&mut kit, "the banner", |kit| {
        has_words(kit, "Journal.json.unreadable-1763302200")
    });
    assert!(!has_words(&kit, "C:\\evolite"));
    button(&kit, "Show in folder").click();
    kit.run_steps(2);
    assert!(
        crate::links::opened()
            .iter()
            .any(|l| l.starts_with("folder:") && l.ends_with("Journal.json.unreadable-1763302200")),
        "{:?}",
        crate::links::opened()
    );
}

/// AC-A2: «Try again» on a file that still does not read leaves the banner and says so.
#[test]
fn try_again_on_a_file_that_still_does_not_read_keeps_the_banner() {
    let (_turn, mut kit) = demo_app("apps-set-aside", "en");
    wait_for(&mut kit, "the banner", |kit| {
        has_words(kit, "The list of connected apps could not be read")
    });
    button(&kit, "Try again").click();
    wait_for(&mut kit, "the answer", |kit| {
        has_words(kit, "The file still does not read")
    });
    assert!(has_words(
        &kit,
        "The list of connected apps could not be read"
    ));
}

/// AC-A3: «Start this part over…» asks first, with what comes back from the network; «Cancel»
/// changes nothing, «Start over» takes the banner away.
#[test]
fn start_over_asks_first_and_then_the_banner_goes() {
    let (_turn, mut kit) = demo_app("history-set-aside", "en");
    wait_for(&mut kit, "the banner", |kit| {
        has_words(kit, "Part of the Platform history could not be read")
    });
    button(&kit, "Start this part over…").click();
    wait_for(&mut kit, "the question", |kit| {
        has_words(kit, "Start the Platform history over?")
    });
    assert!(has_words(&kit, "come back from the network"));
    assert!(has_words(&kit, "nothing is deleted"));
    button(&kit, "Cancel").click();
    wait_for(&mut kit, "the question closed", |kit| {
        !has_words(kit, "Start the Platform history over?")
    });
    assert!(has_words(
        &kit,
        "Part of the Platform history could not be read"
    ));
    button(&kit, "Start this part over…").click();
    wait_for(&mut kit, "the question", |kit| {
        has_words(kit, "Start the Platform history over?")
    });
    button(&kit, "Start over").click();
    wait_for(&mut kit, "the banner gone", |kit| {
        !has_words(kit, "Part of the Platform history could not be read")
    });
}

/// The analyst's action 4: «Report a problem» opens the report with the kind of file in its
/// subject, no path.
#[test]
fn report_a_problem_opens_with_the_subject_filled_in() {
    let (_turn, mut kit) = demo_app("address-book-set-aside", "en");
    wait_for(&mut kit, "the banner", |kit| {
        has_words(kit, "The address book could not be read")
    });
    click_words_of(&mut kit, "Report a problem");
    kit.run_steps(3);
    assert_eq!(kit.state().screen, Screen::Settings);
    let summary = kit
        .state()
        .settings
        .report
        .as_ref()
        .map(|r| r.summary.clone());
    assert_eq!(
        summary.as_deref(),
        Some("A file was set aside: the address book")
    );
}

/// AC-A5: «×» has its name and collapses the banner for this run (until a restart).
#[test]
fn the_cross_hides_the_banner_until_a_restart() {
    let (_turn, mut kit) = demo_app("contacts-set-aside", "en");
    wait_for(&mut kit, "the banner", |kit| {
        has_words(kit, "Your contacts' payment addresses could not be read")
    });
    assert!(has_words(&kit, "new records of this part are not saved"));
    button(&kit, "Hide until restart").click();
    wait_for(&mut kit, "the banner hidden", |kit| {
        !has_words(kit, "Your contacts' payment addresses could not be read")
    });
    // The file is still set aside: only this run's view changed.
    assert_eq!(
        kit.state()
            .snapshot
            .as_ref()
            .unwrap()
            .wallet
            .as_ref()
            .unwrap()
            .set_aside
            .len(),
        1
    );
}

/// The analyst: the journal's banner says its new lines show and when they are kept.
#[test]
fn the_journals_banner_says_its_new_lines_wait_in_memory() {
    let (_turn, mut kit) = demo_app("history-set-aside", "en");
    wait_for(&mut kit, "the banner", |kit| {
        has_words(kit, "Part of the Platform history could not be read")
    });
    assert!(has_words(&kit, "New records still show here"));
    assert!(!has_words(&kit, "new records of this part are not saved"));
}
