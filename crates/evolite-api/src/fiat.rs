//! Amounts in a currency (FEAT-0008): the rules the interface and the core share, in whole
//! numbers only — no floating point between a typed «25 USD» and the duffs that are signed.

use crate::{Duffs, FiatRate, Timestamp};

/// Duffs in one DASH.
const DUFFS_PER_DASH: i128 = 100_000_000;

/// A rate older than this is not used for a payment or for typing in the currency
/// (analyst-35: «через 5 минут»).
pub const STALE_AFTER: Timestamp = 5 * 60;
/// With «Send» or «Request an amount» open, the interface asks for the rate again when it is
/// older than this, and not more often (architect, 2026-11-07).
pub const ASK_AFTER: Timestamp = 4 * 60;
/// The one switch of the rate asked for by need (FEAT-0008): `false` leaves only the
/// 30-minute round, and [`ask_now`] never asks.
pub const ASK_BY_NEED: bool = true;

/// How far a rate's time may be ahead of the clock that judges it (clocks differ a little).
const CLOCK_SLACK: Timestamp = 60;

/// Whether `rate` may still be used at `now`: a price above zero, at most [`STALE_AFTER`] old.
pub fn fresh(rate: &FiatRate, now: Timestamp) -> bool {
    rate.price > 0 && rate.at <= now + CLOCK_SLACK && now - rate.at <= STALE_AFTER
}

/// The rate a payment keeps (АРХ-2): the one «Summary» showed if it is in the setting's
/// `currency` and fresh at signing; else the core's own under the same rule; else none.
/// With the setting off (`currency` none) there is never one (AC-T1).
pub fn payment_rate(
    shown: Option<FiatRate>,
    own: Option<&FiatRate>,
    currency: Option<&str>,
    now: Timestamp,
) -> Option<FiatRate> {
    let currency = currency?;
    let usable = |r: &FiatRate| r.currency == currency && fresh(r, now);
    shown
        .filter(usable)
        .or_else(|| own.filter(|r| usable(r)).cloned())
}

/// Whether the interface should ask for the rate now (AC-T4): the screen that needs it is open,
/// the rate it has is older than [`ASK_AFTER`] (or there is none), and the last ask was at
/// least [`ASK_AFTER`] ago.
pub fn ask_now(
    open: bool,
    rate_at: Option<Timestamp>,
    last_ask: Option<Timestamp>,
    now: Timestamp,
) -> bool {
    ASK_BY_NEED
        && open
        && rate_at.is_none_or(|at| now - at > ASK_AFTER)
        && last_ask.is_none_or(|asked| now - asked >= ASK_AFTER)
}

/// Duffs for an amount in the currency, rounded down to a duff so as never to pay more
/// (R-3, AC-T2): `minor` is the amount in the currency's smallest unit with `decimals` places
/// (2500 with 2 for 25.00 USD). None when the price is not above zero or the result does not
/// fit: typing in the currency is then unavailable, never a panic.
pub fn duffs_for(minor: i128, decimals: u32, price: i64) -> Option<Duffs> {
    if price <= 0 || minor < 0 || decimals > 8 {
        return None;
    }
    let scaled = minor
        .checked_mul(DUFFS_PER_DASH)?
        .checked_mul(10i128.checked_pow(8 - decimals)?)?;
    Duffs::try_from(scaled / price as i128).ok()
}

/// `duffs` in the currency's hundredths, rounded to the nearest (half away from zero), for the
/// «≈» beside an amount (AC-T2). None on overflow.
pub fn cents_of(duffs: Duffs, price: i64) -> Option<i128> {
    let micro = (duffs as i128).checked_mul(price as i128)? / DUFFS_PER_DASH;
    // `micro` is hundred-millionths of the currency: a cent is 1 000 000 of them.
    let cents = (micro.unsigned_abs() + 500_000) / 1_000_000;
    let cents = i128::try_from(cents).ok()?;
    Some(if micro < 0 { -cents } else { cents })
}

/// Whether `duffs` is worth less than half a cent: shown as «< 0,01» (AC-T2).
pub fn below_a_cent(duffs: Duffs, price: i64) -> bool {
    duffs != 0 && cents_of(duffs, price) == Some(0)
}

/// The keys that get the rate now as operations first seen in this session (R-4, AC-T6).
///
/// - `at_unlock`: the keys of the first snapshot after the unlock; they never get one, so old
///   and restored operations keep none.
/// - `seen`: when each other key was first seen; it is eligible for [`STALE_AFTER`] after that.
/// - `rated`: whether a key has a rate already; it never changes.
///
/// Only when `rate` is fresh at `now`.
pub fn first_seen_rates<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    at_unlock: &std::collections::HashSet<String>,
    seen: &mut std::collections::HashMap<String, Timestamp>,
    rated: impl Fn(&str) -> bool,
    rate: Option<&FiatRate>,
    now: Timestamp,
) -> Vec<String> {
    let mut out = Vec::new();
    for key in keys {
        if key.is_empty() || key.starts_with("pending:") || at_unlock.contains(key) {
            continue;
        }
        let first = *seen.entry(key.to_owned()).or_insert(now);
        if rate.is_some_and(|r| fresh(r, now)) && now - first <= STALE_AFTER && !rated(key) {
            out.push(key.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn usd(price: i64, at: Timestamp) -> FiatRate {
        FiatRate {
            currency: "USD".into(),
            price,
            at,
        }
    }

    #[test]
    fn twenty_five_dollars_at_30_78_are_81_221_572_duffs() {
        assert_eq!(duffs_for(2500, 2, 3_078_000_000), Some(81_221_572));
        // More rates: 1 USD at 1.00 is a DASH; 0.01 at 47.96 rounds down.
        assert_eq!(duffs_for(100, 2, 100_000_000), Some(100_000_000));
        assert_eq!(duffs_for(1, 2, 4_796_000_000), Some(20_850));
        assert_eq!(duffs_for(1_000_000, 2, 9_999_000_000), Some(10_001_000_100));
        // Whole units (0 decimals) also give the same
        assert_eq!(duffs_for(25, 0, 3_078_000_000), Some(81_221_572));
    }

    #[test]
    fn a_price_of_zero_or_an_overflow_makes_no_amount() {
        assert_eq!(duffs_for(2500, 2, 0), None);
        assert_eq!(duffs_for(2500, 2, -5), None);
        assert_eq!(duffs_for(i128::MAX / 2, 2, 3_078_000_000), None);
        assert_eq!(duffs_for(i64::MAX as i128, 2, 1), None);
    }

    #[test]
    fn the_shown_amount_rounds_to_the_nearest_cent() {
        // 0.8123 DASH at 30.78 = 25.0026 → 25.00; 0.81221572 → 24.99999… → 25.00.
        assert_eq!(cents_of(81_230_000, 3_078_000_000), Some(2500));
        assert_eq!(cents_of(81_221_572, 3_078_000_000), Some(2500));
        assert_eq!(cents_of(-81_221_572, 3_078_000_000), Some(-2500));
        // Half a cent rounds up; below it shows «< 0,01».
        assert_eq!(cents_of(16_245, 3_078_000_000), Some(1));
        assert!(below_a_cent(16_000, 3_078_000_000));
        assert!(!below_a_cent(0, 3_078_000_000));
        assert!(!below_a_cent(16_245, 3_078_000_000));
    }

    #[test]
    fn a_payment_keeps_the_shown_rate_only_if_fresh_and_in_the_currency() {
        let now = 10_000;
        let shown = usd(3_078_000_000, now - 60);
        let own = usd(3_100_000_000, now - 10);
        assert_eq!(
            payment_rate(Some(shown.clone()), Some(&own), Some("USD"), now),
            Some(shown.clone())
        );
        // Older than 5 minutes: the core's own.
        let old = usd(3_078_000_000, now - STALE_AFTER - 1);
        assert_eq!(
            payment_rate(Some(old.clone()), Some(&own), Some("USD"), now),
            Some(own.clone())
        );
        // Another currency: the core's own, if it is in the setting's.
        let eur = FiatRate {
            currency: "EUR".into(),
            ..shown.clone()
        };
        assert_eq!(
            payment_rate(Some(eur), Some(&own), Some("USD"), now),
            Some(own.clone())
        );
        // Neither usable: none.
        assert_eq!(
            payment_rate(Some(old.clone()), Some(&old), Some("USD"), now),
            None
        );
        // The setting off: none, whatever is shown.
        assert_eq!(payment_rate(Some(shown), Some(&own), None, now), None);
        // A price of zero is not a rate.
        assert_eq!(
            payment_rate(Some(usd(0, now)), None, Some("USD"), now),
            None
        );
    }

    /// AC-T9: a rate never shows its price or time in `Debug` — of a payment, of a line.
    #[test]
    fn debug_names_the_currency_only() {
        let rate = usd(3_078_123_456, 1_700_000_123);
        let send = crate::Spend::Send {
            id: 1,
            from: crate::Source::Core,
            to: "yX".into(),
            amount: 1,
            note: None,
            rate: Some(rate.clone()),
        };
        for text in [format!("{rate:?}"), format!("{send:?}")] {
            assert!(text.contains("USD"), "{text}");
            assert!(
                !text.contains("3078") && !text.contains("1700000"),
                "{text}"
            );
        }
    }

    #[test]
    fn the_rate_is_asked_for_once_in_four_minutes_while_the_screen_is_open() {
        let now = 100_000;
        assert!(!ask_now(false, None, None, now), "closed: never");
        assert!(ask_now(true, None, None, now), "no rate yet");
        assert!(!ask_now(true, Some(now - 60), None, now), "fresh enough");
        assert!(ask_now(true, Some(now - ASK_AFTER - 1), None, now));
        assert!(
            !ask_now(true, Some(now - 600), Some(now - 30), now),
            "asked 30 s ago: not again"
        );
        assert!(ask_now(true, Some(now - 600), Some(now - ASK_AFTER), now));
    }

    #[test]
    fn only_keys_new_in_this_session_get_the_rate_and_only_once() {
        let at_unlock: HashSet<String> = ["core:old".to_owned()].into();
        let mut seen = HashMap::new();
        let rate = usd(3_078_000_000, 1000);
        let mut rated: HashSet<String> = HashSet::new();
        // The first snapshot's keys: none.
        let got = first_seen_rates(
            ["core:old"],
            &at_unlock,
            &mut seen,
            |k| rated.contains(k),
            Some(&rate),
            1000,
        );
        assert!(got.is_empty());
        // A new key with a fresh rate: once.
        let got = first_seen_rates(
            ["core:old", "core:new", "", "pending:pool:1:2"],
            &at_unlock,
            &mut seen,
            |k| rated.contains(k),
            Some(&rate),
            1010,
        );
        assert_eq!(got, vec!["core:new".to_owned()]);
        rated.extend(got);
        let again = first_seen_rates(
            ["core:new"],
            &at_unlock,
            &mut seen,
            |k| rated.contains(k),
            Some(&usd(1, 1020)),
            1020,
        );
        assert!(again.is_empty(), "a kept rate never changes");
        // Seen with no fresh rate, then a rate within 5 minutes: it gets it; after: it never does.
        let none = first_seen_rates(["platform:1"], &at_unlock, &mut seen, |_| false, None, 2000);
        assert!(none.is_empty());
        let late = usd(3_078_000_000, 2200);
        assert_eq!(
            first_seen_rates(
                ["platform:1"],
                &at_unlock,
                &mut seen,
                |_| false,
                Some(&late),
                2200
            ),
            vec!["platform:1".to_owned()]
        );
        let later = usd(3_078_000_000, 2400);
        assert!(
            first_seen_rates(["platform:2"], &at_unlock, &mut seen, |_| false, None, 2000)
                .is_empty()
        );
        assert!(
            first_seen_rates(
                ["platform:2"],
                &at_unlock,
                &mut seen,
                |_| false,
                Some(&later),
                2400
            )
            .is_empty()
        );
    }
}
