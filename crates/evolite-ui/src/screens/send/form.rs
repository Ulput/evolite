//! The form of Send and its summary, drawn from what one frame knows ([`FormView`]).

use super::*;

/// What one frame of the form knows, worked out once before anything is drawn.
pub(super) struct FormView<'a> {
    pub(super) all: &'a [SourceView],
    pub(super) source: &'a SourceView,
    pub(super) core_as_of: Option<evolite_api::CoreAsOf>,
    pub(super) core_as_of_line: Option<String>,
    pub(super) ask_password: bool,
    /// A refusal has just come: its line under the buttons is brought into view, once.
    pub(super) reveal: bool,
    pub(super) address_problem: Option<String>,
    pub(super) amount: Option<Duffs>,
    pub(super) left: Option<Duffs>,
    pub(super) synced: bool,
    pub(super) problem: Option<AmountProblem>,
    pub(super) unavailable: Option<&'static str>,
    /// From the pool to an @name, which it cannot pay (BUG-112): said once, under the recipient.
    pub(super) pool_to_name: bool,
    pub(super) resolved_name: Option<ResolvedName>,
    pub(super) to_self: bool,
    pub(super) target_known: bool,
    pub(super) name_problem: Option<(String, bool)>,
    pub(super) checking: bool,
    pub(super) contact_fee: Duffs,
    /// The warning before "Add to contacts": what the network wants on the identity (BUG-107).
    pub(super) contact_reserve: Option<String>,
    pub(super) not_contact: bool,
    pub(super) recipient_label: String,
    pub(super) book: Vec<SavedAddress>,
    pub(super) address_ok: bool,
    pub(super) own: bool,
    pub(super) summary_to: String,
    pub(super) summary_note: Option<String>,
    pub(super) sent_to: String,
}

/// What the form asks of the core and the app once it is drawn.
#[derive(Default)]
pub(super) struct Asked {
    pub(super) commands: Vec<Command>,
    pub(super) open_contacts: bool,
}

impl<'a> FormView<'a> {
    pub(super) fn of(
        cx: &mut Ctx,
        form: &mut SendForm,
        all: &'a [SourceView],
        typed: String,
        ask_password: bool,
        reveal: bool,
    ) -> Self {
        // Core not checked in this sync: its last checked balance and when, or «not known yet»
        // (BUG-77); a payment waits for the sync all the same.
        let core_as_of = cx.wallet().balances.core_as_of;
        let core_as_of_line = crate::text::core_as_of(core_as_of, cx.now);
        let source = &all[form.source];
        form.follow_source(source);
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
        let left = amount
            .filter(|_| source.read)
            .map(|d| crate::format::left_after(source.duffs, d, source.fee));
        // Not before the wallet knows all its coins: the core would refuse, and a balance
        // still syncing is no basis for a payment (nor for saying it is too small).
        let synced = crate::text::coins_known(cx.snap.sync.stage);
        let problem = form_problem(&form.amount, source, synced);
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
                "You can't send from the pool to an @name. Enter an evolite private address, a Platform address or a Core address, or choose another source."
            )),
            _ => None,
        };
        let pool_to_name =
            source.source == Source::Pool && pool_route(&typed) == Some(PoolRoute::Name);
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
        let contact_reserve = super::super::contacts::reserve_warning(cx.wallet());
        let not_contact = all[form.source].source == Source::Core
            && resolved_name
                .as_ref()
                .is_some_and(|n| n.contact != Some(ContactState::Established));
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
        FormView {
            all,
            source,
            core_as_of,
            core_as_of_line,
            ask_password,
            reveal,
            address_problem,
            amount,
            left,
            synced,
            problem,
            unavailable,
            pool_to_name,
            resolved_name,
            to_self,
            target_known,
            name_problem,
            checking,
            contact_fee,
            contact_reserve,
            not_contact,
            recipient_label,
            book,
            address_ok,
            own,
            summary_to,
            summary_note,
            sent_to,
        }
    }
}

/// The form's card: where from, to whom, how much, and the answer to the last payment.
pub(super) fn form_card(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    v: &FormView,
    form_w: f32,
    asked: &mut Asked,
) {
    let p = a.p;
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
        source_cards(ui, a, form, v);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field_label(ui, &p, tr!("To"));
            let scanner = &mut form.scanner;
            if text_field(
                ui,
                &p,
                "send-recipient",
                &mut form.recipient,
                tr!("@name or Dash address"),
                FieldKind::Text,
                |ui| {
                    crate::components::qr_button(ui, a, "send-recipient", scanner);
                },
            )
            .changed()
            {
                form.result = None;
                // A `dash:` link pasted as text is taken as a QR's (FR-6).
                let typed = form.recipient.clone();
                form.take_link(&typed);
            }
            // The line is about the «To» it was said of: typing, a contact chosen or the
            // payment gone take it away.
            if form.recipient != form.qr_about {
                form.qr = None;
            }
            if let Some(note) = &form.qr {
                crate::components::qr_note(ui, a, note);
            }
        });
        recipient_state(ui, a, form, v, asked);
        if let (true, Some(name)) = (v.not_contact, &v.resolved_name) {
            contact_first(ui, a, form, v, name, asked);
            return;
        }
        amount_row(ui, form, v, &p);
        note_field(ui, a, form);
        kv_row(
            ui,
            &p,
            tr!("Network fee"),
            // Nothing to pay for a payment that cannot be made (BUG-112).
            &if v.pool_to_name {
                "—".to_owned()
            } else {
                crate::format::amount(v.source.fee)
            },
            mono(13.0, Weight::Regular),
        );
        route_notice(ui, a, v);
        send_row(ui, a, form, v, asked);
        payment_answer(ui, a, form, v);
    });
}

/// The sources to pay from, as cards in one row.
fn source_cards(ui: &mut Ui, a: &mut Assets, form: &mut SendForm, v: &FormView) {
    let p = a.p;
    let all = v.all;
    let core_as_of = v.core_as_of;
    let core_as_of_line = &v.core_as_of_line;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            // Equal columns in one row (the designer's summary №25), a long Core balance
            // too (BUG-67); every card whole inside the panel, its words with the padding
            // (BUG-81): as many columns as hold the widest words, without icons if need be.
            let details: Vec<String> = all
                .iter()
                .map(|s| source_line(s, core_as_of, core_as_of_line.as_deref()))
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
            for (i, ((s, detail), column)) in all.iter().zip(&details).zip(columns).enumerate() {
                let selected = i == form.source;
                // The last known amount, not read in this run, is dimmed (BUG-113).
                let card = if source_dim(s) {
                    option_card_dim_in
                } else {
                    option_card_in
                };
                if card(ui, a, s.icon, s.title, detail, selected, column).clicked() {
                    form.source = i;
                    form.result = None;
                }
            }
        });
        if let Some(caption) = sources_caption(core_as_of, core_as_of_line.as_deref(), all) {
            ui.add(egui::Label::new(txt(&caption, 12.0, Weight::Regular, p.muted)).wrap());
        }
    });
}

/// What the form knows about whom it pays: the name found, or why not; the address book.
fn recipient_state(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    v: &FormView,
    asked: &mut Asked,
) {
    recipient_lines(ui, a, form, v, asked);
    // Said once, right under the recipient, not under the amount (BUG-112).
    if v.pool_to_name
        && let Some(why) = v.unavailable
    {
        crate::components::attention_notice(ui, a, why);
    }
}

/// The recipient's card, or why there is none yet, and the address book.
fn recipient_lines(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    v: &FormView,
    asked: &mut Asked,
) {
    let p = a.p;
    let all = v.all;
    let (to_self, checking, own, address_ok) = (v.to_self, v.checking, v.own, v.address_ok);
    let (resolved_name, name_problem) = (&v.resolved_name, &v.name_problem);
    let (address_problem, book) = (&v.address_problem, &v.book);
    if let Some(name) = &resolved_name {
        resolved(ui, a, name, all[form.source].source, &mut asked.commands);
        if to_self {
            crate::components::attention_line(
                ui,
                a,
                tr!("This is your identity: the credits would stay yours, only the fee would go."),
            );
        }
    } else if checking {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));
        ui.add(
            egui::Label::new(txt(
                crate::text::name_waiting(form.asked_at),
                12.0,
                Weight::Regular,
                p.muted,
            ))
            .wrap(),
        );
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
        address_book(ui, a, form, book, address_ok && !own, &mut asked.commands);
    }
}

/// Core pays a name only as a contact: the explanation and the request, instead of an amount.
fn contact_first(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    v: &FormView,
    name: &ResolvedName,
    asked: &mut Asked,
) {
    let p = a.p;
    let contact_fee = v.contact_fee;
    let who = name
        .display
        .clone()
        .unwrap_or_else(|| format!("@{}", name.name));
    let (text, action) = match name.contact {
        Some(ContactState::Outgoing) => (
            format!(
                tr!(
                    "{who} already has your contact request. When {who} answers, you can pay from here. Credits can go to @{} right away: choose “Credits”."
                ),
                name.name,
                who = who
            ),
            None,
        ),
        Some(ContactState::Incoming) => (
            format!(
                tr!("{who} sent you a contact request: accept it, and you can pay from here."),
                who = who
            ),
            Some(tr!("Open contacts").to_string()),
        ),
        _ => (
            format!(
                tr!(
                    "A payment from Core to an @name goes to addresses from the key the person sends in answer to a contact request. Send {who} a request — once they answer, you can pay from here. Credits can go to @{} right away: choose “Credits”."
                ),
                name.name,
                who = who
            ),
            Some(format!(
                tr!("Add to contacts · ≈ {}"),
                crate::format::amount(contact_fee)
            )),
        ),
    };
    notice(ui, a, "contacts", p.accent, &text);
    if name.contact.is_none()
        && let Some(warning) = &v.contact_reserve
    {
        super::super::contacts::reserve_line(ui, &p, warning);
    }
    if let Some(Err(e)) = &form.contact_result
        && !form.contact_password.is_wrong()
    {
        crate::components::error_notice(
            ui,
            a,
            &super::super::contacts::contact_error(e),
            crate::text::error_details(e).as_deref(),
        );
    }
    // The request costs credits: "password for every payment" asks here too.
    let ask = action.is_some() && name.contact != Some(ContactState::Incoming) && v.ask_password;
    let entered = form
        .contact_password
        .field(ui, &p, "send-contact-password", ask);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if let Some(action) = action {
            let label = if form.contact_waiting {
                tr!("Sending…").to_string()
            } else {
                action
            };
            let ready = !form.contact_waiting && form.contact_password.ready(ask);
            if button_enabled(ui, a, &label, ButtonKind::Primary, None, ready).clicked()
                || (entered && ready)
            {
                if name.contact == Some(ContactState::Incoming) {
                    asked.open_contacts = true;
                } else {
                    asked.commands.push(form.contact_password.command(
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
}

/// The amount, and what is wrong with it.
fn amount_row(ui: &mut Ui, form: &mut SendForm, v: &FormView, p: &crate::theme::Palette) {
    let p = *p;
    let source = v.source;
    let (unavailable, problem) = (v.unavailable, v.problem);
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
            form.follow_source(source);
        }
        if resp.changed() || all {
            form.result = None;
        }
        if let Some(why) = unavailable.filter(|_| !v.pool_to_name) {
            ui.add(egui::Label::new(txt(why, 12.0, Weight::Regular, p.muted)).wrap());
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
}

/// How the money travels: a withdrawal arrives later than a payment.
fn route_notice(ui: &mut Ui, a: &mut Assets, v: &FormView) {
    let p = a.p;
    // Credits or the pool to a Core address are a withdrawal: they arrive later, not in seconds;
    // from the pool, the line says what the network sees, which depends on the recipient (BUG-112).
    if let Some((icon, text)) = route_line(v.source, &v.recipient_label) {
        notice(ui, a, icon, p.accent, text);
    }
}

/// "Check and send", and what is still missing for it.
fn send_row(ui: &mut Ui, a: &mut Assets, form: &mut SendForm, v: &FormView, asked: &mut Asked) {
    let p = a.p;
    let source = v.source;
    let (amount, problem, unavailable) = (v.amount, v.problem, v.unavailable);
    let (target_known, synced, checking) = (v.target_known, v.synced, v.checking);
    // A balance not read yet waits like the sync (BUG-113): the pool is read after it.
    let synced = synced && source.read;
    let ready =
        amount.is_some() && problem.is_none() && unavailable.is_none() && target_known && synced;
    // From the pool to a name the button is outlined and says what to change (BUG-112).
    let kind = if v.pool_to_name {
        ButtonKind::Secondary
    } else {
        ButtonKind::Primary
    };
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
    } else if v.pool_to_name {
        Some(tr!("Change the source or the recipient"))
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
        if button_enabled(ui, a, label, kind, None, ready).clicked() {
            form.confirming = true;
            form.result = None;
            // From Core the last look shows where the money goes: the address, or the
            // contact's next address from its key, as the core will pay it.
            form.preview = None;
            form.previewed = form.recipient.trim().to_string();
            if source.source == Source::Core {
                asked.commands.push(Command::PreviewPayment {
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
}

/// The network's answer to the last payment.
fn payment_answer(ui: &mut Ui, a: &mut Assets, form: &SendForm, v: &FormView) {
    let p = a.p;
    let source = v.source;
    let sent_to = &v.sent_to;
    let reveal = v.reveal;
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
}

/// The summary beside or under the form.
pub(super) fn summary_card(ui: &mut Ui, p: &crate::theme::Palette, v: &FormView, width: f32) {
    let source = v.source;
    let (amount, left, not_contact) = (v.amount, v.left, v.not_contact);
    let (summary_to, summary_note) = (&v.summary_to, &v.summary_note);
    card(p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.label(txt(tr!("Summary"), 16.0, Weight::SemiBold, p.text));
            let value = mono(13.0, Weight::Medium);
            let dash = |d: Option<i64>| d.map_or("—".into(), crate::format::amount);
            kv_row(ui, p, tr!("From"), source.title, sans(13.0, Weight::Medium));
            kv_row(
                ui,
                p,
                tr!("To"),
                summary_to,
                sans(13.0, Weight::Medium),
            );
            // Nothing is paid until the name is a contact, nor from the pool to a name (BUG-112).
            let (amount, fee, left) = if v.pool_to_name {
                (amount, None, None)
            } else if not_contact {
                (None, None, source.read.then_some(source.duffs))
            } else {
                (amount, Some(source.fee), left.filter(|l| *l >= 0))
            };
            kv_row(ui, p, tr!("Amount"), &dash(amount), value.clone());
            kv_row(ui, p, tr!("Fee"), &dash(fee), value.clone());
            kv_row(
                ui,
                p,
                tr!("Left"),
                &left.map_or("—".into(), crate::format::balance),
                value,
            );
            if let Some(note) = &summary_note {
                summary_note_row(ui, p, note);
            }
            divider(ui, p);
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
}
