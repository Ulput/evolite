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
