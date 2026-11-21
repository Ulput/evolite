use super::*;
use evolite_api::backup::{BackupError, Restored, Section};

fn file_at(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "evolite-test-backup-{name}-{}.evbackup",
        std::process::id()
    ))
}

async fn unlocked(name: &str) -> Real {
    let mut core = core_with_vault(name, "pw");
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    core
}

async fn save(core: &mut Real, path: &std::path::Path) -> Result<String, BackupError> {
    match core
        .handle(Command::SaveBackup {
            path: path.to_owned(),
            settings: Some(r#"{"fee_rate":2000}"#.into()),
        })
        .await
    {
        (Some(Event::BackupSaved(result)), _) => result,
        other => panic!("{:?}", debug(&other.0)),
    }
}

async fn restore(core: &mut Real, path: &std::path::Path) -> Result<Restored, BackupError> {
    match core
        .handle(Command::RestoreBackup {
            path: path.to_owned(),
        })
        .await
    {
        (Some(Event::BackupRestored(result)), _) => result,
        other => panic!("{:?}", debug(&other.0)),
    }
}

/// A wallet with a note, a named address and a hidden request, saved to a file.
async fn saved(name: &str, file: &std::path::Path) -> Real {
    let mut core = unlocked(name).await;
    core.handle(Command::SetNote {
        key: "core:aa".into(),
        text: NoteText("Rent for May".into()),
    })
    .await;
    core.address_book.save("yExchangeQZX", "Exchange").unwrap();
    core.hidden.add("Spammer1").unwrap();
    let name = save(&mut core, file).await.expect("saved");
    assert_eq!(name, file.file_name().unwrap().to_string_lossy());
    core
}

/// FEAT-0045 AC-A1 (the core's half): what the phrase does not bring back — a note, a named
/// address, a hidden request, the interface's settings — comes back from the file into the same
/// phrase on another computer (another data folder, another password). AC-A4: the same file
/// again changes nothing. AC-A2: the file has neither the phrase nor any of it in the clear.
#[tokio::test(flavor = "multi_thread")]
async fn a_backup_brings_back_what_the_phrase_does_not() {
    let file = file_at("circle");
    let _first = saved("backup-first", &file).await;
    // The phrase, in the clear, is not in the file: only the hex of the sealed part could hold
    // letters a word is made of, so the words are looked for outside it.
    let text = String::from_utf8(std::fs::read(&file).unwrap()).unwrap();
    let open_part: String = {
        let e: serde_json::Value = serde_json::from_str(&text).unwrap();
        format!("{} {} {}", e["format"], e["network"], e["wallet"])
    };
    for word in PHRASE.split_whitespace() {
        assert!(!open_part.contains(word), "{word}");
    }
    assert!(!text.contains(PHRASE) && !text.contains("Rent for May") && !text.contains("QZX"));

    let mut other = core_with_vault("backup-other", "another-pw");
    assert_eq!(other.unlock(&pw("another-pw")).await, Ok(()));
    let restored = restore(&mut other, &file).await.expect("restored");
    assert_eq!(
        restored,
        Restored {
            notes: 1,
            addresses: 1,
            hidden_requests: 1,
            settings: Some(r#"{"fee_rate":2000}"#.into()),
            ..Restored::default()
        }
    );
    let note = other
        .notes
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .get("core:aa")
        .map(str::to_owned);
    assert_eq!(note.as_deref(), Some("Rent for May"));
    assert_eq!(other.address_book.entries()[0].label, "Exchange");
    assert!(other.hidden.contains("Spammer1"));
    // AC-A4: again — nothing added.
    let again = restore(&mut other, &file).await.expect("again");
    assert_eq!(
        (
            again.notes,
            again.addresses,
            again.hidden_requests,
            again.notes_kept
        ),
        (0, 0, 0, 1)
    );
    let _ = std::fs::remove_file(&file);
}

/// AC-A3: a damaged file is refused and the wallet's files are not touched; locked, there is no
/// key to save or open with.
#[tokio::test(flavor = "multi_thread")]
async fn a_damaged_file_changes_nothing_and_a_locked_wallet_has_no_key() {
    let file = file_at("damaged");
    let mut core = saved("backup-damaged", &file).await;
    let mut bytes = std::fs::read(&file).unwrap();
    let at = bytes.len() - 5;
    bytes[at] = if bytes[at] == b'0' { b'1' } else { b'0' };
    std::fs::write(&file, &bytes).unwrap();
    let notes_before = std::fs::read(&core.paths.notes).unwrap();
    let book_before = std::fs::read(&core.paths.address_book).unwrap();
    assert_eq!(restore(&mut core, &file).await, Err(BackupError::Damaged));
    assert_eq!(std::fs::read(&core.paths.notes).unwrap(), notes_before);
    assert_eq!(
        std::fs::read(&core.paths.address_book).unwrap(),
        book_before
    );
    assert_eq!(
        restore(&mut core, &file_at("never-there")).await,
        Err(BackupError::Io("not_found".into()))
    );
    core.handle(Command::Lock).await;
    assert_eq!(save(&mut core, &file).await, Err(BackupError::Locked));
    assert_eq!(restore(&mut core, &file).await, Err(BackupError::Locked));
    let _ = std::fs::remove_file(&file);
}

/// AC-A9 (R-10): a section that cannot be written stops the restore there — the sections
/// before it are merged and written, it and the ones after are not; once it can be written, a
/// repeat finishes it and duplicates nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_restore_cut_short_is_finished_by_a_repeat() {
    let file = file_at("partial");
    let _first = saved("backup-partial-first", &file).await;
    let mut other = unlocked("backup-partial-other").await;
    // The hidden requests' file cannot be written: a folder stands in its place.
    std::fs::create_dir_all(&other.paths.hidden_requests).unwrap();
    match restore(&mut other, &file).await {
        Err(BackupError::Partial { done, failed }) => {
            assert_eq!(failed, Section::HiddenRequests);
            assert_eq!((done.notes, done.addresses), (1, 1));
        }
        other => panic!("{other:?}"),
    }
    assert!(!other.hidden.contains("Spammer1"));
    std::fs::remove_dir_all(&other.paths.hidden_requests).unwrap();
    let again = restore(&mut other, &file).await.expect("finished");
    assert_eq!(
        (again.notes, again.addresses, again.hidden_requests),
        (0, 0, 1)
    );
    assert!(other.hidden.contains("Spammer1"));
    let _ = std::fs::remove_file(&file);
}

/// AC-A6 (AC-T-B2′): every file `Paths` puts in a wallet's folder has a decision in
/// `backup::FOLDER_FILES` — taken or not, and why — and the list names no file the folder does
/// not have. A file new in the folder fails here until it is decided.
#[test]
fn every_file_of_the_wallet_folder_is_decided_for_the_backup() {
    let source = include_str!("../mod.rs");
    let mut in_folder: Vec<&str> = source
        .split("wallets.join(\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    // Beside the journal (`with_file_name`), and the interface's own half of the settings.
    in_folder.extend(["top-up-credits.json", "settings.json"]);
    let decided: Vec<&str> = evolite_local::backup::FOLDER_FILES
        .iter()
        .map(|(name, _, _)| *name)
        .collect();
    for name in &in_folder {
        assert!(
            decided.contains(name),
            "{name}: taken into the backup or not?"
        );
    }
    for name in &decided {
        assert!(
            in_folder.contains(name),
            "{name}: no such file in the folder"
        );
    }
    let taken = |name: &str| {
        evolite_local::backup::FOLDER_FILES
            .iter()
            .any(|(n, yes, _)| *n == name && *yes)
    };
    for never in [
        "wallet.json",
        "pending-names.json",
        "fund-intents.json",
        "platform-wallet.db",
    ] {
        assert!(!taken(never), "{never}");
    }
}

/// A core over PHRASE sealed with the BIP-39 `passphrase` (FEAT-0041), unlocked with «pw».
async fn unlocked_with(name: &str, passphrase: &str) -> Real {
    let dir = std::env::temp_dir().join(format!("evolite-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::new(dir.clone(), TEST_WALLET);
    let meta = VaultMeta {
        network: network().into(),
        wallet_id: "ab".repeat(32),
        birth_height: 1_000,
        word_count: 12,
        created_at: 1_793_000_000,
    };
    let cheap = KdfParams {
        memory_kib: 256,
        iterations: 1,
        parallelism: 1,
    };
    let secret = vault::Secret::new(PHRASE, passphrase);
    vault::save(
        &paths.vault,
        &vault::seal(&secret, "pw", meta, cheap).unwrap(),
    )
    .unwrap();
    let (out, _) = std::sync::mpsc::channel();
    let mut core = Real::new(dir, TEST_WALLET.into(), out, Arc::new(|| {}));
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    core
}

/// AC-A7 (review r1, Must 2): a copy of «the phrase + "x"» does not open in the phrase without a
/// passphrase — «another wallet» — and opens in «the phrase + "x"» on another computer.
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_opens_only_with_its_own_passphrase() {
    let file = file_at("passphrase");
    let mut with_x = unlocked_with("backup-pass-x", "x").await;
    with_x
        .handle(Command::SetNote {
            key: "core:aa".into(),
            text: NoteText("Rent for May".into()),
        })
        .await;
    save(&mut with_x, &file).await.expect("saved");
    let mut without = unlocked_with("backup-pass-none", "").await;
    assert_eq!(
        restore(&mut without, &file).await,
        Err(BackupError::OtherWallet)
    );
    let mut again_x = unlocked_with("backup-pass-x-again", "x").await;
    let restored = restore(&mut again_x, &file)
        .await
        .expect("the same pair opens it");
    assert_eq!(restored.notes, 1);
    let _ = std::fs::remove_file(&file);
}
