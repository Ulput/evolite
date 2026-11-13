//! CoinJoin for evolite (FEAT-0030). Phase A: what the wallet needs to show and spend coins that
//! were mixed elsewhere (DashPay, Dash Core) — the denominations, how many rounds of mixing a coin
//! has been through, and which of the wallet's transactions are mixing ones. Phase B will add the
//! mixing client here.
//!
//! R-1: this crate depends on no evolite crate, only on `dashcore`. The wallet is seen through
//! [`History`]: its own transactions and which scripts are its own.

use dashcore::{OutPoint, Script, Transaction, Txid};
use std::collections::HashMap;

/// The amounts a mixing session works in, largest first: 10.0001, 1.00001, 0.100001, 0.0100001 and
/// 0.00100001 DASH, in duffs (Dash Core, `coinjoin/common.h`, `vecStandardDenominations`).
pub const DENOMINATIONS: [u64; 5] = [1_000_010_000, 100_001_000, 10_000_100, 1_000_010, 100_001];

/// A collateral coin is worth from a tenth of the smallest denomination to four times that:
/// 0.0001 to 0.0004 DASH (Dash Core, `IsCollateralAmount`).
pub const COLLATERAL: std::ops::RangeInclusive<u64> = 10_000..=40_000;

/// The rounds after which a coin counts as fully mixed, and is spent from «Mixed»: 4, as Dash
/// Core's default and DashPay's «Intermediate». One place, until the customer decides otherwise
/// (FEAT-0030, исслед-1).
pub const FULLY_MIXED_ROUNDS: u8 = 4;

/// How deep the count follows a coin's history (Dash Core's `MAX_COINJOIN_ROUNDS`). Dash Core adds
/// a random 0–3 of its own to it; a fixed limit changes nothing below it, and
/// [`FULLY_MIXED_ROUNDS`] is far below.
pub const MAX_ROUNDS: u8 = 16;

/// Whether `value` (duffs) is one of the [`DENOMINATIONS`].
pub fn is_denominated(value: u64) -> bool {
    DENOMINATIONS.contains(&value)
}

/// Whether `value` (duffs) is a collateral amount ([`COLLATERAL`]).
pub fn is_collateral(value: u64) -> bool {
    COLLATERAL.contains(&value)
}

/// An excess a payment from «Mixed» burns without asking: up to 0.001 DASH (АРХ-9а). Mixed coins
/// give no change, so what the chosen coins hold over the amount and the fee goes to the miners
/// with the fee (the customer, арх-30-1, as Dash Core does).
pub const EXCESS_FREE: u64 = 100_000;

/// ... and up to this share of the amount, in percent.
pub const EXCESS_FREE_PERCENT: u64 = 1;

/// The most a payment from «Mixed» may burn at all, with the person's confirmation: 0.01 DASH.
/// Above it the core refuses and suggests an amount that burns nothing.
pub const EXCESS_MAX: u64 = 1_000_000;

/// What the core does with the excess of a payment from «Mixed» (АРХ-9а).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Excess {
    /// Within [`EXCESS_FREE`] and [`EXCESS_FREE_PERCENT`] of the amount: paid as part of the fee.
    Free,
    /// Above either, up to [`EXCESS_MAX`]: only when the person confirmed this very excess.
    Confirm,
    /// Above [`EXCESS_MAX`]: refused.
    Refuse,
}

/// [`Excess`] of `excess` burnt on a payment of `amount` (both in duffs).
pub fn excess_rule(amount: u64, excess: u64) -> Excess {
    if excess > EXCESS_MAX {
        Excess::Refuse
    } else if excess > EXCESS_FREE
        || excess.saturating_mul(100) > amount.saturating_mul(EXCESS_FREE_PERCENT)
    {
        Excess::Confirm
    } else {
        Excess::Free
    }
}

/// The wallet as the count sees it.
pub trait History {
    /// One of the wallet's own transactions, whole.
    fn transaction(&self, txid: &Txid) -> Option<&Transaction>;
    /// Whether an output with `script` is the wallet's own.
    fn is_ours(&self, script: &Script) -> bool;
}

/// How far a coin is mixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounds {
    /// A denomination, through this many rounds (0: made by the preparation, or mixed with
    /// something else in the same transaction).
    Mixed(u8),
    /// Not a denomination: it is not mixed at all.
    NotDenominated,
    /// A collateral coin, kept to pay a session's fee.
    Collateral,
    /// Not one of the wallet's transactions, or no such output.
    Unknown,
}

impl Rounds {
    /// Whether the coin has been through at least [`FULLY_MIXED_ROUNDS`].
    pub fn fully_mixed(self) -> bool {
        matches!(self, Rounds::Mixed(n) if n >= FULLY_MIXED_ROUNDS)
    }
}

/// Counts the rounds of the wallet's coins, remembering each outpoint it has counted: one pass over
/// the history for every coin of a snapshot (FEAT-0030 AC-T8). Make a new one when the history
/// changes.
pub struct RoundCounter<'h, H: History> {
    history: &'h H,
    /// Dash Core's own codes: -1 unknown, -2 not denominated, -3 collateral, -4 no such output,
    /// -10 being counted (a loop guard), 0 and up the rounds.
    seen: HashMap<OutPoint, i32>,
}

impl<'h, H: History> RoundCounter<'h, H> {
    pub fn new(history: &'h H) -> Self {
        Self {
            history,
            seen: HashMap::new(),
        }
    }

    /// The rounds of `coin`, by Dash Core's rule (`GetRealOutpointCoinJoinRounds`).
    pub fn rounds(&mut self, coin: &OutPoint) -> Rounds {
        match self.real(coin, 0) {
            -2 => Rounds::NotDenominated,
            -3 => Rounds::Collateral,
            n if n >= 0 => Rounds::Mixed(n.min(i32::from(u8::MAX)) as u8),
            _ => Rounds::Unknown,
        }
    }

    /// Dash Core's `GetRealOutpointCoinJoinRounds`, `depth` its `nRounds`:
    /// - not a denomination: -2; a collateral: -3;
    /// - a denomination in a transaction with another output that is not one, or where the wallet
    ///   put in more than it got out (a fee): 0;
    /// - else one more than the fewest rounds of the wallet's own inputs that are denominations,
    ///   or 0 when there is none — at most [`MAX_ROUNDS`].
    fn real(&mut self, coin: &OutPoint, depth: u8) -> i32 {
        let max = i32::from(MAX_ROUNDS);
        if depth >= MAX_ROUNDS {
            return max - 1;
        }
        if let Some(&known) = self.seen.get(coin) {
            return known;
        }
        self.seen.insert(*coin, -10);
        let found = self.count(coin, depth, max);
        self.seen.insert(*coin, found);
        found
    }

    fn count(&mut self, coin: &OutPoint, depth: u8, max: i32) -> i32 {
        // The history outlives this borrow of `self`: its transactions can be held while counting.
        let history: &'h H = self.history;
        let Some(tx) = history.transaction(&coin.txid) else {
            return -1;
        };
        let Some(out) = tx.output.get(coin.vout as usize) else {
            return -4;
        };
        if is_collateral(out.value) {
            return -3;
        }
        if !is_denominated(out.value) {
            return -2;
        }
        if tx.output.iter().any(|o| !is_denominated(o.value)) {
            return 0;
        }
        let (debit, credit) = our_flow(tx, history);
        if debit != credit {
            return 0;
        }
        let mut shortest: Option<i32> = None;
        for input in &tx.input {
            if !self.is_our_input(&input.previous_output) {
                continue;
            }
            let n = self.real(&input.previous_output, depth + 1);
            if n >= 0 && shortest.is_none_or(|s| n < s) {
                shortest = Some(n);
            }
        }
        match shortest {
            Some(n) if n >= max - 1 => max,
            Some(n) => n + 1,
            None => 0,
        }
    }

    fn is_our_input(&self, prev: &OutPoint) -> bool {
        self.history
            .transaction(&prev.txid)
            .and_then(|t| t.output.get(prev.vout as usize))
            .is_some_and(|o| self.history.is_ours(&o.script_pubkey))
    }
}

/// What the wallet put into `tx` (its own inputs, as far as their transactions are known) and what
/// it got out of it (its own outputs), in duffs — Dash Core's `GetDebit` and `GetCredit`.
fn our_flow(tx: &Transaction, history: &impl History) -> (u64, u64) {
    let debit = tx
        .input
        .iter()
        .filter_map(|i| {
            let prev = history
                .transaction(&i.previous_output.txid)?
                .output
                .get(i.previous_output.vout as usize)?;
            history.is_ours(&prev.script_pubkey).then_some(prev.value)
        })
        .sum();
    let credit = tx
        .output
        .iter()
        .filter(|o| history.is_ours(&o.script_pubkey))
        .map(|o| o.value)
        .sum();
    (debit, credit)
}

/// Whether `tx` is a mixing transaction of the wallet, by evolite's rule (FEAT-0030 A3): as many
/// inputs as outputs, every output a denomination, and the wallet took part with denominations
/// only and got out exactly what it put in. key-wallet's own mark needs at least 3 inputs and 3
/// outputs, so a session of two on testnet would not be seen; the history and the count of rounds
/// use this one.
pub fn is_mixing(tx: &Transaction, history: &impl History) -> bool {
    let ours_in: Vec<u64> = tx
        .input
        .iter()
        .filter_map(|i| {
            let prev = history
                .transaction(&i.previous_output.txid)?
                .output
                .get(i.previous_output.vout as usize)?;
            history.is_ours(&prev.script_pubkey).then_some(prev.value)
        })
        .collect();
    let (_, credit) = our_flow(tx, history);
    mixing_shape(tx, &ours_in, credit)
}

/// [`is_mixing`] from what the wallet knows of `tx` itself: the values of its own inputs
/// (`ours_in`) and what its own outputs got (`ours_out`). The history line uses it on a record's
/// details, with no other transaction at hand.
pub fn mixing_shape(tx: &Transaction, ours_in: &[u64], ours_out: u64) -> bool {
    !tx.input.is_empty()
        && tx.input.len() == tx.output.len()
        && tx.output.iter().all(|o| is_denominated(o.value))
        && !ours_in.is_empty()
        && ours_in.iter().all(|v| is_denominated(*v))
        && ours_in.iter().sum::<u64>() == ours_out
}

#[cfg(test)]
mod tests;
