//! The one network this process runs on: testnet, unless the app was started with
//! `--network mainnet`. Chosen once, before the real core starts ([`crate::Core::real_on`]), and
//! never changed while it runs: keys, addresses, the SPV client, Platform's SDK and the data
//! folders all follow it. Checks (ChainLocks, InstantLocks, proofs) are the same on both.

use evolite_api::Network;
use std::sync::OnceLock;

static CHOSEN: OnceLock<Network> = OnceLock::new();

/// Sets the network for this process; the first choice stays. Returns whether `network` is
/// the one in force.
pub fn choose(network: Network) -> bool {
    *CHOSEN.get_or_init(|| network) == network
}

/// The network in force: testnet until another was chosen.
pub fn current() -> Network {
    CHOSEN.get().copied().unwrap_or(Network::Testnet)
}

/// Its name in folders and in the vault file: "testnet", "mainnet".
pub fn name() -> &'static str {
    name_of(current())
}

pub fn name_of(network: Network) -> &'static str {
    match network {
        Network::Testnet => "testnet",
        Network::Mainnet => "mainnet",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn testnet_unless_chosen_and_the_first_choice_stays() {
        // Tests share the process: only testnet may be chosen here.
        assert_eq!(current(), Network::Testnet);
        assert!(choose(Network::Testnet));
        assert!(
            !choose(Network::Mainnet),
            "a second choice does not change it"
        );
        assert_eq!(current(), Network::Testnet);
        assert_eq!(name(), "testnet");
        assert_eq!(name_of(Network::Mainnet), "mainnet");
    }
}
