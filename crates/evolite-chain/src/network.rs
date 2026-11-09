//! The network this process runs on (`evolite_common::network`) and what only the real core
//! needs of it: rust-dashcore's name of it and Platform's addresses.

pub use evolite_common::network::*;

use evolite_api::Network;

/// The network as rust-dashcore and key-wallet name it.
pub fn dash() -> key_wallet::Network {
    match current() {
        Network::Testnet => key_wallet::Network::Testnet,
        Network::Mainnet => key_wallet::Network::Mainnet,
    }
}

/// The start of this network's Platform and private (Orchard) addresses: `tdash1` on testnet,
/// `dash1` on mainnet.
pub fn platform_hrp() -> &'static str {
    hrp_of(current())
}

pub fn hrp_of(network: Network) -> &'static str {
    match network {
        Network::Testnet => "tdash1",
        Network::Mainnet => "dash1",
    }
}

/// Whether `to` is a Platform address of this run's network: the same key's address on the
/// other network would take the credits somewhere the person did not mean, and this network's
/// own addresses must work.
pub fn platform_address_here(to: &str) -> bool {
    platform_address_on(to, current())
}

pub fn platform_address_on(to: &str, network: Network) -> bool {
    matches!(
        dpp::address_funds::PlatformAddress::is_mainnet_bech32m(to),
        Ok(mainnet) if mainnet == (network == Network::Mainnet)
    )
}
