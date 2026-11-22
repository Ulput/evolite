//! What evolite does on each network (FEAT-0037, the customer's М2, 2026-11-14): one table the
//! core and the screens both read, so a change of plan is one line here.

use crate::Network;

/// A part of evolite that a network may not have yet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feature {
    /// Dash Platform tokens: the search, the balances, sending (FEAT-0019).
    Tokens,
    /// Signing in to sites and apps with App Connect.
    Apps,
    /// The Collection: NFTs of evolite-standard collections (FEAT-0054; on mainnet with the
    /// beta, the customer's decision 2026-11-22).
    Collection,
    /// Paying from a Platform address (FEAT-0015 stage 1).
    PlatformAddressPayments,
    /// Topping a Platform address up from Core (FEAT-0015 stage 2).
    PlatformAddressTopUp,
}

impl Feature {
    pub const ALL: [Feature; 5] = [
        Feature::Tokens,
        Feature::Apps,
        Feature::Collection,
        Feature::PlatformAddressPayments,
        Feature::PlatformAddressTopUp,
    ];
}

/// Whether `feature` works on `network`. Testnet has everything; the first mainnet release has
/// what the customer let in (2026-11-14: tokens yes; App Connect and the Platform address
/// pending the architect's plan; 2026-11-22: the Collection, once it reads only the evolite
/// standard, FEAT-0054).
pub fn on(feature: Feature, network: Network) -> bool {
    match network {
        Network::Testnet => true,
        Network::Mainnet => match feature {
            Feature::Tokens | Feature::Collection => true,
            Feature::Apps | Feature::PlatformAddressPayments | Feature::PlatformAddressTopUp => {
                false
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On testnet everything works.
    #[test]
    fn everything_works_on_testnet() {
        for feature in Feature::ALL {
            assert!(on(feature, Network::Testnet), "{feature:?}");
        }
    }

    /// The first mainnet release: tokens yes (the customer, 2026-11-14), the Collection with the
    /// beta (2026-11-22, FEAT-0054: the evolite standard only); App Connect and the Platform
    /// address's payments and top-ups not yet.
    #[test]
    fn mainnet_has_tokens_but_not_yet_the_rest() {
        assert!(on(Feature::Tokens, Network::Mainnet));
        assert!(on(Feature::Collection, Network::Mainnet));
        for feature in [
            Feature::Apps,
            Feature::PlatformAddressPayments,
            Feature::PlatformAddressTopUp,
        ] {
            assert!(!on(feature, Network::Mainnet), "{feature:?}");
        }
    }
}
