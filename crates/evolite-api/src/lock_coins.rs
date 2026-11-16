//! What the coins an asset lock may take say of its amount (SPV Б2 and item 3): one function for
//! the forms and the core, so the form refuses before «Check» what the core would refuse before
//! the password.
//!
//! - **Б2:** Dash Core relays an asset lock of at most `max_asset_lock_transaction_inputs` inputs
//!   (100) and drops a larger one silently, its money stuck. platform-wallet guards only
//!   `DrainAll`; an exact amount is chosen by key-wallet. The fewest inputs that make the lock
//!   (the largest coins first) above the platform's limit less [`LOCK_INPUTS_MARGIN`] is
//!   refused.
//! - **Item 3:** an asset lock takes only final coins (in a block, or InstantSend-locked by a
//!   lock verified in this run). When those fall short and the wallet's other coins would cover
//!   it, the answer is to wait for a block.

use crate::{CoreCoin, Duffs, Fees};

/// Inputs left below the platform's limit: key-wallet's `BranchAndBound` may take more coins than
/// the largest-first count, and a margin of 10 covers that in a usual wallet (the architect, Б2).
pub const LOCK_INPUTS_MARGIN: usize = 10;

/// What [`lock_coins`] says of an asset lock's amount.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LockCoins {
    /// The final coins make it with this many inputs (0: nothing known to check).
    Fits(usize),
    /// It needs more than `allowed` inputs: the network would not take it.
    TooMany { allowed: usize },
    /// The final coins fall short; with the coins not confirmed yet it would be enough.
    NotFinal,
    /// The wallet's coins fall short in all: the form's «more than there is on Core».
    Short,
}

/// The fewest inputs of `coins` (largest first, each with the fee of spending it and every
/// larger one, as [`Fees::core_coins`]) that hold `amount` and their own fee; `None` when all of
/// them do not.
pub fn fewest_inputs(coins: &[CoreCoin], amount: Duffs) -> Option<usize> {
    let mut held: Duffs = 0;
    for (n, coin) in coins.iter().enumerate() {
        held = held.saturating_add(coin.value);
        if held >= amount.saturating_add(coin.fee) {
            return Some(n + 1);
        }
    }
    None
}

/// What an asset lock of `amount` duffs can be made of: `final_coins` are the coins it may take
/// (final and signable), `all_coins` every coin of the wallet, both as [`Fees::core_coins`];
/// `max_inputs` the platform's limit, 0 when not known (nothing is refused then).
pub fn lock_coins(
    final_coins: &[CoreCoin],
    all_coins: &[CoreCoin],
    amount: Duffs,
    max_inputs: usize,
) -> LockCoins {
    if max_inputs == 0 {
        return LockCoins::Fits(0);
    }
    let allowed = max_inputs.saturating_sub(LOCK_INPUTS_MARGIN).max(1);
    match fewest_inputs(final_coins, amount) {
        Some(n) if n > allowed => LockCoins::TooMany { allowed },
        Some(n) => LockCoins::Fits(n),
        None if fewest_inputs(all_coins, amount).is_some() => LockCoins::NotFinal,
        None => LockCoins::Short,
    }
}

impl Fees {
    /// [`lock_coins`] with the wallet's coins as the snapshot has them.
    pub fn lock_coins(&self, amount: Duffs) -> LockCoins {
        lock_coins(
            &self.lock_coins,
            &self.core_coins,
            amount,
            self.lock_inputs_max,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As `Fees::core_coins`: largest first, 226 duffs for one input and 148 more for each.
    fn coins(values: &[Duffs]) -> Vec<CoreCoin> {
        let mut values = values.to_vec();
        values.sort_unstable_by(|a, b| b.cmp(a));
        values
            .into_iter()
            .enumerate()
            .map(|(n, value)| CoreCoin {
                value,
                fee: 226 + 148 * n as Duffs,
            })
            .collect()
    }

    /// AC-T1: enough large coins, few inputs; 150 coins of 0.0001 for 0.01 need more than 90,
    /// refused; exactly 90 passes; the limit is the one given (the platform's).
    #[test]
    fn the_fewest_inputs_decide() {
        let big = coins(&[5_000_000, 3_000_000, 10_000]);
        assert_eq!(lock_coins(&big, &big, 6_000_000, 100), LockCoins::Fits(2));
        let small = coins(&[10_000; 150]);
        assert_eq!(
            lock_coins(&small, &small, 1_000_000, 100),
            LockCoins::TooMany { allowed: 90 }
        );
        // Exactly 90 coins: 90 × 10 000 less the fee of 90 inputs.
        let fee_90 = 226 + 148 * 89;
        assert_eq!(
            lock_coins(&small, &small, 900_000 - fee_90, 100),
            LockCoins::Fits(90)
        );
        assert_eq!(
            lock_coins(&small, &small, 900_000 - fee_90 + 1, 100),
            LockCoins::TooMany { allowed: 90 }
        );
        // Another platform limit, another bound.
        assert_eq!(
            lock_coins(&small, &small, 1_000_000, 50),
            LockCoins::TooMany { allowed: 40 }
        );
        // Not known: nothing refused.
        assert_eq!(lock_coins(&small, &small, 1_000_000, 0), LockCoins::Fits(0));
    }

    /// Item 3: final coins short while every coin would do → wait for a block; short in all →
    /// the forms' own «more than there is».
    #[test]
    fn coins_not_final_yet_say_wait() {
        let final_coins = coins(&[1_000_000]);
        let all = coins(&[1_000_000, 5_000_000]);
        assert_eq!(
            lock_coins(&final_coins, &all, 3_000_000, 100),
            LockCoins::NotFinal
        );
        assert_eq!(
            lock_coins(&final_coins, &all, 500_000, 100),
            LockCoins::Fits(1)
        );
        assert_eq!(
            lock_coins(&final_coins, &all, 9_000_000, 100),
            LockCoins::Short
        );
    }
}
