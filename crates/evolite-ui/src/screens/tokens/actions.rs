//! «Claim» and «Buy» (FEAT-0019 T-2; the designer's K1, K1u, P1, P2, P1S): the windows, and the
//! rules that refuse before the password. A row shows «Claim» only with a proved claim this
//! identity may make, «Buy» only with the issuer's proved price (R-15, R-21, AC-D1).

use super::{TokensState, issuer, sender, shown_name, state};
use crate::components::{
    Assets, ButtonKind, FieldKind, Part, button, button_enabled, button_row, button_row_spend,
    card, dialog_with_footer, divider, field_label, kv_row, notice, password_asked, text_field,
    txt,
};
use crate::context::Ctx;
use crate::theme::{Weight, mono, sans};
use egui::{Margin, Sense, Ui};
use evolite_api::tokens::{
    Claims, Every, Offer, Quote, TokenAmount, TokenInfo, TokenRow, purchase_total_duffs, quote,
    stale,
};
use evolite_api::{Command, CoreError, Spend};

/// Credits in one DASH (10¹¹).
const CREDITS_PER_DASH: u128 = 100_000_000_000;

/// Whether a row offers «Claim»: a perpetual distribution to this identity, or a pre-programmed
/// payout due to it (R-15, R-20).
pub fn can_claim(row: &TokenRow) -> bool {
    row.claims
        .is_some_and(|c| c.perpetual.is_some() || c.scheduled.is_some_and(|s| s.due.is_some()))
}

/// What the claim window warns of (the architect's correction to T-3.6): on a proved pause the
/// claim goes — Platform 575d961 checks the pause on a plain claim no more than on a mint — but
/// what is claimed cannot be sent or sold until the token is resumed. A claim to the pool would
/// be refused by the network on a pause; evolite has no such path.
pub fn claim_warning(row: &TokenRow) -> Option<&'static str> {
    (row.paused == Some(true)).then_some(
        tr!(
            "The issuer paused the token: you can claim, but these tokens cannot be transferred or sold until it is resumed."
        ))
}

/// Whether a row offers «Buy»: the issuer sells it (R-21; not to the contract's owner, whose row
/// the core leaves without a price).
pub fn can_buy(row: &TokenRow) -> bool {
    row.offer.is_some()
}

/// Which distribution «Claim» takes: a pre-programmed payout due first (it is one payout, known
/// to the unit), else the perpetual one.
pub fn claims_scheduled(claims: &Claims) -> bool {
    claims.scheduled.is_some_and(|s| s.due.is_some())
}

/// Why a purchase cannot go, said under the button (AC-A9, the designer's P1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuyProblem {
    Paused,
    Frozen,
    /// No more may be made: the limit is reached, or `left` units only.
    Limit {
        left: TokenAmount,
    },
    /// Fewer than the first tier.
    UnderMinimum {
        minimum: TokenAmount,
    },
    NotAmount,
    /// The credits do not cover the total and the fee: what is needed, in duffs.
    NoCredits {
        need: u64,
    },
}

/// What refuses a purchase of `count` (`None`: not typed yet) before the password; `credits`
/// and `fee` in duffs.
pub fn buy_problem(
    row: &TokenRow,
    offer: &Offer,
    count: Option<TokenAmount>,
    credits: Option<u64>,
    fee: u64,
) -> Option<BuyProblem> {
    if row.paused == Some(true) {
        return Some(BuyProblem::Paused);
    }
    if row.frozen == Some(true) {
        return Some(BuyProblem::Frozen);
    }
    if offer.left == Some(0) {
        return Some(BuyProblem::Limit { left: 0 });
    }
    let count = count?;
    if let Some(left) = offer.left.filter(|left| count > *left) {
        return Some(BuyProblem::Limit { left });
    }
    match quote(&offer.tiers, count) {
        Quote::Nothing | Quote::TooLarge => Some(BuyProblem::NotAmount),
        Quote::UnderMinimum { minimum } => Some(BuyProblem::UnderMinimum { minimum }),
        Quote::Total { total, .. } => {
            let need = purchase_total_duffs(total).saturating_add(fee);
            credits
                .is_some_and(|have| have < need)
                .then_some(BuyProblem::NoCredits { need })
        }
    }
}

/// A price per whole token, in duffs, from a price per smallest unit in credits.
fn per_token_duffs(per_unit: u64, decimals: u8) -> i64 {
    let credits = (per_unit as u128).saturating_mul(10u128.pow(decimals.min(30) as u32));
    let duffs = credits / (CREDITS_PER_DASH / 100_000_000);
    duffs.min(i64::MAX as u128) as i64
}

/// «0.0008 DASH» for one whole token.
pub fn price_text(per_unit: u64, decimals: u8) -> String {
    crate::format::amount(per_token_duffs(per_unit, decimals))
}

/// How often a perpetual distribution pays, in words: «every 24 hours».
fn every_text(every: Every) -> String {
    match every {
        Every::Millis(ms) if ms % 3_600_000 == 0 => {
            crate::text::plural((ms / 3_600_000) as u32, tr!("hour"), tr!("hours"))
        }
        Every::Millis(ms) => {
            crate::text::plural((ms / 60_000).max(1) as u32, tr!("minute"), tr!("minutes"))
        }
        Every::Blocks(n) => crate::text::plural(n as u32, tr!("block"), tr!("blocks")),
        Every::Epochs(n) => crate::text::plural(n as u32, tr!("epoch"), tr!("epochs")),
    }
}

/// «Claim»: one token.
pub struct ClaimState {
    pub token_id: String,
    /// Its spend (TECH-spend-flow): the password, sent once, its own answer only.
    pub flow: crate::components::spend_flow::SpendFlow<()>,
    pub result: Option<Result<(), CoreError>>,
    /// What «Sign and claim» signed (BUG-179): shown while it runs and once done, not the next
    /// distribution reckoned on the new balances.
    pub signed: Option<Claims>,
}

impl ClaimState {
    pub fn new(token_id: &str) -> Self {
        ClaimState {
            token_id: token_id.to_owned(),
            flow: Default::default(),
            result: None,
            signed: None,
        }
    }

    /// `Done { what: ClaimToken }`: only an answer this window waits for.
    pub fn on_answer(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) {
        if !self.flow.on_done(ask, result) {
            return;
        }
        let _ = self.flow.take_result();
        self.result = Some(result.clone());
    }
}

/// «Buy»: one token.
pub struct BuyState {
    pub token_id: String,
    pub count: String,
    /// Its spend (TECH-spend-flow): the password, sent once, its own answer only.
    pub flow: crate::components::spend_flow::SpendFlow<()>,
    pub result: Option<Result<(), CoreError>>,
    /// The price per unit the last purchase agreed to, when the network said it grew (P2).
    pub was: Option<u64>,
    /// A fresh price was asked for (R-22), not answered yet.
    pub asked_price: bool,
    /// What «Sign and buy» signed (BUG-179): the price and the credits it was reckoned on.
    pub signed: Option<SignedBuy>,
    /// «Sign and buy» pressed on a price over a minute old (BUG-181): what it showed, while the
    /// price is read again.
    pub pending: Option<Pending>,
    /// The price could not be read again in time (BUG-181 r2, Should 1): nothing was sent.
    pub unchecked: bool,
    /// The price read again came dearer (BUG-181): the new total and the one shown; nothing was
    /// sent, the next «Sign and buy» is a knowing one.
    pub dearer: Option<(u64, u64)>,
}

/// «Sign and buy» pressed on an old price (BUG-181): the total shown, the count it was for
/// (r2, Should 2: another count typed meanwhile was never shown a total) and when, in ms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub total: u64,
    pub count: u64,
    pub at_ms: u64,
}

/// How long a price is waited for after «Sign and buy» on an old one (BUG-181 r2, Should 1).
pub const RECHECK_WAIT_MS: u64 = 30_000;

/// Whether `pending` still stands: the same count in the field, and not waited for longer than
/// [`RECHECK_WAIT_MS`]. `Err(true)`: given up for the time, said; `Err(false)`: the count
/// changed, dropped quietly.
pub fn pending_holds(pending: Pending, count: Option<u64>, now_ms: u64) -> Result<(), bool> {
    if count != Some(pending.count) {
        return Err(false);
    }
    if now_ms.saturating_sub(pending.at_ms) > RECHECK_WAIT_MS {
        return Err(true);
    }
    Ok(())
}

/// A purchase as it was signed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedBuy {
    pub offer: Offer,
    pub credits: Option<u64>,
    /// When it was signed: the History line of it is the first «Bought» since.
    pub at: evolite_api::Timestamp,
}

/// What a price read again means for a purchase pressed on the old one (BUG-181, the
/// architect's rule): decided by the total, not by the price's age.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reread {
    /// The same total, or less: it goes on the same click, the total shown as the ceiling
    /// (Platform takes what is due, АРХ-10).
    Buy,
    /// More: nothing is sent; «now … (was …)», and the next click is a knowing one.
    Dearer { now: u64, was: u64 },
    /// It cannot be bought now (not sold, under the new minimum, paused, the limit): the form
    /// says why.
    Refused,
}

/// [`Reread`] of the purchase that showed `shown`, by the quote of now; `refused`: the form
/// refuses it now.
pub fn after_reread(shown: u64, now: Option<Quote>, refused: bool) -> Reread {
    match now {
        _ if refused => Reread::Refused,
        Some(Quote::Total { total, .. }) if total <= shown => Reread::Buy,
        Some(Quote::Total { total, .. }) => Reread::Dearer {
            now: total,
            was: shown,
        },
        _ => Reread::Refused,
    }
}

impl BuyState {
    pub fn new(token_id: &str) -> Self {
        BuyState {
            token_id: token_id.to_owned(),
            count: String::new(),
            flow: Default::default(),
            result: None,
            was: None,
            asked_price: false,
            signed: None,
            pending: None,
            unchecked: false,
            dearer: None,
        }
    }

    /// `Done { what: BuyToken }`. A price that grew: nothing was taken; the password goes, so
    /// the new price is a new decision (P2).
    pub fn on_answer(
        &mut self,
        ask: Option<u64>,
        result: &Result<(), CoreError>,
        agreed_per_unit: Option<u64>,
    ) {
        if !self.flow.on_done(ask, result) {
            return;
        }
        let _ = self.flow.take_result();
        if result == &Err(CoreError::TokenPriceGrew) {
            self.was = agreed_per_unit;
            self.result = None;
            return;
        }
        self.result = Some(result.clone());
    }
}

fn row_of(cx: &Ctx, token_id: &str) -> Option<TokenRow> {
    cx.wallet()
        .tokens
        .rows
        .iter()
        .find(|r| r.info.token_id == token_id)
        .cloned()
}

/// «Token: Evo Coffee · 7Hq2…q8Rs · @evo-lab»: which token, against look-alikes (R-8, P1).
fn token_kv(ui: &mut Ui, p: &crate::theme::Palette, info: &TokenInfo) {
    kv_row(
        ui,
        p,
        tr!("Token"),
        &crate::i18n::own(&format!(
            "{} · {} · {}",
            state::name(info),
            state::short(&info.token_id),
            issuer(info)
        )),
        sans(13.0, Weight::Medium),
    );
}

/// Words under the figures, muted and wrapped (K1, P1: not a box).
fn quiet(ui: &mut Ui, p: &crate::theme::Palette, words: &str) {
    ui.add(egui::Label::new(txt(words, 12.0, Weight::Regular, p.muted)).wrap());
}

/// The clock time a price was read at: «14:32».
fn clock(read_ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(read_ms as i64)
        .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

/// The dialog's width: 460 in 800×600, 500 above (as «Send a token»).
fn width(ui: &Ui) -> f32 {
    if ui.ctx().content_rect().width() < 900.0 {
        460.0 - 56.0
    } else {
        500.0 - 56.0
    }
}

/// K1, K1u: «Claim the distribution · {token}».
pub(super) fn claim_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, tokens: &mut TokensState) {
    let p = a.p;
    let Some(form) = &mut tokens.claiming else {
        return;
    };
    let Some(row) = row_of(cx, &form.token_id) else {
        tokens.claiming = None;
        return;
    };
    // BUG-179: what was signed stays shown while it runs and once done; an error gives the
    // window back to what is to claim now.
    let done = matches!(form.result, Some(Ok(())));
    if !form.flow.waiting() && !done {
        form.signed = None;
    }
    let Some(claims) = form.signed.or(row.claims) else {
        tokens.claiming = None;
        return;
    };
    let wallet = cx.wallet();
    let who = sender(wallet);
    let from = shown_name(wallet);
    let fee = who.fee;
    let no_credits = who.credits.is_some_and(|c| c < fee);
    let scheduled = claims_scheduled(&claims);
    let ask = password_asked(cx.snap);
    let now = cx.now;
    let mut claim = None;
    let mut top_up = false;
    let title = format!(
        tr!("Claim the distribution · {}"),
        state::name_in_text(&row.info)
    );
    let closed = dialog_with_footer(
        ui,
        a,
        "token-claim",
        "token",
        &title,
        width(ui),
        |ui, a, part| {
            if part == Part::Footer {
                let mut close = false;
                form.flow
                    .password
                    .field(ui, &p, "token-claim-password", ask);
                match form
                    .result
                    .as_ref()
                    .filter(|_| !form.flow.password.is_wrong())
                {
                    Some(Ok(())) => {
                        notice(
                            ui,
                            a,
                            "confirmed",
                            p.accent,
                            tr!("Claimed: the amount is in the balance and in History."),
                        );
                        button_row(ui, |ui| {
                            if button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked() {
                                close = true;
                            }
                        });
                        return close;
                    }
                    // BUG-205: it may have gone through: said so, and no repeat.
                    Some(Err(e @ (CoreError::Unclear | CoreError::UnclearWhich))) => {
                        return super::unclear_footer(ui, a, e);
                    }
                    Some(Err(e)) => crate::components::error_notice(
                        ui,
                        a,
                        &crate::text::failed(tr!("Not claimed"), e),
                        crate::text::error_details(e).as_deref(),
                    ),
                    None => {}
                }
                button_row_spend(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let ready =
                        !form.flow.waiting() && form.flow.password.ready(ask) && !no_credits;
                    if button_enabled(
                        ui,
                        a,
                        tr!("Sign and claim"),
                        ButtonKind::Primary,
                        None,
                        ready,
                    )
                    .clicked()
                    {
                        claim = Some(Spend::ClaimToken {
                            token_id: row.info.token_id.clone(),
                            scheduled,
                        });
                    }
                });
                return close;
            }
            ui.spacing_mut().item_spacing.y = 10.0;
            let value = sans(13.0, Weight::Medium);
            let (kind, accrued) = match (scheduled, claims.scheduled, claims.perpetual) {
                (true, Some(s), _) => {
                    let next = match s.next_at {
                        Some(at) => format!(
                            tr!("on a schedule · next {}"),
                            crate::text::date_short(at, now)
                        ),
                        None => tr!("on a schedule").into(),
                    };
                    let amount = s.due.map(|(_, amount)| amount).unwrap_or(0);
                    (next, Some(state::written(amount, row.info.decimals)))
                }
                (_, _, Some(perpetual)) => (
                    format!(tr!("perpetual · every {}"), every_text(perpetual.every)),
                    perpetual
                        .at_least
                        .map(|n| format!(tr!("at least {}"), state::written(n, row.info.decimals))),
                ),
                _ => (String::new(), None),
            };
            kv_row(ui, &p, tr!("Distribution"), &kind, value.clone());
            kv_row(
                ui,
                &p,
                tr!("Recipient"),
                &crate::i18n::own(&from),
                value.clone(),
            );
            if let Some(warning) = claim_warning(&row) {
                crate::components::attention_notice(ui, a, warning);
            }
            match &accrued {
                // K1: the figure in mono, 15 SemiBold.
                Some(amount) => {
                    kv_row(ui, &p, tr!("Accrued"), amount, mono(15.0, Weight::SemiBold))
                }
                // K1u.
                None => kv_row(
                    ui,
                    &p,
                    tr!("Accrued"),
                    tr!("becomes known after the claim"),
                    sans(13.0, Weight::Regular),
                ),
            }
            if !scheduled && let Some(cycles) = claims.perpetual.and_then(|p| p.capped) {
                ui.label(txt(
                    &format!(
                        tr!("One claim pays at most {} cycles; more stays to claim after it."),
                        cycles
                    ),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
            if let Some(s) = claims.scheduled.filter(|s| scheduled && s.more_due > 0) {
                ui.label(txt(
                    &format!(tr!("{} more payouts are due: one claim each."), s.more_due),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
            divider(ui, &p);
            if no_credits {
                top_up = top_up_line(ui, a, &from);
            } else {
                kv_row(
                    ui,
                    &p,
                    &format!(tr!("Fee, in credits of {}"), crate::i18n::own(&from)),
                    &format!("≈ {}", crate::format::amount(fee as i64)),
                    mono(13.0, Weight::Medium),
                );
            }
            quiet(
                ui,
                &p,
                tr!(
                    "The network counts the exact amount at the claim — it will be in the balance and in History."
                ),
            );
            false
        },
    );
    if top_up {
        cx.top_up();
    }
    if let Some(op) = claim
        && let Some(command) = form.flow.command(op, ask, ())
    {
        cx.send(command);
        form.result = None;
        form.signed = Some(claims);
    }
    if closed {
        form.flow.password.forget();
        tokens.claiming = None;
    }
}

/// «Not enough credits for the fee: top up …» with the link; whether it was clicked.
fn top_up_line(ui: &mut Ui, a: &mut Assets, from: &str) -> bool {
    let p = a.p;
    let mut clicked = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(txt(
            &format!(
                tr!("Not enough credits for the fee: top up {}."),
                crate::i18n::own(from)
            ),
            12.0,
            Weight::Regular,
            p.danger,
        ));
        let link = ui.add(
            egui::Label::new(txt(tr!("Top up"), 12.0, Weight::Medium, p.text).underline())
                .sense(Sense::click()),
        );
        crate::components::named(&link, egui::WidgetType::Button, true, tr!("Top up"));
        clicked = link.clicked();
    });
    clicked
}

fn problem_words(problem: BuyProblem, row: &TokenRow, from: &str, credits: Option<u64>) -> String {
    match problem {
        BuyProblem::Paused => tr!("The issuer paused the token — it cannot be bought now.").into(),
        BuyProblem::Frozen => tr!("Your balance of this token is frozen.").into(),
        BuyProblem::Limit { left: 0 } => tr!("The token's supply limit is reached.").into(),
        BuyProblem::Limit { left } => format!(
            tr!("The supply limit allows {} more."),
            state::written(left, row.info.decimals)
        ),
        BuyProblem::UnderMinimum { minimum } => format!(
            tr!("At least {}"),
            state::written(minimum, row.info.decimals)
        ),
        BuyProblem::NotAmount => tr!("Not an amount of this token").into(),
        BuyProblem::NoCredits { need } => format!(
            tr!("Not enough credits: ≈ {} needed, {} has {}."),
            crate::format::amount(need as i64),
            crate::i18n::own(from),
            crate::format::amount(credits.unwrap_or(0) as i64)
        ),
    }
}

/// P1, P2: «Buy {token}».
pub(super) fn buy_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, tokens: &mut TokensState) {
    let p = a.p;
    let Some(form) = &mut tokens.buying else {
        return;
    };
    let Some(row) = row_of(cx, &form.token_id) else {
        tokens.buying = None;
        return;
    };
    let Some(live) = row.offer.clone() else {
        tokens.buying = None;
        return;
    };
    // BUG-179: what was signed stays shown while it runs and once done.
    let done = matches!(form.result, Some(Ok(())));
    if !form.flow.waiting() && !done {
        form.signed = None;
    }
    let offer = form
        .signed
        .as_ref()
        .map_or(live.clone(), |s| s.offer.clone());
    let wallet = cx.wallet();
    let mut who = sender(wallet);
    if let Some(signed) = &form.signed {
        who.credits = signed.credits;
    }
    let from = shown_name(wallet);
    let fee = who.fee;
    let decimals = row.info.decimals;
    let count = state::typed(&form.count, decimals).ok();
    let problem = buy_problem(&row, &offer, count, who.credits, fee);
    let quoted = count.map(|n| quote(&offer.tiers, n));
    let total = match quoted {
        Some(Quote::Total { total, .. }) => Some(total),
        _ => None,
    };
    let now_ms = cx.now.timestamp_millis().max(0) as u64;
    let old = stale(offer.read_ms, now_ms);
    // R-22: a price over a minute old is read again before the confirmation.
    if old && !form.asked_price {
        form.asked_price = true;
        cx.send(Command::ReadTokenPrices);
    }
    if !old {
        form.asked_price = false;
    }
    // BUG-181: what the purchase really took (the price may have come lower than shown): its
    // History line, the first «Bought» of this token since it was signed.
    let label = evolite_api::tokens::name_in(&row.info, "en").to_owned();
    let charged = form.signed.as_ref().and_then(|s| {
        cx.wallet()
            .activity
            .iter()
            .filter(|act| act.at + 5 >= s.at)
            .find(|act| matches!(&act.kind, evolite_api::ActivityKind::TokenBought { token, .. } if *token == label))
            .map(|act| -act.amount)
    });
    let ask = password_asked(cx.snap);
    let read_at = clock(offer.read_ms);
    let mut buy = None;
    // BUG-181 r2: no fresh price in 30 s (no answer, or the read refused), or another count
    // typed — the pending purchase is dropped and nothing is sent.
    if let Some(pending) = form.pending
        && let Err(said) = pending_holds(pending, count, now_ms)
    {
        form.pending = None;
        form.unchecked = said;
    }
    // BUG-181: pressed on an old price, read again now: by the total — the same or less goes on
    // that click, more is said and waits, what cannot be bought is the form's own refusal.
    if let Some(Pending { total: shown, .. }) =
        form.pending.filter(|_| !old && !form.flow.waiting())
    {
        form.pending = None;
        match (after_reread(shown, quoted, problem.is_some()), count) {
            (Reread::Buy, Some(count)) => {
                buy = Some(Spend::BuyToken {
                    token_id: row.info.token_id.clone(),
                    count,
                    agreed: shown,
                });
            }
            (Reread::Dearer { now, was }, _) => form.dearer = Some((now, was)),
            (Reread::Refused, _) | (Reread::Buy, None) => {}
        }
    }
    let mut top_up = false;
    let title = format!(tr!("Buy {}"), state::name_in_text(&row.info));
    let closed = dialog_with_footer(
        ui,
        a,
        "token-buy",
        "token",
        &title,
        width(ui),
        |ui, a, part| {
            if part == Part::Footer {
                let mut close = false;
                form.flow.password.field(ui, &p, "token-buy-password", ask);
                match form
                    .result
                    .as_ref()
                    .filter(|_| !form.flow.password.is_wrong())
                {
                    Some(Ok(())) => {
                        notice(
                            ui,
                            a,
                            "confirmed",
                            p.accent,
                            &charged.map_or_else(
                                || tr!("Bought: the tokens are in the balance.").to_string(),
                                |c| {
                                    format!(
                                        tr!("Bought: the tokens are in the balance; {} taken in credits."),
                                        crate::format::amount(c)
                                    )
                                },
                            ),
                        );
                        button_row(ui, |ui| {
                            if button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked() {
                                close = true;
                            }
                        });
                        return close;
                    }
                    // BUG-205: it may have gone through: said so, and no repeat.
                    Some(Err(e @ (CoreError::Unclear | CoreError::UnclearWhich))) => {
                        return super::unclear_footer(ui, a, e);
                    }
                    Some(Err(e)) => crate::components::error_notice(
                        ui,
                        a,
                        &crate::text::failed(tr!("Not bought"), e),
                        crate::text::error_details(e).as_deref(),
                    ),
                    None => {}
                }
                if let Some(problem) = problem.filter(|p| !matches!(p, BuyProblem::NotAmount)) {
                    ui.add(
                        egui::Label::new(txt(
                            &problem_words(problem, &row, &from, who.credits),
                            12.0,
                            Weight::Regular,
                            p.danger,
                        ))
                        .wrap(),
                    );
                }
                button_row_spend(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let ready = !form.flow.waiting()
                        && form.pending.is_none()
                        && problem.is_none()
                        && total.is_some()
                        && form.flow.password.ready(ask);
                    if button_enabled(ui, a, tr!("Sign and buy"), ButtonKind::Primary, None, ready)
                        .clicked()
                        && let (Some(count), Some(total)) = (count, total)
                    {
                        form.dearer = None;
                        form.unchecked = false;
                        if old {
                            // R-22, BUG-181: the price is read again first; the purchase goes on
                            // that click unless the total grew.
                            form.pending = Some(Pending {
                                total,
                                count,
                                at_ms: now_ms,
                            });
                            form.asked_price = true;
                            cx.send(Command::ReadTokenPrices);
                        } else {
                            buy = Some(Spend::BuyToken {
                                token_id: row.info.token_id.clone(),
                                count,
                                agreed: total,
                            });
                        }
                    }
                    if form.pending.is_some() {
                        ui.label(txt(
                            tr!("Checking the price…"),
                            12.0,
                            Weight::Regular,
                            p.muted,
                        ));
                    } else if form.unchecked {
                        ui.label(txt(
                            tr!("Could not check the price — try again."),
                            12.0,
                            Weight::Regular,
                            p.danger,
                        ));
                    }
                });
                return close;
            }
            ui.spacing_mut().item_spacing.y = 10.0;
            let value = sans(13.0, Weight::Medium);
            let number = mono(13.0, Weight::Medium);
            // BUG-181: the price read again came dearer: nothing was sent.
            if let Some((now, was)) = form.dearer {
                notice(
                    ui,
                    a,
                    "warning",
                    p.danger,
                    &format!(
                        tr!("The price grew: now {} (was {}) — check and sign again."),
                        crate::format::amount(purchase_total_duffs(now) as i64),
                        crate::format::amount(purchase_total_duffs(was) as i64)
                    ),
                );
            }
            // P2: the price grew before the purchase was written.
            if let (Some(was), Some(now_per_unit)) = (form.was, offer.tiers.first().map(|t| t.1)) {
                notice(
                    ui,
                    a,
                    "warning",
                    p.danger,
                    &format!(
                        tr!(
                            "The price changed: now {} (was {}). Nothing was taken. Check and confirm again."
                        ),
                        price_text(now_per_unit, decimals),
                        price_text(was, decimals)
                    ),
                );
            }
            token_kv(ui, &p, &row.info);
            // P1: «Price» and when it was read, the tiers, the one the count falls in marked.
            let tier_now = match quoted {
                Some(Quote::Total { tier, .. }) => Some(tier),
                _ => None,
            };
            card(&p, Margin::same(12)).fill(p.raised).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                kv_row(
                    ui,
                    &p,
                    tr!("Price"),
                    &format!(tr!("as of {}"), read_at),
                    sans(12.0, Weight::Regular),
                );
                for (from_count, per_unit) in &offer.tiers {
                    let on = tier_now == Some(*from_count);
                    let (left, right) = (
                        if offer.single {
                            String::new()
                        } else {
                            format!(tr!("from {}"), state::written(*from_count, decimals))
                        },
                        format!(tr!("{} each"), price_text(*per_unit, decimals)),
                    );
                    let frame = egui::Frame::new()
                        .fill(if on { p.surface } else { p.raised })
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(8, 6));
                    frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        kv_row(
                            ui,
                            &p,
                            &left,
                            &right,
                            mono(
                                13.0,
                                if on {
                                    Weight::SemiBold
                                } else {
                                    Weight::Regular
                                },
                            ),
                        );
                    });
                }
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("How much to buy"));
                text_field(
                    ui,
                    &p,
                    "token-buy-count",
                    &mut form.count,
                    "0",
                    FieldKind::Amount,
                    |ui| {
                        ui.label(txt(
                            &state::name_in_text(&row.info),
                            13.0,
                            Weight::Regular,
                            p.muted,
                        ));
                    },
                );
                if problem == Some(BuyProblem::NotAmount) && !form.count.trim().is_empty() {
                    ui.label(txt(
                        tr!("Not an amount of this token"),
                        12.0,
                        Weight::Regular,
                        p.danger,
                    ));
                } else if let Some(Quote::Total { tier, per_unit, .. }) = quoted
                    && !offer.single
                {
                    // Which price applies, under the field (P1).
                    ui.label(txt(
                        &format!(
                            tr!("From {} — {} each"),
                            state::written(tier, decimals),
                            price_text(per_unit, decimals)
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
            });
            divider(ui, &p);
            if let Some(total) = total {
                let duffs = purchase_total_duffs(total) as i64;
                kv_row(
                    ui,
                    &p,
                    tr!("Total"),
                    &format!(tr!("{} in credits"), crate::format::amount(duffs)),
                    mono(15.0, Weight::SemiBold),
                );
            }
            if matches!(problem, Some(BuyProblem::NoCredits { .. })) {
                top_up = top_up_line(ui, a, &from);
            } else {
                kv_row(
                    ui,
                    &p,
                    &format!(tr!("Fee, in credits of {}"), crate::i18n::own(&from)),
                    &format!("≈ {}", crate::format::amount(fee as i64)),
                    number.clone(),
                );
            }
            if let (Some(have), Some(total)) = (who.credits, total) {
                let after = have.saturating_sub(purchase_total_duffs(total).saturating_add(fee));
                // BUG-180: a long @name and both sums do not share one line: the value goes under.
                crate::components::kv_row_fit(
                    ui,
                    &p,
                    &format!(tr!("Credits of {}"), crate::i18n::own(&from)),
                    &format!(
                        "{} · {}",
                        crate::format::amount(have as i64),
                        crate::i18n::t(&format!(
                            tr!("after ≈ {}"),
                            crate::format::amount(after as i64)
                        ))
                    ),
                    value.clone(),
                );
            }
            quiet(
                ui,
                &p,
                tr!(
                    "A purchase cannot be undone. If the issuer changes the price before it is written, the network refuses and evolite shows the new price — nothing is taken."
                ),
            );
            false
        },
    );
    if top_up {
        cx.top_up();
    }
    if let Some(op) = buy
        && let Some(command) = form.flow.command(op, ask, ())
    {
        cx.send(command);
        form.result = None;
        form.was = None;
        form.signed = Some(SignedBuy {
            offer: live,
            credits: who.credits,
            at: cx.now.timestamp(),
        });
    }
    if closed {
        form.flow.password.forget();
        tokens.buying = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::tokens::{Perpetual, Scheduled, TokenOrigin};

    fn row(paused: bool, frozen: bool) -> TokenRow {
        TokenRow {
            info: TokenInfo {
                token_id: "T".into(),
                contract_id: "C".into(),
                position: 0,
                names: vec![("en".into(), "Jembe Gold".into())],
                decimals: 2,
                issuer: "I".into(),
                issuer_name: None,
                transferable: true,
                keeps_history: true,
            },
            origin: TokenOrigin::Found,
            balance: Some(1_000),
            paused: Some(paused),
            frozen: Some(frozen),
            claim: false,
            claims: None,
            offer: None,
        }
    }

    fn offer(left: Option<u64>) -> Offer {
        Offer {
            tiers: vec![(100, 50_000), (1_000, 40_000)],
            single: false,
            read_ms: 0,
            left,
        }
    }

    /// AC-A9: pause, freeze, the limit, the minimum and the credits refuse before the password.
    #[test]
    fn a_purchase_is_refused_before_the_password() {
        let ok = row(false, false);
        let o = offer(None);
        assert_eq!(
            buy_problem(&row(true, false), &o, Some(100), None, 0),
            Some(BuyProblem::Paused)
        );
        assert_eq!(
            buy_problem(&row(false, true), &o, Some(100), None, 0),
            Some(BuyProblem::Frozen)
        );
        assert_eq!(
            buy_problem(&ok, &offer(Some(0)), None, None, 0),
            Some(BuyProblem::Limit { left: 0 }),
            "said before a count is typed"
        );
        assert_eq!(
            buy_problem(&ok, &offer(Some(500)), Some(600), None, 0),
            Some(BuyProblem::Limit { left: 500 })
        );
        assert_eq!(
            buy_problem(&ok, &o, Some(99), None, 0),
            Some(BuyProblem::UnderMinimum { minimum: 100 })
        );
        assert_eq!(
            buy_problem(&ok, &o, Some(0), None, 0),
            Some(BuyProblem::NotAmount)
        );
        // 100 × 50 000 credits = 5 000 duffs, and a fee of 1 000.
        assert_eq!(buy_problem(&ok, &o, Some(100), Some(6_000), 1_000), None);
        assert_eq!(
            buy_problem(&ok, &o, Some(100), Some(5_999), 1_000),
            Some(BuyProblem::NoCredits { need: 6_000 })
        );
        assert_eq!(
            buy_problem(&ok, &o, None, Some(0), 1_000),
            None,
            "nothing typed yet"
        );
    }

    /// AC-D1, R-15: «Claim» only for what this identity may claim; «Buy» only with a price.
    #[test]
    fn the_buttons_follow_what_was_proved() {
        let mut r = row(false, false);
        assert!(!can_claim(&r) && !can_buy(&r));
        r.claims = Some(Claims {
            perpetual: None,
            scheduled: Some(Scheduled {
                due: None,
                more_due: 0,
                next_at: Some(10),
            }),
        });
        assert!(!can_claim(&r), "a payout still to come is not claimed");
        r.claims = Some(Claims {
            perpetual: Some(Perpetual {
                every: Every::Millis(86_400_000),
                at_least: None,
                capped: None,
            }),
            scheduled: None,
        });
        assert!(can_claim(&r));
        r.offer = Some(offer(None));
        assert!(can_buy(&r));
    }

    /// A pre-programmed payout due is claimed first: it is one payout, known to the unit.
    #[test]
    fn a_scheduled_payout_due_goes_first() {
        let both = Claims {
            perpetual: Some(Perpetual {
                every: Every::Blocks(100),
                at_least: Some(5),
                capped: None,
            }),
            scheduled: Some(Scheduled {
                due: Some((1, 7)),
                more_due: 0,
                next_at: None,
            }),
        };
        assert!(claims_scheduled(&both));
        assert!(!claims_scheduled(&Claims {
            scheduled: None,
            ..both
        }));
    }

    /// A price per smallest unit reads as a price per whole token in DASH.
    #[test]
    fn a_price_reads_per_whole_token() {
        // 2 decimals: 80 000 credits a unit are 8 000 000 a token, 8 000 duffs: 0.00008 DASH.
        assert_eq!(per_token_duffs(80_000, 2), 8_000);
        assert_eq!(per_token_duffs(u64::MAX, 30), i64::MAX, "never wraps");
    }

    /// P2: a price that grew wipes the password and keeps what was agreed, nothing else.
    #[test]
    fn a_price_that_grew_is_a_new_decision() {
        let mut form = BuyState::new("T");
        form.flow
            .set_stage(crate::components::spend_flow::Stage::Waiting { ask: 0, signed: () });
        form.on_answer(Some(0), &Err(CoreError::TokenPriceGrew), Some(40_000));
        assert_eq!(form.was, Some(40_000));
        assert!(form.result.is_none() && !form.flow.waiting());
        form.on_answer(Some(0), &Ok(()), None);
        assert!(form.result.is_none(), "not waited for: not taken");
    }

    /// The architect's pause regression (T-3 r1): on a proved pause a transfer and a purchase
    /// are refused before the password, each with its reason. A claim is not: Platform 575d961
    /// checks the pause on a transfer and a direct purchase, not on a plain claim
    /// (`token_claim_transition_action` has no pause check, only the claim to the pool has), so
    /// «Claim» stays.
    #[test]
    fn a_proved_pause_refuses_a_transfer_and_a_purchase_with_the_reason() {
        let mut paused = row(true, false);
        assert_eq!(
            super::super::state::row_reason(&paused),
            Some("The issuer paused the token — transfers are not available now."),
            "a transfer"
        );
        assert_eq!(
            buy_problem(&paused, &offer(None), Some(100), Some(u64::MAX), 0),
            Some(BuyProblem::Paused),
            "a purchase"
        );
        assert_eq!(
            buy_problem(&paused, &offer(None), None, None, 0),
            Some(BuyProblem::Paused),
            "said before a count is typed"
        );
        paused.claims = Some(Claims {
            perpetual: Some(Perpetual {
                every: Every::Millis(86_400_000),
                at_least: None,
                capped: None,
            }),
            scheduled: None,
        });
        assert!(can_claim(&paused), "a claim: Platform takes it on a pause");
        assert!(
            claim_warning(&paused).is_some_and(|w| w.contains("you can claim")),
            "with the warning"
        );
        assert!(
            claim_warning(&row(false, false)).is_none(),
            "not without a pause"
        );
    }
}
