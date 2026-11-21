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

/// FEAT-0045 step 2 (the customer, plain-language Q7): the backup file is for every user —
/// with «Features for advanced users» off too.
#[test]
fn the_backup_file_is_for_every_user() {
    let (_turn, mut kit) = demo_app("backup", "en");
    kit.set_size(egui::vec2(1024.0, 1600.0));
    kit.run_steps(3);
    assert!(!kit.state().settings.is_on("advanced"));
    assert!(has_text(&kit, "Save a backup file…"));
    assert!(has_text(&kit, "Restore from a file…"));
    assert!(has_words(&kit, "only this wallet can open it"));
    assert!(has_words(&kit, "Save a copy once a week"));
}

fn toggle_weekly(kit: &mut Kit) {
    kit.get_by_role_and_label(Role::CheckBox, "Save a copy once a week")
        .click();
    kit.run_steps(3);
}

fn auto(kit: &Kit) -> Option<evolite_api::backup::AutoBackup> {
    kit.state()
        .snapshot
        .as_ref()?
        .wallet
        .as_ref()?
        .auto_backup
        .clone()
}

/// UC-4 (AC-A11's interface half): on — the folder is asked, the copies go there, its path and
/// «the last 5» under the switch, «Change folder»; off — none.
#[test]
fn a_weekly_copy_goes_to_the_folder_chosen() {
    let (_turn, mut kit) = demo_app("backup", "en");
    kit.set_size(egui::vec2(1024.0, 1600.0));
    kit.run_steps(3);
    let folder = std::env::temp_dir().join("evolite-kit-copies");
    crate::backup_file::test_pick(Some(folder.clone()));
    toggle_weekly(&mut kit);
    wait_for(&mut kit, "on", |kit| auto(kit).is_some());
    assert_eq!(auto(&kit).unwrap().folder, folder.to_string_lossy());
    wait_for(&mut kit, "its line", |kit| {
        has_words(kit, "The last 5 are kept")
    });
    assert!(has_text(&kit, "Change folder"));
    toggle_weekly(&mut kit);
    wait_for(&mut kit, "off", |kit| auto(kit).is_none());
    assert!(!has_text(&kit, "Change folder"));
}

/// AC-A12 (UC-5): a copy not written says why on Backup and on the Overview.
#[test]
fn a_copy_not_written_is_said_on_backup_and_overview() {
    let (_turn, mut kit) = demo_app("backup", "en");
    kit.set_size(egui::vec2(1024.0, 1600.0));
    kit.run_steps(3);
    crate::backup_file::test_pick(Some(std::env::temp_dir().join("missing-drive")));
    toggle_weekly(&mut kit);
    let said = "The automatic copy was not saved: the folder is not found.";
    wait_for(&mut kit, "on Backup", |kit| has_words(kit, said));
    kit.state_mut().screen = crate::screens::Screen::Overview;
    kit.run_steps(3);
    wait_for(&mut kit, "on the Overview", |kit| has_words(kit, said));
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
