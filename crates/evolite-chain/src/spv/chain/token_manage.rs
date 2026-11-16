//! A token's issuer actions (FEAT-0019 T-3): the reads the token page needs — a fresh contract
//! (rights are checked again right before signing, T-3.1), the supply, the frozen balances among
//! the candidates the wallet knows of (Q2) — and the actions themselves, signed with the
//! identity's CRITICAL authentication key as every token transition is (R-13).

use std::sync::Arc;

use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier};
use dpp::balances::credits::TokenAmount;
use dpp::data_contract::DataContract;
use dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
use dpp::document::DocumentV0Getters;
use dpp::platform_value::Value;
use dpp::platform_value::string_encoding::Encoding;
use dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dpp::tokens::token_pricing_schedule::TokenPricingSchedule;

use super::Chain;

/// An issuer action, as signed (UC-9…UC-14).
#[derive(Clone, Debug, PartialEq)]
pub enum IssuerOp {
    /// `to`: the recipient, when the token lets the minter choose; else Platform's own.
    Mint {
        to: Option<String>,
        amount: TokenAmount,
    },
    Burn {
        amount: TokenAmount,
    },
    Freeze {
        who: String,
    },
    Unfreeze {
        who: String,
    },
    DestroyFrozen {
        who: String,
    },
    Pause,
    Resume,
    /// `None`: not sold any more.
    Price(Option<TokenPricingSchedule>),
    Config(TokenConfigurationChangeItem),
}

/// The kinds of issuer action TokenHistory keeps a document of (T-3.10, the architect's Q5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssuerDocKind {
    Minted,
    Burned,
    Frozen,
    Unfrozen,
    Destroyed,
    Paused,
    Resumed,
    PriceChanged,
    SettingsChanged,
}

impl IssuerDocKind {
    pub const ALL: [IssuerDocKind; 9] = [
        IssuerDocKind::Minted,
        IssuerDocKind::Burned,
        IssuerDocKind::Frozen,
        IssuerDocKind::Unfrozen,
        IssuerDocKind::Destroyed,
        IssuerDocKind::Paused,
        IssuerDocKind::Resumed,
        IssuerDocKind::PriceChanged,
        IssuerDocKind::SettingsChanged,
    ];

    /// The document type, and the `action` of an emergency action (0 pause, 1 resume).
    fn doc_type(self) -> (&'static str, Option<u8>) {
        match self {
            IssuerDocKind::Minted => ("mint", None),
            IssuerDocKind::Burned => ("burn", None),
            IssuerDocKind::Frozen => ("freeze", None),
            IssuerDocKind::Unfrozen => ("unfreeze", None),
            IssuerDocKind::Destroyed => ("destroyFrozenFunds", None),
            IssuerDocKind::Paused => ("emergencyAction", Some(0)),
            IssuerDocKind::Resumed => ("emergencyAction", Some(1)),
            IssuerDocKind::PriceChanged => ("directPricing", None),
            IssuerDocKind::SettingsChanged => ("configUpdate", None),
        }
    }
}

/// One issuer action as its TokenHistory document says it (T-3.10): by whom it was taken is the
/// document's owner; `amount` and `who` (the recipient of a mint, the identity frozen) where the
/// document has them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuerDoc {
    pub id: String,
    pub kind: IssuerDocKind,
    pub amount: Option<TokenAmount>,
    pub who: Option<String>,
    pub at_ms: u64,
}

/// `doc` of `kind` as the wallet keeps it.
fn issuer_doc(doc: &Document, kind: IssuerDocKind) -> Option<IssuerDoc> {
    let props = doc.properties();
    let id_of_field = |name: &str| {
        props
            .get(name)
            .and_then(|v| v.to_identifier().ok())
            .map(|id| id.to_string(Encoding::Base58))
    };
    let amount_of = |name: &str| props.get(name).and_then(|v| v.to_integer::<u64>().ok());
    let (amount, who) = match kind {
        IssuerDocKind::Minted => (amount_of("amount"), id_of_field("recipientId")),
        IssuerDocKind::Burned => (amount_of("amount"), None),
        IssuerDocKind::Frozen | IssuerDocKind::Unfrozen => (None, id_of_field("frozenIdentityId")),
        IssuerDocKind::Destroyed => (
            amount_of("destroyedAmount"),
            id_of_field("frozenIdentityId"),
        ),
        IssuerDocKind::Paused
        | IssuerDocKind::Resumed
        | IssuerDocKind::PriceChanged
        | IssuerDocKind::SettingsChanged => (None, None),
    };
    Some(IssuerDoc {
        id: doc.id().to_string(Encoding::Base58),
        kind,
        amount,
        who,
        at_ms: doc.created_at()?,
    })
}

/// What an action came to: its TokenHistory document when Platform wrote one (the History line's
/// key, Q5), and a balance when the answer carries one.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct IssuerDone {
    pub doc: Option<String>,
    pub balance: Option<TokenAmount>,
}

fn id_of(s: &str) -> Result<Identifier, String> {
    Identifier::from_string(s, Encoding::Base58).map_err(|e| e.to_string())
}

fn doc_id(doc: &Document) -> String {
    doc.id().to_string(Encoding::Base58)
}

/// Freeze documents read at most, in pages of 100.
const FREEZE_PAGES: usize = 5;

impl Chain {
    /// The contract `contract` (base58) as Platform has it now, proved, and handed to the
    /// provider so the action's answer can be checked against it.
    pub async fn fresh_contract(&self, contract: &str) -> Result<Arc<DataContract>, String> {
        self.ready()?;
        let sdk = self.sdk().await?;
        let fetched = DataContract::fetch(&sdk, id_of(contract)?)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("the token's contract is not on Platform")?;
        let fetched = Arc::new(fetched);
        if let Some(provider) = sdk.context_provider() {
            use dash_sdk::platform::ContextProvider as _;
            provider.register_data_contract(fetched.clone());
        }
        Ok(fetched)
    }

    /// How much of `token` is out, proved.
    /// The issuer actions of `kind` that `identity` took on `token` (both base58), from their
    /// TokenHistory documents, proved: the newest 100, oldest first (T-3.10). Each kind by the
    /// index TokenHistory v2 has for it: by the owner (`byOwnerId`), with the emergency action's
    /// `action` and the settings' `changeItemType` in it (the settings descending by both, the
    /// newest of each type); a price only by date, its owner checked here. One query a kind, so a kind refused does not take the others with it.
    pub async fn token_issuer_docs(
        &self,
        token: &str,
        identity: &str,
        kind: IssuerDocKind,
    ) -> Result<Vec<IssuerDoc>, String> {
        let sdk = self.sdk().await?;
        self.ready()?;
        let contract = load_system_data_contract(SystemDataContract::TokenHistory, sdk.version())
            .map(Arc::new)
            .map_err(|e| format!("{e:?}"))?;
        let (token_id, me) = (id_of(token)?, id_of(identity)?);
        let (doc_type, action) = kind.doc_type();
        let equal = |field: &str, value: Value| WhereClause {
            field: field.into(),
            operator: WhereOperator::Equal,
            value,
        };
        let order = |field: &str, ascending: bool| OrderClause {
            field: field.into(),
            ascending,
        };
        let mut query = DocumentQuery::new(contract, doc_type)
            .map_err(|e| format!("{e:?}"))?
            .with_where(equal("tokenId", Value::Identifier(token_id.to_buffer())))
            .with_limit(100);
        let by_owner = kind != IssuerDocKind::PriceChanged;
        if by_owner {
            query = query.with_where(equal("$ownerId", Value::Identifier(me.to_buffer())));
        }
        if let Some(action) = action {
            query = query.with_where(equal("action", Value::U8(action)));
        }
        if kind == IssuerDocKind::SettingsChanged {
            // Both by the index, descending: the newest of each type (review r1, a note).
            query = query
                .with_order_by(order("changeItemType", false))
                .with_order_by(order("$createdAt", false));
        } else {
            query = query.with_order_by(order("$createdAt", false));
        }
        let docs = Document::fetch_many(&sdk, query)
            .await
            .map_err(|e| format!("{e:?}"))?;
        let mut all: Vec<IssuerDoc> = docs
            .into_values()
            .flatten()
            .filter(|d| by_owner || d.owner_id() == me)
            .filter_map(|d| issuer_doc(&d, kind))
            .collect();
        all.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then_with(|| a.id.cmp(&b.id)));
        Ok(all)
    }

    pub async fn token_supply(&self, token: &str) -> Result<TokenAmount, String> {
        use dpp::balances::total_single_token_balance::TotalSingleTokenBalance;
        self.ready()?;
        let sdk = self.sdk().await?;
        Ok(TotalSingleTokenBalance::fetch(&sdk, id_of(token)?)
            .await
            .map_err(|e| e.to_string())?
            .map_or(0, |s| s.token_supply.max(0) as u64))
    }

    /// Which of `candidates` (base58) have their balance of `token` frozen, by a proved read of
    /// each (Q2): only these are listed.
    pub async fn frozen_among(
        &self,
        token: &str,
        candidates: &[String],
    ) -> Result<Vec<String>, String> {
        use dash_sdk::platform::tokens::token_info::IdentitiesTokenInfosQuery;
        use dash_sdk::query_types::token_info::IdentitiesTokenInfos;
        use dpp::tokens::info::IdentityTokenInfo;
        use dpp::tokens::info::v0::IdentityTokenInfoV0Accessors;
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let ids = candidates
            .iter()
            .map(|c| id_of(c))
            .collect::<Result<Vec<_>, _>>()?;
        self.ready()?;
        let sdk = self.sdk().await?;
        let mut frozen = Vec::new();
        for batch in ids.chunks(100) {
            let query = IdentitiesTokenInfosQuery {
                identity_ids: batch.to_vec(),
                token_id: id_of(token)?,
            };
            let infos: IdentitiesTokenInfos = IdentityTokenInfo::fetch_many(&sdk, query)
                .await
                .map_err(|e| e.to_string())?;
            for (who, info) in infos.iter() {
                if info.as_ref().is_some_and(|i| i.frozen()) {
                    frozen.push(who.to_string(Encoding::Base58));
                }
            }
        }
        frozen.sort();
        Ok(frozen)
    }

    /// Whom the `freeze` documents of `token` name (base58, once each): candidates for the
    /// frozen list when the token keeps its freezing history (Q2).
    pub async fn freeze_targets(&self, token: &str) -> Result<Vec<String>, String> {
        let sdk = self.sdk().await?;
        let contract = load_system_data_contract(SystemDataContract::TokenHistory, sdk.version())
            .map(Arc::new)
            .map_err(|e| format!("{e:?}"))?;
        let token_id = id_of(token)?;
        let mut whom: Vec<String> = Vec::new();
        let mut after: Option<u64> = None;
        for _ in 0..FREEZE_PAGES {
            self.ready()?;
            let mut query = DocumentQuery::new(contract.clone(), "freeze")
                .map_err(|e| format!("{e:?}"))?
                .with_where(WhereClause {
                    field: "tokenId".into(),
                    operator: WhereOperator::Equal,
                    value: Value::Identifier(token_id.to_buffer()),
                })
                .with_order_by(OrderClause {
                    field: "$createdAt".into(),
                    ascending: true,
                })
                .with_limit(100);
            if let Some(at) = after {
                query = query.with_where(WhereClause {
                    field: "$createdAt".into(),
                    operator: WhereOperator::GreaterThan,
                    value: Value::U64(at),
                });
            }
            let page: Vec<Document> = Document::fetch_many(&sdk, query)
                .await
                .map_err(|e| format!("{e:?}"))?
                .into_values()
                .flatten()
                .collect();
            let full = page.len() >= 100;
            for doc in &page {
                if let Some(who) = doc
                    .properties()
                    .get("frozenIdentityId")
                    .and_then(|v| v.to_identifier().ok())
                {
                    let who = who.to_string(Encoding::Base58);
                    if !whom.contains(&who) {
                        whom.push(who);
                    }
                }
                after = doc.created_at().max(after);
            }
            if !full {
                break;
            }
        }
        Ok(whom)
    }

    /// Takes `op` on the token at `position` of `contract` as the shown identity. Platform checks
    /// the rights again; the screen checked them on a fresh contract just before.
    pub async fn issuer_act(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        keys: &key_wallet::Wallet,
        me: &Identifier,
        contract: &str,
        position: u16,
        op: IssuerOp,
    ) -> Result<IssuerDone, String> {
        use dash_sdk::platform::tokens::transitions::{
            BurnResult, ConfigUpdateResult, DestroyFrozenFundsResult, EmergencyActionResult,
            FreezeResult, MintResult, SetPriceResult, UnfreezeResult,
        };
        use dpp::identity::accessors::IdentityGettersV0;
        let contract_id = id_of(contract)?;
        self.ready()?;
        // The identity whose rights were checked, not the one shown now (BUG-13 r1 should 1).
        let (me, signer) = Self::identity_as(wallet, keys, me).await?;
        let me = me.id();
        let tokens = wallet.identity();
        let err = |e: platform_wallet::PlatformWalletError| e.to_string();
        let done = |doc: Option<&Document>, balance: Option<TokenAmount>| IssuerDone {
            doc: doc.map(doc_id),
            balance,
        };
        Ok(match op {
            IssuerOp::Mint { to, amount } => {
                let to = to.as_deref().map(id_of).transpose()?;
                match tokens
                    .token_mint_with_external_signer(
                        me,
                        contract_id,
                        position,
                        to,
                        amount,
                        None,
                        None,
                        &signer,
                    )
                    .await
                    .map_err(err)?
                {
                    MintResult::TokenBalance(who, balance) => {
                        done(None, (who == me).then_some(balance))
                    }
                    MintResult::HistoricalDocument(doc) => done(Some(&doc), None),
                    MintResult::GroupActionWithDocument(_, doc) => done(doc.as_ref(), None),
                    MintResult::GroupActionWithBalance(_, _, balance) => done(None, balance),
                }
            }
            IssuerOp::Burn { amount } => match tokens
                .token_burn_with_external_signer(
                    me,
                    contract_id,
                    position,
                    amount,
                    None,
                    None,
                    &signer,
                )
                .await
                .map_err(err)?
            {
                BurnResult::TokenBalance(_, balance) => done(None, Some(balance)),
                BurnResult::HistoricalDocument(doc) => done(Some(&doc), None),
                BurnResult::GroupActionWithDocument(_, doc) => done(doc.as_ref(), None),
                #[allow(unreachable_patterns)]
                _ => done(None, None),
            },
            IssuerOp::Freeze { who } => match tokens
                .token_freeze_with_external_signer(
                    me,
                    contract_id,
                    position,
                    id_of(&who)?,
                    None,
                    None,
                    &signer,
                )
                .await
                .map_err(err)?
            {
                FreezeResult::HistoricalDocument(doc) => done(Some(&doc), None),
                FreezeResult::GroupActionWithDocument(_, doc) => done(doc.as_ref(), None),
                _ => done(None, None),
            },
            IssuerOp::Unfreeze { who } => match tokens
                .token_unfreeze_with_external_signer(
                    me,
                    contract_id,
                    position,
                    id_of(&who)?,
                    None,
                    None,
                    &signer,
                )
                .await
                .map_err(err)?
            {
                UnfreezeResult::HistoricalDocument(doc) => done(Some(&doc), None),
                UnfreezeResult::GroupActionWithDocument(_, doc) => done(doc.as_ref(), None),
                _ => done(None, None),
            },
            IssuerOp::DestroyFrozen { who } => match tokens
                .token_destroy_frozen_funds_with_external_signer(
                    me,
                    contract_id,
                    position,
                    id_of(&who)?,
                    None,
                    None,
                    &signer,
                )
                .await
                .map_err(err)?
            {
                DestroyFrozenFundsResult::HistoricalDocument(doc) => done(Some(&doc), None),
                DestroyFrozenFundsResult::GroupActionWithDocument(_, doc) => {
                    done(doc.as_ref(), None)
                }
            },
            IssuerOp::Pause | IssuerOp::Resume => {
                let result = if op == IssuerOp::Pause {
                    tokens
                        .token_pause_with_external_signer(
                            me,
                            contract_id,
                            position,
                            None,
                            None,
                            &signer,
                        )
                        .await
                } else {
                    tokens
                        .token_resume_with_external_signer(
                            me,
                            contract_id,
                            position,
                            None,
                            None,
                            &signer,
                        )
                        .await
                };
                match result.map_err(err)? {
                    EmergencyActionResult::Document(doc) => done(Some(&doc), None),
                    EmergencyActionResult::GroupActionWithDocument(_, doc) => {
                        done(doc.as_ref(), None)
                    }
                }
            }
            IssuerOp::Price(schedule) => {
                // Tiers need the richer call: the contract and the key as platform-wallet takes
                // them for the simple one.
                let fresh = self.fresh_contract(contract).await?;
                let key = Self::critical_key(wallet, &me, &signer).await?;
                match tokens
                    .token_set_price_with_signer(
                        fresh, position, me, schedule, &key, &signer, None, None, None,
                    )
                    .await
                    .map_err(|e| e.to_string())?
                {
                    SetPriceResult::HistoricalDocument(doc) => done(Some(&doc), None),
                    SetPriceResult::GroupActionWithDocument(_, doc) => done(doc.as_ref(), None),
                    _ => done(None, None),
                }
            }
            IssuerOp::Config(item) => match tokens
                .token_update_config_with_external_signer(
                    me,
                    contract_id,
                    position,
                    item,
                    None,
                    None,
                    &signer,
                )
                .await
                .map_err(err)?
            {
                ConfigUpdateResult::Document(doc)
                | ConfigUpdateResult::GroupActionWithDocument(_, doc) => done(Some(&doc), None),
            },
        })
    }

    /// The identity's AUTHENTICATION · CRITICAL · ECDSA key the signer can sign with, as
    /// platform-wallet picks it for token transitions.
    async fn critical_key<S>(
        wallet: &platform_wallet::PlatformWallet,
        me: &Identifier,
        signer: &S,
    ) -> Result<dpp::identity::IdentityPublicKey, String>
    where
        S: dpp::identity::signer::Signer<dpp::identity::IdentityPublicKey>,
    {
        use dpp::identity::accessors::IdentityGettersV0;
        use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        use dpp::identity::{KeyType, Purpose, SecurityLevel};
        let state = wallet.state().await;
        let identity = state
            .identity_manager
            .identity(me)
            .map(|m| m.identity.clone())
            .ok_or("the identity is not in the wallet")?;
        drop(state);
        // As platform-wallet's own choice (its `AvailableSigningKey`, private): in key-id order,
        // enabled, one the signer holds.
        identity
            .public_keys()
            .values()
            .find(|key| {
                key.purpose() == Purpose::AUTHENTICATION
                    && key.security_level() == SecurityLevel::CRITICAL
                    && key.key_type() == KeyType::ECDSA_SECP256K1
                    && !key.is_disabled()
                    && signer.can_sign_with(key)
            })
            .cloned()
            .ok_or_else(|| "this identity has no key that may sign token actions".into())
    }
}
