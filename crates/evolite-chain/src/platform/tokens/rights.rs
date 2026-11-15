//! What the shown identity may do with a token it manages (FEAT-0019 T-3.1, AC-T13, R-15): each
//! issuer action's rule asked with dpp's own `allowed_for_action_taker`, as Platform asks it. An
//! action the identity completes alone gets its button; one it only takes part in through a
//! group is said as «needs the group's signatures: k of n» (groups are shown only, T-3.9); one
//! it has no part in is not shown at all.

use std::collections::BTreeMap;

use dpp::data_contract::GroupContractPosition;
use dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dpp::data_contract::change_control_rules::ChangeControlRules;
use dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dpp::data_contract::group::Group;
use dpp::data_contract::group::accessors::v0::GroupV0Getters;
use dpp::group::action_taker::{ActionGoal, ActionTaker};
use dpp::prelude::Identifier;

/// An issuer action of the token page (UC-9…UC-14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    Mint,
    Burn,
    Freeze,
    Unfreeze,
    DestroyFrozen,
    /// Pause and resume: the emergency action.
    Pause,
    /// The direct purchase price.
    Price,
}

impl Action {
    pub const ALL: [Action; 7] = [
        Action::Mint,
        Action::Burn,
        Action::Freeze,
        Action::Unfreeze,
        Action::DestroyFrozen,
        Action::Pause,
        Action::Price,
    ];

    /// The rule that says who takes it.
    pub fn rules(self, config: &TokenConfiguration) -> &ChangeControlRules {
        match self {
            Action::Mint => config.manual_minting_rules(),
            Action::Burn => config.manual_burning_rules(),
            Action::Freeze => config.freeze_rules(),
            Action::Unfreeze => config.unfreeze_rules(),
            Action::DestroyFrozen => config.destroy_frozen_funds_rules(),
            Action::Pause => config.emergency_action_rules(),
            Action::Price => config
                .distribution_rules()
                .change_direct_purchase_pricing_rules(),
        }
    }
}

/// What the shown identity may do with one action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Right {
    /// It completes the action alone: the button.
    Alone,
    /// Only with its group: «needs the group's signatures: `needed` of `of`» (power, as the
    /// group counts it), no button.
    Group { needed: u32, of: u32 },
    /// Nothing: not shown.
    No,
}

/// [`Right`] of `me` under `who` (`authorized_to_make_change` of a rule, or its admins).
pub fn right(
    who: &AuthorizedActionTakers,
    owner: &Identifier,
    main_group: Option<GroupContractPosition>,
    groups: &BTreeMap<GroupContractPosition, Group>,
    me: Identifier,
) -> Right {
    let taker = ActionTaker::SingleIdentity(me);
    if who.allowed_for_action_taker(
        owner,
        main_group,
        groups,
        &taker,
        ActionGoal::ActionCompletion,
    ) {
        return Right::Alone;
    }
    if !who.allowed_for_action_taker(
        owner,
        main_group,
        groups,
        &taker,
        ActionGoal::ActionParticipation,
    ) {
        return Right::No;
    }
    let group = match who {
        AuthorizedActionTakers::MainGroup => main_group.and_then(|p| groups.get(&p)),
        AuthorizedActionTakers::Group(p) => groups.get(p),
        _ => None,
    };
    match group {
        Some(group) => Right::Group {
            needed: group.required_power(),
            of: group.members().values().sum(),
        },
        None => Right::No,
    }
}

/// Every action's [`Right`] for `me` on a token of a contract owned by `owner`.
pub fn rights(
    config: &TokenConfiguration,
    owner: &Identifier,
    groups: &BTreeMap<GroupContractPosition, Group>,
    me: Identifier,
) -> BTreeMap<Action, Right> {
    let main_group = config.main_control_group();
    Action::ALL
        .iter()
        .map(|a| {
            let who = a.rules(config).authorized_to_make_change_action_takers();
            (*a, right(who, owner, main_group, groups, me))
        })
        .collect()
}

/// Whether the token page shows «Management» at all (AC-D8): some action is the identity's,
/// alone or with a group.
pub fn manages(rights: &BTreeMap<Action, Right>) -> bool {
    rights.values().any(|r| *r != Right::No)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpp::data_contract::group::v0::GroupV0;

    fn id(n: u8) -> Identifier {
        Identifier::new([n; 32])
    }

    fn group(members: &[(u8, u32)], required: u32) -> Group {
        Group::V0(GroupV0 {
            members: members.iter().map(|(n, p)| (id(*n), *p)).collect(),
            required_power: required,
        })
    }

    /// AC-T13: the owner, a named identity, a group alone or not, and nobody, on sets of
    /// `AuthorizedActionTakers`.
    #[test]
    fn a_rule_gives_a_button_a_group_line_or_nothing() {
        let (owner, me, other) = (id(1), id(2), id(3));
        let groups: BTreeMap<_, _> = [
            (0, group(&[(2, 1), (3, 1), (4, 1)], 2)),
            (1, group(&[(2, 3), (3, 1)], 3)),
        ]
        .into_iter()
        .collect();
        let ask = |who: AuthorizedActionTakers, as_: Identifier, main: Option<u16>| {
            right(&who, &owner, main, &groups, as_)
        };
        assert_eq!(
            ask(AuthorizedActionTakers::ContractOwner, owner, None),
            Right::Alone
        );
        assert_eq!(
            ask(AuthorizedActionTakers::ContractOwner, me, None),
            Right::No
        );
        assert_eq!(
            ask(AuthorizedActionTakers::Identity(me), me, None),
            Right::Alone
        );
        assert_eq!(
            ask(AuthorizedActionTakers::Identity(other), me, None),
            Right::No
        );
        assert_eq!(ask(AuthorizedActionTakers::NoOne, owner, None), Right::No);
        // A group of three, two signatures needed: a member takes part, nobody completes alone.
        assert_eq!(
            ask(AuthorizedActionTakers::Group(0), me, None),
            Right::Group { needed: 2, of: 3 }
        );
        assert_eq!(
            ask(AuthorizedActionTakers::Group(0), id(9), None),
            Right::No
        );
        // A member with the power needed completes it alone.
        assert_eq!(
            ask(AuthorizedActionTakers::Group(1), me, None),
            Right::Alone
        );
        assert_eq!(
            ask(AuthorizedActionTakers::Group(1), other, None),
            Right::Group { needed: 3, of: 4 }
        );
        // The main group, when the token has one.
        assert_eq!(
            ask(AuthorizedActionTakers::MainGroup, me, Some(0)),
            Right::Group { needed: 2, of: 3 }
        );
        assert_eq!(ask(AuthorizedActionTakers::MainGroup, me, None), Right::No);
    }

    /// AC-D8: «Management» only with some action of the identity's.
    #[test]
    fn management_shows_only_with_some_right() {
        let none: BTreeMap<_, _> = Action::ALL.iter().map(|a| (*a, Right::No)).collect();
        assert!(!manages(&none));
        let mut one = none.clone();
        one.insert(Action::Burn, Right::Group { needed: 2, of: 3 });
        assert!(manages(&one));
    }
}
