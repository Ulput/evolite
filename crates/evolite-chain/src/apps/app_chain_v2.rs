//! The network side of App Connect v2 ([`crate::app_connect_v2`], FEAT-0023 round 3): what
//! Platform says about the contracts a request names, the protocol version, the identity's
//! current key for a contract, the IdentityUpdates that add the granted keys or disable a
//! session's, the `loginKeyResponse` in the AppConnect system contract, and the keys'
//! remaining budgets. The rules themselves are pure, in `app_connect_v2::grant`.

use dash_sdk::platform::Fetch;
use dpp::data_contract::DataContract;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::config::v0::DataContractConfigGettersV0;
use dpp::data_contract::storage_requirements::keys_for_document_type::StorageKeyRequirements;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::{IdentityPublicKey, Purpose, SecurityLevel};
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use dpp::system_data_contracts::SystemDataContract;
use platform_wallet::PlatformWallet;
use zeroize::Zeroizing;

use crate::app_chain::SignInIdentity;
use crate::app_connect_v2::grant::{Facts, KeyRule, Response};
use crate::app_connect_v2::keys::SLOT;
use crate::app_connect_v2::{Bounds, EncPurpose, Request};
use crate::chain::Chain;

/// The document the response is, in the AppConnect system contract.
pub const RESPONSE_TYPE: &str = "loginKeyResponse";
/// The first protocol version with V1 keys, contract groups and the AppConnect contract.
pub const MIN_PROTOCOL: u32 = 14;
/// Keys one IdentityUpdate adds at most (DIP, "Identity Updates").
const KEYS_PER_UPDATE: usize = 6;

fn rule(requirement: Option<StorageKeyRequirements>) -> Option<KeyRule> {
    requirement.map(|r| match r {
        StorageKeyRequirements::Unique => KeyRule::Unique,
        StorageKeyRequirements::Multiple => KeyRule::Multiple,
        StorageKeyRequirements::MultipleReferenceToLatest => KeyRule::Latest,
    })
}

fn purpose(purpose: EncPurpose) -> Purpose {
    match purpose {
        EncPurpose::Encryption => Purpose::ENCRYPTION,
        EncPurpose::Decryption => Purpose::DECRYPTION,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The response's properties as platform-wallet takes them (byte arrays as hex).
fn response_json(response: &Response) -> String {
    serde_json::json!({
        "appEphemeralPubKeyHash": hex(&response.app_key_hash),
        "walletEphemeralPubKey": hex(&response.wallet_ephemeral),
        "encryptedPayload": hex(&response.payload),
    })
    .to_string()
}

/// A new key with its private half: the update proves each new key with its own.
pub struct NewKey {
    pub key: IdentityPublicKey,
    pub secret: Zeroizing<[u8; SLOT]>,
}

impl Chain {
    /// The network's protocol version, as the SDK last saw it.
    pub async fn protocol_version(&self) -> Result<u32, String> {
        Ok(self.sdk().await?.protocol_version_number())
    }

    /// What Platform says (proved) about every contract and group `request` names.
    pub async fn connect_facts(&self, request: &Request) -> Result<Facts, String> {
        let sdk = self.sdk().await?;
        let mut facts = Facts::default();
        let mut contracts = vec![request.app_id];
        for binding in &request.auth {
            match &binding.bounds {
                Bounds::SingleContract(c) | Bounds::DocumentType { contract: c, .. } => {
                    contracts.push(*c)
                }
                Bounds::ContractGroup(group) => {
                    use dash_sdk::platform::contract_groups::ContractGroupInfo;
                    let info = ContractGroupInfo::fetch(&sdk, Identifier::from(*group))
                        .await
                        .map_err(|e| e.to_string())?;
                    if let Some(info) = info {
                        facts
                            .group_owner
                            .insert(*group, info.owner().owner_id().to_buffer());
                    }
                }
            }
        }
        contracts.extend(request.enc.iter().map(|e| e.contract));
        contracts.sort_unstable();
        contracts.dedup();
        for id in contracts {
            let Some(contract) = DataContract::fetch(&sdk, Identifier::from(id))
                .await
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            facts
                .contract_owner
                .insert(id, contract.owner_id().to_buffer());
            let config = contract.config();
            facts.key_rule.insert(
                (id, EncPurpose::Encryption),
                rule(config.requires_identity_encryption_bounded_key()),
            );
            facts.key_rule.insert(
                (id, EncPurpose::Decryption),
                rule(config.requires_identity_decryption_bounded_key()),
            );
        }
        Ok(facts)
    }

    /// Platform's current key (proved) of `identity` for `contract` and `purpose`, if any.
    pub async fn current_contract_key(
        &self,
        identity: Identifier,
        contract: [u8; 32],
        purpose: EncPurpose,
    ) -> Result<Option<IdentityPublicKey>, String> {
        use dash_sdk::platform::identities_contract_keys_query::IdentitiesContractKeysQuery;
        use dpp::identity::identities_contract_keys::IdentitiesContractKeys;
        let sdk = self.sdk().await?;
        let wanted = self::purpose(purpose);
        let query = IdentitiesContractKeysQuery::new(
            vec![identity],
            Identifier::from(contract),
            None,
            vec![wanted],
        )
        .map_err(|e| e.to_string())?;
        let found = IdentitiesContractKeys::fetch(&sdk, query)
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        Ok(found
            .get(&identity)
            .and_then(|by_purpose| by_purpose.get(&wanted).cloned().flatten()))
    }

    /// Adds `add` and disables `disable` on the identity, MASTER-signed, at most
    /// [`KEYS_PER_UPDATE`] new keys an update (the disabling goes with the first), then reads
    /// the identity again. The caller checks that every new key is there before it answers.
    pub async fn update_app_keys(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        who: &SignInIdentity,
        add: Vec<NewKey>,
        disable: Vec<u32>,
    ) -> Result<SignInIdentity, String> {
        let (_, mut signer) = who
            .signer(Purpose::AUTHENTICATION, SecurityLevel::MASTER)
            .ok_or(tr!("This identity's MASTER key is not from your phrase"))?;
        signer.add_identity_public_keys(add.iter().map(|k| (k.key.clone(), *k.secret)));
        let mut batches: Vec<Vec<IdentityPublicKey>> = add
            .chunks(KEYS_PER_UPDATE)
            .map(|chunk| chunk.iter().map(|k| k.key.clone()).collect())
            .collect();
        if batches.is_empty() {
            batches.push(Vec::new());
        }
        let mut disable = Some(disable);
        for batch in batches {
            let off = disable.take().unwrap_or_default();
            if batch.is_empty() && off.is_empty() {
                continue;
            }
            wallet
                .identity()
                .update_identity_with_external_signer(&who.id, batch, off, &signer, None)
                .await
                .map_err(|e| e.to_string())?;
        }
        self.sign_in_identity(wallet, keys).await
    }

    /// Publishes the response, signed by an unbound, unlimited AUTHENTICATION key at HIGH, and
    /// answers its document id.
    pub async fn publish_connect_response(
        &self,
        wallet: &PlatformWallet,
        who: &SignInIdentity,
        response: &Response,
    ) -> Result<[u8; 32], String> {
        use dpp::document::DocumentV0Getters;
        let (_, signer) = who
            .signer(Purpose::AUTHENTICATION, SecurityLevel::HIGH)
            .ok_or(tr!("The identity has no HIGH key from your phrase: there is nothing to sign the answer to the app with"))?;
        let document = wallet
            .identity()
            .create_document_with_signer(
                &who.id,
                &SystemDataContract::AppConnect.id(),
                RESPONSE_TYPE,
                &response_json(response),
                None,
                &signer,
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(document.id().to_buffer())
    }

    /// Deletes an earlier response by its values (the type keeps no rows). Best effort: the
    /// caller does not let a failure stop the sign-in or the disconnect.
    pub async fn delete_connect_response(
        &self,
        wallet: &PlatformWallet,
        who: &SignInIdentity,
        document: [u8; 32],
        response: &Response,
    ) -> Result<(), String> {
        let (key, signer) = who
            .signer(Purpose::AUTHENTICATION, SecurityLevel::HIGH)
            .ok_or(tr!("The identity has no HIGH key from your phrase: there is nothing to sign the answer to the app with"))?;
        wallet
            .identity()
            .delete_document_by_values_with_signer(
                &who.id,
                &SystemDataContract::AppConnect.id(),
                RESPONSE_TYPE,
                &Identifier::from(document),
                &response_json(response),
                key.id(),
                &signer,
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// What is left of each key's budget (proved): `None` for a key without one.
    pub async fn remaining_budgets(
        &self,
        identity: Identifier,
        key_ids: Vec<u32>,
    ) -> Result<Vec<(u32, Option<u64>)>, String> {
        use dash_sdk::platform::identity_keys_remaining_budgets::{
            IdentityKeysRemainingBudgets, IdentityKeysRemainingBudgetsQuery,
        };
        if key_ids.is_empty() {
            return Ok(Vec::new());
        }
        let sdk = self.sdk().await?;
        let found = IdentityKeysRemainingBudgets::fetch(
            &sdk,
            IdentityKeysRemainingBudgetsQuery {
                identity_id: identity,
                key_ids: key_ids.clone(),
            },
        )
        .await
        .map_err(|e| e.to_string())?
        .map(|b| b.0)
        .unwrap_or_default();
        Ok(key_ids
            .into_iter()
            .map(|id| (id, found.get(&id).copied().flatten()))
            .collect())
    }
}

/// The base58 of an id, as the store keeps it.
pub fn base58(id: &[u8; 32]) -> String {
    Identifier::from(*id).to_string(Encoding::Base58)
}
