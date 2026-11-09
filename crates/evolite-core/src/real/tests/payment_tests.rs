use super::*;

/// BUG-55 review, should 2: a small payment from a wallet of many coins is shown with the
/// fee of the one coin it takes, as the Send screen estimated it.
#[test]
fn a_sending_payment_is_shown_with_the_fee_of_its_own_coins() {
    let many = vec![100_000_000_u64; 30];
    let one = super::payment_fee(vec![100_000_000], 1_000_000);
    assert_eq!(super::payment_fee(many.clone(), 1_000_000), one);
    assert!(one < super::fees(many.len()).core_payment);
}

#[test]
fn platform_addresses_only_of_the_network_in_force() {
    use evolite_api::Network::{Mainnet, Testnet};
    let address = |network| PlatformAddress::P2pkh([7; 20]).to_bech32m_string(network);
    let (main, test) = (
        address(key_wallet::Network::Mainnet),
        address(key_wallet::Network::Testnet),
    );
    assert!(platform_address_on(&test, Testnet) && !platform_address_on(&main, Testnet));
    // On mainnet its own addresses go through and testnet's are refused — before, the
    // other way round.
    assert!(platform_address_on(&main, Mainnet) && !platform_address_on(&test, Mainnet));
    assert!(!platform_address_on("tdash1 not an address", Testnet));
    assert!(main.starts_with(hrp_of(Mainnet)) && !main.starts_with(hrp_of(Testnet)));
    assert!(test.starts_with(hrp_of(Testnet)) && !test.starts_with(hrp_of(Mainnet)));
}

#[test]
fn app_links_name_the_network_in_force() {
    use evolite_api::Network::{Mainnet, Testnet};
    assert_eq!(app_connect::link_network(Testnet), Some("t"));
    // The key-exchange contract is testnet's: no app sign-in on mainnet yet.
    assert_eq!(app_connect::link_network(Mainnet), None);
}

/// BUG-93 review must 1, should 5: `top_up_identity` itself refuses an amount whose lock
/// Platform would not take, before anything else; the least amount gets as far as the network.
#[tokio::test]
async fn a_top_up_below_platforms_least_lock_is_refused_at_once() {
    let mut core = core_with_vault("top-up-least", "pw");
    let meta = core.saved.clone().unwrap();
    core.unlocked = Some(Unlocked {
        id: [0; 32],
        meta,
        keys: None,
        seed: None,
    });
    assert_eq!(
        core.top_up_identity(40_499).await,
        Err(CoreError::BadAmount)
    );
    assert_eq!(
        core.top_up_identity(40_500).await,
        Err(CoreError::NotSynced)
    );
}

#[tokio::test]
async fn credits_refuse_bad_payees_before_asking_the_network() {
    let mut core = core_with_vault("credits", "pw");
    let meta = core.saved.clone().unwrap();
    core.unlocked = Some(Unlocked {
        id: [0; 32],
        meta,
        keys: None,
        seed: None,
    });
    let address = |network| PlatformAddress::P2pkh([7; 20]).to_bech32m_string(network);
    let mainnet = address(key_wallet::Network::Mainnet);
    let testnet = address(crate::network::dash());
    assert!(testnet.starts_with("tdash1"), "{testnet}");
    assert_eq!(
        core.send(Source::Credits, &mainnet, 1_000, None).await,
        Err(CoreError::BadAddress)
    );
    assert_eq!(
        core.send(Source::Credits, "@a_b", 1_000, None).await,
        Err(CoreError::BadName)
    );
    assert_eq!(
        core.send(Source::Credits, &testnet, 0, None).await,
        Err(CoreError::BadAmount)
    );
    // A good payee gets as far as the network: proofs cannot be checked yet.
    assert_eq!(
        core.send(Source::Credits, &testnet, 1_000, None).await,
        Err(CoreError::NotSynced)
    );
    // From Core a Platform address is refused with the reason, not as a bad address.
    assert!(matches!(
        core.send(Source::Core, &testnet, 1_000, None).await,
        Err(CoreError::NotAvailable(_))
    ));
}

#[test]
fn counts_what_a_write_cost_in_duffs() {
    assert_eq!(spent_credits(Some(5_000_000), Some(4_000_000)), Some(1_000));
    // Unknown, or the balance did not go down (a refresh had not landed): not guessed.
    assert_eq!(spent_credits(None, Some(1)), None);
    assert_eq!(spent_credits(Some(1_000), Some(1_000)), None);
    assert_eq!(spent_credits(Some(1_000), Some(2_000)), None);
}

/// A disconnect is answered as `disconnect`, as the demo does: the screen takes `app` for the
/// answer to the request it shows.
#[tokio::test]
async fn a_disconnect_is_not_taken_for_an_answer_to_a_request() {
    let mut core = core_with_vault("disconnect", "pw");
    let (event, _) = core
        .handle(Command::DisconnectApp { id: "app".into() })
        .await;
    assert!(matches!(
        event,
        Some(Event::Done {
            what: "disconnect",
            ..
        })
    ));
}

/// An app's link is refused at once when it is not one or is for another network, and a
/// request is never approved that the person was not shown.
#[tokio::test]
async fn app_links_are_checked_before_anything_is_signed() {
    let mut core = core_with_vault("app-links", "pw");
    let request = |text: &str| Command::OpenAppLink { text: text.into() };
    let (event, _) = core.handle(request("https://evil.example")).await;
    assert!(matches!(
        event,
        Some(Event::AppRequest(Err(CoreError::BadLink)))
    ));
    let (event, _) = core
        .handle(request("dash-key:3mJr7AoUXx2Wqd?n=m&v=1"))
        .await;
    assert!(matches!(
        event,
        Some(Event::AppRequest(Err(CoreError::NotAvailable(_))))
    ));
    // Declining is always answered; approving a request never shown is refused.
    let answer = |id, approve| match approve {
        true => Command::Spend {
            op: Spend::ApproveApp { id },
            password: Some(pw("pw")),
        },
        false => Command::DeclineApp { id },
    };
    let (event, _) = core.handle(answer(7, false)).await;
    assert!(matches!(
        event,
        Some(Event::Done {
            what: "app",
            result: Ok(())
        })
    ));
    let (event, _) = core.handle(answer(7, true)).await;
    assert!(matches!(
        event,
        Some(Event::Done {
            what: "app",
            result: Err(CoreError::NotAvailable(_))
        })
    ));
}

/// BUG-77 r1 should 1: a payment from Core (send, top-up, a new identity) is checked against
/// the coins each once. werner555 had 7 coins in two accounts: platform-wallet's balance
/// counted them twice and would have let a payment of more than the wallet has through.
#[test]
fn a_coin_two_accounts_hold_does_not_pay_twice() {
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let keys = keys_of(PHRASE).unwrap();
    let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
    let address = key_wallet::dashcore::Address::new(
        key_wallet::Network::Testnet,
        key_wallet::dashcore::address::Payload::PubkeyHash(
            key_wallet::dashcore::PubkeyHash::from_byte_array([2; 20]),
        ),
    );
    let mut coin = key_wallet::Utxo::new(
        key_wallet::dashcore::OutPoint::new(
            key_wallet::dashcore::Txid::from_byte_array([2; 32]),
            0,
        ),
        key_wallet::dashcore::TxOut {
            value: 5_050_000,
            script_pubkey: Default::default(),
        },
        address,
        1_570_000,
        false,
    );
    coin.is_confirmed = true;
    {
        let mut accounts = info.accounts_mut().all_funding_accounts_mut();
        accounts[0].utxos.insert(coin.outpoint, coin.clone());
        accounts[1].utxos.insert(coin.outpoint, coin);
    }
    info.update_balance();
    assert_eq!(info.balance().total(), 10_100_000, "upstream: twice");
    assert!(core_covers(&info, 5_050_000).is_ok());
    assert!(matches!(
        core_covers(&info, 5_050_001),
        Err(CoreError::NotEnough)
    ));
    assert!(matches!(
        core_covers(&info, 10_100_000),
        Err(CoreError::NotEnough)
    ));
}

/// R-4: a payment below the dust limit is refused before anything is reserved or signed; at the
/// limit it goes on, to the next check (here: the sync).
#[tokio::test(flavor = "multi_thread")]
async fn a_dust_payment_is_refused_before_it_is_built() {
    let mut core = core_with_vault("dust", "pw");
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    const TO: &str = "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ";
    let below = core.send(Source::Core, TO, 545, None).await;
    assert!(
        matches!(&below, Err(CoreError::NotAvailable(why)) if why.contains("too small")),
        "{below:?}"
    );
    assert_eq!(
        core.send(Source::Core, TO, 546, None).await,
        Err(CoreError::NotSynced)
    );
}

/// R-5: what is still waiting for its block or lock is said as that, not as too little money;
/// a contact's payment, which the library funds with any coin, waits for others' coins.
#[test]
fn a_payment_waits_for_coins_that_are_not_final() {
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::wallet::ManagedWalletInfo;
    let keys = keys_of(PHRASE).unwrap();
    let wallet = |coins: &[(u8, u64, bool, bool)]| {
        let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
        let address = key_wallet::dashcore::Address::new(
            key_wallet::Network::Testnet,
            key_wallet::dashcore::address::Payload::PubkeyHash(
                key_wallet::dashcore::PubkeyHash::from_byte_array([2; 20]),
            ),
        );
        let mut accounts = info.accounts_mut().all_funding_accounts_mut();
        for (n, value, is_final, trusted) in coins {
            let mut coin = key_wallet::Utxo::new(
                key_wallet::dashcore::OutPoint::new(
                    key_wallet::dashcore::Txid::from_byte_array([*n; 32]),
                    0,
                ),
                key_wallet::dashcore::TxOut {
                    value: *value,
                    script_pubkey: Default::default(),
                },
                address.clone(),
                1_570_000,
                false,
            );
            coin.is_confirmed = *is_final;
            coin.is_trusted = *trusted;
            accounts[0].utxos.insert(coin.outpoint, coin);
        }
        drop(accounts);
        info
    };
    let waits = |r: Result<(), CoreError>| matches!(r, Err(CoreError::NotAvailable(why)) if why.contains("not confirmed yet"));
    let mixed = wallet(&[(1, 1_000_000, true, false), (2, 5_000_000, false, false)]);
    assert_eq!(
        coins_settled(&mixed, 500_000, false),
        Ok(()),
        "final coins cover it"
    );
    assert!(
        waits(coins_settled(&mixed, 2_000_000, false)),
        "only with the waiting coin"
    );
    let settled = wallet(&[(1, 1_000_000, true, false)]);
    assert_eq!(
        coins_settled(&settled, 2_000_000, false),
        Ok(()),
        "left to `core_covers`"
    );
    assert!(
        waits(coins_settled(&mixed, 500_000, true)),
        "a contact: others' coin waits"
    );
    let own = wallet(&[(1, 1_000_000, true, false), (2, 5_000_000, false, true)]);
    assert_eq!(
        coins_settled(&own, 500_000, true),
        Ok(()),
        "a contact: own change is no risk"
    );
}

/// `(value, in a block, InstantSend-locked, own change)` of each coin.
fn wallet_of_coins(coins: &[(u64, bool, bool, bool)]) -> key_wallet::wallet::ManagedWalletInfo {
    use key_wallet::dashcore::hashes::Hash;
    let keys = keys_of(PHRASE).unwrap();
    let mut info = key_wallet::wallet::ManagedWalletInfo::from_wallet(&keys, 0);
    let address = key_wallet::dashcore::Address::new(
        key_wallet::Network::Testnet,
        key_wallet::dashcore::address::Payload::PubkeyHash(
            key_wallet::dashcore::PubkeyHash::from_byte_array([2; 20]),
        ),
    );
    let mut accounts = info.accounts_mut().all_funding_accounts_mut();
    for (n, (value, confirmed, instant, trusted)) in coins.iter().enumerate() {
        let mut coin = key_wallet::Utxo::new(
            key_wallet::dashcore::OutPoint::new(
                key_wallet::dashcore::Txid::from_byte_array([n as u8 + 1; 32]),
                0,
            ),
            key_wallet::dashcore::TxOut {
                value: *value,
                script_pubkey: Default::default(),
            },
            address.clone(),
            1_570_000,
            false,
        );
        coin.is_confirmed = *confirmed;
        coin.is_instantlocked = *instant;
        coin.is_trusted = *trusted;
        accounts[0].utxos.insert(coin.outpoint, coin);
    }
    drop(accounts);
    info
}

fn waits(result: Result<(), CoreError>) -> bool {
    matches!(result, Err(CoreError::NotAvailable(why)) if why.contains("not confirmed yet"))
}

const CONFIRMED: (bool, bool, bool) = (true, false, false);
const LOCKED: (bool, bool, bool) = (false, true, false);
const OTHERS: (bool, bool, bool) = (false, false, false);
const OWN_CHANGE: (bool, bool, bool) = (false, false, true);

fn coin(value: u64, (confirmed, instant, trusted): (bool, bool, bool)) -> (u64, bool, bool, bool) {
    (value, confirmed, instant, trusted)
}

/// R-5 r1 must 1: a contact's payment waits only for a coin the library would take, largest
/// first up to the amount and fee. Someone's coin that never gets mined stays in the wallet with
/// no end, and must not close payments it has no part in.
#[test]
fn a_contact_payment_waits_only_for_coins_it_would_take() {
    let check = |coins: &[(u64, bool, bool, bool)], amount| {
        coins_settled(&wallet_of_coins(coins), amount, true)
    };
    let big_and_stuck = [coin(100_000_000, CONFIRMED), coin(1_000, OTHERS)];
    assert_eq!(check(&big_and_stuck, 500_000), Ok(()), "never reached");
    let needs_both = [coin(1_000_000, CONFIRMED), coin(800_000, OTHERS)];
    assert!(waits(check(&needs_both, 1_500_000)), "reached");
    let larger = [coin(1_000_000, CONFIRMED), coin(5_000_000, OTHERS)];
    assert!(waits(check(&larger, 500_000)), "the largest comes first");
    let tie = [coin(1_000_000, CONFIRMED), coin(1_000_000, OTHERS)];
    assert!(waits(check(&tie, 500_000)), "a tie goes against it");
    let own = [coin(1_000_000, CONFIRMED), coin(800_000, OWN_CHANGE)];
    assert_eq!(check(&own, 1_500_000), Ok(()), "own change is no risk");
    let edge = [coin(1_000_000, CONFIRMED), coin(100_000, OTHERS)];
    assert_eq!(
        check(&edge, 999_000),
        Ok(()),
        "the first coin pays it and its fee"
    );
    assert!(
        waits(check(&edge, 1_000_000)),
        "the fee takes the next coin"
    );
}

/// R-5 r1 should 1: an InstantSend-locked coin, the usual case on Dash, is final on both paths.
#[test]
fn an_instantsend_locked_coin_is_final() {
    let locked = wallet_of_coins(&[coin(3_000_000, LOCKED), coin(5_000_000, OTHERS)]);
    assert_eq!(coins_settled(&locked, 2_000_000, false), Ok(()));
    let only_locked = wallet_of_coins(&[coin(5_000_000, LOCKED)]);
    assert_eq!(coins_settled(&only_locked, 2_000_000, true), Ok(()));
}
