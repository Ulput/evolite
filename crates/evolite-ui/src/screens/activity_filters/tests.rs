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
        note: None,
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
    for (pill, lines) in [(1, vec![0, 1, 2, 4]), (4, vec![3, 4]), (3, vec![])] {
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
    use layer::{CORE, IDENTITY, PLATFORM_ADDRESS, POOL};
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
    ];
    assert_eq!(LAYERS.len(), 5, "a pill added: give it its lines here");
    for (pill, lines) in [
        (0, vec![0, 1, 2, 3, 4, 5, 6]),
        (1, vec![0, 3, 4]),
        (2, vec![1, 2, 3, 4, 5]),
        (3, vec![5, 6]),
        (4, vec![1, 3, 5]),
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
