//! What the SDK needs to check Platform proofs, served from our own chain state, not a server:
//!
//! - **quorum public keys** from platform-wallet's SPV client — only quorums dash-spv has
//!   verified against the masternode list it built itself (BLS); until the masternode lists are
//!   synced, "not ready", never a key we have not checked;
//! - **system data contracts** (DPNS, DashPay) as the protocol version the verifier works with
//!   defines them, so a proof is never checked against another version's schema;
//! - contracts the SDK registers as it fetches them.
//!
//! Same shape as Dash Evo Tool's `SpvProvider`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dash_sdk::error::ContextProviderError;
use dash_sdk::platform::{ContextProvider, DataContract, Identifier};
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contracts::SystemDataContract;
use dpp::prelude::CoreBlockHeight;
use dpp::version::PlatformVersion;
use platform_wallet::SpvRuntime;

use crate::sync::SyncStatus;

/// The system contracts served from the protocol's own definitions. Withdrawals: the proof of a
/// withdrawal from the shielded pool is checked against it.
const SYSTEM: [SystemDataContract; 3] = [
    SystemDataContract::DPNS,
    SystemDataContract::Dashpay,
    SystemDataContract::Withdrawals,
];

/// Core block height at which Platform activated on testnet (the `mn_rr` locked height), as in
/// the SDK's own trusted provider.
const TESTNET_ACTIVATION_HEIGHT: CoreBlockHeight = 1_090_319;

/// System contracts by (id, protocol version), built on first use.
type SystemContracts = Mutex<HashMap<(Identifier, u32), Arc<DataContract>>>;

#[derive(Clone)]
pub struct SpvContextProvider {
    spv: Arc<SpvRuntime>,
    /// The sync status: quorum keys are asked for only once the masternode lists are in.
    status: Arc<Mutex<SyncStatus>>,
    system: Arc<SystemContracts>,
    /// Contracts the SDK fetched (with proofs) and handed back to us.
    registered: Arc<Mutex<HashMap<Identifier, Arc<DataContract>>>>,
}

impl SpvContextProvider {
    pub fn new(spv: Arc<SpvRuntime>, status: Arc<Mutex<SyncStatus>>) -> Self {
        Self {
            spv,
            status,
            system: Arc::default(),
            registered: Arc::default(),
        }
    }

    fn masternodes_ready(&self) -> bool {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .masternodes
            .is_some_and(|m| m.done)
    }
}

/// A system contract with this id, at `version`.
fn system_contract(
    cache: &SystemContracts,
    id: &Identifier,
    version: &PlatformVersion,
) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
    let Some(which) = SYSTEM.into_iter().find(|c| c.id() == *id) else {
        return Ok(None);
    };
    let key = (*id, version.protocol_version);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(contract) = cache.get(&key) {
        return Ok(Some(contract.clone()));
    }
    let contract = Arc::new(
        dpp::system_data_contracts::load_system_data_contract(which, version).map_err(|e| {
            ContextProviderError::DataContractFailure(format!(
                "system contract {which:?} at protocol version {}: {e}",
                version.protocol_version
            ))
        })?,
    );
    cache.insert(key, contract.clone());
    Ok(Some(contract))
}

impl ContextProvider for SpvContextProvider {
    fn get_data_contract(
        &self,
        id: &Identifier,
        platform_version: &PlatformVersion,
    ) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
        if let Some(contract) = system_contract(&self.system, id, platform_version)? {
            return Ok(Some(contract));
        }
        Ok(self
            .registered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned())
    }

    fn register_data_contract(&self, contract: Arc<DataContract>) {
        self.registered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(contract.id(), contract);
    }

    fn get_token_configuration(
        &self,
        _token_id: &Identifier,
    ) -> Result<Option<dpp::data_contract::TokenConfiguration>, ContextProviderError> {
        Ok(None)
    }

    fn get_quorum_public_key(
        &self,
        quorum_type: u32,
        quorum_hash: [u8; 32],
        core_chain_locked_height: u32,
    ) -> Result<[u8; 48], ContextProviderError> {
        if !self.masternodes_ready() {
            return Err(ContextProviderError::Config(
                "masternode list not yet synced (quorums unavailable)".into(),
            ));
        }
        // The trait is sync, the lookup async; this runs inside proof verification on a tokio
        // worker of the core's multi-thread runtime.
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            ContextProviderError::Config("no async runtime for the quorum lookup".into())
        })?;
        let _timed = crate::turn::begin("quorum_key");
        tokio::task::block_in_place(|| {
            handle
                .block_on(self.spv.get_quorum_public_key(
                    quorum_type,
                    quorum_hash,
                    core_chain_locked_height,
                ))
                .map_err(|e| e.quorum_lookup_context_error())
        })
    }

    fn get_platform_activation_height(&self) -> Result<CoreBlockHeight, ContextProviderError> {
        Ok(TESTNET_ACTIVATION_HEIGHT)
    }
}

/// Stands in while the SDK is built: the SDK refuses to build without a provider, ours needs the
/// manager's SPV client, and the manager needs the SDK. Replaced before anything is asked.
pub struct NotYet;

impl ContextProvider for NotYet {
    fn get_data_contract(
        &self,
        _id: &Identifier,
        _platform_version: &PlatformVersion,
    ) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
        Err(ContextProviderError::Config("the chain is starting".into()))
    }

    fn get_token_configuration(
        &self,
        _token_id: &Identifier,
    ) -> Result<Option<dpp::data_contract::TokenConfiguration>, ContextProviderError> {
        Err(ContextProviderError::Config("the chain is starting".into()))
    }

    fn get_quorum_public_key(
        &self,
        _quorum_type: u32,
        _quorum_hash: [u8; 32],
        _core_chain_locked_height: u32,
    ) -> Result<[u8; 48], ContextProviderError> {
        Err(ContextProviderError::Config("the chain is starting".into()))
    }

    fn get_platform_activation_height(&self) -> Result<CoreBlockHeight, ContextProviderError> {
        Ok(TESTNET_ACTIVATION_HEIGHT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_system_contracts_at_the_asked_version() {
        let cache = Mutex::default();
        let v14 = PlatformVersion::get(14).unwrap();
        let dpns = SystemDataContract::DPNS.id();
        let contract = system_contract(&cache, &dpns, v14).unwrap().unwrap();
        assert_eq!(contract.id(), dpns);
        // Cached per version: the same Arc the second time.
        let again = system_contract(&cache, &dpns, v14).unwrap().unwrap();
        assert!(Arc::ptr_eq(&contract, &again));
        let dashpay = SystemDataContract::Dashpay.id();
        assert!(system_contract(&cache, &dashpay, v14).unwrap().is_some());
        let withdrawals = SystemDataContract::Withdrawals.id();
        assert!(
            system_contract(&cache, &withdrawals, v14)
                .unwrap()
                .is_some()
        );
        // Anything else is not a system contract here.
        let other = Identifier::from([7u8; 32]);
        assert!(system_contract(&cache, &other, v14).unwrap().is_none());
    }
}
