use super::*;

/// BUG-81 r2 (the designer): a row shows icons on all its cards or on none. At a 152 px
/// column the Credits card holds its icon (150.4) and Core does not (157): none get one;
/// a wider panel gives them to all; two plus one is decided once too.
#[test]
fn every_row_of_sources_shows_icons_on_all_cards_or_on_none() {
    let card = |words: f32, with_icon: f32| CardWidths { words, with_icon };
    let mixed = [card(129.0, 157.0), card(122.4, 150.4), card(122.4, 150.4)];
    let icons = |width: f32, cards: &[CardWidths]| -> Vec<bool> {
        from_layout(width, cards).iter().map(|c| c.icon).collect()
    };
    assert!(from_layout(474.0, &mixed).iter().all(|c| c.width == 152.0));
    assert_eq!(icons(474.0, &mixed), [false, false, false]);
    // Wide enough for the widest with its icon: on all.
    assert_eq!(icons(490.0, &mixed), [true, true, true]);
    // Two in the first row, one under them: the same column, the same answer.
    let wide = [card(160.0, 240.0), card(120.0, 148.0), card(120.0, 148.0)];
    assert!(from_layout(474.0, &wide).iter().all(|c| c.width == 233.0));
    assert_eq!(icons(474.0, &wide), [false, false, false]);
    for width in [300.0, 400.0, 474.0, 480.0, 520.0, 700.0] {
        for cards in [&mixed[..], &wide[..], &mixed[..2]] {
            let row = icons(width, cards);
            assert!(
                row.iter().all(|&i| i == row[0]),
                "{width}: a mixed row {row:?}"
            );
        }
    }
}

/// FEAT-0003 (AC-A1, AC-D1): a note is cut to 140 characters as a person sees them, and
/// says so until the next change; a family emoji is one character.
#[test]
fn a_note_longer_than_it_holds_is_cut_and_says_so() {
    let mut form = SendForm::empty();
    form.type_note(&"ж".repeat(150));
    assert_eq!(evolite_core::note_text::graphemes(&form.note), 140);
    assert!(form.note_cut);
    form.type_note("За дизайн 👨‍👩‍👧‍👦\u{202E}");
    assert_eq!(form.note, "За дизайн 👨‍👩‍👧‍👦");
    assert!(!form.note_cut, "gone at the next change");
    assert_eq!(form.note_to_send().as_deref(), Some("За дизайн 👨‍👩‍👧‍👦"));
    form.type_note("   ");
    assert_eq!(form.note_to_send(), None);
}

/// FEAT-0003 (AC-A3): the note leaves the form with the payment; a refusal puts it back
/// (nothing was kept), a payment taken keeps it with the core, and a lock wipes it.
#[test]
fn a_refused_payment_gives_the_note_back_and_a_lock_wipes_it() {
    let mut form = SendForm::empty();
    form.type_note("rent");
    form.signed_note = std::mem::take(&mut form.note);
    form.waiting = 1;
    form.on_sent(0, &Err(CoreError::NotEnough));
    assert_eq!(form.note, "rent");
    form.signed_note = std::mem::take(&mut form.note);
    form.waiting = 1;
    form.on_sent(
        0,
        &Ok(SentPayment {
            txid: "aa".into(),
            amount: 1,
            fee: 1,
            relayed: true,
        }),
    );
    assert!(form.note.is_empty() && form.signed_note.is_empty());
    form.type_note("typed, not sent");
    form.on_lock();
    assert!(form.note.is_empty() && form.signed_note.is_empty() && !form.note_cut);
}

/// BUG-77: the Core source says how sure its balance is — the last checked one and when, or
/// only «not known yet» (no 0 as if checked); the other sources are as they were.
#[test]
fn the_core_source_says_when_its_balance_was_checked() {
    use evolite_api::CoreAsOf;
    let view = |source| SourceView {
        source,
        icon: "core",
        title: "Core",
        duffs: 386_574_464,
        fee: 0,
        max_fee: 0,
        withdrawal: None,
        read: true,
        note_icon: "",
        note: "",
    };
    let balance = crate::format::balance(386_574_464);
    let core = view(Source::Core);
    assert_eq!(source_detail(&core, None, None), balance);
    // The accessibility name of a «From» card is its title: the words are checked here. Credits
    // not known are no 0 (BUG-91); kept from an earlier run they are the amount, dimmed (BUG-115).
    assert_eq!(
        source_line(
            &SourceView {
                read: false,
                duffs: 0,
                ..view(Source::Credits)
            },
            None,
            None
        ),
        "Not known yet"
    );
    assert_eq!(source_line(&view(Source::Credits), None, None), balance);
    assert_eq!(source_line(&core, None, None), balance);
    assert_eq!(
        source_detail(&core, Some(CoreAsOf::Checked(1)), Some("as of 23:41")),
        balance
    );
    assert_eq!(
        source_detail(&core, Some(CoreAsOf::NotYet), Some("Not known yet")),
        "Not known yet"
    );
    assert_eq!(
        source_detail(
            &view(Source::Credits),
            Some(CoreAsOf::NotYet),
            Some("Not known yet")
        ),
        balance,
        "credits are not Core"
    );
}

#[test]
fn the_amount_line_says_what_is_wrong() {
    use crate::format::{Display, NumberStyle, Unit, set_display};
    use AmountProblem::*;
    set_display(Display {
        unit: Unit::Dash,
        style: NumberStyle::CommaDot,
        ..Display::default()
    });
    // A payment with a fee, not a withdrawal (that one is below): 0.0018 DASH and a fee of
    // 0.0021 DASH, nothing can go, whatever is typed.
    for typed in ["", "0.000000018", "0.00000018", "0.001"] {
        assert_eq!(
            amount_problem(typed, 180_000, 210_000, None),
            Some(NoFunds),
            "{typed}"
        );
    }
    assert_eq!(
        amount_problem("", 210_000, 210_000, None),
        Some(NoFunds),
        "the fee alone"
    );
    // An empty balance, or a fee not known yet: no line before typing, the usual ones after.
    assert_eq!(amount_problem("", 0, 210_000, None), None, "empty");
    assert_eq!(
        amount_problem("0.001", 0, 210_000, None),
        Some(MoreThanBalance),
        "empty, typed"
    );
    assert_eq!(amount_problem("", 0, 0, None), None, "empty, no fee");
    assert_eq!(amount_problem("", 180_000, 0, None), None, "no fee yet");
    assert_eq!(
        amount_problem("0.001", 180_000, 0, None),
        None,
        "no fee yet, typed"
    );
    // 0.01 DASH, the same fee: the edges.
    let (balance, fee) = (1_000_000, 210_000);
    let cases = [
        ("", None),
        ("0", Some(NotAnAmount)),
        ("abc", Some(NotAnAmount)),
        ("0.00000001", None),
        ("0.0079", None),
        ("0.00790001", Some(NoRoomForFee)),
        ("0.01", Some(NoRoomForFee)),
        ("0.01000001", Some(MoreThanBalance)),
        ("0.000000018", Some(TooPrecise)),
    ];
    for (typed, problem) in cases {
        assert_eq!(
            amount_problem(typed, balance, fee, None),
            problem,
            "{typed}"
        );
    }
    assert_eq!(max_amount(balance, fee, None), 790_000);
    assert_eq!(max_amount(100_000, fee, None), 0, "not below zero");
    let text = amount_problem_text(NoRoomForFee, fee, None);
    assert!(text.contains("0.0021"), "the fee is named: {text}");
    let text = amount_problem_text(TooPrecise, fee, None);
    assert!(
        text.contains("0.00000001"),
        "the smallest step is named: {text}"
    );
    set_display(Display::default());
}

/// Credits to Core: Platform refuses a balance under the amount + 0.004 DASH and an amount
/// under 1190 duffs, whatever the last withdrawal cost (BUG-11, review r1).
#[test]
fn a_withdrawal_keeps_the_reserve_and_the_smallest_amount() {
    use crate::format::{Display, NumberStyle, Unit, set_display};
    use AmountProblem::*;
    set_display(Display {
        unit: Unit::Dash,
        style: NumberStyle::CommaDot,
        ..Display::default()
    });
    let fees = evolite_api::Fees {
        credit_withdrawal: 210_000,
        credit_withdrawal_reserve: 400_000,
        ..zero_fees()
    };
    let w = Withdrawal::of(&fees);
    assert_eq!(
        (w.reserve, w.min),
        (400_000, 1_190),
        "the reserve, not the price"
    );
    let dearer = Withdrawal::of(&evolite_api::Fees {
        credit_withdrawal: 500_000,
        ..fees.clone()
    });
    assert_eq!(
        dearer.reserve, 500_000,
        "a dearer last withdrawal is kept too"
    );
    let fee = fees.credit_withdrawal;
    let at = |typed: &str, balance: Duffs| amount_problem(typed, balance, fee, Some(w));
    // The customer's 0.0018 DASH: nothing can go, whatever is typed.
    for typed in ["", "0.000000018", "0.00000018", "0.001"] {
        assert_eq!(at(typed, 180_000), Some(NoFunds), "{typed}");
    }
    // An empty balance: no line before typing.
    assert_eq!(at("", 0), None);
    assert_eq!(at("0.001", 0), Some(MoreThanBalance));
    // The reserve and the smallest amount, to the duff.
    assert_eq!(at("", 400_000 + 1_189), Some(NoFunds), "reserve + 1189");
    assert_eq!(at("", 400_000 + 1_190), None, "reserve + 1190");
    assert_eq!(at("0.0000119", 400_000 + 1_190), None);
    assert_eq!(max_amount(400_000 + 1_190, fee, Some(w)), 1_190);
    // 0.01 DASH: the edges around balance − reserve (0.006).
    let balance = 1_000_000;
    let cases = [
        ("", None),
        ("0", Some(NotAnAmount)),
        ("0.000000018", Some(TooPrecise)),
        ("0.00000001", Some(BelowMinimum)),
        ("0.00001189", Some(BelowMinimum)),
        ("0.0000119", None),
        ("0.00599999", None),
        ("0.006", None),
        ("0.00600001", Some(NoRoomForFee)),
        // Within balance − price, but Platform refuses it: round 1 let it through.
        ("0.0079", Some(NoRoomForFee)),
        ("0.01", Some(NoRoomForFee)),
        ("0.01000001", Some(MoreThanBalance)),
    ];
    for (typed, problem) in cases {
        assert_eq!(at(typed, balance), problem, "{typed}");
    }
    // "Max": the balance less the reserve, which the form itself takes.
    let max = max_amount(balance, fee, Some(w));
    assert_eq!(max, 600_000);
    assert_eq!(at(&crate::format::input_text(max), balance), None);
    assert_eq!(max_amount(180_000, fee, Some(w)), 0);
    // The lines name the reserve and the smallest amount, not the price.
    let text = amount_problem_text(NoFunds, fee, Some(w));
    assert!(text.contains("0.004"), "{text}");
    let text = amount_problem_text(NoRoomForFee, fee, Some(w));
    assert!(text.contains("0.004"), "{text}");
    let text = amount_problem_text(BelowMinimum, fee, Some(w));
    assert!(text.contains("0.0000119"), "{text}");
    set_display(Display::default());
}

#[test]
fn credits_to_core_cost_a_withdrawal() {
    let fees = evolite_api::Fees {
        credit_transfer: 777,
        credit_transfer_to_address: 888,
        credit_withdrawal: 400_000,
        ..zero_fees()
    };
    assert_eq!(credits_fee(&fees, "@alice"), 777);
    assert_eq!(
        credits_fee(&fees, "tdash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq"),
        888
    );
    assert_eq!(
        credits_fee(&fees, " yStwA1xNxEE2kv5ZVDNGSsPach8pBApWHZ"),
        400_000
    );
    assert!(credits_to_core(" yStwA1xNxEE2kv5ZVDNGSsPach8pBApWHZ"));
    for to in ["", "@alice", "TDASH1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq"] {
        assert!(
            !credits_to_core(to),
            "{to}: a transfer, no withdrawal reserve"
        );
    }
}

fn zero_fees() -> evolite_api::Fees {
    evolite_api::Fees {
        core_payment: 0,
        core_unsignable: 0,
        core_coins: Vec::new(),
        credit_transfer: 0,
        credit_transfer_to_address: 0,
        pool_transfer: 0,
        nft_transfer: 0,
        document_write: 0,
        shield: 0,
        identity_create: 0,
        identity_lock_min: 0,
        top_up_lock_min: 50_500,
        address_fund_on_top: 0,
        address_fund_lock_min: 0,
        name_regular: 0,
        name_contested: 0,
        add_key: 0,
        contact_request: 0,
        contact_request_balance: 0,
        profile: 0,
        shield_from_address: 0,
        shield_reserve: 0,
        unshield: 0,
        pool_withdrawal: 0,
        shield_from_core: 0,
        pool_top_up: 0,
        credit_withdrawal: 0,
        credit_withdrawal_reserve: 0,
    }
}
use evolite_api::{Chain, Proof};

fn paid(to: &str, at: Timestamp, amount: Duffs) -> Activity {
    Activity {
        id: 1,
        kind: ActivityKind::Sent { to: to.into() },
        chain: Chain::Core,
        at,
        amount,
        proof: Proof::Sending,
        txid: None,
        fee: None,
        block: None,
        address: None,
        layers: evolite_api::layer::CORE,
        flow: evolite_api::Flow::Out,
        key: String::new(),
        detail: None,
        note: None,
        rate: None,
    }
}

#[test]
fn offers_saved_addresses_by_name_or_address() {
    let book = [
        SavedAddress {
            address: "yAAA1".into(),
            label: "Rent".into(),
        },
        SavedAddress {
            address: "yBBB2".into(),
            label: "Exchange".into(),
        },
    ];
    assert_eq!(book_matches(&book, "").len(), 2, "all while empty");
    assert_eq!(book_matches(&book, "exc")[0].label, "Exchange", "by name");
    assert_eq!(book_matches(&book, "aaa")[0].label, "Rent", "by address");
    assert!(
        book_matches(&book, "yBBB2").is_empty(),
        "a saved address is chosen"
    );
    assert!(
        book_matches(&book, "@alice").is_empty(),
        "a name is not an address"
    );
}

#[test]
fn a_refused_payment_gives_its_amount_back() {
    let mut form = SendForm {
        waiting: 2,
        last_payment: 2,
        signed: "0,25".into(),
        ..SendForm::empty()
    };
    // Another payment is still out: this answer may be that one's, nothing comes back (BUG-92
    // r2), and what was signed last stays for its own answer.
    form.on_sent(1, &Err(CoreError::NotEnough));
    assert_eq!((form.waiting, form.amount.as_str()), (1, ""));
    assert_eq!(form.signed, "0,25");
    assert!(
        form.result.is_some() && form.reveal_result,
        "the refusal is said"
    );
    // The last one out: it is the payment's own answer, its amount comes back.
    form.on_sent(2, &Err(CoreError::NotEnough));
    assert_eq!((form.waiting, form.amount.as_str()), (0, "0,25"));
    // Taken: the form stays as the person left it.
    form.waiting = 1;
    form.amount = "0,1".into();
    form.on_sent(
        2,
        &Ok(SentPayment {
            txid: "aa".into(),
            amount: 1,
            fee: 1,
            relayed: true,
        }),
    );
    assert_eq!((form.waiting, form.amount.as_str()), (0, "0,1"));
}

/// BUG-92 r2: with another payment out, a refusal gives back neither the note nor a «Max»
/// amount: the note is kept for the last payment's answer, and «Max» would fill the form again.
/// An amount typed by hand for the next payment is the person's and stays.
#[test]
fn a_refusal_with_another_payment_out_gives_nothing_back() {
    let mut form = SendForm {
        waiting: 2,
        last_payment: 2,
        signed: "0,25".into(),
        signed_note: "for B".into(),
        max: true,
        amount: "1,5".into(),
        ..SendForm::empty()
    };
    form.on_sent(1, &Err(CoreError::WrongPassword));
    assert!(
        !form.max && form.amount.is_empty(),
        "«Max» does not refill it"
    );
    assert!(form.note.is_empty());
    assert_eq!(
        (form.signed.as_str(), form.signed_note.as_str()),
        ("0,25", "for B")
    );

    let mut typed = SendForm {
        waiting: 2,
        last_payment: 2,
        amount: "0,7".into(),
        ..SendForm::empty()
    };
    typed.on_sent(1, &Err(CoreError::WrongPassword));
    assert_eq!(typed.amount, "0,7");
}

/// TECH-sent-id: the answer says whose it is, so a refusal of the last payment gives its amount
/// and note back even with an earlier one still out.
#[test]
fn the_last_payments_refusal_comes_back_with_another_still_out() {
    let mut form = SendForm {
        waiting: 2,
        last_payment: 2,
        signed: "0,25".into(),
        signed_note: "for B".into(),
        ..SendForm::empty()
    };
    form.on_sent(2, &Err(CoreError::NotEnough));
    assert_eq!((form.waiting, form.amount.as_str()), (1, "0,25"));
    assert_eq!(form.note, "for B");
}

/// Two payments out, A and B (the last): a form as the last look leaves it after signing B.
fn two_out() -> SendForm {
    SendForm {
        waiting: 2,
        last_payment: 2,
        signed: "0,25".into(),
        signed_note: "for B".into(),
        recipient: ADDRESS.into(),
        sent_to: ADDRESS.into(),
        ..SendForm::empty()
    }
}

fn sent() -> Result<SentPayment, CoreError> {
    Ok(SentPayment {
        txid: "aa".into(),
        amount: 1,
        fee: 1,
        relayed: true,
    })
}

/// R-10 r1 must 1: A taken says nothing of B, which is refused after it.
#[test]
fn an_earlier_payment_taken_leaves_the_last_ones_refusal_its_amount() {
    let mut form = two_out();
    form.on_sent(1, &sent());
    assert_eq!(form.recipient, ADDRESS, "B's recipient stays");
    assert_eq!(form.signed, "0,25", "B's amount waits for its answer");
    form.on_sent(2, &Err(CoreError::NotEnough));
    assert_eq!(
        (form.amount.as_str(), form.note.as_str()),
        ("0,25", "for B")
    );
    assert!(matches!(form.result, Some(Err(CoreError::NotEnough))));
}

/// R-10 r1 must 1: B refused stays said when A is taken after it: B did not go.
#[test]
fn an_earlier_payment_taken_does_not_replace_the_last_ones_refusal() {
    let mut form = two_out();
    form.on_sent(2, &Err(CoreError::NotEnough));
    form.on_sent(1, &sent());
    assert!(matches!(form.result, Some(Err(CoreError::NotEnough))));
    assert_eq!(form.recipient, ADDRESS, "to try B again");
    assert_eq!(form.waiting, 0);
}

/// R-10 r2 must 4: what the person sees is the last payment's own answer, also after an earlier
/// one was refused: a payment that left is never hidden behind another's «not signed».
#[test]
fn the_last_payments_own_answer_is_what_is_shown() {
    let mut form = two_out();
    form.on_sent(1, &Err(CoreError::WrongPassword));
    form.on_sent(2, &sent());
    assert!(matches!(form.result, Some(Ok(_))));
    assert!(!form.reveal_result);
    assert!(form.recipient.is_empty(), "B went");

    // With no clear answer from the network: «Do not send it again» has to be seen.
    let mut unclear = two_out();
    unclear.on_sent(1, &Err(CoreError::WrongPassword));
    let not_relayed = Ok(SentPayment {
        txid: String::new(),
        amount: 1,
        fee: 0,
        relayed: false,
    });
    unclear.on_sent(2, &not_relayed);
    assert!(matches!(
        unclear.result,
        Some(Ok(SentPayment { relayed: false, .. }))
    ));
}

/// R-10 r3 should 4: once the last payment has answered, an earlier payment's answer does not
/// touch that line: not «Do not send it again», and not an amount that came back.
#[test]
fn a_late_answer_of_an_earlier_payment_leaves_the_last_ones_alone() {
    let unclear = || {
        Ok(SentPayment {
            txid: String::new(),
            amount: 1,
            fee: 0,
            relayed: false,
        })
    };
    let lasts = [sent(), unclear(), Err(CoreError::NotEnough)];
    for last in &lasts {
        for late in [sent(), Err(CoreError::WrongPassword)] {
            let mut form = SendForm {
                max: true,
                ..two_out()
            };
            form.on_sent(2, last);
            let amount = form.amount.clone();
            form.on_sent(1, &late);
            assert_eq!(
                format!("{:?}", form.result),
                format!("{:?}", Some(last)),
                "{late:?}"
            );
            assert_eq!(form.reveal_result, last.is_err(), "{late:?}");
            assert_eq!(form.amount, amount, "{late:?}");
            assert_eq!(form.waiting, 0);
        }
    }
}

/// R-10 r1 should 2: A's refusal after B was answered gives nothing back.
#[test]
fn a_late_refusal_of_an_earlier_payment_gives_nothing_back() {
    let mut form = two_out();
    form.on_sent(2, &sent());
    form.signed = "0,25".into();
    form.on_sent(1, &Err(CoreError::NotEnough));
    assert_eq!((form.waiting, form.amount.as_str()), (0, ""));
}

/// R-10 r1 should 1: a number this form never gave is nobody's here.
#[test]
fn an_answer_of_an_unknown_payment_changes_nothing() {
    let mut form = SendForm {
        signing: Some(std::time::Instant::now()),
        ..two_out()
    };
    form.on_sent(99, &Err(CoreError::WrongPassword));
    assert_eq!(form.waiting, 2);
    assert!(form.signing.is_some() && form.result.is_none());
}

/// R-10 r1 should 1: only the last payment's answer ends the wait of the dialog.
#[test]
fn an_earlier_answer_does_not_end_the_wait_of_the_last_look() {
    let mut form = SendForm {
        signing: Some(std::time::Instant::now()),
        confirming: true,
        ..two_out()
    };
    form.on_sent(1, &Err(CoreError::WrongPassword));
    assert!(form.signing.is_some() && form.confirming && !form.password_wrong);
}

/// R-10 r1 must 2: the number outlives a form replaced, or a new payment would take an old one's.
#[test]
fn a_new_form_keeps_the_payment_number() {
    let mut form = SendForm {
        last_payment: 7,
        ..SendForm::empty()
    };
    form.replace(SendForm::paying(Source::Credits, "yX".into()));
    assert_eq!(form.last_payment, 7);
}

#[test]
fn a_withdrawal_opens_from_credits_to_the_address() {
    let form = SendForm::paying(Source::Credits, "yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2".into());
    assert_eq!(
        form.want,
        Some(Source::Credits),
        "Credits, wherever it stands"
    );
    assert_eq!(form.recipient, "yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2");
    assert!(form.amount.is_empty(), "the amount is the person's to type");
}

#[test]
fn warns_of_the_same_payment_within_an_hour() {
    let history = [paid("@bob", 1_000, -100_226)];
    let found = |to: &str, amount: Duffs, now: Timestamp| {
        similar_payment(&history, to, amount, 500, now).is_some()
    };
    assert!(found("@bob", 100_000, 1_100));
    assert!(!found("@dave", 100_000, 1_100), "someone else");
    assert!(!found("@bob", 90_000, 1_100), "another amount");
    assert!(!found("@bob", 100_000, 1_000 + 3_600), "an hour later");
}

#[test]
fn a_lock_closes_the_last_look_but_not_a_payment_with_the_network() {
    let mut form = SendForm {
        recipient: "@alice".into(),
        amount: "0,1".into(),
        confirming: true,
        password: Zeroizing::new("password".into()),
        ..SendForm::default()
    };
    form.on_lock();
    assert!(!form.confirming && form.password.is_empty());

    let mut sent = SendForm {
        waiting: 1,
        ..SendForm::default()
    };
    sent.on_lock();
    assert_eq!(sent.waiting, 1, "the network's answer is still awaited");
}

/// BUG-92: the answer to the password the last look holds for: wrong keeps the dialog, the
/// amount and the password to correct, and is no result of the form; any other closes it,
/// wipes the password, and a refusal comes into view. Without the hold it goes as before.
#[test]
fn the_last_look_stays_only_for_a_wrong_password() {
    let held = || SendForm {
        amount: String::new(),
        signed: "0,1".into(),
        confirming: true,
        waiting: 1,
        password: Zeroizing::new("nope".into()),
        signing: Some(std::time::Instant::now()),
        ..SendForm::default()
    };
    let mut form = held();
    form.on_sent(0, &Err(CoreError::WrongPassword));
    assert!(form.confirming && form.signing.is_none() && form.password_wrong);
    assert!(form.refocus_password && !form.reveal_result);
    assert!(form.result.is_none(), "not said twice");
    assert_eq!(form.amount, "0,1", "the amount is back");
    assert_eq!(form.password.as_str(), "nope", "to correct, not to retype");
    assert_eq!(form.waiting, 0);

    let mut form = held();
    form.on_sent(0, &Err(CoreError::NotEnough));
    assert!(!form.confirming && form.password.is_empty());
    assert!(form.reveal_result && !form.password_wrong);
    assert_eq!(form.result, Some(Err(CoreError::NotEnough)));

    let mut form = held();
    form.on_sent(
        0,
        &Ok(SentPayment {
            txid: "ab".into(),
            amount: 1,
            fee: 1,
            relayed: true,
        }),
    );
    assert!(!form.confirming && form.password.is_empty());
    assert!(!form.reveal_result, "a success does not move the form");

    // The dialog closed meanwhile (the cross, or the wait ran out): the line is on the form.
    let mut form = SendForm {
        waiting: 1,
        ..SendForm::default()
    };
    form.on_sent(0, &Err(CoreError::WrongPassword));
    assert!(form.reveal_result && !form.password_wrong);
    assert_eq!(form.result, Some(Err(CoreError::WrongPassword)));
}

/// "Cancel", "Send" from a contact, "Withdraw to Core": a new form, and a payment still
/// with the network still awaited (the form said nothing of it before).
#[test]
fn a_new_form_still_awaits_the_payments_with_the_network() {
    let mut form = SendForm {
        recipient: "@alice".into(),
        amount: "0,1".into(),
        waiting: 1,
        ..SendForm::default()
    };
    form.replace(SendForm::paying(Source::Credits, "yX".into()));
    assert_eq!(form.waiting, 1);
    assert_eq!((form.recipient.as_str(), form.amount.as_str()), ("yX", ""));
    form.on_sent(0, &Err(CoreError::NotEnough));
    assert_eq!(form.waiting, 0);
}

const ADDRESS: &str = "yNBw8AS5PhKdSm6ytP3uAVEsx5PjCiReb4";

/// BUG-32, BUG-39 №5: the summary's "To" is "—" until the recipient is one, and short.
#[test]
fn the_summary_names_only_a_recipient_and_briefly() {
    assert_eq!(summary_recipient("", true), "—");
    assert_eq!(summary_recipient("q", false), "—", "not a recipient");
    assert_eq!(summary_recipient(ADDRESS, true), "yNBw…Reb4");
    assert_eq!(summary_recipient(" @alice ", true), "@alice");
    let long = format!("@{}", "a".repeat(40));
    let shown = summary_recipient(&long, true);
    assert_eq!(shown.chars().count(), 24);
    assert!(shown.ends_with('…'));

    assert!(looks_like_address(ADDRESS));
    assert!(looks_like_address(
        "tdash1kq0ufhcm3cyzpt8y2nxhsffj4sjwjq3h7ckhfmqczhlkxwj9f9v8ycpdtyl"
    ));
    for junk in [
        "q",
        "yNotAnAddress123",
        "yNBw8AS5PhKdSm6ytP3uAVEsx5PjCiRe0",
        "y N B w",
    ] {
        assert!(!looks_like_address(junk), "{junk}");
    }
}

/// BUG-39 №9: an address of this wallet is not offered for the address book.
#[test]
fn knows_the_wallets_own_addresses() {
    let mut came = paid("", 1, 100);
    came.kind = ActivityKind::Received { from: None };
    came.address = Some("yOld".into());
    let mut went = paid("yTheirs", 1, -100);
    went.address = Some("yTheirs".into());
    let history = [came, went];
    assert!(own_of(&["yNow", "tdash1me"], &history, " yNow "));
    assert!(own_of(&["yNow", "tdash1me"], &history, "tdash1me"));
    assert!(
        own_of(&["yNow"], &history, "yOld"),
        "one that received before"
    );
    assert!(!own_of(&["yNow"], &history, "yTheirs"), "a payee");
    assert!(!own_of(&["", "yNow"], &history, ""), "nothing typed");
}

/// BUG-68: an address this wallet paid itself to — the receive address moved on after the
/// payment — is still its own, typed with spaces around or not.
#[test]
fn an_address_the_wallet_paid_itself_to_is_its_own() {
    let mut moved = paid("", 1, -226);
    moved.kind = ActivityKind::SelfTransfer { moved: 10_000_000 };
    moved.address = Some("yMine".into());
    let history = [moved];
    assert!(own_of(&["yNext"], &history, "yMine"));
    assert!(own_of(&["yNext"], &history, "  yMine  "), "with spaces");
    assert!(!own_of(&["yNext"], &history, "yOther"));
}

/// BUG-39 №6: "Max" follows the source chosen after it; a typed amount stays.
#[test]
fn max_follows_the_source_until_the_amount_is_typed() {
    let mut form = SendForm {
        max: true,
        ..SendForm::empty()
    };
    form.follow_max(100_000_000, 1_000, None);
    assert_eq!(form.amount, crate::format::input_text(99_999_000));
    // Credits now: another balance and fee.
    form.follow_max(50_000_000, 2_000, None);
    assert_eq!(form.amount, crate::format::input_text(49_998_000));
    form.max = false;
    form.amount = "0.1".into();
    form.follow_max(100_000_000, 1_000, None);
    assert_eq!(form.amount, "0.1");
}

fn taken() -> Result<SentPayment, CoreError> {
    Ok(SentPayment {
        txid: "aa".into(),
        amount: 1,
        fee: 1,
        relayed: true,
    })
}

/// BUG-39 №7: a payment taken clears "To", unless another recipient is typed by then; a
/// refused one keeps it.
#[test]
fn a_payment_taken_clears_the_recipient() {
    let mut form = SendForm {
        recipient: ADDRESS.into(),
        sent_to: ADDRESS.into(),
        waiting: 1,
        max: true,
        ..SendForm::empty()
    };
    form.on_sent(0, &Err(CoreError::NotEnough));
    assert_eq!(form.recipient, ADDRESS, "refused: the person may try again");
    form.result = None; // «Check and send» again
    form.waiting = 1;
    form.on_sent(0, &taken());
    assert!(form.recipient.is_empty() && !form.max);
    assert!(
        matches!(form.result, Some(Ok(_))),
        "the answer is still shown"
    );

    let mut next = SendForm {
        recipient: "@bob".into(),
        sent_to: ADDRESS.into(),
        waiting: 1,
        ..SendForm::empty()
    };
    next.on_sent(0, &taken());
    assert_eq!(next.recipient, "@bob", "the next payment typed meanwhile");
}

/// BUG-39 №7: a lock clears the form — what was typed and the last answer — and keeps
/// awaiting a payment with the network.
#[test]
fn a_lock_clears_the_form() {
    let mut form = SendForm {
        source: 1,
        recipient: ADDRESS.into(),
        amount: "0,1".into(),
        max: true,
        signed: "0,2".into(),
        sent_to: ADDRESS.into(),
        waiting: 1,
        result: Some(taken()),
        naming: Some("rent".into()),
        ..SendForm::default()
    };
    form.on_lock();
    assert!(form.recipient.is_empty() && form.amount.is_empty() && form.signed.is_empty());
    assert!(form.result.is_none() && form.naming.is_none() && !form.max);
    assert_eq!((form.source, form.waiting), (0, 1));
    // The answer of the payment with the network gives no amount back after the lock.
    form.on_sent(0, &Err(CoreError::NotEnough));
    assert!(form.amount.is_empty());
}

/// TECH-price-check AC-T2: the price the journal records as promised is the number this form
/// shows, for each way of paying and each kind of recipient.
#[test]
fn the_price_on_record_is_the_one_the_send_form_shows() {
    use evolite_core::price::{Seen, declared};
    let fees = evolite_api::Fees {
        core_payment: 11,
        credit_transfer: 12,
        credit_transfer_to_address: 33,
        credit_withdrawal: 31,
        pool_transfer: 13,
        unshield: 27,
        pool_withdrawal: 28,
        contact_request: 23,
        profile: 24,
        name_regular: 20,
        ..zero_fees()
    };
    let seen = Seen {
        fees: fees.clone(),
        core: 1_000,
        credits: 1_000,
        platform_address: 1_000,
        pool: 1_000,
        mixed: 0,
        platform_inputs: Vec::new(),
        platform_input_addresses: Vec::new(),
        first_name: false,
    };
    let send = |from, to: &str, amount| Spend::Send {
        coins: None,
        id: 1,
        from,
        to: to.into(),
        amount,
        note: None,
        rate: None,
    };
    let price = |op: &Spend| declared(op, &seen, 0).declared;
    let private = format!("tdash1{}", "z".repeat(70));
    for to in [
        "@bob",
        "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ",
        "tdash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq",
        private.as_str(),
        "",
    ] {
        assert_eq!(
            price(&send(Source::Credits, to, 5)),
            credits_fee(&fees, to),
            "{to}"
        );
        assert_eq!(
            price(&send(Source::Pool, to, 5)),
            crate::screens::shielded::pool_fee(&fees, to),
            "{to}"
        );
    }
    assert_eq!(
        price(&send(Source::Core, "@bob", 5)),
        fees.core_payment_for(5)
    );
    // The forms that read a field or a `Fees` method directly: `contacts`, `identity/mod`,
    // `identity/add_name`, and `identity/top_up`, whose fee of the lock is `core_payment`.
    assert_eq!(
        price(&Spend::AddContact { name: "b".into() }),
        fees.contact_request
    );
    assert_eq!(
        price(&Spend::AcceptContact {
            identity_id: "i".into()
        }),
        fees.contact_request
    );
    assert_eq!(
        price(&Spend::SaveProfile {
            display: String::new(),
            about: String::new(),
            avatar_url: None
        }),
        fees.profile
    );
    assert_eq!(
        price(&Spend::AddName { name: "b".into() }),
        fees.name_price(false)
    );
    let first = Seen {
        first_name: true,
        ..seen.clone()
    };
    assert_eq!(
        declared(&Spend::AddName { name: "b".into() }, &first, 0).declared,
        fees.name_price(true)
    );
    assert_eq!(
        price(&Spend::TopUpIdentity { amount: 5 }),
        fees.top_up_on_top() + fees.core_payment
    );
}

/// BUG-108 r1 must 1: Platform meters credits and has no cap, so their fee is an estimate, said so
/// on the fee and on the total. A Core fee is known before signing and stays "at most".
#[test]
fn only_a_fee_known_before_signing_says_at_most() {
    let (label, fee, total) = confirm::fee_lines(100_000, 5_000, true);
    assert_eq!(label, "Fee, about");
    assert!(fee.starts_with("≈ "), "{fee}");
    assert!(total.starts_with("≈ "), "{total}");
    let (label, fee, total) = confirm::fee_lines(100_000, 226, false);
    assert_eq!(label, "Fee, at most");
    assert!(!fee.contains('≈') && !total.contains('≈'), "{fee} {total}");
}

const QR_ADDR: &str = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f";

/// FEAT-0006 FR-6, FR-7: a `dash:` link in «To» (from a QR or pasted) fills the recipient,
/// the amount and the note, and says what it filled; a refused link fills in nothing; a
/// scan of an app link or of no code says why and leaves what was typed.
#[test]
fn a_dash_link_fills_the_form_as_a_qr_does() {
    use crate::components::QrNote;
    use crate::qr_scan::{Kind, Scanned};
    let mut form = SendForm::empty();
    form.recipient = "@typed".into();
    assert!(form.take_link(&format!(
        "dash:{QR_ADDR}?amount=0.25&label=For%20the%20design"
    )));
    assert_eq!(form.recipient, QR_ADDR);
    assert_eq!(crate::format::parse_amount(&form.amount), Some(25_000_000));
    assert_eq!(form.note, "For the design");
    assert!(form.note_from_link());
    assert_eq!(form.qr, Some(QrNote::Filled("address and amount")));
    // The note changed by hand is the person's own.
    form.type_note("For the logo");
    assert!(!form.note_from_link());
    // `user=`: the payment goes to the contact by name; `message` when there is no label.
    let mut form = SendForm::empty();
    assert!(form.take_link(&format!("dash:{QR_ADDR}?user=satoshi&message=Hi")));
    assert_eq!(form.recipient, "@satoshi");
    assert_eq!(form.note, "Hi");
    assert_eq!(form.qr, Some(QrNote::Filled("name")));
    // Refused whole: nothing filled in.
    let mut form = SendForm::empty();
    form.amount = "1".into();
    assert!(form.take_link(&format!("dash:{QR_ADDR}?amount=1e3")));
    assert_eq!(form.amount, "1");
    assert!(form.link_note.is_none());
    assert!(matches!(form.qr, Some(QrNote::Error(_))));
    assert!(!form.take_link(QR_ADDR), "not a link");
    // Scans.
    let mut form = SendForm::empty();
    form.take_scan(&Scanned::Found {
        text: QR_ADDR.into(),
        kind: Kind::Address,
    });
    assert_eq!(
        (form.recipient.as_str(), form.qr.clone()),
        (QR_ADDR, Some(QrNote::Filled("address")))
    );
    form.take_scan(&Scanned::Found {
        text: "dash-key:abc".into(),
        kind: Kind::AppLink,
    });
    assert_eq!(form.recipient, QR_ADDR, "what was there stays");
    assert!(matches!(form.qr, Some(QrNote::Error(_))));
    form.qr = None;
    form.take_scan(&Scanned::Cancelled);
    assert_eq!(form.qr, None, "a closed dialog says nothing");
    // A lock takes the link's note with the rest.
    form.take_link(&format!("dash:{QR_ADDR}?label=x"));
    form.on_lock();
    assert!(form.link_note.is_none() && form.qr.is_none());
}

/// A scan that ends during «Check the payment» does not change the payment being checked.
#[test]
fn a_scan_ending_during_the_last_look_changes_nothing() {
    use crate::qr_scan::{Kind, Scanned};
    let mut form = SendForm::empty();
    form.recipient = "@checked".into();
    form.confirming = true;
    for scanned in [
        Scanned::Found {
            text: QR_ADDR.into(),
            kind: Kind::Address,
        },
        Scanned::Found {
            text: format!("dash:{QR_ADDR}?amount=5&label=x"),
            kind: Kind::Uri,
        },
    ] {
        form.take_scan(&scanned);
        assert_eq!(form.recipient, "@checked");
        assert!(form.amount.is_empty() && form.note.is_empty() && form.qr.is_none());
    }
}

/// The note of an earlier link is not the next recipient's, whatever brings that recipient;
/// a note the person wrote stays.
#[test]
fn a_new_code_takes_the_last_links_note_but_not_the_persons() {
    use crate::qr_scan::{Kind, Scanned};
    let mut form = SendForm::empty();
    form.take_link(&format!("dash:{QR_ADDR}?label=First"));
    assert_eq!(form.note, "First");
    form.take_link(&format!("dash:{QR_ADDR}"));
    assert!(form.note.is_empty() && form.link_note.is_none());
    form.take_link(&format!("dash:{QR_ADDR}?label=First"));
    form.take_scan(&Scanned::Found {
        text: "@bob".into(),
        kind: Kind::Name,
    });
    assert!(form.note.is_empty() && form.link_note.is_none());
    form.take_link(&format!("dash:{QR_ADDR}?label=First"));
    form.type_note("Mine");
    form.take_link(&format!("dash:{QR_ADDR}"));
    assert_eq!(form.note, "Mine");
}

/// While its answer is awaited the note has left the form (BUG-92): the last look still
/// says it came from the link.
#[test]
fn the_last_look_says_from_the_link_while_the_answer_is_awaited() {
    let mut form = SendForm::empty();
    form.take_link(&format!("dash:{QR_ADDR}?label=First"));
    form.signed_note = std::mem::take(&mut form.note);
    form.signing = Some(std::time::Instant::now());
    assert!(form.note_from_link());
}

/// The amount follows the note's rule: the next code takes the last link's amount away, unless
/// the person changed it; a code that brings nothing changes nothing.
#[test]
fn a_new_code_takes_the_last_links_amount_but_not_the_persons() {
    use crate::qr_scan::{Kind, Scanned};
    let link = format!("dash:{QR_ADDR}?amount=0.25");
    let address = Scanned::Found {
        text: QR_ADDR.into(),
        kind: Kind::Address,
    };
    let name = Scanned::Found {
        text: "@bob".into(),
        kind: Kind::Name,
    };
    let bare = Scanned::Found {
        text: format!("dash:{QR_ADDR}"),
        kind: Kind::Uri,
    };
    for next in [&address, &name, &bare] {
        let mut form = SendForm::empty();
        form.take_link(&link);
        assert!(!form.amount.is_empty());
        form.take_scan(next);
        assert!(form.amount.is_empty(), "{next:?}");
        assert!(form.link_amount.is_none());
    }
    // A pasted link, no amount in it.
    let mut form = SendForm::empty();
    form.take_link(&link);
    assert!(form.take_link(&format!("dash:{QR_ADDR}")));
    assert!(form.amount.is_empty());
    // A new amount replaces it.
    form.take_link(&link);
    form.take_link(&format!("dash:{QR_ADDR}?amount=1"));
    assert_eq!(crate::format::parse_amount(&form.amount), Some(100_000_000));
    // Changed by hand, or «Max» taken: the person's.
    form.take_link(&link);
    form.amount = "3".into();
    form.take_scan(&address);
    assert_eq!(form.amount, "3");
    form.take_link(&link);
    form.max = true;
    form.take_scan(&address);
    assert!(!form.amount.is_empty());
    // What was typed before any link stays too.
    let mut form = SendForm::empty();
    form.amount = "2".into();
    form.take_scan(&address);
    assert_eq!(form.amount, "2");
}

/// An error, no code, or a refused link change nothing, and the next good code still knows
/// what the last link brought.
#[test]
fn a_code_that_brings_nothing_leaves_what_the_link_brought() {
    use crate::qr_scan::{Kind, Scanned};
    let mut form = SendForm::empty();
    form.take_link(&format!("dash:{QR_ADDR}?amount=0.25&label=First"));
    let amount = form.amount.clone();
    for nothing in [
        Scanned::NoQr,
        Scanned::NotDash,
        Scanned::NotAPicture,
        Scanned::Cancelled,
        Scanned::Found {
            text: "dash-key:abc".into(),
            kind: Kind::AppLink,
        },
    ] {
        form.take_scan(&nothing);
        assert_eq!(
            (form.amount.as_str(), form.note.as_str()),
            (amount.as_str(), "First")
        );
    }
    assert!(form.take_link(&format!("dash:{QR_ADDR}?amount=1e3")));
    assert_eq!(
        (form.amount.as_str(), form.note.as_str()),
        (amount.as_str(), "First")
    );
    assert!(form.link_amount.is_some() && form.link_note.is_some());
}

/// A wallet locked or switched during a scan (the app resets the form either way): the scan
/// goes with the form.
#[test]
fn a_lock_or_a_switch_during_a_scan_drops_it() {
    use crate::qr_scan::Picture;
    let ctx = egui::Context::default();
    let mut form = SendForm::empty();
    form.scanner.start(
        &ctx,
        Picture::Bytes(crate::qr_scan::test_png(QR_ADDR).into()),
    );
    assert!(form.scanner.busy());
    form.on_lock();
    assert!(!form.scanner.busy());
    assert_eq!(form.scanner.poll(), None);
}

/// The pool's «From» card with `duffs`, read in this run or not.
fn pool_source(duffs: Duffs, read: bool) -> SourceView {
    SourceView {
        source: Source::Pool,
        icon: "shielded",
        title: "Private pool",
        duffs,
        fee: 114_140,
        max_fee: 114_140,
        withdrawal: None,
        read,
        note_icon: "",
        note: "",
    }
}

/// BUG-113: the pool not read yet is no 0 on its card. With nothing kept it is not known; with
/// an amount kept, that amount, dimmed; read, as it is.
#[test]
fn an_unread_pool_is_not_a_zero_on_its_card() {
    let unknown = pool_source(0, false);
    assert_eq!(source_line(&unknown, None, None), "Not known yet");
    let kept = pool_source(931_274, false);
    assert_eq!(
        source_line(&kept, None, None),
        crate::format::balance(931_274)
    );
    assert!(source_dim(&kept));
    let read = pool_source(0, true);
    assert_eq!(source_line(&read, None, None), crate::format::balance(0));
    assert!(!source_dim(&read));
    assert!(!source_dim(&unknown), "nothing to dim");
}

/// BUG-113: one line under the sources says the pool's amount is the last known one, after
/// Core's when that is due too; nothing when every amount on the cards is read.
#[test]
fn the_caption_says_the_pool_is_the_last_known_balance() {
    use evolite_api::CoreAsOf;
    const POOL: &str = "The pool: the last known balance, still checking";
    let kept = [pool_source(931_274, false)];
    assert_eq!(sources_caption(None, None, &kept).as_deref(), Some(POOL));
    let both = sources_caption(Some(CoreAsOf::Checked(1)), Some("5 min ago"), &kept);
    assert_eq!(both, Some(format!("Core · 5 min ago · {POOL}")));
    assert_eq!(sources_caption(None, None, &[pool_source(0, false)]), None);
    assert_eq!(
        sources_caption(None, None, &[pool_source(931_274, true)]),
        None
    );
}

/// BUG-113: an amount over a balance not read yet is not called too much; read, it is.
#[test]
fn an_unread_balance_is_no_reason_to_refuse_an_amount() {
    assert_eq!(form_problem("5", &pool_source(0, false), true), None);
    assert_eq!(
        form_problem("0.000001", &pool_source(931_274, false), true),
        None
    );
    assert_eq!(
        form_problem("5", &pool_source(0, true), true),
        Some(AmountProblem::MoreThanBalance)
    );
    assert_eq!(
        form_problem("abc", &pool_source(0, false), true),
        Some(AmountProblem::NotAnAmount),
        "what is wrong with the words themselves still is"
    );
}

/// BUG-113: «Max» of a balance not read yet fills in nothing; read, it follows the balance.
#[test]
fn max_waits_for_a_balance_to_be_read() {
    let mut form = SendForm::empty();
    form.max = true;
    form.follow_source(&pool_source(931_274, false));
    assert!(form.amount.is_empty(), "{}", form.amount);
    form.follow_source(&pool_source(931_274, true));
    assert!(!form.amount.is_empty());
}

/// The credits' «From» card with `duffs`, checked in this run or not.
fn credits_source(duffs: Duffs, read: bool) -> SourceView {
    SourceView {
        source: Source::Credits,
        title: "Credits",
        icon: "credits",
        ..pool_source(duffs, read)
    }
}

/// BUG-115: credits kept from an earlier run, not checked in this one, are the pool's case: the
/// amount dimmed and said under the row, with the pool's in one line when both are.
#[test]
fn credits_kept_from_before_are_dim_and_said_under_the_row() {
    use evolite_api::CoreAsOf;
    let kept = credits_source(47_500_250, false);
    assert_eq!(
        source_line(&kept, None, None),
        crate::format::balance(47_500_250)
    );
    assert!(source_dim(&kept));
    assert_eq!(
        source_line(&credits_source(0, false), None, None),
        "Not known yet"
    );
    assert_eq!(
        sources_caption(None, None, &[kept]).as_deref(),
        Some("Credits: the last known balance, still checking")
    );
    let both = [
        credits_source(47_500_250, false),
        pool_source(931_274, false),
    ];
    assert_eq!(
        sources_caption(Some(CoreAsOf::Checked(1)), Some("5 min ago"), &both),
        Some(
            "Core · 5 min ago · Credits and the pool: the last known balance, still checking"
                .into()
        )
    );
}

/// FEAT-0015 review r1, Must 1: on mainnet «Platform address» is no source until its payments
/// are proven live on testnet; the three others stand as before, in one row.
#[test]
fn mainnet_has_no_platform_address_source() {
    use evolite_api::Network;
    let all = [
        Source::Core,
        Source::Credits,
        Source::Pool,
        Source::PlatformAddress,
    ];
    let on =
        |network| -> Vec<Source> { all.into_iter().filter(|s| shown_on(network, *s)).collect() };
    assert_eq!(on(Network::Testnet), all);
    assert_eq!(
        on(Network::Mainnet),
        [Source::Core, Source::Credits, Source::Pool]
    );
    // Three cards in one row of equal columns at the form's width, as before FEAT-0015.
    let card = CardWidths {
        words: 120.0,
        with_icon: 148.0,
    };
    let columns = from_layout(474.0, &[card; 3]);
    assert!(columns.iter().all(|c| c.width == columns[0].width));
    assert!(3.0 * columns[0].width + 16.0 <= 474.0, "one row of three");
}

/// BUG-112: from the pool the line under the form says what the network sees, and that depends on
/// the recipient: nothing before one is typed or for an @name (the pool cannot pay one), hidden
/// only for a private address, the amount seen for a Platform address, a withdrawal for Core.
#[test]
fn from_the_pool_the_line_says_only_what_the_recipient_keeps_hidden() {
    const PRIVATE: &str =
        "Private address: the amount and the recipient will not be visible on the network.";
    const UNSHIELD: &str = "The unshielded amount becomes visible on the network, like any transfer to a Platform address.";
    let pool = pool_source(1_000_000, true);
    assert_eq!(route_line(&pool, ""), None);
    assert_eq!(route_line(&pool, "@alice"), None);
    let private = format!("tdash1z{}", "q".repeat(76));
    assert_eq!(pool_route(&private), Some(PoolRoute::Private));
    assert_eq!(route_line(&pool, &private), Some(("shielded", PRIVATE)));
    let platform = "tdash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq";
    assert_eq!(pool_route(platform), Some(PoolRoute::Platform));
    assert_eq!(route_line(&pool, platform), Some(("eye", UNSHIELD)));
    let core = route_line(&pool, "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f");
    assert!(
        core.is_some_and(|(_, t)| t.starts_with("Withdrawal from the pool to Core")),
        "{core:?}"
    );
    assert_eq!(pool_route("@alice"), Some(PoolRoute::Name));
}

/// QA pass 21 (6): a payment made with «Max» leaves no amount behind — not filled again from
/// the balance while its answer comes, nor after it; a refusal gives the amount and «Max» back.
#[test]
fn a_payment_with_max_leaves_no_amount_behind() {
    let mut form = SendForm::empty();
    form.max = true;
    form.amount = "0.19384471".into();
    form.recipient = "yX".into();
    form.take_for_signing("yX");
    form.follow_max(30_000_000, 226, None);
    assert!(form.amount.is_empty(), "refilled while the answer comes");
    form.on_sent(form.last_payment, &sent());
    form.follow_max(10_000_000, 226, None);
    assert!(form.amount.is_empty() && !form.max, "{:?}", form.amount);
    // Refused: what was signed is back, «Max» too.
    form.max = true;
    form.amount = "0.1".into();
    form.recipient = "yX".into();
    form.take_for_signing("yX");
    form.on_sent(form.last_payment, &Err(CoreError::NotEnough));
    assert_eq!(form.amount, "0.1");
    assert!(form.max);
}
