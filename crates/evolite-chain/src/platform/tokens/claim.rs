//! «Something to claim» (FEAT-0019, the architect's answer to designer-45): whether a perpetual
//! distribution has passed at least one interval since this identity last claimed it. No amount:
//! the distribution's function can give zero for an interval (stage 2 counts it).

use std::ops::RangeInclusive;

use dpp::balances::credits::TokenAmount;
use dpp::block::block_info::BlockInfo;
use dpp::block::epoch::EpochIndex;
use dpp::data_contract::associated_token::token_perpetual_distribution::TokenPerpetualDistribution;
use dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
use dpp::data_contract::associated_token::token_perpetual_distribution::methods::v0::{
    TokenPerpetualDistributionV0Accessors, TokenPerpetualDistributionV0Methods,
};
use dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_moment::RewardDistributionMoment;
use dpp::prelude::Identifier;
use dpp::version::PlatformVersion;

/// Whether `identity` may claim a perpetual distribution to `recipient` of a contract owned by
/// `owner` (FEAT-0019 T-2.1, R-15, R-20): the identity named, or the contract's owner; a
/// distribution to evonodes by participation is not claimed here (the customer, analyst-119).
pub fn may_claim(
    recipient: &TokenDistributionRecipient,
    identity: Identifier,
    owner: Identifier,
) -> bool {
    match recipient {
        TokenDistributionRecipient::ContractOwner => identity == owner,
        TokenDistributionRecipient::Identity(id) => *id == identity,
        TokenDistributionRecipient::EvonodesByParticipation => false,
    }
}

/// What a claim of a perpetual distribution pays, counted before it (T-2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Estimate {
    /// At least this much: Platform counts at the block that runs the claim, which may be a cycle
    /// later and pay more, never less (only the recipient claims).
    pub at_least: TokenAmount,
    /// One claim pays at most `max_cycles` cycles and more stays to claim after it.
    pub capped: bool,
    pub max_cycles: u32,
}

/// [`Estimate`] as Platform counts a claim (rs-drive `token_claim_transition_action` v0,
/// 575d961), with dpp's own calls and no arithmetic of ours: from the cycle start of the
/// contract's creation (`creation`, the distribution's kind of moment), or from the last paid
/// moment (`last_paid`, proved), up to the current interval of `block` (the proved metadata's
/// Platform height, time and epoch), at most `max_token_redemption_cycles` cycles.
pub fn estimate(
    distribution: &TokenPerpetualDistribution,
    creation: RewardDistributionMoment,
    last_paid: Option<RewardDistributionMoment>,
    block: &BlockInfo,
    platform_version: &PlatformVersion,
) -> Result<Estimate, String> {
    let kind = distribution.distribution_type();
    let creation_cycle_start = creation
        .cycle_start(kind.interval())
        .map_err(|e| e.to_string())?;
    let start = last_paid.unwrap_or(creation_cycle_start);
    let current = distribution.current_interval(block);
    let max_cycles = platform_version.system_limits.max_token_redemption_cycles;
    let until = kind
        .max_cycle_moment(start, current, max_cycles, platform_version)
        .map_err(|e| e.to_string())?;
    let at_least = kind
        .rewards_in_interval::<fn(RangeInclusive<EpochIndex>) -> Option<_>>(
            creation_cycle_start,
            start,
            until,
            None,
            platform_version,
        )
        .map_err(|e| e.to_string())?;
    Ok(Estimate {
        at_least,
        capped: until < current,
        max_cycles,
    })
}

/// Whether a payout of a distribution every `interval` falls after `since` (excluded) and up to
/// `now` (included). `since`: the last claim, or the distribution's start for a first claim. The
/// steps are counted as Platform counts them for a claim (`evaluate_interval`: start excluded,
/// end included, boundaries at multiples of the interval), so the flag never promises what a
/// claim would not pay for that reason. Moments of different kinds, or a zero interval: `false`.
pub fn interval_passed(
    interval: RewardDistributionMoment,
    since: RewardDistributionMoment,
    now: RewardDistributionMoment,
) -> bool {
    if interval.to_u64() == 0 || !since.same_type(&interval) || !since.same_type(&now) {
        return false;
    }
    since
        .steps_till(&now, &interval, false, true)
        .is_ok_and(|steps| steps > 0)
}

/// What a pre-programmed distribution holds for an identity (T-2.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scheduled {
    /// The payout one claim pays now: the earliest entry for the identity after the last paid
    /// time, not in the future (Platform pays one at a time).
    pub due: Option<(u64, TokenAmount)>,
    /// Entries due now after it, each a claim of its own («N more waiting»).
    pub due_after: usize,
    /// The first entry still in the future («next {date}»).
    pub next_at: Option<u64>,
}

/// [`Scheduled`] as Platform picks a pre-programmed payout (rs-drive
/// `token_claim_transition_action` v0, 575d961): entries of `identity` only (the recipient named
/// in each entry), up to `now_ms` included, after `last_paid_ms`.
pub fn scheduled(
    distributions: &std::collections::BTreeMap<
        u64,
        std::collections::BTreeMap<Identifier, TokenAmount>,
    >,
    identity: Identifier,
    last_paid_ms: Option<u64>,
    now_ms: u64,
) -> Scheduled {
    let mine = distributions
        .iter()
        .filter(|(at, _)| last_paid_ms.is_none_or(|paid| **at > paid))
        .filter_map(|(at, to)| to.get(&identity).map(|amount| (*at, *amount)));
    let (mut due, mut due_after, mut next_at) = (None, 0, None);
    for (at, amount) in mine {
        if at > now_ms {
            next_at = next_at.or(Some(at));
        } else if due.is_none() {
            due = Some((at, amount));
        } else {
            due_after += 1;
        }
    }
    Scheduled {
        due,
        due_after,
        next_at,
    }
}

#[cfg(test)]
mod scheduled_tests {
    use super::*;
    use std::collections::BTreeMap;

    /// T-2.2: the earliest unpaid entry not in the future, the recipient's own; the rest counted.
    #[test]
    fn the_next_payout_is_the_earliest_due_and_unpaid() {
        let (me, other) = (Identifier::new([1; 32]), Identifier::new([2; 32]));
        let entry = |pairs: &[(Identifier, TokenAmount)]| pairs.iter().copied().collect();
        let plan: BTreeMap<u64, BTreeMap<Identifier, TokenAmount>> = [
            (100, entry(&[(me, 5), (other, 7)])),
            (200, entry(&[(other, 9)])),
            (300, entry(&[(me, 6)])),
            (400, entry(&[(me, 8)])),
            (900, entry(&[(me, 10)])),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            scheduled(&plan, me, None, 450),
            Scheduled {
                due: Some((100, 5)),
                due_after: 2,
                next_at: Some(900)
            }
        );
        assert_eq!(
            scheduled(&plan, me, Some(100), 450).due,
            Some((300, 6)),
            "200 is another's"
        );
        assert_eq!(
            scheduled(&plan, me, Some(400), 450),
            Scheduled {
                due: None,
                due_after: 0,
                next_at: Some(900)
            },
            "only the future one is left"
        );
        assert_eq!(
            scheduled(&plan, me, Some(400), 900).due,
            Some((900, 10)),
            "right at now is due"
        );
        assert_eq!(
            scheduled(&plan, Identifier::new([3; 32]), None, 1_000),
            Scheduled::default()
        );
    }
}

#[cfg(test)]
mod estimate_tests {
    use super::*;
    use dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::DistributionFunction;
    use dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_type::RewardDistributionType;
    use dpp::data_contract::associated_token::token_perpetual_distribution::v0::TokenPerpetualDistributionV0;
    use RewardDistributionMoment::TimeBasedMoment as Time;

    const MINUTE: u64 = 60_000;

    fn every_minute(function: DistributionFunction) -> TokenPerpetualDistribution {
        TokenPerpetualDistribution::V0(TokenPerpetualDistributionV0 {
            distribution_type: RewardDistributionType::TimeBasedDistribution {
                interval: MINUTE,
                function,
            },
            distribution_recipient: TokenDistributionRecipient::ContractOwner,
        })
    }

    fn at(time_ms: u64) -> BlockInfo {
        BlockInfo {
            time_ms,
            ..Default::default()
        }
    }

    /// AC-T11: the estimate is what `rewards_in_interval` gives from the creation's cycle start or
    /// the last claim, on sets of moments; a fixed amount pays each whole interval passed.
    #[test]
    fn the_estimate_is_platforms_count() {
        let pv = PlatformVersion::latest();
        let d = every_minute(DistributionFunction::FixedAmount { amount: 10 });
        // Created at 1:30: the cycle starts at 1:00; at 5:20, 2:00…5:00 are paid: 4 payouts.
        let first = estimate(&d, Time(90_000), None, &at(320_000), pv).unwrap();
        assert_eq!(first.at_least, 40);
        assert!(!first.capped);
        // Claimed at 4:00: 5:00 only.
        let next = estimate(&d, Time(90_000), Some(Time(4 * MINUTE)), &at(320_000), pv).unwrap();
        assert_eq!(next.at_least, 10);
        // Claimed at 5:00: nothing before 6:00.
        let none = estimate(&d, Time(90_000), Some(Time(5 * MINUTE)), &at(359_999), pv).unwrap();
        assert_eq!(none.at_least, 0);
        // The same calls, side by side, on a growing function.
        let linear = every_minute(DistributionFunction::Linear {
            a: 1,
            d: 1,
            start_step: None,
            starting_amount: 5,
            min_value: None,
            max_value: None,
        });
        for (created, last, now) in [
            (0, None, 10 * MINUTE),
            (30_000, Some(3 * MINUTE), 7 * MINUTE + 1),
            (MINUTE, Some(2 * MINUTE), 2 * MINUTE),
        ] {
            let kind = linear.distribution_type();
            let start_cycle = Time(created).cycle_start(kind.interval()).unwrap();
            let start = last.map(Time).unwrap_or(start_cycle);
            let current = linear.current_interval(&at(now));
            let until = kind.max_cycle_moment(start, current, 128, pv).unwrap();
            let platform = kind
                .rewards_in_interval::<fn(RangeInclusive<EpochIndex>) -> Option<_>>(
                    start_cycle,
                    start,
                    until,
                    None,
                    pv,
                )
                .unwrap();
            let ours = estimate(&linear, Time(created), last.map(Time), &at(now), pv).unwrap();
            assert_eq!(ours.at_least, platform, "{created} {last:?} {now}");
        }
    }

    /// T-2.1: more cycles than one claim pays: capped, and more stays to claim.
    #[test]
    fn more_cycles_than_one_claim_pays_are_capped() {
        let pv = PlatformVersion::latest();
        let max = pv.system_limits.max_token_redemption_cycles as u64;
        let linear = every_minute(DistributionFunction::Linear {
            a: 0,
            d: 1,
            start_step: None,
            starting_amount: 1,
            min_value: None,
            max_value: None,
        });
        let within = estimate(&linear, Time(0), None, &at(max * MINUTE), pv).unwrap();
        assert!(!within.capped, "{within:?}");
        let over = estimate(&linear, Time(0), None, &at((max + 10) * MINUTE), pv).unwrap();
        assert!(over.capped);
        assert_eq!(over.max_cycles as u64, max);
        assert_eq!(over.at_least, max, "one a cycle, at most {max} cycles");
    }

    /// R-15, R-20: only the recipient claims; nobody claims a distribution to evonodes here.
    #[test]
    fn only_the_recipient_claims() {
        let (me, other) = (Identifier::new([1; 32]), Identifier::new([2; 32]));
        assert!(may_claim(
            &TokenDistributionRecipient::ContractOwner,
            me,
            me
        ));
        assert!(!may_claim(
            &TokenDistributionRecipient::ContractOwner,
            me,
            other
        ));
        assert!(may_claim(
            &TokenDistributionRecipient::Identity(me),
            me,
            other
        ));
        assert!(!may_claim(
            &TokenDistributionRecipient::Identity(other),
            me,
            me
        ));
        assert!(!may_claim(
            &TokenDistributionRecipient::EvonodesByParticipation,
            me,
            me
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RewardDistributionMoment::{
        BlockBasedMoment as Block, EpochBasedMoment as Epoch, TimeBasedMoment as Time,
    };

    #[test]
    fn blocks_count_from_the_last_claim_to_now() {
        // Every 100 blocks: payouts at 100, 200, …
        assert!(!interval_passed(Block(100), Block(150), Block(199)));
        assert!(
            interval_passed(Block(100), Block(150), Block(200)),
            "the payout at 200"
        );
        assert!(interval_passed(Block(100), Block(150), Block(1_000)));
        // Claimed right at a payout: the next one is a whole interval on.
        assert!(!interval_passed(Block(100), Block(200), Block(299)));
        assert!(interval_passed(Block(100), Block(200), Block(300)));
        assert!(
            !interval_passed(Block(100), Block(300), Block(300)),
            "nothing since"
        );
        assert!(
            !interval_passed(Block(100), Block(300), Block(250)),
            "now before the claim"
        );
        // Platform's own count (`steps_till`, start excluded) takes one step off when the last
        // claim was right at a payout: claimed at 200, a claim at 399 is paid nothing for 300 —
        // that one comes with the claim at 400. The flag says what a claim pays now.
        assert!(!interval_passed(Block(100), Block(200), Block(399)));
        assert!(interval_passed(Block(100), Block(200), Block(400)));
    }

    #[test]
    fn time_counts_in_milliseconds() {
        let hour = 3_600_000;
        let since = 10 * hour + 1;
        assert!(!interval_passed(
            Time(hour),
            Time(since),
            Time(11 * hour - 1)
        ));
        assert!(interval_passed(Time(hour), Time(since), Time(11 * hour)));
    }

    #[test]
    fn epochs_count_whole_epochs() {
        assert!(!interval_passed(Epoch(1), Epoch(7), Epoch(7)));
        assert!(interval_passed(Epoch(1), Epoch(7), Epoch(8)));
        // Every 5 epochs: payouts at 5, 10, …
        assert!(!interval_passed(Epoch(5), Epoch(6), Epoch(9)));
        assert!(interval_passed(Epoch(5), Epoch(6), Epoch(10)));
    }

    #[test]
    fn mixed_kinds_or_no_interval_say_nothing() {
        assert!(!interval_passed(Block(100), Time(0), Block(1_000)));
        assert!(!interval_passed(Block(100), Block(0), Time(u64::MAX)));
        assert!(!interval_passed(Block(0), Block(0), Block(1_000)));
        assert!(!interval_passed(Epoch(0), Epoch(0), Epoch(9)));
    }
}
