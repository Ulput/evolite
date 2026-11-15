//! Masternode votes on contested DPNS names (FEAT-0021, АРХ-1): whether a name's domain exists,
//! a contest's proved vote state, and the fund a claim on it pays now. One proved query each; the
//! vote state is read by the normalized label, as the network keys it.

use std::sync::Arc;

use dash_sdk::query_types::Contenders;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::document::DocumentV0Getters;
use dpp::platform_value::string_encoding::Encoding;
use dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dpp::voting::vote_info_storage::contested_document_vote_poll_winner_info::ContestedDocumentVotePollWinnerInfo;
use evolite_local::name_requests::{Contender, ContestSnapshot, Winner};

use super::Chain;

impl Chain {
    /// Whether a domain with this label exists (proved). For a contested label «no domain» is not
    /// «free»: the name may be in a vote or blocked (R-1), which [`Self::contest`] tells.
    pub async fn name_awarded(&self, label: &str) -> Result<bool, String> {
        Ok(!self.name_available(label).await?)
    }

    /// The vote state of the contest on `label` (proved, АРХ-1) and the fund a claim joining it
    /// pays now, in duffs (it doubles past 250 claims, from protocol 14). An empty snapshot when no
    /// one has asked for it.
    pub async fn contest(
        &self,
        label: &str,
    ) -> Result<(ContestSnapshot, evolite_api::Duffs), String> {
        self.ready()?;
        let sdk = self.sdk().await?;
        let normalized = evolite_common::dpns::normalized(&label.to_lowercase());
        let state = sdk
            .get_contested_dpns_vote_state(&normalized, None)
            .await
            .map_err(|e| e.to_string())?;
        let contract = Arc::new(
            load_system_data_contract(SystemDataContract::DPNS, sdk.version())
                .map_err(|e| e.to_string())?,
        );
        let snapshot = snapshot_of(&state, &contract, sdk.version());
        let contenders = u16::try_from(snapshot.contenders.len()).unwrap_or(u16::MAX);
        let fund = dpp::voting::vote_polls::contested_document_resource_vote_poll::required_vote_resolution_fund_to_join(
            &contract.id(),
            "domain",
            contenders,
            sdk.version(),
        );
        Ok((snapshot, fund.div_ceil(1000) as evolite_api::Duffs))
    }
}

impl Chain {
    /// Reads `identity`'s names from Platform into the store again: after a vote it won
    /// (FEAT-0021 AC-T7), the domain Platform awarded it.
    pub async fn sync_names(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        identity: &str,
    ) -> Result<(), String> {
        let id = dpp::prelude::Identifier::from_string(identity, Encoding::Base58)
            .map_err(|e| e.to_string())?;
        wallet
            .identity()
            .sync_dpns_names(&id)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// The proved vote state without the SDK's types: claims with their votes and when each was made
/// (the earliest is when the contest started), the votes to block and to abstain, the winner.
fn snapshot_of(
    state: &Contenders,
    contract: &dpp::data_contract::DataContract,
    version: &dpp::version::PlatformVersion,
) -> ContestSnapshot {
    let documents = contract
        .document_type_cloned_for_name("domain")
        .ok()
        .and_then(|document_type| state.to_contenders(&document_type, version).ok())
        .unwrap_or_default();
    let contenders = state
        .contenders
        .iter()
        .map(|(id, contender)| Contender {
            identity: id.to_string(Encoding::Base58),
            votes: contender.vote_tally().unwrap_or(0),
            created_at: documents
                .get(id)
                .and_then(|c| c.document().as_ref())
                .and_then(|d| d.created_at())
                .map(|ms| (ms / 1000) as evolite_api::Timestamp),
        })
        .collect();
    let winner = state.winner.as_ref().map(|(info, _)| match info {
        ContestedDocumentVotePollWinnerInfo::WonByIdentity(id) => {
            Winner::Identity(id.to_string(Encoding::Base58))
        }
        ContestedDocumentVotePollWinnerInfo::Locked => Winner::Locked,
        ContestedDocumentVotePollWinnerInfo::NoWinner => Winner::NoWinner,
    });
    ContestSnapshot {
        winner,
        contenders,
        lock: state.lock_vote_tally.unwrap_or(0),
        abstain: state.abstain_vote_tally.unwrap_or(0),
    }
}

/// The identity-verify contract by network (FEAT-0021 АРХ-6, AC-T13): where DashPay keeps the
/// link «where my identity is confirmed» of a name's claim (`identityVerify` documents).
pub fn identity_verify_contract_id(network: evolite_api::Network) -> &'static str {
    match network {
        evolite_api::Network::Mainnet => "EVKMFboB3QBUa9Jo7PP5bsLyohzUz8zvw5c2gJs1SfcX",
        evolite_api::Network::Testnet => "Bhptm3yBDhLkRNt7ofjpwaBHhMUKjDrQoPufKzQaxmpK",
    }
}

impl Chain {
    /// The identity-verify contract of this network, proved (AC-T13); `None` when the network has
    /// none: the link is then not offered, and the claim works without it.
    pub async fn identity_verify_contract(
        &self,
    ) -> Result<Option<Arc<dpp::data_contract::DataContract>>, String> {
        use dash_sdk::platform::Fetch;
        self.ready()?;
        let sdk = self.sdk().await?;
        let id = dpp::prelude::Identifier::from_string(
            identity_verify_contract_id(crate::network::current()),
            Encoding::Base58,
        )
        .map_err(|e| e.to_string())?;
        Ok(dpp::data_contract::DataContract::fetch(&sdk, id)
            .await
            .map_err(|e| e.to_string())?
            .map(Arc::new))
    }
}

/// The `identityVerify` document's three fields (FEAT-0021 АРХ-6, AC-T10), as the network's
/// schema has them: the label in DPNS's normal form (lower case, o → 0, i and l → 1), the domain
/// `dash`, the link.
pub fn identity_verify_json(label: &str, url: &str) -> String {
    serde_json::json!({
        "normalizedLabel": evolite_common::dpns::normalized(&label.to_lowercase()),
        "normalizedParentDomainName": "dash",
        "url": url,
    })
    .to_string()
}

impl Chain {
    /// Writes, replaces or (`url: None`) deletes the link «where my identity is confirmed» of the
    /// wallet's first identity's claim on `label` (АРХ-6, AC-T12): one document per identity and
    /// label, found by its index first, so a second write replaces it and never makes a second.
    /// Signed with the identity's HIGH AUTHENTICATION key, as DashPay Android does.
    pub async fn write_identity_link(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        keys: &key_wallet::Wallet,
        as_identity: &dpp::prelude::Identifier,
        label: &str,
        url: Option<&str>,
    ) -> Result<(), String> {
        use dash_sdk::drive::query::{WhereClause, WhereOperator};
        use dash_sdk::platform::{DocumentQuery, FetchMany};
        use dpp::document::{Document, DocumentV0Getters};
        use dpp::identity::accessors::IdentityGettersV0;
        use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        use dpp::identity::signer::Signer;
        use dpp::identity::{IdentityPublicKey, KeyType, Purpose, SecurityLevel};
        use dpp::platform_value::Value;

        let contract = self
            .identity_verify_contract()
            .await?
            .ok_or(tr!("This network has no contract for identity links"))?;
        // Signed as the identity whose claim was checked, even if "Act as" moved on since
        // (FEAT-0021 r2 Should 3, as BUG-13).
        let (identity, signer) = Self::identity_as(wallet, keys, as_identity).await?;
        let key = identity
            .public_keys()
            .values()
            .find(|k| {
                k.purpose() == Purpose::AUTHENTICATION
                    && k.security_level() == SecurityLevel::HIGH
                    && k.key_type() == KeyType::ECDSA_SECP256K1
                    && k.disabled_at().is_none()
                    // A key from this phrase only: another enabled HIGH key would fail the
                    // signature, and the refusal below would not be true (r2 note).
                    && Signer::<IdentityPublicKey>::can_sign_with(&signer, k)
            })
            .ok_or(tr!("The identity has no HIGH key from your phrase: there is nothing to sign the link with"))?
            .id();
        let sdk = self.sdk().await?;
        let normalized = evolite_common::dpns::normalized(&label.to_lowercase());
        let query = DocumentQuery::new(contract.clone(), "identityVerify")
            .map_err(|e| e.to_string())?
            .with_where(WhereClause {
                field: "$ownerId".into(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(identity.id().to_buffer()),
            })
            .with_where(WhereClause {
                field: "normalizedParentDomainName".into(),
                operator: WhereOperator::Equal,
                value: Value::Text("dash".into()),
            })
            .with_where(WhereClause {
                field: "normalizedLabel".into(),
                operator: WhereOperator::Equal,
                value: Value::Text(normalized),
            })
            .with_limit(1);
        let existing = Document::fetch_many(&sdk, query)
            .await
            .map_err(|e| e.to_string())?
            .into_values()
            .flatten()
            .next()
            .map(|d| d.id());
        let contract_id = contract.id();
        let documents = wallet.identity();
        let id = identity.id();
        let result = match (existing, url) {
            (Some(doc), Some(url)) => documents
                .replace_document_with_signer(
                    &id,
                    &contract_id,
                    "identityVerify",
                    &doc,
                    &identity_verify_json(label, url),
                    key,
                    &signer,
                )
                .await
                .map(|_| ()),
            (None, Some(url)) => documents
                .create_document_with_signer(
                    &id,
                    &contract_id,
                    "identityVerify",
                    &identity_verify_json(label, url),
                    None,
                    &signer,
                )
                .await
                .map(|_| ()),
            (Some(doc), None) => documents
                .delete_document_with_signer(
                    &id,
                    &contract_id,
                    "identityVerify",
                    &doc,
                    key,
                    &signer,
                )
                .await
                .map(|_| ()),
            (None, None) => Ok(()),
        };
        result.map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-T10: the document's fields as the schema has them; «Alice», «al1ce» and «AL1CE» are one
    /// label to DPNS, and so one link document.
    #[test]
    fn an_identity_link_names_the_label_as_dpns_does() {
        let fields = |label: &str| -> serde_json::Value {
            serde_json::from_str(&identity_verify_json(label, "https://example.com/me")).unwrap()
        };
        let alice = fields("Alice");
        assert_eq!(alice["normalizedLabel"], "a11ce");
        assert_eq!(alice["normalizedParentDomainName"], "dash");
        assert_eq!(alice["url"], "https://example.com/me");
        assert_eq!(
            alice.as_object().unwrap().len(),
            3,
            "the schema's three fields"
        );
        for same in ["al1ce", "AL1CE"] {
            assert_eq!(
                fields(same)["normalizedLabel"],
                alice["normalizedLabel"],
                "{same}"
            );
        }
    }
}
