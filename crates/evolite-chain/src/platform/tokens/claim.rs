//! «Something to claim» (FEAT-0019, the architect's answer to designer-45): whether a perpetual
//! distribution has passed at least one interval since this identity last claimed it. No amount:
//! the distribution's function can give zero for an interval (stage 2 counts it).

use dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_moment::RewardDistributionMoment;

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
