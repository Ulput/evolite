//! FEAT-0040: the fee of a Core payment to an address, for advanced users — «Fee» under
//! «Coins»: «Normal», «Higher» or «Custom» in duffs per kB, kept for the wallet. The form counts
//! at that rate everywhere a Core fee shows — «Total», «Max», the coins chosen — and the payment
//! carries it ([`evolite_api::Spend::Send::fee_rate`]).

use super::*;
use evolite_api::fee_choice::{self, FeeChoice};

/// The rate the form counts and pays at, when it is not the normal one: the advanced features
/// on, and a payee that is an address (a contact's payment is built with the normal fee, R-8).
pub(super) fn rate(cx: &Ctx, form: &SendForm, typed: &str) -> Option<u64> {
    if !cx.advanced() || typed.trim().starts_with('@') {
        return None;
    }
    match cx.fee() {
        // A «Custom» not typed right is no rate: «Check» waits for it (R-9).
        FeeChoice::Custom(_) if custom_problem(&form.fee_custom).is_some() => None,
        choice => choice.carried(),
    }
}

/// The Core source's fees in `all` at `rate`.
pub(super) fn rate_core(all: &mut [SourceView], rate: Option<u64>) {
    let Some(rate) = rate else { return };
    for s in all.iter_mut().filter(|s| s.source == Source::Core) {
        s.fee = fee_choice::at_rate(s.fee, rate);
        s.max_fee = fee_choice::at_rate(s.max_fee, rate);
    }
}

/// `fees` with the Core payment's at `rate`, for the coins chosen and «Max» (R-7).
pub(super) fn rated(fees: &evolite_api::Fees, rate: Option<u64>) -> evolite_api::Fees {
    let mut fees = fees.clone();
    if let Some(rate) = rate {
        fees.core_payment = fee_choice::at_rate(fees.core_payment, rate);
        fees.core_unsignable = fee_choice::at_rate(fees.core_unsignable, rate);
        for coin in &mut fees.core_coins {
            coin.fee = fee_choice::at_rate(coin.fee, rate);
        }
    }
    fees
}

/// What a typed «Custom» rate is: digits, with spaces between them taken out (R-9). A comma or
/// a dot is no rate in any language — in de, ru, fr and others the comma is the decimal mark,
/// and «2000,5» read as 20005 would be ten times the rate (review r1, Must 1).
pub(super) fn parse_custom(text: &str) -> Option<u64> {
    let digits: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Why a typed «Custom» rate cannot go (R-9): none, under the least, over the most.
pub(super) fn custom_problem(text: &str) -> Option<&'static str> {
    if text.contains([',', '.']) {
        return Some(tr!("Enter a whole number of duffs per kB"));
    }
    match parse_custom(text) {
        None | Some(0) => Some(tr!("Enter a number of duffs per kB")),
        Some(rate) if rate < fee_choice::MIN => Some(tr!(
            "At least 1,000 duffs per kB: nodes do not pass on a cheaper transaction"
        )),
        Some(rate) if rate > fee_choice::MAX => Some(tr!("At most 1,000,000 duffs per kB")),
        Some(_) => None,
    }
}

/// The rate in words and digits, for «Check the payment» (R-7): «Higher · 2 000 duffs per kB».
pub(super) fn shown(rate: u64) -> String {
    let name = match rate {
        fee_choice::NORMAL => tr!("Normal"),
        fee_choice::HIGHER => tr!("Higher"),
        _ => tr!("Custom"),
    };
    format!(
        tr!("{} · {} duffs per kB"),
        name,
        crate::text::group_thousands(rate as i64)
    )
}

/// The row under «Coins» (F1, F1b). `to_contact`: the choice is shown, not active (R-8).
pub(super) fn fee_row(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    choice: FeeChoice,
    to_contact: bool,
    asked: &mut Asked,
) {
    let p = a.p;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, &p, tr!("Fee"));
        if to_contact {
            ui.label(txt(
                &format!(
                    "{} · {}",
                    tr!("Normal"),
                    tr!("A payment to a contact is built with the normal fee")
                ),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            return;
        }
        let mut at = match choice {
            FeeChoice::Normal => 0,
            FeeChoice::Higher => 1,
            FeeChoice::Custom(_) => 2,
        };
        let was = at;
        crate::components::segmented(
            ui,
            &p,
            &[tr!("Normal"), tr!("Higher"), tr!("Custom")],
            &mut at,
            true,
        );
        if at != was {
            asked.fee = Some(match at {
                0 => FeeChoice::Normal,
                1 => FeeChoice::Higher,
                _ => FeeChoice::Custom(parse_custom(&form.fee_custom).unwrap_or(fee_choice::NORMAL)),
            });
        }
        match choice {
            FeeChoice::Higher => {
                ui.label(txt(
                    &format!(
                        tr!("{} duffs per kB — twice the normal"),
                        crate::text::group_thousands(fee_choice::HIGHER as i64)
                    ),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
            FeeChoice::Custom(kept) => {
                if form.fee_custom.is_empty() {
                    form.fee_custom = kept.to_string();
                }
                ui.horizontal(|ui| {
                    let field = text_field(
                        ui,
                        &p,
                        "send-fee-custom",
                        &mut form.fee_custom,
                        "",
                        FieldKind::Text,
                        |_| {},
                    );
                    if field.changed()
                        && let Some(rate) = parse_custom(&form.fee_custom)
                        && !fee_choice::refused(rate)
                    {
                        asked.fee = Some(FeeChoice::Custom(rate));
                    }
                    ui.label(txt(tr!("duffs per kB"), 12.0, Weight::Regular, p.muted));
                });
                match custom_problem(&form.fee_custom) {
                    Some(why) => {
                        ui.label(txt(why, 12.0, Weight::Regular, p.danger));
                    }
                    None if parse_custom(&form.fee_custom)
                        .is_some_and(|r| r > fee_choice::WARN_OVER) =>
                    {
                        ui.label(txt(
                            tr!("The fee is 10 times the normal one"),
                            12.0,
                            Weight::Regular,
                            p.text,
                        ));
                    }
                    None => {}
                }
            }
            FeeChoice::Normal => {}
        }
        ui.label(txt(
            tr!("Dash blocks are rarely full, so the normal fee is almost always enough. InstantSend confirms a payment in seconds at any rate."),
            12.0,
            Weight::Regular,
            p.muted,
        ));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R-9: digits with spaces between them are taken; a comma, a dot, anything else, or out of
    /// bounds, says why.
    #[test]
    fn a_custom_rate_is_digits_within_the_bounds() {
        assert_eq!(parse_custom("12 000"), Some(12_000));
        assert_eq!(parse_custom("12\u{a0}000"), Some(12_000));
        assert_eq!(parse_custom("12\u{202f}000"), Some(12_000));
        // Review r1, Must 1: a decimal mark is no group separator — refused in every language.
        for lang in [
            crate::i18n::Lang::Ru,
            crate::i18n::Lang::De,
            crate::i18n::Lang::En,
        ] {
            crate::i18n::set(lang);
            for bad in ["2000,5", "2000.5", "12,000", "12.000"] {
                assert_eq!(parse_custom(bad), None, "{lang:?}: {bad}");
                assert_eq!(
                    custom_problem(bad),
                    Some("Enter a whole number of duffs per kB"),
                    "{lang:?}: {bad}"
                );
            }
        }
        crate::i18n::set(crate::i18n::Lang::En);
        assert_eq!(parse_custom("12.5"), None);
        assert_eq!(parse_custom("abc"), None);
        assert_eq!(parse_custom(""), None);
        assert!(custom_problem("12 000").is_none());
        for bad in ["", "0", "abc", "12.5", "999", "1000001"] {
            assert!(custom_problem(bad).is_some(), "{bad}");
        }
    }

    /// R-7: the fees the form counts by are the normal ones at the rate — twice at «Higher».
    #[test]
    fn the_form_counts_at_the_chosen_rate() {
        let mut fees = crate::screens::shielded::tests::fees();
        fees.core_payment = 226;
        let higher = rated(&fees, Some(fee_choice::HIGHER));
        assert_eq!(higher.core_payment, 452);
        assert_eq!(rated(&fees, None).core_payment, 226);
    }
}
