//! FEAT-0048 step 2: an asset lock paid only with fully mixed denominations — to create an
//! identity, top one up, or shield into the pool. The coins are ours to choose
//! (`evolite_coinjoin::pick_denominations`); platform-wallet's own builder takes CoinJoin coins
//! only whole-account (the architect, М-1). Here: the transaction and its check. Every input's
//! value, less the Core fee, goes to the one credit output (the architect, М-4: nothing burnt,
//! no change, no other input).

use key_wallet::Utxo;
use key_wallet::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
use key_wallet::dashcore::blockdata::transaction::special_transaction::asset_lock::AssetLockPayload;
use key_wallet::dashcore::{ScriptBuf, Transaction, TxOut};
use key_wallet::wallet::managed_wallet_info::coin_selection::SelectionStrategy;
use key_wallet::wallet::managed_wallet_info::transaction_builder::TransactionBuilder;

/// The lock paying the one-time key `credit` (its script) with exactly `coins`. key-wallet's
/// builder turns the payload into one OP_RETURN burn output and, taking every input (`All`),
/// writes the inputs less the fee into both it and the single credit output; there is no change
/// output. The credit's value here is only a floor (`0`), as platform-wallet's own drain path.
pub fn lock_builder(coins: &[Utxo], credit: ScriptBuf) -> TransactionBuilder {
    lock_builder_at(coins, credit, crate::convert::fee_rate())
}

/// [`lock_builder`] at `rate`.
fn lock_builder_at(
    coins: &[Utxo],
    credit: ScriptBuf,
    rate: key_wallet::wallet::managed_wallet_info::fee::FeeRate,
) -> TransactionBuilder {
    TransactionBuilder::new()
        .require_final_inputs()
        .add_inputs(coins.iter().cloned())
        .set_selection_strategy(SelectionStrategy::All)
        .set_fee_rate(rate)
        .set_special_payload(TransactionPayload::AssetLockPayloadType(
            AssetLockPayload::new(vec![TxOut {
                value: 0,
                script_pubkey: credit,
            }]),
        ))
}

/// The most a lock of `inputs` coins may pay in fee (FEAT-0048 r1, the architect): ten times the
/// usual fee of its size, and never less than 0.001 DASH. A wrong rate or size estimate would
/// otherwise burn the coins into the fee; past it the lock is refused before it is signed.
pub fn fee_ceiling(inputs: usize) -> u64 {
    (10 * crate::mixed_pay::usual_fee(inputs)).max(100_000)
}

/// What a lock built by [`lock_builder`] must be before it is signed away: exactly `coins` as
/// inputs, one output (the burn), an asset-lock payload with one credit output to `credit`, the
/// credit worth the burn, and a fee paid that is not above [`fee_ceiling`]. The credit's value,
/// in duffs.
pub fn check_lock(tx: &Transaction, coins: &[Utxo], credit: &ScriptBuf) -> Result<u64, String> {
    let mut spent: Vec<_> = tx.input.iter().map(|i| i.previous_output).collect();
    let mut chosen: Vec<_> = coins.iter().map(|u| u.outpoint).collect();
    spent.sort();
    chosen.sort();
    if spent != chosen {
        return Err("The lock spends other coins than the chosen ones".into());
    }
    let [burn] = tx.output.as_slice() else {
        return Err("The lock has an output besides its burn".into());
    };
    if !burn.script_pubkey.is_op_return() {
        return Err("The lock's output is not its burn".into());
    }
    let Some(TransactionPayload::AssetLockPayloadType(payload)) = &tx.special_transaction_payload
    else {
        return Err("The lock carries no asset-lock payload".into());
    };
    let [out] = payload.credit_outputs.as_slice() else {
        return Err("The lock has more than one credit output".into());
    };
    if out.script_pubkey != *credit {
        return Err("The lock's credit goes to another key".into());
    }
    let total: u64 = coins.iter().map(|u| u.txout.value).sum();
    if out.value == 0 || out.value != burn.value || out.value >= total {
        return Err("The lock's credit is not the coins less the fee".into());
    }
    if total - out.value > fee_ceiling(coins.len()) {
        return Err("The lock's fee is far above the usual: nothing was sent".into());
    }
    Ok(out.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{Address, OutPoint, PubkeyHash, Txid};

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

    fn denominations() -> Vec<Utxo> {
        (1..=3).map(|n| coin(n, 1_000_010)).collect()
    }

    /// A9: 3 × 0.0100001 → one burn, one credit of the coins less the fee, no change, only the
    /// chosen coins spent.
    #[test]
    fn a_lock_from_three_hundredths_is_one_credit_less_the_fee() {
        let coins = denominations();
        let credit = address(9).script_pubkey();
        let (tx, fee, _) = lock_builder(&coins, credit.clone())
            .build_unsigned_reserved()
            .unwrap();
        let value = check_lock(&tx, &coins, &credit).unwrap();
        assert_eq!(
            value + fee,
            3 * 1_000_010,
            "everything but the fee is credited"
        );
        assert!(fee > 0 && fee < 10_000, "a Core fee: {fee}");
    }

    /// FEAT-0048 r1: a lock whose fee is far above the usual (a wrong rate) is refused before it
    /// is signed; the usual one passes under the ceiling.
    #[test]
    fn a_lock_burning_its_coins_into_the_fee_is_refused() {
        use key_wallet::wallet::managed_wallet_info::fee::FeeRate;
        let coins = denominations();
        let credit = address(9).script_pubkey();
        let (usual, fee, _) = lock_builder(&coins, credit.clone())
            .build_unsigned_reserved()
            .unwrap();
        assert!(fee <= fee_ceiling(coins.len()));
        assert!(check_lock(&usual, &coins, &credit).is_ok());
        // At a thousand times the rate the fee (≈ 0.005) passes the ceiling (0.001 here).
        let rate = crate::convert::fee_rate().as_sat_per_kb() * 1_000;
        let (burning, _, _) = lock_builder_at(&coins, credit.clone(), FeeRate::new(rate))
            .build_unsigned_reserved()
            .unwrap();
        let refused = check_lock(&burning, &coins, &credit).unwrap_err();
        assert!(refused.contains("far above"), "{refused}");
    }

    /// The check refuses what the lock must never be: another coin, a change output, another
    /// key, two credit outputs.
    #[test]
    fn the_check_refuses_another_shape() {
        let coins = denominations();
        let credit = address(9).script_pubkey();
        let (tx, _, _) = lock_builder(&coins, credit.clone())
            .build_unsigned_reserved()
            .unwrap();

        let mut other = coins.clone();
        other[0] = coin(7, 1_000_010);
        assert!(check_lock(&tx, &other, &credit).is_err(), "other coins");

        let mut change = tx.clone();
        change.output.push(TxOut {
            value: 1_000,
            script_pubkey: address(8).script_pubkey(),
        });
        assert!(
            check_lock(&change, &coins, &credit).is_err(),
            "a change output"
        );

        assert!(
            check_lock(&tx, &coins, &address(8).script_pubkey()).is_err(),
            "another key"
        );

        let mut two = tx.clone();
        if let Some(TransactionPayload::AssetLockPayloadType(p)) =
            &mut two.special_transaction_payload
        {
            p.credit_outputs.push(p.credit_outputs[0].clone());
        }
        assert!(check_lock(&two, &coins, &credit).is_err(), "two credits");

        let mut none = tx;
        none.special_transaction_payload = None;
        assert!(check_lock(&none, &coins, &credit).is_err(), "no payload");
    }
}
