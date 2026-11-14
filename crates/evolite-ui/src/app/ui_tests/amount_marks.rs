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
