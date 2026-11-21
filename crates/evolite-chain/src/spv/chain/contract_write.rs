//! The chain as the contract module's network (FEAT-0038, FEAT-0019 T-4): a contract checked
//! and registered by the wallet's identity, signed by its CRITICAL key from this phrase.

use super::*;
use crate::platform::contract_v1::Draft;
use crate::platform::contract_v1::register::{
    self, Checked, Network, RegisterError, Registered, Seen,
};
use dpp::data_contract::DataContract;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::signer::Signer;
use dpp::identity::{KeyType, Purpose, SecurityLevel};

/// The chain, with the owner's signer and the SDK, for one check or registration.
struct Writer<'c> {
    chain: &'c Chain,
    wallet: &'c PlatformWallet,
    sdk: Arc<dash_sdk::Sdk>,
    signer: crate::key_signer::KeySigner,
    /// The owner has an enabled AUTHENTICATION · CRITICAL · ECDSA key the signer holds.
    critical: bool,
}

impl<'c> Writer<'c> {
    async fn new(
        chain: &'c Chain,
        wallet: &'c PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
    ) -> Result<Self, RegisterError> {
        chain.ready().map_err(RegisterError::Failed)?;
        let sdk = chain.sdk().await.map_err(RegisterError::Failed)?;
        let (identity, signer) = Chain::signing(wallet, keys, owner)
            .await
            .map_err(RegisterError::Failed)?;
        let critical = identity.public_keys().values().any(|k| {
            k.purpose() == Purpose::AUTHENTICATION
                && k.security_level() == SecurityLevel::CRITICAL
                && k.key_type() == KeyType::ECDSA_SECP256K1
                && k.disabled_at().is_none()
                && Signer::<IdentityPublicKey>::can_sign_with(&signer, k)
        });
        Ok(Writer {
            chain,
            wallet,
            sdk,
            signer,
            critical,
        })
    }
}

impl Network for Writer<'_> {
    async fn credits(&self, owner: &Identifier) -> Result<u64, String> {
        let credits = self
            .chain
            .identity_credits(self.wallet, owner, true)
            .await
            .ok_or_else(|| "the identity's credits were not read".to_owned())?;
        // BUG-266: the protocol is taken right after; learned with a proved read if this run
        // has none yet, not «wait for the sync» while synced.
        self.chain.known_protocol(&self.sdk).await;
        Ok(credits)
    }

    fn protocol(&self) -> Seen {
        let info = self.sdk.protocol_version_info();
        Seen {
            verified: info.verified_version,
            newer: info.newer_protocol_available,
        }
    }

    fn can_sign(&self, _: &Identifier) -> bool {
        self.critical
    }

    async fn nonce(&self, owner: &Identifier) -> Result<u64, String> {
        self.sdk
            .get_identity_nonce(*owner, false, None)
            .await
            .map_err(|e| e.to_string())
    }

    async fn create(
        &self,
        owner: &Identifier,
        built: &crate::platform::contract_v1::Built,
    ) -> Result<DataContract, String> {
        self.wallet
            .identity()
            .create_data_contract_with_signer(
                owner,
                &built.documents_json,
                built.tokens_json.as_deref(),
                None,
                built.keywords_json.as_deref(),
                built.description.as_deref(),
                Some(&built.config_json),
                &self.signer,
            )
            .await
            .map_err(|e| e.to_string())
    }

    async fn fetch(&self, ids: &[Identifier]) -> Result<Vec<DataContract>, String> {
        use dash_sdk::platform::FetchMany;
        let found: dash_sdk::query_types::DataContracts =
            DataContract::fetch_many(&self.sdk, ids.to_vec())
                .await
                .map_err(|e| e.to_string())?;
        Ok(found.into_values().flatten().collect())
    }

    fn keep(&self, contract: Arc<DataContract>) {
        self.chain.register(&contract);
        if let Some(cache) = self.chain.contracts() {
            let _ = cache.put(contract, evolite_common::wall_secs() as i64);
        }
    }
}

impl Chain {
    /// A contract of `owner` registered with a nonce after `nonce`, read with proof; kept in
    /// the contracts cache and the provider when found (a registration whose answer was lost,
    /// FEAT-0038 V1 r1 Must 1).
    pub async fn registered_after(
        &self,
        owner: &Identifier,
        nonce: u64,
    ) -> Result<Option<Arc<DataContract>>, String> {
        use dash_sdk::platform::FetchMany;
        self.ready()?;
        let sdk = self.sdk().await?;
        let ids: Vec<Identifier> = (1..=3)
            .map(|ahead| DataContract::generate_data_contract_id_v0(owner, nonce + ahead))
            .collect();
        let found: dash_sdk::query_types::DataContracts = DataContract::fetch_many(&sdk, ids)
            .await
            .map_err(|e| e.to_string())?;
        let found = found
            .into_values()
            .flatten()
            .find(|c| {
                use dpp::data_contract::accessors::v0::DataContractV0Getters;
                c.owner_id() == *owner
            })
            .map(Arc::new);
        if let Some(c) = &found {
            self.register(c);
            if let Some(cache) = self.contracts() {
                let _ = cache.put(c.clone(), evolite_common::wall_secs() as i64);
            }
        }
        Ok(found)
    }

    /// The checks of registering `draft` as `owner`, before the password: its fee, and why it
    /// cannot go if it cannot (no CRITICAL key, the draft refused, too few credits).
    pub async fn check_contract(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
        draft: &Draft,
    ) -> Checked {
        match Writer::new(self, wallet, keys, owner).await {
            Ok(writer) => register::check(&writer, owner, draft).await,
            Err(e) => Checked {
                fee: None,
                refusal: Some(e),
            },
        }
    }

    /// Register `draft` as `owner` (the same checks, then the write); the contract is in the
    /// contracts cache and the provider after it.
    pub async fn register_contract(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
        draft: &Draft,
    ) -> Result<Registered, RegisterError> {
        let writer = Writer::new(self, wallet, keys, owner).await?;
        register::register(&writer, owner, draft).await
    }
}
