//! Funding the wallet's Platform address from Core (FEAT-0015 stage 2): an asset lock on Core
//! that Platform turns into credits on the address. One pure function for the form and for the
//! core (AC-T2), so «Check the top-up» shows what the core locks.
//!
//! The transition has one output, the address, with no amount (`None`): the address gets the
//! lock less what Platform keeps for processing it. So the lock is the amount asked for and an
//! upper estimate of that on top ([`Fees::address_fund_on_top`]); the address gets **at least**
//! the amount, and what the estimate leaves over stays on the same address (the architect,
//! дизайнер-51). Platform refuses a lock under its admission threshold
//! ([`Fees::address_fund_lock_min`]), and the DASH would stay locked until «Continue»: the
//! plan refuses it before anything is locked.

use crate::{Duffs, Fees};

/// What Platform keeps of a lock that funds an address, with a margin. Measured live (AC-T1,
/// 2026-11-14, testnet, two locks of 60 000 duffs): 8 827 duffs to a new, empty address and
/// 8 928 to an address already funded. 12 000 is the larger with about a third on top, as an
/// identity's top-up keeps 10 000 for a measured 8 130: Platform's fee moves with the state of
/// its trees. What the margin leaves stays on the address.
pub const ADDRESS_FUND_ON_TOP: Duffs = 12_000;

/// What testnet kept of a lock that funded an address (AC-T1): a new address, a funded one.
pub const ADDRESS_FUND_MEASURED: [Duffs; 2] = [8_827, 8_928];

/// A funding of the Platform address from Core, in duffs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FundPlan {
    /// What the address gets at least: the amount asked for.
    pub arrives_at_least: Duffs,
    /// Platform's processing, on top of the amount (an estimate; what is not needed stays on
    /// the address).
    pub on_top: Duffs,
    /// The asset lock: the amount and what is on top.
    pub lock: Duffs,
    /// The Core transaction's fee.
    pub core_fee: Duffs,
    /// What leaves Core: the lock and its fee.
    pub total: Duffs,
}

/// Why no plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FundRefusal {
    /// The lock would be under Platform's threshold: «At least {least} to the address».
    BelowLeast { least: Duffs },
    /// More than Core can lock with its fee: `most` is the most the address can get now.
    NotEnough { most: Duffs },
}

impl Fees {
    /// What is paid on top of the amount for Platform's processing of a lock that funds an
    /// address: never below [`ADDRESS_FUND_ON_TOP`].
    pub fn address_fund_on_top(&self) -> Duffs {
        self.address_fund_on_top.max(ADDRESS_FUND_ON_TOP)
    }

    /// The least amount to fund the address with: its lock (the amount and what is on top)
    /// reaches Platform's [`Self::address_fund_lock_min`]. Never below one duff.
    pub fn address_fund_least(&self) -> Duffs {
        self.address_fund_lock_min
            .saturating_sub(self.address_fund_on_top())
            .max(1)
    }
}

/// The plan for funding the address with `amount` duffs (what it gets at least), or with
/// `max`, with all that Core can lock: `spendable` is what Core can pay with (its balance less
/// what no payment can sign for yet, `Fees::core_unsignable`).
pub fn fund_plan(
    amount: Duffs,
    max: bool,
    spendable: Duffs,
    fees: &Fees,
) -> Result<FundPlan, FundRefusal> {
    let on_top = fees.address_fund_on_top();
    let least = fees.address_fund_least();
    // «Max»: every coin goes, so the fee is that of spending them all.
    let most = spendable
        .saturating_sub(fees.core_payment)
        .saturating_sub(on_top)
        .max(0);
    let amount = if max { most } else { amount };
    if amount < least {
        return Err(if max {
            FundRefusal::NotEnough { most }
        } else {
            FundRefusal::BelowLeast { least }
        });
    }
    let lock = amount.saturating_add(on_top);
    let core_fee = if max {
        fees.core_payment
    } else {
        fees.core_payment_for(lock)
    };
    let total = lock.saturating_add(core_fee);
    if total > spendable {
        return Err(FundRefusal::NotEnough { most });
    }
    Ok(FundPlan {
        arrives_at_least: amount,
        on_top,
        lock,
        core_fee,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CoreCoin;

    /// The fees the stage 2 mockups were drawn with (the demo's Core fee 522 duffs) and the
    /// threshold at platform `575d961` for one output: 50 000 to start processing + 6 000.
    fn fees(coins: &[Duffs]) -> Fees {
        Fees {
            core_payment: 522,
            core_coins: coins
                .iter()
                .map(|&value| CoreCoin { value, fee: 522 })
                .collect(),
            core_unsignable: 0,
            credit_transfer: 0,
            credit_transfer_to_address: 0,
            pool_transfer: 0,
            nft_transfer: 0,
            document_write: 0,
            shield: 0,
            identity_create: 0,
            identity_lock_min: 0,
            top_up_lock_min: 0,
            address_fund_on_top: ADDRESS_FUND_ON_TOP,
            address_fund_lock_min: 56_000,
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

    /// AC-T2 and the mockups (22): 0,1 DASH to the address locks 0,10012, and 0,10012522 leave
    /// Core; the address gets at least 0,1.
    #[test]
    fn the_lock_is_the_amount_and_what_is_on_top() {
        let plan = fund_plan(10_000_000, false, 50_000_000, &fees(&[50_000_000])).unwrap();
        assert_eq!(
            plan,
            FundPlan {
                arrives_at_least: 10_000_000,
                on_top: 12_000,
                lock: 10_012_000,
                core_fee: 522,
                total: 10_012_522,
            }
        );
    }

    /// The threshold: «At least 0,00044 DASH to the address» (56 000 − 12 000; the mockups,
    /// drawn with 20 000 on top, say 0,00036); below it nothing is locked.
    #[test]
    fn under_the_threshold_nothing_is_locked() {
        let fees = fees(&[50_000_000]);
        assert_eq!(fees.address_fund_least(), 44_000);
        assert_eq!(
            fund_plan(43_999, false, 50_000_000, &fees),
            Err(FundRefusal::BelowLeast { least: 44_000 })
        );
        let plan = fund_plan(44_000, false, 50_000_000, &fees).unwrap();
        assert_eq!(plan.lock, 56_000, "exactly the threshold");
    }

    /// «Max» with one coin: the coin less its fee and what is on top; the whole coin leaves.
    #[test]
    fn max_with_one_coin_spends_it_all() {
        let plan = fund_plan(0, true, 1_000_000, &fees(&[1_000_000])).unwrap();
        assert_eq!(plan.arrives_at_least, 1_000_000 - 522 - 12_000);
        assert_eq!(plan.total, 1_000_000);
        // A coin too small for the threshold: not enough, nothing to lock.
        assert_eq!(
            fund_plan(0, true, 50_000, &fees(&[50_000])),
            Err(FundRefusal::NotEnough {
                most: 50_000 - 522 - 12_000
            })
        );
    }

    /// More than Core holds with the fee: says how much the address can get now.
    #[test]
    fn more_than_core_holds_says_the_most() {
        let fees = fees(&[1_000_000]);
        assert_eq!(
            fund_plan(990_000, false, 1_000_000, &fees),
            Err(FundRefusal::NotEnough {
                most: 1_000_000 - 522 - 12_000
            })
        );
        assert!(fund_plan(1_000_000 - 522 - 12_000, false, 1_000_000, &fees).is_ok());
    }

    /// What is on top never drops under the estimate, whatever the fees say (a demo or an old
    /// data folder with 0).
    #[test]
    fn what_is_on_top_is_never_below_the_estimate() {
        let mut fees = fees(&[1_000_000]);
        fees.address_fund_on_top = 0;
        assert_eq!(fees.address_fund_on_top(), ADDRESS_FUND_ON_TOP);
        fees.address_fund_on_top = 30_000;
        assert_eq!(fees.address_fund_on_top(), 30_000);
    }

    /// AC-T1: the estimate is not below what testnet kept of a lock that funded an address, a
    /// new one or a funded one, and keeps a margin over the larger (as
    /// `the_estimate_is_not_below_what_testnet_charged` in `address_plan`, QA21). The address
    /// gets at least the amount asked for only while this holds.
    #[test]
    fn the_estimate_is_not_below_what_testnet_charged() {
        let most = ADDRESS_FUND_MEASURED.into_iter().max().unwrap();
        assert!(
            ADDRESS_FUND_ON_TOP >= most,
            "{ADDRESS_FUND_ON_TOP} < {most}"
        );
        assert!(
            ADDRESS_FUND_ON_TOP >= most + most / 4,
            "less than a quarter over the measured {most}"
        );
        // And the threshold still leaves an amount: a lock of 56 000 is not all fee.
        assert!(fees(&[]).address_fund_least() > 0);
    }
}
