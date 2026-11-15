//! «Buy» (FEAT-0019 T-2, UC-8, АРХ-10): the issuer's price as the screens read it. The price is
//! per smallest unit of the token, in credits, and the count is in smallest units too (as Dash
//! Evo Tool counts it); what a count costs is [`evolite_api::tokens::quote`].

use dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use evolite_api::tokens::TokenAmount;

/// The price's tiers, ascending, and whether it is one price for any count.
pub fn tiers_of(schedule: &TokenPricingSchedule) -> (Vec<(TokenAmount, u64)>, bool) {
    match schedule {
        TokenPricingSchedule::SinglePrice(price) => (vec![(1, *price)], true),
        TokenPricingSchedule::SetPrices(tiers) => {
            (tiers.iter().map(|(n, p)| (*n, *p)).collect(), false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::tokens::{Quote, quote};

    /// AC-T12: Platform's two kinds of price, as the screens count them.
    #[test]
    fn both_kinds_of_price_read_as_tiers() {
        let (single, one) = tiers_of(&TokenPricingSchedule::SinglePrice(1_000));
        assert!(one);
        assert_eq!(
            quote(&single, 3),
            Quote::Total {
                total: 3_000,
                per_unit: 1_000,
                tier: 1
            }
        );
        let schedule =
            TokenPricingSchedule::SetPrices([(1_000, 40), (100, 50)].into_iter().collect());
        let (tiers, one) = tiers_of(&schedule);
        assert!(!one);
        assert_eq!(tiers, vec![(100, 50), (1_000, 40)], "ascending");
        assert_eq!(quote(&tiers, 50), Quote::UnderMinimum { minimum: 100 });
    }
}
