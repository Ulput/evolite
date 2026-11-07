//! FEAT-0003: local payment notes — in History's rows and its search, in an operation's card
//! (added, edited and deleted in place; Escape and a click outside are the edit's, not the
//! card's), in Send and its last look, and gone with a lock or another wallet.

use super::*;
use crate::screens;
use evolite_api::{Command, Password, VaultState};

/// The demo's note to operation 3, as `--screen activity-card-note-edit` opens it.
const NOTE: &str = "For the logo design · invoice no. 14 of 30 October";

/// The rows of History, as a screen reader hears them («title · amount · detail · “note”»).
fn rows(kit: &Kit) -> Vec<String> {
    kit.query_all_by_role(Role::Button)
        .filter_map(|n| {
            let name = n.accesskit_node().label()?;
            (name.matches(" · ").count() >= 2 && name.contains("DASH")).then_some(name)
        })
        .collect()
}

/// The note's field in the card, if it is being edited.
fn field(kit: &Kit) -> Option<egui_kittest::Node<'_>> {
    text_field(kit, "Note · only you see it")
}

/// A text field named `name`, of one line or several.
fn text_field<'k>(kit: &'k Kit, name: &'k str) -> Option<egui_kittest::Node<'k>> {
    kit.query_all_by_label(name).find(|n| {
        matches!(
            n.accesskit_node().role(),
            Role::TextInput | Role::MultilineTextInput
        )
    })
}

/// Whether the operation's card is open (its own «Close» is drawn).
fn card_open(kit: &Kit) -> bool {
    kit.query_all_by_role_and_label(Role::Button, "Close")
        .next()
        .is_some()
}

/// A press and release of the pointer at `at`.
fn click_at(kit: &mut Kit, at: egui::Pos2) {
    let input = kit.input_mut();
    input.events.push(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        input.events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    kit.run_steps(3);
}

/// FR-4, FR-9 (AC-A11, AC-D3, AC-D8): a row says its note in the language's quotes; the search
/// finds an operation by a word of its note, any case.
#[test]
fn history_rows_show_their_notes_and_the_search_finds_one() {
    let (_turn, mut kit) = demo_app("activity-notes", "ru");
    wait_for(&mut kit, "a row with its note", |kit| {
        rows(kit)
            .iter()
            .any(|r| r.ends_with("«За дизайн логотипа · счёт №14 от 30 октября»"))
    });
    assert!(rows(&kit).iter().any(|r| r.ends_with("«Аренда за ноябрь»")));
    drop(kit);
    drop(_turn);

    let (_turn, mut kit) = demo_app("activity-search-note", "ru");
    wait_for(&mut kit, "the rows found", |kit| !rows(kit).is_empty());
    let found = rows(&kit);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("логотипа"), "{found:?}");
    // The search field says what it looks in (N13).
    kit.get_by_role_and_label(Role::TextInput, "Адрес, имя, сумма, заметка");
}

/// UC-2, FR-5 (AC-A4, AC-D4): «Add a note» in the card of an operation without one — the field
/// takes the focus, Enter saves, the card stays open with the note and «Edit», and the row has
/// it.
#[test]
fn a_note_is_added_in_the_card_and_shows_in_its_row() {
    let (_turn, mut kit) = demo_app("activity-card-add-note", "en");
    wait_for(&mut kit, "«Add a note»", |kit| {
        kit.query_by_role_and_label(Role::Button, "Add a note")
            .is_some()
    });
    button(&kit, "Add a note").click();
    kit.run_steps(3);
    let field_focused = field(&kit)
        .expect("the note's field")
        .accesskit_node()
        .is_focused();
    assert!(field_focused, "the focus in the field");
    field(&kit).unwrap().type_text("Taxes 2026");
    kit.run_steps(2);
    kit.key_press(Key::Enter);
    wait_for(&mut kit, "the row with the new note", |kit| {
        rows(kit).iter().any(|r| r.ends_with("“Taxes 2026”"))
    });
    kit.run_steps(2);
    assert!(field(&kit).is_none(), "back to the note as shown");
    assert!(card_open(&kit), "the card stays");
    assert!(has_text(&kit, "Taxes 2026"));
    button(&kit, "Edit");
}

/// AC-D4: while a note is edited, a click outside the card does not close it and Escape cancels
/// the edit only; the next Escape closes the card.
#[test]
fn escape_and_a_click_outside_belong_to_the_edit_not_to_the_card() {
    let (_turn, mut kit) = demo_app("activity-card-note-edit", "en");
    wait_for(&mut kit, "the note's field", |kit| field(kit).is_some());
    click_at(&mut kit, egui::pos2(12.0, 760.0));
    assert!(field(&kit).is_some(), "still editing");
    assert!(card_open(&kit), "a click outside");
    kit.key_press(Key::Escape);
    kit.run_steps(3);
    assert!(field(&kit).is_none(), "Escape cancelled the edit");
    assert!(card_open(&kit), "and left the card");
    assert!(has_text(&kit, NOTE), "the note as it was");
    kit.key_press(Key::Escape);
    kit.run_steps(3);
    assert!(
        !card_open(&kit),
        "Escape closes the card when nothing is edited"
    );
    // And a click outside the card, with nothing edited, closes it as before.
    screens::open_card(&kit.ctx, 3);
    kit.run_steps(3);
    assert!(card_open(&kit));
    click_at(&mut kit, egui::pos2(12.0, 760.0));
    assert!(!card_open(&kit));
}

/// UC-3, FR-5 (AC-A5, AC-D4): an emptied note turns «Save» into «Delete the note», which deletes
/// it: the row has no note, the card offers «Add a note».
#[test]
fn an_emptied_note_is_deleted() {
    let (_turn, mut kit) = demo_app("activity-card-note-edit", "en");
    wait_for(&mut kit, "the note's field", |kit| field(kit).is_some());
    kit.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    kit.key_press(Key::Backspace);
    kit.run_steps(3);
    assert!(kit.query_by_role_and_label(Role::Button, "Save").is_none());
    button(&kit, "Delete the note").click();
    wait_for(&mut kit, "the row without its note", |kit| {
        !rows(kit).is_empty() && !rows(kit).iter().any(|r| r.contains("For the logo design"))
    });
    kit.run_steps(2);
    button(&kit, "Add a note");
    assert!(card_open(&kit));
}

/// AC-D5: a write that failed says why under the field, with «Details»; the text stays and
/// «Save» is there to try again.
#[test]
fn a_note_not_saved_says_why_and_keeps_the_text() {
    let (_turn, mut kit) = demo_app("activity-card-note-error", "en");
    wait_for(&mut kit, "the note's field", |kit| field(kit).is_some());
    assert!(has_text(
        &kit,
        "The note was not saved: there is no space left on the disk."
    ));
    button(&kit, "Details");
    assert_eq!(field(&kit).unwrap().value().as_deref(), Some(NOTE));
    assert!(!button(&kit, "Save").accesskit_node().is_disabled());
}

/// AC-T6 (§4a, a lock): a note half edited goes with the lock, as the rest typed does.
#[test]
fn a_lock_wipes_a_note_being_edited() {
    let (_turn, mut kit) = demo_app("activity-card-note-edit", "en");
    wait_for(&mut kit, "the note's field", |kit| field(kit).is_some());
    field(&kit).unwrap().type_text(", paid");
    kit.run_steps(2);
    assert!(screens::activity::note_edit(&kit.ctx).is_some());
    // The demo core does not lock: the snapshot says it did, as the real core's would.
    for _ in 0..3 {
        if let Some(s) = kit.state_mut().snapshot.as_mut() {
            s.vault = VaultState::Locked;
        }
        kit.run_steps(1);
    }
    assert!(screens::activity::note_edit(&kit.ctx).is_none());
}

/// §4a (another wallet): a note half edited in Main's card is not Savings'.
#[test]
fn another_wallet_opens_without_the_note_being_edited() {
    let (_turn, mut kit) = demo_app("activity-card-note-edit", "en");
    wait_for(&mut kit, "the note's field", |kit| field(kit).is_some());
    kit.state_mut().core.send(Command::SwitchWallet {
        id: "5e6f7a8b".into(),
        password: Some(Password::new("savings-password".into())),
    });
    wait_for(&mut kit, "Savings open", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallets.iter().find(|w| w.open))
            .is_some_and(|w| w.id == "5e6f7a8b")
    });
    kit.run_steps(2);
    assert!(screens::activity::note_edit(&kit.ctx).is_none());
}

/// UC-1, AC-A2, AC-A3 (AC-D2): the note is in «Check the payment»; a payment the core refused
/// gives it back to the form (the demo core never sends).
#[test]
fn the_note_is_in_the_last_look_and_comes_back_when_not_sent() {
    let (_turn, mut kit) = demo_app("send-confirm-note", "en");
    wait_for(&mut kit, "«Sign and send»", |kit| {
        kit.query_all_by_role_and_label(Role::Button, "Sign and send")
            .any(|b| !b.accesskit_node().is_disabled())
    });
    assert!(has_text(&kit, "For the logo design"));
    assert!(has_text(
        &kit,
        "Saved to the payment on this computer; it does not go to the network."
    ));
    button(&kit, "Sign and send").click();
    wait_for(&mut kit, "the core's answer", |kit| {
        kit.state().send.result.is_some()
    });
    assert!(kit.state().send.result.as_ref().unwrap().is_err());
    assert_eq!(kit.state().send.note, "For the logo design");
}

/// AC-A1, AC-D1: more pasted than a note holds — cut, the counter at the limit, N4 under the
/// field; the summary's last row is the note.
#[test]
fn a_note_cut_at_the_limit_says_so() {
    let (_turn, mut kit) = demo_app("send-note-limit", "en");
    wait_for(&mut kit, "the note's field", |kit| {
        text_field(kit, "Note · only for you").is_some()
    });
    assert!(has_text(&kit, "140 / 140"));
    assert!(has_text(
        &kit,
        "140 characters is all a note holds: the rest was cut off."
    ));
    assert!(
        kit.query_all_by(|n| n
            .value()
            .is_some_and(|l| l.starts_with("Note: For the logo, business cards")))
            .next()
            .is_some(),
        "the summary's row"
    );
}

/// E2 (AC-D6, review r1 must 1): the line says what became of the file. Put aside: new notes
/// go to a new file. Not read now: it is left as it is and no note is saved until the wallet
/// opens again (the core refuses every write then).
#[test]
fn the_notes_line_says_what_became_of_the_file() {
    const ASIDE: &str = "The notes file is damaged or cannot be decrypted. It is kept next to the wallet as is; new notes go to a new file.";
    const NOT_READ: &str = "The notes file could not be read now. It is left as it is, and notes cannot be changed until the wallet is opened again.";
    let (_turn, mut kit) = demo_app("activity-notes-unreadable", "en");
    wait_for(&mut kit, "the line of a file put aside", |kit| {
        has_text(kit, ASIDE)
    });
    assert!(!has_text(&kit, NOT_READ));
    let app = kit.state_mut();
    app.dev.notes_error = Some(evolite_api::NotesError::NotRead);
    if let Some(w) = app.snapshot.as_mut().and_then(|s| s.wallet.as_mut()) {
        w.notes_error = Some(evolite_api::NotesError::NotRead);
    }
    wait_for(&mut kit, "the line of a file not read", |kit| {
        has_text(kit, NOT_READ)
    });
    assert!(!has_text(&kit, ASIDE));
}
