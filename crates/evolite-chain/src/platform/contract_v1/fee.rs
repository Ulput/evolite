//! What registering a contract costs, before the password (FEAT-0038 AC-T-W2, FEAT-0019
//! AC-T15): Platform's registration cost exactly, part by part as the screen shows it, and the
//! storage and processing on top with room to spare. The estimate is never below what the
//! network takes.
//!
//! Calibration, live on testnet (protocol 14):
//! - the FEAT-0038 probe's collection: `registration_cost` 0.19, 2 349 bytes, 0.00163897 on top
//!   (about 69 800 credits a byte);
//! - FEAT-0019's token probe: 0.40, 401 bytes, 0.00186311 on top (with `SetPrices`).

use dpp::balances::credits::Credits;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::accessors::v1::DataContractV1Getters;
use dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dpp::version::PlatformVersion;

use super::Built;

/// Credits on top for each byte of the contract.
pub const STORAGE_PER_BYTE: Credits = 70_000;

/// Credits on top whatever the size (0.0025 DASH).
pub const STORAGE_BASE: Credits = 250_000_000;

/// The cost of a registration, in credits, part by part (architect, 2026-11-20): the screen
/// shows the parts before the password, «+0.1 DASH» at each keyword.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fee {
    /// The contract itself.
    pub base: Credits,
    /// Its document types.
    pub document_types: Credits,
    /// Their indexes: unique, not unique and contested.
    pub indexes: Credits,
    /// Its tokens, with what their distributions add.
    pub tokens: Credits,
    /// Its search keywords.
    pub keywords: Credits,
    /// What one keyword adds, for the screen.
    pub per_keyword: Credits,
    /// Storage and processing, estimated with room to spare.
    pub storage: Credits,
}

impl Fee {
    /// Platform's `registration_cost`: exact.
    pub fn registration(&self) -> Credits {
        [
            self.base,
            self.document_types,
            self.indexes,
            self.tokens,
            self.keywords,
        ]
        .into_iter()
        .fold(0, Credits::saturating_add)
    }

    /// The most the registration takes: the registration and the storage estimated.
    pub fn total(&self) -> Credits {
        self.registration().saturating_add(self.storage)
    }
}

/// The fee of registering `built` on a network at `pv` (the network's version: before
/// protocol 9 nothing is charged).
pub fn estimate(built: &Built, pv: &PlatformVersion) -> Result<Fee, String> {
    let contract = &built.contract;
    let registration = contract.registration_cost(pv).map_err(|e| e.to_string())?;
    if registration == 0 {
        return Ok(Fee {
            storage: storage(built.bytes),
            ..Fee::default()
        });
    }
    let v = &pv.fee_version.data_contract_registration;
    let types = contract.document_types();
    let indexes = types
        .values()
        .flat_map(|t| t.indexes().values())
        .map(|i| {
            if i.contested_index.is_some() {
                v.document_type_base_contested_index_registration_fee
            } else if i.unique {
                v.document_type_base_unique_index_registration_fee
            } else {
                v.document_type_base_non_unique_index_registration_fee
            }
        })
        .fold(0, Credits::saturating_add);
    let mut fee = Fee {
        base: v.base_contract_registration_fee,
        document_types: v
            .document_type_registration_fee
            .saturating_mul(types.len() as Credits),
        indexes,
        tokens: 0,
        keywords: v
            .search_keyword_fee
            .saturating_mul(contract.keywords().len() as Credits),
        per_keyword: v.search_keyword_fee,
        storage: storage(built.bytes),
    };
    if fee.registration() > registration {
        return Err(format!(
            "the parts ({}) are more than Platform's registration cost ({registration})",
            fee.registration()
        ));
    }
    // The tokens and their distributions: what Platform counts beyond the rest.
    fee.tokens = registration - fee.registration();
    Ok(fee)
}

/// Storage and processing of a contract of `bytes`.
pub fn storage(bytes: usize) -> Credits {
    STORAGE_BASE.saturating_add(STORAGE_PER_BYTE.saturating_mul(bytes as Credits))
}
