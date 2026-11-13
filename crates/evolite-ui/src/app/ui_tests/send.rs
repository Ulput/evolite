//! The Send form (branch S: BUG-32, 35, 36, BUG-39 №5–7, 9): what a person sees on it.

use super::*;
use crate::i18n::{Lang, t_in};
use evolite_api::CoreError;

/// What the core says of a typo in an address (`real/convert.rs`, `not_an_address`).
const NOT_AN_ADDRESS: &str =
    "This is not a Dash address: a typo or an extra character — the checksum does not match";

/// Every label whose words are `text`, as drawn.
fn labels<'k>(kit: &'k Kit, text: &'k str) -> Vec<egui_kittest::Node<'k>> {
    kit.query_all(By::new().role(Role::Label).value(text))
        .collect()
}

/// "To" holds `to` and the core has answered for it with `preview`.
fn typed(kit: &mut Kit, to: &str, preview: Result<String, CoreError>) {
    let send = &mut kit.state_mut().send;
    send.recipient = to.into();
    send.previewed = to.into();
    send.preview = Some(preview);
    kit.run_steps(3);
}

/// BUG-35: the core's word for a typo in the address is said in Russian on a Russian screen,
/// with the full stop the form puts after it.
#[test]
fn a_typo_in_the_address_is_said_in_the_language_on_screen() {
    let (_turn, mut kit) = demo_app("send", "ru");
    typed(
        &mut kit,
        "yNotAnAddress123",
        Err(CoreError::NotAvailable(NOT_AN_ADDRESS.into())),
    );
    let russian = format!("{}.", t_in(Lang::Ru, NOT_AN_ADDRESS));
    assert!(russian.starts_with("Это не адрес Dash"), "{russian}");
    assert_eq!(labels(&kit, &russian).len(), 1, "«{russian}» on screen");
    assert!(!has_text(&kit, &format!("{NOT_AN_ADDRESS}.")));
}

/// BUG-36, its steps: while the wallet syncs, «To» = the wallet's own address and «Max». The
/// long «wait for the sync» line stays in the form's column — it ends before «Summary»
/// begins — and the whole «Summary» card stays inside the 1024 px window. Its rows are
/// painted, not in the tree: the card is the heading's left edge plus its inner width.
/// Before the fix the line widened the form and the card's right edge stood at ~1243.
#[test]
fn the_wait_for_sync_line_keeps_the_summary_in_the_window() {
    /// The summary card's inner width (`summary_w - 50` in `screens/send.rs`).
    const SUMMARY_INNER: f32 = 270.0;
    let (_turn, mut kit) = demo_app("send-syncing", "ru");
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 768.0));
    let own = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.core_address.clone())
        .expect("the demo wallet");
    typed(&mut kit, &own, Ok(own.clone()));
    kit.get_by_role_and_label(Role::Button, &crate::i18n::spoken("Max"))
        .click();
    kit.run_steps(3);
    let hint = t_in(Lang::Ru, crate::text::WAIT_FOR_SYNC).into_owned();
    let hints = labels(&kit, &hint);
    assert_eq!(hints.len(), 1, "«{hint}» on screen");
    let line = hints[0].rect();
    let summary = words(&kit, "Итог").rect();
    eprintln!("window {window:?}, the line {line:?}, «Итог» {summary:?}");
    assert!(
        line.right() <= summary.left(),
        "the line ends at {}, «Итог» begins at {}",
        line.right(),
        summary.left()
    );
    let card_right = summary.left() + SUMMARY_INNER;
    assert!(
        window.contains_rect(summary) && card_right <= window.right(),
        "«Итог» {summary:?}: its card ends at {card_right}, past the window {window:?}"
    );
}

/// BUG-39 №9: the wallet's own address is not offered for the address book; it is said to be
/// the wallet's own. Someone else's address still is offered.
#[test]
fn the_wallets_own_address_is_not_offered_for_the_address_book() {
    let (_turn, mut kit) = demo_app("send", "en");
    let own = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.core_address.clone())
        .expect("the demo wallet");
    typed(&mut kit, &own, Ok(own.clone()));
    assert!(!has_text(&kit, "Save to the address book"));
    assert!(has_text(
        &kit,
        "This is an address of this wallet: the money stays yours, only the fee goes."
    ));

    let theirs = "yNBw8AS5PhKdSm6ytP3uAVEsx5PjCiReb4";
    typed(&mut kit, theirs, Ok(theirs.into()));
    assert!(has_text(&kit, "Save to the address book"));
}

/// BUG-68: the wallet's own address pasted with spaces around is its own too — the form asks
/// the core about the trimmed address and says so, not «Save to the address book».
#[test]
fn the_wallets_own_address_with_spaces_around_is_its_own() {
    let (_turn, mut kit) = demo_app("send", "en");
    let own = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.core_address.clone())
        .expect("the demo wallet");
    kit.state_mut().send.recipient = format!("  {own}  ");
    let said = "This is an address of this wallet: the money stays yours, only the fee goes.";
    wait_for(&mut kit, "the own-address line", |kit| has_text(kit, said));
    assert!(!has_text(&kit, "Save to the address book"));
}

/// BUG-67: the sources of «From» stand in one row of equal columns, a Core balance with all
/// eight decimals too and the other two long as well (1024 × 768, the rail), in Russian as the
/// bug was found and in English. Before the fix «Приватный пул» went to a second row.
#[test]
fn the_sources_stay_in_one_row_of_equal_columns() {
    for (lang, names) in [
        ("ru", ["Core", "Кредиты", "Приватный пул", "Адрес Platform"]),
        (
            "en",
            ["Core", "Credits", "Private pool", "Platform address"],
        ),
    ] {
        let (_turn, mut kit) = demo_app("send", lang);
        show_wallet_edited(
            &mut kit,
            5,
            "the wallet edited",
            |w| {
                w.balances.core = 391_724_464;
                // The other two long as well: the widest the row gets.
                w.balances.credits = 123_456_789;
                w.balances.shielded = 98_765_432;
                // The four sources of FEAT-0015; five with mixed coins (FEAT-0030), apart.
                w.balances.mixed = None;
            },
            |_| true,
        );
        let cards: Vec<_> = kit
            .query_all_by_role(Role::RadioButton)
            .filter(|n| {
                n.accesskit_node()
                    .label()
                    .is_some_and(|l| names.contains(&l.as_str()))
            })
            .map(|n| n.rect())
            .collect();
        assert_eq!(cards.len(), 4, "{lang}: {names:?}: {cards:?}");
        let top = cards[0].top();
        assert!(
            even_rows(&cards) && top >= 0.0,
            "{lang}: one row: {cards:?}"
        );
        let width = cards[0].width();
        assert!(
            cards.iter().all(|r| (r.width() - width).abs() < 0.5),
            "{lang}: equal columns: {cards:?}"
        );
    }
}

/// BUG-81: every source of «From» lies whole inside the form's panel — between «From» on the
/// left and «Hide the amounts» on the right — in equal columns, at about 1024 (the rail; 1000
/// is the panel the QA window had) and wider (the menu), in Russian and English. With the
/// bug's balances they stay in one row and the words keep the padding on the right; with
/// thousands of DASH and a Core balance «as of» yesterday they take fewer columns. Before the
/// fix a card widened past its column: at 1000 «Приватный пул» left the row, with long
/// balances the columns were unequal.
#[test]
fn every_source_lies_inside_the_from_panel() {
    // (Core, credits, pool, Core checked a day ago): the bug's wallet, then long balances.
    let sets: [(i64, i64, i64, bool); 2] = [
        (391_724_464, 436_140, 48_885_860, false),
        (1_234_567_890_123, 987_654_321_098, 876_543_210_987, true),
    ];
    for lang in ["ru", "en"] {
        let (_turn, mut kit) = demo_app("send", lang);
        let names =
            ["Core", "Credits", "Private pool", "Platform address"].map(crate::i18n::spoken);
        let from = crate::i18n::spoken("From");
        let hide = crate::i18n::spoken("Hide the amounts");
        for (core, credits, pool, as_of) in sets {
            for (w, h, one_row) in [
                (1000.0, 768.0, true),
                (1024.0, 768.0, true),
                (1024.0, 480.0, true),
                // The rail up to 1280 (R2): three columns fit at 1100 too.
                (1100.0, 768.0, true),
                (1280.0, 800.0, true),
                (1440.0, 900.0, true),
                (1920.0, 1080.0, true),
            ] {
                kit.set_size(egui::vec2(w, h));
                show_wallet_edited(
                    &mut kit,
                    5,
                    "the wallet edited",
                    |wl| {
                        wl.balances.core = core;
                        wl.balances.core_as_of = as_of.then(|| {
                            evolite_api::CoreAsOf::Checked(chrono::Utc::now().timestamp() - 86_400)
                        });
                        wl.balances.credits = credits;
                        wl.balances.shielded = pool;
                        wl.balances.mixed = None;
                    },
                    |_| true,
                );
                let at = format!("{lang} {w}×{h} {core}");
                let left = words(&kit, &from).rect().left();
                let right = button(&kit, &hide).rect().right();
                let cards: Vec<_> = kit
                    .query_all_by_role(Role::RadioButton)
                    .filter(|n| {
                        n.accesskit_node()
                            .label()
                            .is_some_and(|l| names.contains(&l))
                    })
                    .map(|n| n.rect())
                    .collect();
                assert_eq!(cards.len(), 4, "{at}: {names:?}: {cards:?}");
                for r in &cards {
                    assert!(
                        r.left() >= left - 0.5 && r.right() <= right + 0.5,
                        "{at}: a card {r:?} outside the panel {left}..{right}: {cards:?}"
                    );
                }
                let width = cards[0].width();
                assert!(
                    cards.iter().all(|r| (r.width() - width).abs() < 0.5),
                    "{at}: equal columns: {cards:?}"
                );
                if !as_of {
                    let top = cards[0].top();
                    assert!(
                        !one_row || (even_rows(&cards) && top >= 0.0),
                        "{at}: one row: {cards:?}"
                    );
                    // The widest words with 14 px on the left and 16 on the right.
                    let mut widest = 0.0f32;
                    for (text, font) in names
                        .iter()
                        .map(|n| {
                            (
                                n.clone(),
                                crate::theme::sans(13.0, crate::theme::Weight::SemiBold),
                            )
                        })
                        .chain([core, credits, pool].map(|d| {
                            (
                                crate::format::balance(d),
                                crate::theme::mono(11.0, crate::theme::Weight::Regular),
                            )
                        }))
                    {
                        widest = widest.max(kit.ctx.fonts_mut(|f| {
                            f.layout_no_wrap(text, font, egui::Color32::WHITE).size().x
                        }));
                    }
                    assert!(
                        width >= 14.0 + widest + 16.0 - 0.5,
                        "{at}: words {widest} in a card {width} wide"
                    );
                }
            }
        }
    }
}

/// The answer to a name is long in coming: the Send form says the network is slow, not
/// «Checking» for minutes (BUG-87).
#[test]
fn a_name_with_no_answer_for_long_is_said_in_words_on_send() {
    let (_turn, mut kit) = demo_app("send", "en");
    let long_ago = std::time::Instant::now() - std::time::Duration::from_secs(60);
    {
        let send = &mut kit.state_mut().send;
        send.recipient = "@slowname".into();
        send.asked = "@slowname".into();
        send.resolution = None;
        send.asked_at = Some(long_ago);
    }
    kit.run_steps(3);
    assert!(has_words(&kit, "The network is slow to answer"));
}

/// Just asked: still «Checking», no alarm.
#[test]
fn a_name_just_asked_is_only_being_checked_on_send() {
    let (_turn, mut kit) = demo_app("send", "en");
    {
        let send = &mut kit.state_mut().send;
        send.recipient = "@freshname".into();
        send.asked = "@freshname".into();
        send.resolution = None;
        send.asked_at = Some(std::time::Instant::now());
    }
    kit.run_steps(3);
    assert!(has_words(&kit, "Checking the name in DPNS"));
    assert!(!has_words(&kit, "The network is slow to answer"));
}

/// `NotSynced`, sent at once when the lists are not in: the words, not «Checking» (BUG-87).
#[test]
fn a_name_not_checkable_yet_says_why_on_send() {
    let (_turn, mut kit) = demo_app("send", "en");
    {
        let send = &mut kit.state_mut().send;
        send.recipient = "@waiting".into();
        send.asked = "@waiting".into();
        send.resolution = Some(Err(evolite_api::CoreError::NotSynced));
    }
    kit.run_steps(3);
    assert!(has_words(
        &kit,
        "We check the name once the masternode lists load"
    ));
    assert!(!has_words(&kit, "Checking the name in DPNS"));
}

/// BUG-67 in every interface language: the bug's balances at 1024 × 768 (the rail) stay in equal
/// columns, whatever the words of the language, and with the Core balance «as of» yesterday too
/// (its words under the row, not on the card). Before the fix Core's line took the cards to two
/// rows or one column. Since FEAT-0015 there are four sources: one row, or two rows of two
/// (AC-D10: 3 + 1 never).
#[test]
fn the_sources_stay_in_one_row_in_every_language() {
    let mut wrong = Vec::new();
    for lang in crate::i18n::Lang::ALL {
        let code = lang.code();
        let (turn, mut kit) = demo_app("send", code);
        show_wallet_edited(
            &mut kit,
            5,
            "the wallet edited",
            |w| {
                w.balances.core = 391_724_464;
                w.balances.credits = 123_456_789;
                w.balances.shielded = 98_765_432;
                w.balances.core_as_of = Some(evolite_api::CoreAsOf::Checked(
                    chrono::Utc::now().timestamp() - 86_400,
                ));
                w.balances.mixed = None;
            },
            |_| true,
        );
        let names =
            ["Core", "Credits", "Private pool", "Platform address"].map(crate::i18n::spoken);
        let cards: Vec<_> = kit
            .query_all_by_role(Role::RadioButton)
            .filter(|n| {
                n.accesskit_node()
                    .label()
                    .is_some_and(|l| names.contains(&l))
            })
            .map(|n| n.rect())
            .collect();
        // Rows by their top: all of one size (4, or 2 + 2), and every column one width.
        let mut rows: Vec<(f32, usize)> = Vec::new();
        for r in &cards {
            match rows
                .iter_mut()
                .find(|(top, _)| (*top - r.top()).abs() < 0.5)
            {
                Some(row) => row.1 += 1,
                None => rows.push((r.top(), 1)),
            }
        }
        let row = cards.len() == 4
            && rows.iter().all(|(_, n)| *n == rows[0].1)
            && cards
                .iter()
                .all(|r| (r.width() - cards[0].width()).abs() < 0.5);
        if !row {
            wrong.push(format!("{code}: {names:?}: {cards:?}"));
        }
        if lang.code() == "en" && !has_words(&kit, "Core · as of") {
            wrong.push("en: no caption under the row".into());
        }
        drop(kit);
        drop(turn);
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The «From» cards come in rows of one size: all four in one, or two of two (FEAT-0015
/// AC-D10: three and one never).
fn even_rows(cards: &[egui::Rect]) -> bool {
    let mut rows: Vec<(f32, usize)> = Vec::new();
    for r in cards {
        match rows
            .iter_mut()
            .find(|(top, _)| (*top - r.top()).abs() < 0.5)
        {
            Some(row) => row.1 += 1,
            None => rows.push((r.top(), 1)),
        }
    }
    rows.iter().all(|(_, n)| *n == rows[0].1)
}

/// Every piece of text painted with `text` (as the interface draws it, [`crate::i18n::t`]), by
/// where it is drawn.
fn painted_rects(kit: &Kit, text: &str) -> Vec<egui::Rect> {
    fn walk(shape: &egui::Shape, text: &str, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Text(t) if t.galley.job.text == text => out.push(t.visual_bounding_rect()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &kit.output().shapes {
        walk(&c.shape, text, &mut out);
    }
    out
}

/// BUG-126: in Arabic the labels of «Check the payment» stay inside the dialog. «الرسوم، بحد
/// أقصى» («Fee, at most») stood past its right edge over the form behind: the address in «To»
/// behind the dialog is kept left to right, and a label that fell inside its place was moved with
/// it instead of being mirrored. The dialog stands in the middle of the window and the form at its
/// top, so the window's height brings each label of the dialog over the field in turn.
#[test]
fn the_last_looks_labels_stay_inside_the_dialog_in_arabic() {
    // From chosen coins (FEAT-0013 P1), as the bug was seen.
    let (_turn, mut kit) = demo_app("send-coins-confirm", "ar");
    let sign = crate::i18n::spoken("Sign and send");
    wait_for(&mut kit, "«Sign and send»", |kit| {
        kit.query_by_label(&sign).is_some()
    });
    let mut wrong = Vec::new();
    for h in (640..=860).step_by(4) {
        kit.set_size(egui::vec2(1024.0, h as f32));
        kit.run_steps(3);
        let Some(dialog) = kit
            .ctx
            .memory(|m| m.area_rect(egui::Id::new("send-confirm")))
        else {
            wrong.push(format!("{h}: no dialog"));
            continue;
        };
        // Words only the last look has (the form behind says «Network fee», «Amount», «From»).
        for label in ["Fee, at most", "In total", "Change, to your address"] {
            let shown = crate::i18n::t(label).into_owned();
            for r in painted_rects(&kit, &shown) {
                if !dialog.expand(0.5).contains_rect(r) {
                    wrong.push(format!("{h}: «{label}» at {r:?}, the dialog {dialog:?}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// BUG-142: «Max» from Core leaves out a coin no payment can sign for yet (a contact's coin the
/// store filed in account 0): with it, the payment failed to build.
#[test]
fn max_from_core_leaves_out_a_coin_that_cannot_be_signed_for() {
    let (_turn, mut kit) = demo_app("send", "en");
    show_wallet_edited(
        &mut kit,
        4,
        "a coin not to sign for",
        |w| w.fees.core_unsignable = 100_000,
        |_| true,
    );
    let (core, fee) = {
        let w = kit
            .state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .expect("the demo wallet");
        (w.balances.core, w.fees.core_payment)
    };
    kit.state_mut().send.pick(evolite_api::Source::Core);
    kit.state_mut().send.max = true;
    kit.run_steps(4);
    assert_eq!(
        crate::format::parse_amount(&kit.state().send.amount),
        Some(core - fee - 100_000)
    );
}

/// BUG-142 AC-142-1…5: from Core, a part of the balance no payment can sign for yet is said
/// under the amount, hidden with «Hide amounts»; an amount within the balance but past it is
/// refused in its words; other sources say nothing; it goes when the part is 0.
#[test]
fn a_part_of_core_that_cannot_be_signed_for_is_said_under_the_amount() {
    let hint = "of the balance can't be spent yet: the wallet is still taking in these coins";
    let (_turn, mut kit) = demo_app("send", "en");
    let unsignable = |n: i64| move |w: &mut evolite_api::Wallet| w.fees.core_unsignable = n;
    show_wallet_edited(
        &mut kit,
        4,
        "a part not to sign for",
        unsignable(100_000),
        |_| true,
    );
    kit.state_mut().send.pick(evolite_api::Source::Core);
    kit.state_mut().send.amount = "0.01".into();
    kit.run_steps(4);
    let part = crate::format::amount(100_000);
    assert!(has_words(&kit, &format!("{part} {hint}")), "AC-142-1");
    assert!(has_words(&kit, "This normally clears after the next sync."));
    // AC-142-4: hidden with the amounts.
    kit.state_mut().settings.turn_on("hide-amounts");
    kit.run_steps(4);
    assert!(
        has_words(&kit, &format!("•••• DASH {hint}")) && !has_words(&kit, &part),
        "AC-142-4"
    );
    kit.state_mut().settings.set("hide-amounts", false);
    // AC-142-2: within the balance, past what can be spent.
    let (core, fee) = {
        let w = kit
            .state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .expect("the demo wallet");
        (w.balances.core, w.fees.core_payment)
    };
    kit.state_mut().send.amount = crate::format::input_text(core - fee);
    kit.run_steps(4);
    assert!(
        has_words(
            &kit,
            &format!("Not enough to spend: {part} of the balance can't be spent yet.")
        ),
        "AC-142-2"
    );
    // The button's reason, in `danger` (the designer).
    let danger = crate::theme::DARK.danger;
    let mut colors = Vec::new();
    fn walk(s: &egui::Shape, out: &mut Vec<egui::Color32>) {
        match s {
            egui::Shape::Text(t) if t.galley.job.text.starts_with("Not enough to spend") => {
                out.extend(
                    t.galley
                        .job
                        .sections
                        .iter()
                        .map(|section| section.format.color),
                );
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    for c in &kit.output().shapes {
        walk(&c.shape, &mut colors);
    }
    assert!(
        !colors.is_empty() && colors.iter().all(|c| *c == danger),
        "in danger: {colors:?}"
    );
    // AC-142-5: not from the credits.
    kit.state_mut().send.amount = "0.01".into();
    kit.state_mut().send.pick(evolite_api::Source::Credits);
    kit.run_steps(4);
    assert!(!has_words(&kit, hint), "AC-142-5");
    // AC-142-3: the part taken in, the line goes.
    kit.state_mut().send.pick(evolite_api::Source::Core);
    show_wallet_edited(&mut kit, 4, "taken in", unsignable(0), |_| true);
    kit.run_steps(4);
    assert!(!has_words(&kit, hint), "AC-142-3");
}
