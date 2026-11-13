//! What the screens show of a contract's tokens ([`TokenInfo`]), read from the contract as it was
//! proved. Every singular form goes with the token, capitalized when the contract says so; the
//! screen picks the one of its language (`evolite_api::tokens::name_in`, part 3 review r1,
//! Should 2). dpp's own `singular_form_by_language_code_or_default` indexes the English entry
//! and panics when a contract has none, so it is not used here.

use dpp::data_contract::DataContract;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::accessors::v1::DataContractV1Getters;
use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dpp::data_contract::associated_token::token_configuration::accessors::v1::TokenConfigurationV1Getters;
use dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dpp::data_contract::associated_token::token_configuration_localization::accessors::v0::TokenConfigurationLocalizationV0Getters;
use dpp::data_contract::associated_token::token_keeps_history_rules::accessors::v0::TokenKeepsHistoryRulesV0Getters;
use dpp::platform_value::string_encoding::Encoding;
use evolite_api::tokens::TokenInfo;

/// A singular form as shown: trimmed, its first letter upper case when `capitalize`.
pub fn shown_form(name: &str, capitalize: bool) -> String {
    let name = name.trim();
    if !capitalize {
        return name.to_owned();
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The tokens of `contract`, in the order of their positions; none for a contract of version 0.
/// The issuer's `@name` is filled by the caller.
pub fn tokens_of(contract: &DataContract) -> Vec<TokenInfo> {
    let base58 = |id: dpp::prelude::Identifier| id.to_string(Encoding::Base58);
    let contract_id = base58(contract.id());
    let issuer = base58(contract.owner_id());
    contract
        .tokens()
        .iter()
        .filter_map(|(position, config)| {
            let token_id = contract.token_id(*position)?;
            let conventions = config.conventions();
            let names = conventions
                .localizations()
                .iter()
                .map(|(code, l)| {
                    (
                        code.clone(),
                        shown_form(l.singular_form(), l.should_capitalize()),
                    )
                })
                .collect();
            Some(TokenInfo {
                token_id: base58(token_id),
                contract_id: contract_id.clone(),
                position: *position,
                names,
                decimals: conventions.decimals(),
                issuer: issuer.clone(),
                issuer_name: None,
                transferable: config.is_transferable(),
                keeps_history: config.keeps_history().keeps_transfer_history(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capitalized_only_when_the_issuer_says_so() {
        assert_eq!(shown_form("evo coffee", true), "Evo coffee");
        assert_eq!(shown_form(" evo coffee ", false), "evo coffee");
        assert_eq!(shown_form("ёлка", true), "Ёлка");
        assert_eq!(shown_form("  ", true), "");
    }
}

/// A version 1 contract with one token per `(english name, decimals)`, at positions 0, 1, ...
#[cfg(test)]
pub(crate) fn test_contract(id: [u8; 32], owner: [u8; 32], tokens: &[(&str, u8)]) -> DataContract {
    use dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
    use dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
    use dpp::data_contract::associated_token::token_configuration_convention::v0::TokenConfigurationConventionV0;
    use dpp::data_contract::associated_token::token_configuration_localization::TokenConfigurationLocalization;
    use dpp::data_contract::associated_token::token_configuration_localization::v0::TokenConfigurationLocalizationV0;
    use dpp::data_contract::config::DataContractConfig;
    use dpp::data_contract::v1::DataContractV1;
    use dpp::version::PlatformVersion;

    let tokens = tokens
        .iter()
        .enumerate()
        .map(|(position, (name, decimals))| {
            let mut config = TokenConfigurationV0::default_most_restrictive();
            config.conventions = TokenConfigurationConvention::V0(TokenConfigurationConventionV0 {
                localizations: [(
                    "en".to_owned(),
                    TokenConfigurationLocalization::V0(TokenConfigurationLocalizationV0 {
                        should_capitalize: false,
                        singular_form: (*name).to_owned(),
                        plural_form: format!("{name}s"),
                    }),
                )]
                .into(),
                decimals: *decimals,
            });
            (position as u16, TokenConfiguration::V0(config))
        })
        .collect();
    DataContract::V1(DataContractV1 {
        id: id.into(),
        version: 1,
        owner_id: owner.into(),
        document_types: Default::default(),
        config: DataContractConfig::default_for_version(PlatformVersion::latest())
            .expect("a config"),
        schema_defs: None,
        created_at: None,
        updated_at: None,
        created_at_block_height: None,
        updated_at_block_height: None,
        created_at_epoch: None,
        updated_at_epoch: None,
        groups: Default::default(),
        tokens,
        keywords: Vec::new(),
        description: None,
    })
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn every_token_of_a_contract_with_the_id_platform_computes() {
        let contract = test_contract([3; 32], [9; 32], &[("coffee", 2), ("stamp", 0)]);
        let tokens = tokens_of(&contract);
        assert_eq!(tokens.len(), 2);
        for (position, token) in tokens.iter().enumerate() {
            let id = dpp::tokens::calculate_token_id(&[3; 32], position as u16);
            assert_eq!(
                token.token_id,
                dpp::prelude::Identifier::from(id).to_string(Encoding::Base58)
            );
            assert_eq!(token.position, position as u16);
            assert_eq!(
                token.contract_id,
                dpp::prelude::Identifier::from([3; 32]).to_string(Encoding::Base58)
            );
            assert_eq!(
                token.issuer,
                dpp::prelude::Identifier::from([9; 32]).to_string(Encoding::Base58)
            );
        }
        assert_eq!(
            (tokens[0].names.clone(), tokens[0].decimals),
            (vec![("en".to_owned(), "coffee".to_owned())], 2)
        );
        assert_eq!(
            (tokens[1].names.clone(), tokens[1].decimals),
            (vec![("en".to_owned(), "stamp".to_owned())], 0)
        );
        assert!(tokens[0].transferable, "a V0 configuration is transferable");
    }
}

/// A proved contract as `tokens.json` keeps it (FEAT-0019 АРХ-2): Platform's own serialization.
pub fn contract_bytes(contract: &DataContract) -> Result<Vec<u8>, String> {
    use dpp::serialization::PlatformSerializableWithPlatformVersion;
    contract
        .serialize_to_bytes_with_platform_version(dpp::version::PlatformVersion::latest())
        .map_err(|e| e.to_string())
}

/// A contract kept by [`contract_bytes`]; `None` when the bytes are not one. Decoded as untrusted
/// and validated in full: the file is on the person's disk, not a proof.
pub fn contract_from_bytes(bytes: &[u8]) -> Option<DataContract> {
    use dpp::serialization::PlatformDeserializableWithPotentialValidationFromVersionedStructureUntrusted;
    DataContract::versioned_deserialize_untrusted(
        bytes,
        true,
        dpp::version::PlatformVersion::latest(),
    )
    .ok()
}

#[cfg(test)]
mod bytes_tests {
    use super::*;

    #[test]
    fn a_kept_contract_comes_back_whole_and_garbage_does_not() {
        let contract = test_contract([3; 32], [9; 32], &[("coffee", 2)]);
        let bytes = contract_bytes(&contract).unwrap();
        let back = contract_from_bytes(&bytes).expect("the same contract");
        assert_eq!(tokens_of(&back), tokens_of(&contract));
        assert!(contract_from_bytes(&[]).is_none());
        assert!(contract_from_bytes(&[0xff; 40]).is_none());
        assert!(
            contract_from_bytes(&bytes[..bytes.len() / 2]).is_none(),
            "cut short"
        );
    }
}
