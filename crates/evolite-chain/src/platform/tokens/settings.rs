//! A token's settings as its issuer may change them (FEAT-0019 T-3.8, AC-T14, the architect's
//! Q4 table): which fields, under which rule, who may change the value and who may change who
//! may, and the fields locked for good («nobody» chosen at creation). Never offered: the
//! perpetual distribution, the main control group, the pool's minimum notes (АРХ-11).

use std::collections::BTreeMap;

use dpp::data_contract::GroupContractPosition;
use dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
use dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dpp::data_contract::associated_token::token_marketplace_rules::accessors::v0::TokenMarketplaceRulesV0Getters;
use dpp::data_contract::change_control_rules::ChangeControlRules;
use dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dpp::data_contract::group::Group;
use dpp::group::action_taker::{ActionGoal, ActionTaker};
use dpp::prelude::Identifier;

use super::rights::{Action, Right, right};

/// A field of the settings window (M7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Setting {
    /// Names and localizations; the decimals stay as they are.
    Names,
    MaxSupply,
    /// Where new tokens go when the minter does not choose.
    NewTokensTo,
    /// Whether the minter chooses where new tokens go.
    ChooseDestination,
    TradeMode,
    /// Who takes an issuer action (mint, burn, freeze, unfreeze, destroy frozen, pause).
    Who(Action),
}

impl Setting {
    pub const ALL: [Setting; 11] = [
        Setting::Names,
        Setting::MaxSupply,
        Setting::NewTokensTo,
        Setting::ChooseDestination,
        Setting::TradeMode,
        Setting::Who(Action::Mint),
        Setting::Who(Action::Burn),
        Setting::Who(Action::Freeze),
        Setting::Who(Action::Unfreeze),
        Setting::Who(Action::DestroyFrozen),
        Setting::Who(Action::Pause),
    ];

    /// The rule that guards it.
    pub fn rules(self, config: &TokenConfiguration) -> &ChangeControlRules {
        match self {
            Setting::Names => config.conventions_change_rules(),
            Setting::MaxSupply => config.max_supply_change_rules(),
            Setting::NewTokensTo => config
                .distribution_rules()
                .new_tokens_destination_identity_rules(),
            Setting::ChooseDestination => config
                .distribution_rules()
                .minting_allow_choosing_destination_rules(),
            Setting::TradeMode => marketplace(config).trade_mode_change_rules(),
            Setting::Who(action) => action.rules(config),
        }
    }
}

/// The token's names, as its conventions keep them: (language, one, many, capitalize).
pub fn names(config: &TokenConfiguration) -> Vec<(String, String, String, bool)> {
    use dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
    use dpp::data_contract::associated_token::token_configuration_localization::accessors::v0::TokenConfigurationLocalizationV0Getters;
    config
        .conventions()
        .localizations()
        .iter()
        .map(|(lang, l)| {
            (
                lang.clone(),
                l.singular_form().to_owned(),
                l.plural_form().to_owned(),
                l.should_capitalize(),
            )
        })
        .collect()
}

/// `names` as the conventions an update writes, with the token's `decimals` as they are.
pub fn conventions(
    names: &[(String, String, String, bool)],
    decimals: u8,
) -> dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention{
    use dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
    use dpp::data_contract::associated_token::token_configuration_convention::v0::TokenConfigurationConventionV0;
    use dpp::data_contract::associated_token::token_configuration_localization::TokenConfigurationLocalization;
    use dpp::data_contract::associated_token::token_configuration_localization::v0::TokenConfigurationLocalizationV0;
    TokenConfigurationConvention::V0(TokenConfigurationConventionV0 {
        localizations: names
            .iter()
            .map(|(lang, one, many, capitalize)| {
                (
                    lang.clone(),
                    TokenConfigurationLocalization::V0(TokenConfigurationLocalizationV0 {
                        should_capitalize: *capitalize,
                        singular_form: one.clone(),
                        plural_form: many.clone(),
                    }),
                )
            })
            .collect(),
        decimals,
    })
}

/// Whether the token may be traded: its marketplace trade mode.
pub fn tradeable(config: &TokenConfiguration) -> bool {
    use dpp::data_contract::associated_token::token_marketplace_rules::v0::TokenTradeMode;
    matches!(marketplace(config).trade_mode(), TokenTradeMode::Tradeable)
}

/// The token's marketplace rules (no getter on the configuration itself).
fn marketplace(
    config: &TokenConfiguration,
) -> &dpp::data_contract::associated_token::token_marketplace_rules::TokenMarketplaceRules {
    match config {
        TokenConfiguration::V0(c) => &c.marketplace_rules,
        TokenConfiguration::V1(c) => &c.base.marketplace_rules,
    }
}

/// What the shown identity may do with a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingRights {
    /// Change its value (for [`Setting::Who`], the value is who takes the action: `who` rules it).
    pub value: Right,
    /// Change who may change it: the rule's admins.
    pub who: Right,
    /// Locked for good: «nobody» may change it, nor who may.
    pub never: bool,
}

/// Every field's [`SettingRights`] for `me`.
pub fn settings_rights(
    config: &TokenConfiguration,
    owner: &Identifier,
    groups: &BTreeMap<GroupContractPosition, Group>,
    me: Identifier,
) -> BTreeMap<Setting, SettingRights> {
    let main = config.main_control_group();
    Setting::ALL
        .iter()
        .map(|s| {
            let rules = s.rules(config);
            let (makers, admins) = (
                rules.authorized_to_make_change_action_takers(),
                rules.admin_action_takers(),
            );
            let who = right(admins, owner, main, groups, me);
            let value = match s {
                Setting::Who(_) => who,
                _ => right(makers, owner, main, groups, me),
            };
            let never = *makers == AuthorizedActionTakers::NoOne
                && *admins == AuthorizedActionTakers::NoOne;
            (*s, SettingRights { value, who, never })
        })
        .collect()
}

/// Whether Platform takes `item` from `me` alone now: the check right before signing, against a
/// fresh contract (T-3.1, R-24).
pub fn may_apply(
    config: &TokenConfiguration,
    item: &TokenConfigurationChangeItem,
    owner: &Identifier,
    groups: &BTreeMap<GroupContractPosition, Group>,
    me: Identifier,
) -> bool {
    config.can_apply_token_configuration_item(
        item,
        owner,
        config.main_control_group(),
        groups,
        &ActionTaker::SingleIdentity(me),
        ActionGoal::ActionCompletion,
    )
}

/// Whether `item` hands a rule to «nobody»: for good, so the window asks for the tick (AC-T14).
pub fn to_no_one(item: &TokenConfigurationChangeItem) -> bool {
    use TokenConfigurationChangeItem as I;
    matches!(
        item,
        I::ConventionsControlGroup(AuthorizedActionTakers::NoOne)
            | I::ConventionsAdminGroup(AuthorizedActionTakers::NoOne)
            | I::MaxSupplyControlGroup(AuthorizedActionTakers::NoOne)
            | I::MaxSupplyAdminGroup(AuthorizedActionTakers::NoOne)
            | I::NewTokensDestinationIdentityControlGroup(AuthorizedActionTakers::NoOne)
            | I::NewTokensDestinationIdentityAdminGroup(AuthorizedActionTakers::NoOne)
            | I::MintingAllowChoosingDestinationControlGroup(AuthorizedActionTakers::NoOne)
            | I::MintingAllowChoosingDestinationAdminGroup(AuthorizedActionTakers::NoOne)
            | I::ManualMinting(AuthorizedActionTakers::NoOne)
            | I::ManualMintingAdminGroup(AuthorizedActionTakers::NoOne)
            | I::ManualBurning(AuthorizedActionTakers::NoOne)
            | I::ManualBurningAdminGroup(AuthorizedActionTakers::NoOne)
            | I::Freeze(AuthorizedActionTakers::NoOne)
            | I::FreezeAdminGroup(AuthorizedActionTakers::NoOne)
            | I::Unfreeze(AuthorizedActionTakers::NoOne)
            | I::UnfreezeAdminGroup(AuthorizedActionTakers::NoOne)
            | I::DestroyFrozenFunds(AuthorizedActionTakers::NoOne)
            | I::DestroyFrozenFundsAdminGroup(AuthorizedActionTakers::NoOne)
            | I::EmergencyAction(AuthorizedActionTakers::NoOne)
            | I::EmergencyActionAdminGroup(AuthorizedActionTakers::NoOne)
            | I::MarketplaceTradeModeControlGroup(AuthorizedActionTakers::NoOne)
            | I::MarketplaceTradeModeAdminGroup(AuthorizedActionTakers::NoOne)
    )
}

/// Whether an item is one the window offers (АРХ-11): the perpetual distribution, the main group
/// and the pool's minimum notes are never offered.
pub fn offered(item: &TokenConfigurationChangeItem) -> bool {
    use TokenConfigurationChangeItem as I;
    !matches!(
        item,
        I::TokenConfigurationNoChange
            | I::PerpetualDistribution(_)
            | I::PerpetualDistributionControlGroup(_)
            | I::PerpetualDistributionAdminGroup(_)
            | I::MainControlGroup(_)
            | I::MinimumPoolNotesForOutgoing(_)
            | I::MinimumPoolNotesForOutgoingControlGroup(_)
            | I::MinimumPoolNotesForOutgoingAdminGroup(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Setters;
    use dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dpp::data_contract::change_control_rules::v0::ChangeControlRulesV0;

    fn rules(makers: AuthorizedActionTakers, admins: AuthorizedActionTakers) -> ChangeControlRules {
        ChangeControlRules::V0(ChangeControlRulesV0 {
            authorized_to_make_change: makers,
            admin_action_takers: admins,
            changing_authorized_action_takers_to_no_one_allowed: true,
            changing_admin_action_takers_to_no_one_allowed: true,
            self_changing_admin_action_takers_allowed: true,
        })
    }

    /// AC-T14: who may change each field, the lock for «nobody», on the owner's token.
    #[test]
    fn each_field_says_who_may_change_it() {
        let owner = Identifier::new([1; 32]);
        let mut config = TokenConfiguration::V0(TokenConfigurationV0::default_most_restrictive());
        config.set_max_supply_change_rules(rules(
            AuthorizedActionTakers::ContractOwner,
            AuthorizedActionTakers::NoOne,
        ));
        config.set_manual_minting_rules(rules(
            AuthorizedActionTakers::ContractOwner,
            AuthorizedActionTakers::ContractOwner,
        ));
        let all = settings_rights(&config, &owner, &BTreeMap::new(), owner);
        let max = all[&Setting::MaxSupply];
        assert_eq!(
            (max.value, max.who, max.never),
            (Right::Alone, Right::No, false)
        );
        let mint = all[&Setting::Who(Action::Mint)];
        assert_eq!((mint.value, mint.who), (Right::Alone, Right::Alone));
        // The most restrictive defaults: nobody, locked.
        assert!(all[&Setting::Names].never);
        assert_eq!(all[&Setting::Names].value, Right::No);
        // Another identity: nothing.
        let other = settings_rights(&config, &owner, &BTreeMap::new(), Identifier::new([2; 32]));
        assert!(
            other
                .values()
                .all(|r| r.value == Right::No && r.who == Right::No)
        );
    }

    /// AC-T14: the check before signing is Platform's; «nobody» asks for the tick; the items
    /// never offered are not.
    #[test]
    fn a_change_is_checked_as_platform_checks_it() {
        let owner = Identifier::new([1; 32]);
        let mut config = TokenConfiguration::V0(TokenConfigurationV0::default_most_restrictive());
        config.set_max_supply_change_rules(rules(
            AuthorizedActionTakers::ContractOwner,
            AuthorizedActionTakers::NoOne,
        ));
        let groups = BTreeMap::new();
        let raise = TokenConfigurationChangeItem::MaxSupply(Some(1_000));
        assert!(may_apply(&config, &raise, &owner, &groups, owner));
        assert!(!may_apply(
            &config,
            &raise,
            &owner,
            &groups,
            Identifier::new([2; 32])
        ));
        let hand_over = TokenConfigurationChangeItem::MaxSupplyControlGroup(
            AuthorizedActionTakers::Identity(Identifier::new([2; 32])),
        );
        assert!(
            !may_apply(&config, &hand_over, &owner, &groups, owner),
            "admins: nobody"
        );
        assert!(to_no_one(&TokenConfigurationChangeItem::ManualMinting(
            AuthorizedActionTakers::NoOne
        )));
        assert!(!to_no_one(&raise));
        assert!(!offered(&TokenConfigurationChangeItem::MainControlGroup(
            None
        )));
        assert!(offered(&raise));
    }
}
