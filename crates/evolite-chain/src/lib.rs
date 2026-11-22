//! evolite's wrappers of dash-spv, platform-wallet and the Platform SDK: the chain and its sync,
//! the store, and what is read from or sent to Platform.

#![forbid(unsafe_code)]

#[macro_use]
extern crate evolite_common;

mod apps;
pub mod change_reserve;
pub mod coins;
pub mod convert;
pub mod kept_locks;
pub mod key_signer;
pub mod message;
pub mod mixed_pay;
/// Denominations, rounds and the excess rule of mixed coins (FEAT-0030), for the core.
pub use evolite_coinjoin as coinjoin;
pub mod network;
mod platform;
mod spv;
pub mod store;

pub use apps::{app_chain, app_chain_v2, app_connect_v2, key_update};
pub use platform::{
    contract_v1, contracts, dashpay, fund_intents, identity, lock_status, nft_series, nfts,
    platform_history, pool, pool_payout, token_history, tokens, workshop_read,
};
pub use spv::{chain, contact_scans, mined, provider, sync, turn};
pub use store::store_refusal;

use evolite_api::CoreError;
use evolite_art as art;
use evolite_common::{diag, dpns, layers};
use evolite_local::{folders, journal, json_file, node, pending};
use key_wallet::mnemonic::Mnemonic;

/// FEAT-0041 R-4: **the one way** from a phrase to its BIP-39 seed, with its passphrase (empty:
/// none, the same seed as before FEAT-0041). Everything else — the signing keys, the wallet's id,
/// platform-wallet's wallet, identities, DashPay, the pool, App Connect, the notes' key — is
/// built from this seed, never from the phrase (`no_seed_but_from_the_passphrase`). bip39 NFKDs
/// the passphrase itself (its `unicode-normalization`).
pub fn seed_from(
    phrase: &str,
    passphrase: &str,
) -> Result<zeroize::Zeroizing<[u8; 64]>, CoreError> {
    let mnemonic = Mnemonic::from_phrase(phrase).map_err(|_| CoreError::BadPhrase)?;
    Ok(zeroize::Zeroizing::new(mnemonic.to_seed(passphrase)))
}

/// Signing keys from the seed; key-wallet's `Wallet` signs Core transactions itself.
pub fn keys_from(seed: &[u8; 64]) -> Result<key_wallet::Wallet, CoreError> {
    key_wallet::Wallet::from_seed_bytes(
        *seed,
        crate::network::dash(),
        key_wallet::wallet::initialization::WalletAccountCreationOptions::Default,
    )
    .map_err(|_| CoreError::BadPhrase)
}

/// The keys of a phrase with no passphrase: for tests, which know their phrases.
#[cfg(any(test, feature = "test-support"))]
pub fn keys_of(phrase: &str) -> Result<key_wallet::Wallet, CoreError> {
    let seed = seed_from(phrase, "")?;
    keys_from(&seed)
}

#[cfg(test)]
mod seed_tests;

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
