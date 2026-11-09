//! Where the money comes from: the three sources, what each can send, the limits of an amount.

use super::*;

/// A balance to pay from, as the Send screen shows it.
pub(super) struct SourceView {
    pub(super) source: Source,
    pub(super) icon: &'static str,
    pub(super) title: &'static str,
    pub(super) duffs: Duffs,
    /// The fee of the amount typed (BUG-55: from Core, of the coins it takes).
    pub(super) fee: Duffs,
    /// The fee "Max" leaves room for: from Core, spending every coin.
    pub(super) max_fee: Duffs,
    /// Credits to a Core address: what Platform checks before it takes the withdrawal.
    pub(super) withdrawal: Option<Withdrawal>,
    pub(super) note_icon: &'static str,
    pub(super) note: &'static str,
}

/// The smallest withdrawal of credits to Core: Platform's own minimum, 1000 duffs
/// (`system_limits.min_withdrawal_amount`, 1 000 000 credits), plus the Core fee of the asset
/// unlock, 190 duffs (190 bytes at platform-wallet's 1 duff a byte), which Platform takes out of
/// the amount since protocol 14 (platform 575d961, `rs-dpp/src/withdrawal/mod.rs`).
/// It assumes `core_fee_per_byte = 1` (platform-wallet `withdrawal.rs:104`): should evolite pass
/// its own rate, count the minimum with `min_withdrawal_amount_with_core_fee` instead.
pub(super) const MIN_WITHDRAWAL: Duffs = 1_190;

/// What Platform checks before it takes a withdrawal of credits to Core.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Withdrawal {
    /// What must stay on the balance on top of the amount. Platform refuses a balance under
    /// the amount plus its minimum fee (`Fees::credit_withdrawal_reserve`, 0.004 DASH), and the
    /// fee itself (`Fees::credit_withdrawal`, what the last one cost) comes out of the balance
    /// too: the larger of the two.
    pub(super) reserve: Duffs,
    /// The smallest amount.
    pub(super) min: Duffs,
}

impl Withdrawal {
    pub(super) fn of(fees: &evolite_api::Fees) -> Self {
        Self {
            reserve: fees.credit_withdrawal.max(fees.credit_withdrawal_reserve),
            min: MIN_WITHDRAWAL,
        }
    }
}

/// What is wrong with the amount typed for a balance and its fee: the line under the field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum AmountProblem {
    /// Something there, but not even the fee (for a withdrawal, the reserve and the smallest
    /// amount): nothing can be sent from it, whatever is typed. An empty balance says nothing
    /// until an amount is typed (then "more than there is"), and neither does a fee not known
    /// yet.
    NoFunds,
    /// Not an amount, or zero.
    NotAnAmount,
    /// More digits after the decimal mark than the unit has.
    TooPrecise,
    /// More than the balance.
    MoreThanBalance,
    /// A withdrawal under the smallest one.
    BelowMinimum,
    /// Within the balance, but not with the fee (for a withdrawal, the reserve) on top.
    NoRoomForFee,
}

/// What stays on the balance on top of the amount, and the smallest amount.
pub(super) fn keep_and_min(fee: Duffs, withdrawal: Option<Withdrawal>) -> (Duffs, Duffs) {
    match withdrawal {
        Some(w) => (w.reserve, w.min.max(1)),
        None => (fee, 1),
    }
}

pub(super) fn amount_problem(
    typed: &str,
    balance: Duffs,
    fee: Duffs,
    withdrawal: Option<Withdrawal>,
) -> Option<AmountProblem> {
    let (keep, min) = keep_and_min(fee, withdrawal);
    if balance > 0 && keep > 0 && balance < keep.saturating_add(min) {
        return Some(AmountProblem::NoFunds);
    }
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    match crate::format::parse_amount(typed) {
        None if crate::format::too_precise(typed) => Some(AmountProblem::TooPrecise),
        None | Some(0) => Some(AmountProblem::NotAnAmount),
        Some(d) if d > balance => Some(AmountProblem::MoreThanBalance),
        Some(d) if d < min => Some(AmountProblem::BelowMinimum),
        Some(d) if crate::format::left_after(balance, d, keep) < 0 => {
            Some(AmountProblem::NoRoomForFee)
        }
        Some(_) => None,
    }
}

pub(super) fn amount_problem_text(
    problem: AmountProblem,
    fee: Duffs,
    withdrawal: Option<Withdrawal>,
) -> String {
    let (keep, min) = keep_and_min(fee, withdrawal);
    let keep = crate::format::amount(keep);
    match (problem, withdrawal) {
        (AmountProblem::NoFunds, Some(_)) => format!(
            tr!("Not enough to withdraw: the balance must keep at least {} on top of the amount"),
            keep
        ),
        (AmountProblem::NoFunds, None) => {
            format!(tr!("This balance does not cover even the fee ({})"), keep)
        }
        (AmountProblem::NotAnAmount, _) => tr!("Enter an amount, e.g. 0.25").into(),
        (AmountProblem::TooPrecise, _) => format!(
            tr!("Too many digits after the decimal mark: the smallest amount is {}"),
            crate::format::amount(1)
        ),
        (AmountProblem::MoreThanBalance, _) => tr!("More than there is in this balance").into(),
        (AmountProblem::BelowMinimum, _) => format!(
            tr!("The smallest withdrawal is {}"),
            crate::format::amount(min)
        ),
        (AmountProblem::NoRoomForFee, Some(_)) => {
            format!(
                tr!("Not enough for the amount and the fee reserve ({})"),
                keep
            )
        }
        (AmountProblem::NoRoomForFee, None) => {
            format!(tr!("Not enough for the amount and the fee ({})"), keep)
        }
    }
}

/// What "Max" puts in the field: the balance less the fee (for a withdrawal, the reserve).
pub(super) fn max_amount(balance: Duffs, fee: Duffs, withdrawal: Option<Withdrawal>) -> Duffs {
    let (keep, _) = keep_and_min(fee, withdrawal);
    balance.saturating_sub(keep).max(0)
}

/// Credits to `to` go to a Core address — a withdrawal, which Platform pays out later — and not
/// to a name or a Platform address (a transfer).
pub(super) fn credits_to_core(to: &str) -> bool {
    let to = to.trim();
    !(to.is_empty() || to.starts_with('@') || to.to_ascii_lowercase().starts_with("tdash1"))
}

/// What credits to `to` cost: a withdrawal to a Core address, else a transfer.
pub(super) fn credits_fee(fees: &evolite_api::Fees, to: &str) -> Duffs {
    if credits_to_core(to) {
        fees.credit_withdrawal
    } else {
        fees.credit_transfer
    }
}

/// `to`: the recipient typed, which decides what a payment from credits or the pool costs;
/// `amount`: the amount typed (0: none yet), which decides what a payment from Core costs.
pub(super) fn sources(w: &Wallet, to: &str, amount: Duffs) -> [SourceView; 3] {
    let credits = credits_fee(&w.fees, to);
    let pool = super::super::shielded::pool_fee(&w.fees, to);
    [
        SourceView {
            source: Source::Core,
            icon: "core",
            title: "Core",
            duffs: w.balances.core,
            fee: w.fees.core_payment_for(amount),
            max_fee: w.fees.core_payment,
            withdrawal: None,
            note_icon: "instantsend",
            note: tr!("InstantSend: the recipient sees a confirmed payment in a few seconds."),
        },
        SourceView {
            source: Source::Credits,
            icon: "credits",
            title: tr!("Credits"),
            duffs: w.balances.credits,
            fee: credits,
            max_fee: credits,
            withdrawal: credits_to_core(to).then(|| Withdrawal::of(&w.fees)),
            note_icon: "confirmed",
            note: tr!("A credit transfer is confirmed by a Platform proof in a couple of seconds."),
        },
        SourceView {
            source: Source::Pool,
            icon: "shielded",
            title: tr!("Private pool"),
            duffs: w.balances.shielded,
            fee: pool,
            max_fee: pool,
            withdrawal: None,
            note_icon: "shield",
            note: tr!(
                "From the pool: the amount and the recipient are not visible on the network."
            ),
        },
    ]
}

/// The line under a source's title: its balance; for Core not known yet in this sync, only that
/// (BUG-77). «As of when» for a last checked Core balance goes under the row
/// ([`as_of_caption`]), so a long one does not push the third card to a second row (BUG-67).
pub(super) fn source_detail(
    s: &SourceView,
    as_of: Option<evolite_api::CoreAsOf>,
    line: Option<&str>,
) -> String {
    match (line, s.source) {
        (Some(line), Source::Core) if as_of == Some(evolite_api::CoreAsOf::NotYet) => {
            line.to_owned()
        }
        _ => crate::format::balance(s.duffs),
    }
}

/// Under the row of sources: the Core balance on its card is the last checked one, and since
/// when (BUG-77, BUG-67). None when Core is checked in this sync or not known yet.
pub(super) fn as_of_caption(
    as_of: Option<evolite_api::CoreAsOf>,
    line: Option<&str>,
) -> Option<String> {
    match (as_of, line) {
        (Some(evolite_api::CoreAsOf::Checked(_)), Some(line)) => Some(format!("Core · {line}")),
        _ => None,
    }
}

/// What a «From» card says: credits not read yet are not a 0 (BUG-91).
pub(super) fn source_line(
    s: &SourceView,
    credits_read: bool,
    as_of: Option<evolite_api::CoreAsOf>,
    line: Option<&str>,
) -> String {
    if s.source == Source::Credits && !credits_read {
        return tr!("Not known yet").to_owned();
    }
    source_detail(s, as_of, line)
}

/// What a «From» card needs: the width of its words alone and with its icon
/// ([`option_card_width`]).
#[derive(Clone, Copy, Debug)]
pub(super) struct CardWidths {
    pub(super) words: f32,
    pub(super) with_icon: f32,
}

/// The column each «From» card stands in, in `width`: equal columns that hold the widest words
/// ([`from_columns`]), and the icons on every card or on none (the designer, BUG-81 r2) — on
/// all when the column holds the widest card with its icon. The columns are equal, so one
/// answer for the panel is one for each row, two plus one included.
pub(super) fn from_layout(width: f32, cards: &[CardWidths]) -> Vec<CardColumn> {
    let widest = |f: fn(&CardWidths) -> f32| cards.iter().map(f).fold(0.0, f32::max);
    let column = from_columns(width, cards.len(), widest(|c| c.words));
    let icon = column >= widest(|c| c.with_icon);
    cards
        .iter()
        .map(|_| CardColumn {
            width: column,
            icon,
        })
        .collect()
}
