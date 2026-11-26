//! FEAT-0052: the days of mixing as History and Overview put them together.

use super::*;
use chrono::{FixedOffset, Utc};
use evolite_api::{Chain, Detail, Flow, layer};

fn line(id: u64, kind: ActivityKind, at: Timestamp, amount: Duffs, proof: Proof) -> Activity {
    Activity {
        id,
        kind,
        chain: Chain::Core,
        at,
        amount,
        proof,
        txid: Some(format!("{id:016x}{}", "ab".repeat(24))),
        fee: (amount < 0).then_some(-amount),
        block: None,
        address: None,
        layers: layer::MIXED,
        flow: Flow::Between,
        key: String::new(),
        detail: None,
        note: None,
        rate: None,
        tax_transfer: false,
    }
}

const ROUND: ActivityKind = ActivityKind::Mixing { fee: 0 };

/// 2026-11-25 00:00 UTC.
const DAY: Timestamp = 1_764_028_800;
const HOUR: Timestamp = 3_600;

fn all(activity: &[Activity]) -> Vec<usize> {
    (0..activity.len()).collect()
}

/// A1, R-1, R-10: a day's five rounds and its preparation are one line where the latest of them
/// stands; the payment from mixed coins and a payment in that day stay lines of their own; one
/// more round that day moves the line up, it makes no second.
#[test]
fn a_day_of_mixing_is_one_line_where_its_latest_stands() {
    let paid = ActivityKind::Sent { to: "yZr4".into() };
    let mut activity = vec![
        line(1, paid, DAY + 13 * HOUR, -10_002_200, Proof::ChainLock),
        line(2, ROUND, DAY + 12 * HOUR, 0, Proof::ChainLock),
        line(3, ROUND, DAY + 11 * HOUR, 0, Proof::ChainLock),
        line(
            4,
            ActivityKind::Received { from: None },
            DAY + 11 * HOUR - 60,
            5_000,
            Proof::ChainLock,
        ),
        line(
            5,
            ActivityKind::Mixing { fee: 10_000 },
            DAY + 10 * HOUR,
            -10_000,
            Proof::ChainLock,
        ),
        line(6, ROUND, DAY + 9 * HOUR, 0, Proof::ChainLock),
        line(7, ROUND, DAY + 8 * HOUR, 0, Proof::ChainLock),
        line(
            8,
            ActivityKind::IntoMixing,
            DAY + 7 * HOUR,
            -374,
            Proof::ChainLock,
        ),
    ];
    assert_eq!(
        lines(&activity, &all(&activity), &Utc),
        vec![
            Line::One(0),
            Line::Day(vec![1, 2, 4, 5, 6, 7]),
            Line::One(3),
        ]
    );
    activity.insert(0, line(9, ROUND, DAY + 14 * HOUR, 0, Proof::Mempool));
    assert_eq!(
        lines(&activity, &all(&activity), &Utc),
        vec![
            Line::Day(vec![0, 2, 3, 5, 6, 7, 8]),
            Line::One(1),
            Line::One(4),
        ],
        "one line still, now first"
    );
}

/// R-10 where the list is not by time: the line stands where the latest of the day is, not where
/// its first one is.
#[test]
fn the_line_stands_at_the_latest_even_out_of_order() {
    let activity = vec![
        line(1, ROUND, DAY + 8 * HOUR, 0, Proof::ChainLock),
        line(
            2,
            ActivityKind::Received { from: None },
            DAY + 9 * HOUR,
            5_000,
            Proof::ChainLock,
        ),
        line(3, ROUND, DAY + 10 * HOUR, 0, Proof::ChainLock),
    ];
    assert_eq!(
        lines(&activity, &all(&activity), &Utc),
        vec![Line::One(1), Line::Day(vec![0, 2])]
    );
}

/// A3: two days of mixing are two lines, split at the local midnight — the same two rounds are
/// one day by UTC and two three hours east of it.
#[test]
fn the_day_ends_at_local_midnight() {
    let activity = vec![
        line(1, ROUND, DAY + 21 * HOUR + 60, 0, Proof::ChainLock),
        line(2, ROUND, DAY + 21 * HOUR - 60, 0, Proof::ChainLock),
    ];
    let east = FixedOffset::east_opt(3 * 3_600).expect("an offset");
    assert_eq!(
        lines(&activity, &all(&activity), &east),
        vec![Line::Day(vec![0]), Line::Day(vec![1])]
    );
    assert_eq!(
        lines(&activity, &all(&activity), &Utc),
        vec![Line::Day(vec![0, 1])]
    );
    // One with no time is none of them (BUG-47).
    let undated = vec![line(3, ROUND, 0, 0, Proof::ChainLock), activity[0].clone()];
    assert_eq!(
        lines(&undated, &all(&undated), &Utc),
        vec![Line::Day(vec![0]), Line::Day(vec![1])]
    );
}

/// The lines a filter left are the ones put together: «Mixing» under a type, the rest as shown.
#[test]
fn only_the_shown_lines_are_put_together() {
    let activity = vec![
        line(1, ROUND, DAY + 12 * HOUR, 0, Proof::ChainLock),
        line(2, ROUND, DAY + 11 * HOUR, 0, Proof::Mempool),
        line(3, ROUND, DAY + 10 * HOUR, 0, Proof::ChainLock),
    ];
    assert_eq!(
        lines(&activity, &[1], &Utc),
        vec![Line::Day(vec![1])],
        "«Pending» keeps the pending one only"
    );
}

/// A4, R-3: the day is pending while one of its operations is; the seal only when all are
/// checked; one only in a block makes it done, without the seal.
#[test]
fn a_day_is_pending_while_one_is_and_sealed_only_when_all_are_checked() {
    let of = |proofs: &[Proof]| {
        let day: Vec<Activity> = proofs
            .iter()
            .enumerate()
            .map(|(n, p)| line(n as u64, ROUND, DAY + n as Timestamp, 0, *p))
            .collect();
        summary(&day.iter().collect::<Vec<_>>())
            .expect("a day")
            .proof
    };
    use Proof::*;
    assert_eq!(of(&[ChainLock, Mempool, InstantLock]), Mempool);
    assert_eq!(of(&[ChainLock, Unconfirmed]), Unconfirmed);
    // r1 Should: «Not confirmed» is the day's whichever of the two pending ones comes first.
    assert_eq!(of(&[Mempool, Unconfirmed]), Unconfirmed);
    assert_eq!(of(&[Unconfirmed, Sending]), Unconfirmed);
    assert_eq!(of(&[ChainLock, InstantLock, ChainLock]), ChainLock);
    assert!(text::checked(of(&[ChainLock, InstantLock])));
    let in_block = of(&[ChainLock, InBlock]);
    assert_eq!(in_block, InBlock);
    assert!(!text::checked(in_block), "no seal");
    assert_eq!(
        crate::screens::activity_filters::status_of(&line(0, ROUND, 1, 0, in_block)),
        crate::screens::activity_filters::Status::Confirmed
    );
}

/// R-4, UC-2: the fees are what the day took from the balance — a collateral and the
/// preparation's fee; the rounds and the preparations counted; the coins prepared and their sum
/// from the preparation's own outputs, and unknown — not 0 — when its details were not kept.
#[test]
fn the_day_says_its_fees_rounds_and_coins_prepared() {
    let mut prep = line(1, ActivityKind::IntoMixing, DAY, -374, Proof::ChainLock);
    prep.detail = Some(Box::new(Detail {
        own_to: vec![("a".into(), 10_000_100), ("b".into(), 10_000_100)],
        ..Detail::default()
    }));
    let day = [
        line(3, ROUND, DAY + 2, 0, Proof::ChainLock),
        line(
            2,
            ActivityKind::Mixing { fee: 10_000 },
            DAY + 1,
            -10_000,
            Proof::ChainLock,
        ),
        prep.clone(),
    ];
    let s = summary(&day.iter().collect::<Vec<_>>()).expect("a day");
    assert_eq!(
        (s.count, s.rounds, s.preparations, s.change, s.latest),
        (3, 2, 1, -10_374, DAY + 2)
    );
    assert_eq!(s.prepared, Some((2, 20_000_200)));
    prep.detail = None;
    let s = summary(&[&day[0], &prep]).expect("a day");
    assert_eq!(s.prepared, None, "unknown");
    let rounds = summary(&[&day[0]]).expect("a day");
    assert_eq!((rounds.change, rounds.prepared), (0, Some((0, 0))));
    assert_eq!(amount(&rounds), "no fees");
    assert!(summary(&[]).is_none());
}

/// The designer (2026-11-26): a round that took nothing says «no fee», not «0.0000 DASH»; one
/// that took a collateral and the preparation say what they took.
#[test]
fn a_round_with_nothing_taken_says_no_fee() {
    assert_eq!(
        member_amount(&line(1, ROUND, DAY, 0, Proof::ChainLock)),
        None
    );
    let collateral = line(
        2,
        ActivityKind::Mixing { fee: 10_000 },
        DAY,
        -10_000,
        Proof::ChainLock,
    );
    assert_eq!(
        member_amount(&collateral),
        Some(text::row_amount(&collateral))
    );
    let prep = line(3, ActivityKind::IntoMixing, DAY, -374, Proof::ChainLock);
    assert!(member_amount(&prep).is_some());
}

/// R-5: a round and the preparation are «Mixing» under «Type» — not «Transfer to yourself», so
/// that filter shows no day of mixing.
#[test]
fn mixing_is_a_type_of_its_own() {
    use crate::screens::activity_filters::{kind, kind_of};
    for k in [ROUND, ActivityKind::IntoMixing] {
        assert_eq!(kind_of(&line(1, k, DAY, 0, Proof::ChainLock)), kind::MIXING);
    }
    let paid = line(
        2,
        ActivityKind::Sent { to: "y".into() },
        DAY,
        -1,
        Proof::ChainLock,
    );
    assert_eq!(kind_of(&paid), kind::PAYMENTS, "a payment from mixed coins");
    assert!(
        kind::ALL.iter().any(|(bit, _)| *bit == kind::MIXING),
        "in the panel"
    );
}

/// R-8, R-9: the words by mode, and the line's one name.
#[test]
fn the_words_follow_the_mode_and_the_name_is_one_sentence() {
    let now = Local::now();
    let day = [
        line(1, ROUND, now.timestamp() - 60, 0, Proof::ChainLock),
        line(
            2,
            ActivityKind::IntoMixing,
            now.timestamp() - 120,
            -374,
            Proof::ChainLock,
        ),
    ];
    let s = summary(&day.iter().collect::<Vec<_>>()).expect("a day");
    text::set_advanced(false);
    assert_eq!(title(), "Mixing coins");
    assert_eq!(member_kind(&day[0]), "Round");
    assert_eq!(member_kind(&day[1]), "Preparing coins");
    text::set_advanced(true);
    assert_eq!(title(), "Mixing (CoinJoin)");
    assert_eq!(member_kind(&day[1]), "Into mixing");
    let name = spoken_name(&s, now);
    text::set_advanced(false);
    assert!(name.starts_with("Mixing, 2 operations, "), "{name}");
    let fees = format!(", fees {}, ", text::signed_amount(-374));
    assert!(name.contains(&fees), "{name}");
    assert!(name.ends_with(", cryptographically checked"), "{name}");
    assert!(dated(&s, now).starts_with("2 operations · "));
    let rounds = summary(&[&day[0]]).expect("a day");
    let name = spoken_name(&rounds, now);
    assert!(name.contains(", no fees, "), "{name}");
    assert!(dated(&rounds, now).starts_with("1 operation · "));
}

#[cfg(feature = "dev-tools")]
/// R-7, A6: the CSV stays a line a transaction — the day's preparation and collateral each its
/// own line, the payment from mixed coins its own; the rounds with nothing spent are no line, as
/// before (none of the day's lines is merged).
#[test]
fn the_csv_keeps_a_line_a_transaction() {
    let day = example_day(Local::now());
    let csv = crate::csv_export::taxbit(&day, &crate::csv_export::Own::default());
    assert_eq!(csv.lines().count() - 1, 3, "{csv}");
    assert_eq!(
        crate::csv_export::lines(&day, &crate::csv_export::Own::default()),
        3
    );
}

/// The example day (the presets' and the UI tests'): six of CoinJoin's operations and a payment
/// from mixed coins, all yesterday, one line and one apart; in a history once, by its time.
#[cfg(feature = "dev-tools")]
#[test]
fn the_example_day_is_one_line_and_a_payment() {
    let now = Local::now();
    let day = example_day(now);
    assert_eq!(day.iter().filter(|a| is_mixing(a)).count(), 6);
    assert_eq!(
        lines(&day, &all(&day), &Local),
        vec![Line::One(0), Line::Day(vec![1, 2, 3, 4, 5, 6])]
    );
    let t = now.timestamp();
    let paid = || ActivityKind::Received { from: None };
    let mut history = vec![
        line(1, paid(), t, 1, Proof::Mempool),
        line(2, paid(), t - 3 * 86_400, 1, Proof::ChainLock),
        line(3, paid(), 0, 1, Proof::Proved),
    ];
    with_example_day(&mut history, now);
    with_example_day(&mut history, now);
    assert_eq!(history.len(), 3 + 7, "once");
    let ids: Vec<u64> = history.iter().map(|a| a.id).collect();
    assert_eq!(ids[0], 1);
    assert_eq!(&ids[8..], &[2, 3], "the older and the undated after it");
    let found = lines(&history, &all(&history), &Local);
    assert!(
        found.contains(&Line::Day((2..8).collect())),
        "the day one line: {found:?}"
    );
}

/// R-11, AC-A10: the line says the local times of its first and latest operation — one time for
/// one, or for all in one minute; the count alone with no time; the date is in the card's words.
#[test]
fn the_line_says_its_times_and_the_card_its_date() {
    let at = |h: u32, m: u32, sec: u32| {
        Local
            .from_local_datetime(
                &chrono::NaiveDate::from_ymd_opt(2026, 11, 25)
                    .and_then(|d| d.and_hms_opt(h, m, sec))
                    .expect("a time"),
            )
            .earliest()
            .expect("a local time")
            .timestamp()
    };
    let of = |times: &[Timestamp]| {
        let day: Vec<Activity> = times
            .iter()
            .enumerate()
            .map(|(n, t)| line(n as u64, ROUND, *t, 0, Proof::ChainLock))
            .collect();
        summary(&day.iter().collect::<Vec<_>>()).expect("a day")
    };
    assert_eq!(
        detail(&of(&[at(12, 5, 0), at(11, 0, 0), at(10, 0, 30)]), None),
        "3 operations · 10:00–12:05"
    );
    assert_eq!(detail(&of(&[at(10, 0, 0)]), None), "1 operation · 10:00");
    assert_eq!(
        detail(&of(&[at(10, 0, 50), at(10, 0, 5)]), None),
        "2 operations · 10:00",
        "one minute"
    );
    assert_eq!(
        detail(&of(&[0, 0]), None),
        "2 operations",
        "no time (BUG-47)"
    );
    // On the Overview the day as the rows beside it say it (R-11 r2).
    let day = of(&[at(12, 5, 0), at(10, 0, 0)]);
    let on = |d: u32, h: u32| {
        Local
            .from_local_datetime(
                &chrono::NaiveDate::from_ymd_opt(2026, 11, d)
                    .and_then(|x| x.and_hms_opt(h, 0, 0))
                    .expect("a time"),
            )
            .earliest()
            .expect("now")
    };
    assert_eq!(
        detail(&day, Some(on(25, 13))),
        "2 operations · today, 10:00–12:05"
    );
    assert_eq!(
        detail(&day, Some(on(26, 9))),
        "2 operations · yesterday, 10:00–12:05"
    );
    let older = detail(&day, Some(on(28, 9)));
    let date = text::date_short(at(12, 5, 0), on(28, 9));
    assert_eq!(older, format!("2 operations · {date}, 10:00–12:05"));
    assert_eq!(
        detail(&of(&[at(10, 0, 0)]), Some(on(26, 9))),
        "1 operation · yesterday, 10:00"
    );
    assert_eq!(detail(&of(&[0]), Some(on(26, 9))), "1 operation", "no time");
    let now = Local.timestamp_opt(at(13, 0, 0), 0).single().expect("now");
    assert!(
        dated(&of(&[at(10, 0, 0)]), now).ends_with(&text::date_day(at(10, 0, 0), now)),
        "the date in the card"
    );
}
