//! FEAT-0045: «Backup file» on Settings › Backup, on the demo core (which «saves» in memory).

use super::*;
use evolite_api::{Command, NoteText};

fn click(kit: &mut Kit, name: &str) {
    kit.query_all_by_role_and_label(Role::Button, name)
        .next()
        .unwrap_or_else(|| panic!("no «{name}»"))
        .click();
    kit.run_steps(3);
}

fn path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "evolite-kit-{name}-{}.evbackup",
        std::process::id()
    ))
}

/// The first line of the demo's history, its key and note.
fn first_line(kit: &Kit) -> (String, Option<String>) {
    let a = &kit
        .state()
        .snapshot
        .as_ref()
        .unwrap()
        .wallet
        .as_ref()
        .unwrap()
        .activity[0];
    (a.key.clone(), a.note.clone())
}

fn note(kit: &mut Kit, key: &str, text: &str) {
    kit.state().core.send(Command::SetNote {
        key: key.into(),
        text: NoteText(text.into()),
    });
}

/// AC-A13 (the group's half): behind «Features for advanced users» — none while it is off.
#[test]
fn the_backup_file_is_behind_the_advanced_switch() {
    let (_turn, mut kit) = demo_app("backup", "en");
    assert!(!has_text(&kit, "Save a backup file…"), "off: no group");
    kit.state_mut().settings.set("advanced", true);
    // The group is below the fold of 1024 × 768: a click there does not land.
    kit.set_size(egui::vec2(1024.0, 1600.0));
    kit.run_steps(3);
    assert!(has_text(&kit, "Save a backup file…"));
    assert!(has_text(&kit, "Restore from a file…"));
    assert!(has_words(&kit, "only this wallet can open it"));
}

/// UC-1, UC-2 (AC-A1's interface half): saved — «Saved: {name}»; a note deleted since comes
/// back from the file — «Restored from the file: 1 note»; again — «nothing new» (AC-A4); a
/// cancelled dialog sends nothing.
#[test]
fn a_saved_file_brings_back_a_deleted_note() {
    let (_turn, mut kit) = demo_app("backup", "en");
    kit.state_mut().settings.set("advanced", true);
    // The group is below the fold of 1024 × 768: a click there does not land.
    kit.set_size(egui::vec2(1024.0, 1600.0));
    let (key, _) = first_line(&kit);
    note(&mut kit, &key, "Rent for May");
    wait_for(&mut kit, "the note", |kit| {
        first_line(kit).1.as_deref() == Some("Rent for May")
    });
    // Cancelled: nothing said, nothing waited for.
    crate::backup_file::test_pick(None);
    click(&mut kit, "Save a backup file…");
    kit.run_steps(3);
    assert!(kit.state().settings.backup_file.waiting.is_none());
    assert!(kit.state().settings.backup_file.said.is_none());

    let file = path("circle");
    crate::backup_file::test_pick(Some(file.clone()));
    click(&mut kit, "Save a backup file…");
    let saved = format!("Saved: {}", file.file_name().unwrap().to_string_lossy());
    wait_for(&mut kit, "«Saved»", |kit| has_text(kit, &saved));

    note(&mut kit, &key, "");
    wait_for(&mut kit, "the note deleted", |kit| {
        first_line(kit).1.is_none()
    });
    crate::backup_file::test_pick(Some(file.clone()));
    click(&mut kit, "Restore from a file…");
    wait_for(&mut kit, "«Restored»", |kit| {
        has_text(kit, "Restored from the file: 1 note")
    });
    assert_eq!(first_line(&kit).1.as_deref(), Some("Rent for May"));

    crate::backup_file::test_pick(Some(file));
    click(&mut kit, "Restore from a file…");
    wait_for(&mut kit, "«nothing new»", |kit| {
        has_text(kit, "Restored from the file: nothing new")
    });
}

/// AC-A3 (the interface's half): a file that does not open says why, and nothing changes.
#[test]
fn a_file_that_does_not_open_says_why() {
    let (_turn, mut kit) = demo_app("backup", "en");
    kit.state_mut().settings.set("advanced", true);
    // The group is below the fold of 1024 × 768: a click there does not land.
    kit.set_size(egui::vec2(1024.0, 1600.0));
    kit.run_steps(2);
    crate::backup_file::test_pick(Some(path("never-saved")));
    click(&mut kit, "Restore from a file…");
    wait_for(&mut kit, "the refusal", |kit| {
        has_text(kit, "The file is damaged or changed")
    });
}
