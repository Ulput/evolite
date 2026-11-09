//! evolite's wrappers of dash-spv, platform-wallet and the Platform SDK: the chain and its sync,
//! the store, and what is read from or sent to Platform.

#![forbid(unsafe_code)]

#[macro_use]
extern crate evolite_common;

mod apps;
pub mod convert;
pub mod network;
mod platform;
mod spv;
pub mod store;

pub use apps::{app_chain, app_connect};
pub use platform::{dashpay, identity, nfts, platform_history, pool};
pub use spv::{chain, mined, provider, sync, turn};
pub use store::store_refusal;

use evolite_api::CoreError;
use evolite_art as art;
use evolite_common::{diag, dpns, layers};
use evolite_local::{folders, journal, json_file, node, pending};
use key_wallet::mnemonic::Mnemonic;

/// Signing keys from the phrase; key-wallet's `Wallet` signs Core transactions itself.
pub fn keys_of(phrase: &str) -> Result<key_wallet::Wallet, CoreError> {
    let mnemonic = Mnemonic::from_phrase(phrase).map_err(|_| CoreError::BadPhrase)?;
    key_wallet::Wallet::from_mnemonic(
        mnemonic,
        crate::network::dash(),
        key_wallet::wallet::initialization::WalletAccountCreationOptions::Default,
    )
    .map_err(|_| CoreError::BadPhrase)
}

#[cfg(test)]
use evolite_common::hex;

/// A value from `.env.test.local` at the workspace root (never committed): the live tests'
/// phrases and the names of the test accounts stay out of the repository.
#[cfg(any(test, feature = "test-support"))]
pub fn test_env(key: &str) -> Option<zeroize::Zeroizing<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env.test.local");
    let text = zeroize::Zeroizing::new(std::fs::read_to_string(path).ok()?);
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| zeroize::Zeroizing::new(v.trim().trim_matches('"').to_owned()))
        .filter(|v| !v.is_empty())
}

/// A test account's DPNS name from `.env.test.local`, without the `@`.
#[cfg(any(test, feature = "test-support"))]
pub fn test_name(key: &str) -> String {
    test_env(key)
        .map(|v| v.trim_start_matches('@').to_owned())
        .unwrap_or_else(|| panic!("{key} in .env.test.local"))
}
