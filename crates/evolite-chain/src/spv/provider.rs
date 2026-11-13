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
/// withdrawal from the shielded pool is checked against it. AppConnect (protocol 14): the
/// `loginKeyResponse` of an App Connect v2 sign-in (FEAT-0023).
const SYSTEM: [SystemDataContract; 4] = [
    SystemDataContract::DPNS,
    SystemDataContract::Dashpay,
    SystemDataContract::Withdrawals,
    SystemDataContract::AppConnect,
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

/// The configuration of token `token_id` from the contracts registered (FEAT-0019 АРХ-2): the SDK
/// asks for it to check a token transition's proof, and the contract, proved when it came in, has it.
fn token_configuration(
    registered: &HashMap<Identifier, Arc<DataContract>>,
    token_id: &Identifier,
) -> Option<dpp::data_contract::TokenConfiguration> {
    use dpp::data_contract::accessors::v1::DataContractV1Getters;
    registered.values().find_map(|contract| {
        contract
            .tokens()
            .iter()
            .find(|(position, _)| contract.token_id(**position).as_ref() == Some(token_id))
            .map(|(_, config)| config.clone())
    })
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
        token_id: &Identifier,
    ) -> Result<Option<dpp::data_contract::TokenConfiguration>, ContextProviderError> {
        Ok(token_configuration(
            &self.registered.lock().unwrap_or_else(|e| e.into_inner()),
            token_id,
        ))
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
        // FEAT-0023: App Connect v2 publishes its response in this one.
        let app_connect = SystemDataContract::AppConnect.id();
        assert!(
            system_contract(&cache, &app_connect, v14)
                .unwrap()
                .is_some()
        );
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

#[cfg(test)]
mod token_tests {
    use super::*;
    use crate::platform::tokens::info::test_contract;

    #[test]
    fn a_registered_contract_answers_for_its_tokens() {
        let contract = Arc::new(test_contract(
            [3; 32],
            [9; 32],
            &[("coffee", 2), ("stamp", 0)],
        ));
        let registered = HashMap::from([(contract.id(), contract)]);
        let stamp: Identifier = dpp::tokens::calculate_token_id(&[3; 32], 1).into();
        let config = token_configuration(&registered, &stamp).expect("the stamp's configuration");
        use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
        use dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
        assert_eq!(config.conventions().decimals(), 0);
        let unknown: Identifier = dpp::tokens::calculate_token_id(&[4; 32], 0).into();
        assert!(token_configuration(&registered, &unknown).is_none());
    }
}
