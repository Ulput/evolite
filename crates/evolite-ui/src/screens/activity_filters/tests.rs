use super::*;
use chrono::FixedOffset;
use evolite_api::{Chain, Duffs};

fn line(
    kind: ActivityKind,
    at: i64,
    amount: Duffs,
    proof: Proof,
    layers: u8,
    flow: Flow,
) -> Activity {
    Activity {
        id: at as u64,
        kind,
        chain: Chain::Core,
        at,
        amount,
        proof,
        txid: None,
        fee: None,
        block: None,
        address: None,
        layers,
        flow,
        key: String::new(),
        detail: None,
        note: None,
        rate: None,
        tax_transfer: false,
    }
}

fn received(at: i64, amount: Duffs) -> Activity {
    line(
        ActivityKind::Received { from: None },
        at,
        amount,
        Proof::ChainLock,
        layer::CORE,
        Flow::In,
    )
}

/// 2026-10-31 23:30 in UTC+3.
fn now() -> DateTime<FixedOffset> {
    FixedOffset::east_opt(3 * 3600)
        .unwrap()
        .with_ymd_and_hms(2026, 10, 31, 23, 30, 0)
        .unwrap()
}

/// Seconds of a moment in UTC+3.
fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
    FixedOffset::east_opt(3 * 3600)
        .unwrap()
        .with_ymd_and_hms(y, m, d, h, min, 0)
        .unwrap()
        .timestamp()
}

/// AC-T3: every proof and every state of a lock's credits.
#[test]
fn each_proof_has_its_status() {
    let with = |proof| {
        line(
            ActivityKind::Sent { to: "y".into() },
            1,
            -1,
            proof,
            1,
            Flow::Out,
        )
    };
    for (proof, status) in [
        (Proof::Mempool, Status::Pending),
        (Proof::Sending, Status::Pending),
        (Proof::Unconfirmed, Status::Pending),
        (Proof::InstantLock, Status::Confirmed),
        (Proof::InBlock, Status::Confirmed),
        (Proof::ChainLock, Status::Final),
        (Proof::Proved, Status::Final),
    ] {
        assert_eq!(status_of(&with(proof)), status, "{proof:?}");
    }
    let lock = |credits| ActivityKind::TopUp {
        identity: "@a".into(),
        creates: false,
        credits,
    };
    let of = |credits, proof| status_of(&line(lock(credits), 1, -1, proof, 3, Flow::Between));
    assert_eq!(of(Credits::NotAdded, Proof::ChainLock), Status::Pending);
    assert_eq!(
        of(Credits::Unknown, Proof::ChainLock),
        Status::Final,
        "no news is not waiting"
    );
    assert_eq!(of(Credits::Unknown, Proof::InBlock), Status::Confirmed);
    assert_eq!(of(Credits::Added, Proof::ChainLock), Status::Final);
}

/// AC-T5: days from midnight to midnight of the computer's zone, «to» included; a custom period
/// either way round; a line with no time only in «All dates».
#[test]
fn a_period_counts_whole_days_of_the_local_zone() {
    let now = now();
    let late = at(2026, 10, 31, 0, 0);
    let before = at(2026, 10, 30, 23, 59);
    assert!(in_period(Period::Today, late, &now));
    assert!(!in_period(Period::Today, before, &now));
    // 7 days: 25 October to today.
    assert!(in_period(Period::Days(7), at(2026, 10, 25, 0, 0), &now));
    assert!(!in_period(Period::Days(7), at(2026, 10, 24, 23, 59), &now));
    assert!(in_period(Period::ThisYear, at(2026, 1, 1, 0, 0), &now));
    assert!(!in_period(Period::ThisYear, at(2025, 12, 31, 23, 59), &now));
    let (oct1, oct31) = (
        NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
    );
    for custom in [Period::Custom(oct1, oct31), Period::Custom(oct31, oct1)] {
        assert!(
            in_period(custom, at(2026, 10, 31, 23, 59), &now),
            "«to» is in"
        );
        assert!(in_period(custom, at(2026, 10, 1, 0, 0), &now));
        assert!(!in_period(custom, at(2026, 9, 30, 23, 59), &now));
    }
    // The same moment is another day in UTC: the zone of `now` counts.
    let utc = now.with_timezone(&FixedOffset::east_opt(0).unwrap());
    assert!(!in_period(Period::Today, late, &utc));
    for p in [
        Period::Today,
        Period::Days(30),
        Period::ThisYear,
        Period::Custom(oct1, oct31),
    ] {
        assert!(!in_period(p, 0, &now), "no time: {p:?}");
    }
    assert!(in_period(Period::All, 0, &now));
}

/// AC-T6: `0.2` and `0,2` find «+0,2000» whatever the number format.
#[test]
fn an_amount_is_found_with_a_comma_or_a_dot() {
    use crate::format::{Display, NumberStyle, set_display};
    let act = received(1, 20_000_000);
    let other = received(1, 30_000_000);
    for style in [
        NumberStyle::Language,
        NumberStyle::SpaceComma,
        NumberStyle::CommaDot,
    ] {
        set_display(Display {
            style,
            ..Display::default()
        });
        for q in ["0.2", "0,2", "0.20"] {
            assert!(matches_query(&act, q, &[]), "{q} in {style:?}");
            assert!(!matches_query(&other, q, &[]), "{q} in {style:?}");
        }
    }
    set_display(Display::default());
}

/// AC-T4, R-3: between groups all, within «Type» any; AC-A5 «30 days» + «In» + `0.2`.
#[test]
fn the_filters_go_together() {
    let now = now();
    let recent = at(2026, 10, 20, 12, 0);
    let history = [
        received(recent, 20_000_000),
        received(at(2026, 9, 1, 12, 0), 20_000_000),
        line(
            ActivityKind::Sent { to: "y".into() },
            recent,
            -20_000_000,
            Proof::ChainLock,
            layer::CORE,
            Flow::Out,
        ),
        line(
            ActivityKind::NameRegistered { label: "a".into() },
            recent,
            -1,
            Proof::Proved,
            layer::IDENTITY,
            Flow::Out,
        ),
        line(
            ActivityKind::TopUp {
                identity: "@a".into(),
                creates: false,
                credits: Credits::NotAdded,
            },
            recent,
            -30_000_000,
            Proof::ChainLock,
            layer::CORE | layer::IDENTITY,
            Flow::Between,
        ),
    ];
    let kept = |f: &Filters, q: &str| -> Vec<usize> {
        (0..history.len())
            .filter(|i| keep(f, &history[*i], q, &[], &now))
            .collect()
    };
    let a5 = Filters {
        flow: Some(Flow::In),
        period: Period::Days(30),
        ..Filters::default()
    };
    assert_eq!(kept(&a5, "0.2"), [0]);
    assert_eq!(kept(&Filters::default(), ""), [0, 1, 2, 3, 4]);
    // Within «Type»: either.
    let types = Filters {
        kinds: kind::NAMES | kind::IDENTITY,
        ..Filters::default()
    };
    assert_eq!(kept(&types, ""), [3, 4]);
    // The layer pill by the money moved: the lock is in Core and in Identity.
    for (pill, lines) in [(1, vec![0, 1, 2, 4]), (5, vec![3, 4]), (4, vec![])] {
        let f = Filters {
            layer: pill,
            ..Filters::default()
        };
        assert_eq!(kept(&f, ""), lines, "pill {pill}");
    }
    // UC-8: nothing under «Pending» + «Names».
    let none = Filters {
        status: Some(Status::Pending),
        kinds: kind::NAMES,
        ..Filters::default()
    };
    assert!(kept(&none, "").is_empty());
    assert_eq!(none.chips().len(), 2);
}

/// Review r1 should 1: each layer pill keeps a line that moves any of its balances, and
/// «Platform» is identities and Platform addresses both (the coordinator's decision): a
/// contact request only touches the identity, a payment to the Platform address only the
/// address. «Identity» leaves the transfer to the Platform address (AC-A2); «Core» keeps a
/// top-up (AC-A1).
#[test]
fn each_layer_pill_keeps_what_moves_any_of_its_balances() {
    use layer::{CORE, IDENTITY, MIXED, PLATFORM_ADDRESS, POOL};
    let now = now();
    let t = at(2026, 10, 20, 12, 0);
    let lock = |identity: &str| ActivityKind::TopUp {
        identity: identity.into(),
        creates: false,
        credits: Credits::Added,
    };
    let history = [
        received(t, 1),
        line(
            ActivityKind::ContactRequest {
                to: "@bob".into(),
                accepting: false,
                ours: None,
            },
            t,
            0,
            Proof::Proved,
            IDENTITY,
            Flow::Out,
        ),
        line(
            ActivityKind::PlatformReceived,
            t,
            1,
            Proof::Proved,
            PLATFORM_ADDRESS,
            Flow::In,
        ),
        line(
            lock("@a"),
            t,
            -1,
            Proof::ChainLock,
            CORE | IDENTITY,
            Flow::Between,
        ),
        // «Transfer to Platform»: a lock to the Platform address.
        line(
            lock(""),
            t,
            -1,
            Proof::ChainLock,
            CORE | PLATFORM_ADDRESS,
            Flow::Between,
        ),
        line(
            ActivityKind::Shielded {
                from: Some("identity credits".into()),
            },
            t,
            -1,
            Proof::Proved,
            IDENTITY | POOL,
            Flow::Between,
        ),
        line(
            ActivityKind::PoolReceived,
            t,
            1,
            Proof::Proved,
            POOL,
            Flow::In,
        ),
        // FEAT-0030: a mixing round stays in «Mixed»; the preparation moves Core into it.
        line(
            ActivityKind::Mixing { fee: 0 },
            t,
            0,
            Proof::ChainLock,
            MIXED,
            Flow::Between,
        ),
        line(
            ActivityKind::IntoMixing,
            t,
            -1,
            Proof::ChainLock,
            CORE | MIXED,
            Flow::Between,
        ),
    ];
    assert_eq!(LAYERS.len(), 7, "a pill added: give it its lines here");
    for (pill, lines) in [
        (0, vec![0, 1, 2, 3, 4, 5, 6, 7, 8]),
        (1, vec![0, 3, 4, 8]),
        (2, vec![7, 8]),
        (3, vec![1, 2, 3, 4, 5]),
        (4, vec![5, 6]),
        (5, vec![1, 3, 5]),
        // FEAT-0019 T-1: «Tokens» keeps only the lines of tokens; none here.
        (6, vec![]),
    ] {
        let f = Filters {
            layer: pill,
            ..Filters::default()
        };
        let kept: Vec<usize> = (0..history.len())
            .filter(|i| keep(&f, &history[*i], "", &[], &now))
            .collect();
        assert_eq!(kept, lines, "pill {pill}");
    }
}

/// The chips: one for each filter on (a type each), the cross takes one, «Reset» all of the
/// panel's and leaves the layer pill.
#[test]
fn a_chip_for_each_filter_on() {
    let mut f = Filters {
        layer: 1,
        flow: Some(Flow::In),
        kinds: kind::PAYMENTS | kind::POOL,
        period: Period::Days(30),
        ..Filters::default()
    };
    assert_eq!(
        f.chips(),
        [
            Chip::Flow(Flow::In),
            Chip::Kind(kind::PAYMENTS),
            Chip::Kind(kind::POOL),
            Chip::Period(Period::Days(30))
        ]
    );
    f.remove(Chip::Kind(kind::PAYMENTS));
    assert_eq!(f.kinds, kind::POOL);
    f.reset();
    assert!(f.chips().is_empty());
    assert_eq!(f.layer, 1);
}

#[test]
fn a_date_is_typed_with_dots() {
    let d = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
    assert_eq!(parse_date("01.10.2026"), Some(d));
    assert_eq!(parse_date(" 1/10/2026 "), Some(d));
    assert_eq!(parse_date("2026.10.01"), None);
    assert_eq!(date_text(d), "01.10.2026");
}

/// AC-T7: the list is worked out once per change, not every frame; 1 000 lines in well under
/// a frame (not strict in debug: the release figure is in the handoff).
#[test]
fn the_list_is_worked_out_only_when_something_changes() {
    let now = now();
    let mut activity: Vec<Activity> = (0..1000)
        .map(|i| received(at(2026, 10, 1, 0, 0) + i * 600, 1_000 + i))
        .collect();
    let mut h = History::default();
    let started = std::time::Instant::now();
    assert_eq!(h.shown(&activity, &[], &now).len(), 1000);
    let first = started.elapsed();
    h.query = "0,00001".into();
    assert_eq!(h.shown(&activity, &[], &now).len(), 1);
    eprintln!("1 000 lines: {first:?}");
    assert!(first.as_millis() < 500, "{first:?}");
    // Other lines under the same snapshot: worked out again.
    activity.truncate(10);
    assert_eq!(h.shown(&activity, &[], &now).len(), 1);
    h.query.clear();
    assert_eq!(h.shown(&activity, &[], &now).len(), 10);
}

/// AC-T8: «Only the shown ones» gives `taxbit` the lines on screen — as many rows as shown,
/// under the same header.
#[test]
fn the_export_of_the_shown_lines_has_as_many_rows() {
    let now = now();
    let activity: Vec<Activity> = (0..6)
        .map(|i| received(at(2026, 10, 1 + i as u32, 12, 0), 1_000_000 * (i + 1)))
        .collect();
    let mut h = History::default();
    h.filters.period = Period::Days(28);
    let shown: Vec<Activity> = h
        .shown(&activity, &[], &now)
        .iter()
        .map(|i| activity[*i].clone())
        .collect();
    assert_eq!(shown.len(), 3, "1 to 3 October are older than 28 days");
    let own = crate::csv_export::Own::default();
    let some = crate::csv_export::taxbit(&shown, &own);
    let all = crate::csv_export::taxbit(&activity, &own);
    assert_eq!(some.lines().count(), shown.len() + 1);
    assert_eq!(some.lines().next(), all.lines().next(), "the same columns");
}

/// FEAT-0056 (the architect's class test, UI half): a payment to three whose second recipient
/// is a contact, as History has it named — found by the contact's name and by any recipient's
/// address, counted among the contact's payments, three rows in CSV, titled by the count.
/// Mutant «only `to`» (the search, the contact's payments) → red.
#[test]
fn a_payment_to_several_is_found_and_counted_by_any_recipient() {
    let mut act = line(
        ActivityKind::Sent {
            to: "yFirst1".into(),
        },
        1_700_000_000,
        -(1_000 + 2_000 + 3_000 + 300),
        Proof::ChainLock,
        layer::CORE,
        Flow::Out,
    );
    act.txid = Some("ab12".into());
    act.fee = Some(300);
    let to = |address: &str, name: Option<&str>, amount| evolite_api::SentTo {
        address: address.into(),
        name: name.map(Into::into),
        amount,
    };
    act.detail = Some(Box::new(evolite_api::Detail {
        sent_to: vec![
            to("yFirst1", None, 1_000),
            to("ySecond2", Some("@carol"), 2_000),
            to("yThird3", None, 3_000),
        ],
        ..Default::default()
    }));
    assert!(
        matches_query(&act, "@carol", &[]),
        "by the second recipient's name"
    );
    assert!(
        matches_query(&act, "ythird3", &[]),
        "by the third's address"
    );
    assert!(!matches_query(&act, "@dave", &[]));
    assert_eq!(crate::text::activity_title(&act), "Sent to 3 recipients");
    assert_eq!(
        crate::screens::contacts::paid_for_tests(std::slice::from_ref(&act), "@carol"),
        1,
        "the contact's payments count it"
    );
    let own = crate::csv_export::Own::default();
    assert_eq!(crate::csv_export::lines([&act], &own), 3);
}

/// FEAT-0057 П7, A7.5 (А-3): an IntoMixing, a CoinJoin round and a payment — at «All» the totals
/// are the wallet's change (only the fees of the moves between its own places go), at «Core»
/// Core's (the whole IntoMixing goes: the mixed coins are not Core's, BUG-121).
#[test]
fn the_totals_are_the_balance_s_change() {
    let mut into = line(
        ActivityKind::IntoMixing,
        1,
        -(1_000 + 10),
        Proof::ChainLock,
        layer::CORE | layer::MIXED,
        Flow::Between,
    );
    into.fee = Some(10);
    let round = line(
        ActivityKind::Mixing { fee: 5 },
        2,
        -5,
        Proof::ChainLock,
        layer::MIXED,
        Flow::Between,
    );
    let paid = line(
        ActivityKind::Received { from: None },
        3,
        2_000,
        Proof::ChainLock,
        layer::CORE,
        Flow::In,
    );
    let all = [&into, &round, &paid];
    let t = totals(&all, 0).expect("DASH");
    assert_eq!((t.came, t.went), (2_000, 15), "the fees only");
    assert_eq!(t.came - t.went, 2_000 - 10 - 5, "the wallet's change");
    let core: Vec<&Activity> = all
        .iter()
        .copied()
        .filter(|a| a.layers & layer::CORE != 0)
        .collect();
    let t = totals(&core, 1).expect("DASH");
    assert_eq!((t.came, t.went), (2_000, 1_010));
    assert_eq!(t.came - t.went, 2_000 - 1_010, "Core's change");
}

/// FEAT-0057 П7, A7.2–A7.3 (R-7.2): a pending line is not added — only counted; a send's fee is
/// in its amount; «Tokens» has no DASH totals.
#[test]
fn pending_lines_are_counted_apart() {
    let sent = line(
        ActivityKind::Sent {
            to: "yPayee".into(),
        },
        1,
        -(3_000 + 226),
        Proof::ChainLock,
        layer::CORE,
        Flow::Out,
    );
    let waiting = line(
        ActivityKind::Received { from: None },
        2,
        12_000,
        Proof::Mempool,
        layer::CORE,
        Flow::In,
    );
    let t = totals(&[&sent, &waiting], 1).expect("DASH");
    assert_eq!(
        t,
        Totals {
            came: 0,
            went: 3_226,
            estimated: 0,
            went_estimated: 0,
            pending: 1,
            counted: 1
        }
    );
    assert_eq!(totals(&[&sent], TOKENS_LAYER), None, "tokens are not DASH");
}

/// The architect on R-7.1: a top-up from Core in the place it came to — «Identity» — adds what
/// Platform credited when the line knows it (exactly); without it, what left Core less the Core
/// fee, counted as an estimate. «Core»: the whole of what left. «All»: the Core fee, and
/// Platform's when what arrived is known.
#[test]
fn a_move_counts_by_the_side_of_the_place_chosen() {
    let top_up = |credited: Option<Duffs>| {
        let mut act = line(
            ActivityKind::TopUp {
                identity: "@me".into(),
                creates: false,
                credits: Credits::Added,
            },
            1,
            -(100_000 + 226),
            Proof::ChainLock,
            layer::CORE | layer::IDENTITY,
            Flow::Between,
        );
        act.fee = Some(226);
        act.detail = Some(Box::new(evolite_api::Detail {
            credits_added: credited,
            ..Default::default()
        }));
        act
    };
    let identity = 5;
    assert_eq!(LAYERS[identity], layer::IDENTITY);
    let known = top_up(Some(99_000));
    let t = totals(&[&known], identity).expect("DASH");
    assert_eq!((t.came, t.estimated), (99_000, 0), "credited: exactly");
    let guessed = top_up(None);
    let t = totals(&[&guessed], identity).expect("DASH");
    assert_eq!(
        (t.came, t.estimated),
        (100_000, 1),
        "the most that came, an estimate"
    );
    let t = totals(&[&guessed], 1).expect("DASH");
    assert_eq!(
        (t.came, t.went, t.estimated),
        (0, 100_226, 0),
        "Core: all that left"
    );
    let t = totals(&[&guessed], 0).expect("DASH");
    assert_eq!((t.came, t.went), (0, 226), "All: the Core fee");
    let t = totals(&[&known], 0).expect("DASH");
    assert_eq!(t.went, 100_226 - 99_000, "All: Core's fee and Platform's");
}

/// FEAT-0057 П7 r1 Must: what is not known is never added silently — a write whose cost is not
/// known (BUG-176) and, at «All», a move whose Core fee is not known count in `went_estimated`.
#[test]
fn what_is_not_known_is_counted_as_an_estimate() {
    let mut write = line(
        ActivityKind::ProfileSaved,
        1,
        0,
        Proof::Proved,
        layer::IDENTITY,
        Flow::Out,
    );
    write.detail = Some(Box::new(evolite_api::Detail {
        cost_unknown: true,
        ..Default::default()
    }));
    let t = totals(&[&write], 0).expect("DASH");
    assert_eq!((t.went, t.went_estimated), (0, 1), "not known: an estimate");
    let to_self = line(
        ActivityKind::SelfTransfer { moved: 0 },
        2,
        -226,
        Proof::ChainLock,
        layer::CORE,
        Flow::Between,
    );
    let t = totals(&[&to_self], 0).expect("DASH");
    assert_eq!((t.went, t.went_estimated), (0, 1), "no fee known: said");
    let mut paid = to_self.clone();
    paid.fee = Some(226);
    let t = totals(&[&paid], 0).expect("DASH");
    assert_eq!((t.went, t.went_estimated), (226, 0), "its fee: exact");
}
