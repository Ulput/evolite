//! Spending from the wallet's Platform addresses (FEAT-0015): which addresses pay, how much
//! each gives, and the fee, by the protocol's own formulas. One pure function for the form and
//! for the core, so that «Check the payment» shows the plan the core signs (R-7).
//!
//! Amounts here are **credits** (1 duff = 1 000 credits): an address holds credits, not duffs.
//! The fees are the protocol's minimums (`state_transition_min_fees`); Platform may meter more,
//! so a form says «about» (BUG-108). There is no change output: each address gives only what
//! the plan takes from it, and the rest stays where it was.

/// Credits in one duff.
pub const CREDITS_PER_DUFF: u64 = 1_000;
/// Each input address of an address transition.
pub const INPUT_COST: u64 = 500_000;
/// Each output of a transfer (at least one is counted).
pub const OUTPUT_COST: u64 = 6_000_000;
/// The base of a withdrawal to Core.
pub const WITHDRAWAL_COST: u64 = 400_000_000;
/// The base of an identity's top-up from addresses.
pub const TOP_UP_COST: u64 = 500_000;
/// The least an input may give.
pub const MIN_INPUT: u64 = 100_000;
/// The least an output may receive.
pub const MIN_OUTPUT: u64 = 500_000;
/// The least a top-up of an identity may bring.
pub const MIN_FUNDING: u64 = 200_000;
/// The least a withdrawal may take out (`system_limits` v2).
pub const MIN_WITHDRAWAL: u64 = 1_000_000;
/// The most inputs one transition takes.
pub const MAX_INPUTS: usize = 16;

/// What the money on the addresses does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressMove {
    /// To a Platform address, ours or anyone's.
    Transfer,
    /// To a Core address of this wallet.
    Withdraw,
    /// Into the credits of one of the wallet's identities.
    TopUp,
}

impl AddressMove {
    /// The protocol's fee with `inputs` addresses paying.
    pub fn fee(self, inputs: usize) -> u64 {
        let inputs = INPUT_COST.saturating_mul(inputs as u64);
        match self {
            AddressMove::Transfer => inputs.saturating_add(OUTPUT_COST),
            AddressMove::Withdraw => inputs.saturating_add(WITHDRAWAL_COST),
            AddressMove::TopUp => inputs.saturating_add(TOP_UP_COST),
        }
    }

    /// The least the recipient may get.
    pub fn least(self) -> u64 {
        match self {
            AddressMove::Transfer => MIN_OUTPUT,
            AddressMove::Withdraw => MIN_WITHDRAWAL,
            AddressMove::TopUp => MIN_FUNDING,
        }
    }
}

/// What a payment from the addresses to `to` is (UC-1): `@name` tops up an identity of the
/// wallet, a Platform address (`tdash1…`, `dash1…`) is a transfer, anything else a withdrawal to
/// Core. Whether the recipient may get it at all is the core's check.
pub fn move_for(to: &str) -> AddressMove {
    let to = to.trim();
    let lower = to.to_ascii_lowercase();
    if to.starts_with('@') {
        AddressMove::TopUp
    } else if lower.starts_with("tdash1") || lower.starts_with("dash1") {
        AddressMove::Transfer
    } else {
        AddressMove::Withdraw
    }
}

/// Credits as duffs, rounded up: a fee is never shown smaller than it is.
pub fn duffs_up(credits: u64) -> i64 {
    credits.div_ceil(CREDITS_PER_DUFF) as i64
}

/// Which addresses pay and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressPlan {
    /// `(index into the balances given, credits it gives)`, in the order of the balances given
    /// (the addresses' own order, as the transition keeps them).
    pub inputs: Vec<(usize, u64)>,
    /// The fee, as the protocol's formula gives it.
    pub fee: u64,
    /// Where the fee comes from: the rest of this input (an index into the balances given), or,
    /// `None`, out of what the recipient gets («Max» of a transfer).
    pub fee_payer: Option<usize>,
    /// What the recipient gets.
    pub receives: u64,
}

impl AddressPlan {
    /// What leaves the addresses in all: what they give and the fee.
    pub fn spent(&self) -> u64 {
        let given: u64 = self.inputs.iter().map(|(_, c)| c).sum();
        match self.fee_payer {
            Some(_) => given.saturating_add(self.fee),
            None => given,
        }
    }
}

/// Why no plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressRefusal {
    /// Below what the protocol lets this move carry.
    BelowLeast { least: u64 },
    /// More than the addresses hold, the fee included: `most` is what they can carry now.
    NotEnough { most: u64 },
}

/// The plan for moving `amount` credits (what the recipient gets), or, with `max`, everything.
/// `balances`: each address's credits, in the addresses' own order. Fewest inputs first, the
/// largest addresses first; the fee comes from the rest of the payer ([`payer_of`]), or, for
/// «Max» of a transfer, out of the amount (`ReduceOutput`).
pub fn address_plan(
    kind: AddressMove,
    amount: u64,
    max: bool,
    balances: &[u64],
) -> Result<AddressPlan, AddressRefusal> {
    // The largest first; an address below the least input cannot give anything.
    let mut by_size: Vec<usize> = (0..balances.len())
        .filter(|&i| balances[i] >= MIN_INPUT)
        .collect();
    by_size.sort_by(|&a, &b| balances[b].cmp(&balances[a]).then(a.cmp(&b)));
    by_size.truncate(MAX_INPUTS);
    if max {
        return everything(kind, &by_size, balances);
    }
    if amount < kind.least() {
        return Err(AddressRefusal::BelowLeast {
            least: kind.least(),
        });
    }
    let mut most = 0;
    for n in 1..=by_size.len() {
        let mut chosen = by_size[..n].to_vec();
        chosen.sort_unstable();
        match take(kind, amount, &chosen, balances) {
            Ok(plan) => return Ok(plan),
            Err(carries) => most = most.max(carries),
        }
    }
    Err(AddressRefusal::NotEnough { most })
}

/// Which of `chosen` (in the addresses' own order) keeps the fee: the largest, as platform-wallet's
/// `plan_withdrawal` reserves it (the architect, 2026-11-10); a top-up's transition always takes
/// it from its first input (`DeductFromInput(0)` in dpp), so there the first in that order.
fn payer_of(kind: AddressMove, chosen: &[usize], balances: &[u64]) -> usize {
    if kind == AddressMove::TopUp {
        return chosen[0];
    }
    *chosen
        .iter()
        .max_by(|&&a, &&b| balances[a].cmp(&balances[b]).then(b.cmp(&a)))
        .expect("at least one address")
}

/// «Max»: every chosen address gives all it has. A transfer takes the fee out of the amount;
/// a withdrawal and a top-up leave the fee on the payer, which gives the rest.
fn everything(
    kind: AddressMove,
    by_size: &[usize],
    balances: &[u64],
) -> Result<AddressPlan, AddressRefusal> {
    let mut chosen = by_size.to_vec();
    chosen.sort_unstable();
    let fee = kind.fee(chosen.len());
    let total: u64 = chosen.iter().map(|&i| balances[i]).sum();
    let receives = total.saturating_sub(fee);
    if chosen.is_empty() || receives < kind.least() {
        return Err(AddressRefusal::NotEnough { most: receives });
    }
    let mut inputs: Vec<(usize, u64)> = chosen.iter().map(|&i| (i, balances[i])).collect();
    if kind == AddressMove::Transfer {
        return Ok(AddressPlan {
            inputs,
            fee,
            fee_payer: None,
            receives,
        });
    }
    let payer = payer_of(kind, &chosen, balances);
    let gives = balances[payer].saturating_sub(fee);
    if gives < MIN_INPUT {
        // The payer cannot give its part and keep the fee: without the smallest.
        return if chosen.len() > 1 {
            everything(kind, &by_size[..by_size.len() - 1], balances)
                .or(Err(AddressRefusal::NotEnough { most: receives }))
        } else {
            Err(AddressRefusal::NotEnough { most: receives })
        };
    }
    if let Some(input) = inputs.iter_mut().find(|(i, _)| *i == payer) {
        input.1 = gives;
    }
    Ok(AddressPlan {
        inputs,
        fee,
        fee_payer: Some(payer),
        receives,
    })
}

/// `amount` from the `chosen` addresses (in their own order), the fee from the payer's rest.
/// `Err`: what they could carry.
fn take(
    kind: AddressMove,
    amount: u64,
    chosen: &[usize],
    balances: &[u64],
) -> Result<AddressPlan, u64> {
    let fee = kind.fee(chosen.len());
    let payer = payer_of(kind, chosen, balances);
    let total: u64 = chosen.iter().map(|&i| balances[i]).sum();
    let carries = total.saturating_sub(fee);
    let payer_most = balances[payer].saturating_sub(fee);
    if carries < amount || payer_most < MIN_INPUT {
        return Err(if payer_most < MIN_INPUT { 0 } else { carries });
    }
    // The others give what they have, the largest first; the payer gives the rest.
    let mut others: Vec<usize> = chosen.iter().copied().filter(|&i| i != payer).collect();
    others.sort_by(|&a, &b| balances[b].cmp(&balances[a]).then(a.cmp(&b)));
    let mut gives: Vec<(usize, u64)> = Vec::new();
    let mut left = amount;
    // The payer first, as much as its rest allows: fewer others needed.
    let from_payer = left.min(payer_most);
    left -= from_payer;
    let mut payer_gives = from_payer;
    for &i in &others {
        if left == 0 {
            break;
        }
        let mut part = left.min(balances[i]);
        left -= part;
        if part < MIN_INPUT {
            // Too small an input: this one gives the least an input may, the payer that
            // much less (every address here holds at least that).
            let more = MIN_INPUT - part;
            if payer_gives < MIN_INPUT + more {
                return Err(carries);
            }
            payer_gives -= more;
            part = MIN_INPUT;
        }
        gives.push((i, part));
    }
    if left > 0 || payer_gives < MIN_INPUT {
        return Err(carries);
    }
    gives.push((payer, payer_gives));
    gives.sort_unstable();
    // Inputs left out do not pay their part of the fee.
    let fee = kind.fee(gives.len());
    Ok(AddressPlan {
        inputs: gives,
        fee,
        fee_payer: Some(payer),
        receives: amount,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DASH: u64 = 100_000_000 * CREDITS_PER_DUFF;

    fn given(plan: &AddressPlan) -> u64 {
        plan.inputs.iter().map(|(_, c)| c).sum()
    }

    #[test]
    fn the_recipient_says_what_the_payment_is() {
        assert_eq!(move_for("@bob"), AddressMove::TopUp);
        assert_eq!(
            move_for(" tdash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq"),
            AddressMove::Transfer
        );
        assert_eq!(move_for("DASH1qq7x"), AddressMove::Transfer);
        assert_eq!(
            move_for("yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f"),
            AddressMove::Withdraw
        );
        assert_eq!(
            move_for("td1qxyz"),
            AddressMove::Withdraw,
            "bech32 Core: refused by the core"
        );
        assert_eq!(duffs_up(1), 1);
        assert_eq!(duffs_up(6_500_000), 6_500);
    }

    #[test]
    fn one_address_pays_and_keeps_the_fee() {
        let plan = address_plan(AddressMove::Transfer, DASH / 10, false, &[DASH]).unwrap();
        assert_eq!(plan.inputs, [(0, DASH / 10)]);
        assert_eq!(plan.fee, INPUT_COST + OUTPUT_COST);
        assert_eq!(plan.fee_payer, Some(0));
        assert_eq!(plan.receives, DASH / 10);
        assert_eq!(plan.spent(), DASH / 10 + plan.fee);
    }

    #[test]
    fn more_addresses_only_when_one_is_not_enough() {
        let balances = [DASH / 2, DASH, DASH / 4];
        let one = address_plan(AddressMove::Transfer, DASH / 2, false, &balances).unwrap();
        assert_eq!(one.inputs.len(), 1, "{one:?}");
        let two = address_plan(AddressMove::Transfer, DASH + DASH / 4, false, &balances).unwrap();
        assert_eq!(two.inputs.len(), 2, "{two:?}");
        assert_eq!(given(&two), DASH + DASH / 4);
        assert_eq!(two.fee, 2 * INPUT_COST + OUTPUT_COST);
        // The fee from the largest address, which keeps it.
        let payer = two.fee_payer.unwrap();
        assert_eq!(payer, 1);
        let payer_gives = two.inputs.iter().find(|(i, _)| *i == payer).unwrap().1;
        assert!(payer_gives + two.fee <= balances[payer]);
        for (i, c) in &two.inputs {
            assert!(*c >= MIN_INPUT && *c <= balances[*i], "{two:?}");
        }
    }

    #[test]
    fn the_protocols_limits_refuse() {
        for (kind, least) in [
            (AddressMove::Transfer, MIN_OUTPUT),
            (AddressMove::TopUp, MIN_FUNDING),
            (AddressMove::Withdraw, MIN_WITHDRAWAL),
        ] {
            assert_eq!(
                address_plan(kind, least - 1, false, &[DASH]),
                Err(AddressRefusal::BelowLeast { least })
            );
            assert!(
                address_plan(kind, least, false, &[DASH]).is_ok(),
                "{kind:?}"
            );
        }
        // Not enough with the fee: says how much is possible.
        let fee = AddressMove::Transfer.fee(1);
        assert_eq!(
            address_plan(AddressMove::Transfer, DASH, false, &[DASH]),
            Err(AddressRefusal::NotEnough { most: DASH - fee })
        );
        assert!(address_plan(AddressMove::Transfer, DASH - fee, false, &[DASH]).is_ok());
        // An address below the least input gives nothing.
        assert_eq!(
            address_plan(AddressMove::Transfer, MIN_OUTPUT, false, &[MIN_INPUT - 1]),
            Err(AddressRefusal::NotEnough { most: 0 })
        );
    }

    #[test]
    fn at_most_sixteen_addresses_pay() {
        let balances = vec![DASH / 100; 20];
        let all = address_plan(AddressMove::Transfer, 0, true, &balances).unwrap();
        assert_eq!(all.inputs.len(), MAX_INPUTS);
        // Seventeen addresses' worth cannot be sent at once.
        let more = address_plan(AddressMove::Transfer, 17 * (DASH / 100), false, &balances);
        assert!(
            matches!(more, Err(AddressRefusal::NotEnough { .. })),
            "{more:?}"
        );
    }

    #[test]
    fn max_leaves_nothing_on_the_addresses() {
        let balances = [DASH / 2, DASH / 4];
        let transfer = address_plan(AddressMove::Transfer, 0, true, &balances).unwrap();
        assert_eq!(
            transfer.fee_payer, None,
            "the fee from the amount (ReduceOutput)"
        );
        assert_eq!(given(&transfer), DASH / 2 + DASH / 4);
        assert_eq!(transfer.receives, DASH / 2 + DASH / 4 - transfer.fee);
        assert_eq!(transfer.spent(), DASH / 2 + DASH / 4);
        for kind in [AddressMove::Withdraw, AddressMove::TopUp] {
            let plan = address_plan(kind, 0, true, &balances).unwrap();
            assert_eq!(plan.spent(), DASH / 2 + DASH / 4, "{kind:?}");
            assert_eq!(plan.receives, DASH / 2 + DASH / 4 - plan.fee, "{kind:?}");
        }
        // Too little for the fee: nothing to move.
        assert!(matches!(
            address_plan(AddressMove::Withdraw, 0, true, &[DASH / 1000]),
            Err(AddressRefusal::NotEnough { .. })
        ));
    }

    /// Whatever the balances, a plan never takes more than an address holds, never gives less
    /// than the least input, and spends what it says.
    #[test]
    fn a_plan_keeps_to_the_balances() {
        let mut seed = 7u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed >> 33
        };
        for _ in 0..2_000 {
            let n = (next() % 6 + 1) as usize;
            let balances: Vec<u64> = (0..n).map(|_| next() % (2 * DASH / 1000) * 1000).collect();
            let amount = next() % (DASH / 500) + MIN_OUTPUT;
            for kind in [
                AddressMove::Transfer,
                AddressMove::Withdraw,
                AddressMove::TopUp,
            ] {
                for max in [false, true] {
                    let Ok(plan) = address_plan(kind, amount, max, &balances) else {
                        continue;
                    };
                    let mut rest = balances.clone();
                    for (i, c) in &plan.inputs {
                        assert!(*c >= MIN_INPUT, "{kind:?} {balances:?} {plan:?}");
                        rest[*i] = rest[*i].checked_sub(*c).expect("more than held");
                    }
                    if let Some(p) = plan.fee_payer {
                        assert!(rest[p] >= plan.fee, "{kind:?} {balances:?} {plan:?}");
                        // The largest keeps the fee; a top-up's transition takes it from the first.
                        if kind == AddressMove::TopUp {
                            assert_eq!(p, plan.inputs[0].0);
                        } else {
                            let largest = plan.inputs.iter().map(|(i, _)| balances[*i]).max();
                            assert_eq!(Some(balances[p]), largest, "{balances:?} {plan:?}");
                        }
                    } else {
                        assert_eq!(plan.receives + plan.fee, given(&plan));
                    }
                    assert_eq!(plan.fee, kind.fee(plan.inputs.len()));
                    assert!(plan.receives >= kind.least());
                    if !max {
                        assert_eq!(plan.receives, amount);
                        assert_eq!(given(&plan), amount);
                    }
                }
            }
        }
    }
}
