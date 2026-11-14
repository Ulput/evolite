//! BUG-161: in English «0,002» in «Top up the identity» was read as 2 DASH — the comma passed as a
//! thousands mark — and only «More than there is on Core» stopped it. One comma or point is the
//! decimal mark in every field; two marks are refused with a line under the field.

use super::*;

/// The amount field of a form, by its preset: where a test writes what was typed.
fn field_of<'a>(kit: &'a mut Kit, preset: &str) -> &'a mut String {
    let app = kit.state_mut();
    match preset {
        "top-up" => {
            &mut app
                .identity
                .top_up
                .as_mut()
                .expect("the top-up form")
                .amount
        }
        "create-identity" => &mut app.create_identity.amount,
        "shield" => &mut app.pool.amount,
        "unshield" => &mut app.pool.unshield_amount,
        "fund-address" => &mut app.fund_address.form.as_mut().expect("the form").amount,
        other => panic!("no amount field for {other}"),
    }
}

/// QA's steps, in English: «0,002» tops up 0.0020 DASH, not 2.
#[test]
fn a_top_up_of_0_comma_002_is_0_002_dash_in_english() {
    let (_turn, mut kit) = demo_app("top-up", "en");
    wait_for(&mut kit, "the top-up form", |kit| {
        kit.state().identity.top_up.is_some()
    });
    *field_of(&mut kit, "top-up") = "0,002".into();
    kit.run_steps(3);
    wait_for(&mut kit, "the button with the amount", |kit| {
        has_text(
            kit,
            &format!("Top up by {}", crate::format::amount(200_000)),
        )
    });
    let button = format!("Top up by {}", crate::format::amount(200_000));
    assert!(has_text(&kit, &button), "«{button}»");
    assert!(
        !has_text(
            &kit,
            &format!("Top up by {}", crate::format::amount(200_000_000))
        ),
        "not 2 DASH"
    );
    assert!(!has_text(&kit, "Enter an amount, e.g. 0.25"));
}

/// Every DASH amount field outside «Send» and «Receive» (which have their own lines): two marks
/// say «Enter an amount» under the field and no amount is taken; one mark is an amount.
#[test]
fn two_marks_are_refused_under_every_amount_field() {
    for preset in [
        "top-up",
        "create-identity",
        "shield",
        "unshield",
        "fund-address",
    ] {
        let (turn, mut kit) = demo_app(preset, "en");
        kit.run_steps(10);
        *field_of(&mut kit, preset) = "1,234.56".into();
        kit.run_steps(3);
        assert!(
            has_text(&kit, "Enter an amount, e.g. 0.25"),
            "{preset}: the line under the field"
        );
        assert_eq!(
            crate::format::parse_amount(field_of(&mut kit, preset)),
            None
        );
        *field_of(&mut kit, preset) = "0,002".into();
        kit.run_steps(3);
        assert!(
            !has_text(&kit, "Enter an amount, e.g. 0.25"),
            "{preset}: «0,002» is an amount"
        );
        drop(kit);
        drop(turn);
    }
}

/// BUG-161, every field (the customer: «ни в одном поле, касающемся денег, токенов и чего угодно
/// ценного»): for each, «0,002» and «0.002» are the same amount, and «1,000,000» — no amount —
/// shows the field's own line, never read as another number. The table in `BUG-161-1.md`.
#[test]
fn every_amount_field_reads_one_mark_and_refuses_two() {
    // (preset, where the text goes, how the screen reads it, its line for no amount)
    type Set = fn(&mut App, &str);
    type Read = fn(&App) -> Option<i128>;
    let dash: Read = |_| None;
    let fields: &[(&str, Set, Read, &str)] = &[
        (
            "send",
            |app, t| app.send.amount = t.into(),
            |app| crate::format::parse_amount(&app.send.amount).map(i128::from),
            "Enter an amount, e.g. 0.25",
        ),
        (
            "send-mixed",
            |app, t| app.send.amount = t.into(),
            |app| crate::format::parse_amount(&app.send.amount).map(i128::from),
            "Enter an amount, e.g. 0.25",
        ),
        (
            "send-from-address",
            |app, t| app.send.amount = t.into(),
            |app| crate::format::parse_amount(&app.send.amount).map(i128::from),
            "Enter an amount, e.g. 0.25",
        ),
        (
            "receive",
            |app, t| app.receive.amount = t.into(),
            |app| crate::format::parse_amount(&app.receive.amount).map(i128::from),
            "Enter an amount, e.g. 0.25",
        ),
        (
            "top-up",
            |app, t| app.identity.top_up.as_mut().unwrap().amount = t.into(),
            |app| {
                crate::format::parse_amount(&app.identity.top_up.as_ref().unwrap().amount)
                    .map(i128::from)
            },
            "Enter an amount, e.g. 0.25",
        ),
        (
            "create-identity",
            |app, t| app.create_identity.amount = t.into(),
            |app| crate::format::parse_amount(&app.create_identity.amount).map(i128::from),
            "Enter an amount, e.g. 0.25",
        ),
        (
            "shield",
            |app, t| app.pool.amount = t.into(),
            |app| crate::format::parse_amount(&app.pool.amount).map(i128::from),
            "Enter an amount, e.g. 0.25",
        ),
        (
            "unshield",
            |app, t| app.pool.unshield_amount = t.into(),
            |app| crate::format::parse_amount(&app.pool.unshield_amount).map(i128::from),
            "Enter an amount, e.g. 0.25",
        ),
        (
            "fund-address",
            |app, t| app.fund_address.form.as_mut().unwrap().amount = t.into(),
            |app| {
                crate::format::parse_amount(&app.fund_address.form.as_ref().unwrap().amount)
                    .map(i128::from)
            },
            "Enter an amount, e.g. 0.25",
        ),
        (
            "tokens-send",
            |app, t| app.tokens.sending.as_mut().unwrap().amount = t.into(),
            |app| {
                // The demo's Evo Coffee has 2 decimals: the token's own (per token, R-4).
                crate::screens::tokens::state::typed(
                    &app.tokens.sending.as_ref().unwrap().amount,
                    2,
                )
                .ok()
                .map(i128::from)
            },
            "Not an amount of this token",
        ),
    ];
    let _ = dash;
    for &(preset, set, read, line) in fields {
        let (turn, mut kit) = demo_app(preset, "en");
        kit.run_steps(10);
        // DASH: 0.002 is 200 000 duffs. A token of 2 decimals has no 0.002: 0.02 is 2 of its
        // units, and «0,002» there is too many decimals (the token's line).
        let (pair, want) = if preset == "tokens-send" {
            (["0,02", "0.02"], 2)
        } else {
            (["0,002", "0.002"], 200_000)
        };
        for typed in pair {
            set(kit.state_mut(), typed);
            kit.run_steps(3);
            assert_eq!(read(kit.state()), Some(want), "{preset}: «{typed}»");
            assert!(!has_text(&kit, line), "{preset}: «{typed}» says nothing");
        }
        set(kit.state_mut(), "1,000,000");
        kit.run_steps(3);
        assert_eq!(
            read(kit.state()),
            None,
            "{preset}: «1,000,000» is no amount"
        );
        assert!(has_text(&kit, line), "{preset}: «{line}» under the field");
        drop(kit);
        drop(turn);
    }
}

/// BUG-161 r1 Should 1, the customer's decision, as the person sees it in English: «1,000» and
/// «2,500» in «Top up the identity» take no amount and ask which number is meant, under the field.
#[test]
fn an_english_thousand_asks_rather_than_being_read_as_one() {
    let (_turn, mut kit) = demo_app("top-up", "en");
    kit.run_steps(10);
    for (typed, line) in [
        ("1,000", "Unclear: is 1,000 1 or 1000? Type 1000 or 1.0."),
        ("2,500", "Unclear: is 2,500 2.5 or 2500? Type 2500 or 2.5."),
    ] {
        *field_of(&mut kit, "top-up") = typed.into();
        kit.run_steps(3);
        assert_eq!(
            crate::format::parse_amount(typed),
            None,
            "{typed}: no amount"
        );
        assert!(has_text(&kit, line), "«{line}» under the field");
        assert!(
            !has_text(
                &kit,
                &format!("Top up by {}", crate::format::amount(100_000_000))
            ),
            "{typed}: never 1 DASH"
        );
    }
}
