use super::*;

#[test]
fn a_resync_clears_the_chain_and_the_store_and_keeps_the_rest() {
    let dir = std::env::temp_dir().join(format!("evolite-resync-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::new(dir.clone(), TEST_WALLET);
    let wallets = paths.vault.parent().unwrap().to_owned();
    std::fs::create_dir_all(&wallets).unwrap();
    std::fs::create_dir_all(paths.chain.spv.join("headers")).unwrap();
    let store_wal = wallets.join("platform-wallet.db-wal");
    let backfill = store::backfill_file(&paths.chain.store);
    for file in [
        &paths.vault,
        &paths.journal,
        &paths.platform_sync,
        &paths.contact_addresses,
        &paths.chain.store,
        &store_wal,
        &backfill,
    ] {
        std::fs::write(file, b"x").unwrap();
    }
    assert!(
        !resync_on_start(&paths).unwrap(),
        "nothing asked: nothing goes"
    );
    assert!(paths.chain.store.exists());

    std::fs::write(&paths.resync, b"").unwrap();
    assert!(resync_on_start(&paths).unwrap());
    assert!(!paths.chain.spv.exists() && !paths.chain.store.exists() && !store_wal.exists());
    assert!(paths.vault.exists() && paths.journal.exists() && paths.contact_addresses.exists());
    // BUG-58: Platform is read back from the start; the journal stays.
    assert!(!paths.platform_sync.exists());
    // BUG-77: both marks go — the backfill record, and the spent coins with the store.
    assert!(
        !backfill.exists(),
        "the backfill record vouches for a scan that is gone"
    );
    assert_eq!(
        store::spent_claims(&paths.chain.store, &[0; 32], 0),
        store::Claims::default()
    );
    assert!(!paths.resync.exists(), "once");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_data_folder_one_evolite() {
    let dir = std::env::temp_dir().join(format!("evolite-lock-{}", std::process::id()));
    let first = lock_data_dir(&dir).unwrap();
    assert!(first.is_some(), "the first window gets the folder");
    assert!(
        lock_data_dir(&dir).unwrap().is_none(),
        "a second one waits while the first is open"
    );
    drop(first);
    assert!(
        lock_data_dir(&dir).unwrap().is_some(),
        "and gets it once the first has closed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-8: unlocking starts the chain itself, but never over a deleted wallet's files —
/// the right password then says why, and the wallet stays locked.
#[tokio::test]
async fn unlocking_over_leftovers_says_restart() {
    let mut core = core_with_vault("unlock-leftovers", "pw");
    core.leftovers = true;
    assert_eq!(
        core.unlock(&pw("pw")).await,
        Err(CoreError::NotAvailable(LEFTOVERS.into()))
    );
    assert!(core.unlocked.is_none());
    // A wrong password is still said first.
    assert_eq!(core.unlock(&pw("no")).await, Err(CoreError::WrongPassword));
}

/// F3 (BUG-24): the password opens the wallet while the chain is still starting; the
/// wallet goes into platform-wallet once it runs, and until then a payment or a Platform
/// action is "wait for the sync", not an error. Locking forgets the phrase kept for that.
#[tokio::test]
async fn unlocking_does_not_wait_for_the_chain() {
    let mut core = core_with_vault("unlock-no-chain", "pw");
    assert!(!core.chain.running());
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    let id = core.unlocked.as_ref().map(|o| o.id).unwrap();
    assert_eq!(id, chain::wallet_id_of(PHRASE).unwrap());
    assert!(core.attach.is_some(), "opened in platform-wallet later");
    assert_eq!(core.snapshot().await.vault, VaultState::Unlocked);
    assert_eq!(
        loaded(&core.chain, &id).await.err(),
        Some(CoreError::NotSynced)
    );
    core.lock().await.unwrap();
    assert!(core.attach.is_none() && core.unlocked.is_none());
}

/// F4 (BUG-21): deleting forgets the vault at once — the next snapshot says "no wallet"
/// without waiting for the chain to stop (the loop stops it off the loop, F3).
#[tokio::test]
async fn deleting_says_no_wallet_at_once() {
    let mut core = core_with_vault("delete-at-once", "pw");
    assert_eq!(
        core.delete_wallet(Some(&pw("no"))).await,
        Err(CoreError::WrongPassword)
    );
    assert!(core.saved.is_some() && !core.restart);
    assert_eq!(core.delete_wallet(Some(&pw("pw"))).await, Ok(()));
    assert!(core.restart && !core.paths.vault.exists());
    assert_eq!(core.snapshot().await.vault, VaultState::NoWallet);
}

/// Each wallet's files are in its own folder: one wallet's store, journals and vault are
/// never another's. Only headers and filters and the chosen Core node are shared.
#[test]
fn wallets_share_no_files_but_the_chain() {
    let dir = fresh_dir("paths");
    let (a, b) = (
        Paths::new(dir.clone(), "aaaa0001"),
        Paths::new(dir.clone(), "bbbb0002"),
    );
    let own = |p: &Paths| {
        vec![
            p.chain.store.clone(),
            p.vault.clone(),
            p.pending_names.clone(),
            p.journal.clone(),
            p.platform_sync.clone(),
            p.contact_addresses.clone(),
            p.hidden_requests.clone(),
            p.pool_history.clone(),
            p.resync.clone(),
            p.address_book.clone(),
            p.mempool.clone(),
            p.pool.clone(),
            p.connected_apps.clone(),
        ]
    };
    for path in own(&a) {
        assert!(
            path.starts_with(wallets::wallet_dir(&dir, "aaaa0001")),
            "{path:?}"
        );
    }
    for path in own(&b) {
        assert!(
            path.starts_with(wallets::wallet_dir(&dir, "bbbb0002")),
            "{path:?}"
        );
    }
    assert_eq!(a.chain.spv, b.chain.spv);
    assert_eq!(a.chain.core_node, b.chain.core_node);
}

/// A phrase already a wallet here is refused when another wallet is added, by name.
#[tokio::test]
async fn a_phrase_already_here_is_not_a_second_wallet() {
    let dir = fresh_dir("same");
    seal_wallet(&dir, "aaaa0001");
    let mut core = core_at(&dir, "bbbb0002");
    let phrase = Phrase::new(PHRASE.split_whitespace().map(str::to_owned).collect());
    let (event, _) = core
        .handle(Command::UsePhrase {
            phrase,
            dashpay: false,
        })
        .await;
    match event {
        Some(Event::PhraseAccepted(Err(CoreError::NotAvailable(text)))) => {
            assert!(text.contains("already on this computer"), "{text}")
        }
        other => panic!("{other:?}"),
    }
    assert!(core.pending.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Switching asks the loop for the other wallet and remembers it for the next start; an
/// added wallet starts in a folder of its own, and "Cancel" goes back.
#[tokio::test]
async fn switching_and_adding_go_through_the_index() {
    let dir = fresh_dir("switch");
    seal_wallet(&dir, "aaaa0001");
    seal_wallet(&dir, "bbbb0002");
    let mut core = core_at(&dir, "aaaa0001");
    let open: Vec<bool> = core.wallet_entries().iter().map(|w| w.open).collect();
    assert_eq!(open, [true, false]);

    // Nowhere to go: the open one, or one not here.
    for id in ["aaaa0001", "cccc0003"] {
        core.handle(Command::SwitchWallet {
            id: id.into(),
            password: None,
        })
        .await;
        assert!(core.switch_to.is_none());
    }
    // Figma 29: with the other wallet's password, a wrong one changes nothing.
    let (event, _) = core
        .handle(Command::SwitchWallet {
            id: "bbbb0002".into(),
            password: Some(pw("wrong")),
        })
        .await;
    assert!(matches!(
        event,
        Some(Event::Done {
            what: "switch",
            result: Err(CoreError::WrongPassword)
        })
    ));
    assert!(core.switch_to.is_none());
    assert_eq!(wallets::load(&dir).current().unwrap().id, "aaaa0001");
    let (event, _) = core
        .handle(Command::SwitchWallet {
            id: "bbbb0002".into(),
            password: Some(pw("pw")),
        })
        .await;
    assert!(
        event.is_none(),
        "the new core answers once it has opened it"
    );
    assert_eq!(core.switch_to.as_deref(), Some("bbbb0002"));
    assert!(core.switch_opened.is_some());
    // The other wallet's phrase by its own password (its "…" → "Show the phrase").
    let (event, _) = core
        .handle(Command::RevealPhrase {
            password: pw("pw"),
            wallet: Some("bbbb0002".into()),
        })
        .await;
    assert!(matches!(event, Some(Event::PhraseRevealed(Ok(_)))));
    assert_eq!(wallets::load(&dir).current().unwrap().id, "bbbb0002");

    let mut core = core_at(&dir, "bbbb0002");
    core.handle(Command::AddWallet).await;
    let new = core.switch_to.clone().unwrap();
    assert!(new != "aaaa0001" && new != "bbbb0002");
    assert_eq!(core.adding_from.as_deref(), Some("bbbb0002"));
    // Until its vault is written the new folder is no wallet: the index has two.
    assert_eq!(wallets::load(&dir).wallets.len(), 2);

    let mut adding = core_at(&dir, &new);
    adding.adding_from = core.adding_from.take();
    assert!(adding.snapshot().await.adding_wallet);
    adding.handle(Command::CancelAddWallet).await;
    assert_eq!(adding.switch_to.as_deref(), Some("bbbb0002"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// R-28 (BUG-44): another wallet goes by its own password, files only; the open one stays
/// unlocked and no new core is asked for.
#[tokio::test]
async fn deletes_another_wallet_by_its_password_and_keeps_the_open_one() {
    let dir = fresh_dir("delete-other");
    seal_wallet(&dir, "aaaa0001");
    seal_wallet(&dir, "bbbb0002");
    let art = dir.join("art");
    std::fs::create_dir_all(art.join("bbbb0002")).unwrap();
    std::fs::create_dir_all(art.join("aaaa0001")).unwrap();
    let mut core = core_at(&dir, "aaaa0001");
    core.art_base = Some(art.clone());
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    let other = wallets::wallet_dir(&dir, "bbbb0002");
    std::fs::write(other.join("platform-wallet.db"), b"store").unwrap();
    let delete = |password: Option<&str>, wallet: &str| Command::DeleteWallet {
        password: password.map(pw),
        wallet: Some(wallet.into()),
    };
    let answer = |event: Option<Event>| match event {
        Some(Event::Done { what, result }) => (what, result),
        other => panic!("{other:?}"),
    };
    let untouched = |core: &Real| {
        assert!(core.unlocked.is_some(), "the open wallet stays unlocked");
        assert!(!core.restart && core.switch_to.is_none(), "no new core");
    };

    // A wrong password, or none: nothing goes.
    for password in [Some("wrong"), None] {
        let (event, _) = core.handle(delete(password, "bbbb0002")).await;
        assert_eq!(
            answer(event),
            ("delete-other", Err(CoreError::WrongPassword))
        );
        assert!(other.join("wallet.json").exists());
        untouched(&core);
    }
    // Its own password: its folder, record and pictures go; the open one's stay.
    let (event, changed) = core.handle(delete(Some("pw"), "bbbb0002")).await;
    assert_eq!(answer(event), ("delete-other", Ok(())));
    assert!(changed, "the list of wallets is sent again");
    assert!(!other.exists());
    assert!(!art.join("bbbb0002").exists() && art.join("aaaa0001").exists());
    let ids: Vec<String> = wallets::load(&dir)
        .wallets
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["aaaa0001"]);
    assert_eq!(core.wallet_entries().len(), 1);
    assert!(core.paths.vault.exists());
    untouched(&core);
    // Not there any more.
    let (event, _) = core.handle(delete(Some("pw"), "bbbb0002")).await;
    assert!(matches!(
        answer(event),
        ("delete-other", Err(CoreError::NotAvailable(_)))
    ));
    // The open one's id is the open one's deletion, as before.
    let (event, _) = core.handle(delete(Some("wrong"), "aaaa0001")).await;
    assert_eq!(answer(event), ("delete", Err(CoreError::WrongPassword)));
    untouched(&core);
    let _ = std::fs::remove_dir_all(&dir);
}

/// FEAT-0002 (AC-A2, AC-A5, AC-A10, AC-T2): the 5th wrong password in a row starts a pause
/// of 30 s; in it even the right password is refused without the vault being read, and a
/// new core over the folder (a restart) is in the same pause.
#[tokio::test]
async fn the_fifth_wrong_password_pauses_the_unlock_even_after_a_restart() {
    let dir = fresh_dir("unlock-pause");
    seal_wallet(&dir, "aaaa0001");
    seal_wallet(&dir, "bbbb0002");
    let mut core = core_at(&dir, "aaaa0001");
    let unlock = |p: &str| Command::Unlock { password: pw(p) };
    for n in 1..=5 {
        let (event, changed) = core.handle(unlock("no")).await;
        assert_eq!(
            answer_of(event),
            ("unlock", Err(CoreError::WrongPassword)),
            "try {n}"
        );
        assert!(changed, "the count goes out with the next snapshot");
        assert_eq!(entry(&mut core, "aaaa0001").wrong_tries, n);
        assert_eq!(entry(&mut core, "aaaa0001").wait_until.is_some(), n == 5);
    }
    let until = entry(&mut core, "aaaa0001").wait_until.unwrap();
    assert!((29..=31).contains(&(until - now())), "{}", until - now());
    // The vault is not even read: without it the answer is still the pause.
    let vault = wallets::wallet_dir(&dir, "aaaa0001").join("wallet.json");
    let sealed = std::fs::read(&vault).unwrap();
    std::fs::remove_file(&vault).unwrap();
    let (event, _) = core.handle(unlock("pw")).await;
    assert_eq!(
        answer_of(event),
        ("unlock", Err(CoreError::TooManyTries { until }))
    );
    std::fs::write(&vault, sealed).unwrap();
    assert!(core.unlocked.is_none());
    assert_eq!(
        entry(&mut core, "aaaa0001").wrong_tries,
        5,
        "a refusal is no try"
    );
    // The other wallet has no pause.
    let other = entry(&mut core, "bbbb0002");
    assert_eq!((other.wrong_tries, other.wait_until), (0, None));
    // A restart: the same pause.
    let mut again = core_at(&dir, "aaaa0001");
    assert_eq!(entry(&mut again, "aaaa0001").wait_until, Some(until));
    assert_eq!(entry(&mut again, "aaaa0001").wrong_tries, 5);
    let _ = std::fs::remove_dir_all(&dir);
}

/// FEAT-0002 (AC-A4, AC-A6, AC-A7, AC-A8, AC-A9): errors that are not the password do not
/// count, the right one clears the count, "Open "…"" counts for the wallet it opens, and a
/// deleted wallet's count goes with it.
#[tokio::test]
async fn only_wrong_passwords_count_each_for_its_own_wallet() {
    let dir = fresh_dir("unlock-count");
    seal_wallet(&dir, "aaaa0001");
    seal_wallet(&dir, "bbbb0002");
    let mut core = core_at(&dir, "aaaa0001");
    let unlock = |p: &str| Command::Unlock { password: pw(p) };
    for _ in 0..2 {
        core.handle(unlock("no")).await;
    }
    // The right password over a deleted wallet's files: an error, but not the password's.
    core.leftovers = true;
    let (event, _) = core.handle(unlock("pw")).await;
    assert!(matches!(
        answer_of(event),
        ("unlock", Err(CoreError::NotAvailable(_)))
    ));
    assert_eq!(entry(&mut core, "aaaa0001").wrong_tries, 2);
    core.leftovers = false;
    let (event, _) = core.handle(unlock("pw")).await;
    assert_eq!(answer_of(event), ("unlock", Ok(())));
    assert_eq!(entry(&mut core, "aaaa0001").wrong_tries, 0);
    assert_eq!(
        attempts::Attempts::read(&dir, ["aaaa0001"]).failures("aaaa0001"),
        0
    );
    // "Open "Savings"" with a wrong password: Savings counts, the open wallet does not.
    let switch = |p: &str| Command::SwitchWallet {
        id: "bbbb0002".into(),
        password: Some(pw(p)),
    };
    let (event, changed) = core.handle(switch("no")).await;
    assert_eq!(answer_of(event), ("switch", Err(CoreError::WrongPassword)));
    assert!(changed);
    assert_eq!(entry(&mut core, "bbbb0002").wrong_tries, 1);
    assert_eq!(entry(&mut core, "aaaa0001").wrong_tries, 0);
    assert!(core.unlocked.is_some(), "the open wallet stays open");
    // Savings deleted: nothing of its count is left.
    let (event, _) = core
        .handle(Command::DeleteWallet {
            password: Some(pw("pw")),
            wallet: Some("bbbb0002".into()),
        })
        .await;
    assert_eq!(answer_of(event), ("delete-other", Ok(())));
    assert_eq!(core.attempts.failures("bbbb0002"), 0);
    assert!(
        !wallets::wallet_dir(&dir, "bbbb0002")
            .join(attempts::FILE)
            .exists()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn renames_a_wallet_within_24_characters() {
    let dir = fresh_dir("rename");
    seal_wallet(&dir, "aaaa0001");
    let mut core = core_at(&dir, "aaaa0001");
    let rename = |name: &str| Command::RenameWallet {
        id: "aaaa0001".into(),
        name: name.into(),
        color: 4,
    };
    for name in ["   ", &"ω".repeat(25)] {
        let (event, _) = core.handle(rename(name)).await;
        assert!(matches!(
            event,
            Some(Event::Done {
                what: "wallet",
                result: Err(_)
            })
        ));
    }
    let (event, _) = core.handle(rename(" Savings ")).await;
    assert!(matches!(
        event,
        Some(Event::Done {
            what: "wallet",
            result: Ok(())
        })
    ));
    assert_eq!(wallets::load(&dir).wallets[0].name, tr!("Savings"));
    assert_eq!(wallets::load(&dir).wallets[0].color, 4);
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Check the phrase" on a real vault: the password opens it, the words come with the right
/// one among the choices, the right picks pass, a wrong one is named, a check is answered
/// once, and a lock forgets an unanswered one.
#[tokio::test]
async fn checks_the_phrase_without_giving_it_out() {
    let mut core = core_with_vault("check", "pw");
    let words: Vec<&str> = PHRASE.split_whitespace().collect();
    let ask = |password: &str| Command::AskPhraseCheck {
        password: pw(password),
    };
    let (event, _) = core.handle(ask("wrong")).await;
    assert!(matches!(
        event,
        Some(Event::PhraseCheck(Err(CoreError::WrongPassword)))
    ));
    let questions = |event: Option<Event>| match event {
        Some(Event::PhraseCheck(Ok(q))) => q,
        other => panic!("{other:?}"),
    };
    let asked = questions(core.handle(ask("pw")).await.0);
    assert_eq!(asked.len(), 4);
    let right: Vec<String> = asked
        .iter()
        .map(|q| words[q.number - 1].to_owned())
        .collect();
    let (event, _) = core
        .handle(Command::CheckPhraseWords {
            picks: right.clone(),
        })
        .await;
    assert!(matches!(event, Some(Event::PhraseChecked { wrong: None })));
    // Answered once: the same picks again need a new check.
    let (event, _) = core
        .handle(Command::CheckPhraseWords { picks: right })
        .await;
    assert!(matches!(event, Some(Event::PhraseCheck(Err(_)))));

    let asked = questions(core.handle(ask("pw")).await.0);
    let mut picks: Vec<String> = asked
        .iter()
        .map(|q| words[q.number - 1].to_owned())
        .collect();
    picks[1] = asked[1]
        .choices
        .iter()
        .find(|c| **c != picks[1])
        .unwrap()
        .clone();
    let (event, _) = core.handle(Command::CheckPhraseWords { picks }).await;
    assert!(matches!(event, Some(Event::PhraseChecked { wrong: Some(n) }) if n == asked[1].number));

    questions(core.handle(ask("pw")).await.0);
    core.lock().await.unwrap();
    assert!(
        core.phrase_check.is_none(),
        "a lock forgets the right words"
    );
}

#[tokio::test]
async fn changes_the_password_and_keeps_the_phrase() {
    let mut core = core_with_vault("password", "old one");
    assert_eq!(
        core.change_password(&pw("wrong"), &pw("new one!")).await,
        Err(CoreError::WrongPassword)
    );
    assert!(core.change_password(&pw("old one"), &pw("")).await.is_err());
    // The core holds the floor itself, whatever screen sent the command.
    assert_eq!(
        core.change_password(&pw("old one"), &pw("1234567")).await,
        Err(CoreError::NotAvailable(
            tr!("The new password needs at least 8 characters").into()
        ))
    );
    core.change_password(&pw("old one"), &pw("new one!"))
        .await
        .unwrap();
    assert_eq!(
        core.open_vault(&pw("old one")).await.err(),
        Some(CoreError::WrongPassword)
    );
    let (file, phrase) = core.open_vault(&pw("new one!")).await.unwrap();
    assert_eq!(phrase.as_str(), PHRASE);
    assert_eq!(file.meta.birth_height, 1_000);
    // Sealed with the full cost again, not the cheap one it had.
    assert_eq!(file.kdf, KdfParams::default());
}

/// BUG-42, R-27 AC-T1: with "password for every send" on, no spend goes without the
/// password, and none with a wrong one; each is answered as its own failure, and the core
/// went no further than the gate (no lock, no broadcast: the answer is the gate's).
#[tokio::test]
async fn with_the_setting_on_no_spend_goes_without_the_right_password() {
    let mut core = opened("gate", "right");
    assert!(core.password_each_send, "on by default");
    for op in every_spend() {
        let none = Command::Spend {
            op: op.clone(),
            password: None,
        };
        let (event, _) = core.handle(none).await;
        assert_eq!(
            debug(&event),
            debug(&Some(op.refused(CoreError::NotAvailable(
                crate::PASSWORD_NEEDED.into()
            )))),
            "{} without the password",
            op.what()
        );
        let wrong = Command::Spend {
            op: op.clone(),
            password: Some(pw("wrong")),
        };
        let (event, _) = core.handle(wrong).await;
        assert_eq!(
            debug(&event),
            debug(&Some(op.refused(CoreError::WrongPassword))),
            "{} with a wrong password",
            op.what()
        );
    }
    // The right one gets past the gate, to the payment's own checks: not synced.
    let right = Command::Spend {
        op: every_spend().remove(0),
        password: Some(pw("right")),
    };
    assert_eq!(
        debug(&core.handle(right).await.0),
        debug(&Some(Event::Sent(Err(CoreError::NotSynced))))
    );
}

/// R-27 AC-T2: off only with the wallet's password; then a spend needs none, and the
/// choice is kept for the next core of this wallet.
#[tokio::test]
async fn the_setting_goes_off_only_with_the_password() {
    let mut core = opened("each-off", "right");
    let off = |password: Option<&str>| Command::SetPasswordEachSend {
        on: false,
        password: password.map(pw),
    };
    for password in [None, Some("wrong")] {
        assert!(matches!(
            core.handle(off(password)).await.0,
            Some(Event::Done {
                what: "password-each-send",
                result: Err(CoreError::WrongPassword)
            })
        ));
        assert!(core.password_each_send);
    }
    let (event, _) = core.handle(off(Some("right"))).await;
    assert!(matches!(
        event,
        Some(Event::Done {
            what: "password-each-send",
            result: Ok(())
        })
    ));
    assert!(!core.password_each_send);
    assert!(!security::load(&core.paths.root, &core.wallet), "kept");
    let send = Command::Spend {
        op: every_spend().remove(0),
        password: None,
    };
    assert_eq!(
        debug(&core.handle(send).await.0),
        debug(&Some(Event::Sent(Err(CoreError::NotSynced))))
    );
    // On again at once, without the password.
    let on = Command::SetPasswordEachSend {
        on: true,
        password: None,
    };
    core.handle(on).await;
    assert!(core.password_each_send && security::load(&core.paths.root, &core.wallet));
}

#[tokio::test(start_paused = true)]
async fn locks_after_the_idle_time_only_when_open() {
    let mut core = core_with_vault("idle", "pw");
    core.auto_lock = Some(Duration::from_secs(300));
    tokio::time::advance(Duration::from_secs(301)).await;
    assert!(!core.idle_too_long(), "nothing to lock");
    let meta = core.saved.clone().unwrap();
    core.unlocked = Some(Unlocked {
        id: [0; 32],
        meta,
        keys: None,
        seed: None,
    });
    assert!(core.idle_too_long());
    core.last_touch = tokio::time::Instant::now();
    assert!(!core.idle_too_long());
    core.auto_lock = None;
    tokio::time::advance(Duration::from_secs(10_000)).await;
    assert!(!core.idle_too_long(), "\"on close\": never by time");
}
