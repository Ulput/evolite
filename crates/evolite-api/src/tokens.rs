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
    /// What this identity may claim of it, read with a proof (T-2.1, T-2.2); `None`: nothing,
    /// or not read yet. «Claim» is shown only with something here.
    pub claims: Option<Claims>,
    /// The issuer's price, read with a proof (T-2.4); `None`: not sold, or not read yet.
    /// «Buy» is shown only with a price.
    pub offer: Option<Offer>,
}

/// How often a perpetual distribution pays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Every {
    Blocks(u64),
    Millis(u64),
    Epochs(u16),
}

/// A perpetual distribution this identity may claim (T-2.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Perpetual {
    pub every: Every,
    /// What a claim pays at least, counted as Platform counts it; `None`: not known before the
    /// claim (K1u).
    pub at_least: Option<TokenAmount>,
    /// One claim pays at most this many cycles and more stays after it.
    pub capped: Option<u32>,
}

/// A pre-programmed distribution naming this identity (T-2.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Scheduled {
    /// The payout a claim pays now: when it was due, and how much.
    pub due: Option<(Timestamp, TokenAmount)>,
    /// Payouts due after it, each a claim of its own.
    pub more_due: u32,
    /// The next payout still to come.
    pub next_at: Option<Timestamp>,
}

/// What this identity may claim of a token.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Claims {
    pub perpetual: Option<Perpetual>,
    pub scheduled: Option<Scheduled>,
}

impl Claims {
    /// A claim pays now: a perpetual payout passed, or a scheduled one is due.
    pub fn any_due(&self, perpetual_due: bool) -> bool {
        perpetual_due || self.scheduled.is_some_and(|s| s.due.is_some())
    }
}

/// The issuer's price (T-2.4): per smallest unit of the token, in credits, from a count
/// (`tiers`, ascending; a single price is one tier from 1).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Offer {
    pub tiers: Vec<(TokenAmount, u64)>,
    /// One price for any count: no tiers shown.
    pub single: bool,
    /// When it was read, in milliseconds (R-22).
    pub read_ms: u64,
    /// How many units may still be made under the token's limit; `None`: no limit.
    pub left: Option<TokenAmount>,
}

/// What a count costs by an [`Offer`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quote {
    /// `total` credits: `per_unit` each, by the tier from `tier` units.
    Total {
        total: u64,
        per_unit: u64,
        tier: TokenAmount,
    },
    /// Fewer than the first tier: at least `minimum` units (refused before the password).
    UnderMinimum { minimum: TokenAmount },
    /// No count.
    Nothing,
    /// A total over what credits can hold.
    TooLarge,
}

/// What `count` units cost by `tiers` (АРХ-10): the largest tier not over the count sets the
/// price of each unit, as Platform counts it.
pub fn quote(tiers: &[(TokenAmount, u64)], count: TokenAmount) -> Quote {
    if count == 0 {
        return Quote::Nothing;
    }
    let Some((tier, per_unit)) = tiers.iter().rev().find(|(from, _)| *from <= count).copied()
    else {
        return Quote::UnderMinimum {
            minimum: tiers.first().map_or(1, |(from, _)| *from),
        };
    };
    match per_unit.checked_mul(count) {
        Some(total) => Quote::Total {
            total,
            per_unit,
            tier,
        },
        None => Quote::TooLarge,
    }
}

/// How old a price may be at the confirmation (R-22): older, it is read again.
pub const PRICE_FRESH_MS: u64 = 60_000;

/// Whether a price read at `read_ms` is read again at `now_ms` before the confirmation.
pub fn stale(read_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(read_ms) > PRICE_FRESH_MS
}

/// Credits in one duff (10¹¹ credits are 1 DASH).
pub const CREDITS_PER_DUFF: u64 = 1_000;

/// A purchase's total in duffs, for FEAT-0004's confirmation threshold (T-2.6): what leaves the
/// wallet, rounded up.
pub fn purchase_total_duffs(credits: u64) -> u64 {
    credits.div_ceil(CREDITS_PER_DUFF)
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
    /// The token page of the token asked for ([`crate::Command::ReadTokenPage`]), once read
    /// (FEAT-0019 T-3.2).
    pub page: Option<TokenPage>,
    /// The page's last reads failed (BUG-222): of which token, and how many times in a row.
    /// The count grows with each failure, so the page tells a new one from one it has seen.
    pub page_failed: Option<TokenPageFailed>,
}

/// A token page that could not be read (BUG-222).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TokenPageFailed {
    pub token_id: String,
    pub times: u32,
}

impl Tokens {
    /// No tokens and no search (mainnet, or before the core reads anything).
    pub fn off() -> Self {
        Tokens {
            rows: Vec::new(),
            search: TokenSearch::off(),
            hidden: Vec::new(),
            page: None,
            page_failed: None,
        }
    }
}

/// An issuer action of the token page (UC-9…UC-14).
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub enum IssuerAction {
    Mint,
    Burn,
    Freeze,
    Unfreeze,
    DestroyFrozen,
    /// Pause and resume.
    Pause,
    Price,
}

/// What the shown identity may do with an action or a setting (AC-T13).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Right {
    /// Alone: the button.
    Alone,
    /// Only with its group: «needs the group's signatures: `needed` of `of`», no button.
    Group { needed: u32, of: u32 },
    /// Nothing: not shown.
    No,
}

/// Who a rule names, in words the screen says.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Taker {
    NoOne,
    ContractOwner,
    /// An identity (base58), with its `@name` when known.
    Identity(String),
    MainGroup,
    Group(u16),
}

/// A field of the settings window (M7, the architect's Q4 table).
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub enum SettingField {
    Names,
    MaxSupply,
    NewTokensTo,
    ChooseDestination,
    TradeMode,
    /// Who takes an action.
    Who(IssuerAction),
}

/// One field of the settings window.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SettingView {
    pub field: SettingField,
    /// Who may change its value (for [`SettingField::Who`], who takes the action).
    pub makers: Taker,
    /// Who may change `makers`.
    pub admins: Taker,
    /// The shown identity may change the value.
    pub change: Right,
    /// The shown identity may change who may.
    pub change_who: Right,
    /// Locked for good: «nobody» was chosen at creation.
    pub never: bool,
    /// Handing `makers` to «nobody» is allowed by the rule.
    pub to_no_one_allowed: bool,
}

/// «Check» under the frozen balances (BUG-222 QA): what was typed, as typed, and the answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FrozenCheck {
    pub asked: String,
    pub answer: FrozenAnswer,
}

/// The answer to «Check»: proved either way, or why there is none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrozenAnswer {
    Frozen,
    NotFrozen,
    /// An @name no identity has.
    NoSuchName,
    /// Neither an @name nor an identity id.
    NotAnIdentity,
}

/// The token page of a token this identity manages (T-3.2, M1).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TokenPage {
    pub token_id: String,
    /// In circulation, proved.
    pub supply: TokenAmount,
    /// The limit; `None`: none.
    pub max_supply: Option<TokenAmount>,
    /// The contract's owner (base58) and whether it is the shown identity.
    pub owner: String,
    pub owner_is_me: bool,
    /// The actions this identity has some part in; one it has none in is left out (R-15).
    pub actions: Vec<(IssuerAction, Right)>,
    /// The token is paused now, proved.
    pub paused: bool,
    /// Whether the minter chooses where new tokens go, and where they go otherwise.
    pub choose_destination: bool,
    pub new_tokens_to: Option<String>,
    /// The token may be traded (its marketplace trade mode), as the fresh contract says.
    pub tradeable: bool,
    /// Its names, a language each, as the fresh contract's conventions say them.
    pub names: Vec<TokenName>,
    /// Frozen balances among those the wallet knows of, proved (Q2).
    pub frozen: Vec<String>,
    /// The identity asked about by «Check» on this read, and the answer (BUG-222 QA).
    pub checked: Option<FrozenCheck>,
    pub settings: Vec<SettingView>,
    /// When it was read, in milliseconds.
    pub read_ms: u64,
}

impl TokenPage {
    /// «Management» is shown (AC-D8).
    pub fn manages(&self) -> bool {
        !self.actions.is_empty()
            || self
                .settings
                .iter()
                .any(|s| s.change != Right::No || s.change_who != Right::No)
    }

    /// The shown identity's right to `action`.
    pub fn right(&self, action: IssuerAction) -> Right {
        self.actions
            .iter()
            .find(|(a, _)| *a == action)
            .map_or(Right::No, |(_, r)| *r)
    }
}

/// A token's name in one language, as its conventions keep it: the forms for one and for many.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TokenName {
    pub lang: String,
    pub one: String,
    pub many: String,
    /// The issuer's hint to write it with a capital letter.
    pub capitalize: bool,
}

/// Why names cannot be written, as Platform would refuse them with a fee
/// (`TokenConfigurationConvention::validate_localizations` v0, 575d961).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NamesProblem {
    /// No English name: Platform requires one.
    NoEnglish,
    /// Not a language code Platform takes (here: two or three letters, then at most a script of
    /// four letters and a region of two, «zh-Hans», «pt-BR» — fewer than Platform takes, never
    /// more).
    Language(String),
    /// The same language twice.
    Twice(String),
    /// A form not 3 to 25 bytes long.
    Length(String),
    /// A form with a space or a control character.
    Spaces(String),
}

/// Whether `code` is a language code in the form [`NamesProblem::Language`] says.
fn language_code(code: &str) -> bool {
    let mut parts = code.split('-');
    let letters = |s: &str, n: std::ops::RangeInclusive<usize>| {
        n.contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphabetic())
    };
    if !parts.next().is_some_and(|l| letters(l, 2..=3)) {
        return false;
    }
    let rest: Vec<&str> = parts.collect();
    match rest.as_slice() {
        [] => true,
        [one] => letters(one, 4..=4) || letters(one, 2..=2),
        [script, region] => letters(script, 4..=4) && letters(region, 2..=2),
        _ => false,
    }
}

/// What refuses `names` before the password, the first found; `None`: Platform takes them.
pub fn names_problem(names: &[TokenName]) -> Option<NamesProblem> {
    if !names.iter().any(|n| n.lang == "en") {
        return Some(NamesProblem::NoEnglish);
    }
    let mut seen = std::collections::BTreeSet::new();
    for n in names {
        if !language_code(&n.lang) {
            return Some(NamesProblem::Language(n.lang.clone()));
        }
        if !seen.insert(n.lang.as_str()) {
            return Some(NamesProblem::Twice(n.lang.clone()));
        }
        for form in [&n.one, &n.many] {
            if form.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Some(NamesProblem::Spaces(form.clone()));
            }
            if !(3..=25).contains(&form.len()) {
                return Some(NamesProblem::Length(form.clone()));
            }
        }
    }
    None
}

/// A change of one setting (T-3.8).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SettingChange {
    MaxSupply(Option<TokenAmount>),
    NewTokensTo(Option<String>),
    ChooseDestination(bool),
    /// Tradeable or not.
    TradeMode(bool),
    /// The names, a language each; the decimals stay as they are (АРХ-11).
    Names(Vec<TokenName>),
    /// Who may change the field's value (for `Who`, who takes the action).
    Makers(SettingField, Taker),
    /// Who may change who may.
    Admins(SettingField, Taker),
}

impl SettingChange {
    /// Hands a rule to «nobody»: for good, so the window asks for the tick (AC-T14).
    pub fn to_no_one(&self) -> bool {
        matches!(
            self,
            SettingChange::Makers(_, Taker::NoOne) | SettingChange::Admins(_, Taker::NoOne)
        )
    }
}

/// What an issuer action did, as History says it (T-3.10, the analyst's strings); amounts in
/// words, as other token lines.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TokenAct {
    Minted { amount: String },
    Burned { amount: String },
    Frozen { who: String },
    Unfrozen { who: String },
    Destroyed { who: String },
    Paused,
    Resumed,
    PriceChanged,
    SettingsChanged,
}

/// An issuer action, as the screen asks for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum IssuerOp {
    /// `to`: the recipient (base58 or `@name`), when the token lets the minter choose.
    Mint {
        to: Option<String>,
        amount: TokenAmount,
    },
    Burn {
        amount: TokenAmount,
    },
    Freeze {
        who: String,
    },
    Unfreeze {
        who: String,
    },
    DestroyFrozen {
        who: String,
    },
    Pause,
    Resume,
    /// `None`: not sold any more; else tiers as [`Offer::tiers`].
    Price(Option<Vec<(TokenAmount, u64)>>),
    Setting(SettingChange),
}

impl IssuerOp {
    /// Its action, for the rights.
    pub fn action(&self) -> Option<IssuerAction> {
        Some(match self {
            IssuerOp::Mint { .. } => IssuerAction::Mint,
            IssuerOp::Burn { .. } => IssuerAction::Burn,
            IssuerOp::Freeze { .. } => IssuerAction::Freeze,
            IssuerOp::Unfreeze { .. } => IssuerAction::Unfreeze,
            IssuerOp::DestroyFrozen { .. } => IssuerAction::DestroyFrozen,
            IssuerOp::Pause | IssuerOp::Resume => IssuerAction::Pause,
            IssuerOp::Price(_) => IssuerAction::Price,
            IssuerOp::Setting(_) => return None,
        })
    }

    /// Cannot be undone: the window asks for «I understand» (R-23): burn, destroy frozen, pause,
    /// a rule handed to «nobody».
    pub fn needs_tick(&self) -> bool {
        match self {
            IssuerOp::Burn { .. } | IssuerOp::DestroyFrozen { .. } | IssuerOp::Pause => true,
            IssuerOp::Setting(change) => change.to_no_one(),
            _ => false,
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
            claims: None,
            offer: None,
        }
    }

    /// AC-T12: a single price per unit; tiers: the largest not over the count; under the first,
    /// the minimum; an overflow refused, never wrapped.
    #[test]
    fn a_count_costs_by_its_tier() {
        assert_eq!(
            quote(&[(1, 1_000)], 250),
            Quote::Total {
                total: 250_000,
                per_unit: 1_000,
                tier: 1
            }
        );
        assert_eq!(quote(&[(1, 1_000)], 0), Quote::Nothing);
        let tiers = [(100, 50), (1_000, 40), (10_000, 30)];
        assert_eq!(quote(&tiers, 99), Quote::UnderMinimum { minimum: 100 });
        let at = |count| match quote(&tiers, count) {
            Quote::Total {
                total,
                per_unit,
                tier,
            } => (tier, per_unit, total),
            other => panic!("{other:?}"),
        };
        assert_eq!(at(100), (100, 50, 5_000), "right at the first tier");
        assert_eq!(at(999), (100, 50, 49_950));
        assert_eq!(at(1_000), (1_000, 40, 40_000));
        assert_eq!(at(50_000), (10_000, 30, 1_500_000), "over the last tier");
        assert_eq!(quote(&[(1, u64::MAX / 2)], 3), Quote::TooLarge);
    }

    /// R-22 and T-2.6: a price over a minute old is read again; the total in duffs rounds up.
    #[test]
    fn a_price_ages_and_a_total_is_in_duffs() {
        assert!(!stale(1_000, 61_000));
        assert!(stale(1_000, 61_001));
        assert!(!stale(5_000, 1_000), "a clock gone back is not stale");
        assert_eq!(purchase_total_duffs(20_000_000_000), 20_000_000, "0.2 DASH");
        assert_eq!(purchase_total_duffs(1), 1);
        assert_eq!(purchase_total_duffs(0), 0);
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

#[cfg(test)]
mod names_tests {
    use super::*;

    fn name(lang: &str, one: &str, many: &str) -> TokenName {
        TokenName {
            lang: lang.into(),
            one: one.into(),
            many: many.into(),
            capitalize: true,
        }
    }

    /// Names as dpp's `validate_localizations` v0 takes or refuses them.
    #[test]
    fn names_platform_would_refuse() {
        let en = name("en", "Coffee", "Coffees");
        assert_eq!(names_problem(std::slice::from_ref(&en)), None);
        assert_eq!(
            names_problem(&[
                en.clone(),
                name("ru", "Кофе", "Кофе"),
                name("zh-Hans", "咖啡币", "咖啡币")
            ]),
            None
        );
        assert_eq!(
            names_problem(&[name("pt-BR", "Café", "Cafés")]),
            Some(NamesProblem::NoEnglish)
        );
        assert_eq!(
            names_problem(&[en.clone(), name("e", "Kaffee", "Kaffees")]),
            Some(NamesProblem::Language("e".into()))
        );
        assert_eq!(
            names_problem(&[en.clone(), name("de-abc", "Kaffee", "Kaffees")]),
            Some(NamesProblem::Language("de-abc".into()))
        );
        assert_eq!(
            names_problem(&[en.clone(), en.clone()]),
            Some(NamesProblem::Twice("en".into()))
        );
        assert_eq!(
            names_problem(&[name("en", "Evo Coffee", "Coffees")]),
            Some(NamesProblem::Spaces("Evo Coffee".into()))
        );
        assert_eq!(
            names_problem(&[name("en", "Ab", "Coffees")]),
            Some(NamesProblem::Length("Ab".into()))
        );
        // Bytes, not letters: 13 Cyrillic letters are 26 bytes.
        assert_eq!(
            names_problem(&[en, name("ru", "Кофекофекофек", "Кофе")]),
            Some(NamesProblem::Length("Кофекофекофек".into()))
        );
    }
}
