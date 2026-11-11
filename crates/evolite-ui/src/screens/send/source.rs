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
    /// The balance is read in this run: Core is (its own rules are `core_as_of`), credits once
    /// `credits_read`, the pool once `checked_at` (BUG-113). Unread, `duffs` is no amount to
    /// refuse or fill in by.
    pub(super) read: bool,
    pub(super) note_icon: &'static str,
    pub(super) note: &'static str,
}

/// The smallest withdrawal of credits to Core: Platform's own minimum, 1000 duffs
/// (`system_limits.min_withdrawal_amount`, 1 000 000 credits), plus the Core fee of the asset
/// unlock, 190 duffs (190 bytes at platform-wallet's 1 duff a byte), which Platform takes out of
/// the amount since protocol 14 (platform 575d961, `rs-dpp/src/withdrawal/mod.rs`).
/// It assumes `core_fee_per_byte = 1` (platform-wallet `withdrawal.rs:104`): should evolite pass
/// its own rate, count the minimum with `min_withdrawal_amount_with_core_fee` instead.
const MIN_WITHDRAWAL: Duffs = 1_190;

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
fn keep_and_min(fee: Duffs, withdrawal: Option<Withdrawal>) -> (Duffs, Duffs) {
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
    !(to.is_empty() || to.starts_with('@') || credits_to_platform_address(to))
}

/// Credits to `to` go to a Platform address, whose transfer costs more than one to a name (BUG-108).
fn credits_to_platform_address(to: &str) -> bool {
    to.trim().to_ascii_lowercase().starts_with("tdash1")
}

/// What credits to `to` cost: a withdrawal to a Core address, else a transfer to a Platform address
/// or a name.
pub(super) fn credits_fee(fees: &evolite_api::Fees, to: &str) -> Duffs {
    if credits_to_core(to) {
        fees.credit_withdrawal
    } else if credits_to_platform_address(to) {
        fees.credit_transfer_to_address
    } else {
        fees.credit_transfer
    }
}

/// `to`: the recipient typed, which decides what a payment from credits or the pool costs;
/// `amount`: the amount typed (0: none yet), which decides what a payment from Core costs.
pub(super) fn sources(w: &Wallet, to: &str, amount: Duffs) -> Vec<SourceView> {
    sources_on(evolite_core::network::current(), w, to, amount)
}

/// [`sources`] on `network`: on mainnet without «Platform address» until its payments are proven
/// live on testnet (FEAT-0015 review r1, Must 1).
pub(super) fn sources_on(
    network: evolite_api::Network,
    w: &Wallet,
    to: &str,
    amount: Duffs,
) -> Vec<SourceView> {
    let credits = credits_fee(&w.fees, to);
    let pool = super::super::shielded::pool_fee(&w.fees, to);
    let (address, address_max) = address_fees(w, to, amount);
    let mut all = vec![
        SourceView {
            source: Source::Core,
            icon: "core",
            title: "Core",
            duffs: w.balances.core,
            fee: w.fees.core_payment_for(amount),
            max_fee: w.fees.core_payment,
            withdrawal: None,
            read: true,
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
            read: w.balances.credits_checked,
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
            read: w.pool.checked_at.is_some(),
            note_icon: "shield",
            note: tr!(
                "From the pool: the amount and the recipient are not visible on the network."
            ),
        },
        // FEAT-0015: what the addresses do is chosen by the recipient; public, unlike the pool
        // (R-10).
        SourceView {
            source: Source::PlatformAddress,
            icon: "platform-address",
            title: tr!("Platform address"),
            duffs: w.balances.platform_address,
            fee: address,
            max_fee: address_max,
            withdrawal: None,
            // Not checked in this run: its last known balance, dimmed (BUG-113), as the others.
            read: w.balances.platform_address_read,
            note_icon: "eye",
            note: tr!(
                "Amounts on a Platform address are visible to everyone. For privacy, use the private pool."
            ),
        },
    ];
    all.retain(|s| shown_on(network, s.source));
    all
}

/// Whether `source` is offered on `network`: «Platform address» not on mainnet until its payments
/// are proven live on testnet (FEAT-0015 review r1, Must 1). One place to change then.
pub(super) fn shown_on(network: evolite_api::Network, source: Source) -> bool {
    !(network == evolite_api::Network::Mainnet && source == Source::PlatformAddress)
}

/// What a payment of `amount` duffs from the Platform addresses to `to` costs, by the plan the
/// core signs (FEAT-0015 R-7), and the fee «Max» leaves room for: the balance less what «Max»
/// gets. With no plan possible (nothing typed, too little): the fee of one address paying.
pub(super) fn address_fees(w: &Wallet, to: &str, amount: Duffs) -> (Duffs, Duffs) {
    use evolite_api::address_plan::{CREDITS_PER_DUFF, address_plan, duffs_up, move_for};
    let kind = move_for(to);
    let credits = (amount.max(0) as u64).saturating_mul(CREDITS_PER_DUFF);
    let fee = address_plan(kind, credits, false, &w.platform_inputs)
        .map_or_else(|_| duffs_up(kind.fee(1)), |plan| duffs_up(plan.fee));
    let max_fee = address_plan(kind, 0, true, &w.platform_inputs).map_or(fee, |plan| {
        w.balances.platform_address - (plan.receives / CREDITS_PER_DUFF) as Duffs
    });
    (fee, max_fee)
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

/// What a «From» card says: a balance not read yet is not a 0 — with nothing kept from before it
/// is not known (BUG-91, BUG-113, BUG-115); kept, it is shown dimmed ([`source_dim`]).
pub(super) fn source_line(
    s: &SourceView,
    as_of: Option<evolite_api::CoreAsOf>,
    line: Option<&str>,
) -> String {
    if !s.read && s.duffs == 0 {
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
    let mut column = from_columns(width, cards.len(), widest(|c| c.words));
    // Four sources: one row of four, or two rows of two — never three and one (FEAT-0015
    // AC-D10).
    let per_row = ((width + 8.0) / (column + 8.0)).floor() as usize;
    if cards.len() == 4 && per_row == 3 {
        column = ((width - 8.0) / 2.0).floor();
    }
    let icon = column >= widest(|c| c.with_icon);
    cards
        .iter()
        .map(|_| CardColumn {
            width: column,
            icon,
        })
        .collect()
}

/// Whether a «From» card shows its amount dimmed: the last known one, not read in this run.
pub(super) fn source_dim(s: &SourceView) -> bool {
    !s.read && s.duffs > 0
}

/// The one line under the row of sources: Core's last check, then credits and the pool kept from
/// before and not read yet, in one line (BUG-113, BUG-115).
pub(super) fn sources_caption(
    as_of: Option<evolite_api::CoreAsOf>,
    line: Option<&str>,
    all: &[SourceView],
) -> Option<String> {
    let kept = |source| all.iter().any(|s| s.source == source && source_dim(s));
    let pool = match (kept(Source::Credits), kept(Source::Pool)) {
        (true, true) => Some(tr!(
            "Credits and the pool: the last known balance, still checking"
        )),
        (true, false) => Some(tr!("Credits: the last known balance, still checking")),
        (false, true) => Some(tr!("The pool: the last known balance, still checking")),
        (false, false) => None,
    }
    .map(str::to_owned);
    match (as_of_caption(as_of, line), pool) {
        (Some(core), Some(pool)) => Some(format!("{core} · {pool}")),
        (core, pool) => core.or(pool),
    }
}

/// What is wrong with the amount typed from `s`, as far as the wallet can tell now.
pub(super) fn form_problem(typed: &str, s: &SourceView, synced: bool) -> Option<AmountProblem> {
    use AmountProblem::{MoreThanBalance, NoFunds, NoRoomForFee};
    amount_problem(typed, s.duffs, s.fee, s.withdrawal)
        .filter(|p| synced || *p != NoFunds)
        // Over a balance not read yet nothing is known to be too much (BUG-113).
        .filter(|p| s.read || !matches!(p, NoFunds | MoreThanBalance | NoRoomForFee))
}

/// Where a payment from the pool goes, by the recipient typed (BUG-112).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PoolRoute {
    /// An @name: not from the pool (no private address in a DashPay profile).
    Name,
    /// An evolite private address: a transfer inside the pool.
    Private,
    /// A Platform address: out of the pool, the amount becomes visible.
    Platform,
    /// A Core address: a withdrawal.
    Core,
}

/// [`PoolRoute`] of the recipient typed; none while nothing is typed.
pub(super) fn pool_route(to: &str) -> Option<PoolRoute> {
    let to = to.trim();
    if to.is_empty() {
        None
    } else if to.starts_with('@') {
        Some(PoolRoute::Name)
    } else if to.to_ascii_lowercase().starts_with("tdash1") {
        Some(if super::super::shielded::is_private_address(to) {
            PoolRoute::Private
        } else {
            PoolRoute::Platform
        })
    } else {
        Some(PoolRoute::Core)
    }
}

/// The line under the form about where the money goes and what the network sees: its icon and
/// its words, or none.
pub(super) fn route_line(s: &SourceView, to: &str) -> Option<(&'static str, &'static str)> {
    let to = to.trim();
    // From the pool only a private address keeps the amount and the recipient hidden (BUG-112).
    if s.source == Source::Pool {
        return match pool_route(to)? {
            PoolRoute::Name => None,
            PoolRoute::Private => Some((
                "shielded",
                tr!(
                    "Private address: the amount and the recipient will not be visible on the network."
                ),
            )),
            PoolRoute::Platform => Some((
                "eye",
                tr!(
                    "The unshielded amount becomes visible on the network, like any transfer to a Platform address."
                ),
            )),
            PoolRoute::Core => Some((
                "pending",
                tr!(
                    "Withdrawal from the pool to Core: the amount becomes visible on the network; the DASH arrive at this address in a few minutes, when Platform releases a batch of withdrawals."
                ),
            )),
        };
    }
    let withdrawal = s.source == Source::Credits
        && !to.is_empty()
        && !to.starts_with('@')
        && !to.starts_with("tdash1");
    Some(if withdrawal {
        (
            "pending",
            tr!(
                "Withdrawing credits to Core: the DASH arrive at this address in a few minutes, when Platform releases a batch of withdrawals."
            ),
        )
    } else {
        (s.note_icon, s.note)
    })
}
