//! FEAT-0030 phase A: the coins of the CoinJoin account, as the real core sees and spends them.

use super::*;

/// FEAT-0030 AC-T10, the first test of phase A: the signing keys made from the phrase on unlock
/// (`keys_of`, kept in `Unlocked.keys`) hold CoinJoin account 0 (`m/9'/coin'/4'/0'`), and a
/// payment built from that account alone is signed by them — the input's signature carries the
/// public key of the coin's own address. Phase A spends mixed coins with these keys; were the
/// account missing, the plan would change before anything else.
#[tokio::test]
async fn the_unlock_keys_hold_coinjoin_account_0_and_sign_its_coins() {
    use key_wallet::dashcore::hashes::{Hash, hash160};
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::coin_selection::SelectionStrategy;
    use key_wallet::wallet::managed_wallet_info::fee::FeeRate;
    use key_wallet::wallet::managed_wallet_info::transaction_building::AccountTypePreference;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;

    let keys = keys_of(PHRASE).expect("the phrase");
    assert!(
        keys.accounts.coinjoin_accounts.contains_key(&0),
        "the unlock keys have no CoinJoin account 0"
    );
    let mut info = key_wallet::wallet::ManagedWalletInfo::from_wallet(&keys, 0);
    let account = info
        .accounts_mut()
        .coinjoin_accounts
        .get_mut(&0)
        .expect("the managed CoinJoin account 0");
    let address = account
        .keys()
        .managed_account_type()
        .all_addresses()
        .into_iter()
        .next()
        .expect("an address of the CoinJoin account");
    // One mixed-size coin (0.00100001 DASH) on that address, in a block.
    let mut coin = key_wallet::Utxo::new(
        key_wallet::dashcore::OutPoint::new(
            key_wallet::dashcore::Txid::from_byte_array([7; 32]),
            0,
        ),
        key_wallet::dashcore::TxOut {
            value: 100_001,
            script_pubkey: address.script_pubkey(),
        },
        address.clone(),
        1_570_000,
        false,
    );
    coin.is_confirmed = true;
    account.utxos.insert(coin.outpoint, coin);

    let to: key_wallet::dashcore::Address = key_wallet::dashcore::Address::new(
        key_wallet::Network::Testnet,
        key_wallet::dashcore::address::Payload::PubkeyHash(
            key_wallet::dashcore::PubkeyHash::from_byte_array([2; 20]),
        ),
    );
    let (tx, _fee) = info
        .build_and_sign_transaction(
            &keys,
            &[AccountTypePreference::CoinJoin],
            0,
            vec![(to.as_unchecked().clone(), 90_000)],
            FeeRate::normal(),
            SelectionStrategy::All,
        )
        .await
        .expect("a payment of the CoinJoin coin, signed");
    assert_eq!(tx.input.len(), 1, "{tx:?}");
    // P2PKH: the signature, then the public key; its hash is the coin's address.
    let pushes: Vec<Vec<u8>> = tx.input[0]
        .script_sig
        .instructions()
        .filter_map(|i| i.ok()?.push_bytes().map(|b| b.as_bytes().to_vec()))
        .collect();
    assert_eq!(pushes.len(), 2, "a signature and a key: {pushes:?}");
    let key_hash = hash160::Hash::hash(&pushes[1]);
    assert_eq!(
        address.script_pubkey(),
        key_wallet::dashcore::ScriptBuf::new_p2pkh(
            &key_wallet::dashcore::PubkeyHash::from_raw_hash(key_hash)
        ),
        "signed by the key of the coin's own address"
    );
}

/// FEAT-0030 AC-T7: «Mixed» pays only a Dash address. A contact by `@name` and a Platform
/// address are refused with their reason, before anything is read or signed; a Dash address
/// passes that check (and stops at the sync here: this core has no chain running).
#[tokio::test]
async fn mixed_coins_go_only_to_a_dash_address() {
    let core = opened("mixed-payee", "pw-mixed");
    let refused = |r: Result<MixedQuote, CoreError>, words: &str| match r {
        Err(CoreError::NotAvailable(why)) => why.contains(words),
        _ => false,
    };
    assert!(refused(
        core.preview_mixed("@bob", 100_000).await,
        "only to a Core address"
    ));
    let platform = PlatformAddress::P2pkh([7; 20]).to_bech32m_string(key_wallet::Network::Testnet);
    assert!(refused(
        core.preview_mixed(&platform, 100_000).await,
        "do not go to Platform"
    ));
    assert!(matches!(
        core.preview_mixed("yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ", 100_000)
            .await,
        Err(CoreError::NotSynced)
    ));
}

/// АРХ-9а in the core: the quote's rule from the excess, an amount without excess with a
/// refusal, and the excess agreed to — a larger one only when it is the very one built.
#[test]
fn the_excess_is_quoted_and_must_be_agreed_to() {
    use evolite_chain::mixed_pay::MixedPlan;
    let plan = |excess: u64, fee: u64, value: u64| MixedPlan {
        coins: vec![key_wallet::Utxo::new(
            key_wallet::dashcore::OutPoint::default(),
            key_wallet::dashcore::TxOut {
                value,
                script_pubkey: Default::default(),
            },
            key_wallet::dashcore::Address::new(
                key_wallet::Network::Testnet,
                key_wallet::dashcore::address::Payload::PubkeyHash(
                    key_wallet::dashcore::PubkeyHash::from_byte_array([1; 20]),
                ),
            ),
            1,
            false,
        )],
        fee,
        excess,
    };
    use key_wallet::dashcore::hashes::Hash;
    // An excess of 0.0049 DASH on a payment of 0.99 DASH needs a confirmation.
    let confirm = ops::mixed::quote(&plan(490_000, 226, 100_001_000), 99_000_000);
    assert_eq!(confirm.rule, MixedExcess::Confirm);
    assert_eq!(confirm.suggested, None);
    assert!(ops::mixed::excess_agreed(&confirm, confirm.excess).is_ok());
    assert_eq!(
        ops::mixed::excess_agreed(&confirm, 0),
        Err(CoreError::MixedExcess {
            excess: confirm.excess,
            suggested: None
        }),
        "not agreed: the excess it would be now, to confirm again"
    );
    assert!(
        ops::mixed::excess_agreed(&confirm, confirm.excess - 1).is_err(),
        "another one"
    );
    // A tiny excess needs nothing.
    let free = ops::mixed::quote(&plan(500, 226, 1_000_010), 999_284);
    assert_eq!(free.rule, MixedExcess::Free);
    assert!(ops::mixed::excess_agreed(&free, 0).is_ok());
    assert!(ops::mixed::excess_agreed(&free, 500).is_ok());
    // Over 0.01 DASH: refused, with what the same coins pay without excess.
    let refuse = ops::mixed::quote(&plan(9_000_000, 226, 10_000_100), 1_000_000);
    assert_eq!(refuse.rule, MixedExcess::Refuse);
    assert_eq!(refuse.suggested, Some(10_000_100 - 226));
    assert_eq!(
        ops::mixed::excess_agreed(&refuse, refuse.excess),
        Err(CoreError::MixedExcess {
            excess: refuse.excess,
            suggested: refuse.suggested
        }),
        "over the bound: refused with the amount that has no excess"
    );
}

/// The screens and the demo say and answer a payment from «Mixed» by the thresholds of
/// `evolite_coinjoin` (АРХ-9а): they build without that crate and keep a copy in `evolite-api`.
#[test]
fn the_screens_burn_an_excess_by_the_cores_thresholds() {
    use evolite_chain::coinjoin::{EXCESS_FREE, EXCESS_FREE_PERCENT, EXCESS_MAX};
    assert_eq!(
        (
            evolite_api::MIXED_EXCESS_FREE,
            evolite_api::MIXED_EXCESS_FREE_PERCENT,
            evolite_api::MIXED_EXCESS_MAX
        ),
        (
            EXCESS_FREE as Duffs,
            EXCESS_FREE_PERCENT as Duffs,
            EXCESS_MAX as Duffs
        )
    );
}

/// FEAT-0030 screens review r1, P3 2: `Spend::Send` from «Mixed» is refused before anything is
/// read — mixed coins go only as `Spend::SendMixed`, with their excess agreed (АРХ-9а).
#[tokio::test]
async fn an_ordinary_payment_from_mixed_is_refused() {
    let mut core = opened("mixed-as-send", "pw-mixed");
    let result = core
        .send(
            1,
            Source::Mixed,
            "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ",
            10_000,
            (None, None, None),
        )
        .await;
    assert!(
        matches!(&result, Err(CoreError::NotAvailable(why)) if why.contains("from mixed coins")),
        "{result:?}"
    );
}
