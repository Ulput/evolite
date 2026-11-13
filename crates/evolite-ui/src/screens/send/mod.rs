//! 03 · Send: the form on the left (source, recipient, amount, fee) and the summary on
//! the right. Balances and fees come from the core; a name is resolved by the core (DPNS);
//! "Check and send" shows the payment once more (the full address) before the core
//! signs it, then the network's answer.

mod coins;
mod confirm;
mod currency;
mod form;
mod mixed;
mod note;
mod recipient;
mod source;

use confirm::*;
use form::*;
use mixed::*;
pub use note::*;
use recipient::*;
use source::*;

use super::txt;
use crate::components::{
    Assets, ButtonKind, CardColumn, FieldKind, button, button_enabled, card, divider, field_label,
    from_columns, kv_row, notice, option_card_dim_in, option_card_in, option_card_width,
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
    /// The source chosen, as its place in the list on screen ([`sources`]).
    pub source: usize,
    /// A source asked for by its kind: the list depends on the wallet (FEAT-0030: «Mixed» after
    /// Core only with mixed coins), so it is found there on the next frame.
    pub want: Option<Source>,
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
    /// «Coins» (FEAT-0013): the coins chosen for a payment from Core, and the dialog.
    pub coins: coins::CoinPick,
    /// «Mixed» (FEAT-0030): the core's plan for the amount typed, and the excess agreed to.
    pub mixed: mixed::MixedPay,
    /// "Note · only for you" (FEAT-0003): kept with the payment on this computer, never sent.
    pub note: String,
    /// The last change was cut to what a note holds: N4 shows until the next change.
    pub note_cut: bool,
    /// The note of the last payment signed: cleared from the form with the amount, put back
    /// if the core refuses the payment (AC-A3).
    pub signed_note: String,
    /// «Max» was on for the last payment signed: off while its answer comes (nothing fills the
    /// amount from a balance it is leaving), back on if the core refuses (QA pass 21).
    pub signed_max: bool,
    /// The QR button's menu and its scan (FEAT-0006).
    pub scanner: crate::qr_scan::Scanner,
    /// The line under «To» after a QR or a pasted `dash:` link, and the «To» it is about:
    /// gone once «To» holds anything else.
    pub qr: Option<crate::components::QrNote>,
    pub qr_about: String,
    /// The note a `dash:` link brought (`label`, else `message`, FR-7): put in «Note», and while
    /// the note is still it, «Check the payment» says it is from the link.
    pub link_note: Option<String>,
    /// The amount a `dash:` link brought, the same way: the next code takes it away unless
    /// the person changed it.
    pub link_amount: Option<Duffs>,
    /// The amount is typed in the currency of «Fiat rate» (FEAT-0008): kept for the session,
    /// through a lock too (behaviour 2–3).
    pub in_currency: bool,
    /// What the field holds in the currency, and the amount in DASH it was last in step with.
    pub fiat: String,
    pub fiat_for: String,
    /// The amount was typed in the currency: the amount in DASH follows the rate until the
    /// last look (R-3).
    pub fiat_typed: bool,
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
            want: Some(source),
            recipient: to,
            ..Self::empty()
        }
    }

    /// Pay from `source`, wherever it stands in the list: on the next frame.
    pub fn pick(&mut self, source: Source) {
        self.want = Some(source);
        self.result = None;
    }

    /// `next` in place of this form. Payments still with the network stay counted: the form
    /// goes on saying that their answer is coming (and about which kind of payment).
    pub fn replace(&mut self, next: SendForm) {
        let waiting = self.waiting;
        let last_payment = self.last_payment;
        let last_answered = self.last_answered;
        let sent_to = std::mem::take(&mut self.sent_to);
        let in_currency = self.in_currency;
        *self = SendForm {
            waiting,
            last_payment,
            last_answered,
            sent_to,
            in_currency,
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
        if let Some(note) = &mut self.link_note {
            note.zeroize();
        }
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
                self.max = self.signed_max;
            }
            self.signed_max = false;
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
                self.link_amount = None;
                self.link_note = None;
            }
            // The chosen coins were this payment's (R-9).
            if result.is_ok() {
                self.coins.reset();
                // And the excess agreed to (FEAT-0030).
                self.mixed.reset();
            }
        } else if result.is_err() && self.max {
            // An earlier payment's refusal gives nothing back to send a second time (BUG-92 r2).
            // «Max» would fill the amount again; an amount typed since is the person's.
            self.max = false;
            self.amount.clear();
        }
        // FEAT-0030: the core built another excess than the one agreed (P2b), or one too large
        // (P4). Nothing was sent: the last look again, with the core's new plan.
        if last && let Err(CoreError::MixedExcess { excess, suggested }) = result {
            self.mixed.changed = suggested.is_none().then_some(*excess);
            self.mixed.revealed = None;
            self.mixed.agreed = None;
            self.mixed.ask_again();
            self.confirming = true;
            self.signing = None;
            self.result = None;
            return;
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

    /// [`Self::follow_max`] of `source`, if its balance is read: «Max» of a balance not read yet
    /// would fill in a guess (BUG-113).
    fn follow_source(&mut self, source: &SourceView) {
        if source.read {
            self.follow_max(source.duffs, source.max_fee, source.withdrawal);
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

    /// The line under «To» about what «To» holds now.
    fn say_qr(&mut self, note: crate::components::QrNote) {
        self.qr = Some(note);
        self.qr_about = self.recipient.clone();
    }

    /// A new recipient from a code: what the last link brought is not this one's. What the
    /// person changed since is theirs and stays.
    fn forget_what_the_link_brought(&mut self) {
        if let Some(was) = self.link_amount.take()
            && !self.max
            && crate::format::parse_amount(&self.amount) == Some(was)
        {
            self.amount.clear();
            self.suggested = None;
        }
        if let Some(mut was) = self.link_note.take() {
            if was.trim() == self.note.trim() {
                self.note.zeroize();
                self.note_cut = false;
            }
            was.zeroize();
        }
    }

    /// A `dash:` link in «To», from a QR or pasted (FR-6, FR-7): the recipient (`@name` for
    /// `user=`), the amount, the note from `label`/`message`; what was in the fields is replaced.
    /// A link refused whole fills in nothing and says so. `false`: `text` is not a link.
    pub fn take_link(&mut self, text: &str) -> bool {
        use crate::components::QrNote;
        match crate::payment_uri::parse(text) {
            None => false,
            Some(Err(_)) => {
                self.say_qr(QrNote::Error(tr!(
                    "This Dash link can't be used: nothing from it was filled in"
                )));
                true
            }
            Some(Ok(link)) => {
                self.recipient = link.recipient();
                self.forget_what_the_link_brought();
                if let Some(amount) = link.amount {
                    self.amount = crate::format::input_text(amount);
                    self.suggested = None;
                    self.max = false;
                    self.link_amount = Some(amount);
                }
                if let Some(note) = &link.note {
                    self.type_note(note);
                    self.link_note = Some(self.note.clone());
                }
                let what = match (link.user.is_some(), link.amount.is_some()) {
                    (false, false) => tr!("address"),
                    (false, true) => tr!("address and amount"),
                    (true, false) => tr!("name"),
                    (true, true) => tr!("name and amount"),
                };
                self.say_qr(QrNote::Filled(what));
                self.result = None;
                true
            }
        }
    }

    /// What a scan brought to «To»: a link as [`Self::take_link`], an address or `@name` as if
    /// typed; anything else says why and leaves what was typed. A scan ending during the last
    /// look is dropped: the payment being checked does not change under the person's eyes.
    pub fn take_scan(&mut self, scanned: &crate::qr_scan::Scanned) {
        use crate::components::QrNote;
        use crate::qr_scan::{Kind, Scanned};
        if self.confirming {
            return;
        }
        match scanned {
            Scanned::Found {
                text,
                kind: Kind::Uri,
            } => {
                self.take_link(text);
            }
            Scanned::Found {
                text,
                kind: kind @ (Kind::Address | Kind::Name),
            } => {
                self.recipient = text.clone();
                self.forget_what_the_link_brought();
                self.say_qr(QrNote::Filled(if *kind == Kind::Name {
                    tr!("name")
                } else {
                    tr!("address")
                }));
                self.result = None;
            }
            Scanned::Found {
                kind: Kind::AppLink,
                ..
            } => {
                self.say_qr(QrNote::Error(tr!(
                    "This is an app link: open it on the “Apps” screen"
                )));
            }
            other => {
                if let Some(why) = crate::components::qr_error(other) {
                    self.say_qr(QrNote::Error(why));
                }
            }
        }
    }

    /// Whether the note the last look shows is still the one the link brought.
    fn note_from_link(&self) -> bool {
        self.link_note
            .as_deref()
            .is_some_and(|n| Some(n.trim()) == self.note_shown().as_deref())
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

    /// The core's plan for a payment from «Mixed» (FEAT-0030).
    pub fn on_mixed_preview(
        &mut self,
        ask: u64,
        result: &Result<evolite_api::MixedQuote, CoreError>,
    ) {
        self.mixed.on_preview(ask, result);
    }

    /// Where a payment goes, asked for the last look or while an address is typed.
    pub fn on_preview(&mut self, to: &str, result: &Result<String, CoreError>) {
        if to == self.previewed {
            self.preview = Some(result.clone());
        }
    }
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
            want: None,
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
            coins: Default::default(),
            mixed: Default::default(),
            note: String::new(),
            note_cut: false,
            signed_note: String::new(),
            signed_max: false,
            scanner: Default::default(),
            qr: None,
            qr_about: String::new(),
            link_note: None,
            link_amount: None,
            in_currency: false,
            fiat: String::new(),
            fiat_for: String::new(),
            fiat_typed: false,
        }
    }
}

pub fn send(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut SendForm) {
    // "A password for every payment": the core keeps it for the wallet (BUG-42).
    let ask_password = crate::components::password_asked(cx.snap);
    // A refusal has just come: its line under the buttons is brought into view, once.
    let reveal = std::mem::take(&mut form.reveal_result);
    crate::format::fill_suggested(&mut form.amount, &mut form.suggested);
    form.sync_currency(currency::Rate::now(cx.now.timestamp()).as_ref());
    ui.spacing_mut().item_spacing.y = 24.0;
    page_header(ui, a, tr!("Send"), None, |_, _| {});

    // A picture dropped on the window, then what a scan brought (FEAT-0006): «To» gets it.
    if !form.confirming
        && let Some(picture) = crate::qr_scan::dropped(ui.ctx())
    {
        form.scanner.start(ui.ctx(), picture);
    }
    if let Some(scanned) = form.scanner.poll() {
        form.take_scan(&scanned);
    }
    if !form.confirming && crate::qr_scan::dragging_picture(ui.ctx()) {
        crate::components::drop_overlay(ui.ctx(), a, ui.max_rect());
    }

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
    let mut all = sources(cx.wallet(), &typed, for_fee);
    // «Mixed»: the fee of the core's plan for this very amount (FEAT-0030).
    let typed_amount = crate::format::parse_amount(&form.amount).unwrap_or(0);
    apply_quote(
        &mut all,
        form.mixed
            .quote_for(&typed, typed_amount)
            .and_then(|q| q.as_ref().ok()),
    );
    let view = FormView::of(cx, form, &all, typed, ask_password, reveal);
    let mut asked = Asked::default();

    // Side by side when the form keeps at least 420 px, otherwise the summary goes below.
    let summary_w = 320.0;
    let side = ui.available_width() - summary_w - 24.0 >= 420.0;
    let form_w = if side {
        ui.available_width() - summary_w - 24.0
    } else {
        ui.available_width()
    };
    let summary_inner = if side {
        summary_w - 50.0
    } else {
        form_w - 50.0
    };

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            if side {
                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 24.0;
                    ui.vertical(|ui| form_card(ui, a, form, &view, form_w, &mut asked));
                    ui.vertical(|ui| summary_card(ui, &a.p, &view, summary_inner));
                });
            } else {
                ui.spacing_mut().item_spacing.y = 24.0;
                form_card(ui, a, form, &view, form_w, &mut asked);
                summary_card(ui, &a.p, &view, summary_inner);
            }
        });
    for command in asked.commands {
        cx.send(command);
    }
    if asked.open_contacts {
        cx.go(super::Screen::Contacts);
    }

    // While its answer is awaited the amount has left the form for the payment (BUG-92).
    let signed = form
        .signing
        .and_then(|_| crate::format::parse_amount(&form.signed).filter(|d| *d > 0));
    if form.confirming
        && let Some(amount) = view.amount.or(signed)
    {
        confirm(
            ui,
            a,
            cx,
            form,
            (amount, view.rate.as_ref(), view.coin_plan),
            ask_password,
        );
    }
    coins::coins_dialog(ui, a, form, &view);
}

#[cfg(test)]
mod tests;
