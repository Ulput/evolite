//! FEAT-0044: signed messages in dash-qt's format (`signmessage` / `verifymessage`): a compact
//! recoverable signature over the message's UTF-8 bytes as they are, base64. Checking needs no
//! wallet and no unlock — it is called by the interface directly (the architect, answer 4).
//! Signing is platform-wallet's (`CoreWallet::sign_message`), with this wallet's keys; the core
//! first allows only an address of BIP44 account 0 (R-11).

use key_wallet::dashcore::sign_message::{
    MessageSignatureError, from_base64, is_signed_by_address, signed_msg_hash,
};
use key_wallet::dashcore::{Address, AddressType, Network};
use std::str::FromStr;

pub use evolite_api::message::{Verified, VerifyError};

fn network_name(n: Network) -> String {
    match n {
        Network::Mainnet => "mainnet".into(),
        _ => "testnet".into(),
    }
}

/// Checks `signature` (base64; spaces at its ends are dropped — R-8's edge case) over `message`
/// (its bytes exactly) against `address` on `network`.
pub fn verify_message(
    address: &str,
    message: &str,
    signature: &str,
    network: Network,
) -> Result<Verified, VerifyError> {
    let unchecked =
        key_wallet::dashcore::Address::<key_wallet::dashcore::address::NetworkUnchecked>::from_str(
            address.trim(),
        )
        .map_err(|_| VerifyError::NotAnAddress)?;
    let address: Address = match unchecked.clone().require_network(network) {
        Ok(a) => a,
        Err(_) => {
            let other = if network == Network::Mainnet {
                Network::Testnet
            } else {
                Network::Mainnet
            };
            return Err(match unchecked.require_network(other) {
                Ok(_) => VerifyError::OtherNetwork(network_name(other)),
                Err(_) => VerifyError::NotAnAddress,
            });
        }
    };
    if address.address_type() != Some(AddressType::P2pkh) {
        return Err(VerifyError::NotP2pkh);
    }
    let Ok(signature) = from_base64(signature.trim()) else {
        return Ok(Verified::NotASignature);
    };
    match is_signed_by_address(&signature, &address, signed_msg_hash(message)) {
        Ok(true) => Ok(Verified::Valid),
        Ok(false) => Ok(Verified::NotThisAddress),
        Err(MessageSignatureError::UnsupportedAddressType(_)) => Err(VerifyError::NotP2pkh),
        Err(_) => Ok(Verified::NotASignature),
    }
}

#[cfg(test)]
mod tests;
