//! FEAT-0048 R-M1: which fully mixed denominations pay an amount to Platform (an identity
//! created or topped up, or the pool) without change. First an exact set: denominations whose
//! nominal amounts add up to the amount itself, so what reaches Platform is round (the
//! denominations' tails, a hundred-thousandth of each, go with the Core fee). Only when no exact
//! set exists, the set with the smallest excess.

use crate::DENOMINATIONS;

/// What a denomination is worth on paper: 0.0100001 DASH is 0.01 (its tail is one
/// hundred-thousandth of it).
pub fn nominal(denomination: u64) -> u64 {
    denomination / 100_001 * 100_000
}

/// The smallest nominal amount: 0.001 DASH, in duffs. Every nominal is a whole number of it.
pub const NOMINAL_UNIT: u64 = 100_000;

/// The denominations chosen for an amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    /// How many coins of each of [`DENOMINATIONS`], in its order (largest first).
    pub counts: [u32; 5],
    /// Their nominal sum, in duffs.
    pub nominal: u64,
    /// The nominal sum is the amount itself.
    pub exact: bool,
}

impl Pick {
    /// What the coins hold, tails included, in duffs.
    pub fn value(&self) -> u64 {
        self.counts
            .iter()
            .zip(DENOMINATIONS)
            .map(|(count, denomination)| u64::from(*count) * denomination)
            .sum()
    }

    /// How many coins.
    pub fn coins(&self) -> u32 {
        self.counts.iter().sum()
    }
}

/// FEAT-0048 R-M1: the denominations to pay `amount` (duffs, the nominal amount asked) from
/// coins worth `available` (one value per coin; only [`DENOMINATIONS`] count, anything else is
/// left out). An exact set with the fewest coins when one exists; else the smallest nominal sum
/// above the amount, with the fewest coins among those. `None`: not enough.
pub fn pick_denominations(amount: u64, available: &[u64]) -> Option<Pick> {
    let mut have = [0u32; 5];
    for value in available {
        if let Some(k) = DENOMINATIONS.iter().position(|d| d == value) {
            have[k] += 1;
        }
    }
    let units: [u64; 5] = DENOMINATIONS.map(|d| nominal(d) / NOMINAL_UNIT);
    let total: u64 = have.iter().zip(units).map(|(n, u)| u64::from(*n) * u).sum();
    // The amount in whole units, rounded up: no sum below it pays the amount.
    let target = amount.div_ceil(NOMINAL_UNIT);
    if amount == 0 || target > total {
        return None;
    }
    // A sum past the target by more than the largest unit never has the least excess: one of
    // its coins can go and it still covers the target.
    let limit = usize::try_from((target + units[0]).min(total)).ok()?;
    // Bounded knapsack by binary splitting: each item is a bundle of one denomination.
    let mut items: Vec<(usize, u32)> = Vec::new();
    for (k, &n) in have.iter().enumerate() {
        let (mut left, mut size) = (n, 1u32);
        while left > 0 {
            let take = size.min(left);
            items.push((k, take));
            left -= take;
            size = size.saturating_mul(2);
        }
    }
    const NONE: u32 = u32::MAX;
    // best[s]: the fewest coins whose units add up to s.
    let mut best = vec![NONE; limit + 1];
    best[0] = 0;
    // improved[i][s]: item i lowered best[s] when it was added.
    let mut improved = vec![vec![false; limit + 1]; items.len()];
    let weight = |&(k, bundle): &(usize, u32)| (units[k] * u64::from(bundle)) as usize;
    for (i, item) in items.iter().enumerate() {
        let w = weight(item);
        if w > limit {
            continue;
        }
        for s in (w..=limit).rev() {
            let from = best[s - w];
            if from != NONE && from + item.1 < best[s] {
                best[s] = from + item.1;
                improved[i][s] = true;
            }
        }
    }
    let target = usize::try_from(target).ok()?;
    let sum = (target..=limit).find(|&s| best[s] != NONE)?;
    // Back along the last improvements: the latest item that improved a sum is on its best way.
    let mut counts = [0u32; 5];
    let mut s = sum;
    for (i, item) in items.iter().enumerate().rev() {
        if s > 0 && improved[i][s] {
            counts[item.0] += item.1;
            s -= weight(item);
        }
    }
    debug_assert_eq!(s, 0);
    let nominal = sum as u64 * NOMINAL_UNIT;
    Some(Pick {
        counts,
        nominal,
        exact: nominal == amount,
    })
}

/// FEAT-0048 UC-7/UC-8 «Exactly from denominations: {}»: the largest nominal amount, not above
/// `amount`, that `available`'s denominations make exactly (`None`: not even the smallest).
/// Each denomination is ten times the next, so taking the largest that still fits, largest
/// first, is the best: a sum the greedy misses would need ten of a smaller coin where one larger
/// fits.
pub fn largest_exact_at_most(amount: u64, available: &[u64]) -> Option<u64> {
    let mut have = [0u64; 5];
    for value in available {
        if let Some(k) = DENOMINATIONS.iter().position(|d| d == value) {
            have[k] += 1;
        }
    }
    let mut left = amount;
    let mut sum = 0;
    for (k, denomination) in DENOMINATIONS.iter().enumerate() {
        let worth = nominal(*denomination);
        let take = (left / worth).min(have[k]);
        sum += take * worth;
        left -= take * worth;
    }
    (sum > 0).then_some(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    const D10: u64 = DENOMINATIONS[0];
    const D1: u64 = DENOMINATIONS[1];
    const D01: u64 = DENOMINATIONS[2];
    const D001: u64 = DENOMINATIONS[3];
    const D0001: u64 = DENOMINATIONS[4];

    fn coins(of: &[(u64, usize)]) -> Vec<u64> {
        of.iter()
            .flat_map(|&(d, n)| std::iter::repeat_n(d, n))
            .collect()
    }

    #[test]
    fn a_denominations_nominal_drops_its_tail() {
        assert_eq!(
            DENOMINATIONS.map(nominal),
            [1_000_000_000, 100_000_000, 10_000_000, 1_000_000, 100_000]
        );
    }

    /// A9: 0.03 from 3 × 0.01 — exact, nothing over the amount but the tails.
    #[test]
    fn three_hundredths_pay_three_hundredths_exactly() {
        let pick = pick_denominations(3_000_000, &coins(&[(D001, 3), (D01, 2)])).unwrap();
        assert_eq!(pick.counts, [0, 0, 0, 3, 0]);
        assert!(pick.exact);
        assert_eq!(pick.value(), 3 * D001);
    }

    /// R-M1: 0.25 = 2 × 0.1 + 5 × 0.01, the fewest coins of the exact sets.
    #[test]
    fn the_exact_set_with_the_fewest_coins() {
        let have = coins(&[(D01, 3), (D001, 10), (D0001, 60)]);
        let pick = pick_denominations(25_000_000, &have).unwrap();
        assert_eq!(pick.counts, [0, 0, 2, 5, 0]);
        assert!(pick.exact);
    }

    /// The smaller coins make up what the larger ones lack.
    #[test]
    fn smaller_coins_complete_an_exact_set() {
        let pick = pick_denominations(3_000_000, &coins(&[(D001, 2), (D0001, 10)])).unwrap();
        assert_eq!(pick.counts, [0, 0, 0, 2, 10]);
        assert!(pick.exact);
    }

    /// A10: only 0.1 for 0.03 — no exact set, the excess is 0.07.
    #[test]
    fn without_an_exact_set_the_least_excess() {
        let pick = pick_denominations(3_000_000, &coins(&[(D01, 2)])).unwrap();
        assert_eq!(pick.counts, [0, 0, 1, 0, 0]);
        assert!(!pick.exact);
        assert_eq!(pick.nominal, 10_000_000);
    }

    /// The least excess may take one larger coin rather than the smaller ones plus another:
    /// 0.015 from 0.01, 3 × 0.001 and 0.1 is 0.1 alone (excess 0.085), not 0.01 + 3 × 0.001 +
    /// 0.1 (0.098).
    #[test]
    fn the_least_excess_is_not_the_greedy_one() {
        let have = coins(&[(D001, 1), (D0001, 3), (D01, 1)]);
        let pick = pick_denominations(1_500_000, &have).unwrap();
        assert_eq!(pick.nominal, 10_000_000);
        assert_eq!(pick.counts, [0, 0, 1, 0, 0]);
    }

    /// An amount that is not a whole number of 0.001 has no exact set: the next one up.
    #[test]
    fn an_amount_finer_than_the_smallest_denomination_is_never_exact() {
        let pick = pick_denominations(3_050_000, &coins(&[(D001, 3), (D0001, 5)])).unwrap();
        assert!(!pick.exact);
        assert_eq!(pick.nominal, 3_100_000);
    }

    /// Not enough, nothing, or an amount of zero: no pick.
    #[test]
    fn not_enough_is_none() {
        assert_eq!(pick_denominations(3_000_000, &coins(&[(D001, 2)])), None);
        assert_eq!(pick_denominations(3_000_000, &[]), None);
        assert_eq!(pick_denominations(0, &coins(&[(D001, 2)])), None);
    }

    /// Coins that are not denominations (change, collateral) are never chosen.
    #[test]
    fn only_denominations_are_chosen() {
        let have = vec![3_000_000, 20_000, D001, D001, D001];
        let pick = pick_denominations(3_000_000, &have).unwrap();
        assert_eq!(pick.counts, [0, 0, 0, 3, 0]);
    }

    /// A large wallet stays quick and still exact: 12.345 from many coins of each.
    #[test]
    fn a_large_amount_from_many_coins() {
        let have = coins(&[(D10, 2), (D1, 5), (D01, 9), (D001, 9), (D0001, 9)]);
        let pick = pick_denominations(1_234_500_000, &have).unwrap();
        assert_eq!(pick.counts, [1, 2, 3, 4, 5]);
        assert!(pick.exact);
    }

    /// A13: 0.05 asked with 0.1 and 3 × 0.01 → «Exactly from denominations: 0.03».
    #[test]
    fn the_largest_exact_amount_not_above_the_one_asked() {
        let have = coins(&[(D01, 1), (D001, 3)]);
        assert_eq!(largest_exact_at_most(5_000_000, &have), Some(3_000_000));
        assert_eq!(largest_exact_at_most(10_000_000, &have), Some(10_000_000));
        assert_eq!(largest_exact_at_most(12_500_000, &have), Some(12_000_000));
        assert_eq!(
            largest_exact_at_most(900_000, &have),
            None,
            "below the smallest"
        );
        assert_eq!(largest_exact_at_most(5_000_000, &[]), None);
    }
}
