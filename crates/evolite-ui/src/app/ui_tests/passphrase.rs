//! FEAT-0041 · the BIP-39 passphrase on the demo core: «Restore» offers it always, «Create» only
//! with the advanced features on (AC-A8, analyst-158); the field's notes (AC-A9, AC-A15); «Show
//! the phrase» shows it with the words and keeps it out of the tree (AC-A5); «Check the phrase»
//! asks it only of a wallet with one (AC-A12); the empty restored wallet's banner (AC-A6).

use super::*;
use evolite_api::passphrase::Passphrase;
use evolite_api::{Phrase, PhraseQuestion, PhraseQuiz, SyncStage, WalletOrigin};

/// Clicks the `nth` password field (from the top) and types `text` into it.
fn type_password(kit: &mut Kit, nth: usize, text: &str) {
    kit.query_all_by_role(Role::PasswordInput)
        .nth(nth)
        .expect("the field")
        .click();
    kit.run_steps(2);
    kit.query_all_by_role(Role::PasswordInput)
        .nth(nth)
        .expect("the field")
        .type_text(text);
    kit.run_steps(3);
}

fn open_block(kit: &mut Kit, title: &str) {
    wait_for(kit, title, |kit| {
        kit.query_by_role_and_label(Role::Group, title).is_some() || has_text(kit, title)
    });
    click_words_of(kit, title);
    kit.run_steps(3);
    assert!(
        has_text(kit, "BIP-39 passphrase (optional)"),
        "{title} opens"
    );
}

/// AC-A8, UC-6: «Restore» has «Have a BIP-39 passphrase?» with the switch off; open, the field,
/// the warning R-5 and the line about DashPay Android; the notes under it; 257 letters refused.
#[test]
fn restore_offers_the_passphrase_with_the_switch_off() {
    let (_turn, mut kit) = demo_app("restore", "en");
    assert!(!kit.state().settings.is_on("advanced"));
    assert!(!has_text(&kit, "BIP-39 passphrase (optional)"), "folded");
    open_block(&mut kit, "Have a BIP-39 passphrase?");
    assert!(has_words(
        &kit,
        "Without it the same phrase opens another, empty wallet"
    ));
    assert!(has_words(
        &kit,
        "DashPay Android does not support a BIP-39 passphrase"
    ));
    assert!(
        !has_text(&kit, "Repeat the BIP-39 passphrase"),
        "no repeat on «Restore»"
    );
    type_password(&mut kit, 0, " пароль");
    assert!(has_words(&kit, "spaces at the ends or double spaces"));
    assert!(has_words(&kit, "letters outside the Latin alphabet"));
    type_password(&mut kit, 0, &"a".repeat(257));
    assert!(has_text(&kit, "The BIP-39 passphrase is too long"));
}

/// AC-A8, UC-3: «Create» offers nothing with the switch off, nor with it on while the core does not
/// take it (r1 (б)); on and taken, «Advanced» opens the passphrase and its repeat, and a repeat
/// that differs is said.
#[test]
fn create_offers_the_passphrase_only_with_the_switch() {
    let (_turn, mut kit) = demo_app("create", "en");
    wait_for(&mut kit, "the phrase", |kit| {
        kit.state()
            .onboarding
            .as_ref()
            .is_some_and(|o| o.has_phrase())
    });
    kit.run_steps(3);
    assert!(
        kit.query_by_label("Advanced").is_none(),
        "hidden with the switch off"
    );
    kit.state_mut().settings.set("advanced", true);
    // FEAT-0041 r1 (б): the switch on, but the core does not take it (adding from a locked
    // wallet's choice): nothing is offered — the passphrase would be refused.
    let refused = |app: &mut App| {
        if let Some(s) = app.snapshot.as_mut() {
            s.advanced_allowed = false;
            s.advanced_features = false;
        }
    };
    draw_edited(&mut kit, 4, refused);
    assert!(
        kit.query_by_label("Advanced").is_none(),
        "the core would refuse it"
    );
    // The core takes it again (the demo sends no snapshot by itself): told, it has it on.
    show_edited(
        &mut kit,
        2,
        "the core has it on",
        |app| {
            if let Some(s) = app.snapshot.as_mut() {
                s.advanced_allowed = true;
            }
        },
        |kit| {
            kit.state()
                .snapshot
                .as_ref()
                .is_some_and(|s| s.advanced_features)
        },
    );
    open_block(&mut kit, "Advanced");
    assert!(has_text(&kit, "Repeat the BIP-39 passphrase"));
    let typed = |kit: &mut Kit, repeat: &str| {
        if let Some(o) = kit.state_mut().onboarding.as_mut() {
            o.type_bip39("TREZOR", repeat);
        }
        kit.run_steps(3);
    };
    typed(&mut kit, "TREZ");
    assert!(has_text(&kit, "The passphrases do not match"));
    typed(&mut kit, "TREZOR");
    assert!(!has_text(&kit, "The passphrases do not match"));
}

/// AC-A5, P5: «Show the phrase» of a wallet with a passphrase shows it under the words, painted
/// — the tree has only its name; a wallet without one has no such line (R-11).
#[test]
fn the_passphrase_is_shown_under_the_words() {
    let (_turn, mut kit) = demo_app("phrase", "en");
    let words: Vec<String> = ["abandon"; 11]
        .iter()
        .map(|w| w.to_string())
        .chain(["about".to_string()])
        .collect();
    kit.state_mut()
        .settings
        .on_revealed(Ok(Phrase::with_passphrase(
            words.clone(),
            Passphrase::new("Kestrel-quartz".into()),
        )));
    wait_for(&mut kit, "the passphrase", |kit| {
        painted_has(kit, "Kestrel-quartz")
    });
    assert!(painted_has(&kit, "BIP-39 PASSPHRASE"));
    assert!(has_words(&kit, "this phrase opens another, empty wallet"));
    assert!(
        kit.query_by_label("BIP-39 passphrase, shown on the screen")
            .is_some()
    );
    let in_tree = kit.query_all_by(|_| true).any(|n| {
        let node = n.accesskit_node();
        [node.label(), node.value(), node.description()]
            .into_iter()
            .flatten()
            .any(|t| t.contains("Kestrel-quartz"))
    });
    assert!(!in_tree, "the passphrase is not in the tree");

    kit.state_mut().settings.on_revealed(Ok(Phrase::new(words)));
    kit.run_steps(3);
    assert!(!painted_has(&kit, "BIP-39 PASSPHRASE"), "none, no line");
}

/// AC-A12, UC-5: «Check the phrase» asks the passphrase of a wallet with one — and its wrong
/// answer is said — and nothing of a wallet without one.
#[test]
fn the_check_asks_the_passphrase_only_of_a_wallet_with_one() {
    let (_turn, mut kit) = demo_app("check-phrase", "en");
    wait_for(&mut kit, "the questions", |kit| {
        kit.state()
            .settings
            .check
            .as_ref()
            .is_some_and(|c| c.questions.is_some())
    });
    let quiz = |passphrase: bool| {
        Ok(PhraseQuiz {
            questions: vec![PhraseQuestion {
                number: 2,
                choices: vec!["canvas".into(), "cactus".into(), "camera".into()],
            }],
            passphrase,
        })
    };
    kit.state_mut().settings.on_phrase_questions(&quiz(false));
    kit.run_steps(3);
    assert!(!has_text(&kit, "BIP-39 passphrase"), "none asked");
    kit.state_mut().settings.on_phrase_questions(&quiz(true));
    kit.run_steps(3);
    assert!(has_text(&kit, "BIP-39 passphrase"));
    kit.state_mut()
        .settings
        .on_phrase_checked(None, true, 1_793_404_800);
    kit.run_steps(3);
    assert!(has_text(
        &kit,
        "The BIP-39 passphrase does not match the saved one"
    ));
    assert!(
        kit.state().settings.phrase_checked.is_none(),
        "not «checked»"
    );
}

/// AC-A6, R-13, analyst-160: a restored wallet the whole sync found empty says to check the
/// passphrase, or the phrase and a passphrase it may have had; «×» closes it for good; a new
/// wallet has none.
#[test]
fn an_empty_restored_wallet_says_what_to_check() {
    let (_turn, mut kit) = demo_app("overview", "en");
    const WITH: &str =
        "The wallet is empty. If you expected money here, check the BIP-39 passphrase.";
    const WITHOUT: &str = "The wallet is empty. Check the phrase. If the wallet had a BIP-39 passphrase, it is needed here too.";
    // The demo wallet has payments: its first frames closed the banner for good.
    let empty = |origin: WalletOrigin, passphrase: bool| {
        move |app: &mut App| {
            app.settings.empty_hint_closed = false;
            let Some(s) = app.snapshot.as_mut() else {
                return;
            };
            s.sync.stage = SyncStage::Done;
            for w in &mut s.wallets {
                if w.open {
                    w.origin = origin;
                }
            }
            if let Some(w) = s.wallet.as_mut() {
                w.activity.clear();
                w.identity = None;
                w.identities.clear();
                w.balances.core = 0;
                w.balances.credits = 0;
                w.balances.all_credits = 0;
                w.balances.platform_address = 0;
                w.balances.shielded = 0;
                w.bip39_passphrase = passphrase;
            }
        }
    };
    show_edited(
        &mut kit,
        2,
        "with a passphrase",
        empty(WalletOrigin::Restored, true),
        |kit| has_text(kit, WITH),
    );
    show_edited(
        &mut kit,
        2,
        "without",
        empty(WalletOrigin::Restored, false),
        |kit| has_text(kit, WITHOUT),
    );
    show_edited(
        &mut kit,
        2,
        "a new wallet",
        empty(WalletOrigin::Created, false),
        |kit| !has_text(kit, WITHOUT) && !has_text(kit, WITH),
    );
    show_edited(
        &mut kit,
        2,
        "again",
        empty(WalletOrigin::Restored, false),
        |kit| has_text(kit, WITHOUT),
    );
    let kept = |app: &mut App| {
        let closed = app.settings.empty_hint_closed;
        empty(WalletOrigin::Restored, false)(app);
        app.settings.empty_hint_closed = closed;
    };
    draw_edited(&mut kit, 2, kept);
    kit.get_by_label("Close").click();
    draw_edited(&mut kit, 3, kept);
    assert!(kit.state().settings.empty_hint_closed, "closed for good");
    assert!(!has_text(&kit, WITHOUT));
}
