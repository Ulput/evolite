//! The chain's side of the contracts cache (FEAT-0031 C5, `platform/contracts`): Platform as the
//! cache's [`Source`] — versions and bodies with proofs — and every proved read kept and
//! registered in one place.

use std::time::Duration;

use super::*;
use crate::platform::contracts::fresh::{self, Source};
use crate::platform::contracts::{Cached, ContractCache};
use dpp::data_contract::DataContract;

/// Platform, read with proofs, for the cache.
struct Platform<'c> {
    chain: &'c Chain,
}

impl Source for Platform<'_> {
    async fn latest_versions(
        &self,
        ids: &[Identifier],
    ) -> Result<std::collections::HashMap<Identifier, u32>, String> {
        use dash_sdk::platform::FetchMany;
        use dash_sdk::platform::data_contracts_latest_versions::{
            DataContractLatestVersion, DataContractsLatestVersions,
        };
        self.chain.ready()?;
        let sdk = self.chain.sdk().await?;
        let versions: DataContractsLatestVersions =
            DataContractLatestVersion::fetch_many(&sdk, ids.to_vec())
                .await
                .map_err(|e| format!("{e:?}"))?;
        Ok(versions
            .0
            .iter()
            .filter_map(|(id, v)| Some((*id, v.as_ref()?.version)))
            .collect())
    }

    async fn fetch(&self, ids: &[Identifier]) -> Result<Vec<Arc<DataContract>>, String> {
        use dash_sdk::platform::FetchMany;
        self.chain.ready()?;
        let sdk = self.chain.sdk().await?;
        let contracts: dash_sdk::query_types::DataContracts =
            DataContract::fetch_many(&sdk, ids.to_vec())
                .await
                .map_err(|e| format!("{e:?}"))?;
        Ok(contracts
            .into_iter()
            .filter_map(|(_, c)| c.map(Arc::new))
            .collect())
    }

    fn keep(&self, cache: &ContractCache, contract: Arc<DataContract>) -> Result<Cached, String> {
        self.chain.register(&contract);
        cache.put(contract, evolite_common::wall_secs() as i64)
    }
}

impl Chain {
    /// The network's contracts cache, once the store opened it.
    pub fn contracts(&self) -> Option<Arc<ContractCache>> {
        self.contracts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Registers a proved contract with the provider for this run (the SDK's answers about its
    /// documents and tokens are checked against it). Never waits for the network.
    pub(crate) fn register(&self, contract: &Arc<DataContract>) {
        let manager = self.manager.try_lock().ok().and_then(|m| m.clone());
        if let Some(provider) = manager.as_ref().and_then(|m| m.sdk().context_provider()) {
            use dash_sdk::platform::ContextProvider as _;
            provider.register_data_contract(contract.clone());
        }
    }

    /// Contract `id` checked against Platform within `within` (a read: a token's page 60 s, a
    /// collection's card 10 min). A body read from the disk is good for a read; a check that
    /// failed gives the one kept, `version_checked: false` (review condition 8).
    pub async fn fresh_cached(&self, id: Identifier, within: Duration) -> Result<Cached, String> {
        self.fresh_many(&[id], within, false)
            .await
            .pop()
            .unwrap_or_else(|| Err("not answered".into()))
    }

    /// Contract `id` for a write, or anything that counts money or amounts (review condition 1):
    /// its body proved in this run and its version checked now; anything less is an error.
    pub async fn fresh_proved(&self, id: Identifier) -> Result<Cached, String> {
        self.fresh_many(&[id], Duration::ZERO, true)
            .await
            .pop()
            .unwrap_or_else(|| Err("not answered".into()))
    }

    /// `ids` fresh within `within` (review condition 7): the stale ones' versions in one request,
    /// the bodies to read in one more; one flight per id.
    pub async fn fresh_many(
        &self,
        ids: &[Identifier],
        within: Duration,
        proved: bool,
    ) -> Vec<Result<Cached, String>> {
        let Some(cache) = self.contracts() else {
            return ids
                .iter()
                .map(|_| Err("the contracts cache is not open".into()))
                .collect();
        };
        let now = evolite_common::wall_secs() as i64;
        fresh::fresh_many(
            &cache,
            &Platform { chain: self },
            ids,
            within.as_secs() as i64,
            proved,
            now,
        )
        .await
    }

    /// The contract and position of `token_id` for a write: the contract proved in this run and
    /// its own token id at that position checked (review conditions 1 and 9).
    pub async fn token_for_write(&self, token_id: &Identifier) -> Result<(Cached, u16), String> {
        let cache = self.contracts().ok_or("the contracts cache is not open")?;
        fresh::token_for_write(
            &cache,
            &Platform { chain: self },
            token_id,
            evolite_common::wall_secs() as i64,
        )
        .await
    }
}

/// An id as base58.
fn ident(s: &str) -> Result<Identifier, String> {
    Identifier::from_string(s, dpp::platform_value::string_encoding::Encoding::Base58)
        .map_err(|e| e.to_string())
}

/// A token to write to (TokenReader R1, review FEAT-0031-C5 r1 Must): its contract proved in this
/// run and its position the contract's own for this token id. Only [`Chain::token_to_write`]
/// makes one, and every token write takes one, so no position or contract from a file on the
/// disk reaches a signature.
#[derive(Clone, Debug)]
pub struct WriteToken {
    contract: Arc<DataContract>,
    position: u16,
}

impl WriteToken {
    pub fn contract(&self) -> &Arc<DataContract> {
        &self.contract
    }

    pub fn contract_id(&self) -> Identifier {
        use dpp::data_contract::accessors::v0::DataContractV0Getters;
        self.contract.id()
    }

    pub fn position(&self) -> u16 {
        self.position
    }

    /// The token's settings as the proved contract has them.
    pub fn config(&self) -> Option<&dpp::data_contract::TokenConfiguration> {
        use dpp::data_contract::accessors::v1::DataContractV1Getters;
        self.contract.tokens().get(&self.position)
    }

    /// How many decimals the proved contract gives the token (review TokenReader r1, S1).
    pub fn decimals(&self) -> Option<u8> {
        use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
        use dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
        self.config().map(|c| c.conventions().decimals())
    }
}

impl Chain {
    /// `token_id` (base58) to write to, through [`Self::token_for_write`]. `contract_hint` is
    /// the contract the wallet's list says it is in: read (proved) first when the cache does not
    /// know the token yet — a token followed before the cache — and only to find it; the
    /// position is still the proved contract's own.
    pub async fn token_to_write(
        &self,
        token_id: &str,
        contract_hint: &str,
    ) -> Result<WriteToken, String> {
        let token = ident(token_id)?;
        let checked = match self.token_for_write(&token).await {
            Ok(found) => found,
            Err(first) => {
                // Not kept yet: the hinted contract, proved and kept, then the same check.
                let hint = ident(contract_hint)?;
                self.fresh_proved(hint)
                    .await
                    .map_err(|e| format!("{first}; {e}"))?;
                self.token_for_write(&token).await?
            }
        };
        let (cached, position) = checked;
        Ok(WriteToken {
            contract: cached.contract,
            position,
        })
    }

    /// `token_id` (base58) to read by (the token page): its contract fresh within `within`, and
    /// its position the contract's own; the hint as in [`Self::token_to_write`].
    pub async fn token_to_read(
        &self,
        token_id: &str,
        contract_hint: &str,
        within: Duration,
    ) -> Result<(Cached, u16), String> {
        use dpp::data_contract::accessors::v1::DataContractV1Getters;
        let token = ident(token_id)?;
        let contract = match self.contracts().and_then(|c| c.token(&token)) {
            Some((contract, _)) => contract,
            None => ident(contract_hint)?,
        };
        let cached = self.fresh_cached(contract, within).await?;
        let position = cached
            .contract
            .tokens()
            .keys()
            .copied()
            .find(|p| cached.contract.token_id(*p).as_ref() == Some(&token))
            .ok_or("the token is not in its contract")?;
        Ok((cached, position))
    }
}

impl Chain {
    /// The contracts the wallet's list names (base58), in the cache: the kept ones as they are,
    /// the others read with proofs (TokenReader R1). In place of handing the list's bytes to the
    /// provider unproved (review FEAT-0031-C5 r1, could). A failure is left: the contract is read
    /// again when it is used.
    pub async fn keep_contracts(&self, ids: &[String]) {
        let ids: Vec<Identifier> = ids.iter().filter_map(|id| ident(id).ok()).collect();
        if !ids.is_empty() {
            let _ = self.fresh_many(&ids, KEPT_FOR, false).await;
        }
    }
}

/// How long a contract the list names stays good without a version check when it is only kept
/// (a page or a write checks it again for itself).
const KEPT_FOR: Duration = Duration::from_secs(24 * 3600);
