//! The amounts of History in the currency of «Fiat rate» (FEAT-0008, UC-3…5): under a row's
//! amount and in the card's «In currency» block. At the rate kept when the operation was seen,
//! else at today's, said so. With «Fiat rate» off, nothing (R-1).

use chrono::{DateTime, Local};
use evolite_api::{Activity, ActivityKind, Duffs, FiatRate, Timestamp};

/// What the operation is worth in the currency: a self transfer only its fee (the coins stayed
/// in the wallet); a payment out its amount without the fee (the fee has its own line in the
/// card); one in, what came.
fn worth(act: &Activity) -> Duffs {
    let fee = act.fee.unwrap_or(0);
    match act.kind {
        ActivityKind::SelfTransfer { .. }
        | ActivityKind::Mixing { .. }
        | ActivityKind::IntoMixing => fee,
        _ if act.amount < 0 => act.amount.saturating_neg().saturating_sub(fee).max(0),
        _ => act.amount,
    }
}

/// «30,78 USD for a DASH».
fn per_dash(rate: &FiatRate) -> String {
    let price = format!(
        "{} {}",
        crate::format::rate_number(rate.price),
        rate.currency
    );
    format!(tr!("{} for a DASH"), price)
}

/// The rate «Fiat rate» has now, if it is on.
fn rate_now() -> Option<FiatRate> {
    let r = crate::format::display().rate?;
    Some(FiatRate {
        currency: r.code().to_owned(),
        price: r.price,
        at: r.at,
    })
}

/// Under a row's amount (frame 6): «≈ 7,70 USD» at the rate of then, «≈ 153,90 USD · now» at
/// today's, «< 0,01 USD» below a cent. None with «Fiat rate» off.
pub fn row(act: &Activity) -> Option<String> {
    let now = rate_now()?;
    let amount = act.amount.saturating_abs();
    Some(match &act.rate {
        Some(then) => crate::format::about(amount, then.price, &then.currency),
        None => format!(
            "{} · {}",
            crate::format::about(amount, now.price, &now.currency),
            tr!("now")
        ),
    })
}

/// The card's «In currency» block (frames 7–8): the value, and the lines under it.
pub struct CardLines {
    pub value: String,
    pub lines: Vec<String>,
}

pub fn card(act: &Activity, now: DateTime<Local>) -> Option<CardLines> {
    let current = rate_now()?;
    let amount = worth(act);
    let about = |rate: &FiatRate, duffs| crate::format::about(duffs, rate.price, &rate.currency);
    let mut lines = Vec::new();
    let value = match &act.rate {
        Some(then) => {
            let at = format!(
                tr!("by the rate at {}"),
                crate::text::date_time(then.at, now)
            );
            lines.push(format!("{at} · {}", per_dash(then)));
            lines.push(format!(
                "{} {} · {}",
                tr!("Now"),
                about(&current, amount),
                per_dash(&current)
            ));
            about(then, amount)
        }
        None => {
            lines.push(format!(
                "{} · {}",
                tr!("by the current rate"),
                tr!("the rate at the time of the operation was not saved")
            ));
            about(&current, amount)
        }
    };
    // The fee in the currency only here and in the last look, never in the row.
    if let Some(fee) = act.fee.filter(|_| {
        !matches!(
            act.kind,
            ActivityKind::SelfTransfer { .. }
                | ActivityKind::Mixing { .. }
                | ActivityKind::IntoMixing
        )
    }) {
        let rate = act.rate.as_ref().unwrap_or(&current);
        lines.push(format!("{} {}", tr!("Fee"), about(rate, fee)));
    }
    Some(CardLines { value, lines })
}

/// The rate a form works with this frame: «Fiat rate» on and a price from the core.
#[derive(Clone, PartialEq, Debug)]
pub struct Rate {
    pub price: i64,
    pub code: String,
    pub at: Timestamp,
    /// At most 5 minutes old: typing in the currency is allowed (analyst-35).
    pub fresh: bool,
}

impl Rate {
    /// From what Settings and the core say now; none with «Fiat rate» off.
    pub fn now(now: Timestamp) -> Option<Self> {
        let r = crate::format::display().rate?;
        let api = evolite_api::FiatRate {
            currency: r.code().to_owned(),
            price: r.price,
            at: r.at,
        };
        Some(Self {
            fresh: evolite_api::fiat::fresh(&api, now),
            price: r.price,
            code: api.currency,
            at: r.at,
        })
    }

    /// The rate as the core is given it with the payment (АРХ-2).
    pub fn for_core(&self) -> evolite_api::FiatRate {
        evolite_api::FiatRate {
            currency: self.code.clone(),
            price: self.price,
            at: self.at,
        }
    }

    /// The amount in DASH for `typed` in the currency: «0» stays a zero to be refused as
    /// usual; text that is no amount, or does not fit, gives none.
    pub fn dash_text(&self, typed: &str) -> Option<String> {
        let cents = crate::format::parse_fiat(typed)?;
        evolite_api::fiat::duffs_for(cents, 2, self.price).map(crate::format::input_text)
    }

    /// The amount in the currency for `amount` typed in DASH.
    pub fn currency_text(&self, amount: &str) -> String {
        crate::format::parse_amount(amount)
            .and_then(|d| evolite_api::fiat::cents_of(d, self.price))
            .map(crate::format::fiat_input_text)
            .unwrap_or_default()
    }

    /// «30,78 USD».
    pub fn price_text(&self) -> String {
        format!("{} {}", crate::format::rate_number(self.price), self.code)
    }
}

/// The time of a rate: «10:42».
pub fn clock(at: Timestamp) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{Display, NumberStyle, Rate, set_display};
    use evolite_api::{Chain, Flow, Proof};

    fn paid(amount: Duffs, fee: Option<Duffs>, rate: Option<FiatRate>) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Sent { to: "@bob".into() },
            chain: Chain::Core,
            at: 1_700_000_000,
            amount,
            proof: Proof::Proved,
            txid: None,
            fee,
            block: None,
            address: None,
            layers: evolite_api::layer::CORE,
            flow: Flow::Out,
            key: "core:aa".into(),
            note: None,
            rate,
            detail: None,
        }
    }

    fn on(price: i64, code: &[u8; 3]) {
        set_display(Display {
            style: NumberStyle::CommaDot,
            rate: Some(Rate {
                price,
                currency: *code,
                at: 0,
            }),
            ..Display::default()
        });
    }

    fn then(price: i64) -> FiatRate {
        FiatRate {
            currency: "USD".into(),
            price,
            at: 1_700_000_000,
        }
    }

    #[test]
    fn off_says_nothing() {
        set_display(Display::default());
        let act = paid(-81_221_572, Some(226), Some(then(3_078_000_000)));
        assert_eq!(row(&act), None);
        assert!(card(&act, Local::now()).is_none());
    }

    #[test]
    fn a_kept_rate_stays_and_today_is_said_apart() {
        on(3_120_000_000, b"USD");
        let act = paid(-81_221_798, Some(226), Some(then(3_078_000_000)));
        assert_eq!(row(&act).as_deref(), Some("≈ 25.00 USD"));
        let card = card(&act, Local::now()).unwrap();
        assert_eq!(card.value, "≈ 25.00 USD");
        assert!(
            card.lines[0].ends_with("30.78 USD for a DASH"),
            "{:?}",
            card.lines
        );
        assert_eq!(card.lines[1], "Now ≈ 25.34 USD · 31.20 USD for a DASH");
        assert_eq!(card.lines[2], "Fee < 0.01 USD");
        set_display(Display::default());
    }

    #[test]
    fn an_old_operation_is_at_todays_rate_and_says_so() {
        on(3_078_000_000, b"USD");
        let act = paid(-81_221_798, Some(226), None);
        assert_eq!(row(&act).as_deref(), Some("≈ 25.00 USD · now"));
        let card = card(&act, Local::now()).unwrap();
        assert_eq!(
            card.lines[0],
            "by the current rate · the rate at the time of the operation was not saved"
        );
        set_display(Display::default());
    }

    #[test]
    fn another_currency_keeps_the_kept_rate_in_its_own() {
        // UC-5, AC-T8: USD kept, EUR now; nothing is converted.
        on(2_900_000_000, b"EUR");
        let act = paid(-81_221_798, Some(226), Some(then(3_078_000_000)));
        let card = card(&act, Local::now()).unwrap();
        assert_eq!(card.value, "≈ 25.00 USD");
        assert!(card.lines[1].contains("EUR"), "{:?}", card.lines);
        set_display(Display::default());
    }

    #[test]
    fn hidden_amounts_hide_the_currency_too() {
        on(3_078_000_000, b"USD");
        let mut d = crate::format::display();
        d.hide = true;
        set_display(d);
        let act = paid(-81_221_798, Some(226), Some(then(3_078_000_000)));
        assert_eq!(row(&act).as_deref(), Some("≈ •••• USD"));
        assert_eq!(card(&act, Local::now()).unwrap().value, "≈ •••• USD");
        set_display(Display::default());
    }
}
