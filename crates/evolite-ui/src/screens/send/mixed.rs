//! «Mixed» on Send (FEAT-0030 phase A): the fifth source — the CoinJoin account's fully mixed
//! coins, to a Core address only, with no change. What the coins hold over the amount and the
//! fee goes to the miners with the fee (the customer, арх-30-1); the core decides how much may
//! go and when the person must agree (АРХ-9а), the form asks it for each amount and recipient.

use super::*;
use evolite_api::{MixedExcess, MixedQuote};

/// The plan the core gave for a payment from «Mixed», and the person's agreement to its excess.
#[derive(Default)]
pub struct MixedPay {
    /// The number of the last `PreviewMixed`: only its answer is taken.
    pub ask: u64,
    /// What it was asked for: the recipient and the amount.
    pub asked: Option<(String, Duffs)>,
    pub quote: Option<Result<MixedQuote, CoreError>>,
    /// «I understand: the excess … goes to the miners», ticked: the excess agreed to. Any change
    /// takes it off (the architect).
    pub agreed: Option<Duffs>,
    /// The core found another excess when it built the payment (P2b): said over the tick until it
    /// is ticked again.
    pub changed: Option<Duffs>,
    /// The block of the last look the middle was last brought to (P2, P2b, P4): its excess, and
    /// whether refused; and the frames still to bring it.
    pub revealed: Option<(Duffs, bool)>,
    pub reveal: u8,
    /// The refusal under «Amount» last brought into view (S3): its excess and suggested amount.
    pub refusal_revealed: Option<(Duffs, Duffs)>,
}

impl MixedPay {
    /// Asks the core for the plan of `amount` to `to`, once for each: a new one takes the
    /// agreement off.
    pub(super) fn ask(&mut self, to: &str, amount: Duffs) -> Option<Command> {
        let want = (to.to_owned(), amount);
        if self.asked.as_ref() == Some(&want) {
            return None;
        }
        self.ask += 1;
        self.asked = Some(want);
        self.quote = None;
        self.agreed = None;
        Some(Command::PreviewMixed {
            ask: self.ask,
            to: to.to_owned(),
            amount,
        })
    }

    /// The core's plan: the last ask's only. Another excess than the one agreed takes the tick
    /// off.
    pub fn on_preview(&mut self, ask: u64, result: &Result<MixedQuote, CoreError>) {
        if ask != self.ask {
            return;
        }
        if let Ok(q) = result
            && self.agreed.is_some_and(|agreed| agreed != q.excess)
        {
            self.agreed = None;
        }
        self.quote = Some(result.clone());
    }

    /// The plan for exactly `to` and `amount`, when the core has given it.
    pub(super) fn quote_for(
        &self,
        to: &str,
        amount: Duffs,
    ) -> Option<&Result<MixedQuote, CoreError>> {
        match &self.asked {
            Some((t, a)) if t == to && *a == amount => self.quote.as_ref(),
            _ => None,
        }
    }

    /// Asked again on the next frame: the core refused what was built (P2b, P4).
    pub(super) fn ask_again(&mut self) {
        self.asked = None;
        self.quote = None;
    }

    pub fn reset(&mut self) {
        *self = Self {
            ask: self.ask,
            ..Self::default()
        };
    }
}

/// «Mixed» among the sources, after Core: only while the CoinJoin accounts hold something.
pub(super) fn mixed_source(w: &Wallet) -> Option<SourceView> {
    let m = w.balances.mixed?;
    Some(SourceView {
        source: Source::Mixed,
        icon: "mixed",
        title: tr!("Mixed"),
        duffs: m.spendable,
        // Until the core gives the plan: that of spending them all, the most it can be.
        fee: (m.spendable - m.max).max(0),
        max_fee: (m.spendable - m.max).max(0),
        withdrawal: None,
        // As sure as Core, with whose sync it comes (O2).
        read: true,
        note_icon: "mixed",
        note: tr!(
            "Paid only from mixed coins, to a Core address. There is no change: the coins are whole denominations."
        ),
    })
}

/// The fee of the core's plan in «Mixed»'s card, once there is one (`quote`).
pub(super) fn apply_quote(all: &mut [SourceView], quote: Option<&MixedQuote>) {
    // A refused plan too: the summary shows its fee and its excess as they are (S3).
    if let (Some(q), Some(s)) = (quote, all.iter_mut().find(|s| s.source == Source::Mixed)) {
        s.fee = q.fee;
    }
}

/// «1,2000 DASH · no change», the card's line.
pub(super) fn card_line(s: &SourceView) -> String {
    format!(
        tr!("{amount} · no change"),
        amount = crate::format::balance(s.duffs)
    )
}

/// Why «Mixed» cannot be chosen at all: it has no fully mixed coin yet (S2, the card dimmed).
pub(super) fn card_hint(w: &Wallet) -> Option<String> {
    let m = w.balances.mixed?;
    (m.spendable == 0).then(|| {
        format!(
            tr!("No mixed coins yet — {amount} is not mixed yet."),
            amount = crate::format::amount(m.unfinished)
        )
    })
}

/// Why a payment from «Mixed» cannot be made as typed (AC-A5, S2): to a name or to Platform,
/// not enough coins, or what the core said. The refusal of a large excess (AC-A12) is apart:
/// [`refused`].
pub(super) fn refusal(
    typed: &str,
    quote: Option<&Result<MixedQuote, CoreError>>,
    spendable: Duffs,
    amount: Option<Duffs>,
) -> Option<String> {
    let typed = typed.trim();
    if typed.starts_with('@') {
        return Some(tr!("From mixed coins you can pay only to a Core address for now.").into());
    }
    if typed.to_ascii_lowercase().starts_with("tdash1") {
        return Some(
            tr!("Mixed coins do not go to Platform: a top-up would put them into one transaction and show they are yours.")
                .into(),
        );
    }
    match quote? {
        Err(CoreError::NotEnough) => amount.map(|amount| {
            // The fee of the coins there are, the most it can be.
            let short = (amount - spendable).max(1);
            format!(
                tr!("Not enough mixed coins: {amount} more is needed."),
                amount = crate::format::amount(short)
            )
        }),
        Err(CoreError::NotAvailable(why)) => Some(format!("{}.", why.trim_end_matches('.'))),
        Err(e) => Some(send_error(e)),
        Ok(_) => None,
    }
}

/// The core refuses the excess (АРХ-9а, AC-A12): it and the amount that burns nothing.
pub(super) fn refused(quote: Option<&Result<MixedQuote, CoreError>>) -> Option<(Duffs, Duffs)> {
    match quote? {
        Ok(q) if q.rule == MixedExcess::Refuse => Some((q.excess, q.suggested?)),
        _ => None,
    }
}

/// The plan, when the core has given one it would send.
pub(super) fn usable(quote: Option<&Result<MixedQuote, CoreError>>) -> Option<&MixedQuote> {
    match quote? {
        Ok(q) if q.rule != MixedExcess::Refuse => Some(q),
        _ => None,
    }
}

/// An excess the person sees: more than dust (smaller is part of the fee, AC-A3).
pub(super) fn shown_excess(q: &MixedQuote) -> Option<Duffs> {
    (q.excess >= evolite_api::coin_plan::DUST).then_some(q.excess)
}

/// Under «Amount»: the excess is too large (S3, AC-A12) — why, what the suggested amount does,
/// and «Use …», which puts it in the field. In the colour of a reason, not of an error: no money
/// is lost. No «Pick coins» in phase A (the architect: the plan already takes the coins with the
/// least excess).
pub(super) fn refusal_block(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    (excess, suggested): (Duffs, Duffs),
) {
    let p = a.p;
    let block =     ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.add(
            crate::components::para(
                &crate::text::mixed_excess_words(excess, Some(suggested)),
                12.0,
                Weight::Medium,
                p.text,
            ),
        );
        ui.add(
            crate::components::para(
                &format!(
                    tr!("With {suggested} the recipient gets more than you entered, and nothing goes to the miners beyond the fee."),
                    suggested = crate::format::amount(suggested)
                ),
                12.0,
                Weight::Regular,
                p.muted,
            ),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            // With its unit, as the analyst's {suggested} is (the designer).
            let shown = crate::format::amount(suggested);
            if link(ui, &p, &format!(tr!("Use {suggested}"), suggested = shown)).clicked()
            {
                form.amount = crate::format::input_text(suggested);
                form.suggested = None;
                form.max = false;
                form.result = None;
            }
        });
    });
    // Brought into view once, with the field above it, when it comes (the designer, S3-440):
    // low, the form showed only its top.
    if form.mixed.refusal_revealed != Some((excess, suggested)) {
        form.mixed.refusal_revealed = Some((excess, suggested));
        block.response.scroll_to_me(Some(egui::Align::BOTTOM));
    }
}

/// A link-button of the form: 12 SemiBold `accent` words, a Tab stop named by them (S3).
fn link(ui: &mut Ui, p: &crate::theme::Palette, words: &str) -> egui::Response {
    let resp = ui
        .add(
            egui::Label::new(txt(words, 12.0, Weight::SemiBold, p.accent))
                .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    crate::components::named(&resp, egui::WidgetType::Button, true, words);
    resp
}

/// The summary's rows for «Mixed»: «From» with the coins, and the excess going to the miners.
pub(super) fn summary_from(q: Option<&MixedQuote>) -> String {
    match q {
        Some(q) => format!(
            "{} · {}",
            crate::i18n::t(tr!("Mixed")),
            crate::i18n::t(&crate::text::plural(q.coins, tr!("coin"), tr!("coins")))
        ),
        None => tr!("Mixed").to_owned(),
    }
}

/// Which of АРХ-9а's thresholds the excess is over, in one line (P2): of the amount, of 0.001
/// DASH, or both.
pub(super) fn over_words(q: &MixedQuote) -> Option<&'static str> {
    let over_share = q.excess.saturating_mul(100)
        > q.amount
            .saturating_mul(evolite_api::MIXED_EXCESS_FREE_PERCENT);
    let over_free = q.excess > evolite_api::MIXED_EXCESS_FREE;
    match (over_share, over_free) {
        (true, true) => Some(tr!(
            "The excess is over 1 % of the amount and over 0.001 DASH. You can change the amount to lose less."
        )),
        (true, false) => Some(tr!(
            "The excess is over 1 % of the amount. You can change the amount to lose less."
        )),
        (false, true) => Some(tr!(
            "The excess is over 0.001 DASH. You can change the amount to lose less."
        )),
        (false, false) => None,
    }
}

/// The excess block of the last look (P1, P2): what goes to the miners, the fee and it, what
/// leaves the wallet; to agree to (P2), in `accent` with the threshold passed.
pub(super) fn excess_block(ui: &mut Ui, a: &mut Assets, q: &MixedQuote) -> Option<egui::Response> {
    let p = a.p;
    let excess = shown_excess(q)?;
    let confirm = q.rule == MixedExcess::Confirm;
    let (icon, color, stroke) = if confirm {
        ("warning", p.accent, Stroke::new(1.0, p.accent))
    } else {
        ("info", p.muted, Stroke::NONE)
    };
    let dash = crate::format::amount;
    let block = egui::Frame::new()
        .fill(p.raised)
        .stroke(stroke)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(a.icons.icon(icon, color, &p, 16.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let first = if confirm { p.accent } else { p.text };
                    let line = |ui: &mut Ui, text: &str, size: f32, weight, color| {
                        // Through `para`: an Arabic paragraph wraps in reading order (BUG-119).
                        ui.add(crate::components::para(text, size, weight, color));
                    };
                    line(
                        ui,
                        &format!(
                            tr!("Mixed coins give no change: the excess {excess} goes to the miners as part of the fee."),
                            excess = dash(excess)
                        ),
                        13.0,
                        Weight::Medium,
                        first,
                    );
                    line(
                        ui,
                        &format!(
                            tr!("Fee {fee} + excess {excess} = {total}"),
                            fee = dash(q.fee),
                            excess = dash(excess),
                            total = dash(q.fee + excess)
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    );
                    line(
                        ui,
                        &format!(
                            tr!("In all, {spent} leaves the wallet; the recipient gets {amount}."),
                            spent = dash(q.amount + q.fee + excess),
                            amount = dash(q.amount)
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    );
                    if confirm && let Some(over) = over_words(q) {
                        line(ui, over, 12.0, Weight::Medium, p.accent);
                    }
                });
            });
        });
    Some(block.response)
}

/// The core's refusal in the last look (P4): the excess grew past what may burn. No «Use» nor
/// «Pick coins» here: their place is the form.
pub(super) fn refused_block(
    ui: &mut Ui,
    a: &mut Assets,
    (excess, suggested): (Duffs, Duffs),
) -> egui::Response {
    let p = a.p;
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add(
                crate::components::para(
                    &crate::text::mixed_excess_words(excess, Some(suggested)),
                    13.0,
                    Weight::Medium,
                    p.text,
                ),
            );
            ui.add(
                crate::components::para(
                    &format!(
                        tr!("With {suggested} the recipient gets more than you entered, and nothing goes to the miners beyond the fee."),
                        suggested = crate::format::amount(suggested)
                    ),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ),
            );
        })
        .response
}

/// The tick that agrees to the excess (P2), in the pinned bottom by the button it unlocks; the
/// line over it when the core found another excess (P2b). Whether the payment may be signed.
pub(super) fn agreement(ui: &mut Ui, a: &mut Assets, form: &mut SendForm, q: &MixedQuote) -> bool {
    if q.rule != MixedExcess::Confirm {
        return true;
    }
    let p = a.p;
    if let Some(changed) = form.mixed.changed {
        ui.add(crate::components::para(
            &crate::text::mixed_excess_words(changed, None),
            12.0,
            Weight::Medium,
            p.accent,
        ));
    }
    let mut on = form.mixed.agreed == Some(q.excess);
    let words = format!(
        tr!("I understand: the excess {excess} goes to the miners"),
        excess = crate::format::amount(q.excess)
    );
    crate::components::checkbox(ui, a, &mut on, &words);
    form.mixed.agreed = on.then_some(q.excess);
    if on {
        form.mixed.changed = None;
    }
    on
}

/// The payment from «Mixed» goes to the core with the excess agreed (0 when there was nothing to
/// agree to), and the form lets go of it as of any payment.
pub(super) fn sign_and_send_mixed(
    cx: &mut Ctx,
    form: &mut SendForm,
    to: String,
    (amount, rate): (Duffs, Option<&super::currency::Rate>),
    q: &MixedQuote,
    ask_password: bool,
) {
    form.sent_to = to.clone();
    form.last_payment += 1;
    cx.send(Command::Spend {
        op: Spend::SendMixed {
            id: form.last_payment,
            to,
            amount,
            note: form.note_to_send().map(evolite_api::NoteText),
            rate: rate.map(super::currency::Rate::for_core),
            accept_excess: match q.rule {
                MixedExcess::Confirm => form.mixed.agreed.unwrap_or(0),
                _ => q.excess,
            },
        },
        password: ask_password.then(|| Password::new(form.password.to_string())),
    });
    form.waiting += 1;
    form.let_go_of_payment();
    form.signing = (ask_password && form.waiting == 1).then(std::time::Instant::now);
    form.password_wrong = false;
}

/// P2, P2b, P4: the middle of the last look is brought to the block once it is there, and to a
/// new one — the designer's exception to BUG-122's «a new opening shows the middle from its top»:
/// at 800 × 440 the tick was in view and the reason for it below the edge. For a few frames: the
/// dialog's first frame puts its middle at the top. Its top to the middle's: at 800 × 440 the middle
/// is lower than the block, and its first line says why. P1's block, nothing to agree to, is not.
pub(super) fn reveal(form: &mut SendForm, key: (Duffs, bool), block: &egui::Response) {
    if form.mixed.revealed != Some(key) {
        form.mixed.revealed = Some(key);
        form.mixed.reveal = 6;
    }
    if form.mixed.reveal > 0 {
        form.mixed.reveal -= 1;
        block.scroll_to_me(Some(egui::Align::TOP));
    }
}
