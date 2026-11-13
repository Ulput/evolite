//! The rounds of coins on made-up chains of transactions, by Dash Core's rule (FEAT-0030 AC-T1).

use super::*;
use dashcore::hashes::Hash;
use dashcore::{PubkeyHash, ScriptBuf, TxIn, TxOut};
use std::collections::HashSet;

/// A wallet of made-up transactions. Scripts numbered below 100 are its own, the others belong to
/// other people.
#[derive(Default)]
struct Wallet {
    txs: HashMap<Txid, Transaction>,
    ours: HashSet<ScriptBuf>,
    next: u32,
}

impl History for Wallet {
    fn transaction(&self, txid: &Txid) -> Option<&Transaction> {
        self.txs.get(txid)
    }
    fn is_ours(&self, script: &Script) -> bool {
        self.ours.contains(script)
    }
}

fn script(n: u8) -> ScriptBuf {
    ScriptBuf::new_p2pkh(&PubkeyHash::from_byte_array([n; 20]))
}

/// The smallest denomination, the one these chains move.
const D: u64 = 100_001;

impl Wallet {
    /// A transaction of the wallet spending `inputs`, paying `(value, script number)` each; its
    /// first output's outpoint.
    fn tx(&mut self, inputs: &[OutPoint], outputs: &[(u64, u8)]) -> Txid {
        self.next += 1;
        let tx = Transaction {
            version: 2,
            lock_time: self.next,
            input: inputs
                .iter()
                .map(|o| TxIn {
                    previous_output: *o,
                    ..Default::default()
                })
                .collect(),
            output: outputs
                .iter()
                .map(|&(value, n)| {
                    if n < 100 {
                        self.ours.insert(script(n));
                    }
                    TxOut {
                        value,
                        script_pubkey: script(n),
                    }
                })
                .collect(),
            special_transaction_payload: None,
        };
        let txid = tx.txid();
        self.txs.insert(txid, tx);
        txid
    }

    /// Someone else's coin, not in the wallet.
    fn foreign(&mut self) -> OutPoint {
        self.next += 1;
        OutPoint::new(Txid::from_byte_array([self.next as u8; 32]), 7)
    }

    /// Money that came in: 0.05 DASH on our script 1.
    fn funded(&mut self) -> OutPoint {
        let funding = self.foreign();
        OutPoint::new(self.tx(&[funding], &[(5_000_000, 1)]), 0)
    }

    /// The preparation: our money into three denominations and change. Its denominations have 0
    /// rounds (the change is not one).
    fn prepared(&mut self) -> OutPoint {
        let money = self.funded();
        OutPoint::new(
            self.tx(&[money], &[(D, 2), (D, 3), (D, 4), (4_600_000, 5)]),
            0,
        )
    }

    /// One mixing round of `coin` with two other people: three inputs, three denominations out,
    /// ours on our script `to`.
    fn mixed(&mut self, coin: OutPoint, to: u8) -> OutPoint {
        let (a, b) = (self.foreign(), self.foreign());
        OutPoint::new(self.tx(&[a, coin, b], &[(D, to), (D, 200), (D, 201)]), 0)
    }

    fn rounds(&self, coin: &OutPoint) -> Rounds {
        RoundCounter::new(self).rounds(coin)
    }
}

#[test]
fn the_denominations_are_dash_cores() {
    assert!(is_denominated(100_001) && is_denominated(1_000_010_000));
    assert!(!is_denominated(100_000) && !is_denominated(1_000_000));
    assert!(is_collateral(10_000) && is_collateral(40_000));
    assert!(!is_collateral(9_999) && !is_collateral(40_001) && !is_collateral(D));
}

#[test]
fn a_prepared_denomination_has_0_rounds_and_the_change_is_not_one() {
    let mut w = Wallet::default();
    let coin = w.prepared();
    assert_eq!(w.rounds(&coin), Rounds::Mixed(0));
    let change = OutPoint::new(coin.txid, 3);
    assert_eq!(w.rounds(&change), Rounds::NotDenominated);
}

#[test]
fn denominations_made_with_a_fee_have_0_rounds() {
    let mut w = Wallet::default();
    let money = w.funded();
    // All outputs denominations, but the wallet put in more than it got: the fee.
    let made = w.tx(&[money], &[(D, 2), (D, 3)]);
    assert_eq!(w.rounds(&OutPoint::new(made, 0)), Rounds::Mixed(0));
}

#[test]
fn each_mixing_round_adds_one() {
    let mut w = Wallet::default();
    let mut coin = w.prepared();
    for round in 1..=4u8 {
        coin = w.mixed(coin, 10 + round);
        assert_eq!(w.rounds(&coin), Rounds::Mixed(round), "round {round}");
        assert_eq!(w.rounds(&coin).fully_mixed(), round >= FULLY_MIXED_ROUNDS);
        let tx = w.txs[&coin.txid].clone();
        assert!(is_mixing(&tx, &w), "round {round} is a mixing transaction");
    }
}

#[test]
fn a_denomination_beside_another_output_has_0_rounds() {
    let mut w = Wallet::default();
    let prepared = w.prepared();
    let coin = w.mixed(prepared, 10);
    let (a, b) = (w.foreign(), w.foreign());
    // A mixing shape, but one output is not a denomination.
    let odd = w.tx(&[a, coin, b], &[(D, 11), (D, 200), (150_000, 201)]);
    assert_eq!(w.rounds(&OutPoint::new(odd, 0)), Rounds::Mixed(0));
    assert!(!is_mixing(&w.txs[&odd].clone(), &w));
}

#[test]
fn collateral_unknown_and_missing_outputs() {
    let mut w = Wallet::default();
    let money = w.funded();
    let collateral = w.tx(&[money], &[(20_000, 6), (4_970_000, 7)]);
    assert_eq!(w.rounds(&OutPoint::new(collateral, 0)), Rounds::Collateral);
    let elsewhere = w.foreign();
    assert_eq!(w.rounds(&elsewhere), Rounds::Unknown, "not the wallet's");
    assert_eq!(
        w.rounds(&OutPoint::new(collateral, 9)),
        Rounds::Unknown,
        "no such output"
    );
}

#[test]
fn the_fewest_rounds_of_our_inputs_count() {
    let mut w = Wallet::default();
    let mut two = w.prepared();
    for n in 0..2 {
        two = w.mixed(two, 20 + n);
    }
    let mut five = w.prepared();
    for n in 0..5 {
        five = w.mixed(five, 30 + n);
    }
    let a = w.foreign();
    let both = w.tx(&[two, a, five], &[(D, 40), (D, 41), (D, 200)]);
    assert_eq!(w.rounds(&two), Rounds::Mixed(2));
    assert_eq!(w.rounds(&five), Rounds::Mixed(5));
    for vout in 0..2 {
        assert_eq!(w.rounds(&OutPoint::new(both, vout)), Rounds::Mixed(3));
    }
}

#[test]
fn a_chain_longer_than_the_limit_stops_at_it() {
    let mut w = Wallet::default();
    let mut coin = w.prepared();
    for n in 0..25u8 {
        coin = w.mixed(coin, 50 + n);
    }
    assert_eq!(w.rounds(&coin), Rounds::Mixed(MAX_ROUNDS));
}

#[test]
fn an_ordinary_payment_is_not_mixing() {
    let mut w = Wallet::default();
    let money = w.funded();
    let paid = w.tx(&[money], &[(1_000_000, 200), (3_990_000, 8)]);
    assert!(!is_mixing(&w.txs[&paid].clone(), &w));
    // Inputs and outputs alike, but our input is no denomination.
    let money = w.funded();
    let (a, b) = (w.foreign(), w.foreign());
    let odd = w.tx(&[a, money, b], &[(D, 9), (D, 200), (D, 201)]);
    assert!(!is_mixing(&w.txs[&odd].clone(), &w));
}

#[test]
fn one_counter_counts_many_coins_once_each() {
    let mut w = Wallet::default();
    let mut coins = Vec::new();
    for n in 0..20u8 {
        let mut coin = w.prepared();
        for r in 0..4u8 {
            coin = w.mixed(coin, n.wrapping_mul(5).wrapping_add(r) % 90 + 9);
        }
        coins.push(coin);
    }
    let mut counter = RoundCounter::new(&w);
    for coin in &coins {
        assert_eq!(counter.rounds(coin), Rounds::Mixed(4));
        assert_eq!(counter.rounds(coin), Rounds::Mixed(4), "the same again");
    }
}

/// R-1: the crate depends on no evolite crate — only on `dashcore`.
#[test]
fn the_crate_depends_on_no_evolite_crate() {
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a [dependencies] table");
    let deps = deps.split("\n[").next().unwrap_or(deps);
    assert!(!deps.contains("evolite"), "{deps}");
    assert!(deps.contains("dashcore"), "{deps}");
}

/// АРХ-9а: an excess within 0.001 DASH and 1 % of the amount is burnt without asking; above
/// either, up to 0.01 DASH, it needs a confirmation; above that, a refusal.
#[test]
fn the_excess_rule_has_three_steps() {
    assert_eq!(excess_rule(100_000_000, 0), Excess::Free);
    assert_eq!(excess_rule(100_000_000, 100_000), Excess::Free);
    assert_eq!(
        excess_rule(100_000_000, 100_001),
        Excess::Confirm,
        "over 0.001"
    );
    assert_eq!(excess_rule(5_000_000, 60_000), Excess::Confirm, "over 1 %");
    assert_eq!(excess_rule(5_000_000, 50_000), Excess::Free, "1 % exactly");
    assert_eq!(excess_rule(100_000_000, 1_000_000), Excess::Confirm);
    assert_eq!(excess_rule(100_000_000, 1_000_001), Excess::Refuse);
}

/// The testnet mixing transactions of `tests/data/dstx-testnet.txt`, parsed, with their txids.
fn real_mixing() -> Vec<(Txid, Transaction)> {
    let hex = |s: &str| -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    };
    include_str!("../tests/data/dstx-testnet.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let (id, raw) = l.split_once(' ').expect("txid and hex");
            let tx: Transaction =
                dashcore::consensus::deserialize(&hex(raw)).expect("a transaction");
            (id.parse().expect("a txid"), tx)
        })
        .collect()
}

/// AC-T1 on real bytes: the 4 testnet mixing transactions the research probe caught
/// (2026-11-13) parse, hash to their txids, have as many inputs as outputs and denominations only
/// — of 10.0001 DASH, the only one testnet mixed then; the other denominations are covered by
/// the made-up chains above.
#[test]
fn real_testnet_mixing_transactions_have_the_shape() {
    let all = real_mixing();
    assert_eq!(all.len(), 4);
    for (id, tx) in &all {
        assert_eq!(tx.txid(), *id, "the bytes hash to the txid");
        assert_eq!(tx.input.len(), tx.output.len(), "{id}");
        assert!(tx.input.len() >= 3, "{id}");
        assert!(
            tx.output.iter().all(|o| o.value == DENOMINATIONS[0]),
            "{id}: all 10.0001 DASH"
        );
    }
}

/// AC-T1 on real bytes: in a real testnet mixing transaction, an input made the wallet's own (a
/// prepared 0-round coin of the same denomination at that outpoint) and one output on its own
/// script make it the wallet's mixing transaction, and that output has 1 round.
#[test]
fn a_real_mixing_transaction_counts_as_one_round() {
    for (id, tx) in real_mixing() {
        let mut w = Wallet::default();
        // The input's coin, as our preparation would have made it: a denomination beside change.
        let ours = tx.input[1].previous_output;
        let prep = Transaction {
            version: 2,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: w.foreign(),
                ..Default::default()
            }],
            output: (0..=ours.vout)
                .map(|n| TxOut {
                    value: if n == ours.vout {
                        DENOMINATIONS[0]
                    } else {
                        4_000_000
                    },
                    script_pubkey: script(if n == ours.vout { 1 } else { 2 }),
                })
                .chain(std::iter::once(TxOut {
                    value: 4_000_000,
                    script_pubkey: script(2),
                }))
                .collect(),
            special_transaction_payload: None,
        };
        w.ours.insert(script(1));
        w.ours.insert(script(2));
        // The made-up preparation stands in under the real input's txid.
        w.txs.insert(ours.txid, prep);
        w.ours.insert(tx.output[0].script_pubkey.clone());
        w.txs.insert(id, tx.clone());
        assert!(is_mixing(&w.txs[&id].clone(), &w), "{id}");
        assert_eq!(w.rounds(&OutPoint::new(id, 0)), Rounds::Mixed(1), "{id}");
    }
}
