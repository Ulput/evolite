//! App Connect v2 keys and the response envelope (FEAT-0023, round 2), as the DIP sets them:
//! "Session Keys", "Encryption Keys" and "Encryption Envelope".
//!
//! Session keys are derived from the phrase, not made up: the same request on the same phrase
//! gives the same keys, so evolite and DashPay on one phrase recognise each other's sessions after
//! a restore. The app receives the private keys inside the response, encrypted to its ephemeral
//! key; the response document is public, the envelope is what keeps them the app's.

use dpp::dashcore::hashes::{Hash, hash160, sha256d};
use dpp::secp256k1::{PublicKey, SecretKey};
use key_wallet::bip32::{ApplicationKeyPurpose, ChildNumber, DerivationPath};
use zeroize::Zeroizing;

use super::EncPurpose;

/// HKDF-SHA256 of `ikm` with `salt` and the parts of `info`, 32 bytes.
pub(crate) fn hkdf(ikm: &[u8], salt: &[u8], info: &[&[u8]]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    ::hkdf::Hkdf::<sha2::Sha256>::new(Some(salt), ikm)
        .expand_multi_info(info, out.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 length");
    out
}

/// The X coordinate of `secret · point`, unhashed, as the app computes it.
pub(crate) fn ecdh_x(secret: &SecretKey, point: &PublicKey) -> Zeroizing<[u8; 32]> {
    let xy = Zeroizing::new(dpp::secp256k1::ecdh::shared_secret_point(point, secret));
    let mut x = Zeroizing::new([0u8; 32]);
    x.copy_from_slice(&xy[..32]);
    x
}

/// HKDF salt and info of the envelope key, and the prefix of its associated data.
const SALT: &[u8] = b"dash:key-exchange:v1";
const INFO: &[u8] = b"app-connect:v2";
const AAD_PREFIX: &[u8] = b"dash:app-connect:v2";

/// One private key in the response.
pub const SLOT: usize = 32;
/// The nonce and the tag around the slots.
const NONCE: usize = 12;
const TAG: usize = 16;

/// The leaf of a session key: `hash256(appEphPub ‖ u8(bindingIndex))`. The index makes each
/// binding's key its own (Platform refuses two keys with the same hash on an identity); the bare
/// `hash256(appEphPub)` is reserved for the earlier one-key protocol.
pub fn request_id(app_ephemeral: &[u8; 33], binding_index: u8) -> [u8; 32] {
    let mut data = [0u8; 34];
    data[..33].copy_from_slice(app_ephemeral);
    data[33] = binding_index;
    sha256d::Hash::hash(&data).to_byte_array()
}

/// `m/9'/coin'/5'/6'/0'/identityId'/requestId'`, the ids DIP-14 256-bit hardened indexes.
pub fn session_key_path(
    network: key_wallet::Network,
    identity_id: [u8; 32],
    request_id: [u8; 32],
) -> DerivationPath {
    DerivationPath::application_session_authentication_path(network, identity_id, request_id)
}

/// `m/9'/coin'/5'/7'/0'/identityId'/contractId'/purpose'/rotation'`.
///
/// The last level, `rotation'`, is added here: key-wallet aca0202's
/// `application_encryption_path` predates it and stops at `purpose'`. When rust-dashcore moves
/// to a revision with the rotation level, take its function and drop this extension (the DIP's key
/// rows in the tests say whether the two agree).
pub fn encryption_key_path(
    network: key_wallet::Network,
    identity_id: [u8; 32],
    contract_id: [u8; 32],
    purpose: EncPurpose,
    rotation: u32,
) -> DerivationPath {
    let purpose = match purpose {
        EncPurpose::Encryption => ApplicationKeyPurpose::Encryption,
        EncPurpose::Decryption => ApplicationKeyPurpose::Decryption,
    };
    DerivationPath::application_encryption_path(network, identity_id, contract_id, purpose)
        .extend([ChildNumber::Hardened { index: rotation }])
}

/// The private key at `path` of the phrase's wallet, as the envelope's slot takes it: kept in
/// `Zeroizing` up to the seal, since session keys go to another party (review r2 P3).
/// key-wallet's own key type is another secp256k1 crate's, so only the bytes cross.
pub fn derive(
    keys: &key_wallet::Wallet,
    path: &DerivationPath,
) -> Result<Zeroizing<[u8; SLOT]>, String> {
    let key = keys.derive_private_key(path).map_err(|e| e.to_string())?;
    Ok(Zeroizing::new(*key.to_bytes()))
}

/// The response's associated data: `"dash:app-connect:v2" ‖ hash160(appEphPub) ‖ ownerId`.
pub fn associated_data(app_ephemeral: &[u8; 33], owner: &[u8; 32]) -> Vec<u8> {
    let mut aad = AAD_PREFIX.to_vec();
    aad.extend_from_slice(&hash160::Hash::hash(app_ephemeral).to_byte_array());
    aad.extend_from_slice(owner);
    aad
}

/// The envelope's key: HKDF-SHA256 of the ECDH X coordinate of the two ephemeral keys.
pub fn envelope_key(
    wallet_ephemeral: &SecretKey,
    app_ephemeral: &PublicKey,
) -> Zeroizing<[u8; 32]> {
    hkdf(
        ecdh_x(wallet_ephemeral, app_ephemeral).as_ref(),
        SALT,
        &[INFO],
    )
}

/// `encryptedPayload`: `nonce ‖ AES-256-GCM(key, nonce, slots, aad) ‖ tag`, the slots the
/// granted auth bindings' private keys in order, then the granted enc bindings'.
pub fn seal(
    slots: &[Zeroizing<[u8; SLOT]>],
    wallet_ephemeral: &SecretKey,
    app_ephemeral: &[u8; 33],
    owner: &[u8; 32],
    nonce: [u8; NONCE],
) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    let point = PublicKey::from_slice(app_ephemeral).map_err(|e| e.to_string())?;
    let key = envelope_key(wallet_ephemeral, &point);
    let mut plain = Zeroizing::new(Vec::with_capacity(slots.len() * SLOT));
    for slot in slots {
        plain.extend_from_slice(slot.as_ref());
    }
    let aad = associated_data(app_ephemeral, owner);
    let sealed = aes_gcm::Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|e| e.to_string())?
        .encrypt(
            aes_gcm::Nonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &aad,
            },
        )
        .map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(NONCE + sealed.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    debug_assert_eq!(out.len(), NONCE + TAG + slots.len() * SLOT);
    Ok(out)
}

/// What the app does with the response: the slots back, or `None`. Used by the tests, which play
/// the app.
#[cfg(test)]
pub fn open(
    payload: &[u8],
    app_secret: &SecretKey,
    wallet_ephemeral: &PublicKey,
    app_ephemeral: &[u8; 33],
    owner: &[u8; 32],
) -> Option<Vec<[u8; SLOT]>> {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    let key = envelope_key(app_secret, wallet_ephemeral);
    let (nonce, sealed) = payload.split_at_checked(NONCE)?;
    let aad = associated_data(app_ephemeral, owner);
    let plain = aes_gcm::Aes256Gcm::new_from_slice(key.as_ref())
        .ok()?
        .decrypt(
            aes_gcm::Nonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: &aad,
            },
        )
        .ok()?;
    (plain.len() % SLOT == 0).then(|| {
        plain
            .chunks(SLOT)
            .map(|c| c.try_into().expect("a slot"))
            .collect()
    })
}

#[cfg(test)]
mod tests;
