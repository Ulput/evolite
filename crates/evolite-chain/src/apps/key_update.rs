//! FEAT-0020 (АРХ-5): one way to change an identity's keys, for the keys screen and for App
//! Connect. The identity is read again, the change planned on what Platform has now
//! ([`evolite_api::key_plan`]), the new keys derived from the phrase at the identity's slot and
//! sent in one IdentityUpdate signed by its main key ([`Chain::send_key_update`]), and the
//! identity read again to see it done.

use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dpp::identity::{KeyType, Purpose, SecurityLevel};
use dpp::platform_value::BinaryData;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use evolite_api::KeyLevel;
use evolite_api::key_plan::{self, KeyPlan, NewPurpose};
use platform_wallet::{PlatformWallet, derive_identity_auth_keypair};
use zeroize::Zeroizing;

use crate::app_chain::SignInIdentity;
use crate::app_chain_v2::NewKey;
use crate::chain::Chain;

/// The planned key `new` of identity slot `slot`, from the phrase (R-1): DIP-13 at its number,
/// as every key evolite makes; the DashPay pair bound to DashPay's contact requests.
pub fn phrase_key(
    keys: &key_wallet::Wallet,
    slot: u32,
    new: &key_plan::NewKey,
) -> Result<NewKey, String> {
    let (_, xprv, public) =
        derive_identity_auth_keypair(keys, crate::network::dash(), slot, new.id)
            .map_err(|e| e.to_string())?;
    let purpose = match new.purpose {
        NewPurpose::Authentication => Purpose::AUTHENTICATION,
        NewPurpose::Transfer => Purpose::TRANSFER,
        NewPurpose::Encryption => Purpose::ENCRYPTION,
        NewPurpose::Decryption => Purpose::DECRYPTION,
    };
    let security_level = match new.level {
        KeyLevel::Master => SecurityLevel::MASTER,
        KeyLevel::Critical => SecurityLevel::CRITICAL,
        KeyLevel::High => SecurityLevel::HIGH,
        KeyLevel::Medium => SecurityLevel::MEDIUM,
    };
    let contract_bounds = if new.dashpay {
        Some(ContractBounds::SingleContractDocumentType {
            id: Identifier::from_string(key_plan::DASHPAY_CONTRACT, Encoding::Base58)
                .map_err(|e| e.to_string())?,
            document_type_name: key_plan::DASHPAY_DOCUMENT.into(),
        })
    } else {
        None
    };
    Ok(NewKey {
        key: IdentityPublicKeyV0 {
            id: new.id,
            purpose,
            security_level,
            contract_bounds,
            key_type: KeyType::ECDSA_SECP256K1,
            read_only: false,
            data: BinaryData::new(public.to_compressed().to_vec()),
            disabled_at: None,
        }
        .into(),
        secret: Zeroizing::new(*xprv.private_key.to_bytes()),
    })
}

/// Whether `who`, read again, shows `plan` done: every new key there and acting, every one
/// disabled not acting.
pub fn shows(who: &SignInIdentity, plan: &KeyPlan) -> bool {
    let acting = |id: u32| {
        who.keys
            .iter()
            .any(|k| k.id() == id && k.disabled_at().is_none())
    };
    plan.add.iter().all(|k| acting(k.id)) && plan.disable.iter().all(|id| !acting(*id))
}

impl Chain {
    /// Sends `plan` for `who` (read again just before, its numbers planned on that) and reads
    /// the identity again. An error after the send may still have changed the keys: the
    /// caller reads again before anything else, never sends it twice by itself (R-7, AC-T4).
    pub async fn update_keys(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        who: &SignInIdentity,
        plan: &KeyPlan,
    ) -> Result<SignInIdentity, String> {
        let add = plan
            .add
            .iter()
            .map(|k| phrase_key(keys, who.slot, k))
            .collect::<Result<Vec<_>, _>>()?;
        self.send_key_update(wallet, keys, who, add, plan.disable.clone())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;

    /// The BIP-39 test vector's phrase: a published test key, never a wallet's.
    fn test_wallet() -> key_wallet::Wallet {
        crate::keys_of(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap()
    }

    /// R-1, AC-T3: a planned key is this phrase's DIP-13 key at its number — the one K1 then
    /// tells as this wallet's — with its purpose and level; DashPay's bound to its contact
    /// requests.
    #[test]
    fn a_new_key_is_the_phrases_key_at_its_number() {
        let keys = test_wallet();
        let new = key_plan::NewKey {
            id: 7,
            purpose: NewPurpose::Encryption,
            level: KeyLevel::Medium,
            dashpay: true,
        };
        let made = phrase_key(&keys, 1, &new).unwrap();
        let mine = crate::identity::this_wallet_keys(&keys, 1, [7]);
        assert_eq!(made.key.data().as_slice(), mine[&7].as_slice());
        assert_eq!(made.key.id(), 7);
        assert_eq!(made.key.purpose(), Purpose::ENCRYPTION);
        assert_eq!(made.key.security_level(), SecurityLevel::MEDIUM);
        match made.key.contract_bounds() {
            Some(ContractBounds::SingleContractDocumentType {
                document_type_name, ..
            }) => assert_eq!(document_type_name, "contactRequest"),
            other => panic!("{other:?}"),
        }
        let tokens = key_plan::NewKey {
            id: 8,
            purpose: NewPurpose::Authentication,
            level: KeyLevel::Critical,
            dashpay: false,
        };
        let made = phrase_key(&keys, 1, &tokens).unwrap();
        assert!(made.key.contract_bounds().is_none());
        assert_eq!(made.key.security_level(), SecurityLevel::CRITICAL);
    }
}
