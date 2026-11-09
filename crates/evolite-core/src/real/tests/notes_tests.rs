use super::*;

/// FEAT-0003 (AC-T7): "Sync again" keeps the payment notes, and they still open, on the
/// same keys.
#[test]
fn a_resync_keeps_the_payment_notes() {
    let dir = fresh_dir("resync-notes");
    let paths = Paths::new(dir.clone(), TEST_WALLET);
    std::fs::create_dir_all(paths.chain.spv.join("headers")).unwrap();
    std::fs::create_dir_all(paths.vault.parent().unwrap()).unwrap();
    std::fs::write(&paths.chain.store, b"x").unwrap();
    let seed = [7u8; 64];
    notes::Notes::open(paths.notes.clone(), &seed, "ab")
        .set("core:aa", Some("rent".into()))
        .unwrap();
    std::fs::write(&paths.resync, b"").unwrap();
    assert!(resync_on_start(&paths).unwrap());
    assert!(!paths.chain.store.exists());
    let back = notes::Notes::open(paths.notes.clone(), &seed, "ab");
    assert_eq!(back.get("core:aa"), Some("rent"));
    assert!(!back.unreadable());
    let _ = std::fs::remove_dir_all(&dir);
}

/// FEAT-0003: the notes open with the wallet and go with its lock (the key and every
/// decrypted note), are kept encrypted only, come back after the next unlock — with a new
/// password too, since their key is the seed's (AC-T8) — and are not taken while locked.
#[tokio::test(flavor = "multi_thread")]
async fn the_notes_open_with_the_wallet_and_go_with_its_lock() {
    let mut core = core_with_vault("notes-lock", "pw");
    assert!(!has_notes(&core));
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    assert!(has_notes(&core));
    let note = |text: &str| Command::SetNote {
        key: "core:KEY-QZX-4410".into(),
        text: NoteText(text.into()),
    };
    let (event, changed) = core.handle(note("For the logo ZQX-7731")).await;
    assert!(changed);
    assert!(matches!(
        event,
        Some(Event::Done {
            what: DoneWhat::Note,
            result: Ok(())
        })
    ));
    assert!(core.paths.notes.exists());
    let folder = core.paths.vault.parent().unwrap().to_owned();
    assert!(!every_file_text(&folder).contains("ZQX-7731"));
    assert!(!every_file_text(&folder).contains("KEY-QZX-4410"));
    // Not an operation's key: refused, nothing kept.
    let (event, _) = core
        .handle(Command::SetNote {
            key: "../wallet.json".into(),
            text: NoteText("x".into()),
        })
        .await;
    assert!(matches!(event, Some(Event::Done { result: Err(_), .. })));

    core.lock().await.unwrap();
    assert!(!has_notes(&core), "the key and the notes go with the lock");
    let (event, changed) = core.handle(note("while locked")).await;
    assert!(!changed && matches!(event, Some(Event::Done { result: Err(_), .. })));

    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    let (event, _) = core
        .handle(Command::ChangePassword {
            old: pw("pw"),
            new: pw("a new password 2026"),
        })
        .await;
    assert!(matches!(event, Some(Event::Done { result: Ok(()), .. })));
    core.lock().await.unwrap();
    assert_eq!(core.unlock(&pw("a new password 2026")).await, Ok(()));
    let notes = core.notes.lock().unwrap();
    let notes = notes.as_ref().unwrap();
    assert_eq!(
        notes.get("core:KEY-QZX-4410"),
        Some("For the logo ZQX-7731")
    );
    assert!(!notes.unreadable());
}

/// Review r1 must 1 (§4a, lock → unlock): a notes file the unlock could not read (a folder
/// in its place: no access) is not written over, "Details" says why, and the next unlock
/// that reads it saves notes again.
#[tokio::test(flavor = "multi_thread")]
async fn a_notes_file_not_read_at_the_unlock_is_kept_until_one_reads_it() {
    let mut core = core_with_vault("notes-not-read", "pw");
    std::fs::create_dir_all(core.paths.notes.join("in-the-way")).unwrap();
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    assert_eq!(
        core.notes.lock().unwrap().as_ref().unwrap().trouble(),
        Some(evolite_api::NotesError::NotRead)
    );
    let note = || Command::SetNote {
        key: "core:ab12".into(),
        text: NoteText("rent".into()),
    };
    let (event, _) = core.handle(note()).await;
    assert!(matches!(
        event,
        Some(Event::Done {
            what: DoneWhat::Note,
            result: Err(CoreError::NotAvailable(_))
        })
    ));
    assert!(core.paths.notes.join("in-the-way").is_dir());
    core.lock().await.unwrap();
    std::fs::remove_dir_all(&core.paths.notes).unwrap();
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    let (event, _) = core.handle(note()).await;
    assert!(matches!(event, Some(Event::Done { result: Ok(()), .. })));
    assert_eq!(core.notes.lock().unwrap().as_ref().unwrap().trouble(), None);
}

/// FEAT-0003 with BUG-76: "Add a wallet" locks this one (the loop locks the core it
/// replaces), so its notes go; "Back" opens it again with its password, and they come back.
#[tokio::test(flavor = "multi_thread")]
async fn adding_a_wallet_drops_the_notes_and_back_opens_them_again() {
    let mut core = core_with_vault("notes-add", "pw");
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    core.handle(Command::SetNote {
        key: "platform:00ff".into(),
        text: NoteText("rent".into()),
    })
    .await;
    let (_, changed) = core.handle(Command::AddWallet).await;
    assert!(changed && core.switch_to.is_some());
    // What the loop does with the core it replaces.
    core.lock().await.unwrap();
    assert!(!has_notes(&core) && core.unlocked.is_none());
    let dir = core.paths.root.clone();
    drop(core);
    // "Back": a new core for this wallet, locked until its password.
    let mut back = core_at(&dir, TEST_WALLET);
    assert!(!has_notes(&back));
    assert_eq!(back.snapshot().await.wallet.map(|w| w.notes_error), None);
    assert_eq!(back.unlock(&pw("pw")).await, Ok(()));
    assert_eq!(
        back.notes
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .get("platform:00ff"),
        Some("rent")
    );
}

/// FEAT-0003 (AC-T7, UC-7): a deleted wallet's notes go with its folder, so the same
/// phrase restored later finds none.
#[tokio::test]
async fn a_deleted_wallets_notes_go_with_its_folder() {
    let dir = fresh_dir("notes-deleted");
    let paths = Paths::new(dir.clone(), TEST_WALLET);
    notes::Notes::open(paths.notes.clone(), &[1; 64], "ab")
        .set("core:aa", Some("rent".into()))
        .unwrap();
    // Deleted: the vault is gone, the rest is left for the next start.
    assert!(!paths.vault.exists() && paths.notes.exists());
    assert!(clear_leftovers(&paths).await);
    assert!(!paths.notes.exists());
    let restored = notes::Notes::open(paths.notes.clone(), &[1; 64], "ab");
    assert!(!restored.any() && !restored.unreadable());
    let _ = std::fs::remove_dir_all(&dir);
}
