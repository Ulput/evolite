//! The last look before signing.

use super::*;

/// How long the last look waits for the core's answer to its password before closing all the same
/// (BUG-92). The check is quick; a right password is answered only once the network has the
/// payment, and the form must be free meanwhile. A later refusal comes into view on the form.
pub(super) const SIGNING_HOLD: std::time::Duration = std::time::Duration::from_secs(5);

/// A payment of this amount (with at most `fee` on top) to `to` within the last hour: signing
/// it again may be a repeat of one that already went.
pub(super) fn similar_payment<'a>(
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
pub(super) fn confirm(
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
    let look = LastLook {
        to: &to,
        resolved: &resolved,
        lookalike: &lookalike,
        from_core,
        address: &address,
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
            recipient_lines(ui, &p, form, &look);
            amount_lines(ui, &p, amount, source.fee);
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
                password_field(ui, &p, form);
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
        sign_and_send(cx, form, source.source, to, amount, ask_password);
    }
    if closed {
        form.confirming = false;
        form.signing = None;
        form.password_wrong = false;
        form.password.zeroize();
    }
}

/// What the last look shows about whom the payment goes to.
#[derive(Clone, Copy)]
struct LastLook<'a> {
    to: &'a String,
    resolved: &'a Option<ResolvedName>,
    lookalike: &'a Option<String>,
    from_core: bool,
    address: &'a Option<String>,
}

/// The recipient in full, and where exactly the money goes.
fn recipient_lines(ui: &mut Ui, p: &crate::theme::Palette, form: &SendForm, look: &LastLook) {
    let LastLook {
        to,
        resolved,
        lookalike,
        from_core,
        address,
    } = *look;
    ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, p, tr!("Recipient"));
                // An address in groups of four: compared with the one meant, group by group.
                let shown_to = if to.starts_with('@') {
                    to.clone()
                } else {
                    crate::text::grouped(to)
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
                            let line = if *address != *to {
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
}

/// The amount, the most the fee can be, and the two together.
fn amount_lines(ui: &mut Ui, p: &crate::theme::Palette, amount: Duffs, fee: Duffs) {
    let value = mono(13.0, Weight::Medium);
    let dash = |d: Duffs| crate::format::amount(d);
    kv_row(ui, p, tr!("Amount"), &dash(amount), value.clone());
    kv_row(ui, p, tr!("Fee, at most"), &dash(fee), value.clone());
    divider(ui, p);
    kv_row(
        ui,
        p,
        tr!("In total"),
        &dash(amount.saturating_add(fee)),
        value,
    );
}

/// The wallet's password, asked in the last look.
fn password_field(ui: &mut Ui, p: &crate::theme::Palette, form: &mut SendForm) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, p, tr!("Wallet password"));
        let field = text_field(
            ui,
            p,
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

/// The payment goes to the core, and the form lets go of it.
fn sign_and_send(
    cx: &mut Ctx,
    form: &mut SendForm,
    from: Source,
    to: String,
    amount: Duffs,
    ask_password: bool,
) {
    form.sent_to = to.clone();
    // The password goes to the core and is wiped here at once.
    form.last_payment += 1;
    cx.send(Command::Spend {
        op: Spend::Send {
            id: form.last_payment,
            from,
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
