//! Platform tokens (FEAT-0019): what the screens show of them, and the pure rules that the UI
//! and the core both keep — an amount with the token's decimals, and what refuses a send before
//! the password (R-3, R-4). No network here: the core fills the facts, the screens read them.

use crate::Timestamp;

/// A token amount in the token's smallest units (Platform's `TokenAmount`). Shown with the
/// token's `decimals`, never as DASH (R-4).
pub type TokenAmount = u64;

/// What a token is, from its contract read with a proof.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TokenInfo {
    /// Base58, 32 bytes: `calculate_token_id(contract, position)`.
    pub token_id: String,
    /// Base58 id of the data contract that issues it.
    pub contract_id: String,
    /// Its place among the contract's tokens.
    pub position: u16,
    /// Its names as the issuer gave them: `(language code, singular form)` in the contract's
    /// order, capitalized already when the contract says so. The screen picks one ([`name_in`],
    /// review r1 of part 3, Should 2): the core does not know the interface's language. Platform has no ticker (architect, 2026-11-11), and anyone can call a token anything:
    /// the screens show it with the short `token_id` and the issuer (R-8); the id is the only
    /// unique key. Amounts are shown with the name beside them («100,00 · Evo Coffee»): the
    /// contract's plural form is not used, as Russian and Arabic need more forms than it gives
    /// (designer, 2026-11-11).
    pub names: Vec<(String, String)>,
    /// Digits after the point when shown (Platform's default is 8; 0 means whole units).
    pub decimals: u8,
    /// Base58 id of the contract's owner.
    pub issuer: String,
    /// The owner's `@name`, when known.
    pub issuer_name: Option<String>,
    /// `false`: the network refuses every transfer of it (configuration V1).
    pub transferable: bool,
    /// Transfers are kept in the TokenHistory contract, so incoming ones can be read (UC-5).
    pub keeps_history: bool,
}

/// How a token came into the wallet's list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenOrigin {
    /// The wallet's own search of the network found a balance on one of its identities.
    Found,
    /// Added by its id or found by its keyword (UC-1).
    ByHand,
}

/// A token on the shown identity. `None` is "not known yet", never zero (AC-T5).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TokenRow {
    pub info: TokenInfo,
    pub origin: TokenOrigin,
    pub balance: Option<TokenAmount>,
    /// The issuer paused the token: nothing of it moves.
    pub paused: Option<bool>,
    /// The issuer froze this identity's balance of it.
    pub frozen: Option<bool>,
    /// A perpetual distribution to this identity has passed an interval since its last claim:
    /// there may be something to claim. No amount (stage 2).
    pub claim: bool,
}

/// Where the wallet's own search of the network for its tokens stands (UC-1а, R-11).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SearchState {
    /// Not on this network (mainnet, R-9): nothing is searched and nothing is asked.
    Off,
    /// Not started on this network yet: it starts when the tokens screen first opens.
    NotYet,
    /// Going through the network's contracts.
    Running,
    /// A node asked to slow down: the search goes on by itself after `until`.
    Paused { until: Timestamp },
    /// The network does not answer: the search goes on from where it stopped once it does.
    NoNetwork,
    /// Not running.
    Idle,
}

/// What a pass of the search is doing now (F, BUG-134/135).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SearchStage {
    /// Reading the network's contracts: `checked` of `total`.
    #[default]
    Contracts,
    /// Asking the balances of the tokens found, on identity `identity` of `of`.
    Balances { identity: u32, of: u32 },
}

/// The search's progress and its last whole pass.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TokenSearch {
    pub state: SearchState,
    /// What the pass under way is doing.
    pub stage: SearchStage,
    /// Contracts checked in the stage under way.
    pub checked: u32,
    /// How many the stage has in all, once known; `None` while a first pass reads the network's
    /// pages, whose count is unknown until the last one. Never less than `checked`.
    pub total: Option<u32>,
    /// About how long the pass still takes, once its pace is known.
    pub left_secs: Option<u32>,
    /// When the last whole pass ended, and how many contracts it checked.
    pub done: Option<(Timestamp, u32)>,
    /// The last pass the person asked for ("Find my tokens") failed; the list is the one of
    /// `done`. A failed pass in the background says nothing.
    pub failed: bool,
}

impl TokenSearch {
    /// Nothing on this network.
    pub fn off() -> Self {
        TokenSearch {
            state: SearchState::Off,
            stage: SearchStage::Contracts,
            checked: 0,
            total: None,
            left_secs: None,
            done: None,
            failed: false,
        }
    }
}

/// The wallet's tokens on the shown identity, and its search.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tokens {
    pub rows: Vec<TokenRow>,
    pub search: TokenSearch,
    /// Ids of the followed tokens the person hid on the shown identity: «Add a token» offers
    /// them back («Return to the list») instead of «Already in the list».
    pub hidden: Vec<String>,
}

impl Tokens {
    /// No tokens and no search (mainnet, or before the core reads anything).
    pub fn off() -> Self {
        Tokens {
            rows: Vec::new(),
            search: TokenSearch::off(),
            hidden: Vec::new(),
        }
    }
}

/// Why an amount typed for a token is not one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AmountError {
    /// Empty, or not digits with at most one point.
    NotANumber,
    /// More digits after the point than the token has.
    TooManyDecimals,
    /// More than a token amount can hold.
    TooBig,
}

/// `amount` with the token's `decimals`, digits only and a point: "1250.00" for 125000 with 2,
/// "12" with 0. Grouping and the language's separators are the screen's.
pub fn amount_text(amount: TokenAmount, decimals: u8) -> String {
    let digits = amount.to_string();
    let decimals = usize::from(decimals);
    if decimals == 0 {
        return digits;
    }
    // At least one digit before the point.
    let padded = format!("{digits:0>width$}", width = decimals + 1);
    let (whole, fraction) = padded.split_at(padded.len() - decimals);
    format!("{whole}.{fraction}")
}

/// The amount typed as digits with an optional point ("1250.5"), in the token's smallest
/// units. The screen turns the language's separators into this first.
pub fn parse_amount(text: &str, decimals: u8) -> Result<TokenAmount, AmountError> {
    let text = text.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() && fraction.is_empty() || !digits(whole) || !digits(fraction) {
        return Err(AmountError::NotANumber);
    }
    // Zeros at the end of the fraction change nothing.
    let fraction = fraction.trim_end_matches('0');
    let decimals = usize::from(decimals);
    if fraction.len() > decimals {
        return Err(AmountError::TooManyDecimals);
    }
    let mut amount: TokenAmount = 0;
    let more = (0..decimals - fraction.len()).map(|_| b'0');
    for b in whole.bytes().chain(fraction.bytes()).chain(more) {
        amount = amount
            .checked_mul(10)
            .and_then(|a| a.checked_add(TokenAmount::from(b - b'0')))
            .ok_or(AmountError::TooBig)?;
    }
    Ok(amount)
}

/// Why a token cannot be sent now; the words are the screen's (R-3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenRefusal {
    /// The issuer paused the token.
    Paused,
    /// The issuer froze this balance.
    Frozen,
    /// The issuer made it not transferable.
    NotTransferable,
    /// The identity has no key that may sign token transfers (CRITICAL authentication).
    NoKey,
    /// Its balance, pause or freeze is not known yet.
    NotKnown,
    /// Nothing to send.
    Zero,
    /// More than the balance; the balance is said back.
    MoreThanBalance(TokenAmount),
    /// The identity's credits do not cover the fee.
    NoCredits,
}

/// What the sender has besides the token row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sender {
    /// It has a CRITICAL authentication key (ECDSA), which platform-wallet signs token
    /// transitions with.
    pub has_key: bool,
    /// The identity's credits, when known.
    pub credits: Option<u64>,
    /// The fee in credits the send may take at most.
    pub fee: u64,
}

/// The first reason `amount` of `row` cannot be sent, checked before the password (R-3, R-5);
/// `None`: nothing stops it here (the recipient is checked on the network). `amount` `None`:
/// nothing typed yet, only what stops any send.
pub fn send_refusal(
    row: &TokenRow,
    amount: Option<TokenAmount>,
    sender: Sender,
) -> Option<TokenRefusal> {
    if row.paused == Some(true) {
        return Some(TokenRefusal::Paused);
    }
    if row.frozen == Some(true) {
        return Some(TokenRefusal::Frozen);
    }
    if !row.info.transferable {
        return Some(TokenRefusal::NotTransferable);
    }
    if !sender.has_key {
        return Some(TokenRefusal::NoKey);
    }
    let (Some(balance), Some(_), Some(_), Some(credits)) =
        (row.balance, row.paused, row.frozen, sender.credits)
    else {
        return Some(TokenRefusal::NotKnown);
    };
    match amount {
        Some(0) => Some(TokenRefusal::Zero),
        Some(a) if a > balance => Some(TokenRefusal::MoreThanBalance(balance)),
        _ if credits < sender.fee => Some(TokenRefusal::NoCredits),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> TokenRow {
        TokenRow {
            info: TokenInfo {
                token_id: "7Hq2".into(),
                contract_id: "Ab3x".into(),
                position: 0,
                names: vec![("en".into(), "Evo Coffee".into())],
                decimals: 2,
                issuer: "5TKG".into(),
                issuer_name: Some("evo-lab".into()),
                transferable: true,
                keeps_history: true,
            },
            origin: TokenOrigin::Found,
            balance: Some(125_000),
            paused: Some(false),
            frozen: Some(false),
            claim: false,
        }
    }

    const SENDER: Sender = Sender {
        has_key: true,
        credits: Some(10_000_000),
        fee: 2_000_000,
    };

    /// AC-T4: shown with the token's decimals, read back the same, at the edges of `u64`.
    #[test]
    fn amounts_go_to_text_and_back_without_loss() {
        assert_eq!(amount_text(125_000, 2), "1250.00");
        assert_eq!(amount_text(12, 0), "12");
        assert_eq!(amount_text(5, 8), "0.00000005");
        assert_eq!(amount_text(0, 3), "0.000");
        assert_eq!(amount_text(u64::MAX, 0), "18446744073709551615");
        assert_eq!(amount_text(u64::MAX, 8), "184467440737.09551615");
        // More decimals than a u64 has digits: still a number, all of it after the point.
        assert_eq!(amount_text(1, 25), format!("0.{}1", "0".repeat(24)));
        for (amount, decimals) in [
            (0, 0),
            (1, 0),
            (125_000, 2),
            (5, 8),
            (u64::MAX, 0),
            (u64::MAX, 8),
            (u64::MAX, 19),
            (u64::MAX, 20),
            (7, 30),
        ] {
            let text = amount_text(amount, decimals);
            assert_eq!(
                parse_amount(&text, decimals),
                Ok(amount),
                "{text} ({decimals})"
            );
        }
    }

    #[test]
    fn typed_amounts_are_read_or_refused() {
        assert_eq!(parse_amount("1250.5", 2), Ok(125_050));
        assert_eq!(parse_amount("1250", 2), Ok(125_000));
        assert_eq!(parse_amount(".5", 2), Ok(50));
        assert_eq!(parse_amount("1250.", 2), Ok(125_000));
        assert_eq!(parse_amount(" 12 ", 0), Ok(12));
        assert_eq!(parse_amount("0.001", 2), Err(AmountError::TooManyDecimals));
        assert_eq!(parse_amount("1.5", 0), Err(AmountError::TooManyDecimals));
        // Trailing zeros past the decimals change nothing: still the same amount.
        assert_eq!(parse_amount("1.500", 2), Ok(150));
        for bad in ["", ".", "-1", "1,5", "1.2.3", "1e3", "abc", "+1"] {
            assert_eq!(
                parse_amount(bad, 2),
                Err(AmountError::NotANumber),
                "{bad:?}"
            );
        }
        assert_eq!(
            parse_amount("18446744073709551616", 0),
            Err(AmountError::TooBig)
        );
        assert_eq!(
            parse_amount("184467440737.09551616", 8),
            Err(AmountError::TooBig)
        );
        assert_eq!(
            parse_amount(&"9".repeat(60), 0),
            Err(AmountError::TooBig),
            "no overflow on a long number"
        );
    }

    /// AC-T3: pause, freeze, not transferable, no key, more than the balance, no credits —
    /// refused before the password, in that order of what a person can do least about.
    #[test]
    fn a_send_is_refused_before_the_password() {
        assert_eq!(send_refusal(&row(), Some(100), SENDER), None);
        assert_eq!(
            send_refusal(&row(), Some(125_000), SENDER),
            None,
            "all of it"
        );
        assert_eq!(
            send_refusal(&row(), None, SENDER),
            None,
            "nothing typed yet"
        );

        let paused = TokenRow {
            paused: Some(true),
            frozen: Some(true),
            ..row()
        };
        assert_eq!(
            send_refusal(&paused, Some(1), SENDER),
            Some(TokenRefusal::Paused)
        );
        let frozen = TokenRow {
            frozen: Some(true),
            ..row()
        };
        assert_eq!(
            send_refusal(&frozen, None, SENDER),
            Some(TokenRefusal::Frozen)
        );
        let mut fixed = row();
        fixed.info.transferable = false;
        assert_eq!(
            send_refusal(&fixed, Some(1), SENDER),
            Some(TokenRefusal::NotTransferable)
        );
        assert_eq!(
            send_refusal(
                &row(),
                Some(1),
                Sender {
                    has_key: false,
                    ..SENDER
                }
            ),
            Some(TokenRefusal::NoKey)
        );
        for unknown in [
            TokenRow {
                balance: None,
                ..row()
            },
            TokenRow {
                paused: None,
                ..row()
            },
            TokenRow {
                frozen: None,
                ..row()
            },
        ] {
            assert_eq!(
                send_refusal(&unknown, Some(1), SENDER),
                Some(TokenRefusal::NotKnown),
                "{unknown:?}"
            );
        }
        assert_eq!(
            send_refusal(&row(), Some(0), SENDER),
            Some(TokenRefusal::Zero)
        );
        assert_eq!(
            send_refusal(&row(), Some(125_001), SENDER),
            Some(TokenRefusal::MoreThanBalance(125_000))
        );
        assert_eq!(
            send_refusal(
                &row(),
                Some(1),
                Sender {
                    credits: Some(1_999_999),
                    ..SENDER
                }
            ),
            Some(TokenRefusal::NoCredits)
        );
        // Credits not known yet: no fee can be promised, so not now either (R-5).
        assert_eq!(
            send_refusal(
                &row(),
                Some(1),
                Sender {
                    credits: None,
                    ..SENDER
                }
            ),
            Some(TokenRefusal::NotKnown)
        );
    }
}

/// The name of `info` to show in the interface's language `lang` (a code: "ru", "ar", "zh-Hans"):
/// that language's singular form if the issuer gave one, else English, else the first; empty
/// when the contract names none. The UI and the demo both call this (part 3 review r1, Should 2).
pub fn name_in<'a>(info: &'a TokenInfo, lang: &str) -> &'a str {
    let form = |code: &str| info.names.iter().find(|(c, _)| c == code);
    form(lang)
        .or_else(|| form("en"))
        .or_else(|| info.names.first())
        .map_or("", |(_, name)| name.as_str())
}

#[cfg(test)]
mod name_tests {
    use super::*;

    fn named(names: &[(&str, &str)]) -> TokenInfo {
        TokenInfo {
            token_id: "T".into(),
            contract_id: "C".into(),
            position: 0,
            names: names
                .iter()
                .map(|(c, n)| ((*c).to_owned(), (*n).to_owned()))
                .collect(),
            decimals: 0,
            issuer: "I".into(),
            issuer_name: None,
            transferable: true,
            keeps_history: false,
        }
    }

    #[test]
    fn the_interface_language_then_english_then_the_first() {
        let all = named(&[("de", "Kaffee"), ("en", "Coffee"), ("ru", "Кофе")]);
        assert_eq!(name_in(&all, "ru"), "Кофе");
        assert_eq!(
            name_in(&all, "ar"),
            "Coffee",
            "English when the interface's is not given"
        );
        let no_english = named(&[("fr", "Café"), ("ru", "Кофе")]);
        assert_eq!(
            name_in(&no_english, "ar"),
            "Café",
            "the first, in the contract's order"
        );
        assert_eq!(name_in(&named(&[]), "en"), "");
    }
}
