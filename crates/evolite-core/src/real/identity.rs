//! The wallet's identities on Platform: their keys (DIP-13 identity slots of the phrase, as Dash
//! Evo Tool derives them), the signer of the asset lock, and what the interface shows of them.
//! Private keys exist only in signers built for one operation from the open wallet's keys.

use async_trait::async_trait;
use dpp::identity::accessors::IdentityGettersV0;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dpp::identity::{IdentityPublicKey, KeyType, Purpose, SecurityLevel};
use dpp::platform_value::BinaryData;
use dpp::platform_value::string_encoding::Encoding;
use evolite_api::{DpnsName, Identity, IdentityKey, KeyKind, KeyLevel, KeyPurpose};
use key_wallet::bip32::{DerivationPath, ExtendedPubKey};
use key_wallet::dashcore::ecdsa::{EcdsaPublicKey, EcdsaSignature};
use key_wallet::signer::{ExtendedPubKeySigner, Signer, SignerMethod};
use platform_wallet::{ManagedIdentity, derive_identity_auth_keypair};

/// A new identity's keys: (id, purpose, level). DashPay needs ENCRYPTION to send contact
/// requests and DECRYPTION to read them; TRANSFER moves credits.
const KEYS: [(u32, Purpose, SecurityLevel); 6] = [
    (0, Purpose::AUTHENTICATION, SecurityLevel::MASTER),
    (1, Purpose::AUTHENTICATION, SecurityLevel::HIGH),
    (2, Purpose::AUTHENTICATION, SecurityLevel::CRITICAL),
    (3, Purpose::TRANSFER, SecurityLevel::CRITICAL),
    (4, Purpose::ENCRYPTION, SecurityLevel::MEDIUM),
    (5, Purpose::DECRYPTION, SecurityLevel::MEDIUM),
];

/// The keys of identity slot `index` with their private halves (key id = key index, as
/// platform-wallet's discovery expects).
pub fn new_identity_keys(
    keys: &key_wallet::Wallet,
    index: u32,
) -> Result<Vec<(IdentityPublicKey, [u8; 32])>, String> {
    KEYS.iter()
        .map(|(id, purpose, level)| {
            let (_, xprv, public) =
                derive_identity_auth_keypair(keys, crate::network::dash(), index, *id)
                    .map_err(|e| e.to_string())?;
            let key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id: *id,
                purpose: *purpose,
                security_level: *level,
                contract_bounds: None,
                key_type: KeyType::ECDSA_SECP256K1,
                read_only: false,
                data: BinaryData::new(public.to_compressed().to_vec()),
                disabled_at: None,
            });
            Ok((key, *xprv.private_key.to_bytes()))
        })
        .collect()
}

/// The private keys of an identity found in slot `index`: each published key that this
/// phrase reproduces.
pub fn found_identity_keys(
    keys: &key_wallet::Wallet,
    index: u32,
    identity: &dpp::identity::Identity,
) -> Vec<(IdentityPublicKey, [u8; 32])> {
    identity
        .public_keys()
        .iter()
        .filter(|(_, key)| key.key_type() == KeyType::ECDSA_SECP256K1)
        .filter_map(|(key_id, key)| {
            let (_, xprv, public) =
                derive_identity_auth_keypair(keys, crate::network::dash(), index, *key_id).ok()?;
            (key.data().as_slice() == public.to_compressed().as_slice())
                .then(|| (key.clone(), *xprv.private_key.to_bytes()))
        })
        .collect()
}

/// Signs the asset lock (a Core transaction) with the open wallet's keys.
pub struct AssetLockSigner(pub key_wallet::Wallet);

#[async_trait]
impl Signer for AssetLockSigner {
    type Error = String;

    fn supported_methods(&self) -> &[SignerMethod] {
        &[SignerMethod::Digest]
    }

    async fn sign_ecdsa(
        &self,
        path: &DerivationPath,
        sighash: [u8; 32],
    ) -> Result<(EcdsaSignature, EcdsaPublicKey), Self::Error> {
        let key = self.0.derive_private_key(path).map_err(|e| e.to_string())?;
        let signature = key
            .sign_recoverable(&sighash)
            .signature()
            .map_err(|e| e.to_string())?;
        Ok((signature, key.public_key()))
    }

    async fn public_key(&self, path: &DerivationPath) -> Result<EcdsaPublicKey, Self::Error> {
        Ok(self
            .0
            .derive_private_key(path)
            .map_err(|e| e.to_string())?
            .public_key())
    }
}

#[async_trait]
impl ExtendedPubKeySigner for AssetLockSigner {
    async fn extended_public_key(&self, path: &DerivationPath) -> Result<ExtendedPubKey, String> {
        self.0
            .derive_extended_public_key(path)
            .map_err(|e| e.to_string())
    }
}

/// What the interface shows of an identity platform-wallet keeps. Platform does not say when an
/// identity was made: `created_at` and `created_block` stay 0.
pub fn to_api(managed: &ManagedIdentity) -> Identity {
    let identity = &managed.identity;
    let names = managed
        .dpns_names
        .iter()
        .enumerate()
        .map(|(i, n)| DpnsName {
            label: n.label.clone(),
            primary: i == 0,
            voting_days_left: None,
        })
        .collect();
    let keys = identity
        .public_keys()
        .values()
        .filter(|k| k.disabled_at().is_none())
        .map(|k| IdentityKey {
            id: k.id(),
            level: match k.security_level() {
                SecurityLevel::MASTER => KeyLevel::Master,
                SecurityLevel::CRITICAL => KeyLevel::Critical,
                SecurityLevel::HIGH => KeyLevel::High,
                _ => KeyLevel::Medium,
            },
            purpose: match k.purpose() {
                Purpose::AUTHENTICATION if k.security_level() == SecurityLevel::MASTER => {
                    KeyPurpose::Identity
                }
                Purpose::TRANSFER => KeyPurpose::Transfers,
                Purpose::ENCRYPTION | Purpose::DECRYPTION => KeyPurpose::Encryption,
                _ => KeyPurpose::Documents,
            },
            kind: match k.key_type() {
                KeyType::ECDSA_SECP256K1 => KeyKind::Ecdsa,
                KeyType::ECDSA_HASH160 => KeyKind::EcdsaHash160,
                KeyType::BLS12_381 => KeyKind::Bls,
                KeyType::EDDSA_25519_HASH160 => KeyKind::EddsaHash160,
                KeyType::BIP13_SCRIPT_HASH => KeyKind::Bip13Script,
            },
            decrypts: k.purpose() == Purpose::DECRYPTION,
        })
        .collect();
    Identity {
        id: identity.id().to_string(Encoding::Base58),
        created_at: 0,
        created_block: 0,
        revision: identity.revision() as u32,
        key_source: managed
            .identity_index
            .map_or(tr!("not from this phrase").into(), |i| {
                format!("DIP-13 · identity {i}")
            }),
        names,
        keys,
        profile: None,
    }
}

/// Credits as duffs (1 duff = 1000 credits), as the interface counts every balance.
pub fn credits_as_duffs(credits: u64) -> i64 {
    (credits / 1000) as i64
}

/// The identity an asset lock would register: Platform makes a new identity's id from the
/// lock's outpoint (its txid and the credit output, the first), so the txid alone tells it —
/// for good, also after a restore or a restart.
pub fn registered_by(txid: &str) -> Option<String> {
    let txid: key_wallet::dashcore::Txid = txid.parse().ok()?;
    let outpoint: [u8; 36] = key_wallet::dashcore::OutPoint::new(txid, 0).into();
    Some(
        dpp::prelude::Identifier::new(dpp::util::hash::hash_double(outpoint))
            .to_string(Encoding::Base58),
    )
}

/// Which of the wallet's identities a Core asset lock (`txid`) funded, as the history names it
/// (`@name`, else the id): the one it registered, or — while platform-wallet still tracks the
/// lock — the identity of its slot. `None` for a Platform address or a lock no longer tracked.
pub fn funded_identity(
    txid: &str,
    tracked: Option<(platform_wallet::AssetLockFundingType, u32)>,
    mine: &[evolite_api::IdentityChoice],
) -> Option<String> {
    use platform_wallet::AssetLockFundingType as Funding;
    let shown = |c: &evolite_api::IdentityChoice| {
        c.name
            .as_ref()
            .map_or_else(|| c.id.clone(), |n| format!("@{n}"))
    };
    if let Some(id) = registered_by(txid)
        && let Some(choice) = mine.iter().find(|c| c.id == id)
    {
        return Some(shown(choice));
    }
    match tracked {
        Some((Funding::IdentityRegistration | Funding::IdentityTopUp, slot)) => {
            mine.iter().find(|c| c.slot == slot).map(shown)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_asset_lock_names_the_identity_it_funded() {
        use evolite_api::IdentityChoice;
        use platform_wallet::AssetLockFundingType as Funding;
        let txid = "35af26a8000000000000000000000000000000000000000000000000c0c81f18";
        let registered = registered_by(txid).unwrap();
        let mine = [
            IdentityChoice {
                id: registered.clone(),
                name: Some("alice".into()),
                slot: 0,
            },
            IdentityChoice {
                id: "Bob1111111111111111111111111111111111111111".into(),
                name: None,
                slot: 1,
            },
        ];
        // Registration: known from the txid alone, tracked or not.
        assert_eq!(
            funded_identity(txid, None, &mine).as_deref(),
            Some("@alice")
        );
        // A top-up still tracked: the identity of its slot, by id when it has no name.
        let other = "11".repeat(32);
        assert_eq!(
            funded_identity(&other, Some((Funding::IdentityTopUp, 1)), &mine).as_deref(),
            Some(mine[1].id.as_str())
        );
        // A Platform address, an untracked lock, a broken txid: not said.
        assert_eq!(
            funded_identity(&other, Some((Funding::AssetLockAddressTopUp, 0)), &mine),
            None
        );
        assert_eq!(funded_identity(&other, None, &mine), None);
        assert_eq!(funded_identity("nope", None, &mine), None);
        // Platform's own formula: the double SHA-256 of the 36-byte outpoint.
        assert_ne!(registered, registered_by(&other).unwrap());
    }

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn found_keys_are_the_keys_made_for_the_slot() {
        let keys = super::super::keys_of(PHRASE).unwrap();
        let made = new_identity_keys(&keys, 2).unwrap();
        let identity = dpp::identity::Identity::V0(dpp::identity::v0::IdentityV0 {
            id: dpp::prelude::Identifier::random(),
            public_keys: made.iter().map(|(k, _)| (k.id(), k.clone())).collect(),
            balance: 0,
            revision: 0,
        });
        assert_eq!(found_identity_keys(&keys, 2, &identity).len(), KEYS.len());
        assert!(found_identity_keys(&keys, 1, &identity).is_empty());
    }
}
