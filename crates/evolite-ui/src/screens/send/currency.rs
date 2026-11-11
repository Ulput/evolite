//! The amount in the currency of «Fiat rate» (FEAT-0008): the unit switch's state, the
//! amount it makes in DASH and the lines that say by which rate. What is signed is always the
//! amount in DASH (R-2); the currency is an estimate with «≈».

use super::*;

pub(super) use crate::fiat_lines::{Rate, clock};

impl SendForm {
    /// Whether the field takes the amount in the currency now: chosen, and the rate fresh.
    pub(super) fn typing_currency(&self, rate: Option<&Rate>) -> bool {
        self.in_currency && rate.is_some_and(|r| r.fresh)
    }

    /// Each frame, before anything reads the amount: what the field shows in the currency and
    /// the amount in DASH agree. An amount put in from elsewhere («Max», a link, a refusal
    /// giving it back) is shown in the currency; one typed in the currency follows the rate
    /// until «Check and send» (R-3), then stays as it is.
    pub(super) fn sync_currency(&mut self, rate: Option<&Rate>) {
        let Some(rate) = rate.filter(|_| self.typing_currency(rate)) else {
            return;
        };
        if self.amount != self.fiat_for {
            self.fiat = rate.currency_text(&self.amount);
            self.fiat_typed = false;
        } else if self.fiat_typed
            && (!self.confirming || self.amount.is_empty())
            && self.signing.is_none()
        {
            self.amount = rate.dash_text(&self.fiat).unwrap_or_default();
        }
        self.fiat_for = self.amount.clone();
    }

    /// The person typed `self.fiat`: the amount in DASH from it, rounded down (R-3).
    pub(super) fn typed_currency(&mut self, rate: &Rate) {
        self.fiat_typed = true;
        self.amount = rate.dash_text(&self.fiat).unwrap_or_default();
        self.fiat_for = self.amount.clone();
    }

    /// The unit switched: the amount keeps its meaning, 25 USD becomes 0,8122 DASH (behaviour 1).
    pub(super) fn switched_unit(&mut self) {
        // Shown again from the amount in DASH at the next sync.
        self.fiat_for = String::from('\u{0}');
        self.fiat_typed = false;
    }

    /// Text in the currency that is no amount: said as for DASH.
    pub(super) fn currency_not_an_amount(&self, rate: Option<&Rate>) -> bool {
        self.typing_currency(rate) && !self.fiat.trim().is_empty() && self.amount.is_empty()
    }
}

/// The line under the field (frames 1–3): by which rate, or why the currency cannot be typed.
pub(super) fn field_line(
    form: &SendForm,
    rate: Option<&Rate>,
    amount: Option<Duffs>,
) -> Option<String> {
    let rate = rate?;
    if !rate.fresh {
        return Some(format!(
            tr!(
                "The rate has not been updated since {}: input in {} is unavailable until it updates"
            ),
            clock(rate.at),
            rate.code
        ));
    }
    let amount = amount?;
    let dash = format!(
        "{} {}",
        crate::format::number(amount, false),
        crate::format::unit_name()
    );
    if form.typing_currency(Some(rate)) {
        Some(format!(
            tr!("≈ {} by the rate {} · updated {}. The amount is sent in DASH"),
            dash,
            rate.price_text(),
            clock(rate.at)
        ))
    } else {
        let about = crate::format::about(amount, rate.price, &rate.code);
        Some(format!(
            tr!("≈ {} by the rate {} · updated {}"),
            about.trim_start_matches("≈ "),
            rate.price_text(),
            clock(rate.at)
        ))
    }
}

/// Under «Amount» in «Summary»: «≈ 25,00 USD», with an old rate «… · rate at 08:12» (frame 3).
pub(super) fn summary_line(rate: Option<&Rate>, amount: Duffs) -> Option<String> {
    let rate = rate?;
    let about = crate::format::about(amount, rate.price, &rate.code);
    Some(if rate.fresh {
        about
    } else {
        let at = format!(tr!("rate at {}"), clock(rate.at));
        format!("{about} · {at}")
    })
}

/// Under the amount in the last look (frame 4): «≈ 25,00 USD by the rate 30,78 USD · updated 10:42».
pub(super) fn confirm_line(rate: Option<&Rate>, amount: Duffs) -> Option<String> {
    let rate = rate?;
    let about = crate::format::about(amount, rate.price, &rate.code);
    Some(format!(
        tr!("≈ {} by the rate {} · updated {}"),
        about.trim_start_matches("≈ "),
        rate.price_text(),
        clock(rate.at)
    ))
}

/// Under the fee in the last look: «< 0,01 USD».
pub(super) fn fee_line(rate: Option<&Rate>, fee: Duffs) -> Option<String> {
    let rate = rate?;
    Some(crate::format::about(fee, rate.price, &rate.code))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usd(at: Timestamp, fresh: bool) -> Rate {
        Rate {
            price: 3_078_000_000,
            code: "USD".into(),
            at,
            fresh,
        }
    }

    #[test]
    fn twenty_five_dollars_make_the_amount_in_dash_rounded_down() {
        crate::format::set_display(crate::format::Display {
            style: crate::format::NumberStyle::CommaDot,
            ..Default::default()
        });
        let rate = usd(0, true);
        let mut form = SendForm {
            in_currency: true,
            fiat: "25".into(),
            ..SendForm::empty()
        };
        form.typed_currency(&rate);
        assert_eq!(form.amount, "0.81221572");
        // «Max» put a DASH amount there: the currency follows it.
        form.amount = "1".into();
        form.sync_currency(Some(&rate));
        assert_eq!(form.fiat, "30.78");
        // Not an amount: none in DASH, said as such.
        form.fiat = "2x".into();
        form.typed_currency(&rate);
        assert!(form.amount.is_empty() && form.currency_not_an_amount(Some(&rate)));
        crate::format::set_display(Default::default());
    }

    #[test]
    fn the_amount_is_fixed_once_the_last_look_is_open() {
        crate::format::set_display(crate::format::Display {
            style: crate::format::NumberStyle::CommaDot,
            ..Default::default()
        });
        let mut form = SendForm {
            in_currency: true,
            fiat: "25".into(),
            ..SendForm::empty()
        };
        form.typed_currency(&usd(0, true));
        // A new rate before «Check and send»: the amount follows it.
        let higher = Rate {
            price: 5_000_000_000,
            ..usd(0, true)
        };
        form.sync_currency(Some(&higher));
        assert_eq!(form.amount, "0.5000");
        // In the last look a new rate changes nothing (R-3, AC-T3).
        form.confirming = true;
        form.sync_currency(Some(&usd(0, true)));
        assert_eq!(form.amount, "0.5000");
        crate::format::set_display(Default::default());
    }

    #[test]
    fn an_old_rate_takes_the_currency_away_and_says_since_when() {
        let rate = usd(0, false);
        let form = SendForm {
            in_currency: true,
            ..SendForm::empty()
        };
        assert!(!form.typing_currency(Some(&rate)));
        let line = field_line(&form, Some(&rate), None).unwrap();
        assert!(line.contains("USD"), "{line}");
        assert!(field_line(&form, None, Some(1)).is_none(), "off: nothing");
    }
}
