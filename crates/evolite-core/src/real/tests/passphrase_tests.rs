//! FEAT-0041: the BIP-39 passphrase through the core — made with a new wallet (only with the
//! advanced features on), brought by a restored phrase, kept in the vault with the phrase, shown
//! with it, asked by the phrase check; one phrase with two passphrases is two wallets.

use super::*;

const CHEAP: KdfParams = KdfParams {
    memory_kib: 256,
    iterations: 1,
    parallelism: 1,
};

fn id_of(phrase: &str, passphrase: &str) -> String {
    hex(&chain::wallet_id_of(&evolite_chain::seed_from(phrase, passphrase).unwrap()).unwrap())
}

/// Seals PHRASE with `passphrase` as wallet `id` of the data folder `dir`, password «pw».
fn seal_secret(dir: &std::path::Path, id: &str, passphrase: &str) {
    let meta = VaultMeta {
        network: network().into(),
        wallet_id: id_of(PHRASE, passphrase),
        birth_height: 1_000,
        word_count: 12,
        created_at: 1_793_000_000,
    };
    let file = vault::seal(&vault::Secret::new(PHRASE, passphrase), "pw", meta, CHEAP).unwrap();
    vault::save(&wallets::wallet_dir(dir, id).join("wallet.json"), &file).unwrap();
}

fn vault_of(dir: &std::path::Path, id: &str) -> vault::VaultFile {
    vault::load(&wallets::wallet_dir(dir, id).join("wallet.json"))
        .unwrap()
        .unwrap()
}

fn phrase_with(passphrase: &str) -> Phrase {
    Phrase::with_passphrase(
        PHRASE.split_whitespace().map(str::to_owned).collect(),
        passphrase::Passphrase::new(passphrase.into()),
    )
}

fn refusal(event: Option<Event>) -> String {
    match event {
        Some(Event::PhraseAccepted(Err(CoreError::NotAvailable(why)))) => why,
        other => panic!("{other:?}"),
    }
}

/// R-2, FEAT-0040 «hidden is off»: a new wallet takes a passphrase only with the advanced
/// features on; then the vault holds it with the phrase, the wallet's id is the seed's with it,
/// and the unlock opens that wallet again.
#[tokio::test]
async fn a_new_wallet_takes_its_passphrase_only_with_the_advanced_features() {
    let dir = fresh_dir("bip39-create");
    let mut core = core_at(&dir, "cccc0003");
    core.pending = Some((vault::Secret::new(PHRASE, ""), false));
    core.birth_hint = Some(777);
    let trezor = passphrase::Passphrase::new("TREZOR".into());
    assert_eq!(
        core.create(&pw("Second-wallet-7"), Some(&trezor)).await,
        Err(CoreError::NotAvailable(evolite_common::ADVANCED_OFF.into()))
    );
    assert!(
        core.saved.is_none() && core.pending.is_some(),
        "nothing written"
    );
    core.handle(Command::AdvancedFeatures { on: true }).await;
    assert_eq!(
        core.create(&pw("Second-wallet-7"), Some(&trezor)).await,
        Ok(())
    );
    let file = vault_of(&dir, "cccc0003");
    assert_eq!(file.format, "evolite/vault/v2");
    assert_eq!(file.meta.wallet_id, id_of(PHRASE, "TREZOR"));
    assert_ne!(file.meta.wallet_id, id_of(PHRASE, ""));
    let secret = vault::open(&file, "Second-wallet-7").unwrap();
    assert_eq!(secret, vault::Secret::new(PHRASE, "TREZOR"));
    assert!(core.unlocked.as_ref().is_some_and(|u| u.passphrase));

    core.lock().await.unwrap();
    assert_eq!(core.unlock(&pw("Second-wallet-7")).await, Ok(()));
    let open = core.unlocked.as_ref().unwrap();
    assert_eq!(hex(&open.id), id_of(PHRASE, "TREZOR"));
    assert!(open.passphrase);
    let seed = evolite_chain::seed_from(PHRASE, "TREZOR").unwrap();
    assert_eq!(seed_bytes(open.seed.as_ref().unwrap()), &*seed);
    let _ = std::fs::remove_dir_all(&dir);
}

/// FEAT-0041 r1 (with BUG-252): «on» is taken (а) with no wallet on this computer — the first
/// onboarding, nothing is locked; (б) not while adding a wallet from a locked one's choice;
/// (в) while adding from an open wallet. Where it is not taken, a new wallet's passphrase is
/// refused, and the snapshot says so (`advanced_allowed`) — the app then offers none.
#[tokio::test]
async fn the_switch_is_taken_with_no_wallet_or_from_an_open_one() {
    let trezor = passphrase::Passphrase::new("TREZOR".into());
    let make = async |core: &mut Real| {
        core.pending = Some((vault::Secret::new(PHRASE, ""), false));
        core.birth_hint = Some(777);
        core.handle(Command::AdvancedFeatures { on: true }).await;
        let allowed = core.snapshot().await.advanced_allowed;
        let made = core.create(&pw("Second-wallet-7"), Some(&trezor)).await;
        (core.advanced_features, allowed, made)
    };
    // (а) The first onboarding.
    let dir = fresh_dir("bip39-first");
    let mut first = core_at(&dir, "aaaa0001");
    assert_eq!(make(&mut first).await, (true, true, Ok(())));
    let _ = std::fs::remove_dir_all(&dir);

    // (б) A wallet here, locked; «Add a wallet» from its choice.
    let dir = fresh_dir("bip39-locked-add");
    seal_secret(&dir, "aaaa0001", "");
    let mut locked = core_at(&dir, "aaaa0001");
    locked.handle(Command::AddWallet).await;
    assert!(!locked.adding_open);
    let mut adding = core_at(&dir, locked.switch_to.as_deref().unwrap());
    adding.adding_from = locked.adding_from.take();
    adding.adding_open = locked.adding_open;
    let (on, allowed, made) = make(&mut adding).await;
    assert!(!on && !allowed);
    assert_eq!(
        made,
        Err(CoreError::NotAvailable(evolite_common::ADVANCED_OFF.into()))
    );
    let _ = std::fs::remove_dir_all(&dir);

    // (в) The wallet open; «Add a wallet».
    let dir = fresh_dir("bip39-open-add");
    seal_secret(&dir, "aaaa0001", "");
    let mut open = core_at(&dir, "aaaa0001");
    assert_eq!(open.unlock(&pw("pw")).await, Ok(()));
    open.handle(Command::AddWallet).await;
    assert!(open.adding_open);
    let mut adding = core_at(&dir, open.switch_to.as_deref().unwrap());
    adding.adding_from = open.adding_from.take();
    adding.adding_open = open.adding_open;
    assert_eq!(make(&mut adding).await, (true, true, Ok(())));
    let _ = std::fs::remove_dir_all(&dir);
}

/// R-10, AC-A15: over 256 bytes after NFKD is refused before anything is written — never cut;
/// 256 is taken.
#[tokio::test]
async fn too_long_a_passphrase_is_refused_before_anything_is_written() {
    let dir = fresh_dir("bip39-long");
    let mut core = core_at(&dir, "cccc0003");
    core.pending = Some((vault::Secret::new(PHRASE, ""), false));
    core.birth_hint = Some(777);
    core.advanced_features = true;
    let long = passphrase::Passphrase::new("a".repeat(257));
    assert_eq!(
        core.create(&pw("Second-wallet-7"), Some(&long)).await,
        Err(CoreError::NotAvailable(
            evolite_common::PASSPHRASE_TOO_LONG.into()
        ))
    );
    assert!(
        vault::load(&wallets::wallet_dir(&dir, "cccc0003").join("wallet.json"))
            .unwrap()
            .is_none()
    );
    let fits = passphrase::Passphrase::new("a".repeat(256));
    assert_eq!(
        core.create(&pw("Second-wallet-7"), Some(&fits)).await,
        Ok(())
    );
    let secret = vault::open(&vault_of(&dir, "cccc0003"), "Second-wallet-7").unwrap();
    assert_eq!(secret.passphrase.len(), 256, "kept whole");
    let _ = std::fs::remove_dir_all(&dir);
}

/// R-1, R-12, AC-A16: a restored phrase brings its passphrase, the switch on or off. The same
/// phrase as a wallet here with another passphrase is another wallet and is made; the same pair
/// again is «already here». The duplicate check is by the seed (architect, condition 4).
#[tokio::test]
async fn one_phrase_with_two_passphrases_is_two_wallets() {
    let dir = fresh_dir("bip39-two");
    seal_secret(&dir, "aaaa0001", "");
    let mut core = core_at(&dir, "bbbb0002");
    assert!(!core.advanced_features, "restoring needs no switch");
    let use_phrase = |passphrase: &str| Command::UsePhrase {
        phrase: phrase_with(passphrase),
        dashpay: false,
    };
    let why = refusal(core.handle(use_phrase("")).await.0);
    assert!(why.contains("already on this computer"), "{why}");
    let long = "ж".repeat(129);
    assert_eq!(
        refusal(core.handle(use_phrase(&long)).await.0),
        evolite_common::PASSPHRASE_TOO_LONG
    );
    assert!(matches!(
        core.handle(use_phrase("TREZOR")).await.0,
        Some(Event::PhraseAccepted(Ok(None)))
    ));
    // A restored phrase's passphrase came with it: another one with `CreateWallet` is refused.
    let other = passphrase::Passphrase::new("other".into());
    assert!(matches!(
        core.create(&pw("Second-wallet-7"), Some(&other)).await,
        Err(CoreError::NotAvailable(_))
    ));
    assert_eq!(core.create(&pw("Second-wallet-7"), None).await, Ok(()));
    assert_eq!(
        vault_of(&dir, "bbbb0002").meta.wallet_id,
        id_of(PHRASE, "TREZOR")
    );

    let mut third = core_at(&dir, "cccc0003");
    let why = refusal(third.handle(use_phrase("TREZOR")).await.0);
    assert!(why.contains("already on this computer"), "{why}");
    // R-8: a space is part of it, so this one is a third wallet.
    assert!(matches!(
        third.handle(use_phrase(" TREZOR")).await.0,
        Some(Event::PhraseAccepted(Ok(None)))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// R-3, R-6, AC-A5, AC-A12: «Show the phrase» gives the passphrase with the words; «Check the
/// phrase» asks it of a wallet with one, compares it as BIP-39 salts it (NFKD), and asks nothing
/// of a wallet without one.
#[tokio::test]
async fn the_passphrase_is_shown_and_checked_with_the_phrase() {
    let dir = fresh_dir("bip39-show");
    seal_secret(&dir, "aaaa0001", "caf\u{e9}");
    seal_secret(&dir, "bbbb0002", "");
    let words: Vec<&str> = PHRASE.split_whitespace().collect();
    for (id, saved) in [("aaaa0001", "caf\u{e9}"), ("bbbb0002", "")] {
        let mut core = core_at(&dir, id);
        assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
        let (event, _) = core
            .handle(Command::RevealPhrase {
                password: pw("pw"),
                wallet: None,
            })
            .await;
        match event {
            Some(Event::PhraseRevealed(Ok(phrase))) => {
                assert_eq!(phrase.words().join(" "), PHRASE);
                assert_eq!(phrase.passphrase().expose(), saved);
            }
            other => panic!("{other:?}"),
        }
        let mut check = async |typed: Option<&str>| {
            let (event, _) = core
                .handle(Command::AskPhraseCheck { password: pw("pw") })
                .await;
            let quiz = match event {
                Some(Event::PhraseCheck(Ok(quiz))) => quiz,
                other => panic!("{other:?}"),
            };
            assert_eq!(quiz.passphrase, !saved.is_empty());
            let picks = quiz
                .questions
                .iter()
                .map(|q| words[q.number - 1].to_owned())
                .collect();
            let (event, _) = core
                .handle(Command::CheckPhraseWords {
                    picks,
                    passphrase: typed.map(|t| passphrase::Passphrase::new(t.into())),
                })
                .await;
            match event {
                Some(Event::PhraseChecked {
                    wrong: None,
                    passphrase_wrong,
                }) => passphrase_wrong,
                other => panic!("{other:?}"),
            }
        };
        if saved.is_empty() {
            assert!(!check(None).await);
            assert!(!check(Some("anything")).await, "none asked, none compared");
        } else {
            assert!(!check(Some("caf\u{e9}")).await);
            assert!(
                !check(Some("cafe\u{301}")).await,
                "NFKD: the same passphrase"
            );
            assert!(check(Some("Caf\u{e9}")).await);
            assert!(check(Some("")).await);
            assert!(check(None).await);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// R-15 (architect, condition 3): a new password seals the phrase and its passphrase again, as
/// v2; the passphrase stays.
#[tokio::test]
async fn a_new_password_keeps_the_passphrase() {
    let dir = fresh_dir("bip39-password");
    seal_secret(&dir, "aaaa0001", "TREZOR");
    let mut core = core_at(&dir, "aaaa0001");
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    let (event, _) = core
        .handle(Command::ChangePassword {
            old: pw("pw"),
            new: pw("New-password-7"),
        })
        .await;
    assert!(matches!(event, Some(Event::Done { result: Ok(()), .. })));
    let file = vault_of(&dir, "aaaa0001");
    assert_eq!(file.format, "evolite/vault/v2");
    assert_eq!(
        vault::open(&file, "New-password-7").unwrap(),
        vault::Secret::new(PHRASE, "TREZOR")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// AC-A13 (R-15): nothing outside the encryption tells a wallet with a passphrase — its vault
/// file has the same fields and the same length as one without, its record in `wallets.json`
/// the same fields; only the id differs.
#[tokio::test]
async fn nothing_outside_tells_a_passphrase() {
    let dir = fresh_dir("bip39-plain");
    seal_secret(&dir, "aaaa0001", "");
    seal_secret(&dir, "bbbb0002", &"a".repeat(256));
    let (plain, secret) = (vault_of(&dir, "aaaa0001"), vault_of(&dir, "bbbb0002"));
    assert_eq!(plain.format, secret.format);
    assert_eq!(plain.ciphertext.len(), secret.ciphertext.len());
    let fields = |file: &vault::VaultFile| {
        let mut value = serde_json::to_value(file).unwrap();
        value["meta"]["walletId"] = serde_json::Value::Null;
        value["salt"] = serde_json::Value::Null;
        value["nonce"] = serde_json::Value::Null;
        value["ciphertext"] = serde_json::Value::Null;
        value
    };
    assert_eq!(fields(&plain), fields(&secret));
    let core = core_at(&dir, "aaaa0001");
    let records: Vec<_> = core
        .index
        .wallets
        .iter()
        .map(|r| {
            let mut value = serde_json::to_value(r).unwrap();
            // The times the two wallets were made (or opened) may fall a second apart; they
            // tell nothing of a passphrase (batch 73: 1795625853 against 1795625854).
            for key in ["id", "name", "color", "created_at", "last_opened"] {
                value[key] = serde_json::Value::Null;
            }
            value
        })
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0], records[1]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// R-14, AC-T-P5: no `Debug` of what carries a passphrase shows it — not a command, not an
/// event, not the vault's secret, not whether there is one.
#[test]
fn no_debug_shows_a_passphrase() {
    let commands = [
        format!(
            "{:?}",
            Command::UsePhrase {
                phrase: phrase_with("TREZOR"),
                dashpay: false
            }
        ),
        format!(
            "{:?}",
            Command::CreateWallet {
                password: pw("pw"),
                passphrase: Some(passphrase::Passphrase::new("TREZOR".into())),
            }
        ),
        format!(
            "{:?}",
            Command::CheckPhraseWords {
                picks: vec![],
                passphrase: Some(passphrase::Passphrase::new("TREZOR".into())),
            }
        ),
        format!("{:?}", Event::PhraseRevealed(Ok(phrase_with("TREZOR")))),
        format!("{:?}", vault::Secret::new(PHRASE, "TREZOR")),
    ];
    for text in &commands {
        assert!(!text.contains("TREZOR"), "{text}");
    }
    assert_eq!(
        format!("{:?}", phrase_with("TREZOR")),
        format!("{:?}", phrase_with(""))
    );
}
