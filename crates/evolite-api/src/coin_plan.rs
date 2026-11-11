//! What a Core payment from chosen coins will take (FEAT-0013, path 2): the form shows it in
//! «Summary» and the last look, and the core refuses a set that cannot pay before building.
//! The same rule as key-wallet's `LargestFirst` within the set: the largest coins until they
//! cover the amount and the fee of that many inputs; «Max» (`All`) every coin, no change.

use crate::{Duffs, Fees};

/// Change below this is not made: it goes to the fee (key-wallet's dust threshold for a P2PKH
/// output, R-7).
pub const DUST: Duffs = 546;

/// The plan of a payment from chosen coins.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CoinPlan {
    /// How many of the chosen coins it takes, largest first.
    pub taken: usize,
    /// What they hold.
    pub inputs: Duffs,
    /// The fee, with dust change in it.
    pub fee: Duffs,
    /// The change back to the wallet; 0 when there is none.
    pub change: Duffs,
    /// The change was dust and went to the fee.
    pub dust_to_fee: bool,
}

impl Fees {
    /// The fee of a Core payment with `inputs` inputs and change: the wallet's coins' fees
    /// depend only on how many there are ([`crate::CoreCoin::fee`]).
    pub fn core_fee_for_inputs(&self, inputs: usize) -> Duffs {
        match inputs.checked_sub(1) {
            Some(i) => self
                .core_coins
                .get(i)
                .or(self.core_coins.last())
                .map_or(self.core_payment, |c| c.fee),
            None => 0,
        }
    }
}

/// The plan for `amount` from coins of `values` (any order), or how much is short (R-5).
/// With `all` («Max»), every coin, and the amount is what is left after the fee.
pub fn coin_plan(
    values: &[Duffs],
    amount: Duffs,
    all: bool,
    fee_of: impl Fn(usize) -> Duffs,
) -> Result<CoinPlan, Duffs> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let total: Duffs = sorted.iter().sum();
    if all {
        let fee = fee_of(sorted.len());
        return if total > fee {
            Ok(CoinPlan {
                taken: sorted.len(),
                inputs: total,
                fee,
                change: 0,
                dust_to_fee: false,
            })
        } else {
            Err(fee - total + 1)
        };
    }
    let mut held: Duffs = 0;
    for (n, value) in sorted.iter().enumerate() {
        held = held.saturating_add(*value);
        let fee = fee_of(n + 1);
        if held >= amount.saturating_add(fee) {
            let change = held - amount - fee;
            let dust = change > 0 && change < DUST;
            return Ok(CoinPlan {
                taken: n + 1,
                inputs: held,
                fee: if dust { fee + change } else { fee },
                change: if dust { 0 } else { change },
                dust_to_fee: dust,
            });
        }
    }
    let fee = fee_of(sorted.len().max(1));
    Err(amount.saturating_add(fee).saturating_sub(total))
}

/// «Max» of the chosen coins: what they hold less the fee of spending them all (R-7).
pub fn max_of(values: &[Duffs], fee_of: impl Fn(usize) -> Duffs) -> Duffs {
    let total: Duffs = values.iter().sum();
    total.saturating_sub(fee_of(values.len())).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 226 duffs for one input, then 148 more for each.
    fn fees() -> impl Fn(usize) -> Duffs {
        |n| {
            if n == 0 {
                0
            } else {
                226 + 148 * (n as Duffs - 1)
            }
        }
    }

    #[test]
    fn the_largest_chosen_coins_until_they_cover_the_amount_and_fee() {
        // 0.08, 0.05, 0.03 chosen; 0.05 to pay: the 0.08 alone covers it.
        let plan = coin_plan(&[5_000_000, 8_000_000, 3_000_000], 5_000_000, false, fees()).unwrap();
        assert_eq!((plan.taken, plan.inputs, plan.fee), (1, 8_000_000, 226));
        assert_eq!(plan.change, 8_000_000 - 5_000_000 - 226);
        // 0.1 needs two: 0.08 + 0.05.
        let plan = coin_plan(
            &[5_000_000, 8_000_000, 3_000_000],
            10_000_000,
            false,
            fees(),
        )
        .unwrap();
        assert_eq!((plan.taken, plan.fee), (2, 374));
    }

    #[test]
    fn a_set_too_small_says_how_much_is_short() {
        let short = coin_plan(&[5_000_000, 3_000_000], 8_000_000, false, fees()).unwrap_err();
        assert_eq!(short, 374);
    }

    /// R-11: a payment's `Debug` names how many coins were chosen, never which.
    #[test]
    fn debug_counts_the_chosen_coins_only() {
        let outpoint = format!("{}:1", "ab".repeat(32));
        let send = crate::Spend::Send {
            id: 1,
            from: crate::Source::Core,
            to: "yX".into(),
            amount: 1,
            note: None,
            rate: None,
            coins: Some(crate::CoinChoice::Within {
                coins: vec![outpoint.clone()],
                all: true,
            }),
        };
        let text = format!("{send:?}");
        assert!(text.contains("Within(1 coins, all: true)"), "{text}");
        assert!(!text.contains("abab"), "{text}");
    }

    #[test]
    fn dust_change_goes_to_the_fee() {
        let plan = coin_plan(&[5_000_500], 5_000_000, false, fees()).unwrap();
        assert!(plan.dust_to_fee);
        assert_eq!((plan.fee, plan.change), (500, 0));
    }

    #[test]
    fn max_spends_every_chosen_coin_less_the_fee_of_them_all() {
        let values = [5_000_000, 8_000_000, 3_000_000];
        assert_eq!(max_of(&values, fees()), 16_000_000 - 522);
        let plan = coin_plan(&values, 0, true, fees()).unwrap();
        assert_eq!((plan.taken, plan.fee, plan.change), (3, 522, 0));
    }
}
