//! FEAT-0019: the tokens of the real core on mainnet and testnet, without a network.

use super::*;
use crate::real::ops::tokens::TokenState;

/// A core whose tokens are as on `network`, with Platform's lists in (proofs could be asked).
fn core_on(name: &str, network: Network) -> (Real, Paths) {
    core_with(name, |paths| {
        TokenState::on(network, &paths.tokens, &paths.root)
    })
}

/// A core with tokens off, as on a network the matrix leaves them off on.
fn core_without_tokens(name: &str) -> (Real, Paths) {
    core_with(name, |paths| {
        TokenState::with(false, &paths.tokens, &paths.root)
    })
}

fn core_with(name: &str, tokens: impl FnOnce(&Paths) -> TokenState) -> (Real, Paths) {
    let core_dir = fresh_dir(name);
    let mut core = core_at(&core_dir, TEST_WALLET);
    let paths = Paths::new(core_dir, TEST_WALLET);
    core.tokens = tokens(&paths);
    lists_in(&core.chain);
    (core, paths)
}

fn off(event: Option<Event>) -> bool {
    matches!(
        event,
        Some(
            Event::TokenFound {
                result: Err(CoreError::NotAvailable(_)),
                ..
            } | Event::Done {
                result: Err(CoreError::NotAvailable(_)),
                ..
            }
        ) | None
    )
}

/// AC-T6: with tokens off (a network the matrix leaves them off on) no token command asks
/// anything of the network and no file is written.
#[tokio::test(flavor = "multi_thread")]
async fn with_tokens_off_no_token_request_goes_out() {
    let (mut core, paths) = core_without_tokens("tokens-off");
    let id = Identifier::from([7u8; 32]).to_string(Encoding::Base58);
    for command in [
        Command::FindToken { id: id.clone() },
        Command::SearchTokens {
            keyword: "coffee".into(),
        },
        Command::TrackToken {
            token_id: id.clone(),
            on: true,
        },
        Command::FindMyTokens,
    ] {
        let (event, _) = core.handle(command).await;
        assert!(off(event), "answered at once, off");
    }
    assert!(off(Some(core.send_token(id.clone(), "@alice".into(), 1))));
    assert_eq!(
        core.shown_tokens(Some(&id), vec![id.clone()]),
        evolite_api::tokens::Tokens::off()
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(count(&core.chain.proof_reads), 0, "nothing asked");
    assert!(
        !paths.tokens.exists(),
        "tokens.json is not opened with tokens off"
    );
    assert!(
        !evolite_chain::tokens::store::path(&paths.root).exists(),
        "nor the search's cache"
    );
}

/// The same command on testnet goes to the network, so the test above sees a real difference.
#[tokio::test(flavor = "multi_thread")]
async fn on_testnet_finding_a_token_asks_with_a_proof() {
    let (mut core, _) = core_on("tokens-testnet", Network::Testnet);
    let id = Identifier::from([7u8; 32]).to_string(Encoding::Base58);
    let (event, _) = core.handle(Command::FindToken { id }).await;
    assert!(event.is_none(), "answered from the background");
    assert!(
        counted(&core.chain.proof_reads, 1).await >= 1,
        "a proved read was asked"
    );
}

/// FEAT-0037: tokens on each network are what the network matrix says (the customer let them into
/// the first mainnet release): on, a token is asked with a proof; off, nothing is asked.
#[tokio::test(flavor = "multi_thread")]
async fn tokens_follow_the_network_matrix() {
    use evolite_api::features::{Feature, on};
    for network in [Network::Testnet, Network::Mainnet] {
        let (mut core, _) = core_on(&format!("tokens-matrix-{network:?}"), network);
        let id = Identifier::from([7u8; 32]).to_string(Encoding::Base58);
        let (event, _) = core.handle(Command::FindToken { id }).await;
        if on(Feature::Tokens, network) {
            assert!(event.is_none(), "{network:?}: answered from the background");
            assert!(
                counted(&core.chain.proof_reads, 1).await >= 1,
                "{network:?}: a proved read was asked"
            );
        } else {
            assert!(off(event), "{network:?}: off at once");
        }
    }
}
