//! `dash:` payment links (BIP21, as DashPay writes them): the address, `amount`, `label`,
//! `message` and `user` — the DashPay name (FEAT-0006, FR-2, FR-7). Pure functions: the same
//! reading serves a QR code and a link pasted into «To». The address is not checked here: it
//! goes into the field as if typed, and the field's usual checks say whether it is one of this
//! network.

use crate::format::DUFFS_PER_DASH;
use evolite_api::Duffs;

/// The most DASH there can be, so the most a link may ask.
const MAX_DASH: Duffs = 21_000_000;

/// A note longer than this is cut: it is a line in «Check the payment».
const NOTE_CHARS: usize = 200;

/// What a `dash:` link asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentLink {
    pub address: String,
    pub amount: Option<Duffs>,
    /// The DashPay name of `user=`, without `@`.
    pub user: Option<String>,
    /// `label`, else `message`: shown as «Note from the link», never as the recipient's name.
    pub note: Option<String>,
}

impl PaymentLink {
    /// What «To» gets: `@name` when the link names a DashPay user (the payment then goes to
    /// the contact, as in DashPay), else the address.
    pub fn recipient(&self) -> String {
        match &self.user {
            Some(name) => format!("@{name}"),
            None => self.address.clone(),
        }
    }
}

/// Why a `dash:` link is not taken: it is refused whole, nothing of it is filled in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No address, or one with characters an address does not have.
    NoAddress,
    /// `amount` with an exponent, a sign, more than 8 decimals, more than 21 million or not a number.
    Amount,
    /// A `req-` parameter evolite does not know: BIP21 says the link must then be refused.
    Required,
}

/// Whether `text` is a `dash:` link at all (the scheme in any case).
pub fn is_link(text: &str) -> bool {
    // `get`: the fifth byte may fall inside a letter of any text typed in «To».
    text.trim()
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("dash:"))
}

/// Reads a `dash:` link; `None` when `text` is not one.
pub fn parse(text: &str) -> Option<Result<PaymentLink, Refused>> {
    if !is_link(text) {
        return None;
    }
    let rest = &text.trim()[5..];
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let (address, query) = rest.split_once('?').unwrap_or((rest, ""));
    Some(read(address, query))
}

fn read(address: &str, query: &str) -> Result<PaymentLink, Refused> {
    let address = decode(address).ok_or(Refused::NoAddress)?;
    if address.is_empty() || !address.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Refused::NoAddress);
    }
    let mut link = PaymentLink {
        address,
        amount: None,
        user: None,
        note: None,
    };
    let mut label = None;
    let mut message = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = key.to_ascii_lowercase();
        match key.as_str() {
            "amount" => link.amount = Some(amount(value).ok_or(Refused::Amount)?),
            "label" => label = decode(value),
            "message" => message = decode(value),
            "user" => link.user = decode(value).and_then(|u| user(&u)),
            _ if key.starts_with("req-") => return Err(Refused::Required),
            _ => {}
        }
    }
    link.note = [label, message]
        .into_iter()
        .flatten()
        .map(|n| clean_note(&n))
        .find(|n| !n.is_empty());
    Ok(link)
}

/// `0.25` → 25 000 000 duffs: digits, at most one dot, at most 8 decimals, more than zero and
/// at most 21 million DASH.
pub fn amount(text: &str) -> Option<Duffs> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty())
        || !digits(whole)
        || !digits(fraction)
        || fraction.len() > 8
        || whole.len() > 8
    {
        return None;
    }
    let whole: Duffs = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction: Duffs = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<8}").parse().ok()?
    };
    let duffs = whole.checked_mul(DUFFS_PER_DASH)?.checked_add(fraction)?;
    (duffs > 0 && duffs <= MAX_DASH * DUFFS_PER_DASH).then_some(duffs)
}

/// A DashPay name from `user=`: `@` dropped, letters, digits and hyphens, 3–63 long; anything
/// else is not a name and is left out (the address still pays).
fn user(text: &str) -> Option<String> {
    let name = text.trim().trim_start_matches('@');
    let fits = (3..=63).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    fits.then(|| name.to_string())
}

/// Percent-decoding, `+` as a space (as Android's `Uri` reads DashPay's links); `None` when
/// the bytes are not UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// A note as text only: control and bidi characters dropped (they could turn the words around
/// the amount or the address), spaces folded, at most [`NOTE_CHARS`].
pub fn clean_note(text: &str) -> String {
    let kept: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && !is_bidi_or_invisible(*c))
        .collect();
    let folded = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    folded.chars().take(NOTE_CHARS).collect()
}

fn is_bidi_or_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// «My QR» (FR-8, as DashPay's `ReceiveInfoView`): `dash:<address>?user=<name>`, no amount.
pub fn my_link(address: &str, name: &str) -> String {
    format!("dash:{address}?user={}", name.trim_start_matches('@'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRESS: &str = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f";

    fn ok(text: &str) -> PaymentLink {
        parse(text).expect("a dash: link").expect("taken")
    }

    #[test]
    fn the_scheme_is_read_in_any_case() {
        for scheme in ["dash:", "DASH:", "Dash:", "dAsH:"] {
            assert_eq!(ok(&format!("{scheme}{ADDRESS}")).address, ADDRESS);
        }
        assert_eq!(ok(&format!("dash://{ADDRESS}")).address, ADDRESS);
        assert!(parse(ADDRESS).is_none());
        assert!(parse("https://example.com").is_none());
        assert!(parse("dash-key:yappr").is_none());
    }

    /// Anything can be typed in «To», and every change is asked whether it is a link.
    #[test]
    fn text_that_is_not_ascii_is_not_a_link_and_does_not_panic() {
        for text in [
            "дашь",
            "дашь мне",
            "abcdé",
            "🙂🙂",
            "a🙂",
            "@имя",
            "ДАШ:",
            "",
        ] {
            assert!(parse(text).is_none(), "{text}");
            assert_eq!(
                crate::qr_scan::classify(text),
                crate::qr_scan::Kind::NotDash
            );
        }
    }

    #[test]
    fn the_amount_is_dash_with_a_dot() {
        let at = |a: &str| parse(&format!("dash:{ADDRESS}?amount={a}")).unwrap();
        assert_eq!(at("0.25").unwrap().amount, Some(25_000_000));
        assert_eq!(at("1").unwrap().amount, Some(DUFFS_PER_DASH));
        assert_eq!(at(".5").unwrap().amount, Some(50_000_000));
        assert_eq!(at("0.00000001").unwrap().amount, Some(1));
        assert_eq!(
            at("21000000").unwrap().amount,
            Some(MAX_DASH * DUFFS_PER_DASH)
        );
        for wrong in [
            "1e3",
            "-1",
            "+1",
            "0.000000001",
            "21000000.00000001",
            "0",
            "0,25",
            "",
            ".",
            "1.2.3",
            "999999999999",
        ] {
            assert_eq!(at(wrong), Err(Refused::Amount), "{wrong}");
        }
    }

    #[test]
    fn label_message_and_user_are_url_decoded() {
        let link = ok(&format!(
            "dash:{ADDRESS}?label=Coffee%20%26%20cake&message=ignored&user=satoshi"
        ));
        assert_eq!(link.note.as_deref(), Some("Coffee & cake"));
        assert_eq!(link.user.as_deref(), Some("satoshi"));
        assert_eq!(link.recipient(), "@satoshi");
        let link = ok(&format!("dash:{ADDRESS}?message=Invoice+42%E2%84%96"));
        assert_eq!(link.note.as_deref(), Some("Invoice 42№"));
        assert_eq!(link.recipient(), ADDRESS);
    }

    #[test]
    fn an_unknown_required_parameter_refuses_the_link_and_others_are_ignored() {
        assert_eq!(
            parse(&format!("dash:{ADDRESS}?req-somethingnew=1")),
            Some(Err(Refused::Required))
        );
        assert_eq!(
            parse(&format!("dash:{ADDRESS}?REQ-x=1&amount=1")),
            Some(Err(Refused::Required))
        );
        let link = ok(&format!("dash:{ADDRESS}?is=1&r=https%3A%2F%2Fx&amount=0.1"));
        assert_eq!(link.amount, Some(10_000_000));
    }

    #[test]
    fn a_link_without_an_address_is_refused() {
        assert_eq!(parse("dash:"), Some(Err(Refused::NoAddress)));
        assert_eq!(parse("dash:?amount=1"), Some(Err(Refused::NoAddress)));
        assert_eq!(parse("dash:yX k3"), Some(Err(Refused::NoAddress)));
    }

    #[test]
    fn a_user_that_is_not_a_name_is_left_out() {
        for wrong in ["ab", "-bob", "bob-", "bo b", "b%C3%B6b", &"x".repeat(64)] {
            let link = ok(&format!("dash:{ADDRESS}?user={wrong}"));
            assert_eq!(link.user, None, "{wrong}");
            assert_eq!(link.recipient(), ADDRESS);
        }
        assert_eq!(
            ok(&format!("dash:{ADDRESS}?user=%40alice")).user.as_deref(),
            Some("alice")
        );
    }

    #[test]
    fn control_and_bidi_characters_leave_the_note() {
        let link = ok(&format!(
            "dash:{ADDRESS}?label=%E2%80%AEpay%E2%80%AC%0Anow%07%E2%81%A6x%E2%81%A9"
        ));
        assert_eq!(link.note.as_deref(), Some("pay nowx"));
        let only_marks = ok(&format!("dash:{ADDRESS}?label=%E2%80%8F&message=hello"));
        assert_eq!(only_marks.note.as_deref(), Some("hello"));
        let long = ok(&format!("dash:{ADDRESS}?label={}", "a".repeat(500)));
        assert_eq!(long.note.unwrap().chars().count(), NOTE_CHARS);
    }

    #[test]
    fn my_qr_reads_back() {
        let link = ok(&my_link(ADDRESS, "@satoshi"));
        assert_eq!(link.address, ADDRESS);
        assert_eq!(link.user.as_deref(), Some("satoshi"));
        assert_eq!(link.amount, None);
        assert_eq!(
            my_link(ADDRESS, "satoshi"),
            format!("dash:{ADDRESS}?user=satoshi")
        );
    }

    #[test]
    fn the_receive_screens_link_reads_back() {
        let uri = crate::screens::receive::payment_uri(ADDRESS, Some(25_000_000));
        let link = ok(&uri);
        assert_eq!(
            (link.address.as_str(), link.amount),
            (ADDRESS, Some(25_000_000))
        );
    }
}
