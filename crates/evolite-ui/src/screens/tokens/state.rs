//! The tokens screen's rules without drawing (FEAT-0019 part 4): how a token amount is written
//! and read in the interface's number format (review r1, P3 1), the order of the rows and why a
//! row cannot be sent, and the send form's checks before the password (AC-T3, R-3).

use evolite_api::tokens::{
    AmountError, Sender, TokenAmount, TokenRefusal, TokenRow, amount_text, parse_amount,
    send_refusal,
};

/// The token's name in the interface's language (`name_in`: else English, else the first).
pub fn name(info: &evolite_api::tokens::TokenInfo) -> &str {
    evolite_api::tokens::name_in(info, crate::i18n::current().code())
}

/// `amount` with the token's `decimals` in the chosen number format, thousands marked:
/// «1 250,00», «12». No unit: the row shows the name beside it (R-4).
pub fn written(amount: TokenAmount, decimals: u8) -> String {
    let (group, decimal) = crate::format::number_marks();
    let plain = amount_text(amount, decimals);
    let (whole, fraction) = match plain.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (plain.as_str(), None),
    };
    let mut out = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            out.push(group);
        }
        out.push(c);
    }
    if let Some(fraction) = fraction {
        out.push(decimal);
        out.push_str(fraction);
    }
    out
}

/// A typed amount in the chosen number format, in the token's smallest units. With spaces
/// between thousands both a comma and a dot are a decimal mark, as for DASH; with a comma or a
/// dot between thousands it groups exactly three digits.
pub fn typed(input: &str, decimals: u8) -> Result<TokenAmount, AmountError> {
    let (group, decimal) = crate::format::number_marks();
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let plain = if group.is_whitespace() {
        clean.replace(',', ".")
    } else {
        let (whole, fraction) = match clean.split_once(decimal) {
            Some((w, f)) => (w, Some(f)),
            None => (clean.as_str(), None),
        };
        let mut groups = whole.split(group);
        let first = groups.next().unwrap_or("");
        let rest: Vec<&str> = groups.collect();
        if !rest.is_empty()
            && (first.is_empty() || first.len() > 3 || rest.iter().any(|g| g.len() != 3))
        {
            return Err(AmountError::NotANumber);
        }
        let whole: String = whole.chars().filter(|c| *c != group).collect();
        match fraction {
            Some(f) => format!("{whole}.{f}"),
            None => whole,
        }
    };
    parse_amount(&plain, decimals)
}

/// «7Hq2…9fLm»: an id in a row, as the mockups write it.
pub fn short(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= 10 {
        return id.to_owned();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// Paused or frozen: the rows that go last.
fn held(row: &TokenRow) -> bool {
    row_reason(row).is_some()
}

/// Sendable rows first, then the ones a reason keeps (R-3а); by name inside each group (design §4,
/// review 4b).
pub fn ordered(rows: &[TokenRow]) -> Vec<&TokenRow> {
    let mut rows: Vec<&TokenRow> = rows.iter().collect();
    rows.sort_by(|a, b| {
        held(a)
            .cmp(&held(b))
            .then_with(|| {
                name(&a.info)
                    .to_lowercase()
                    .cmp(&name(&b.info).to_lowercase())
            })
            .then_with(|| a.info.token_id.cmp(&b.info.token_id))
    });
    rows
}

/// Why the row's «Send» is off, said in the row; `None`: it can be sent. Only what the token
/// and the balance say, one reason in R-3а's order (paused, frozen, not transferable, not
/// checked, 0): the recipient, the amount and the credits are the dialog's.
pub fn row_reason(row: &TokenRow) -> Option<&'static str> {
    if row.paused == Some(true) {
        Some(tr!(
            "The issuer paused the token — transfers are not available now."
        ))
    } else if row.frozen == Some(true) {
        Some(tr!("The issuer froze this balance — it cannot be sent."))
    } else if !row.info.transferable {
        Some(tr!(
            "This token cannot be transferred: its issuer decided so."
        ))
    } else if row.balance.is_none() {
        // R-3а (analyst, 2026-11-13): «—» says why too.
        Some(tr!(
            "The balance is not checked yet — you can send after the check."
        ))
    } else if row.balance == Some(0) {
        Some(tr!("The balance is 0 — there is nothing to send."))
    } else {
        None
    }
}

/// Written like a Dash address (Core Base58 or a Platform `tdash1…`): a token goes to an
/// identity, not an address (UC-3). An identity id is 43–44 characters, longer than a Core
/// address.
pub fn looks_like_address(to: &str) -> bool {
    let to = to.trim();
    let lower = to.to_ascii_lowercase();
    if lower.starts_with("tdash1") || lower.starts_with("dash1") {
        return to.len() >= 40 && to.chars().all(|c| c.is_ascii_alphanumeric());
    }
    (25..=36).contains(&to.chars().count())
        && to
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && !matches!(c, '0' | 'O' | 'I' | 'l'))
}

/// What stops the send form now, under its field (AC-T3); checked before the password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormProblem {
    /// Under «To».
    Address,
    /// Under «Amount»: not a number for this token, or more digits than it has.
    Amount,
    /// Under «Amount»: more than the balance; the balance is said back.
    MoreThan(TokenAmount),
    /// The token or the balance refuses any send (paused, frozen, not transferable, no key,
    /// not known yet).
    Token(TokenRefusal),
    /// Instead of the fee line: the identity's credits do not cover it.
    NoCredits,
}

/// The first thing that stops sending `amount` (as typed) of `row` to `to`; `None`: «Check»
/// may go on. Empty fields stop nothing yet: they only keep «Check» off.
pub fn form_problem(row: &TokenRow, to: &str, amount: &str, sender: Sender) -> Option<FormProblem> {
    if let Some(refusal) = send_refusal(row, None, sender) {
        return Some(match refusal {
            TokenRefusal::NoCredits => FormProblem::NoCredits,
            other => FormProblem::Token(other),
        });
    }
    if !to.trim().is_empty() && looks_like_address(to) {
        return Some(FormProblem::Address);
    }
    if amount.trim().is_empty() {
        return None;
    }
    let Ok(amount) = typed(amount, row.info.decimals) else {
        return Some(FormProblem::Amount);
    };
    match send_refusal(row, Some(amount), sender)? {
        TokenRefusal::MoreThanBalance(balance) => Some(FormProblem::MoreThan(balance)),
        TokenRefusal::Zero => Some(FormProblem::Amount),
        TokenRefusal::NoCredits => Some(FormProblem::NoCredits),
        other => Some(FormProblem::Token(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::tokens::{TokenInfo, TokenOrigin};

    fn row(name: &str, balance: u64, paused: bool, frozen: bool) -> TokenRow {
        TokenRow {
            info: TokenInfo {
                token_id: format!("7Hq2{name}xxxxxxxxxxxxxxxxxxxxxxxxxxxx9fLm"),
                contract_id: "C".into(),
                position: 0,
                names: vec![("en".into(), name.into())],
                decimals: 2,
                issuer: "I".into(),
                issuer_name: None,
                transferable: true,
                keeps_history: false,
            },
            origin: TokenOrigin::Found,
            balance: Some(balance),
            paused: Some(paused),
            frozen: Some(frozen),
            claim: false,
        }
    }

    const SENDER: Sender = Sender {
        has_key: true,
        credits: Some(1_000_000),
        fee: 1_000,
    };

    #[test]
    fn amounts_are_written_and_read_in_the_chosen_format() {
        crate::i18n::set(crate::i18n::Lang::Ru);
        crate::format::set_display(crate::format::Display::default());
        assert_eq!(written(125_000, 2), "1\u{a0}250,00");
        assert_eq!(written(12, 0), "12");
        assert_eq!(typed("1 250,5", 2), Ok(125_050));
        assert_eq!(
            typed("1250.50", 2),
            Ok(125_050),
            "a dot is a decimal mark too"
        );
        assert_eq!(typed("12", 0), Ok(12));
        assert_eq!(typed("0,001", 2), Err(AmountError::TooManyDecimals));
        crate::i18n::set(crate::i18n::Lang::En);
        assert_eq!(written(125_000, 2), "1,250.00");
        assert_eq!(typed("1,250.5", 2), Ok(125_050));
        assert!(
            typed("1,25", 2).is_err(),
            "a comma groups three digits in English"
        );
    }

    #[test]
    fn rows_sendable_first_then_by_name() {
        let rows = [
            row("Zeta", 1, false, false),
            row("Alpha", 1, true, false),
            row("Beta", 1, false, false),
            row("Gamma", 1, false, true),
            row("Aardvark", 0, false, false),
        ];
        let names: Vec<&str> = ordered(&rows).iter().map(|r| name(&r.info)).collect();
        // A balance of 0 cannot be sent either: it goes with the held ones (review 4b).
        assert_eq!(names, ["Beta", "Zeta", "Aardvark", "Alpha", "Gamma"]);
    }

    #[test]
    fn a_row_says_why_it_cannot_be_sent() {
        assert_eq!(row_reason(&row("A", 5, false, false)), None);
        assert!(row_reason(&row("A", 5, true, false)).is_some_and(|r| r.contains("paused")));
        assert!(row_reason(&row("A", 5, false, true)).is_some_and(|r| r.contains("froze")));
        let mut fixed = row("A", 5, false, false);
        fixed.info.transferable = false;
        assert!(row_reason(&fixed).is_some_and(|r| r.contains("cannot be transferred")));
        // R-3а: a zero balance and one not checked say why too, after the issuer's reasons.
        assert_eq!(
            row_reason(&row("A", 0, false, false)),
            Some("The balance is 0 — there is nothing to send.")
        );
        let mut unread = row("A", 0, false, false);
        unread.balance = None;
        assert_eq!(
            row_reason(&unread),
            Some("The balance is not checked yet — you can send after the check.")
        );
        unread.paused = Some(true);
        assert!(
            row_reason(&unread).is_some_and(|r| r.contains("paused")),
            "paused first"
        );
    }

    #[test]
    fn short_ids_as_the_mockups_write_them() {
        assert_eq!(short("7Hq2abcdefghijklmnopqrstuvwxyz9fLm"), "7Hq2…9fLm");
        assert_eq!(short("abc"), "abc");
    }

    #[test]
    fn the_form_refuses_before_the_password() {
        crate::i18n::set(crate::i18n::Lang::En);
        let r = row("Coffee", 125_000, false, false);
        assert_eq!(
            form_problem(&r, "", "", SENDER),
            None,
            "empty: nothing said yet"
        );
        assert_eq!(form_problem(&r, "@alice", "5", SENDER), None);
        assert_eq!(
            form_problem(&r, "yNk2Hc8pWq4mR7vT3sLb9XeJ6uZa5Df1Gh", "5", SENDER),
            Some(FormProblem::Address)
        );
        assert_eq!(
            form_problem(&r, "@alice", "5x", SENDER),
            Some(FormProblem::Amount)
        );
        assert_eq!(
            form_problem(&r, "@alice", "1251", SENDER),
            Some(FormProblem::MoreThan(125_000))
        );
        let poor = Sender {
            credits: Some(10),
            ..SENDER
        };
        assert_eq!(
            form_problem(&r, "@alice", "5", poor),
            Some(FormProblem::NoCredits)
        );
        let keyless = Sender {
            has_key: false,
            ..SENDER
        };
        assert_eq!(
            form_problem(&r, "", "", keyless),
            Some(FormProblem::Token(TokenRefusal::NoKey))
        );
    }

    #[test]
    fn dash_addresses_are_told_apart_from_identity_ids() {
        assert!(looks_like_address("yNk2Hc8pWq4mR7vT3sLb9XeJ6uZa5Df1Gh"));
        assert!(looks_like_address(
            "tdash1zq8f5v6tnhx0ldqpw9c3ujmtk2yg7s4ea5r9lnvx3wd8kzp0q6myh2jhc7tq9ejw5aqr0"
        ));
        assert!(!looks_like_address(
            "5TKGz2Jm9wDqYp4rHcV8sNbE1xUa7LfQ3kRt6WhXoP2d"
        ));
        assert!(!looks_like_address("@alice"));
    }
}
