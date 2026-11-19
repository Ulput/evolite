//! The token page and the issuer actions (FEAT-0019 T-3): what the shown identity may do with a
//! token, read with proofs, and the actions themselves, past the password gate. Rights are asked
//! again of a fresh contract right before signing (T-3.1, R-24): a rule changed meanwhile refuses
//! with nothing signed.

use std::collections::BTreeMap;

use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::accessors::v1::DataContractV1Getters;
use dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
use dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dpp::data_contract::associated_token::token_keeps_history_rules::accessors::v0::TokenKeepsHistoryRulesV0Getters;
use dpp::data_contract::change_control_rules::ChangeControlRules;
use dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use evolite_api::tokens::{
    IssuerAction, IssuerOp, Right, SettingChange, SettingField, SettingView, Taker, TokenPage,
};
use evolite_chain::tokens::rights::{self as chain_rights, Action};
use evolite_chain::tokens::settings::{self as chain_settings, Setting};

use super::identity::own_line;
use super::tokens::{make_due, token_label};
use crate::real::*;

fn action_of(a: Action) -> IssuerAction {
    match a {
        Action::Mint => IssuerAction::Mint,
        Action::Burn => IssuerAction::Burn,
        Action::Freeze => IssuerAction::Freeze,
        Action::Unfreeze => IssuerAction::Unfreeze,
        Action::DestroyFrozen => IssuerAction::DestroyFrozen,
        Action::Pause => IssuerAction::Pause,
        Action::Price => IssuerAction::Price,
    }
}

fn chain_action(a: IssuerAction) -> Action {
    match a {
        IssuerAction::Mint => Action::Mint,
        IssuerAction::Burn => Action::Burn,
        IssuerAction::Freeze => Action::Freeze,
        IssuerAction::Unfreeze => Action::Unfreeze,
        IssuerAction::DestroyFrozen => Action::DestroyFrozen,
        IssuerAction::Pause => Action::Pause,
        IssuerAction::Price => Action::Price,
    }
}

fn right_of(r: chain_rights::Right) -> Right {
    match r {
        chain_rights::Right::Alone => Right::Alone,
        chain_rights::Right::Group { needed, of } => Right::Group { needed, of },
        chain_rights::Right::No => Right::No,
    }
}

fn taker_of(t: &AuthorizedActionTakers) -> Taker {
    match t {
        AuthorizedActionTakers::NoOne => Taker::NoOne,
        AuthorizedActionTakers::ContractOwner => Taker::ContractOwner,
        AuthorizedActionTakers::Identity(id) => Taker::Identity(id.to_string(Encoding::Base58)),
        AuthorizedActionTakers::MainGroup => Taker::MainGroup,
        AuthorizedActionTakers::Group(p) => Taker::Group(*p),
    }
}

fn chain_taker(t: &Taker) -> Result<AuthorizedActionTakers, CoreError> {
    Ok(match t {
        Taker::NoOne => AuthorizedActionTakers::NoOne,
        Taker::ContractOwner => AuthorizedActionTakers::ContractOwner,
        Taker::Identity(id) => AuthorizedActionTakers::Identity(
            Identifier::from_string(id, Encoding::Base58).map_err(|_| CoreError::BadAddress)?,
        ),
        Taker::MainGroup => AuthorizedActionTakers::MainGroup,
        Taker::Group(p) => AuthorizedActionTakers::Group(*p),
    })
}

fn field_of(s: Setting) -> SettingField {
    match s {
        Setting::Names => SettingField::Names,
        Setting::MaxSupply => SettingField::MaxSupply,
        Setting::NewTokensTo => SettingField::NewTokensTo,
        Setting::ChooseDestination => SettingField::ChooseDestination,
        Setting::TradeMode => SettingField::TradeMode,
        Setting::Who(a) => SettingField::Who(action_of(a)),
    }
}

/// Whether the rule lets its makers be handed to «nobody».
fn to_no_one_allowed(rules: &ChangeControlRules) -> bool {
    match rules {
        ChangeControlRules::V0(v0) => v0.changing_authorized_action_takers_to_no_one_allowed,
    }
}

/// What was read of a token for its page, besides its contract.
pub(in crate::real) struct PageFacts {
    pub supply: u64,
    pub paused: bool,
    pub frozen: Vec<String>,
    pub read_ms: u64,
}

/// The token page of `config` for `me` (pure, from what was read).
pub(in crate::real) fn page_of(
    token_id: &str,
    config: &TokenConfiguration,
    owner: &Identifier,
    groups: &BTreeMap<u16, dpp::data_contract::group::Group>,
    me: Identifier,
    read: PageFacts,
) -> TokenPage {
    let PageFacts {
        supply,
        paused,
        frozen,
        read_ms,
    } = read;
    let rights = chain_rights::rights(config, owner, groups, me);
    let actions = rights
        .iter()
        .filter(|(_, r)| **r != chain_rights::Right::No)
        .map(|(a, r)| (action_of(*a), right_of(*r)))
        .collect();
    let settings = chain_settings::settings_rights(config, owner, groups, me)
        .into_iter()
        .map(|(s, r)| {
            let rules = s.rules(config);
            SettingView {
                field: field_of(s),
                makers: taker_of(rules.authorized_to_make_change_action_takers()),
                admins: taker_of(rules.admin_action_takers()),
                change: right_of(r.value),
                change_who: right_of(r.who),
                never: r.never,
                to_no_one_allowed: to_no_one_allowed(rules),
            }
        })
        .collect();
    let distribution = config.distribution_rules();
    TokenPage {
        token_id: token_id.to_owned(),
        supply,
        max_supply: config.max_supply(),
        owner: owner.to_string(Encoding::Base58),
        owner_is_me: *owner == me,
        actions,
        paused,
        choose_destination: distribution.minting_allow_choosing_destination(),
        new_tokens_to: distribution
            .new_tokens_destination_identity()
            .map(|id| id.to_string(Encoding::Base58)),
        tradeable: chain_settings::tradeable(config),
        names: chain_settings::names(config)
            .into_iter()
            .map(
                |(lang, one, many, capitalize)| evolite_api::tokens::TokenName {
                    lang,
                    one,
                    many,
                    capitalize,
                },
            )
            .collect(),
        frozen,
        settings,
        read_ms,
    }
}

/// A settings change as Platform takes it (T-3.8); new names with the token's `decimals` as they
/// are (АРХ-11), refused before signing where Platform would refuse them.
pub(in crate::real) fn change_item(
    change: &SettingChange,
    decimals: u8,
) -> Result<TokenConfigurationChangeItem, CoreError> {
    use TokenConfigurationChangeItem as I;
    let id =
        |s: &str| Identifier::from_string(s, Encoding::Base58).map_err(|_| CoreError::BadAddress);
    Ok(match change {
        SettingChange::MaxSupply(max) => I::MaxSupply(*max),
        SettingChange::NewTokensTo(to) => I::NewTokensDestinationIdentity(match to {
            Some(to) => Some(id(to)?),
            None => None,
        }),
        SettingChange::ChooseDestination(on) => I::MintingAllowChoosingDestination(*on),
        SettingChange::Names(names) => {
            if evolite_api::tokens::names_problem(names).is_some() {
                return Err(CoreError::NotAvailable(
                    tr!("Platform would not take these names: check them and sign again").into(),
                ));
            }
            let names: Vec<_> = names
                .iter()
                .map(|n| (n.lang.clone(), n.one.clone(), n.many.clone(), n.capitalize))
                .collect();
            I::Conventions(chain_settings::conventions(&names, decimals))
        }
        SettingChange::TradeMode(on) => {
            use dpp::data_contract::associated_token::token_marketplace_rules::v0::TokenTradeMode;
            I::MarketplaceTradeMode(if *on {
                TokenTradeMode::Tradeable
            } else {
                TokenTradeMode::NotTradeable
            })
        }
        SettingChange::Makers(field, who) => {
            let who = chain_taker(who)?;
            match field {
                SettingField::Names => I::ConventionsControlGroup(who),
                SettingField::MaxSupply => I::MaxSupplyControlGroup(who),
                SettingField::NewTokensTo => I::NewTokensDestinationIdentityControlGroup(who),
                SettingField::ChooseDestination => {
                    I::MintingAllowChoosingDestinationControlGroup(who)
                }
                SettingField::TradeMode => I::MarketplaceTradeModeControlGroup(who),
                SettingField::Who(IssuerAction::Mint) => I::ManualMinting(who),
                SettingField::Who(IssuerAction::Burn) => I::ManualBurning(who),
                SettingField::Who(IssuerAction::Freeze) => I::Freeze(who),
                SettingField::Who(IssuerAction::Unfreeze) => I::Unfreeze(who),
                SettingField::Who(IssuerAction::DestroyFrozen) => I::DestroyFrozenFunds(who),
                SettingField::Who(IssuerAction::Pause) => I::EmergencyAction(who),
                SettingField::Who(IssuerAction::Price) => {
                    return Err(CoreError::NotAvailable(
                        tr!("This rule is not changed here").into(),
                    ));
                }
            }
        }
        SettingChange::Admins(field, who) => {
            let who = chain_taker(who)?;
            match field {
                SettingField::Names => I::ConventionsAdminGroup(who),
                SettingField::MaxSupply => I::MaxSupplyAdminGroup(who),
                SettingField::NewTokensTo => I::NewTokensDestinationIdentityAdminGroup(who),
                SettingField::ChooseDestination => {
                    I::MintingAllowChoosingDestinationAdminGroup(who)
                }
                SettingField::TradeMode => I::MarketplaceTradeModeAdminGroup(who),
                SettingField::Who(IssuerAction::Mint) => I::ManualMintingAdminGroup(who),
                SettingField::Who(IssuerAction::Burn) => I::ManualBurningAdminGroup(who),
                SettingField::Who(IssuerAction::Freeze) => I::FreezeAdminGroup(who),
                SettingField::Who(IssuerAction::Unfreeze) => I::UnfreezeAdminGroup(who),
                SettingField::Who(IssuerAction::DestroyFrozen) => {
                    I::DestroyFrozenFundsAdminGroup(who)
                }
                SettingField::Who(IssuerAction::Pause) => I::EmergencyActionAdminGroup(who),
                SettingField::Who(IssuerAction::Price) => {
                    return Err(CoreError::NotAvailable(
                        tr!("This rule is not changed here").into(),
                    ));
                }
            }
        }
    })
}

/// Whether `me` may take `op` on the fresh `config` now, as Platform would (T-3.1, R-24).
pub(in crate::real) fn may_take(
    op: &IssuerOp,
    config: &TokenConfiguration,
    owner: &Identifier,
    groups: &BTreeMap<u16, dpp::data_contract::group::Group>,
    me: Identifier,
) -> Result<bool, CoreError> {
    Ok(match op {
        IssuerOp::Setting(change) => {
            let item = change_item(change, config.conventions().decimals())?;
            chain_settings::offered(&item)
                && chain_settings::may_apply(config, &item, owner, groups, me)
        }
        other => {
            let action = chain_action(other.action().expect("an action"));
            chain_rights::rights(config, owner, groups, me).get(&action)
                == Some(&chain_rights::Right::Alone)
        }
    })
}

/// Where a mint goes, and whether it fits, on the contract and the supply as they are now (the
/// architect's Must 1 and Must 2). Platform mints to the identity named in the mint, else to the
/// rule's `NewTokensDestinationIdentity`, never to the signer by itself; a mint over the supply
/// limit it refuses with a fee. Both are refused here before anything is signed.
pub(in crate::real) fn mint_to(
    config: &TokenConfiguration,
    to: Option<String>,
    amount: u64,
    supply: u64,
    decimals: u8,
) -> Result<Option<String>, CoreError> {
    let distribution = config.distribution_rules();
    let to = match (
        to,
        distribution.minting_allow_choosing_destination(),
        distribution.new_tokens_destination_identity(),
    ) {
        (Some(to), true, _) => Some(to),
        (Some(_), false, _) => {
            return Err(CoreError::NotAvailable(
                tr!("The token's rules changed: this identity may not do this now").into(),
            ));
        }
        (None, _, Some(_)) => None,
        (None, _, None) => {
            return Err(CoreError::NotAvailable(
                tr!("Minting is impossible: the token’s rule does not say whom new tokens go to.")
                    .into(),
            ));
        }
    };
    if let Some(max) = config.max_supply()
        && supply.saturating_add(amount) > max
    {
        return Err(CoreError::NotAvailable(format!(
            tr!("The supply limit allows {} more."),
            evolite_api::tokens::amount_text(max.saturating_sub(supply), decimals)
        )));
    }
    Ok(to)
}

/// The kind of History line an action makes, as the journal keeps it.
fn kind_of(op: &IssuerOp, label: String, decimals: u8, who: Option<String>) -> journal::Kind {
    let amount = |n: u64| Some(evolite_api::tokens::amount_text(n, decimals));
    let (act, amount, units) = match op {
        IssuerOp::Mint { amount: n, .. } => (journal::IssuerKind::Minted, amount(*n), Some(*n)),
        IssuerOp::Burn { amount: n } => (journal::IssuerKind::Burned, amount(*n), Some(*n)),
        IssuerOp::Freeze { .. } => (journal::IssuerKind::Frozen, None, None),
        IssuerOp::Unfreeze { .. } => (journal::IssuerKind::Unfrozen, None, None),
        IssuerOp::DestroyFrozen { .. } => (journal::IssuerKind::Destroyed, None, None),
        IssuerOp::Pause => (journal::IssuerKind::Paused, None, None),
        IssuerOp::Resume => (journal::IssuerKind::Resumed, None, None),
        IssuerOp::Price(_) => (journal::IssuerKind::PriceChanged, None, None),
        IssuerOp::Setting(_) => (journal::IssuerKind::SettingsChanged, None, None),
    };
    journal::Kind::TokenIssuer {
        token: label,
        act,
        amount,
        units,
        who,
    }
}

/// The page's failed reads after one more (BUG-222): counted on for the same identity and
/// token, from 1 for another.
fn next_failure(
    before: Option<(String, String, u32)>,
    me: String,
    token_id: String,
) -> (String, String, u32) {
    let times = match before {
        Some((who, t, n)) if who == me && t == token_id => n + 1,
        _ => 1,
    };
    (me, token_id, times)
}

impl Real {
    /// [`Command::ReadTokenPage`]: the page read in the background, the snapshots answer.
    pub(in crate::real) fn read_token_page(&self, token_id: String, check: Option<String>) {
        let Some(file) = self.tokens.file.clone() else {
            return;
        };
        let Some(token) = file.get(&token_id) else {
            return;
        };
        let Some(me) = self.tokens.shown_identity() else {
            return;
        };
        let (chain, page, failed, journal) = (
            self.chain.clone(),
            self.tokens.page_handle(),
            self.tokens.page_failed_handle(),
            self.journal.clone(),
        );
        let refresh = self.refresher();
        let working = self.work(PlatformWork::Tokens);
        tokio::spawn(async move {
            let _working = working;
            // Which read failed goes to the journal (BUG-222): the page reads up to six things.
            let at = |step: &'static str| move |e: String| (step, e);
            let read = async {
                let contract = chain
                    .fresh_contract(&token.contract_id)
                    .await
                    .map_err(at("token_page.contract"))?;
                let config = contract
                    .tokens()
                    .get(&token.position)
                    .cloned()
                    .ok_or_else(|| ("token_page.contract", "no token at the position".into()))?;
                let me_id = Identifier::from_string(&me, Encoding::Base58)
                    .map_err(|e| ("token_page", e.to_string()))?;
                let supply = chain
                    .token_supply(&token_id)
                    .await
                    .map_err(at("token_page.supply"))?;
                let facts = chain
                    .token_facts(&me, std::slice::from_ref(&token_id))
                    .await
                    .map_err(at("token_page.facts"))?;
                let paused = facts.paused.get(&token_id).copied().unwrap_or(false);
                // Q2: the frozen among those the wallet knows of.
                let mut candidates = frozen_in_journal(&journal.entries(), &token_id);
                if config.keeps_history().keeps_freezing_history() {
                    candidates.extend(
                        chain
                            .freeze_targets(&token_id)
                            .await
                            .map_err(at("token_page.freezes"))?,
                    );
                }
                if let Some(check) = check.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                    // BUG-233: by the label, not «@name»; an id is checked.
                    let id = super::names::recipient_id(&chain, check)
                        .await
                        .map_err(|e| ("token_page.name", format!("{e:?}")))?;
                    candidates.push(id);
                }
                candidates.sort();
                candidates.dedup();
                let frozen = chain
                    .frozen_among(&token_id, &candidates)
                    .await
                    .map_err(at("token_page.frozen"))?;
                let read_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as u64);
                Ok::<_, (&'static str, String)>(page_of(
                    &token_id,
                    &config,
                    &contract.owner_id(),
                    contract.groups(),
                    me_id,
                    PageFacts {
                        supply,
                        paused,
                        frozen,
                        read_ms,
                    },
                ))
            };
            let started = std::time::Instant::now();
            let read = read.await;
            crate::diag::record(crate::diag::Record::TokenPage {
                ms: started.elapsed().as_millis() as u64,
            });
            let mut failed = failed.lock().unwrap_or_else(|e| e.into_inner());
            match read {
                Ok(read) => {
                    if failed.as_ref().is_some_and(|(_, t, _)| *t == token_id) {
                        *failed = None;
                    }
                    *page.lock().unwrap_or_else(|e| e.into_inner()) = Some((me, read));
                }
                Err((step, e)) => {
                    crate::diag::platform_failed(step, &e);
                    // The page says so and tries again (BUG-222), not «Reading…» for ever.
                    *failed = Some(next_failure(failed.take(), me, token_id));
                }
            }
            drop(failed);
            refresh();
        });
    }

    /// Spend::IssuerAction (T-3): checked again on a fresh contract, then signed. The History
    /// line takes its key from the action's document when Platform wrote one (Q5).
    pub(in crate::real) fn issuer_action(&self, token_id: String, op: IssuerOp) -> Event {
        let refused = |e| Event::Done {
            what: DoneWhat::IssuerAction,
            result: Err(e),
        };
        let Some(file) = self.tokens.file.clone() else {
            return refused(CoreError::NotAvailable(
                tr!("Tokens are not on this network").into(),
            ));
        };
        let Some(token) = file.get(&token_id) else {
            return refused(CoreError::Failed("the token is not in the list".into()));
        };
        let Some(info) = self.tokens.infos_of(&file).get(&token_id).cloned() else {
            return refused(CoreError::Failed(
                "the token's contract is not read yet".into(),
            ));
        };
        if !self.chain.platform_ready() {
            return refused(CoreError::NotSynced);
        }
        let Some((id, keys)) = self
            .unlocked
            .as_ref()
            .and_then(|open| Some((open.id, open.keys.clone()?)))
        else {
            return refused(CoreError::NotAvailable(tr!("The keys are not open").into()));
        };
        let (chain, facts, journal, page) = (
            self.chain.clone(),
            self.tokens.facts_handle(),
            self.journal.clone(),
            self.tokens.page_handle(),
        );
        let unclear = self.unclear.clone();
        let refresh = self.refresher();
        // BUG-206: the identity the screen shows when the command comes: the task checks, pays
        // and signs as it even if the choice moves while it runs, and refuses if it is gone.
        let pinned = self.shown_now(id);
        self.answer_later("task:issuer_action", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let (me, me_id) =
                    pinned_acting(&wallet, pinned)
                        .await
                        .ok_or(CoreError::NotAvailable(
                            tr!("The wallet has no identity").into(),
                        ))?;
                // T-3.1, R-24: the rights again, on the contract as it is now.
                let contract = chain
                    .fresh_contract(&token.contract_id)
                    .await
                    .map_err(read_failure)?;
                let config = contract
                    .tokens()
                    .get(&token.position)
                    .ok_or(CoreError::Failed("no token at the position".into()))?;
                // BUG-233: by the label, not «@name»; an id is checked before anything is asked.
                let resolve = |to: &str| {
                    let (chain, to) = (chain.clone(), to.to_owned());
                    async move { super::names::recipient_id(&chain, &to).await }
                };
                // The architect's note: an @name in a setting is resolved as in a mint or a
                // freeze, before the rights are asked of the change.
                let op = match op {
                    IssuerOp::Setting(change) => IssuerOp::Setting(match change {
                        SettingChange::NewTokensTo(Some(to)) => {
                            SettingChange::NewTokensTo(Some(resolve(&to).await?))
                        }
                        SettingChange::Makers(field, Taker::Identity(who)) => {
                            SettingChange::Makers(field, Taker::Identity(resolve(&who).await?))
                        }
                        SettingChange::Admins(field, Taker::Identity(who)) => {
                            SettingChange::Admins(field, Taker::Identity(resolve(&who).await?))
                        }
                        other => other,
                    }),
                    other => other,
                };
                if !may_take(&op, config, &contract.owner_id(), contract.groups(), me_id)? {
                    return Err(CoreError::NotAvailable(
                        tr!("The token's rules changed: this identity may not do this now").into(),
                    ));
                }
                let (chain_op, who) = match &op {
                    IssuerOp::Mint { to, amount } => {
                        let to = match to {
                            Some(to) => Some(resolve(to).await?),
                            None => None,
                        };
                        // Must 1: the supply read again, beside the fresh contract.
                        let supply = chain.token_supply(&token_id).await.map_err(read_failure)?;
                        let to = mint_to(config, to, *amount, supply, info.decimals)?;
                        (
                            chain::IssuerOp::Mint {
                                to: to.clone(),
                                amount: *amount,
                            },
                            to,
                        )
                    }
                    IssuerOp::Burn { amount } => (chain::IssuerOp::Burn { amount: *amount }, None),
                    IssuerOp::Freeze { who } => {
                        let who = resolve(who).await?;
                        (chain::IssuerOp::Freeze { who: who.clone() }, Some(who))
                    }
                    IssuerOp::Unfreeze { who } => {
                        let who = resolve(who).await?;
                        (chain::IssuerOp::Unfreeze { who: who.clone() }, Some(who))
                    }
                    IssuerOp::DestroyFrozen { who } => {
                        let who = resolve(who).await?;
                        (
                            chain::IssuerOp::DestroyFrozen { who: who.clone() },
                            Some(who),
                        )
                    }
                    IssuerOp::Pause => (chain::IssuerOp::Pause, None),
                    IssuerOp::Resume => (chain::IssuerOp::Resume, None),
                    IssuerOp::Price(tiers) => (
                        chain::IssuerOp::Price(tiers.as_ref().map(|t| schedule_of(t))),
                        None,
                    ),
                    IssuerOp::Setting(change) => (
                        chain::IssuerOp::Config(change_item(
                            change,
                            config.conventions().decimals(),
                        )?),
                        None,
                    ),
                };
                // Should 1: checked, signed and paid as that one identity, whatever «Act as»
                // shows meanwhile (BUG-13 r1 should 1).
                // BUG-205: an earlier write of this token not known to have gone through first;
                // a mint or a burn is shown by the supply, read before it.
                super::tokens::earlier_settled(
                    &unclear, &journal, &chain, &wallet, &me, me_id, &info,
                )
                .await?;
                // BUG-205 r1 M1: the identity's nonce with the token's contract, proved, before
                // signing: only its own writes of this contract move it.
                let n0 = chain.contract_nonce(&me_id, &token.contract_id).await.ok();
                let shows = match &op {
                    IssuerOp::Mint { amount, .. } => chain
                        .token_supply(&token_id)
                        .await
                        .ok()
                        .map(|s| sent::TokenShows::SupplyAtLeast(s + amount)),
                    IssuerOp::Burn { amount } => chain
                        .token_supply(&token_id)
                        .await
                        .ok()
                        .map(|s| sent::TokenShows::SupplyAtMost(s.saturating_sub(*amount))),
                    _ => None,
                };
                let before = chain.identity_credits(&wallet, &me_id, false).await;
                let kind = kind_of(&op, token_label(&info), info.decimals, who);
                // TECH-after-send-outcome: never cut; without its answer, not known.
                let act = {
                    let (chain, wallet, keys) = (chain.clone(), wallet.clone(), keys.clone());
                    let (contract, position) = (token.contract_id.clone(), token.position);
                    async move {
                        chain
                            .issuer_act(&wallet, &keys, &me_id, &contract, position, chain_op)
                            .await
                    }
                };
                let outcome = sent::run(
                    act,
                    sent::SEND_WAIT,
                    |e: &String| sent::refused_before(e),
                    || {
                        sent::token_state_if(
                            &chain,
                            &me_id,
                            &me,
                            &token.contract_id,
                            &token_id,
                            n0,
                            shows,
                        )
                    },
                    |state| n0.is_some_and(|n0| state.shows(n0, shows)),
                )
                .await;
                make_due(&facts);
                *page.lock().unwrap_or_else(|e| e.into_inner()) = None;
                let done = match outcome.landed() {
                    Ok(sent::Landed::Answer(done)) => done,
                    // The supply shows it: no document to key the line by.
                    Ok(sent::Landed::Read(_)) => chain::IssuerDone::default(),
                    Err(sent::NoAnswer::Refused(e)) => {
                        let token = token_label(&info);
                        sent::refused_line(&chain, &wallet, me_id, before, token, me, &journal)
                            .await;
                        return Err(dashpay_failure(e));
                    }
                    Err(sent::NoAnswer::Unclear(late)) => {
                        let pending = sent::Pending::new(
                            me.clone(),
                            sent::token_watch(&token_id, &token.contract_id, n0, shows),
                            Some(kind),
                            before,
                            Some(me),
                        );
                        return Err(unclear.not_known(
                            pending,
                            late,
                            journal,
                            move |done, p| async move {
                                let mut line =
                                    sent::line_after(chain, wallet, me_id, None, p).await;
                                line.key = done.doc.map(|doc| format!("tokx:{doc}"));
                                line
                            },
                        ));
                    }
                };
                let after = chain.identity_credits(&wallet, &me_id, true).await;
                let mut line = own_line(kind, before, after, Some(me));
                line.key = done.doc.map(|doc| format!("tokx:{doc}"));
                journal.add(line);
                refresh();
                Ok(())
            }
            .await;
            Event::Done {
                what: DoneWhat::IssuerAction,
                result,
            }
        });
        Event::Progress {
            what: "issuer_action",
            fraction: 0.0,
        }
    }
}

/// The price as Platform takes it: one price, or tiers.
fn schedule_of(tiers: &[(u64, u64)]) -> dpp::tokens::token_pricing_schedule::TokenPricingSchedule {
    use dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
    match tiers {
        [(1, price)] => TokenPricingSchedule::SinglePrice(*price),
        tiers => TokenPricingSchedule::SetPrices(tiers.iter().copied().collect()),
    }
}

/// Whom this wallet froze of `token`, from its own lines (Q2): candidates for the frozen list.
fn frozen_in_journal(entries: &[journal::Entry], token: &str) -> Vec<String> {
    let _ = token;
    entries
        .iter()
        .filter_map(|e| match &e.kind {
            Some(journal::Kind::TokenIssuer {
                act: journal::IssuerKind::Frozen,
                who: Some(who),
                ..
            }) => Some(who.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Setters;
    use dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dpp::data_contract::change_control_rules::v0::ChangeControlRulesV0;

    fn rules(makers: AuthorizedActionTakers) -> ChangeControlRules {
        ChangeControlRules::V0(ChangeControlRulesV0 {
            authorized_to_make_change: makers,
            admin_action_takers: AuthorizedActionTakers::ContractOwner,
            changing_authorized_action_takers_to_no_one_allowed: true,
            changing_admin_action_takers_to_no_one_allowed: true,
            self_changing_admin_action_takers_allowed: true,
        })
    }

    fn config() -> TokenConfiguration {
        let mut c = TokenConfiguration::V0(TokenConfigurationV0::default_most_restrictive());
        c.set_manual_minting_rules(rules(AuthorizedActionTakers::ContractOwner));
        c.set_manual_burning_rules(rules(AuthorizedActionTakers::ContractOwner));
        c
    }

    fn facts() -> PageFacts {
        PageFacts {
            supply: 1_000,
            paused: false,
            frozen: vec![],
            read_ms: 0,
        }
    }

    /// AC-T13, AC-D8: the owner's page lists the actions it takes alone, and only those; another
    /// identity's page has no «Management».
    #[test]
    fn the_page_lists_only_what_the_rules_allow() {
        let (owner, other) = (Identifier::new([1; 32]), Identifier::new([2; 32]));
        let groups = BTreeMap::new();
        let page = page_of("T", &config(), &owner, &groups, owner, facts());
        let actions: Vec<_> = page.actions.iter().map(|(a, _)| *a).collect();
        assert_eq!(actions, [IssuerAction::Mint, IssuerAction::Burn]);
        assert!(page.owner_is_me && page.manages());
        let theirs = page_of("T", &config(), &owner, &groups, other, facts());
        assert!(theirs.actions.is_empty());
        assert!(!theirs.manages(), "no «Management» at all");
    }

    /// T-3.1, R-24: right before signing, the fresh contract decides; a settings change is
    /// Platform's own check; a rule never offered is refused.
    #[test]
    fn the_rights_are_asked_again_before_signing() {
        let owner = Identifier::new([1; 32]);
        let groups = BTreeMap::new();
        let c = config();
        let mint = IssuerOp::Mint {
            to: None,
            amount: 5,
        };
        assert!(may_take(&mint, &c, &owner, &groups, owner).unwrap());
        assert!(
            !may_take(&IssuerOp::Pause, &c, &owner, &groups, owner).unwrap(),
            "nobody"
        );
        assert!(!may_take(&mint, &c, &owner, &groups, Identifier::new([2; 32])).unwrap());
        // Who mints, handed to another identity: the rule's admins are the owner.
        let hand_over = IssuerOp::Setting(SettingChange::Makers(
            SettingField::Who(IssuerAction::Mint),
            Taker::Identity(Identifier::new([3; 32]).to_string(Encoding::Base58)),
        ));
        assert!(may_take(&hand_over, &c, &owner, &groups, owner).unwrap());
        // The price rule is not changed here.
        assert!(
            change_item(
                &SettingChange::Makers(SettingField::Who(IssuerAction::Price), Taker::NoOne),
                2
            )
            .is_err()
        );
    }

    /// Names: the conventions with the token's decimals as they are (АРХ-11); names Platform
    /// would refuse are refused before signing.
    #[test]
    fn names_are_the_conventions_with_the_decimals_kept() {
        use dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
        use dpp::data_contract::associated_token::token_configuration_localization::accessors::v0::TokenConfigurationLocalizationV0Getters;
        use evolite_api::tokens::TokenName;
        let name = |lang: &str, one: &str, many: &str| TokenName {
            lang: lang.into(),
            one: one.into(),
            many: many.into(),
            capitalize: true,
        };
        let item = change_item(
            &SettingChange::Names(vec![
                name("en", "Coffee", "Coffees"),
                name("ru", "Кофе", "Кофе"),
            ]),
            2,
        )
        .unwrap();
        let TokenConfigurationChangeItem::Conventions(conv) = item else {
            panic!("{item:?}")
        };
        assert_eq!(conv.decimals(), 2, "the decimals kept");
        let en = conv.localizations().get("en").expect("en");
        assert_eq!(
            (en.singular_form(), en.plural_form()),
            ("Coffee", "Coffees")
        );
        assert!(conv.localizations().contains_key("ru"));
        assert!(
            change_item(&SettingChange::Names(vec![name("ru", "Кофе", "Кофе")]), 2).is_err(),
            "no English"
        );
        assert!(
            change_item(
                &SettingChange::Names(vec![name("en", "Evo Coffee", "Coffees")]),
                2
            )
            .is_err(),
            "a space"
        );
    }

    /// T-3.8: each change is the item Platform takes.
    #[test]
    fn a_change_is_the_item_platform_takes() {
        use TokenConfigurationChangeItem as I;
        assert_eq!(
            change_item(&SettingChange::MaxSupply(Some(9)), 2).unwrap(),
            I::MaxSupply(Some(9))
        );
        assert_eq!(
            change_item(
                &SettingChange::Makers(SettingField::Who(IssuerAction::Pause), Taker::NoOne),
                2
            )
            .unwrap(),
            I::EmergencyAction(AuthorizedActionTakers::NoOne)
        );
        assert_eq!(
            change_item(
                &SettingChange::Admins(SettingField::MaxSupply, Taker::ContractOwner),
                2
            )
            .unwrap(),
            I::MaxSupplyAdminGroup(AuthorizedActionTakers::ContractOwner)
        );
        assert!(SettingChange::Makers(SettingField::Names, Taker::NoOne).to_no_one());
        assert!(IssuerOp::Burn { amount: 1 }.needs_tick());
        assert!(
            !IssuerOp::Mint {
                to: None,
                amount: 1
            }
            .needs_tick()
        );
    }

    /// Must 1, Must 2: a mint goes to the one named, else to the rule's destination; with no
    /// choice and no destination, or over the limit as read now, it is refused before signing.
    #[test]
    fn a_mint_goes_somewhere_and_fits_or_is_refused() {
        use dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Setters;
        let me = "Me111".to_string();
        let with = |choose: bool, dest: Option<Identifier>, max: Option<u64>| {
            let mut c = config();
            c.distribution_rules_mut()
                .set_minting_allow_choosing_destination(choose);
            c.distribution_rules_mut()
                .set_new_tokens_destination_identity(dest);
            c.set_max_supply(max);
            c
        };
        let dest = Some(Identifier::new([7; 32]));
        // The choice allowed: «Yourself» names this identity, and it goes there.
        assert_eq!(
            mint_to(&with(true, None, None), Some(me.clone()), 5, 0, 2).unwrap(),
            Some(me.clone())
        );
        // No choice, a destination: none named, the rule's destination takes it.
        assert_eq!(
            mint_to(&with(false, dest, None), None, 5, 0, 2).unwrap(),
            None
        );
        // No choice, no destination: refused with the line the page shows.
        assert!(matches!(
            mint_to(&with(false, None, None), None, 5, 0, 2),
            Err(CoreError::NotAvailable(why)) if why.starts_with("Minting is impossible")
        ));
        // Named where the rule no longer lets one choose: refused.
        assert!(mint_to(&with(false, dest, None), Some(me.clone()), 5, 0, 2).is_err());
        // The limit 100.00, 90.00 out as read now: 10.00 fits, 20.00 does not.
        let limited = with(true, None, Some(10_000));
        assert!(mint_to(&limited, Some(me.clone()), 1_000, 9_000, 2).is_ok());
        assert!(matches!(
            mint_to(&limited, Some(me), 2_000, 9_000, 2),
            Err(CoreError::NotAvailable(why)) if why.contains("10.00")
        ));
    }

    /// BUG-222: failed page reads are counted in a row for one identity and token, so the page
    /// tells a new failure from one it has seen.
    #[test]
    fn failed_page_reads_are_counted_for_one_token_and_identity() {
        let first = super::next_failure(None, "me".into(), "T".into());
        assert_eq!(first, ("me".into(), "T".into(), 1));
        let second = super::next_failure(Some(first), "me".into(), "T".into());
        assert_eq!(second.2, 2);
        let other = super::next_failure(Some(second.clone()), "me".into(), "U".into());
        assert_eq!(other.2, 1, "another token");
        let someone = super::next_failure(Some(second), "you".into(), "T".into());
        assert_eq!(someone.2, 1, "another identity");
    }
}
