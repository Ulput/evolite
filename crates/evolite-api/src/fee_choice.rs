//! FEAT-0040: the fee of a Core payment to an address, for advanced users — «Normal», «Higher»
//! or «Custom», in duffs per kB. One place for the presets and the bounds: the form refuses what
//! the core refuses.

use crate::Duffs;

/// «Normal»: Dash's relay minimum, key-wallet's `FeeRate::normal()`.
pub const NORMAL: u64 = 1_000;
/// «Higher»: key-wallet's `FeeRate::priority()` — a preset, not a number of one's own, so that a
/// rare rate does not single out one person's payments in the chain.
pub const HIGHER: u64 = 2_000;
/// The least rate: below it nodes do not pass the transaction on (key-wallet's `economy()`, 500,
/// would hang).
pub const MIN: u64 = 1_000;
/// The most «Custom» takes: 1 000 duffs a byte (R-3a, R-9; the architect 2026-11-20). One zero
/// too many is warned of when the fee passes the payment, not refused.
pub const MAX: u64 = 1_000_000;
/// Over this «Custom» is warned of: ten times the normal.
pub const WARN_OVER: u64 = 10_000;

/// The person's choice, kept for each wallet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FeeChoice {
    #[default]
    Normal,
    Higher,
    Custom(u64),
}

impl FeeChoice {
    /// Its rate in duffs per kB.
    pub fn rate(self) -> u64 {
        match self {
            FeeChoice::Normal => NORMAL,
            FeeChoice::Higher => HIGHER,
            FeeChoice::Custom(rate) => rate,
        }
    }

    /// The rate a payment carries: `None` for «Normal», which is what a payment has without one.
    pub fn carried(self) -> Option<u64> {
        (self != FeeChoice::Normal).then(|| self.rate())
    }
}

/// Whether the core and the form refuse `rate`: under [`MIN`] or over [`MAX`].
pub fn refused(rate: u64) -> bool {
    !(MIN..=MAX).contains(&rate)
}

/// A fee estimated at the normal rate, at `rate` instead: a fee is the size times the rate.
pub fn at_rate(normal_fee: Duffs, rate: u64) -> Duffs {
    let scaled = i128::from(normal_fee) * i128::from(rate) / i128::from(NORMAL);
    Duffs::try_from(scaled).unwrap_or(Duffs::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presets_and_the_bounds() {
        assert_eq!(FeeChoice::default().rate(), 1_000);
        assert_eq!(FeeChoice::Higher.rate(), 2_000);
        assert_eq!(FeeChoice::Custom(12_000).rate(), 12_000);
        assert_eq!(FeeChoice::Normal.carried(), None);
        assert_eq!(FeeChoice::Higher.carried(), Some(2_000));
        assert!(refused(999) && !refused(MIN) && !refused(MAX) && refused(MAX + 1));
        assert_eq!(at_rate(226, HIGHER), 452, "twice the rate, twice the fee");
        assert_eq!(at_rate(226, NORMAL), 226);
    }
}
