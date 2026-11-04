//! 03 · Send: the form on the left (source, recipient, amount, fee) and the summary on
//! the right. Balances and fees come from the core; a name is resolved by the core (DPNS);
//! "Check and send" shows the payment once more (the full address) before the core
//! signs it, then the network's answer.

use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, amount_field, button, button_enabled, card, dialog, divider,
    field_label, kv_row, notice, option_card, page_header, pill, text_field,
};
use crate::context::Ctx;
use crate::text::{name_problem, short_address};
use crate::theme::{Weight, mono, sans};
use egui::{Align, Layout, Margin, Stroke, Ui, vec2};
use evolite_api::{
    Activity, ActivityKind, Command, ContactState, CoreError, Duffs, Password, ResolvedName,
    SavedAddress, SentPayment, Source, Timestamp, Wallet,
};
use zeroize::{Zeroize, Zeroizing};

pub struct SendForm {
    pub source: usize,
    pub recipient: String,
    pub amount: String,
    /// The example amount in `amount` until the person changes it (`format::fill_suggested`).
    pub suggested: Option<i64>,
    /// The name last sent to the core for resolving, and its answer.
    pub asked: String,
    pub resolution: Option<Result<ResolvedName, CoreError>>,
    /// The payment shown for a last look before signing.
    pub confirming: bool,
    /// Payments signed and with the network; each answer comes as `Event::Sent`. The form is free
    /// meanwhile: a payment on its way is in the history as "Being sent".
    pub waiting: u32,
    /// The amount as typed for the last payment signed: cleared from the form at once, put back
    /// if the core refuses it.
    pub signed: String,
    /// The name being typed for the recipient's address, to keep it in the address book.
    pub naming: Option<String>,
    /// The address book's answer to the last save or removal, when it refused.
    pub book_error: Option<CoreError>,
    pub result: Option<Result<SentPayment, CoreError>>,
    /// "Password for every payment": typed in the confirmation, wiped once sent or closed.
    pub password: Zeroizing<String>,
    /// "Add to contacts" from here went to the core, no answer yet; then its answer.
    pub contact_waiting: bool,
    pub contact_result: Option<Result<(), CoreError>>,
    /// The recipient whose Core address the core was asked for (the last look before
    /// signing), and its answer: the address the payment will go to.
    pub previewed: String,
    pub preview: Option<Result<String, CoreError>>,
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
    /// goes on saying that their answer is coming.
    pub fn replace(&mut self, next: SendForm) {
        let waiting = self.waiting;
        *self = SendForm { waiting, ..next };
    }

    /// The wallet locked: a payment waiting for its last look is not signed after the next
    /// unlock with what was on screen before — the confirmation closes and its password goes.
    /// One already with the network keeps waiting for its answer.
    pub fn on_lock(&mut self) {
        self.confirming = false;
        self.password.zeroize();
    }

    pub fn on_contact_done(&mut self, result: &Result<(), CoreError>) {
        if !self.contact_waiting {
            return;
        }
        self.contact_waiting = false;
        self.contact_result = Some(result.clone());
        if result.is_ok() {
            // Asked again: the answer now says where the name stands among the contacts.
            self.asked.clear();
        }
    }

    pub fn on_sent(&mut self, result: &Result<SentPayment, CoreError>) {
        self.waiting = self.waiting.saturating_sub(1);
        if result.is_err() && self.amount.is_empty() {
            self.amount = std::mem::take(&mut self.signed);
        }
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
            asked: String::new(),
            resolution: None,
            confirming: false,
            waiting: 0,
            signed: String::new(),
            naming: None,
            book_error: None,
            result: None,
            password: Zeroizing::default(),
            contact_waiting: false,
            contact_result: None,
            previewed: String::new(),
            preview: None,
        }
    }
}

/// A balance to pay from, as the Send screen shows it.
struct SourceView {
    source: Source,
    icon: &'static str,
    title: &'static str,
    duffs: Duffs,
    fee: Duffs,
    note_icon: &'static str,
    note: &'static str,
}

/// What credits to `to` cost: to a Core address a withdrawal (Platform pays it out later),
/// to a name or a Platform address a transfer.
fn credits_fee(fees: &evolite_api::Fees, to: &str) -> Duffs {
    let to = to.trim();
    if to.is_empty() || to.starts_with('@') || to.to_ascii_lowercase().starts_with("tdash1") {
        fees.credit_transfer
    } else {
        fees.credit_withdrawal
    }
}

/// `to`: the recipient typed, which decides what a payment from credits or the pool costs.
fn sources(w: &Wallet, to: &str) -> [SourceView; 3] {
    [
        SourceView {
            source: Source::Core,
            icon: "core",
            title: "Core",
            duffs: w.balances.core,
            fee: w.fees.core_payment,
            note_icon: "instantsend",
            note: tr!("InstantSend: the recipient sees a confirmed payment in a few seconds."),
        },
        SourceView {
            source: Source::Credits,
            icon: "credits",
            title: tr!("Credits"),
            duffs: w.balances.credits,
            fee: credits_fee(&w.fees, to),
            note_icon: "confirmed",
            note: tr!("A credit transfer is confirmed by a Platform proof in a couple of seconds."),
        },
        SourceView {
            source: Source::Pool,
            icon: "shielded",
            title: tr!("Private pool"),
            duffs: w.balances.shielded,
            fee: super::shielded::pool_fee(&w.fees, to),
            note_icon: "shield",
            note: tr!(
                "From the pool: the amount and the recipient are not visible on the network."
            ),
        },
    ]
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

/// `ask_password`: "a password for every payment" is on in the settings.
pub fn send(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut SendForm, ask_password: bool) {
    let p = a.p;
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
    }

    let all = sources(cx.wallet(), &typed);
    let source = &all[form.source];
    // An address for Core is checked as it is typed, so a typo shows before the password.
    let typed_address =
        source.source == Source::Core && !typed.is_empty() && !typed.starts_with('@');
    if typed_address && typed != form.previewed {
        cx.send(Command::PreviewPayment { to: typed.clone() });
        form.previewed = typed.clone();
        form.preview = None;
    }
    let address_problem = match &form.preview {
        // Nothing was being sent yet: the reason alone.
        Some(Err(CoreError::NotAvailable(why))) if typed_address && form.previewed == typed => {
            Some(format!("{why}."))
        }
        Some(Err(e)) if typed_address && form.previewed == typed => Some(send_error(e)),
        _ => None,
    };
    let amount = crate::format::parse_amount(&form.amount).filter(|d| *d > 0);
    let left = amount.map(|d| crate::format::left_after(source.duffs, d, source.fee));
    let too_much = left.is_some_and(|l| l < 0);
    // A source that cannot pay at all says why, not "more than there is".
    let unavailable = match source.source {
        Source::Credits if cx.wallet().identity.is_none() => Some(tr!(
            "Only an identity has credits, and the wallet has none yet: make one on the «Identity» screen."
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
    let mut draw_form = |ui: &mut Ui, a: &mut Assets| {
        card(&p, Margin::same(24)).show(ui, |ui| {
            ui.set_width(form_w - 50.0);
            ui.spacing_mut().item_spacing.y = 18.0;
            field_label(ui, &p, tr!("From"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                for (i, s) in all.iter().enumerate() {
                    let detail = crate::format::amount(s.duffs);
                    if option_card(ui, a, s.icon, s.title, &detail, i == form.source).clicked() {
                        form.source = i;
                        form.result = None;
                    }
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
                    ui.label(txt(
                        tr!("This is your identity: the credits would stay yours, only the fee would go."),
                        12.0,
                        Weight::Regular,
                        p.danger,
                    ));
                }
            } else if checking {
                ui.label(txt(tr!("Checking the name in DPNS…"), 12.0, Weight::Regular, p.muted));
            } else if let Some((why, typo)) = &name_problem {
                let color = if *typo { p.danger } else { p.muted };
                ui.label(txt(why, 12.0, Weight::Regular, color));
            } else if let Some(why) = &address_problem {
                ui.add(egui::Label::new(txt(why, 12.0, Weight::Regular, p.danger)).wrap());
            }
            if all[form.source].source == Source::Core {
                address_book(ui, a, form, &book, address_ok, &mut asked);
            }
            if let (true, Some(name)) = (not_contact, &resolved_name) {
                let who = name.display.clone().unwrap_or_else(|| format!("@{}", name.name));
                let (text, action) = match name.contact {
                    Some(ContactState::Outgoing) => (
                        format!(tr!("{who} already has your contact request. When {who} answers, you can pay from here. Credits can go to @{} right away: choose «Credits»."), name.name, who = who),
                        None,
                    ),
                    Some(ContactState::Incoming) => (
                        format!(tr!("{who} sent you a contact request: accept it, and you can pay from here."), who = who),
                        Some(tr!("Open contacts").to_string()),
                    ),
                    _ => (
                        format!(tr!("A payment from Core to an @name goes to addresses from the key the person sends in answer to a contact request. Send {who} a request — once they answer, you can pay from here. Credits can go to @{} right away: choose «Credits»."), name.name, who = who),
                        Some(format!(tr!("Add to contacts · ≈ {}"), crate::format::amount(contact_fee))),
                    ),
                };
                notice(ui, a, "contacts", p.accent, &text);
                if let Some(Err(e)) = &form.contact_result {
                    crate::components::error_notice(ui, a, &super::contacts::contact_error(e), crate::text::error_details(e).as_deref());
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if let Some(action) = action {
                        let label = if form.contact_waiting { tr!("Sending…").to_string() } else { action };
                        if button_enabled(ui, a, &label, ButtonKind::Primary, None, !form.contact_waiting)
                            .clicked()
                        {
                            if name.contact == Some(ContactState::Incoming) {
                                open_contacts = true;
                            } else {
                                asked.push(Command::AddContact {
                                    name: format!("@{}", name.name),
                                });
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
                if all {
                    form.amount = crate::format::input_text((source.duffs - source.fee).max(0));
                }
                if resp.changed() || all {
                    form.result = None;
                }
                if let Some(why) = unavailable {
                    ui.add(
                        egui::Label::new(txt(why, 12.0, Weight::Regular, p.danger)).wrap(),
                    );
                } else if too_much {
                    ui.label(txt(
                        tr!("More than there is in this balance"),
                        12.0,
                        Weight::Regular,
                        p.danger,
                    ));
                } else if !form.amount.trim().is_empty() && amount.is_none() {
                    ui.label(txt(
                        tr!("Enter an amount, e.g. 0.25"),
                        12.0,
                        Weight::Regular,
                        p.danger,
                    ));
                }
            });
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
            // Not before the wallet knows all its coins: the core would refuse, and a balance
            // still syncing is no basis for a payment.
            let synced = crate::text::coins_known(cx.snap.sync.stage);
            let ready =
                amount.is_some() && !too_much && unavailable.is_none() && target_known && synced;
            if form.waiting > 0 {
                ui.add(
                    egui::Label::new(txt(
                        tr!("The payment awaits the network's answer: it is in the history as «Sending», and the answer will come. The form is free for the next one."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ))
                    .wrap(),
                );
            }
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
                // A grey button says nothing by itself: what is still missing.
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
                if let Some(missing) = missing {
                    ui.label(txt(missing, 12.0, Weight::Regular, p.muted));
                }
            });
            match &form.result {
                // No clear answer from the network: the payment may have gone.
                Some(Ok(sent)) if !sent.relayed => notice(
                    ui,
                    a,
                    "pending",
                    p.muted,
                    &format!(
                        tr!("The payment was handed to the nodes, but the network has not confirmed it yet. Do not send it again: it may go through. In the history it is «Sending».{}"),
                        if sent.txid.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", short_address(&sent.txid))
                        }
                    ),
                ),
                // Credits: Platform proves the transition, there is no Core transaction.
                Some(Ok(sent)) if sent.txid.is_empty() && source.source == Source::Pool => {
                    let withdrawal = !recipient_label.starts_with("tdash1");
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
                    let withdrawal = !recipient_label.starts_with('@')
                        && !recipient_label.starts_with("tdash1");
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
                if recipient_label.is_empty() {
                    "—"
                } else {
                    &recipient_label
                },
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
            kv_row(ui, &p, tr!("Left"), &dash(left), value);
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

    if form.confirming
        && let Some(amount) = amount
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
                        tr!("«{}» · from the address book"),
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
    let all = sources(cx.wallet(), form.recipient.trim());
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
                &format!("{} · {}", source.title, dash(source.duffs)),
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
            ui.label(txt(
                tr!("A payment cannot be undone."),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            if let Some(when) = &repeat {
                notice(
                    ui,
                    a,
                    "warning",
                    p.danger,
                    &format!(
                        tr!(
                            "Such a payment to {to} is already in the history ({when}). Check that this is not a repeat: the last one may not have arrived yet."
                        ),
                        to = to,
                        when = when
                    ),
                );
            }
            // Enter in the password field signs, as the button does (only from the field: a
            // stray Enter elsewhere sends nothing).
            let mut entered = false;
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
                    entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                });
            }
            let mut close = false;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let ready = (!ask_password || !form.password.is_empty())
                    && (!from_core || address.is_some());
                let clicked = button_enabled(
                    ui,
                    a,
                    tr!("Sign and send"),
                    ButtonKind::Primary,
                    None,
                    ready,
                )
                .clicked();
                if ready && (clicked || entered) {
                    send_now = true;
                    close = true;
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close
        },
    );
    if send_now {
        // The password goes to the core and is wiped here at once.
        cx.send(Command::Send {
            from: source.source,
            to,
            amount,
            password: ask_password.then(|| Password::new(form.password.to_string())),
        });
        form.waiting += 1;
        // Not twice by a second click: the amount leaves the form with the payment.
        form.signed = std::mem::take(&mut form.amount);
    }
    if closed {
        form.confirming = false;
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
    }

    fn zero_fees() -> evolite_api::Fees {
        evolite_api::Fees {
            core_payment: 0,
            credit_transfer: 0,
            pool_transfer: 0,
            nft_transfer: 0,
            document_write: 0,
            shield: 0,
            identity_create: 0,
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
            signed: "0,25".into(),
            ..SendForm::empty()
        };
        form.on_sent(&Err(CoreError::NotEnough));
        assert_eq!((form.waiting, form.amount.as_str()), (1, "0,25"));
        // Taken: the form stays as the person left it.
        form.amount = "0,1".into();
        form.on_sent(&Ok(SentPayment {
            txid: "aa".into(),
            amount: 1,
            fee: 1,
            relayed: true,
        }));
        assert_eq!((form.waiting, form.amount.as_str()), (0, "0,1"));
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
        form.on_sent(&Err(CoreError::NotEnough));
        assert_eq!(form.waiting, 0);
    }
}
