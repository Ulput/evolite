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
    (amount, rate, coin_plan): (
        Duffs,
        Option<&super::currency::Rate>,
        Option<Result<evolite_api::coin_plan::CoinPlan, Duffs>>,
    ),
    ask_password: bool,
) {
    let p = a.p;
    // Chosen coins (FEAT-0013): what the payment takes from them; one gone since — said, and
    // nothing signed (R-6, R-10).
    let coins_gone = form.coins.gone(&form.coins.chosen.clone());
    let coin_plan = coin_plan.and_then(Result::ok);
    let all = sources(cx.wallet(), form.recipient.trim(), amount);
    let source = &all[form.source.min(all.len() - 1)];
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
        crate::text::lookalike(&to, known).map(str::to_owned)
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
            let from = match &coin_plan {
                // «Core · 2 of 3 coins»; all taken: «Core · 3 coins» (P1).
                Some(plan) if plan.taken < form.coins.chosen.len() => format!(
                    tr!("Core · {} of {} coins"),
                    plan.taken,
                    form.coins.chosen.len()
                ),
                Some(plan) => format!(tr!("Core · {} coins"), plan.taken),
                None => format!(
                    "{} · {}",
                    source.title,
                    crate::format::balance(source.duffs)
                ),
            };
            kv_row(ui, &p, tr!("From"), &from, sans(13.0, Weight::Medium));
            recipient_lines(ui, &p, form, &look);
            match &coin_plan {
                Some(plan) => coin_amount_lines(ui, a, form, (amount, plan), rate),
                None => amount_lines(
                    ui,
                    &p,
                    (amount, source.fee),
                    matches!(source.source, Source::Credits | Source::PlatformAddress),
                    rate,
                ),
            }
            if let Some(plan) = &coin_plan {
                if plan.dust_to_fee {
                    ui.label(txt(
                        &format!(
                            tr!("The change is under {} and goes to the fee."),
                            crate::format::amount(evolite_api::coin_plan::DUST)
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
                let mut taken = form.coins.chosen_coins();
                taken.sort_by_key(|c| std::cmp::Reverse(c.value));
                taken.truncate(plan.taken);
                if let Some(note) = super::coins::linked_note(&taken) {
                    crate::components::notice(ui, a, "info", p.muted, &note);
                }
            }
            if let Some(outpoint) = &coins_gone {
                super::coins::spent_banner(ui, a, &form.coins.coin_name(outpoint));
            }
            if let Some(note) = form.note_shown() {
                confirm_note(ui, a, &note, form.note_from_link());
            }
            // From the Platform addresses: public, unlike the pool (FEAT-0015 R-10, frame 7).
            if source.source == Source::PlatformAddress {
                crate::components::notice(
                    ui,
                    a,
                    "eye",
                    p.accent,
                    tr!("The amount and both addresses will be visible to everyone on Platform."),
                );
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
                    && (!from_core || address.is_some())
                    && coins_gone.is_none();
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
        sign_and_send(cx, form, (source.source, to), (amount, rate), ask_password);
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
        // A name as it is; an address solid, as Dash writes it, in Mono Medium and
        // one colour (FEAT-0035, the designer: no heavier ends).
        if to.starts_with('@') {
            let who = match resolved.as_ref().and_then(|n| n.display.as_deref()) {
                Some(display) => format!("{to} · {display}"),
                None => to.clone(),
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(crate::i18n::t(&who))
                        .font(mono(14.0, Weight::Medium))
                        .color(p.text),
                )
                .wrap(),
            );
        } else {
            let marked = lookalike
                .as_deref()
                .map(|similar| crate::text::differing(to, similar))
                .unwrap_or_default();
            ui.add(
                egui::Label::new(address_job(
                    to,
                    15.0,
                    (p.text, Some(p.danger)),
                    Weight::Medium,
                    &marked,
                ))
                .wrap(),
            );
        }
        if let Some(similar) = &lookalike {
            lookalike_warning(ui, p, to, similar);
        }
        if from_core {
            match (&address, &form.preview) {
                // The recipient's own address is above already (FEAT-0035, the designer):
                // only a contact's, from its key, is said here.
                (Some(address), _) if *address != *to => {
                    // A contact (by name or id) is paid to an address of its key.
                    let line = format!(
                        tr!("to an address from the contact's key: {address}"),
                        address = address
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(crate::i18n::t(&line))
                                .font(mono(12.0, Weight::Regular))
                                .color(p.muted),
                        )
                        .wrap(),
                    );
                }
                (Some(_), _) => {}
                (None, Some(Err(e))) => {
                    ui.add(
                        egui::Label::new(txt(&send_error(e), 12.0, Weight::Regular, p.danger))
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

/// What the last look says of the fee: its label, the fee and the total. `about`: Platform meters
/// credits with no cap and a transfer cannot set a maximum, so their fee is an estimate; a Core
/// fee is known before signing and is "at most" (BUG-108 r1).
pub(super) fn fee_lines(amount: Duffs, fee: Duffs, about: bool) -> (&'static str, String, String) {
    let dash = crate::format::amount;
    let total = amount.saturating_add(fee);
    if about {
        (
            tr!("Fee, about"),
            format!("≈ {}", dash(fee)),
            format!("≈ {}", dash(total)),
        )
    } else {
        (tr!("Fee, at most"), dash(fee), dash(total))
    }
}

/// The amount, the fee and the two together. `about`: see [`fee_lines`].
/// With «Fiat rate» on, each in the currency under it, smaller (R-2, frame 4).
fn amount_lines(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    (amount, fee_duffs): (Duffs, Duffs),
    about: bool,
    rate: Option<&super::currency::Rate>,
) {
    let value = mono(13.0, Weight::Medium);
    let (label, fee, total) = fee_lines(amount, fee_duffs, about);
    kv_row(
        ui,
        p,
        tr!("Amount"),
        &crate::format::amount(amount),
        value.clone(),
    );
    if let Some(line) = super::currency::confirm_line(rate, amount) {
        small_right(ui, p, &line);
    }
    kv_row(ui, p, label, &fee, value.clone());
    if let Some(line) = super::currency::fee_line(rate, fee_duffs) {
        small_right(ui, p, &line);
    }
    divider(ui, p);
    kv_row(ui, p, tr!("In total"), &total, value);
}

/// [`amount_lines`] of a payment from chosen coins (P1): the amount, the inputs it takes, the
/// fee at most, the change, the total.
fn coin_amount_lines(
    ui: &mut Ui,
    a: &mut Assets,
    form: &SendForm,
    (amount, plan): (Duffs, &evolite_api::coin_plan::CoinPlan),
    rate: Option<&super::currency::Rate>,
) {
    let p = a.p;
    let value = mono(13.0, Weight::Medium);
    let (label, fee, total) = fee_lines(amount, plan.fee, false);
    kv_row(
        ui,
        &p,
        tr!("Amount"),
        &crate::format::amount(amount),
        value.clone(),
    );
    if let Some(line) = super::currency::confirm_line(rate, amount) {
        small_right(ui, &p, &line);
    }
    super::coins::confirm_inputs(ui, a, form, plan);
    kv_row(ui, &p, label, &fee, value.clone());
    if plan.change > 0 {
        kv_row(
            ui,
            &p,
            tr!("Change, to your address"),
            &crate::format::amount(plan.change),
            value.clone(),
        );
    }
    divider(ui, &p);
    kv_row(ui, &p, tr!("In total"), &total, value);
}

/// An address solid (FEAT-0035), in one `weight` and one colour (the designer, 2026-11-11: no
/// heavier ends); the characters at `marked` underlined, and in `colors.1` when given. It stays
/// whole on its line and breaks by characters only when longer than the line (egui wraps a
/// word longer than the row inside it). Left to right in every language: never mirrored.
fn address_job(
    address: &str,
    size: f32,
    (color, marked_color): (egui::Color32, Option<egui::Color32>),
    weight: Weight,
    marked: &[usize],
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    append_address(
        &mut job,
        address,
        size,
        (color, marked_color),
        weight,
        marked,
    );
    job
}

/// [`address_job`] appended to `job`.
fn append_address(
    job: &mut egui::text::LayoutJob,
    address: &str,
    size: f32,
    (color, marked_color): (egui::Color32, Option<egui::Color32>),
    weight: Weight,
    marked: &[usize],
) {
    for (i, c) in address.chars().enumerate() {
        let mark = marked.contains(&i);
        let format = egui::TextFormat {
            font_id: mono(size, weight),
            color: match (mark, marked_color) {
                (true, Some(c)) => c,
                _ => color,
            },
            underline: if mark {
                egui::Stroke::new(1.0, marked_color.unwrap_or(color))
            } else {
                egui::Stroke::NONE
            },
            ..Default::default()
        };
        job.append(&c.to_string(), 0.0, format);
    }
}

/// The look-alike warning with the earlier address in it, in reading order for a screen reader.
pub(super) fn lookalike_spoken(similar: &str) -> String {
    crate::i18n::spoken(&format!(
        tr!(
            "Careful: this address only looks like {} that you paid or saved before — the middle is different. Compare it character by character."
        ),
        similar
    ))
}

/// «Careful: this address only looks like …» (FEAT-0035): the earlier address solid, the
/// characters where it differs from the recipient's underlined. A screen reader hears the
/// sentence, the recipient's address and where they differ, not the colours.
fn lookalike_warning(ui: &mut Ui, p: &crate::theme::Palette, to: &str, similar: &str) {
    const MARK: &str = "\u{E000}";
    let sentence = crate::i18n::t(&format!(
        tr!("Careful: this address only looks like {} that you paid or saved before — the middle is different. Compare it character by character."),
        MARK
    ))
    .into_owned();
    let (before, after) = sentence.split_once(MARK).unwrap_or((&sentence, ""));
    let words = || egui::TextFormat {
        font_id: sans(12.0, Weight::Medium),
        color: p.danger,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(before, 0.0, words());
    let marked = crate::text::differing(similar, to);
    append_address(
        &mut job,
        similar,
        12.0,
        (p.danger, None),
        Weight::Regular,
        &marked,
    );
    job.append(after, 0.0, words());
    let resp = ui.add(egui::Label::new(job).wrap());
    // For a screen reader the sentence in reading order (`spoken`), not as drawn: `t` has
    // Arabic in visual order, which would be read backwards (r1 Should 1).
    let said = format!(
        "{} · {to} · {}",
        lookalike_spoken(similar),
        crate::text::positions(&crate::text::differing(to, similar))
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said));
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

impl SendForm {
    /// What the form gives the payment just signed: the amount and the note leave it (not twice
    /// by a second click), kept to put back if the core refuses.
    fn let_go_of_payment(&mut self) {
        self.signed = std::mem::take(&mut self.amount);
        // «Max» goes with it: left on, it would fill the field again from the balance the
        // payment is leaving, and «more than there is» once that balance drops (QA pass 21).
        self.signed_max = std::mem::take(&mut self.max);
        // And its note, which is this payment's only.
        self.signed_note = std::mem::take(&mut self.note);
        self.note_cut = false;
    }

    /// [`Self::let_go_of_payment`] of a payment to `to`, as [`sign_and_send`] does it, for tests.
    #[cfg(test)]
    pub(super) fn take_for_signing(&mut self, to: &str) {
        self.sent_to = to.to_owned();
        self.last_payment += 1;
        self.waiting += 1;
        self.let_go_of_payment();
    }
}

/// The payment goes to the core, and the form lets go of it.
fn sign_and_send(
    cx: &mut Ctx,
    form: &mut SendForm,
    (from, to): (Source, String),
    (amount, rate): (Duffs, Option<&super::currency::Rate>),
    ask_password: bool,
) {
    form.sent_to = to.clone();
    // The password goes to the core and is wiped here at once.
    form.last_payment += 1;
    cx.send(Command::Spend {
        op: Spend::Send {
            // The coins chosen, «Max» spending them all (FEAT-0013); none: the wallet chooses.
            coins: form.coins.choice(form.max),
            id: form.last_payment,
            from,
            to,
            amount,
            note: form.note_to_send().map(evolite_api::NoteText),
            // The rate «Summary» showed (АРХ-2): the core keeps it only if still fresh.
            rate: rate.map(super::currency::Rate::for_core),
        },
        password: ask_password.then(|| Password::new(form.password.to_string())),
    });
    form.waiting += 1;
    form.let_go_of_payment();
    // A wrong password is said here, at its field (BUG-92): the dialog waits for the answer.
    form.signing = (ask_password && form.waiting == 1).then(std::time::Instant::now);
    form.password_wrong = false;
}
