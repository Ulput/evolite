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
