//! Platform tokens read with proofs (FEAT-0019 part 3): a token or a contract by its id, and the
//! contracts that carry a keyword. Each contract found is registered with the provider, so a
//! later proof about its tokens (balances, a transfer) is checked against it (АРХ-2).

use std::sync::Arc;

use dash_sdk::platform::{ContextProvider, DataContract, Fetch, Identifier};
use dpp::platform_value::string_encoding::Encoding;
use dpp::tokens::contract_info::TokenContractInfo;
use dpp::tokens::contract_info::v0::TokenContractInfoV0Accessors;
use evolite_api::tokens::TokenInfo;

use super::Chain;
use crate::platform::tokens::info;

/// What a search found: the tokens, and the proved contracts they belong to, as `tokens.json`
/// keeps them (by contract id, base58).
#[derive(Default)]
pub struct Found {
    pub tokens: Vec<TokenInfo>,
    pub contracts: Vec<(String, Vec<u8>)>,
}

impl Found {
    fn add(&mut self, contract: &DataContract, only: Option<u16>) {
        let tokens: Vec<TokenInfo> = info::tokens_of(contract)
            .into_iter()
            .filter(|t| only.is_none_or(|p| t.position == p))
            .collect();
        if tokens.is_empty() {
            return;
        }
        if let Ok(bytes) = info::contract_bytes(contract) {
            let id = tokens[0].contract_id.clone();
            if !self.contracts.iter().any(|(c, _)| *c == id) {
                self.contracts.push((id, bytes));
            }
        }
        self.tokens.extend(tokens);
    }
}

impl Chain {
    /// The token with id `id`, or every token of the contract with that id (one field takes
    /// both, UC-1). Nothing found is an empty list: the proof said there is no such token or
    /// contract.
    pub async fn find_token(&self, id: &str) -> Result<Found, String> {
        let id = Identifier::from_string(id.trim(), Encoding::Base58)
            .map_err(|_| "not a token or contract id".to_owned())?;
        self.ready()?;
        let sdk = self.sdk().await?;
        let mut found = Found::default();
        if let Some(token) = TokenContractInfo::fetch(&sdk, id)
            .await
            .map_err(|e| e.to_string())?
        {
            if let Some(contract) = self.proved_contract(&sdk, token.contract_id()).await? {
                found.add(&contract, Some(token.token_contract_position()));
            }
            return Ok(found);
        }
        if let Some(contract) = self.proved_contract(&sdk, id).await? {
            found.add(&contract, None);
        }
        Ok(found)
    }

    /// The tokens of the contracts that carry `keyword` (KeywordSearch, `contractKeywords`, an exact
    /// match). One page of [`KEYWORD_PAGE`] contracts: a keyword more contracts carry is too
    /// broad to choose from anyway. The keyword contract and the contracts come with proofs.
    pub async fn search_tokens(&self, keyword: &str) -> Result<Found, String> {
        use dash_sdk::drive::query::{WhereClause, WhereOperator};
        use dash_sdk::platform::{Document, DocumentQuery, FetchMany};
        use dpp::data_contracts::SystemDataContract;
        use dpp::document::DocumentV0Getters;
        use dpp::platform_value::Value;

        if !keyword_ok(keyword) {
            return Ok(Found::default());
        }
        self.ready()?;
        let sdk = self.sdk().await?;
        let Some(search) = self
            .proved_contract(&sdk, SystemDataContract::KeywordSearch.id())
            .await?
        else {
            return Err("the keyword search contract is not on this network".to_owned());
        };
        let mut query =
            DocumentQuery::new(search, "contractKeywords").map_err(|e| e.to_string())?;
        query.limit = KEYWORD_PAGE;
        let query = query.with_where(WhereClause {
            field: "keyword".into(),
            operator: WhereOperator::Equal,
            value: Value::Text(keyword.to_owned()),
        });
        let documents = Document::fetch_many(&sdk, query)
            .await
            .map_err(|e| e.to_string())?;
        let ids: Vec<Identifier> = documents
            .values()
            .flatten()
            .filter_map(|d| d.get("contractId")?.to_identifier().ok())
            .collect();
        let mut found = Found::default();
        if ids.is_empty() {
            return Ok(found);
        }
        self.ready()?;
        let contracts = DataContract::fetch_many(&sdk, ids)
            .await
            .map_err(|e| e.to_string())?;
        let provider = sdk.context_provider();
        for contract in contracts.into_values().flatten() {
            let contract = Arc::new(contract);
            if let Some(provider) = &provider {
                provider.register_data_contract(contract.clone());
            }
            found.add(&contract, None);
        }
        Ok(found)
    }

    /// Contract `id` with its proof, registered with the provider; `None`: the proof says
    /// there is none.
    async fn proved_contract(
        &self,
        sdk: &dash_sdk::Sdk,
        id: Identifier,
    ) -> Result<Option<Arc<DataContract>>, String> {
        self.ready()?;
        let contract = DataContract::fetch(sdk, id)
            .await
            .map_err(|e| e.to_string())?
            .map(Arc::new);
        if let (Some(contract), Some(provider)) = (&contract, sdk.context_provider()) {
            provider.register_data_contract(contract.clone());
        }
        Ok(contract)
    }

    /// Hands contracts kept in `tokens.json` to the provider at start (AC-T2), so a token's
    /// proofs are checked after a restart before the contract is read again. A damaged one is
    /// left out: it is read again when needed.
    pub async fn register_kept_contracts(&self, kept: &[Vec<u8>]) {
        let Ok(sdk) = self.sdk().await else { return };
        let Some(provider) = sdk.context_provider() else {
            return;
        };
        for bytes in kept {
            if let Some(contract) = info::contract_from_bytes(bytes) {
                provider.register_data_contract(Arc::new(contract));
            }
        }
    }
}

/// Contracts one keyword search reads at most.
const KEYWORD_PAGE: u32 = 100;

/// A keyword Platform keeps: 3 to 50 characters, lower case (FEAT-0019 АРХ-5). Another one is
/// not asked: nothing could match it.
fn keyword_ok(keyword: &str) -> bool {
    (3..=50).contains(&keyword.chars().count()) && keyword.to_lowercase() == keyword
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_keyword_platform_could_keep_is_asked() {
        assert!(keyword_ok("coffee") && keyword_ok("кофе") && keyword_ok("abc"));
        assert!(!keyword_ok("ab"), "3 characters at least");
        assert!(!keyword_ok(&"a".repeat(51)), "50 at most");
        assert!(!keyword_ok("Coffee"), "lower case");
    }
}

/// What Platform proves about tokens on one identity: its balance, whether the token is paused
/// and whether this balance is frozen, by token id (base58). A token the proof says nothing of
/// has no entry; a balance the proof says is absent is 0 (it never had any).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Facts {
    pub balances: std::collections::HashMap<String, u64>,
    pub paused: std::collections::HashMap<String, bool>,
    pub frozen: std::collections::HashMap<String, bool>,
}

/// Token ids in one request (Platform's limit).
const PER_REQUEST: usize = 100;

impl Chain {
    /// [`Facts`] of `tokens` on `identity`, three proved requests per [`PER_REQUEST`] tokens.
    pub async fn token_facts(&self, identity: &str, tokens: &[String]) -> Result<Facts, String> {
        use dash_sdk::platform::FetchMany;
        use dash_sdk::platform::tokens::identity_token_balances::{
            IdentityTokenBalances, IdentityTokenBalancesQuery,
        };
        use dash_sdk::platform::tokens::token_info::IdentityTokenInfosQuery;
        use dash_sdk::query_types::token_info::IdentityTokenInfos;
        use dash_sdk::query_types::token_status::TokenStatuses;
        use dpp::balances::credits::TokenAmount;
        use dpp::tokens::info::IdentityTokenInfo;
        use dpp::tokens::info::v0::IdentityTokenInfoV0Accessors;
        use dpp::tokens::status::TokenStatus;
        use dpp::tokens::status::v0::TokenStatusV0Accessors;

        let id = |s: &str| Identifier::from_string(s, Encoding::Base58).map_err(|e| e.to_string());
        let identity_id = id(identity)?;
        let ids = tokens
            .iter()
            .map(|t| id(t))
            .collect::<Result<Vec<_>, _>>()?;
        let base58 = |i: &Identifier| i.to_string(Encoding::Base58);
        let mut facts = Facts::default();
        if ids.is_empty() {
            return Ok(facts);
        }
        let sdk = self.sdk().await?;
        for batch in ids.chunks(PER_REQUEST) {
            self.ready()?;
            let query = IdentityTokenBalancesQuery {
                identity_id,
                token_ids: batch.to_vec(),
            };
            let balances: IdentityTokenBalances = TokenAmount::fetch_many(&sdk, query)
                .await
                .map_err(|e| e.to_string())?;
            for (token, amount) in balances.iter() {
                facts.balances.insert(base58(token), amount.unwrap_or(0));
            }
            self.ready()?;
            let statuses: TokenStatuses = TokenStatus::fetch_many(&sdk, batch.to_vec())
                .await
                .map_err(|e| e.to_string())?;
            for token in batch {
                let paused = statuses
                    .get(token)
                    .cloned()
                    .flatten()
                    .is_some_and(|s| s.paused());
                facts.paused.insert(base58(token), paused);
            }
            self.ready()?;
            let query = IdentityTokenInfosQuery {
                identity_id,
                token_ids: batch.to_vec(),
            };
            let infos: IdentityTokenInfos = IdentityTokenInfo::fetch_many(&sdk, query)
                .await
                .map_err(|e| e.to_string())?;
            for token in batch {
                let frozen = infos
                    .get(token)
                    .cloned()
                    .flatten()
                    .is_some_and(|i| i.frozen());
                facts.frozen.insert(base58(token), frozen);
            }
        }
        Ok(facts)
    }
}

/// What the search keeps of a contract with tokens: its token ids (base58).
pub type CrawlTokens = Vec<String>;

/// Why a step of the search did not get its answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrawlFailure {
    /// A node asked to slow down (429, `ResourceExhausted`): wait this long. The node is not
    /// held against (BUG-78): the search's requests turn the SDK's ban off.
    Slow(std::time::Duration),
    /// The network did not answer, or the proof did not check.
    Failed(String),
}

/// How long to wait after a rate limit that did not say.
const SLOW_DEFAULT: std::time::Duration = std::time::Duration::from_secs(60);

/// A rate limit in `error`, with how long to wait.
fn slow_down(error: &dash_sdk::Error) -> Option<std::time::Duration> {
    use dash_sdk::dapi_client::CanRetry;
    if let dash_sdk::Error::DapiClientError(e) = error
        && let Some(wait) = e.rate_limit_ban_duration()
    {
        return Some(wait);
    }
    let text = error.to_string().to_lowercase();
    (text.contains("resourceexhausted")
        || text.contains("resource exhausted")
        || text.contains("429"))
    .then_some(SLOW_DEFAULT)
}

fn failure(error: dash_sdk::Error) -> CrawlFailure {
    match slow_down(&error) {
        Some(wait) => CrawlFailure::Slow(wait),
        None => CrawlFailure::Failed(error.to_string()),
    }
}

impl Chain {
    /// The proved answer to one step of the search ([`crawl::Ask`]). One request, no retries and
    /// no ban: the search keeps its own pace ([`crate::platform::tokens::pace`]). With it, the
    /// contracts read that carry tokens, as [`Found`]: the search follows the ones the wallet
    /// holds from them, without reading them again (dev2-4).
    pub async fn crawl_step(
        &self,
        ask: &crate::platform::tokens::crawl::Ask,
    ) -> Result<(crate::platform::tokens::crawl::Reply<CrawlTokens>, Found), CrawlFailure> {
        use crate::platform::tokens::crawl::{Ask, Read, Reply};
        use dash_sdk::RequestSettings;
        use dash_sdk::platform::FetchMany;
        use dash_sdk::platform::data_contracts_by_range::{
            DataContractsByRange, DataContractsByRangeQuery, DataContractsByRangeStart,
        };
        use dash_sdk::platform::data_contracts_latest_versions::{
            DataContractLatestVersion, DataContractsLatestVersions,
        };
        use dpp::data_contract::accessors::v0::DataContractV0Getters;

        self.ready().map_err(CrawlFailure::Failed)?;
        let sdk = self.sdk().await.map_err(CrawlFailure::Failed)?;
        let mut settings = RequestSettings::default();
        settings.ban_failed_address = Some(false);
        settings.retries = Some(0);
        let id = |bytes: &[u8; 32]| Identifier::from(*bytes);
        let mut found = Found::default();
        // A contract read with its proof: what the crawl keeps of it, and its tokens found.
        let mut read = |id: Identifier, contract: &DataContract| {
            found.add(contract, None);
            let tokens: Vec<String> = info::tokens_of(contract)
                .into_iter()
                .map(|t| t.token_id)
                .collect();
            Read {
                id: id.to_buffer(),
                version: contract.version(),
                tokens: (!tokens.is_empty()).then_some(tokens),
            }
        };
        let reply = match ask {
            Ask::Pages { after, limit } => {
                let query = DataContractsByRangeQuery {
                    start: after.map(|a| DataContractsByRangeStart::After(id(&a))),
                    limit: Some(*limit),
                    ids_only: false,
                };
                let page = DataContractsByRange::fetch_with_settings(&sdk, query, settings)
                    .await
                    .map_err(failure)?
                    .unwrap_or_default();
                // Bodies only from the proved page: `verify_contracts_by_range` checked them.
                let mut reads = Vec::with_capacity(page.0.len());
                for (k, contract) in page.0 {
                    let contract = contract.ok_or_else(|| {
                        CrawlFailure::Failed("a page came without its contracts".into())
                    })?;
                    reads.push(read(k, &contract));
                }
                Reply::Pages(reads)
            }
            Ask::Ids { after, limit } => {
                let query = DataContractsByRangeQuery {
                    start: after.map(|a| DataContractsByRangeStart::After(id(&a))),
                    limit: Some(*limit),
                    ids_only: true,
                };
                let page = DataContractsByRange::fetch_with_settings(&sdk, query, settings)
                    .await
                    .map_err(failure)?
                    .unwrap_or_default();
                Reply::Ids(page.0.keys().map(|k| k.to_buffer()).collect())
            }
            Ask::Versions(ids) => {
                let ids: Vec<Identifier> = ids.iter().map(id).collect();
                let (versions, _): (DataContractsLatestVersions, _) =
                    DataContractLatestVersion::fetch_many_with_metadata(&sdk, ids, Some(settings))
                        .await
                        .map_err(failure)?;
                Reply::Versions(
                    versions
                        .0
                        .iter()
                        .filter_map(|(k, v)| Some((k.to_buffer(), v.as_ref()?.version)))
                        .collect(),
                )
            }
            Ask::Contracts(ids) => {
                let ids: Vec<Identifier> = ids.iter().map(id).collect();
                let (contracts, _): (dash_sdk::query_types::DataContracts, _) =
                    DataContract::fetch_many_with_metadata(&sdk, ids, Some(settings))
                        .await
                        .map_err(failure)?;
                Reply::Contracts(
                    contracts
                        .into_iter()
                        .filter_map(|(k, contract)| Some(read(k, &contract?)))
                        .collect(),
                )
            }
        };
        Ok((reply, found))
    }
}

#[cfg(test)]
mod slow_tests {
    use super::*;

    #[test]
    fn a_rate_limit_is_a_pause_and_anything_else_a_failure() {
        let limited =
            dash_sdk::Error::Generic("status: ResourceExhausted, message: too many".into());
        assert_eq!(failure(limited), CrawlFailure::Slow(SLOW_DEFAULT));
        let other = dash_sdk::Error::Generic("connection refused".into());
        assert!(matches!(failure(other), CrawlFailure::Failed(_)));
    }
}

impl Chain {
    /// The balances of `tokens` on `identity` that are not zero, proved, one request per
    /// [`PER_REQUEST`] tokens with `gap` between requests (the search's pace).
    pub async fn token_balances(
        &self,
        identity: &str,
        tokens: &[String],
        gap: std::time::Duration,
    ) -> Result<std::collections::HashMap<String, u64>, String> {
        use dash_sdk::platform::FetchMany;
        use dash_sdk::platform::tokens::identity_token_balances::{
            IdentityTokenBalances, IdentityTokenBalancesQuery,
        };
        use dpp::balances::credits::TokenAmount;

        let id = |s: &str| Identifier::from_string(s, Encoding::Base58).map_err(|e| e.to_string());
        let identity_id = id(identity)?;
        let ids = tokens
            .iter()
            .map(|t| id(t))
            .collect::<Result<Vec<_>, _>>()?;
        let sdk = self.sdk().await?;
        let mut held = std::collections::HashMap::new();
        for (n, batch) in ids.chunks(PER_REQUEST).enumerate() {
            if n > 0 {
                tokio::time::sleep(gap).await;
            }
            self.ready()?;
            let query = IdentityTokenBalancesQuery {
                identity_id,
                token_ids: batch.to_vec(),
            };
            let balances: IdentityTokenBalances = TokenAmount::fetch_many(&sdk, query)
                .await
                .map_err(|e| e.to_string())?;
            for (token, amount) in balances.iter() {
                if let Some(amount) = amount.filter(|a| *a > 0) {
                    held.insert(token.to_string(Encoding::Base58), amount);
                }
            }
        }
        Ok(held)
    }
}

impl Chain {
    /// Sends `amount` of the token at `position` of `contract` from the shown identity to
    /// identity `to`, signed with its CRITICAL authentication key (platform-wallet's choice; an
    /// identity without one is refused by it). No note in stage 1. The sender's balance after
    /// it, when the proved answer says (`None` for a token that keeps its history there).
    pub async fn send_token(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        keys: &key_wallet::Wallet,
        contract: &str,
        position: u16,
        to: Identifier,
        amount: u64,
    ) -> Result<Option<u64>, String> {
        use dash_sdk::platform::tokens::transitions::TransferResult;
        use dpp::identity::accessors::IdentityGettersV0;

        let contract =
            Identifier::from_string(contract, Encoding::Base58).map_err(|e| e.to_string())?;
        self.ready()?;
        let (from, signer) = Self::first_identity(wallet, keys).await?;
        let result = wallet
            .identity()
            .token_transfer_with_external_signer(
                from.id(),
                contract,
                position,
                to,
                amount,
                None,
                &signer,
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(match result {
            TransferResult::IdentitiesBalances(balances) => balances.get(&from.id()).copied(),
            _ => None,
        })
    }
}
