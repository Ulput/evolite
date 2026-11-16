//! Claim and purchase of tokens (FEAT-0019 T-2): what a claim would pay and what a count costs,
//! read with proofs before the buttons are shown; the claim and the purchase themselves, signed
//! with the identity's CRITICAL authentication key (platform-wallet's choice, as the transfer).

use std::sync::Arc;

use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier};
use dash_sdk::platform::fetch_current_no_parameters::FetchCurrent;
use dpp::balances::credits::TokenAmount;
use dpp::block::block_info::BlockInfo;
use dpp::block::epoch::Epoch;
use dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dpp::data_contract::DataContract;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::accessors::v1::DataContractV1Getters;
use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dpp::data_contract::associated_token::token_perpetual_distribution::methods::v0::TokenPerpetualDistributionV0Accessors;
use dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_moment::RewardDistributionMoment;
use dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_type::RewardDistributionType;
use dpp::data_contract::associated_token::token_pre_programmed_distribution::accessors::v0::TokenPreProgrammedDistributionV0Methods;
use dpp::document::DocumentV0Getters;
use dpp::platform_value::Value;
use dpp::platform_value::string_encoding::Encoding;
use dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};

use super::Chain;
use crate::platform::tokens::claim::{self, Estimate, Scheduled};

/// How often a perpetual distribution pays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Every {
    Blocks(u64),
    Millis(u64),
    Epochs(u16),
}

/// A perpetual distribution this identity may claim (T-2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Perpetual {
    pub every: Every,
    /// What a claim pays now; `None` when it cannot be counted before (K1u).
    pub estimate: Option<Estimate>,
    /// Whether a payout passed since the last claim (the row's flag, stage 1's rule).
    pub due: bool,
}

/// What this identity may claim of a token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Claimable {
    pub perpetual: Option<Perpetual>,
    /// A pre-programmed distribution naming this identity (T-2.2).
    pub scheduled: Option<Scheduled>,
}

/// What a token costs and how much more of it may be made (T-2.4, R-21).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    /// The issuer's price; `None`: the token is not sold.
    pub price: Option<TokenPricingSchedule>,
    /// How many units may still be made under `max_supply`; `None`: no limit.
    pub left: Option<TokenAmount>,
}

/// A claim document of TokenHistory, as the wallet keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimDoc {
    pub id: String,
    pub amount: TokenAmount,
    /// 0 pre-programmed, 1 perpetual, 2 once per identity (the schema of TokenHistory v2).
    pub kind: u8,
    pub at_ms: u64,
}

/// What a purchase came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bought {
    /// The token keeps its purchases: the proved document, its amount and key.
    Document { id: String, amount: TokenAmount },
    /// No history: the buyer's balance after it.
    Balance(TokenAmount),
}

/// Why a purchase was not made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuyRefused {
    /// The price grew over the agreed one: nothing was taken (АРХ-10).
    PriceGrew,
    Other(String),
}

/// The block a claim would be counted at, from a proved answer's metadata (Platform's height,
/// time and epoch), as rs-drive takes it.
fn block_of(metadata: &dash_sdk::dapi_grpc::platform::v0::ResponseMetadata) -> BlockInfo {
    BlockInfo {
        time_ms: metadata.time_ms,
        height: metadata.height,
        core_height: metadata.core_chain_locked_height,
        epoch: Epoch::new(metadata.epoch as u16).unwrap_or_default(),
    }
}

fn every(kind: &RewardDistributionType) -> Every {
    match kind {
        RewardDistributionType::BlockBasedDistribution { interval, .. } => Every::Blocks(*interval),
        RewardDistributionType::TimeBasedDistribution { interval, .. } => Every::Millis(*interval),
        RewardDistributionType::EpochBasedDistribution { interval, .. } => Every::Epochs(*interval),
    }
}

fn id_of(s: &str) -> Result<Identifier, String> {
    Identifier::from_string(s, Encoding::Base58).map_err(|e| e.to_string())
}

/// Claim documents read at most, in pages of 100.
const CLAIM_PAGES: usize = 10;

impl Chain {
    /// What `identity` may claim of the token at `position` of `contract`, proved (T-2.1, T-2.2):
    /// the last perpetual claim and the current block for the estimate; the pre-programmed
    /// payouts already received, counted from the `claim` documents (Platform pays the earliest
    /// due entry each time, so the n-th paid is the n-th entry). Nothing to claim: the default.
    pub async fn claimable(
        &self,
        identity: &str,
        contract: &DataContract,
        position: u16,
    ) -> Result<Claimable, String> {
        let me = id_of(identity)?;
        let Some(config) = contract.tokens().get(&position) else {
            return Ok(Claimable::default());
        };
        let token = contract
            .token_id(position)
            .ok_or("no token at the position")?;
        let rules = config.distribution_rules();
        let mut found = Claimable::default();
        let sdk = self.sdk().await?;
        if let Some(perpetual) = rules.perpetual_distribution()
            && claim::may_claim(&perpetual.distribution_recipient(), me, contract.owner_id())
        {
            self.ready()?;
            let last: Option<RewardDistributionMoment> = RewardDistributionMoment::fetch(
                &sdk,
                dash_sdk::platform::query::TokenLastClaimQuery {
                    token_id: token,
                    identity_id: me,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
            self.ready()?;
            let (_, metadata) = ExtendedEpochInfo::fetch_current_with_metadata(&sdk)
                .await
                .map_err(|e| e.to_string())?;
            let block = block_of(&metadata);
            let kind = perpetual.distribution_type();
            let creation = kind.contract_creation_moment(contract);
            let estimate = creation.and_then(|creation| {
                claim::estimate(perpetual, creation, last, &block, sdk.version()).ok()
            });
            let now = {
                use dpp::data_contract::associated_token::token_perpetual_distribution::methods::v0::TokenPerpetualDistributionV0Methods;
                perpetual.current_interval(&block)
            };
            let since = last.or_else(|| creation.and_then(|c| c.cycle_start(kind.interval()).ok()));
            let due =
                since.is_some_and(|since| claim::interval_passed(kind.interval(), since, now));
            found.perpetual = Some(Perpetual {
                every: every(kind),
                estimate,
                due,
            });
        }
        if let Some(plan) = rules.pre_programmed_distribution() {
            let schedule = plan.distributions();
            if schedule.values().any(|to| to.contains_key(&me)) {
                let paid = self
                    .token_claims(&token.to_string(Encoding::Base58), identity)
                    .await?
                    .iter()
                    .filter(|c| c.kind == 0)
                    .count();
                let mine: Vec<u64> = schedule
                    .iter()
                    .filter(|(_, to)| to.contains_key(&me))
                    .map(|(at, _)| *at)
                    .collect();
                let last_paid = paid.checked_sub(1).and_then(|n| mine.get(n).copied());
                self.ready()?;
                let (_, metadata) = ExtendedEpochInfo::fetch_current_with_metadata(&sdk)
                    .await
                    .map_err(|e| e.to_string())?;
                found.scheduled = Some(claim::scheduled(schedule, me, last_paid, metadata.time_ms));
            }
        }
        Ok(found)
    }

    /// Adds to `facts` what `identity` may claim of each token and what each costs (T-2), the
    /// contracts given as `(token id, kept contract bytes, position)`. A token whose read fails
    /// has no entry: no button is shown on what was not proved.
    pub async fn token_extras(
        &self,
        identity: &str,
        tokens: &[(String, Vec<u8>, u16)],
        facts: &mut super::TokenFacts,
    ) {
        facts.read_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        for (token, bytes, position) in tokens {
            let Some(contract) = crate::platform::tokens::info::contract_from_bytes(bytes) else {
                continue;
            };
            if let Ok(offer) = self.token_offer(&contract, *position).await
                && offer.price.is_some()
            {
                facts.offers.insert(token.clone(), offer);
            }
            if let Ok(claimable) = self.claimable(identity, &contract, *position).await
                && (claimable.perpetual.is_some() || claimable.scheduled.is_some())
            {
                facts.claims.insert(token.clone(), claimable);
            }
        }
    }

    /// The `claim` documents of `token` to `identity` (both base58), oldest first, proved.
    pub async fn token_claims(&self, token: &str, identity: &str) -> Result<Vec<ClaimDoc>, String> {
        let sdk = self.sdk().await?;
        let contract = load_system_data_contract(SystemDataContract::TokenHistory, sdk.version())
            .map(Arc::new)
            .map_err(|e| format!("{e:?}"))?;
        let (token_id, me) = (id_of(token)?, id_of(identity)?);
        let mut all: Vec<ClaimDoc> = Vec::new();
        let mut after: Option<u64> = None;
        for _ in 0..CLAIM_PAGES {
            self.ready()?;
            let mut query = DocumentQuery::new(contract.clone(), "claim")
                .map_err(|e| format!("{e:?}"))?
                .with_where(WhereClause {
                    field: "tokenId".into(),
                    operator: WhereOperator::Equal,
                    value: Value::Identifier(token_id.to_buffer()),
                })
                .with_where(WhereClause {
                    field: "recipientId".into(),
                    operator: WhereOperator::Equal,
                    value: Value::Identifier(me.to_buffer()),
                })
                .with_order_by(OrderClause {
                    field: "$createdAt".into(),
                    ascending: true,
                })
                .with_limit(100);
            if let Some(at) = after {
                query = query.with_where(WhereClause {
                    field: "$createdAt".into(),
                    operator: WhereOperator::GreaterThanOrEquals,
                    value: Value::U64(at),
                });
            }
            let page: Vec<ClaimDoc> = Document::fetch_many(&sdk, query)
                .await
                .map_err(|e| format!("{e:?}"))?
                .into_values()
                .flatten()
                .filter_map(|d| claim_doc(&d))
                .filter(|d| !all.iter().any(|a| a.id == d.id))
                .collect();
            let full = page.len() >= 100;
            after = page.iter().map(|d| d.at_ms).max().or(after);
            all.extend(page);
            if !full {
                break;
            }
        }
        all.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then_with(|| a.id.cmp(&b.id)));
        Ok(all)
    }

    /// The price of the token at `position` of `contract` and how much of it may still be made,
    /// proved (T-2.4, R-21).
    pub async fn token_offer(
        &self,
        contract: &DataContract,
        position: u16,
    ) -> Result<Offer, String> {
        use dpp::balances::total_single_token_balance::TotalSingleTokenBalance;
        let token = contract
            .token_id(position)
            .ok_or("no token at the position")?;
        let max = contract
            .tokens()
            .get(&position)
            .and_then(|c| c.max_supply());
        let sdk = self.sdk().await?;
        self.ready()?;
        let prices = TokenPricingSchedule::fetch_many(&sdk, &[token][..])
            .await
            .map_err(|e| e.to_string())?;
        let price = prices.get(&token).cloned().flatten();
        let left = match max {
            None => None,
            Some(max) => {
                self.ready()?;
                let supply = TotalSingleTokenBalance::fetch(&sdk, token)
                    .await
                    .map_err(|e| e.to_string())?
                    .map_or(0, |s| s.token_supply.max(0) as u64);
                Some(max.saturating_sub(supply))
            }
        };
        Ok(Offer { price, left })
    }

    /// Claims the token at `position` of `contract` for the shown identity: the perpetual
    /// distribution, or the pre-programmed payout due (`scheduled`). The proved `claim` document.
    pub async fn claim_token(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        keys: &key_wallet::Wallet,
        me: &Identifier,
        contract: &str,
        position: u16,
        scheduled: bool,
    ) -> Result<ClaimDoc, String> {
        use dash_sdk::platform::tokens::transitions::ClaimResult;
        use dpp::identity::accessors::IdentityGettersV0;
        let contract = id_of(contract)?;
        self.ready()?;
        let (me, signer) = Self::signing(wallet, keys, me).await?;
        let kind = if scheduled {
            TokenDistributionType::PreProgrammed
        } else {
            TokenDistributionType::Perpetual
        };
        let result = wallet
            .identity()
            .token_claim_with_external_signer(me.id(), contract, position, kind, None, &signer)
            .await
            .map_err(|e| e.to_string())?;
        let doc = match result {
            ClaimResult::Document(doc) | ClaimResult::GroupActionWithDocument(_, doc) => doc,
        };
        claim_doc(&doc).ok_or_else(|| "the claim's document has no amount".into())
    }

    /// Buys `count` units of the token at `position` of `contract` for the shown identity, at
    /// most `agreed` credits in all (the total shown).
    #[allow(clippy::too_many_arguments)]
    pub async fn buy_token(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        keys: &key_wallet::Wallet,
        me: &Identifier,
        contract: &str,
        position: u16,
        count: TokenAmount,
        agreed: dpp::fee::Credits,
    ) -> Result<Bought, BuyRefused> {
        use dash_sdk::platform::tokens::transitions::DirectPurchaseResult;
        use dpp::identity::accessors::IdentityGettersV0;
        let contract = id_of(contract).map_err(BuyRefused::Other)?;
        self.ready().map_err(BuyRefused::Other)?;
        let (me, signer) = Self::signing(wallet, keys, me)
            .await
            .map_err(BuyRefused::Other)?;
        let result = wallet
            .identity()
            .token_purchase_with_external_signer(
                me.id(),
                contract,
                position,
                count,
                agreed,
                &signer,
            )
            .await
            .map_err(|e| {
                let text = format!("{e:?}");
                if text.contains("PriceTooLow") {
                    BuyRefused::PriceGrew
                } else {
                    BuyRefused::Other(e.to_string())
                }
            })?;
        Ok(match result {
            DirectPurchaseResult::TokenBalance(_, balance) => Bought::Balance(balance),
            DirectPurchaseResult::HistoricalDocument(doc)
            | DirectPurchaseResult::GroupActionWithDocument(_, Some(doc)) => Bought::Document {
                id: doc.id().to_string(Encoding::Base58),
                amount: doc
                    .properties()
                    .get("tokenAmount")
                    .and_then(|v| v.to_integer::<u64>().ok())
                    .unwrap_or(count),
            },
            DirectPurchaseResult::GroupActionWithDocument(_, None) => Bought::Balance(0),
        })
    }
}

/// A `claim` document's fields; `None` for one without them.
fn claim_doc(doc: &Document) -> Option<ClaimDoc> {
    let props = doc.properties();
    Some(ClaimDoc {
        id: doc.id().to_string(Encoding::Base58),
        amount: props.get("amount")?.to_integer::<u64>().ok()?,
        kind: props.get("distributionType")?.to_integer::<u8>().ok()?,
        at_ms: doc.created_at()?,
    })
}
