//! A data contract of evolite's own, made and paid for by the wallet (FEAT-0038 «Workshop»
//! collections, FEAT-0019 T-4 tokens): one module for both. A [`Draft`] is built into a V1
//! contract the way platform-wallet builds it before it signs, checked by DPP at the
//! **network's** protocol version, read after a proved answer (the SDK's seed before one is
//! 13, whose tables refuse the NFT profile; testnet is on 14), and by the rules DPP does not
//! check but the network does ([`rules`]). [`fee`]
//! says what the registration costs before the password. The write itself is
//! `Chain::register_contract`.
//!
//! No network here.

pub mod fee;
pub mod register;
pub mod rules;
pub mod templates;

use dpp::data_contract::DataContract;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use dpp::serialization::PlatformSerializableWithPlatformVersion;
use dpp::version::PlatformVersion;
use serde_json::{Map, Value, json};

use rules::Refusal;

/// What the person (or a template) asks for: the parts of a V1 contract as JSON, the way
/// `platform-wallet::create_data_contract_with_signer` takes them.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    /// `documentSchemas`: the document types by name.
    pub documents: Value,
    /// `tokens` by position, if any.
    pub tokens: Option<Value>,
    /// Search keywords (KeywordSearch); made lowercase by [`build`].
    pub keywords: Vec<String>,
    /// The contract's description, 3–100 characters, if any.
    pub description: Option<String>,
    /// `config.readonly`: no update ever (the NFT profile asks for it).
    pub readonly: bool,
}

/// A draft that passed: the contract as DPP built it (its id a placeholder: the network makes
/// it from the owner's nonce) and the JSON parts to sign, as they were checked.
#[derive(Clone, Debug)]
pub struct Built {
    pub contract: DataContract,
    pub documents_json: String,
    pub tokens_json: Option<String>,
    pub keywords_json: Option<String>,
    pub description: Option<String>,
    pub config_json: String,
    /// The contract's size as the network stores it, for [`fee::estimate`].
    pub bytes: usize,
}

/// The id the format carries until the network gives the real one.
const PLACEHOLDER_ID: &str = "11111111111111111111111111111111";

/// The version a contract is built, checked and priced at: the network's protocol (the SDK's
/// once a proved answer named it: before one the SDK holds its seed), never the newest this
/// build knows.
pub fn version_for(protocol: u32) -> Result<&'static PlatformVersion, Refusal> {
    PlatformVersion::get(protocol).map_err(|e| Refusal::Invalid(e.to_string()))
}

/// Build `draft` for `owner` at `pv`, the protocol version of the network it goes to
/// ([`version_for`]), and check it: [`rules`] first — in the release the only check of what
/// the network refuses (`$price` or a field the type lacks in an index, a repeated index name,
/// an unknown key), dpp's `validation` feature being for tests only (the customer,
/// 2026-11-20) — then DPP's decoding, as platform-wallet does just before it signs (with the
/// feature, in tests, DPP's full schema check too).
pub fn build(draft: &Draft, owner: Identifier, pv: &PlatformVersion) -> Result<Built, Refusal> {
    let keywords = rules::keywords(&draft.keywords)?;
    let description = rules::description(draft.description.as_deref())?;
    rules::schema_shape(&draft.documents)?;
    assemble(draft, keywords, description, owner, pv)
}

/// The contract of `draft` as DPP decodes it, past [`rules`].
fn assemble(
    draft: &Draft,
    keywords: Vec<String>,
    description: Option<String>,
    owner: Identifier,
    pv: &PlatformVersion,
) -> Result<Built, Refusal> {
    let config_version = pv.dpp.contract_versions.config.default_current_version;
    let mut config = Map::new();
    if draft.readonly {
        config.insert("readonly".into(), Value::Bool(true));
    }
    let config_json = Value::Object(config.clone()).to_string();
    config.insert(
        "$formatVersion".into(),
        Value::String(config_version.to_string()),
    );

    let mut format = json!({
        "$formatVersion": "1",
        "id": PLACEHOLDER_ID,
        "ownerId": owner.to_string(Encoding::Base58),
        "version": 1,
        "documentSchemas": draft.documents,
        "config": Value::Object(config),
    });
    if let Some(tokens) = &draft.tokens {
        format["tokens"] = tokens.clone();
    }
    if !keywords.is_empty() {
        format["keywords"] = json!(keywords);
    }
    if let Some(d) = &description {
        format["description"] = Value::String(d.clone());
    }
    let text = format.to_string();
    let contract = validated(&text, pv)?;
    let bytes = contract
        .clone()
        .serialize_to_bytes_with_platform_version(pv)
        .map_err(|e| Refusal::Invalid(e.to_string()))?
        .len();
    Ok(Built {
        contract,
        documents_json: draft.documents.to_string(),
        tokens_json: draft.tokens.as_ref().map(Value::to_string),
        keywords_json: (!keywords.is_empty()).then(|| json!(keywords).to_string()),
        description,
        config_json,
        bytes,
    })
}

/// The contract of the V1 format `text`, fully validated by DPP at `pv`. Through a string, as
/// platform-wallet does: serde's tagged enums lose fields on the `from_value` path.
fn validated(text: &str, pv: &PlatformVersion) -> Result<DataContract, Refusal> {
    let format = serde_json::from_str(text).map_err(|e| Refusal::Invalid(e.to_string()))?;
    // The validation's operations, for its cost; refusals come as the error.
    let mut operations = vec![];
    DataContract::try_from_platform_versioned(format, true, &mut operations, pv)
        .map_err(|e| Refusal::Invalid(e.to_string()))
}

#[cfg(test)]
mod register_tests;
#[cfg(test)]
mod tests;
