use super::*;
use key_wallet::dashcore::PublicKey;
use key_wallet::dashcore::crypto::ecdsa::EcdsaRecSignature;
use key_wallet::dashcore::hashes::Hash;
use key_wallet::dashcore::sign_message::{from_slice, to_base64};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const MESSAGE: &str = "I own this address";

/// A signature of `message` by the test phrase's first receiving key, as dash-qt makes it
/// (`compressed`: false for an older wallet's uncompressed key), and that key's address.
fn signed(message: &str, compressed: bool) -> (String, String) {
    use std::str::FromStr;
    let keys = crate::keys_of(PHRASE).unwrap();
    let path = key_wallet::DerivationPath::from_str("m/44'/1'/0'/0/0").unwrap();
    let key = keys.derive_private_key(&path).unwrap();
    let rec = key.sign_recoverable(signed_msg_hash(message).as_byte_array());
    let rec = EcdsaRecSignature::from_parts(
        rec.signature().unwrap(),
        rec.recovery_id().unwrap(),
        compressed,
    )
    .unwrap();
    let public = PublicKey {
        inner: key.public_key(),
        compressed,
    };
    (
        Address::p2pkh(&public, Network::Testnet).to_string(),
        to_base64(rec),
    )
}

/// AC-T-S2 (AC-A1, AC-A3): dash-qt's format — a signature checks against its address; another
/// message, or what is no signature, each say so.
#[test]
fn a_signature_is_checked_as_dash_qt_does() {
    let (address, signature) = signed(MESSAGE, true);
    assert_eq!(
        verify_message(&address, MESSAGE, &signature, Network::Testnet),
        Ok(Verified::Valid)
    );
    // Spaces around the signature (pasted from a letter) are dropped.
    assert_eq!(
        verify_message(
            &address,
            MESSAGE,
            &format!("  {signature}\n"),
            Network::Testnet
        ),
        Ok(Verified::Valid)
    );
    // AC-A7 (R-8): the bytes exactly — a space or a line break more is another message.
    for other in [
        format!("{MESSAGE} "),
        format!("{MESSAGE}\r\n"),
        "I own this addresS".into(),
    ] {
        assert_eq!(
            verify_message(&address, &other, &signature, Network::Testnet),
            Ok(Verified::NotThisAddress),
            "{other:?}"
        );
    }
    // Another address of this network.
    let (_, other_signature) = signed("something else", true);
    assert_eq!(
        verify_message(&address, MESSAGE, &other_signature, Network::Testnet),
        Ok(Verified::NotThisAddress)
    );
    assert_eq!(
        verify_message(&address, MESSAGE, "not base64!", Network::Testnet),
        Ok(Verified::NotASignature)
    );
    assert_eq!(
        verify_message(&address, MESSAGE, "AAAA", Network::Testnet),
        Ok(Verified::NotASignature),
        "too short"
    );
}

/// FEAT-0044 review r1, Should: an outside implementation's signature — dashj's
/// `ECKeyTest.verifyMessage` (the vector platform-wallet checks too): «hello» signed for a mainnet
/// address. A change of dashcore's prefix («DarkCoin Signed Message») or of its encoding breaks
/// this test, not only our own round trip.
#[test]
fn dashj_signature_verifies() {
    const ADDRESS: &str = "Xt5QmmzX2LaMgt81dcXGHhdf8pAhaVJKUW";
    const SIGNATURE: &str =
        "HPygR8+G/HJ0kSp0azMeW6bvzd1tGg0Nx1mCJ/ls5Yh2Z1WgA10Nc/yPbVYU4HbF8Z98vvXFC8iqGTGsdDgqfe4=";
    assert_eq!(
        verify_message(ADDRESS, "hello", SIGNATURE, Network::Mainnet),
        Ok(Verified::Valid)
    );
    assert_eq!(
        verify_message(ADDRESS, "hello ", SIGNATURE, Network::Mainnet),
        Ok(Verified::NotThisAddress)
    );
}

/// AC-T-S4, AC-A9 (R-12): an address of the other network says which; a P2SH one has no
/// signatures; what is no address says so.
#[test]
fn an_address_of_another_network_or_kind_is_said() {
    let (address, signature) = signed(MESSAGE, true);
    assert_eq!(
        verify_message(&address, MESSAGE, &signature, Network::Mainnet),
        Err(VerifyError::OtherNetwork("testnet".into()))
    );
    let pk = PublicKey::from_str_hex();
    let script = key_wallet::dashcore::ScriptBuf::new_p2pkh(&pk.pubkey_hash());
    let p2sh = Address::p2sh(&script, Network::Testnet)
        .unwrap()
        .to_string();
    assert_eq!(
        verify_message(&p2sh, MESSAGE, &signature, Network::Testnet),
        Err(VerifyError::NotP2pkh)
    );
    assert_eq!(
        verify_message("hello", MESSAGE, &signature, Network::Testnet),
        Err(VerifyError::NotAnAddress)
    );
}

/// AC-T-S3: a signature by an uncompressed key (header 27–30, older wallets) checks against the
/// uncompressed key's address and not the compressed one's; a header out of range (26, 35) is no
/// signature.
#[test]
fn an_uncompressed_signature_matches_its_own_address() {
    let (uncompressed_address, uncompressed) = signed(MESSAGE, false);
    let (compressed_address, _) = signed(MESSAGE, true);
    assert_ne!(uncompressed_address, compressed_address);
    assert_eq!(
        verify_message(
            &uncompressed_address,
            MESSAGE,
            &uncompressed,
            Network::Testnet
        ),
        Ok(Verified::Valid)
    );
    assert_eq!(
        verify_message(
            &compressed_address,
            MESSAGE,
            &uncompressed,
            Network::Testnet
        ),
        Ok(Verified::NotThisAddress)
    );
    for header in [26u8, 35] {
        let mut bytes = [0u8; 65];
        bytes[0] = header;
        let b64 = to_base64(from_slice(&bytes).unwrap());
        assert_eq!(
            verify_message(&compressed_address, MESSAGE, &b64, Network::Testnet),
            Ok(Verified::NotASignature),
            "{header}"
        );
    }
}

trait FromHex {
    fn from_str_hex() -> Self;
}

impl FromHex for PublicKey {
    /// Any valid key, for a P2SH address.
    fn from_str_hex() -> Self {
        let hex = "0351537cc127b4fa40b72dea904a34ab60dcd45132708d746b78e6c04172f90afa";
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        PublicKey::from_slice(&bytes).unwrap()
    }
}
