//! 03 · Send: the form on the left (source, recipient, amount, fee) and the summary on
//! the right. Balances and fees come from the core; a name is resolved by the core (DPNS);
//! "Check and send" shows the payment once more (the full address) before the core
//! signs it, then the network's answer.

mod confirm;
mod note;
mod recipient;
mod source;

use confirm::*;
pub use note::*;
use recipient::*;
use source::*;

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

#[cfg(test)]
mod tests;
