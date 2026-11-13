//! A payment from «Mixed» (FEAT-0030 phase A, AC-T4): only fully mixed coins of the CoinJoin
//! account, chosen here, and no change. Mixed coins give no change — change would link them back
//! — so the recipient gets exactly the amount and what the coins hold over it goes to the miners
//! as the fee (the customer, арх-30-1, as Dash Core does). key-wallet's builder burns on its own
//! only a remainder up to dust; for the rest the fee rate is raised until the fee takes it all
//! ([`burning_rate`]). The excess is bounded by evolite-coinjoin's [`evolite_coinjoin::excess_rule`].

use key_wallet::Utxo;
use key_wallet::dashcore::{Address, Transaction};
use key_wallet::wallet::managed_wallet_info::coin_selection::SelectionStrategy;
use key_wallet::wallet::managed_wallet_info::fee::{FeeRate, estimate_tx_size};
use key_wallet::wallet::managed_wallet_info::transaction_builder::{
    BuilderError, TransactionBuilder,
};

/// The coins a payment from «Mixed» spends, and what it costs.
#[derive(Debug, Clone, PartialEq)]
pub struct MixedPlan {
    pub coins: Vec<Utxo>,
    /// The fee at the usual rate for these inputs and one output.
    pub fee: u64,
    /// What the coins hold over the amount and `fee`: burnt with the fee.
    pub excess: u64,
}

impl MixedPlan {
    /// What the coins hold.
    pub fn total(&self) -> u64 {
        self.coins.iter().map(Utxo::value).sum()
    }
}

/// Why «Mixed» cannot pay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MixedShort {
    /// The fully mixed coins do not cover the amount and the fee: at most `available` can be sent.
    NotEnough { available: u64 },
}

/// The usual fee of a payment from «Mixed» with `inputs` inputs: one output, no change.
pub fn usual_fee(inputs: usize) -> u64 {
    crate::convert::fee_rate().calculate_fee(estimate_tx_size(inputs.max(1), 1, false))
}

/// The coins among `spendable` (fully mixed, final) that pay `amount` and its fee with the least
/// excess; none of them could be left out. Mixed coins are denominations, so going from the
/// largest down finds it nearly always; an extra pass drops any coin not needed after all.
pub fn plan(spendable: &[Utxo], amount: u64) -> Result<MixedPlan, MixedShort> {
    let mut coins: Vec<&Utxo> = spendable.iter().collect();
    coins.sort_by_key(|c| std::cmp::Reverse(c.value()));
    let all: u64 = coins.iter().map(|c| c.value()).sum();
    if all < amount + usual_fee(coins.len()) {
        return Err(MixedShort::NotEnough {
            available: all.saturating_sub(usual_fee(coins.len())),
        });
    }
    let sum = |taken: &[&Utxo]| taken.iter().map(|c| c.value()).sum::<u64>();
    let covers = |taken: &[&Utxo]| sum(taken) >= amount + usual_fee(taken.len());
    // Largest first, each coin only if it does not overshoot what is still missing.
    let mut taken: Vec<&Utxo> = Vec::new();
    let mut rest: Vec<&Utxo> = Vec::new();
    for c in coins {
        let missing = (amount + usual_fee(taken.len() + 1)).saturating_sub(sum(&taken));
        if !covers(&taken) && c.value() <= missing {
            taken.push(c);
        } else {
            rest.push(c);
        }
    }
    // Still short: the smallest coin left that closes the gap, else the largest and again.
    while !covers(&taken) {
        let missing = (amount + usual_fee(taken.len() + 1)).saturating_sub(sum(&taken));
        let pick = rest
            .iter()
            .enumerate()
            .filter(|(_, c)| c.value() >= missing)
            .min_by_key(|(_, c)| c.value())
            .or_else(|| rest.iter().enumerate().max_by_key(|(_, c)| c.value()))
            .map(|(i, _)| i)
            .expect("all the coins cover it");
        taken.push(rest.remove(pick));
    }
    // A coin the others cover without goes back, smallest first.
    taken.sort_by_key(|c| c.value());
    let mut i = 0;
    while i < taken.len() {
        let mut without = taken.clone();
        without.remove(i);
        if !without.is_empty() && covers(&without) {
            taken = without;
        } else {
            i += 1;
        }
    }
    let fee = usual_fee(taken.len());
    let excess = sum(&taken) - amount - fee;
    Ok(MixedPlan {
        coins: taken.into_iter().cloned().collect(),
        fee,
        excess,
    })
}

/// The payment of `amount` to `to` from exactly `coins`, at `rate`: no change address, so the
/// builder takes the remainder into the fee when it is dust and refuses otherwise.
pub fn mixed_builder(
    to: &Address,
    amount: u64,
    coins: &[Utxo],
    rate: FeeRate,
) -> TransactionBuilder {
    TransactionBuilder::new()
        .require_final_inputs()
        .add_output(to, amount)
        .add_inputs(coins.iter().cloned())
        .set_selection_strategy(SelectionStrategy::LargestFirst)
        .set_fee_rate(rate)
}

/// The fee rate at which a payment of `amount` to `to` from the coins of `plan` has one output,
/// worth `amount`, and a fee of everything else: the highest rate the builder still takes (a
/// higher one asks for more than the coins hold; at a lower one the remainder is above dust and
/// would need change). Found by a dry run, unsigned, nothing reserved.
pub fn burning_rate(to: &Address, amount: u64, plan: &MixedPlan) -> Result<FeeRate, String> {
    let total = plan.total();
    let dry = |rate: u64| -> Result<(Transaction, u64), BuilderError> {
        let (tx, fee, _) =
            mixed_builder(to, amount, &plan.coins, FeeRate::new(rate)).build_unsigned_reserved()?;
        Ok((tx, fee))
    };
    let burns_all = |rate: u64| {
        matches!(dry(rate), Ok((tx, fee)) if tx.output.len() == 1
            && tx.input.len() == plan.coins.len() && fee == total - amount)
    };
    let usual = crate::convert::fee_rate().as_sat_per_kb();
    if burns_all(usual) {
        return Ok(FeeRate::new(usual));
    }
    // The highest rate that builds at all lies between the usual one and the one at which the
    // fee alone would be the whole excess on the smallest size the builder may assume.
    let smallest = estimate_tx_size(plan.coins.len(), 1, false) as u64;
    let (mut low, mut high) = (usual, (total - amount) * 1000 / smallest.max(1) + 1000);
    // `low` builds (with change or burning all), `high` does not.
    if dry(low).is_err() && !matches!(dry(low), Err(BuilderError::NoChangeAddress)) {
        return Err(format!("the coins do not pay {amount} at the usual rate"));
    }
    // Too high a rate: the coins do not cover the fee, said by the builder or by its selection.
    let short = |e: &BuilderError| {
        matches!(
            e,
            BuilderError::InsufficientFunds { .. }
                | BuilderError::CoinSelection(
                    key_wallet::wallet::managed_wallet_info::coin_selection::SelectionError::InsufficientFunds { .. }
                )
        )
    };
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        match dry(mid) {
            Err(e) if short(&e) => high = mid,
            _ => low = mid,
        }
    }
    if burns_all(low) {
        Ok(FeeRate::new(low))
    } else {
        Err(format!(
            "no fee rate burns the excess of {total} over {amount}"
        ))
    }
}

/// The signed payment from «Mixed», checked before anything is sent (AC-T4, as FEAT-0013's
/// АРХ-2): its inputs are exactly the coins of `plan`, it has one output, to `to`, worth exactly
/// `amount`. Its fee is then the coins less the amount: the fee and the excess shown, to the
/// duff. No change, so nothing goes back to an address of the CoinJoin account.
pub fn check_signed(
    tx: &Transaction,
    to: &Address,
    amount: u64,
    plan: &MixedPlan,
) -> Result<(), String> {
    let mut spent: Vec<_> = tx.input.iter().map(|i| i.previous_output).collect();
    let mut chosen: Vec<_> = plan.coins.iter().map(|c| c.outpoint).collect();
    spent.sort();
    chosen.sort();
    if spent != chosen {
        return Err("the payment spends other coins than the ones chosen".into());
    }
    let [only] = tx.output.as_slice() else {
        return Err(format!("{} outputs, not one", tx.output.len()));
    };
    if only.script_pubkey != to.script_pubkey() || only.value != amount {
        return Err("the output is not the amount to the recipient".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{OutPoint, PubkeyHash, TxOut, Txid};

    fn address(n: u8) -> Address {
        Address::new(
            key_wallet::Network::Testnet,
            key_wallet::dashcore::address::Payload::PubkeyHash(PubkeyHash::from_byte_array(
                [n; 20],
            )),
        )
    }

    fn coin(n: u8, value: u64) -> Utxo {
        let at = address(n);
        let mut u = Utxo::new(
            OutPoint::new(Txid::from_byte_array([n; 32]), 0),
            TxOut {
                value,
                script_pubkey: at.script_pubkey(),
            },
            at,
            1_570_000,
            false,
        );
        u.is_confirmed = true;
        u
    }

    const TENTH: u64 = 10_000_100;
    const HUNDREDTH: u64 = 1_000_010;
    const THOUSANDTH: u64 = 100_001;

    /// AC-T4, its first test (АРХ-9а): key-wallet's builder pays exactly the amount from exactly
    /// the chosen coins, with no change, when the fee rate is raised to burn the excess — the
    /// recipient gets what was typed and the miners the rest. Were this impossible, a payment from
    /// «Mixed» would need our own signing (the fallback, +1 day).
    #[test]
    fn the_builder_burns_the_excess_at_a_raised_rate() {
        let spendable = [
            coin(1, TENTH),
            coin(2, HUNDREDTH),
            coin(3, HUNDREDTH),
            coin(4, THOUSANDTH),
        ];
        let to = address(9);
        for amount in [10_500_000, 10_090_000, 1_500_000, 50_000] {
            let plan = plan(&spendable, amount).expect("covered");
            let rate = burning_rate(&to, amount, &plan).expect("a rate that burns it");
            let (tx, fee, _) = mixed_builder(&to, amount, &plan.coins, rate)
                .build_unsigned_reserved()
                .expect("built");
            assert_eq!(tx.output.len(), 1, "{amount}: no change");
            assert_eq!(tx.output[0].value, amount, "{amount}: exactly the amount");
            assert_eq!(
                tx.input.len(),
                plan.coins.len(),
                "{amount}: every chosen coin"
            );
            assert_eq!(fee, plan.fee + plan.excess, "{amount}: the fee is the rest");
            assert_eq!(fee, plan.total() - amount);
        }
    }

    /// The choice takes the coins with the least excess and none it does not need.
    #[test]
    fn the_plan_takes_the_least_excess() {
        let spendable = [
            coin(1, TENTH),
            coin(2, HUNDREDTH),
            coin(3, HUNDREDTH),
            coin(4, THOUSANDTH),
        ];
        let p = plan(&spendable, 10_500_000).unwrap();
        let mut values: Vec<u64> = p.coins.iter().map(Utxo::value).collect();
        values.sort_unstable();
        assert_eq!(values, [HUNDREDTH, TENTH]);
        // 0.0005 needs one thousandth, not a hundredth.
        let small = plan(&spendable, 50_000).unwrap();
        assert_eq!(small.coins.len(), 1);
        assert_eq!(small.coins[0].value(), THOUSANDTH);
        assert_eq!(small.excess, THOUSANDTH - 50_000 - small.fee);
    }

    /// Not enough fully mixed coins: refused with how much could be sent, before any build.
    #[test]
    fn not_enough_mixed_coins_is_refused_with_the_most_possible() {
        let spendable = [coin(1, HUNDREDTH), coin(2, THOUSANDTH)];
        let all = HUNDREDTH + THOUSANDTH;
        assert_eq!(
            plan(&spendable, all),
            Err(MixedShort::NotEnough {
                available: all - usual_fee(2)
            })
        );
        assert_eq!(plan(&[], 1), Err(MixedShort::NotEnough { available: 0 }));
    }

    /// AC-T4: the check after signing passes the payment built from the plan and catches an
    /// extra input, an extra output (change) and another amount — such a payment is abandoned,
    /// never sent.
    #[test]
    fn the_signed_payment_is_checked_against_the_plan() {
        let spendable = [coin(1, TENTH), coin(2, HUNDREDTH), coin(4, THOUSANDTH)];
        let to = address(9);
        let amount = 10_500_000;
        let plan = plan(&spendable, amount).unwrap();
        let rate = burning_rate(&to, amount, &plan).unwrap();
        let (tx, _, _) = mixed_builder(&to, amount, &plan.coins, rate)
            .build_unsigned_reserved()
            .unwrap();
        assert_eq!(check_signed(&tx, &to, amount, &plan), Ok(()));

        let mut extra_input = tx.clone();
        extra_input.input.push(key_wallet::dashcore::TxIn {
            previous_output: coin(4, THOUSANDTH).outpoint,
            ..Default::default()
        });
        assert!(check_signed(&extra_input, &to, amount, &plan).is_err());

        let mut change = tx.clone();
        change.output.push(TxOut {
            value: 1_000,
            script_pubkey: address(5).script_pubkey(),
        });
        assert!(check_signed(&change, &to, amount, &plan).is_err());

        let mut less = tx.clone();
        less.output[0].value -= 1;
        assert!(check_signed(&less, &to, amount, &plan).is_err());

        let elsewhere = address(8);
        assert!(check_signed(&tx, &elsewhere, amount, &plan).is_err());
    }
}
