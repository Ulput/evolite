//! 03 · Send: the form on the left (source, recipient, amount, fee) and the summary on
//! the right. Balances and fees come from the core; a name is resolved by the core (DPNS);
//! "Check and send" shows the payment once more (the full address) before the core
//! signs it, then the network's answer.

use super::txt;
use crate::components::{
    Assets, ButtonKind, CardColumn, FieldKind, amount_field, button, button_enabled, card, dialog,
    divider, field_label, from_columns, kv_row, notice, option_card_in, option_card_width,
    page_header, pill, text_field,
};
use crate::context::Ctx;
use crate::text::{name_problem, short_address};
use crate::theme::{Weight, mono, sans};
use egui::{Align, Layout, Margin, Stroke, Ui, vec2};
use evolite_api::{
    Activity, ActivityKind, Command, ContactState, CoreError, Duffs, Password, ResolvedName,
    SavedAddress, SentPayment, Source, Spend, Timestamp, Wallet,
};
use zeroize::{Zeroize, Zeroizing};

pub struct SendForm {
    pub source: usize,
    pub recipient: String,
    pub amount: String,
    /// The example amount in `amount` until the person changes it (`format::fill_suggested`).
    pub suggested: Option<i64>,
    /// "Max" put the amount there and the person has not changed it since: it follows the
    /// source (and its fee) chosen after it, instead of staying the last source's (BUG-39 №6).
    pub max: bool,
    /// The name last sent to the core for resolving, and its answer.
    pub asked: String,
    pub resolution: Option<Result<ResolvedName, CoreError>>,
    /// When [`Self::asked`] went to the core: a long silence is said in words (BUG-87).
    pub asked_at: Option<std::time::Instant>,
    /// The payment shown for a last look before signing.
    pub confirming: bool,
    /// Payments signed and with the network; each answer comes as `Event::Sent`. The form is free
    /// meanwhile: a payment on its way is in the history as "Being sent".
    pub waiting: u32,
    /// The number of the last payment handed to the core: its answer ([`Event::Sent`]) is told
    /// from an earlier one's by it (ids only grow, also across a form replaced).
    pub last_payment: u64,
    /// The last payment whose own answer has come: after it an earlier one's changes nothing.
    pub last_answered: u64,
    /// The amount as typed for the last payment signed: cleared from the form at once, put back
    /// if the core refuses it.
    pub signed: String,
    /// The recipient of the last payment signed: its answer clears "To" while it still holds
    /// it (BUG-39 №7), and says which kind of payment the answer is about.
    pub sent_to: String,
    /// The name being typed for the recipient's address, to keep it in the address book.
    pub naming: Option<String>,
    /// The address book's answer to the last save or removal, when it refused.
    pub book_error: Option<CoreError>,
    pub result: Option<Result<SentPayment, CoreError>>,
    /// "Password for every payment": typed in the confirmation, wiped once answered or closed
    /// (BUG-92: kept for a correction when the core says it is wrong).
    pub password: Zeroizing<String>,
    /// The last look stays open on the core's answer for this password (BUG-92): when it was
    /// sent, so a slow answer stops holding the dialog after [`SIGNING_HOLD`].
    pub signing: Option<std::time::Instant>,
    /// The core said this password is wrong: said at the field, until it changes (BUG-92).
    pub password_wrong: bool,
    /// That answer has come: the password field takes the focus again, once.
    pub refocus_password: bool,
    /// A refusal has just come with the dialog closed: its line is brought into view, once.
    pub reveal_result: bool,
    /// "Add to contacts" from here went to the core, no answer yet; then its answer.
    pub contact_waiting: bool,
    pub contact_result: Option<Result<(), CoreError>>,
    /// "Password for every payment" for that request: it spends credits too (BUG-42).
    pub contact_password: crate::components::SpendPassword,
    /// The recipient whose Core address the core was asked for (the last look before
    /// signing), and its answer: the address the payment will go to.
    pub previewed: String,
    pub preview: Option<Result<String, CoreError>>,
    /// "Note · only for you" (FEAT-0003): kept with the payment on this computer, never sent.
    pub note: String,
    /// The last change was cut to what a note holds: N4 shows until the next change.
    pub note_cut: bool,
    /// The note of the last payment signed: cleared from the form with the amount, put back
    /// if the core refuses the payment (AC-A3).
    pub signed_note: String,
}

impl SendForm {
    /// An empty form, for a real wallet.
    pub fn empty() -> Self {
        Self {
            recipient: String::new(),
            amount: String::new(),
            suggested: None,
            ..Self::default()
        }
    }

    /// A form paying from `source` to `to`: "Withdraw to Core" (credits to the wallet's own Core
    /// address), "Transfer" (credits, the recipient to type).
    pub fn paying(source: Source, to: String) -> Self {
        Self {
            source: match source {
                Source::Core => 0,
                Source::Credits => 1,
                Source::Pool => 2,
            },
            recipient: to,
            ..Self::empty()
        }
    }

    /// `next` in place of this form. Payments still with the network stay counted: the form
    /// goes on saying that their answer is coming (and about which kind of payment).
    pub fn replace(&mut self, next: SendForm) {
        let waiting = self.waiting;
        let last_payment = self.last_payment;
        let last_answered = self.last_answered;
        let sent_to = std::mem::take(&mut self.sent_to);
        *self = SendForm {
            waiting,
            last_payment,
            last_answered,
            sent_to,
            ..next
        };
    }

    /// The wallet locked: what was typed and shown goes with it (BUG-39 №7) — the recipient,
    /// the amount, the last answer; a payment waiting for its last look is not signed after the
    /// next unlock with what was on screen before. One already with the network keeps waiting
    /// for its answer.
    pub fn on_lock(&mut self) {
        self.password.zeroize();
        self.signed.zeroize();
        // The note is the person's too: not left in memory after a lock (AC-T6).
        self.note.zeroize();
        self.signed_note.zeroize();
        self.contact_password.forget();
        self.replace(SendForm::empty());
    }

    pub fn on_contact_done(&mut self, result: &Result<(), CoreError>) {
        if !self.contact_waiting {
            return;
        }
        self.contact_waiting = false;
        self.contact_result = Some(result.clone());
        self.contact_password.on_answer(result);
        if result.is_ok() {
            // Asked again: the answer now says where the name stands among the contacts.
            self.asked.clear();
        }
    }

    pub fn on_sent(&mut self, id: u64, result: &Result<SentPayment, CoreError>) {
        // A number this form never gave.
        if id > self.last_payment {
            return;
        }
        self.waiting = self.waiting.saturating_sub(1);
        // What the form holds (the amount, the note, the recipient) is the last payment's.
        let last = id == self.last_payment;
        // An earlier payment's answer, after the last one's: that line and that form stay.
        if !last && self.last_answered == self.last_payment {
            return;
        }
        if last {
            self.last_answered = id;
        }
        // The last look was held open for this answer (BUG-92).
        let held = last && self.signing.take().is_some();
        if last {
            // Not sent: the form has it again (AC-A3); sent: the core keeps the note with it.
            if result.is_err() && self.amount.is_empty() {
                self.amount = std::mem::take(&mut self.signed);
            }
            if result.is_err() && self.note.is_empty() {
                self.note = std::mem::take(&mut self.signed_note);
            } else {
                self.signed_note.zeroize();
            }
            // Gone: "To" is cleared for the next payment, unless the person already typed another.
            if result.is_ok() && self.recipient.trim() == self.sent_to.trim() {
                self.recipient.clear();
                self.naming = None;
                self.book_error = None;
                self.max = false;
                self.signed.clear();
            }
        } else if result.is_err() && self.max {
            // An earlier payment's refusal gives nothing back to send a second time (BUG-92 r2).
            // «Max» would fill the amount again; an amount typed since is the person's.
            self.max = false;
            self.amount.clear();
        }
        if held && matches!(result, Err(CoreError::WrongPassword)) {
            // Nothing was sent: the dialog stays, the error at its field, the password to correct.
            self.password_wrong = true;
            self.refocus_password = true;
            self.result = None;
            return;
        }
        if held {
            self.confirming = false;
            self.password.zeroize();
        }
        // One line for the answers, and the last payment's own is what it says. An earlier
        // payment's «sent» does not take the place of a refusal already there.
        if !last && result.is_ok() && matches!(self.result, Some(Err(_))) {
            return;
        }
        // Under the buttons, which may be below the window's edge at 1024×768 (BUG-92).
        self.reveal_result = result.is_err();
        self.result = Some(result.clone());
    }

    pub fn on_book_done(&mut self, result: &Result<(), CoreError>) {
        match result {
            Ok(()) => {
                self.naming = None;
                self.book_error = None;
            }
            Err(e) => self.book_error = Some(e.clone()),
        }
    }

    /// "Max" taken and not changed since: the amount is what the balance chosen now can send
    /// (`balance`, `fee`, `withdrawal` of the source on screen).
    fn follow_max(&mut self, balance: Duffs, fee: Duffs, withdrawal: Option<Withdrawal>) {
        if self.max {
            self.amount = crate::format::input_text(max_amount(balance, fee, withdrawal));
            self.suggested = None;
        }
    }

    /// The note field changed to `typed`: cleaned and cut to what a note holds (140 characters
    /// as a person sees them, 1024 bytes); a cut says so until the next change (AC-D1, Q-D5).
    pub fn type_note(&mut self, typed: &str) {
        let (fit, cut) = evolite_core::note_text::fit(typed);
        self.note = fit;
        self.note_cut = cut;
    }

    /// The note to send with the payment: trimmed, none when empty.
    fn note_to_send(&self) -> Option<String> {
        Some(self.note.trim().to_owned()).filter(|n| !n.is_empty())
    }

    /// The note the last look shows: the one sent, while its answer is awaited (BUG-92).
    fn note_shown(&self) -> Option<String> {
        if self.signing.is_some() {
            Some(self.signed_note.trim().to_owned()).filter(|n| !n.is_empty())
        } else {
            self.note_to_send()
        }
    }

    /// Where a payment goes, asked for the last look or while an address is typed.
    pub fn on_preview(&mut self, to: &str, result: &Result<String, CoreError>) {
        if to == self.previewed {
            self.preview = Some(result.clone());
        }
    }
}

/// N3 under the note field (FEAT-0003, FR-8).
pub const NOTE_PRIVACY: &str = tr!(
    "Kept only on this computer, encrypted with the history. It never goes to the network and the recipient does not see it. Restoring from the recovery phrase does not bring it back."
);

/// "N / 140": the characters of a note as a person sees them; numbers only, no words, and no
/// thousands to group (AC-T10).
pub fn note_counter(note: &str) -> String {
    format!(
        "{} / {}",
        evolite_core::note_text::graphemes(note),
        evolite_api::NOTE_MAX_GRAPHEMES
    )
}

/// "Note · only for you" (Figma 03, AC-D1): optional, up to 140 characters with the counter
/// on the right (red at the limit), cut with N4 when more was pasted, and the privacy line.
fn note_field(ui: &mut Ui, a: &mut Assets, form: &mut SendForm) {
    let p = a.p;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let counter = note_counter(&form.note);
        let full =
            evolite_core::note_text::graphemes(&form.note) >= evolite_api::NOTE_MAX_GRAPHEMES;
        ui.horizontal(|ui| {
            field_label(ui, &p, tr!("Note · only for you"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(&counter)
                        .font(mono(12.0, Weight::Regular))
                        .color(if full { p.danger } else { p.muted }),
                );
            });
        });
        crate::a11y::name_next_field(ui, tr!("Note · only for you"));
        let mut typed = form.note.clone();
        let resp = crate::components::text_area(
            ui,
            &p,
            "send-note",
            &mut typed,
            tr!("What the payment is for"),
            1,
        );
        if resp.changed() {
            form.type_note(&typed);
            form.result = None;
        }
        if form.note_cut {
            // N4; the number comes first in the key (`plural`), the language puts it where it
            // goes.
            let most = evolite_api::NOTE_MAX_GRAPHEMES as u32;
            let cut = crate::text::plural(
                most,
                tr!("character is all a note holds: the rest was cut off."),
                tr!("characters is all a note holds: the rest was cut off."),
            );
            ui.add(egui::Label::new(txt(&cut, 12.0, Weight::Regular, p.danger)).wrap());
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(a.icons.icon("lock", p.muted, &p, 14.0));
            ui.add(egui::Label::new(txt(NOTE_PRIVACY, 12.0, Weight::Regular, p.muted)).wrap());
        });
    });
}

/// «Note» in the summary (AC-D1, Figma 08): the last row, the note in one line cut with «…», in
/// its own direction.
fn summary_note_row(ui: &mut Ui, p: &crate::theme::Palette, note: &str) {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), 16.0), egui::Sense::hover());
    // Painted words say nothing to a screen reader: the row reads «Note: …», as `kv_row`.
    let spoken = format!("{}: {}", crate::i18n::spoken(tr!("Note")), note);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
    let label = crate::components::paint_text(
        ui,
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        tr!("Note"),
        sans(13.0, Weight::Regular),
        p.muted,
    );
    let room = rect.right() - label.right() - 16.0;
    if room <= 0.0 {
        return;
    }
    let mut job = egui::text::LayoutJob::single_section(
        crate::i18n::t(&crate::i18n::own(note)).into_owned(),
        egui::TextFormat::simple(sans(13.0, Weight::Medium), p.text),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: room,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(job);
    let at = egui::pos2(
        rect.right() - galley.size().x,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(at, galley, p.text);
}

/// The note in «Check the payment» (AC-D2): `raised`, r12, after the total and before «A
/// payment cannot be undone.»; its heading with the `document` icon, the note in the person's
/// own direction, and N15.
fn confirm_note(ui: &mut Ui, a: &mut Assets, note: &str) {
    let p = a.p;
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(a.icons.icon("document", p.muted, &p, 14.0));
                ui.label(txt(
                    tr!("Note · only for you"),
                    12.0,
                    Weight::Medium,
                    p.muted,
                ));
            });
            ui.add(
                egui::Label::new(txt(&crate::i18n::own(note), 14.0, Weight::Regular, p.text))
                    .wrap(),
            );
            ui.add(
                egui::Label::new(txt(
                    tr!("Saved to the payment on this computer; it does not go to the network."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ))
                .wrap(),
            );
        });
}

/// Why the core did not send, in words.
fn send_error(e: &CoreError) -> String {
    match e {
        CoreError::NotAvailable(why) => format!(tr!("{why}. The payment went nowhere."), why = why),
        CoreError::BadAddress => match evolite_core::network::current() {
            evolite_api::Network::Testnet => {
                tr!("This is not a Dash testnet address: check it.").into()
            }
            evolite_api::Network::Mainnet => {
                tr!("This is not a Dash mainnet address: check it.").into()
            }
        },
        CoreError::BadAmount => tr!("The amount must be more than zero.").into(),
        CoreError::NotEnough => tr!("Not enough for the amount and the fee.").into(),
        CoreError::WrongPassword => tr!("Wrong password — the payment is not signed.").into(),
        CoreError::NotSynced => {
            tr!("Wait for the sync to finish: the wallet does not know all its coins yet.").into()
        }
        other => crate::text::failed(tr!("Not sent"), other),
    }
}

impl Default for SendForm {
    /// The example from Figma.
    fn default() -> Self {
        Self {
            source: 0,
            recipient: "@alice".into(),
            amount: crate::format::input_text(25_000_000),
            suggested: Some(25_000_000),
            max: false,
            asked: String::new(),
            resolution: None,
            asked_at: None,
            confirming: false,
            waiting: 0,
            last_payment: 0,
            last_answered: 0,
            signed: String::new(),
            sent_to: String::new(),
            naming: None,
            book_error: None,
            result: None,
            password: Zeroizing::default(),
            signing: None,
            password_wrong: false,
            refocus_password: false,
            reveal_result: false,
            contact_waiting: false,
            contact_result: None,
            contact_password: Default::default(),
            previewed: String::new(),
            preview: None,
            note: String::new(),
            note_cut: false,
            signed_note: String::new(),
        }
    }
}

/// A balance to pay from, as the Send screen shows it.
struct SourceView {
    source: Source,
    icon: &'static str,
    title: &'static str,
    duffs: Duffs,
    /// The fee of the amount typed (BUG-55: from Core, of the coins it takes).
    fee: Duffs,
    /// The fee "Max" leaves room for: from Core, spending every coin.
    max_fee: Duffs,
    /// Credits to a Core address: what Platform checks before it takes the withdrawal.
    withdrawal: Option<Withdrawal>,
    note_icon: &'static str,
    note: &'static str,
}

/// How long the last look waits for the core's answer to its password before closing all the same
/// (BUG-92). The check is quick; a right password is answered only once the network has the
/// payment, and the form must be free meanwhile. A later refusal comes into view on the form.
const SIGNING_HOLD: std::time::Duration = std::time::Duration::from_secs(5);

/// The smallest withdrawal of credits to Core: Platform's own minimum, 1000 duffs
/// (`system_limits.min_withdrawal_amount`, 1 000 000 credits), plus the Core fee of the asset
/// unlock, 190 duffs (190 bytes at platform-wallet's 1 duff a byte), which Platform takes out of
/// the amount since protocol 14 (platform 575d961, `rs-dpp/src/withdrawal/mod.rs`).
/// It assumes `core_fee_per_byte = 1` (platform-wallet `withdrawal.rs:104`): should evolite pass
/// its own rate, count the minimum with `min_withdrawal_amount_with_core_fee` instead.
const MIN_WITHDRAWAL: Duffs = 1_190;

/// What Platform checks before it takes a withdrawal of credits to Core.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Withdrawal {
    /// What must stay on the balance on top of the amount. Platform refuses a balance under
    /// the amount plus its minimum fee (`Fees::credit_withdrawal_reserve`, 0.004 DASH), and the
    /// fee itself (`Fees::credit_withdrawal`, what the last one cost) comes out of the balance
    /// too: the larger of the two.
    reserve: Duffs,
    /// The smallest amount.
    min: Duffs,
}

impl Withdrawal {
    fn of(fees: &evolite_api::Fees) -> Self {
        Self {
            reserve: fees.credit_withdrawal.max(fees.credit_withdrawal_reserve),
            min: MIN_WITHDRAWAL,
        }
    }
}

/// What is wrong with the amount typed for a balance and its fee: the line under the field.
#[derive(Clone, Copy, Debug, PartialEq)]
enum AmountProblem {
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

fn amount_problem(
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

fn amount_problem_text(
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
fn max_amount(balance: Duffs, fee: Duffs, withdrawal: Option<Withdrawal>) -> Duffs {
    let (keep, _) = keep_and_min(fee, withdrawal);
    balance.saturating_sub(keep).max(0)
}

/// Credits to `to` go to a Core address — a withdrawal, which Platform pays out later — and not
/// to a name or a Platform address (a transfer).
fn credits_to_core(to: &str) -> bool {
    let to = to.trim();
    !(to.is_empty() || to.starts_with('@') || to.to_ascii_lowercase().starts_with("tdash1"))
}

/// What credits to `to` cost: a withdrawal to a Core address, else a transfer.
fn credits_fee(fees: &evolite_api::Fees, to: &str) -> Duffs {
    if credits_to_core(to) {
        fees.credit_withdrawal
    } else {
        fees.credit_transfer
    }
}

/// `to`: the recipient typed, which decides what a payment from credits or the pool costs;
/// `amount`: the amount typed (0: none yet), which decides what a payment from Core costs.
fn sources(w: &Wallet, to: &str, amount: Duffs) -> [SourceView; 3] {
    let credits = credits_fee(&w.fees, to);
    let pool = super::shielded::pool_fee(&w.fees, to);
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
fn source_detail(
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
fn as_of_caption(as_of: Option<evolite_api::CoreAsOf>, line: Option<&str>) -> Option<String> {
    match (as_of, line) {
        (Some(evolite_api::CoreAsOf::Checked(_)), Some(line)) => Some(format!("Core · {line}")),
        _ => None,
    }
}

/// What a «From» card says: credits not read yet are not a 0 (BUG-91).
fn source_line(
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
struct CardWidths {
    words: f32,
    with_icon: f32,
}

/// The column each «From» card stands in, in `width`: equal columns that hold the widest words
/// ([`from_columns`]), and the icons on every card or on none (the designer, BUG-81 r2) — on
/// all when the column holds the widest card with its icon. The columns are equal, so one
/// answer for the panel is one for each row, two plus one included.
fn from_layout(width: f32, cards: &[CardWidths]) -> Vec<CardColumn> {
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

enum Recipient<'a> {
    Empty,
    /// Waiting for the core to resolve the name.
    Checking,
    Name(&'a ResolvedName),
    /// No identity for the name, or no answer yet: why.
    NameProblem(&'a CoreError),
    Address,
}

fn recipient<'a>(form: &'a SendForm) -> Recipient<'a> {
    let input = form.recipient.trim();
    if input.is_empty() {
        return Recipient::Empty;
    }
    if !input.starts_with('@') {
        return Recipient::Address;
    }
    match &form.resolution {
        _ if form.asked != input => Recipient::Checking,
        None => Recipient::Checking,
        Some(Ok(name)) => Recipient::Name(name),
        Some(Err(e)) => Recipient::NameProblem(e),
    }
}

/// Whether `to` is written like a Core address (Base58, 25–36 characters) or a Platform one
/// (`tdash1…`/`dash1…`): only the core says it really is one, this keeps a stray `q` out of the
/// summary.
fn looks_like_address(to: &str) -> bool {
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

/// "To" in the summary: "—" until the recipient is one (`known`: a resolved name, an address
/// the core took), then the name, or the address as `yNBw…Reb4` — never a whole line running
/// over the label (BUG-32).
fn summary_recipient(typed: &str, known: bool) -> String {
    const NAME_SHOWN: usize = 24;
    let typed = typed.trim();
    if !known || typed.is_empty() {
        return "—".into();
    }
    if typed.starts_with('@') {
        if typed.chars().count() <= NAME_SHOWN {
            return typed.into();
        }
        let head: String = typed.chars().take(NAME_SHOWN - 1).collect();
        return format!("{head}…");
    }
    short_address(typed)
}

/// Whether `to` is an address of this wallet: the one to receive on now, its Platform address,
/// or one that received before.
fn own_address(w: &Wallet, to: &str) -> bool {
    own_of(&[&w.core_address, &w.platform_address], &w.activity, to)
}

/// [`own_address`] from the receive addresses `now` and the history: an address that received,
/// from someone else or from this wallet itself (a payment to its own address, BUG-68 — the
/// receive address has moved on since, as «Earlier addresses» of Receive count it).
fn own_of(now: &[&str], activity: &[Activity], to: &str) -> bool {
    let to = to.trim();
    !to.is_empty()
        && (now.contains(&to)
            || activity.iter().any(|act| {
                matches!(
                    act.kind,
                    ActivityKind::Received { .. }
                        | ActivityKind::SelfTransfer { .. }
                        | ActivityKind::PlatformReceived
                ) && act.address.as_deref() == Some(to)
            }))
}

pub fn send(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut SendForm) {
    // "A password for every payment": the core keeps it for the wallet (BUG-42).
    let ask_password = crate::components::password_asked(cx.snap);
    let p = a.p;
    // A refusal has just come: its line under the buttons is brought into view, once.
    let reveal = std::mem::take(&mut form.reveal_result);
    crate::format::fill_suggested(&mut form.amount, &mut form.suggested);
    ui.spacing_mut().item_spacing.y = 24.0;
    page_header(ui, a, tr!("Send"), None, |_, _| {});

    // A name goes to the core once per change; the answer comes back as an event.
    let typed = form.recipient.trim().to_string();
    if typed.starts_with('@') && typed.len() > 1 && typed != form.asked {
        cx.send(Command::ResolveName {
            name: typed.clone(),
        });
        form.asked = typed.clone();
        form.resolution = None;
        form.asked_at = Some(std::time::Instant::now());
    }

    // The fee from Core is of the coins the amount takes: "Max" spends them all (BUG-55).
    let for_fee = if form.max {
        Duffs::MAX
    } else {
        crate::format::parse_amount(&form.amount).unwrap_or(0)
    };
    let all = sources(cx.wallet(), &typed, for_fee);
    // Core not checked in this sync: its last checked balance and when, or «not known yet»
    // (BUG-77); a payment waits for the sync all the same.
    let core_as_of = cx.wallet().balances.core_as_of;
    let core_as_of_line = crate::text::core_as_of(core_as_of, cx.now);
    let source = &all[form.source];
    form.follow_max(source.duffs, source.max_fee, source.withdrawal);
    // An address for Core is checked as it is typed, so a typo shows before the password.
    let typed_address =
        source.source == Source::Core && !typed.is_empty() && !typed.starts_with('@');
    if typed_address && typed != form.previewed {
        cx.send(Command::PreviewPayment { to: typed.clone() });
        form.previewed = typed.clone();
        form.preview = None;
    }
    let address_problem = match &form.preview {
        // Nothing was being sent yet: the reason alone (the catalogs translate it with the full
        // stop after it, BUG-35).
        Some(Err(CoreError::NotAvailable(why))) if typed_address && form.previewed == typed => {
            Some(format!("{}.", why.trim_end_matches('.')))
        }
        Some(Err(e)) if typed_address && form.previewed == typed => Some(send_error(e)),
        _ => None,
    };
    let amount = crate::format::parse_amount(&form.amount).filter(|d| *d > 0);
    let left = amount.map(|d| crate::format::left_after(source.duffs, d, source.fee));
    // Not before the wallet knows all its coins: the core would refuse, and a balance
    // still syncing is no basis for a payment (nor for saying it is too small).
    let synced = crate::text::coins_known(cx.snap.sync.stage);
    let problem = amount_problem(&form.amount, source.duffs, source.fee, source.withdrawal)
        .filter(|p| synced || *p != AmountProblem::NoFunds);
    // A source that cannot pay at all says why, not "more than there is".
    let unavailable = match source.source {
        Source::Credits if cx.wallet().identity.is_none() => Some(tr!(
            "Only an identity has credits, and the wallet has none yet: make one on the “Identity” screen."
        )),
        Source::Pool if cx.wallet().pool.address.is_none() => Some(tr!(
            "The private pool connects after syncing with Platform — usually a minute."
        )),
        // A private address in a DashPay profile is an open question.
        Source::Pool if typed.starts_with('@') => Some(tr!(
            "From the pool you can send to an evolite private address (the amount stays hidden), to a Platform address or to a Core address (the amount becomes visible)."
        )),
        _ => None,
    };
    let to = recipient(form);
    let resolved_name = match &to {
        Recipient::Name(n) => Some((*n).clone()),
        _ => None,
    };
    // Credits to the wallet's own identity would only cost the fee.
    let to_self = all[form.source].source == Source::Credits
        && resolved_name.as_ref().is_some_and(|n| {
            cx.wallet()
                .identity
                .as_ref()
                .is_some_and(|i| i.id == n.identity_id)
        });
    let target_known = matches!(to, Recipient::Name(_) | Recipient::Address)
        && !to_self
        && address_problem.is_none();
    let name_problem = match &to {
        Recipient::NameProblem(e) => Some(name_problem(e)),
        _ => None,
    };
    let checking = matches!(to, Recipient::Checking);
    // Core pays a name only as a DashPay contact: to the addresses its answer to our request
    // gave us. Until then the form explains and offers the request instead of an amount.
    let contact_fee = cx.wallet().fees.contact_request;
    let not_contact = all[form.source].source == Source::Core
        && resolved_name
            .as_ref()
            .is_some_and(|n| n.contact != Some(ContactState::Established));
    let mut asked: Vec<Command> = Vec::new();
    let mut open_contacts = false;

    // Side by side when the form keeps at least 420 px, otherwise the summary goes below.
    let summary_w = 320.0;
    let side = ui.available_width() - summary_w - 24.0 >= 420.0;
    let form_w = if side {
        ui.available_width() - summary_w - 24.0
    } else {
        ui.available_width()
    };

    let recipient_label = form.recipient.trim().to_string();
    // The address book: Core addresses the person named, to pick or to keep.
    let book = cx.wallet().address_book.clone();
    let address_ok =
        typed_address && form.previewed == typed && matches!(&form.preview, Some(Ok(_)));
    // The summary names the recipient only once it is one (BUG-39 №5), short (BUG-32).
    let known = match &to {
        Recipient::Name(_) => true,
        Recipient::Address => {
            looks_like_address(&typed) && (source.source != Source::Core || address_ok)
        }
        _ => false,
    };
    let summary_to = summary_recipient(&typed, known);
    // The note, last in the summary (AC-D1); as it was when the frame began.
    let summary_note = form.note_to_send();
    // Nothing to keep in the address book: it is this wallet's own (BUG-39 №9).
    let own = own_address(cx.wallet(), &typed);
    // The kind of the last payment signed, for its answer: "To" may be cleared or new by then.
    let sent_to = form.sent_to.trim().to_string();
    let mut draw_form = |ui: &mut Ui, a: &mut Assets| {
        card(&p, Margin::same(24)).show(ui, |ui| {
            ui.set_width(form_w - 50.0);
            ui.spacing_mut().item_spacing.y = 18.0;
            // «From» and, on its right, the app's one «Hide amounts» (FEAT-0007): the balances
            // of the sources and «Left» hide, the amount typed and the fee do not.
            ui.horizontal(|ui| {
                field_label(ui, &p, tr!("From"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    crate::components::amount_toggle(ui, a, false);
                });
            });
            ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                // Equal columns in one row (the designer's summary №25), a long Core balance
                // too (BUG-67); every card whole inside the panel, its words with the padding
                // (BUG-81): as many columns as hold the widest words, without icons if need be.
                let details: Vec<String> = all
                    .iter()
                    .map(|s| {
                        source_line(
                            s,
                            cx.wallet().balances.credits_read,
                            core_as_of,
                            core_as_of_line.as_deref(),
                        )
                    })
                    .collect();
                let widths: Vec<CardWidths> = all
                    .iter()
                    .zip(&details)
                    .map(|(s, d)| CardWidths {
                        words: option_card_width(ui, s.title, d, false),
                        with_icon: option_card_width(ui, s.title, d, true),
                    })
                    .collect();
                let columns = from_layout(ui.available_width(), &widths);
                for (i, ((s, detail), column)) in
                    all.iter().zip(&details).zip(columns).enumerate()
                {
                    let selected = i == form.source;
                    if option_card_in(ui, a, s.icon, s.title, detail, selected, column)
                        .clicked()
                    {
                        form.source = i;
                        form.result = None;
                    }
                }
            });
            if let Some(caption) = as_of_caption(core_as_of, core_as_of_line.as_deref()) {
                ui.add(egui::Label::new(txt(&caption, 12.0, Weight::Regular, p.muted)).wrap());
            }
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("To"));
                if text_field(
                    ui,
                    &p,
                    "send-recipient",
                    &mut form.recipient,
                    tr!("@name or Dash address"),
                    FieldKind::Text,
                    |_| {},
                )
                .changed()
                {
                    form.result = None;
                }
            });
            if let Some(name) = &resolved_name {
                resolved(ui, a, name, all[form.source].source, &mut asked);
                if to_self {
                    crate::components::attention_line(ui, a, tr!("This is your identity: the credits would stay yours, only the fee would go."));
                }
            } else if checking {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                ui.add(egui::Label::new(txt(crate::text::name_waiting(form.asked_at), 12.0, Weight::Regular, p.muted)).wrap());
            } else if let Some((why, typo)) = &name_problem {
                let color = if *typo { p.danger } else { p.muted };
                ui.label(txt(why, 12.0, Weight::Regular, color));
            } else if let Some(why) = &address_problem {
                ui.add(egui::Label::new(txt(why, 12.0, Weight::Regular, p.danger)).wrap());
            }
            if all[form.source].source == Source::Core {
                if own && address_ok {
                    ui.add(
                        egui::Label::new(txt(
                            tr!("This is an address of this wallet: the money stays yours, only the fee goes."),
                            12.0,
                            Weight::Regular,
                            p.muted,
                        ))
                        .wrap(),
                    );
                }
                address_book(ui, a, form, &book, address_ok && !own, &mut asked);
            }
            if let (true, Some(name)) = (not_contact, &resolved_name) {
                let who = name.display.clone().unwrap_or_else(|| format!("@{}", name.name));
                let (text, action) = match name.contact {
                    Some(ContactState::Outgoing) => (
                        format!(tr!("{who} already has your contact request. When {who} answers, you can pay from here. Credits can go to @{} right away: choose “Credits”."), name.name, who = who),
                        None,
                    ),
                    Some(ContactState::Incoming) => (
                        format!(tr!("{who} sent you a contact request: accept it, and you can pay from here."), who = who),
                        Some(tr!("Open contacts").to_string()),
                    ),
                    _ => (
                        format!(tr!("A payment from Core to an @name goes to addresses from the key the person sends in answer to a contact request. Send {who} a request — once they answer, you can pay from here. Credits can go to @{} right away: choose “Credits”."), name.name, who = who),
                        Some(format!(tr!("Add to contacts · ≈ {}"), crate::format::amount(contact_fee))),
                    ),
                };
                notice(ui, a, "contacts", p.accent, &text);
                if let Some(Err(e)) = &form.contact_result
                    && !form.contact_password.is_wrong()
                {
                    crate::components::error_notice(ui, a, &super::contacts::contact_error(e), crate::text::error_details(e).as_deref());
                }
                // The request costs credits: "password for every payment" asks here too.
                let ask = action.is_some()
                    && name.contact != Some(ContactState::Incoming)
                    && crate::components::password_asked(cx.snap);
                let entered = form.contact_password.field(ui, &p, "send-contact-password", ask);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if let Some(action) = action {
                        let label = if form.contact_waiting { tr!("Sending…").to_string() } else { action };
                        let ready = !form.contact_waiting && form.contact_password.ready(ask);
                        if button_enabled(ui, a, &label, ButtonKind::Primary, None, ready)
                            .clicked() || (entered && ready)
                        {
                            if name.contact == Some(ContactState::Incoming) {
                                open_contacts = true;
                            } else {
                                asked.push(form.contact_password.command(
                                    Spend::AddContact {
                                        name: format!("@{}", name.name),
                                    },
                                    ask,
                                ));
                                form.contact_waiting = true;
                                form.contact_result = None;
                            }
                        }
                    }
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        form.replace(SendForm::empty());
                    }
                });
                return;
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("Amount"));
                let field = amount_field(ui, &p, "send-amount", &mut form.amount, true);
                let (resp, all) = (field.response, field.all);
                if resp.changed() && !all {
                    form.max = false;
                }
                if all {
                    form.max = true;
                    form.follow_max(source.duffs, source.max_fee, source.withdrawal);
                }
                if resp.changed() || all {
                    form.result = None;
                }
                if let Some(why) = unavailable {
                    ui.add(
                        egui::Label::new(txt(why, 12.0, Weight::Regular, p.muted)).wrap(),
                    );
                } else if let Some(problem) = problem {
                    ui.add(
                        egui::Label::new(txt(
                            &amount_problem_text(problem, source.fee, source.withdrawal),
                            12.0,
                            Weight::Regular,
                            p.danger,
                        ))
                        .wrap(),
                    );
                }
            });
            note_field(ui, a, form);
            kv_row(
                ui,
                &p,
                tr!("Network fee"),
                &crate::format::amount(source.fee),
                mono(13.0, Weight::Regular),
            );
            // Credits or the pool to a Core address are a withdrawal: they arrive later, not in
            // seconds.
            let withdrawal = matches!(source.source, Source::Credits | Source::Pool)
                && !recipient_label.is_empty()
                && !recipient_label.starts_with('@')
                && !recipient_label.starts_with("tdash1");
            if withdrawal && source.source == Source::Pool {
                notice(
                    ui,
                    a,
                    "pending",
                    p.accent,
                    tr!("Withdrawal from the pool to Core: the amount becomes visible on the network; the DASH arrive at this address in a few minutes, when Platform releases a batch of withdrawals."),
                );
            } else if withdrawal {
                notice(
                    ui,
                    a,
                    "pending",
                    p.accent,
                    tr!("Withdrawing credits to Core: the DASH arrive at this address in a few minutes, when Platform releases a batch of withdrawals."),
                );
            } else {
                notice(ui, a, source.note_icon, p.accent, source.note);
            }
            let ready = amount.is_some()
                && problem.is_none()
                && unavailable.is_none()
                && target_known
                && synced;
            if form.waiting > 0 {
                ui.add(
                    egui::Label::new(txt(
                        tr!("The payment awaits the network's answer: it is in the history as “Sending”, and the answer will come. The form is free for the next one."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ))
                    .wrap(),
                );
            }
            // A grey button says nothing by itself: what is still missing — under the buttons,
            // wrapped, so a long reason does not widen the form past the window (BUG-36).
            let missing = if ready {
                None
            } else if !synced {
                Some(crate::text::WAIT_FOR_SYNC)
            } else if form.recipient.trim().is_empty() {
                Some(tr!("Enter whom to pay"))
            } else if form.amount.trim().is_empty() {
                Some(tr!("Enter the amount"))
            } else if checking {
                Some(tr!("Checking the recipient…"))
            } else {
                None
            };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let label = tr!("Check and send");
                if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked() {
                    form.confirming = true;
                    form.result = None;
                    // From Core the last look shows where the money goes: the address, or the
                    // contact's next address from its key, as the core will pay it.
                    form.preview = None;
                    form.previewed = form.recipient.trim().to_string();
                    if source.source == Source::Core {
                        asked.push(Command::PreviewPayment {
                            to: form.previewed.clone(),
                        });
                    }
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    form.replace(SendForm::empty());
                }
            });
            if let Some(missing) = missing {
                ui.add(egui::Label::new(txt(missing, 12.0, Weight::Regular, p.muted)).wrap());
            }
            let answer = ui.scope(|ui| match &form.result {
                // No clear answer from the network: the payment may have gone.
                Some(Ok(sent)) if !sent.relayed => notice(
                    ui,
                    a,
                    "pending",
                    p.muted,
                    &format!(
                        tr!("The payment was handed to the nodes, but the network has not confirmed it yet. Do not send it again: it may go through. In the history it is “Sending”.{}"),
                        if sent.txid.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", short_address(&sent.txid))
                        }
                    ),
                ),
                // Credits: Platform proves the transition, there is no Core transaction.
                Some(Ok(sent)) if sent.txid.is_empty() && source.source == Source::Pool => {
                    let withdrawal = !sent_to.starts_with("tdash1");
                    notice(
                        ui,
                        a,
                        "confirmed",
                        p.accent,
                        if withdrawal {
                            tr!("Platform accepted the withdrawal from the pool. The money arrives at the Core address in a few blocks, when the masternodes pay it out.")
                        } else {
                            tr!("Platform accepted the transfer from the pool.")
                        },
                    )
                }
                Some(Ok(sent)) if sent.txid.is_empty() => {
                    let withdrawal = !sent_to.starts_with('@')
                        && !sent_to.starts_with("tdash1");
                    notice(
                        ui,
                        a,
                        "confirmed",
                        p.accent,
                        if withdrawal {
                            tr!("Platform accepted the credit withdrawal. The money arrives at the Core address in a few blocks, when the masternodes pay it out.")
                        } else {
                            tr!("Credits sent: Platform confirmed the transfer with a proof.")
                        },
                    )
                }
                Some(Ok(sent)) => notice(
                    ui,
                    a,
                    "confirmed",
                    p.accent,
                    &format!(
                        tr!("Payment sent, the network accepted it. Fee {} · {}"),
                        crate::format::amount(sent.fee),
                        short_address(&sent.txid)
                    ),
                ),
                Some(Err(e @ CoreError::NotAvailable(_))) => {
                    notice(ui, a, "info", p.muted, &send_error(e))
                }
                Some(Err(e)) => crate::components::error_notice(ui, a, &send_error(e), crate::text::error_details(e).as_deref()),
                None => {}
            });
            if reveal {
                answer.response.scroll_to_me(Some(Align::Center));
            }
        });
    };

    let draw_summary = |ui: &mut Ui| {
        card(&p, Margin::same(24)).show(ui, |ui| {
            ui.set_width(if side { summary_w - 50.0 } else { form_w - 50.0 });
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.label(txt(tr!("Summary"), 16.0, Weight::SemiBold, p.text));
            let value = mono(13.0, Weight::Medium);
            let dash = |d: Option<i64>| d.map_or("—".into(), crate::format::amount);
            kv_row(ui, &p, tr!("From"), source.title, sans(13.0, Weight::Medium));
            kv_row(
                ui,
                &p,
                tr!("To"),
                &summary_to,
                sans(13.0, Weight::Medium),
            );
            // Nothing is paid until the name is a contact.
            let (amount, fee, left) = if not_contact {
                (None, None, Some(source.duffs))
            } else {
                (amount, Some(source.fee), left.filter(|l| *l >= 0))
            };
            kv_row(ui, &p, tr!("Amount"), &dash(amount), value.clone());
            kv_row(ui, &p, tr!("Fee"), &dash(fee), value.clone());
            kv_row(
                ui,
                &p,
                tr!("Left"),
                &left.map_or("—".into(), crate::format::balance),
                value,
            );
            if let Some(note) = &summary_note {
                summary_note_row(ui, &p, note);
            }
            divider(ui, &p);
            ui.add(
                egui::Label::new(
                    txt(
                        tr!("A payment cannot be undone. Before signing, evolite shows the recipient's address once more."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(Some(18.0)),
                )
                .wrap(),
            );
        });
    };

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            if side {
                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 24.0;
                    ui.vertical(|ui| draw_form(ui, a));
                    ui.vertical(|ui| draw_summary(ui));
                });
            } else {
                ui.spacing_mut().item_spacing.y = 24.0;
                draw_form(ui, a);
                draw_summary(ui);
            }
        });
    for command in asked {
        cx.send(command);
    }
    if open_contacts {
        cx.go(super::Screen::Contacts);
    }

    // While its answer is awaited the amount has left the form for the payment (BUG-92).
    let signed = form
        .signing
        .and_then(|_| crate::format::parse_amount(&form.signed).filter(|d| *d > 0));
    if form.confirming
        && let Some(amount) = amount.or(signed)
    {
        confirm(ui, a, cx, form, amount, ask_password);
    }
}

/// The address-book entries to offer for what is typed in "To": all of them while it is
/// empty, those whose name or address holds it while it is being typed, none once it is a whole
/// saved address or a name (`@`).
fn book_matches<'a>(book: &'a [SavedAddress], typed: &str) -> Vec<&'a SavedAddress> {
    let typed = typed.trim().to_lowercase();
    if typed.starts_with('@') || book.iter().any(|e| e.address.to_lowercase() == typed) {
        return Vec::new();
    }
    book.iter()
        .filter(|e| {
            typed.is_empty()
                || e.label.to_lowercase().contains(&typed)
                || e.address.to_lowercase().contains(&typed)
        })
        .collect()
}

/// A muted text that can be clicked.
fn link(ui: &mut Ui, p: &crate::theme::Palette, text: &str) -> egui::Response {
    ui.add(egui::Label::new(txt(text, 12.0, Weight::Medium, p.muted)).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Under "To", paying from Core: saved addresses to pick; for a saved one its name, to rename
/// or remove; for a good address not saved yet, "Save to the address book" and its name.
fn address_book(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    book: &[SavedAddress],
    address_ok: bool,
    asked: &mut Vec<Command>,
) {
    let p = a.p;
    let typed = form.recipient.trim().to_string();
    let saved = book.iter().find(|e| e.address == typed);
    let offer = book_matches(book, &typed);
    if !offer.is_empty() {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field_label(ui, &p, tr!("Address book"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                for e in offer {
                    let label = format!("{} · {}", e.label, short_address(&e.address));
                    if pill(ui, &p, &label, false)
                        .on_hover_text(crate::i18n::t(&e.address).into_owned())
                        .clicked()
                    {
                        form.recipient = e.address.clone();
                        form.result = None;
                        form.naming = None;
                    }
                }
            });
        });
    }
    if form.naming.is_none() {
        if let Some(e) = saved {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.label(txt(
                    &format!(
                        tr!("“{}” · from the address book"),
                        crate::i18n::own(&e.label)
                    ),
                    12.0,
                    Weight::Medium,
                    p.accent,
                ));
                if link(ui, &p, tr!("Rename")).clicked() {
                    form.naming = Some(e.label.clone());
                    form.book_error = None;
                }
                if link(ui, &p, tr!("Remove from the book")).clicked() {
                    asked.push(Command::ForgetAddress {
                        address: e.address.clone(),
                    });
                }
            });
        } else if address_ok && link(ui, &p, tr!("Save to the address book")).clicked() {
            form.naming = Some(String::new());
            form.book_error = None;
        }
    }
    let mut cancel = false;
    if let Some(name) = &mut form.naming {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let field_w = (ui.available_width() - 230.0).max(160.0);
            ui.allocate_ui(vec2(field_w, 44.0), |ui| {
                text_field(
                    ui,
                    &p,
                    "send-book-name",
                    name,
                    tr!("A label: exchange, rent, Masha…"),
                    FieldKind::Text,
                    |_| {},
                );
            });
            let ready = !name.trim().is_empty();
            if button_enabled(ui, a, tr!("Save"), ButtonKind::Secondary, None, ready).clicked() {
                asked.push(Command::SaveAddress {
                    address: typed.clone(),
                    label: name.trim().to_owned(),
                });
            }
            if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                cancel = true;
            }
        });
    }
    if cancel {
        form.naming = None;
        form.book_error = None;
    }
    if let Some(e) = &form.book_error {
        ui.label(txt(
            &crate::text::failed(tr!("Not saved"), e),
            12.0,
            Weight::Regular,
            p.danger,
        ));
    }
}

/// A payment of this amount (with at most `fee` on top) to `to` within the last hour: signing
/// it again may be a repeat of one that already went.
fn similar_payment<'a>(
    activity: &'a [Activity],
    to: &str,
    amount: Duffs,
    fee: Duffs,
    now: Timestamp,
) -> Option<&'a Activity> {
    activity.iter().find(|act| {
        matches!(&act.kind, ActivityKind::Sent { to: paid } if paid == to)
            && now - act.at < 60 * 60
            && (amount..=amount.saturating_add(fee)).contains(&-act.amount)
    })
}

/// The last look before signing: where the money comes from, who gets it and the full address
/// it goes to (from Core the core says which: a contact's is the next address of its key), the
/// amount and the most the fee can be; with `ask_password`, the wallet password too.
fn confirm(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut SendForm,
    amount: Duffs,
    ask_password: bool,
) {
    let p = a.p;
    let all = sources(cx.wallet(), form.recipient.trim(), amount);
    let source = &all[form.source];
    let to = form.recipient.trim().to_string();
    let from_core = source.source == Source::Core;
    // Shown, and signed, only once the core has said where the payment goes.
    let address = match &form.preview {
        Some(Ok(address)) if form.previewed == to => Some(address.clone()),
        _ => None,
    };
    let resolved = match &form.resolution {
        Some(Ok(name)) if form.asked == to => Some(name.clone()),
        _ => None,
    };
    let repeat = similar_payment(
        &cx.wallet().activity,
        &to,
        amount,
        source.fee,
        cx.now.timestamp(),
    )
    // Once signed the payment is in the history itself: it is no repeat of itself (BUG-92 r1).
    .filter(|_| form.signing.is_none())
    .map(|act| crate::text::when(act.at, cx.now));
    // An address only like one from the book or the history: a planted lookalike.
    let lookalike = {
        let w = cx.wallet();
        let known = w.address_book.iter().map(|s| s.address.as_str()).chain(
            w.activity
                .iter()
                .filter(|act| act.amount < 0)
                .filter_map(|act| act.address.as_deref()),
        );
        crate::text::lookalike(&to, known).map(crate::text::grouped)
    };
    let signing = form.signing.is_some();
    // A slow answer does not hold the dialog for ever: the form says it is awaited.
    let timed_out = form.signing.is_some_and(|at| at.elapsed() >= SIGNING_HOLD);
    if signing {
        ui.ctx().request_repaint_after(SIGNING_HOLD);
    }
    let mut send_now = false;
    let closed = dialog(
        ui,
        a,
        "send-confirm",
        "send",
        tr!("Check the payment"),
        420.0,
        |ui, a| {
            ui.spacing_mut().item_spacing.y = 14.0;
            let value = mono(13.0, Weight::Medium);
            let dash = |d: Duffs| crate::format::amount(d);
            kv_row(
                ui,
                &p,
                tr!("From"),
                &format!(
                    "{} · {}",
                    source.title,
                    crate::format::balance(source.duffs)
                ),
                sans(13.0, Weight::Medium),
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("Recipient"));
                // An address in groups of four: compared with the one meant, group by group.
                let shown_to = if to.starts_with('@') {
                    to.clone()
                } else {
                    crate::text::grouped(&to)
                };
                let who = match resolved.as_ref().and_then(|n| n.display.as_deref()) {
                    Some(display) => format!("{shown_to} · {display}"),
                    None => shown_to,
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(crate::i18n::t(&who))
                            .font(mono(14.0, Weight::Medium))
                            .color(p.text),
                    )
                    .wrap(),
                );
                if let Some(similar) = &lookalike {
                    ui.add(
                        egui::Label::new(txt(
                            &format!(
                                tr!("Careful: this address only looks like {} that you paid or saved before — the middle is different. Compare it character by character."),
                                similar
                            ),
                            12.0,
                            Weight::Medium,
                            p.danger,
                        ))
                        .wrap(),
                    );
                }
                if from_core {
                    match (&address, &form.preview) {
                        (Some(address), _) => {
                            // A contact (by name or id) is paid to an address of its key.
                            let line = if *address != to {
                                format!(
                                    tr!("to an address from the contact's key: {address}"),
                                    address = crate::text::grouped(address)
                                )
                            } else {
                                format!(
                                    tr!("to address {address}"),
                                    address = crate::text::grouped(address)
                                )
                            };
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(crate::i18n::t(&line))
                                        .font(mono(12.0, Weight::Regular))
                                        .color(p.muted),
                                )
                                .wrap(),
                            );
                        }
                        (None, Some(Err(e))) => {
                            ui.add(
                                egui::Label::new(txt(
                                    &send_error(e),
                                    12.0,
                                    Weight::Regular,
                                    p.danger,
                                ))
                                .wrap(),
                            );
                        }
                        (None, _) => {
                            ui.label(txt(
                                tr!("Finding the address the payment goes to…"),
                                12.0,
                                Weight::Regular,
                                p.muted,
                            ));
                        }
                    }
                } else if let Some(name) = &resolved {
                    ui.label(txt(
                        &format!(tr!("identity {} gets the credits"), name.identity_id),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
            });
            kv_row(ui, &p, tr!("Amount"), &dash(amount), value.clone());
            kv_row(
                ui,
                &p,
                tr!("Fee, at most"),
                &dash(source.fee),
                value.clone(),
            );
            divider(ui, &p);
            kv_row(
                ui,
                &p,
                tr!("In total"),
                &dash(amount.saturating_add(source.fee)),
                value,
            );
            if let Some(note) = form.note_shown() {
                confirm_note(ui, a, &note);
            }
            ui.label(txt(
                tr!("A payment cannot be undone."),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            if let Some(when) = &repeat {
                crate::components::attention_notice(
                    ui,
                    a,
                    &format!(
                        tr!(
                            "Such a payment to {to} is already in the history ({when}). Check that this is not a repeat: the last one may not have arrived yet."
                        ),
                        to = to,
                        when = when
                    ),
                );
            }
            // Enter in the password field signs, as the button does: the row presses its
            // primary (`components::button_row`).
            if ask_password {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    field_label(ui, &p, tr!("Wallet password"));
                    let field = text_field(
                        ui,
                        &p,
                        "send-password",
                        &mut form.password,
                        "",
                        FieldKind::Password,
                        |_| {},
                    );
                    if field.changed() {
                        form.password_wrong = false;
                    }
                    if std::mem::take(&mut form.refocus_password) {
                        field.request_focus();
                    }
                    if form.password_wrong {
                        ui.add(
                            egui::Label::new(txt(
                                &send_error(&CoreError::WrongPassword),
                                12.0,
                                Weight::Regular,
                                p.danger,
                            ))
                            .wrap(),
                        );
                    }
                });
            }
            let mut close = false;
            crate::components::button_row_spend(ui, |ui| {
                // It is with the core: nothing to cancel any more, only its answer to wait for.
                if !signing && button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                let ready = !signing
                    && (!ask_password || !form.password.is_empty())
                    && (!from_core || address.is_some());
                let clicked = button_enabled(
                    ui,
                    a,
                    if signing {
                        tr!("Sending…")
                    } else {
                        tr!("Sign and send")
                    },
                    ButtonKind::Primary,
                    None,
                    ready,
                )
                .clicked();
                if ready && clicked {
                    send_now = true;
                    // With a password its answer decides, when it can only be this payment's:
                    // `Event::Sent` says not whose it is, so with another still unanswered the
                    // dialog closes and the answers go to the form (BUG-92 r1).
                    close = !ask_password || form.waiting > 0;
                }
            });
            close || timed_out
        },
    );
    if send_now {
        form.sent_to = to.clone();
        // The password goes to the core and is wiped here at once.
        form.last_payment += 1;
        cx.send(Command::Spend {
            op: Spend::Send {
                id: form.last_payment,
                from: source.source,
                to,
                amount,
                note: form.note_to_send().map(evolite_api::NoteText),
            },
            password: ask_password.then(|| Password::new(form.password.to_string())),
        });
        form.waiting += 1;
        // Not twice by a second click: the amount leaves the form with the payment.
        form.signed = std::mem::take(&mut form.amount);
        // And its note, which is this payment's only.
        form.signed_note = std::mem::take(&mut form.note);
        form.note_cut = false;
        // A wrong password is said here, at its field (BUG-92): the dialog waits for the answer.
        form.signing = (ask_password && form.waiting == 1).then(std::time::Instant::now);
        form.password_wrong = false;
    }
    if closed {
        form.confirming = false;
        form.signing = None;
        form.password_wrong = false;
        form.password.zeroize();
    }
}

/// The name resolved through DPNS: its avatar and profile name, where the money goes, and
/// "Name verified" — or, paying from Core someone not yet a contact, "Not a contact".
fn resolved(ui: &mut Ui, a: &mut Assets, c: &ResolvedName, from: Source, asked: &mut Vec<Command>) {
    let p = a.p;
    let contact = c.contact == Some(ContactState::Established);
    let (goes, mark, icon, border) = match (from, contact) {
        (Source::Core, true) => (
            tr!("the payment goes to a new address from the contact's key"),
            tr!("Name verified"),
            "confirmed",
            p.accent,
        ),
        (Source::Core, false) => (
            tr!("not in your contacts"),
            tr!("Not a contact"),
            "pending",
            p.line,
        ),
        _ => (
            tr!("the owner's identity gets the credits"),
            tr!("Name verified"),
            "confirmed",
            p.accent,
        ),
    };
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(12)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (face, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
                let label = c.display.clone().unwrap_or_else(|| c.name.clone());
                let picture = c.profile.as_ref().and_then(|pr| pr.avatar.as_deref());
                crate::components::avatar(
                    ui,
                    a,
                    face,
                    &c.identity_id,
                    &label,
                    picture,
                    &mut |cmd| asked.push(cmd),
                );
                // The mark first, at the right: the two lines take what is left and end in "…"
                // there, whole on hover, instead of running under "Name verified".
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(txt(mark, 12.0, Weight::SemiBold, p.accent));
                    ui.add(a.icons.icon(icon, p.accent, &p, 18.0));
                    ui.add_space(12.0);
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(
                            egui::Label::new(txt(
                                &match &c.display {
                                    Some(display) => format!("@{} · {display}", c.name),
                                    None => format!("@{}", c.name),
                                },
                                14.0,
                                Weight::SemiBold,
                                p.text,
                            ))
                            .truncate(),
                        );
                        ui.add(
                            egui::Label::new(txt(
                                &format!("identity {} · {goes}", short_address(&c.identity_id)),
                                12.0,
                                Weight::Regular,
                                p.muted,
                            ))
                            .truncate(),
                        );
                    });
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-81 r2 (the designer): a row shows icons on all its cards or on none. At a 152 px
    /// column the Credits card holds its icon (150.4) and Core does not (157): none get one;
    /// a wider panel gives them to all; two plus one is decided once too.
    #[test]
    fn every_row_of_sources_shows_icons_on_all_cards_or_on_none() {
        let card = |words: f32, with_icon: f32| CardWidths { words, with_icon };
        let mixed = [card(129.0, 157.0), card(122.4, 150.4), card(122.4, 150.4)];
        let icons = |width: f32, cards: &[CardWidths]| -> Vec<bool> {
            from_layout(width, cards).iter().map(|c| c.icon).collect()
        };
        assert!(from_layout(474.0, &mixed).iter().all(|c| c.width == 152.0));
        assert_eq!(icons(474.0, &mixed), [false, false, false]);
        // Wide enough for the widest with its icon: on all.
        assert_eq!(icons(490.0, &mixed), [true, true, true]);
        // Two in the first row, one under them: the same column, the same answer.
        let wide = [card(160.0, 240.0), card(120.0, 148.0), card(120.0, 148.0)];
        assert!(from_layout(474.0, &wide).iter().all(|c| c.width == 233.0));
        assert_eq!(icons(474.0, &wide), [false, false, false]);
        for width in [300.0, 400.0, 474.0, 480.0, 520.0, 700.0] {
            for cards in [&mixed[..], &wide[..], &mixed[..2]] {
                let row = icons(width, cards);
                assert!(
                    row.iter().all(|&i| i == row[0]),
                    "{width}: a mixed row {row:?}"
                );
            }
        }
    }

    /// FEAT-0003 (AC-A1, AC-D1): a note is cut to 140 characters as a person sees them, and
    /// says so until the next change; a family emoji is one character.
    #[test]
    fn a_note_longer_than_it_holds_is_cut_and_says_so() {
        let mut form = SendForm::empty();
        form.type_note(&"ж".repeat(150));
        assert_eq!(evolite_core::note_text::graphemes(&form.note), 140);
        assert!(form.note_cut);
        form.type_note("За дизайн 👨‍👩‍👧‍👦\u{202E}");
        assert_eq!(form.note, "За дизайн 👨‍👩‍👧‍👦");
        assert!(!form.note_cut, "gone at the next change");
        assert_eq!(form.note_to_send().as_deref(), Some("За дизайн 👨‍👩‍👧‍👦"));
        form.type_note("   ");
        assert_eq!(form.note_to_send(), None);
    }

    /// FEAT-0003 (AC-A3): the note leaves the form with the payment; a refusal puts it back
    /// (nothing was kept), a payment taken keeps it with the core, and a lock wipes it.
    #[test]
    fn a_refused_payment_gives_the_note_back_and_a_lock_wipes_it() {
        let mut form = SendForm::empty();
        form.type_note("rent");
        form.signed_note = std::mem::take(&mut form.note);
        form.waiting = 1;
        form.on_sent(0, &Err(CoreError::NotEnough));
        assert_eq!(form.note, "rent");
        form.signed_note = std::mem::take(&mut form.note);
        form.waiting = 1;
        form.on_sent(
            0,
            &Ok(SentPayment {
                txid: "aa".into(),
                amount: 1,
                fee: 1,
                relayed: true,
            }),
        );
        assert!(form.note.is_empty() && form.signed_note.is_empty());
        form.type_note("typed, not sent");
        form.on_lock();
        assert!(form.note.is_empty() && form.signed_note.is_empty() && !form.note_cut);
    }

    /// BUG-77: the Core source says how sure its balance is — the last checked one and when, or
    /// only «not known yet» (no 0 as if checked); the other sources are as they were.
    #[test]
    fn the_core_source_says_when_its_balance_was_checked() {
        use evolite_api::CoreAsOf;
        let view = |source| SourceView {
            source,
            icon: "core",
            title: "Core",
            duffs: 386_574_464,
            fee: 0,
            max_fee: 0,
            withdrawal: None,
            note_icon: "",
            note: "",
        };
        let balance = crate::format::balance(386_574_464);
        let core = view(Source::Core);
        assert_eq!(source_detail(&core, None, None), balance);
        // The accessibility name of a «From» card is its title: the words are checked here.
        assert_eq!(
            source_line(&view(Source::Credits), false, None, None),
            "Not known yet"
        );
        assert_eq!(
            source_line(&view(Source::Credits), true, None, None),
            balance
        );
        assert_eq!(source_line(&core, false, None, None), balance);
        assert_eq!(
            source_detail(&core, Some(CoreAsOf::Checked(1)), Some("as of 23:41")),
            balance
        );
        assert_eq!(
            source_detail(&core, Some(CoreAsOf::NotYet), Some("Not known yet")),
            "Not known yet"
        );
        assert_eq!(
            source_detail(
                &view(Source::Credits),
                Some(CoreAsOf::NotYet),
                Some("Not known yet")
            ),
            balance,
            "credits are not Core"
        );
    }

    #[test]
    fn the_amount_line_says_what_is_wrong() {
        use crate::format::{Display, NumberStyle, Unit, set_display};
        use AmountProblem::*;
        set_display(Display {
            unit: Unit::Dash,
            style: NumberStyle::CommaDot,
            ..Display::default()
        });
        // A payment with a fee, not a withdrawal (that one is below): 0.0018 DASH and a fee of
        // 0.0021 DASH, nothing can go, whatever is typed.
        for typed in ["", "0.000000018", "0.00000018", "0.001"] {
            assert_eq!(
                amount_problem(typed, 180_000, 210_000, None),
                Some(NoFunds),
                "{typed}"
            );
        }
        assert_eq!(
            amount_problem("", 210_000, 210_000, None),
            Some(NoFunds),
            "the fee alone"
        );
        // An empty balance, or a fee not known yet: no line before typing, the usual ones after.
        assert_eq!(amount_problem("", 0, 210_000, None), None, "empty");
        assert_eq!(
            amount_problem("0.001", 0, 210_000, None),
            Some(MoreThanBalance),
            "empty, typed"
        );
        assert_eq!(amount_problem("", 0, 0, None), None, "empty, no fee");
        assert_eq!(amount_problem("", 180_000, 0, None), None, "no fee yet");
        assert_eq!(
            amount_problem("0.001", 180_000, 0, None),
            None,
            "no fee yet, typed"
        );
        // 0.01 DASH, the same fee: the edges.
        let (balance, fee) = (1_000_000, 210_000);
        let cases = [
            ("", None),
            ("0", Some(NotAnAmount)),
            ("abc", Some(NotAnAmount)),
            ("0.00000001", None),
            ("0.0079", None),
            ("0.00790001", Some(NoRoomForFee)),
            ("0.01", Some(NoRoomForFee)),
            ("0.01000001", Some(MoreThanBalance)),
            ("0.000000018", Some(TooPrecise)),
        ];
        for (typed, problem) in cases {
            assert_eq!(
                amount_problem(typed, balance, fee, None),
                problem,
                "{typed}"
            );
        }
        assert_eq!(max_amount(balance, fee, None), 790_000);
        assert_eq!(max_amount(100_000, fee, None), 0, "not below zero");
        let text = amount_problem_text(NoRoomForFee, fee, None);
        assert!(text.contains("0.0021"), "the fee is named: {text}");
        let text = amount_problem_text(TooPrecise, fee, None);
        assert!(
            text.contains("0.00000001"),
            "the smallest step is named: {text}"
        );
        set_display(Display::default());
    }

    /// Credits to Core: Platform refuses a balance under the amount + 0.004 DASH and an amount
    /// under 1190 duffs, whatever the last withdrawal cost (BUG-11, review r1).
    #[test]
    fn a_withdrawal_keeps_the_reserve_and_the_smallest_amount() {
        use crate::format::{Display, NumberStyle, Unit, set_display};
        use AmountProblem::*;
        set_display(Display {
            unit: Unit::Dash,
            style: NumberStyle::CommaDot,
            ..Display::default()
        });
        let fees = evolite_api::Fees {
            credit_withdrawal: 210_000,
            credit_withdrawal_reserve: 400_000,
            ..zero_fees()
        };
        let w = Withdrawal::of(&fees);
        assert_eq!(
            (w.reserve, w.min),
            (400_000, 1_190),
            "the reserve, not the price"
        );
        let dearer = Withdrawal::of(&evolite_api::Fees {
            credit_withdrawal: 500_000,
            ..fees.clone()
        });
        assert_eq!(
            dearer.reserve, 500_000,
            "a dearer last withdrawal is kept too"
        );
        let fee = fees.credit_withdrawal;
        let at = |typed: &str, balance: Duffs| amount_problem(typed, balance, fee, Some(w));
        // The customer's 0.0018 DASH: nothing can go, whatever is typed.
        for typed in ["", "0.000000018", "0.00000018", "0.001"] {
            assert_eq!(at(typed, 180_000), Some(NoFunds), "{typed}");
        }
        // An empty balance: no line before typing.
        assert_eq!(at("", 0), None);
        assert_eq!(at("0.001", 0), Some(MoreThanBalance));
        // The reserve and the smallest amount, to the duff.
        assert_eq!(at("", 400_000 + 1_189), Some(NoFunds), "reserve + 1189");
        assert_eq!(at("", 400_000 + 1_190), None, "reserve + 1190");
        assert_eq!(at("0.0000119", 400_000 + 1_190), None);
        assert_eq!(max_amount(400_000 + 1_190, fee, Some(w)), 1_190);
        // 0.01 DASH: the edges around balance − reserve (0.006).
        let balance = 1_000_000;
        let cases = [
            ("", None),
            ("0", Some(NotAnAmount)),
            ("0.000000018", Some(TooPrecise)),
            ("0.00000001", Some(BelowMinimum)),
            ("0.00001189", Some(BelowMinimum)),
            ("0.0000119", None),
            ("0.00599999", None),
            ("0.006", None),
            ("0.00600001", Some(NoRoomForFee)),
            // Within balance − price, but Platform refuses it: round 1 let it through.
            ("0.0079", Some(NoRoomForFee)),
            ("0.01", Some(NoRoomForFee)),
            ("0.01000001", Some(MoreThanBalance)),
        ];
        for (typed, problem) in cases {
            assert_eq!(at(typed, balance), problem, "{typed}");
        }
        // "Max": the balance less the reserve, which the form itself takes.
        let max = max_amount(balance, fee, Some(w));
        assert_eq!(max, 600_000);
        assert_eq!(at(&crate::format::input_text(max), balance), None);
        assert_eq!(max_amount(180_000, fee, Some(w)), 0);
        // The lines name the reserve and the smallest amount, not the price.
        let text = amount_problem_text(NoFunds, fee, Some(w));
        assert!(text.contains("0.004"), "{text}");
        let text = amount_problem_text(NoRoomForFee, fee, Some(w));
        assert!(text.contains("0.004"), "{text}");
        let text = amount_problem_text(BelowMinimum, fee, Some(w));
        assert!(text.contains("0.0000119"), "{text}");
        set_display(Display::default());
    }

    #[test]
    fn credits_to_core_cost_a_withdrawal() {
        let fees = evolite_api::Fees {
            credit_transfer: 777,
            credit_withdrawal: 400_000,
            ..zero_fees()
        };
        assert_eq!(credits_fee(&fees, "@alice"), 777);
        assert_eq!(
            credits_fee(&fees, "tdash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq"),
            777
        );
        assert_eq!(
            credits_fee(&fees, " yStwA1xNxEE2kv5ZVDNGSsPach8pBApWHZ"),
            400_000
        );
        assert!(credits_to_core(" yStwA1xNxEE2kv5ZVDNGSsPach8pBApWHZ"));
        for to in ["", "@alice", "TDASH1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq"] {
            assert!(
                !credits_to_core(to),
                "{to}: a transfer, no withdrawal reserve"
            );
        }
    }

    fn zero_fees() -> evolite_api::Fees {
        evolite_api::Fees {
            core_payment: 0,
            core_coins: Vec::new(),
            credit_transfer: 0,
            pool_transfer: 0,
            nft_transfer: 0,
            document_write: 0,
            shield: 0,
            identity_create: 0,
            identity_lock_min: 0,
            top_up_lock_min: 50_500,
            name_regular: 0,
            name_contested: 0,
            add_key: 0,
            contact_request: 0,
            profile: 0,
            shield_from_address: 0,
            shield_reserve: 0,
            unshield: 0,
            pool_withdrawal: 0,
            shield_from_core: 0,
            pool_top_up: 0,
            credit_withdrawal: 0,
            credit_withdrawal_reserve: 0,
        }
    }
    use evolite_api::{Chain, Proof};

    fn paid(to: &str, at: Timestamp, amount: Duffs) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Sent { to: to.into() },
            chain: Chain::Core,
            at,
            amount,
            proof: Proof::Sending,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            detail: None,
            note: None,
        }
    }

    #[test]
    fn offers_saved_addresses_by_name_or_address() {
        let book = [
            SavedAddress {
                address: "yAAA1".into(),
                label: "Rent".into(),
            },
            SavedAddress {
                address: "yBBB2".into(),
                label: "Exchange".into(),
            },
        ];
        assert_eq!(book_matches(&book, "").len(), 2, "all while empty");
        assert_eq!(book_matches(&book, "exc")[0].label, "Exchange", "by name");
        assert_eq!(book_matches(&book, "aaa")[0].label, "Rent", "by address");
        assert!(
            book_matches(&book, "yBBB2").is_empty(),
            "a saved address is chosen"
        );
        assert!(
            book_matches(&book, "@alice").is_empty(),
            "a name is not an address"
        );
    }

    #[test]
    fn a_refused_payment_gives_its_amount_back() {
        let mut form = SendForm {
            waiting: 2,
            last_payment: 2,
            signed: "0,25".into(),
            ..SendForm::empty()
        };
        // Another payment is still out: this answer may be that one's, nothing comes back (BUG-92
        // r2), and what was signed last stays for its own answer.
        form.on_sent(1, &Err(CoreError::NotEnough));
        assert_eq!((form.waiting, form.amount.as_str()), (1, ""));
        assert_eq!(form.signed, "0,25");
        assert!(
            form.result.is_some() && form.reveal_result,
            "the refusal is said"
        );
        // The last one out: it is the payment's own answer, its amount comes back.
        form.on_sent(2, &Err(CoreError::NotEnough));
        assert_eq!((form.waiting, form.amount.as_str()), (0, "0,25"));
        // Taken: the form stays as the person left it.
        form.waiting = 1;
        form.amount = "0,1".into();
        form.on_sent(
            2,
            &Ok(SentPayment {
                txid: "aa".into(),
                amount: 1,
                fee: 1,
                relayed: true,
            }),
        );
        assert_eq!((form.waiting, form.amount.as_str()), (0, "0,1"));
    }

    /// BUG-92 r2: with another payment out, a refusal gives back neither the note nor a «Max»
    /// amount: the note is kept for the last payment's answer, and «Max» would fill the form again.
    /// An amount typed by hand for the next payment is the person's and stays.
    #[test]
    fn a_refusal_with_another_payment_out_gives_nothing_back() {
        let mut form = SendForm {
            waiting: 2,
            last_payment: 2,
            signed: "0,25".into(),
            signed_note: "for B".into(),
            max: true,
            amount: "1,5".into(),
            ..SendForm::empty()
        };
        form.on_sent(1, &Err(CoreError::WrongPassword));
        assert!(
            !form.max && form.amount.is_empty(),
            "«Max» does not refill it"
        );
        assert!(form.note.is_empty());
        assert_eq!(
            (form.signed.as_str(), form.signed_note.as_str()),
            ("0,25", "for B")
        );

        let mut typed = SendForm {
            waiting: 2,
            last_payment: 2,
            amount: "0,7".into(),
            ..SendForm::empty()
        };
        typed.on_sent(1, &Err(CoreError::WrongPassword));
        assert_eq!(typed.amount, "0,7");
    }

    /// TECH-sent-id: the answer says whose it is, so a refusal of the last payment gives its amount
    /// and note back even with an earlier one still out.
    #[test]
    fn the_last_payments_refusal_comes_back_with_another_still_out() {
        let mut form = SendForm {
            waiting: 2,
            last_payment: 2,
            signed: "0,25".into(),
            signed_note: "for B".into(),
            ..SendForm::empty()
        };
        form.on_sent(2, &Err(CoreError::NotEnough));
        assert_eq!((form.waiting, form.amount.as_str()), (1, "0,25"));
        assert_eq!(form.note, "for B");
    }

    /// Two payments out, A and B (the last): a form as the last look leaves it after signing B.
    fn two_out() -> SendForm {
        SendForm {
            waiting: 2,
            last_payment: 2,
            signed: "0,25".into(),
            signed_note: "for B".into(),
            recipient: ADDRESS.into(),
            sent_to: ADDRESS.into(),
            ..SendForm::empty()
        }
    }

    fn sent() -> Result<SentPayment, CoreError> {
        Ok(SentPayment {
            txid: "aa".into(),
            amount: 1,
            fee: 1,
            relayed: true,
        })
    }

    /// R-10 r1 must 1: A taken says nothing of B, which is refused after it.
    #[test]
    fn an_earlier_payment_taken_leaves_the_last_ones_refusal_its_amount() {
        let mut form = two_out();
        form.on_sent(1, &sent());
        assert_eq!(form.recipient, ADDRESS, "B's recipient stays");
        assert_eq!(form.signed, "0,25", "B's amount waits for its answer");
        form.on_sent(2, &Err(CoreError::NotEnough));
        assert_eq!(
            (form.amount.as_str(), form.note.as_str()),
            ("0,25", "for B")
        );
        assert!(matches!(form.result, Some(Err(CoreError::NotEnough))));
    }

    /// R-10 r1 must 1: B refused stays said when A is taken after it: B did not go.
    #[test]
    fn an_earlier_payment_taken_does_not_replace_the_last_ones_refusal() {
        let mut form = two_out();
        form.on_sent(2, &Err(CoreError::NotEnough));
        form.on_sent(1, &sent());
        assert!(matches!(form.result, Some(Err(CoreError::NotEnough))));
        assert_eq!(form.recipient, ADDRESS, "to try B again");
        assert_eq!(form.waiting, 0);
    }

    /// R-10 r2 must 4: what the person sees is the last payment's own answer, also after an earlier
    /// one was refused: a payment that left is never hidden behind another's «not signed».
    #[test]
    fn the_last_payments_own_answer_is_what_is_shown() {
        let mut form = two_out();
        form.on_sent(1, &Err(CoreError::WrongPassword));
        form.on_sent(2, &sent());
        assert!(matches!(form.result, Some(Ok(_))));
        assert!(!form.reveal_result);
        assert!(form.recipient.is_empty(), "B went");

        // With no clear answer from the network: «Do not send it again» has to be seen.
        let mut unclear = two_out();
        unclear.on_sent(1, &Err(CoreError::WrongPassword));
        let not_relayed = Ok(SentPayment {
            txid: String::new(),
            amount: 1,
            fee: 0,
            relayed: false,
        });
        unclear.on_sent(2, &not_relayed);
        assert!(matches!(
            unclear.result,
            Some(Ok(SentPayment { relayed: false, .. }))
        ));
    }

    /// R-10 r3 should 4: once the last payment has answered, an earlier payment's answer does not
    /// touch that line: not «Do not send it again», and not an amount that came back.
    #[test]
    fn a_late_answer_of_an_earlier_payment_leaves_the_last_ones_alone() {
        let unclear = || {
            Ok(SentPayment {
                txid: String::new(),
                amount: 1,
                fee: 0,
                relayed: false,
            })
        };
        let lasts = [sent(), unclear(), Err(CoreError::NotEnough)];
        for last in &lasts {
            for late in [sent(), Err(CoreError::WrongPassword)] {
                let mut form = SendForm {
                    max: true,
                    ..two_out()
                };
                form.on_sent(2, last);
                let amount = form.amount.clone();
                form.on_sent(1, &late);
                assert_eq!(
                    format!("{:?}", form.result),
                    format!("{:?}", Some(last)),
                    "{late:?}"
                );
                assert_eq!(form.reveal_result, last.is_err(), "{late:?}");
                assert_eq!(form.amount, amount, "{late:?}");
                assert_eq!(form.waiting, 0);
            }
        }
    }

    /// R-10 r1 should 2: A's refusal after B was answered gives nothing back.
    #[test]
    fn a_late_refusal_of_an_earlier_payment_gives_nothing_back() {
        let mut form = two_out();
        form.on_sent(2, &sent());
        form.signed = "0,25".into();
        form.on_sent(1, &Err(CoreError::NotEnough));
        assert_eq!((form.waiting, form.amount.as_str()), (0, ""));
    }

    /// R-10 r1 should 1: a number this form never gave is nobody's here.
    #[test]
    fn an_answer_of_an_unknown_payment_changes_nothing() {
        let mut form = SendForm {
            signing: Some(std::time::Instant::now()),
            ..two_out()
        };
        form.on_sent(99, &Err(CoreError::WrongPassword));
        assert_eq!(form.waiting, 2);
        assert!(form.signing.is_some() && form.result.is_none());
    }

    /// R-10 r1 should 1: only the last payment's answer ends the wait of the dialog.
    #[test]
    fn an_earlier_answer_does_not_end_the_wait_of_the_last_look() {
        let mut form = SendForm {
            signing: Some(std::time::Instant::now()),
            confirming: true,
            ..two_out()
        };
        form.on_sent(1, &Err(CoreError::WrongPassword));
        assert!(form.signing.is_some() && form.confirming && !form.password_wrong);
    }

    /// R-10 r1 must 2: the number outlives a form replaced, or a new payment would take an old one's.
    #[test]
    fn a_new_form_keeps_the_payment_number() {
        let mut form = SendForm {
            last_payment: 7,
            ..SendForm::empty()
        };
        form.replace(SendForm::paying(Source::Credits, "yX".into()));
        assert_eq!(form.last_payment, 7);
    }

    #[test]
    fn a_withdrawal_opens_from_credits_to_the_address() {
        let form = SendForm::paying(Source::Credits, "yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2".into());
        assert_eq!(form.source, 1, "Credits");
        assert_eq!(form.recipient, "yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2");
        assert!(form.amount.is_empty(), "the amount is the person's to type");
    }

    #[test]
    fn warns_of_the_same_payment_within_an_hour() {
        let history = [paid("@bob", 1_000, -100_226)];
        let found = |to: &str, amount: Duffs, now: Timestamp| {
            similar_payment(&history, to, amount, 500, now).is_some()
        };
        assert!(found("@bob", 100_000, 1_100));
        assert!(!found("@dave", 100_000, 1_100), "someone else");
        assert!(!found("@bob", 90_000, 1_100), "another amount");
        assert!(!found("@bob", 100_000, 1_000 + 3_600), "an hour later");
    }

    #[test]
    fn a_lock_closes_the_last_look_but_not_a_payment_with_the_network() {
        let mut form = SendForm {
            recipient: "@alice".into(),
            amount: "0,1".into(),
            confirming: true,
            password: Zeroizing::new("password".into()),
            ..SendForm::default()
        };
        form.on_lock();
        assert!(!form.confirming && form.password.is_empty());

        let mut sent = SendForm {
            waiting: 1,
            ..SendForm::default()
        };
        sent.on_lock();
        assert_eq!(sent.waiting, 1, "the network's answer is still awaited");
    }

    /// BUG-92: the answer to the password the last look holds for: wrong keeps the dialog, the
    /// amount and the password to correct, and is no result of the form; any other closes it,
    /// wipes the password, and a refusal comes into view. Without the hold it goes as before.
    #[test]
    fn the_last_look_stays_only_for_a_wrong_password() {
        let held = || SendForm {
            amount: String::new(),
            signed: "0,1".into(),
            confirming: true,
            waiting: 1,
            password: Zeroizing::new("nope".into()),
            signing: Some(std::time::Instant::now()),
            ..SendForm::default()
        };
        let mut form = held();
        form.on_sent(0, &Err(CoreError::WrongPassword));
        assert!(form.confirming && form.signing.is_none() && form.password_wrong);
        assert!(form.refocus_password && !form.reveal_result);
        assert!(form.result.is_none(), "not said twice");
        assert_eq!(form.amount, "0,1", "the amount is back");
        assert_eq!(form.password.as_str(), "nope", "to correct, not to retype");
        assert_eq!(form.waiting, 0);

        let mut form = held();
        form.on_sent(0, &Err(CoreError::NotEnough));
        assert!(!form.confirming && form.password.is_empty());
        assert!(form.reveal_result && !form.password_wrong);
        assert_eq!(form.result, Some(Err(CoreError::NotEnough)));

        let mut form = held();
        form.on_sent(
            0,
            &Ok(SentPayment {
                txid: "ab".into(),
                amount: 1,
                fee: 1,
                relayed: true,
            }),
        );
        assert!(!form.confirming && form.password.is_empty());
        assert!(!form.reveal_result, "a success does not move the form");

        // The dialog closed meanwhile (the cross, or the wait ran out): the line is on the form.
        let mut form = SendForm {
            waiting: 1,
            ..SendForm::default()
        };
        form.on_sent(0, &Err(CoreError::WrongPassword));
        assert!(form.reveal_result && !form.password_wrong);
        assert_eq!(form.result, Some(Err(CoreError::WrongPassword)));
    }

    /// "Cancel", "Send" from a contact, "Withdraw to Core": a new form, and a payment still
    /// with the network still awaited (the form said nothing of it before).
    #[test]
    fn a_new_form_still_awaits_the_payments_with_the_network() {
        let mut form = SendForm {
            recipient: "@alice".into(),
            amount: "0,1".into(),
            waiting: 1,
            ..SendForm::default()
        };
        form.replace(SendForm::paying(Source::Credits, "yX".into()));
        assert_eq!(form.waiting, 1);
        assert_eq!((form.recipient.as_str(), form.amount.as_str()), ("yX", ""));
        form.on_sent(0, &Err(CoreError::NotEnough));
        assert_eq!(form.waiting, 0);
    }

    const ADDRESS: &str = "yNBw8AS5PhKdSm6ytP3uAVEsx5PjCiReb4";

    /// BUG-32, BUG-39 №5: the summary's "To" is "—" until the recipient is one, and short.
    #[test]
    fn the_summary_names_only_a_recipient_and_briefly() {
        assert_eq!(summary_recipient("", true), "—");
        assert_eq!(summary_recipient("q", false), "—", "not a recipient");
        assert_eq!(summary_recipient(ADDRESS, true), "yNBw…Reb4");
        assert_eq!(summary_recipient(" @alice ", true), "@alice");
        let long = format!("@{}", "a".repeat(40));
        let shown = summary_recipient(&long, true);
        assert_eq!(shown.chars().count(), 24);
        assert!(shown.ends_with('…'));

        assert!(looks_like_address(ADDRESS));
        assert!(looks_like_address(
            "tdash1kq0ufhcm3cyzpt8y2nxhsffj4sjwjq3h7ckhfmqczhlkxwj9f9v8ycpdtyl"
        ));
        for junk in [
            "q",
            "yNotAnAddress123",
            "yNBw8AS5PhKdSm6ytP3uAVEsx5PjCiRe0",
            "y N B w",
        ] {
            assert!(!looks_like_address(junk), "{junk}");
        }
    }

    /// BUG-39 №9: an address of this wallet is not offered for the address book.
    #[test]
    fn knows_the_wallets_own_addresses() {
        let mut came = paid("", 1, 100);
        came.kind = ActivityKind::Received { from: None };
        came.address = Some("yOld".into());
        let mut went = paid("yTheirs", 1, -100);
        went.address = Some("yTheirs".into());
        let history = [came, went];
        assert!(own_of(&["yNow", "tdash1me"], &history, " yNow "));
        assert!(own_of(&["yNow", "tdash1me"], &history, "tdash1me"));
        assert!(
            own_of(&["yNow"], &history, "yOld"),
            "one that received before"
        );
        assert!(!own_of(&["yNow"], &history, "yTheirs"), "a payee");
        assert!(!own_of(&["", "yNow"], &history, ""), "nothing typed");
    }

    /// BUG-68: an address this wallet paid itself to — the receive address moved on after the
    /// payment — is still its own, typed with spaces around or not.
    #[test]
    fn an_address_the_wallet_paid_itself_to_is_its_own() {
        let mut moved = paid("", 1, -226);
        moved.kind = ActivityKind::SelfTransfer { moved: 10_000_000 };
        moved.address = Some("yMine".into());
        let history = [moved];
        assert!(own_of(&["yNext"], &history, "yMine"));
        assert!(own_of(&["yNext"], &history, "  yMine  "), "with spaces");
        assert!(!own_of(&["yNext"], &history, "yOther"));
    }

    /// BUG-39 №6: "Max" follows the source chosen after it; a typed amount stays.
    #[test]
    fn max_follows_the_source_until_the_amount_is_typed() {
        let mut form = SendForm {
            max: true,
            ..SendForm::empty()
        };
        form.follow_max(100_000_000, 1_000, None);
        assert_eq!(form.amount, crate::format::input_text(99_999_000));
        // Credits now: another balance and fee.
        form.follow_max(50_000_000, 2_000, None);
        assert_eq!(form.amount, crate::format::input_text(49_998_000));
        form.max = false;
        form.amount = "0.1".into();
        form.follow_max(100_000_000, 1_000, None);
        assert_eq!(form.amount, "0.1");
    }

    fn taken() -> Result<SentPayment, CoreError> {
        Ok(SentPayment {
            txid: "aa".into(),
            amount: 1,
            fee: 1,
            relayed: true,
        })
    }

    /// BUG-39 №7: a payment taken clears "To", unless another recipient is typed by then; a
    /// refused one keeps it.
    #[test]
    fn a_payment_taken_clears_the_recipient() {
        let mut form = SendForm {
            recipient: ADDRESS.into(),
            sent_to: ADDRESS.into(),
            waiting: 1,
            max: true,
            ..SendForm::empty()
        };
        form.on_sent(0, &Err(CoreError::NotEnough));
        assert_eq!(form.recipient, ADDRESS, "refused: the person may try again");
        form.result = None; // «Check and send» again
        form.waiting = 1;
        form.on_sent(0, &taken());
        assert!(form.recipient.is_empty() && !form.max);
        assert!(
            matches!(form.result, Some(Ok(_))),
            "the answer is still shown"
        );

        let mut next = SendForm {
            recipient: "@bob".into(),
            sent_to: ADDRESS.into(),
            waiting: 1,
            ..SendForm::empty()
        };
        next.on_sent(0, &taken());
        assert_eq!(next.recipient, "@bob", "the next payment typed meanwhile");
    }

    /// BUG-39 №7: a lock clears the form — what was typed and the last answer — and keeps
    /// awaiting a payment with the network.
    #[test]
    fn a_lock_clears_the_form() {
        let mut form = SendForm {
            source: 1,
            recipient: ADDRESS.into(),
            amount: "0,1".into(),
            max: true,
            signed: "0,2".into(),
            sent_to: ADDRESS.into(),
            waiting: 1,
            result: Some(taken()),
            naming: Some("rent".into()),
            ..SendForm::default()
        };
        form.on_lock();
        assert!(form.recipient.is_empty() && form.amount.is_empty() && form.signed.is_empty());
        assert!(form.result.is_none() && form.naming.is_none() && !form.max);
        assert_eq!((form.source, form.waiting), (0, 1));
        // The answer of the payment with the network gives no amount back after the lock.
        form.on_sent(0, &Err(CoreError::NotEnough));
        assert!(form.amount.is_empty());
    }
}
