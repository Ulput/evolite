//! Registering another name: the dialog and its errors.

use super::*;
use crate::components::Part;

/// "Add a name": one more DPNS name for this identity, checked live like on "Create an
/// identity" and paid from its credits. A regular name is registered at once; a contested one
/// (FEAT-0021) goes through «Check the name request» (N2) — the write and the fund on lines of
/// their own, the vote's times, and the fund «not returned» before the password — and is then
/// in the masternode vote (N3).
pub(super) fn add_name_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut IdentityState,
    wallet: &Wallet,
    id: &Identity,
    asked: &mut Vec<Command>,
) {
    let Some(form) = &mut state.adding_name else {
        return;
    };
    // «@name»; empty for an identity without a name, whose sentences are whole of their own
    // (BUG-270: a Latin «identity» put into «for {}» read «для identity» in ru).
    let me = id.name().map_or_else(String::new, |n| format!("@{n}"));
    let fee = wallet.fees.name_price(id.names.is_empty());
    // The name goes to the core once per change; the answer comes back as an event.
    let label = form.name.trim().trim_start_matches('@').to_lowercase();
    if form.review.is_none() && label.len() >= 3 && label != form.asked {
        asked.push(Command::CheckName {
            label: label.clone(),
        });
        form.asked = label.clone();
        form.check = None;
        form.result = None;
        form.asked_at = Some(std::time::Instant::now());
    }
    let check = if label.is_empty() || label != form.asked {
        None
    } else {
        form.check.clone()
    };
    let contested = matches!(&check, Some(Ok(c)) if c.contested());
    // BUG-97: what the name wants on the identity up front, not its price; a contested one wants
    // the fund too (R-3).
    let fund = form
        .review
        .map_or(wallet.fees.name_contest_fund, |r| r.fund);
    let needed = wallet.fees.name_credits_needed() + if contested { fund } else { 0 };
    let credits = wallet.balances.credits;
    let read = wallet.balances.credits_read;
    // Kept credits not checked in this run claim no lack (BUG-116).
    let short = read && wallet.balances.credits_checked && credits < needed;
    let sent = form.review.is_some() && matches!(form.result, Some(Ok(())));
    // BUG-274: «Register» went through — the dialog says so and closes with «Done»; the form
    // (the same name «available», its new price, «Register») is not shown again for a name just
    // bought.
    let written = form.review.is_none() && matches!(form.result, Some(Ok(())));
    let title = if sent {
        format!(
            tr!("The request for @{label} is in the vote"),
            label = form.asked
        )
    } else if form.review.is_some() {
        tr!("Check the name request").to_string()
    } else {
        tr!("Add a name").to_string()
    };
    let mut register = false;
    let mut review = false;
    let mut back = false;
    let mut open = false;
    let mut top_up = false;
    // BUG-225: a contested name's writes are its claim's, the figure N2t shows.
    let shown = if contested {
        wallet.fees.name_claim_writes()
    } else {
        fee
    };
    // BUG-249: what the name costs and who pays: the summary when «Register» pays at once, the
    // form's middle when the review (N2) still comes.
    let costs = |ui: &mut Ui, p: &crate::theme::Palette| {
        let value = style(UI.weight(Weight::Medium).mono());
        kv_row(
            ui,
            p,
            tr!("Name registration"),
            &format!("≈ {}", dash(shown)),
            value.clone(),
        );
        if contested {
            kv_row(ui, p, tr!("Fund for the vote"), &dash(fund), value.clone());
        }
        crate::components::kv_row_fit(
            ui,
            p,
            tr!("Paid by"),
            &if me.is_empty() {
                tr!("Identity · credits").to_string()
            } else {
                format!(tr!("{me} · credits"), me = me)
            },
            value,
        );
    };
    let close = crate::components::dialog_with_summary(
        ui,
        a,
        "add-name",
        if form.review.is_some() {
            "vote"
        } else {
            "username"
        },
        &title,
        464.0,
        |ui, a, part| {
            let p = a.p;
            // BUG-249: what is agreed to, pinned over the password: N2's costs, the total and
            // the fund «not returned»; the form's costs when «Register» pays at once.
            if part == Part::Summary {
                match form.review {
                    _ if sent || written => {}
                    Some(r) => {
                        review_summary(ui, a, wallet, form, r);
                        // FEAT-0004 FR-7: the large-spend tick with what it agrees to.
                        form.flow.password.summary_tick(ui, &p);
                    }
                    None if !contested => {
                        costs(ui, &p);
                        form.flow.password.summary_tick(ui, &p);
                    }
                    None => {}
                }
                return false;
            }
            if part == Part::Footer {
                // The footer (BUG-122): always in the window.
                if sent {
                    let mut close = false;
                    button_row(ui, |ui| {
                        if button(ui, a, tr!("Open the request"), ButtonKind::Ghost, None).clicked()
                        {
                            open = true;
                        }
                        if button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked() {
                            close = true;
                        }
                    });
                    return close;
                }
                if written {
                    let mut close = false;
                    button_row(ui, |ui| {
                        close = button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked();
                    });
                    return close;
                }
                if form.review.is_some() {
                    return review_footer(ui, a, form, (&mut register, &mut back));
                }
                let ask = wallet.password_each_send && form.result.is_none() && !contested;
                let entered = form.flow.password.field(ui, &p, "add-name-password", ask);
                let mut close = false;
                button_row(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let claimable = matches!(&check, Some(Ok(c)) if c.claimable());
                    let ready = claimable
                        && read
                        && !short
                        && !form.flow.waiting()
                        && form.result.is_none()
                        && (contested || form.flow.password.ready(ask));
                    let label = if form.flow.waiting() {
                        tr!("Writing…")
                    } else if contested {
                        tr!("Check the request")
                    } else {
                        tr!("Register")
                    };
                    if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked()
                        || (entered && ready && !contested)
                    {
                        if contested {
                            review = true;
                        } else {
                            register = true;
                        }
                    }
                });
                return close;
            }
            if sent {
                sent_body(ui, a, wallet, &me, form);
                return false;
            }
            if written {
                name_answer(ui, a, form);
                return false;
            }
            if let Some(r) = form.review {
                review_body(ui, a, wallet, &me, form, r);
                return false;
            }
            ui.add(
                crate::components::para(
                        &if me.is_empty() {
                            tr!("One more DPNS name for your identity: people can find you and pay you by it too.").to_string()
                        } else {
                            format!(
                                tr!("One more DPNS name for {me}: people can find you and pay you by it too."),
                                me = me
                            )
                        }, UI.size, UI.weight, p.muted).line_height(UI.line)
                    .line_height(20.0),
            );
            if name_field(ui, &p, form, &label, &check, (wallet, id)) {
                open = true;
            }
            if contested {
                costs(ui, &p);
            }
            // Not an error before anything was tried (calm states CS-1): what is missing, muted,
            // and the way to it.
            credits_state(ui, &p, read, short, needed, credits, &mut top_up);
            name_answer(ui, a, form);
            false
        },
    );
    // UC-5: opened from «Claim @x ›», the review comes as soon as the name can be claimed.
    let review = review
        || (form.review_asap
            && form.review.is_none()
            && matches!(&check, Some(Ok(c)) if c.claimable() && c.contested())
            && read
            && !short);
    if review {
        form.review_asap = false;
        let now = chrono::Local::now().timestamp();
        let fees = &wallet.fees;
        let join_until = match check {
            Some(Ok(NameCheck::Joinable { join_until, .. })) => join_until,
            _ => now + fees.name_join_secs as i64,
        };
        form.review = Some(Review {
            fund: fees.name_contest_fund,
            was: None,
            join_until,
            ends_about: join_until - fees.name_join_secs as i64 + fees.name_vote_secs as i64,
        });
        form.result = None;
    } else if back && !form.flow.waiting() {
        form.review = None;
        form.result = None;
        form.flow.password.forget();
    } else if register {
        let op = Spend::AddName {
            name: form.asked.clone(),
            contest_fund: form.review.map(|r| r.fund),
            link: form
                .review
                .and_then(|_| link_of(form).ok().flatten())
                .filter(|_| wallet.identity_links),
        };
        // A claim pays a fund that is not returned: the password every time (R-8).
        let ask = form.review.is_some() || wallet.password_each_send;
        if let Some(command) = form.flow.command(op, ask, ()) {
            asked.push(command);
            form.result = None;
        }
    } else if open {
        let label = form.asked.clone();
        state.adding_name = None;
        state.open_contest(&id.id, &label);
    } else if top_up && !form.flow.waiting() {
        // «Top up ›»: the name waits, the top-up opens (or says why it cannot now).
        state.adding_name = None;
        state.top_up_clicked(wallet);
    } else if close && !form.flow.waiting() {
        state.adding_name = None;
    }
}

/// N2's body: the name and identity, the optional link (R-5, АРХ-6), and the vote's times; its
/// costs are in [`review_summary`] (BUG-249).
fn review_body(
    ui: &mut Ui,
    a: &mut Assets,
    wallet: &Wallet,
    me: &str,
    form: &mut NameForm,
    r: Review,
) {
    let p = a.p;
    let now = chrono::Local::now();
    kv_row(
        ui,
        &p,
        tr!("Name"),
        &format!(tr!("@{label} · contested"), label = form.asked),
        style(UI.weight(Weight::SemiBold)),
    );
    kv_row(
        ui,
        &p,
        tr!("Identity"),
        me,
        style(UI.weight(Weight::Medium)),
    );
    // FEAT-0021 АРХ-6 (AC-T11, AC-T14): the link «where my identity is confirmed», optional, a
    // document of its own; its cost on a line of its own in the summary.
    if wallet.identity_links {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field_label(
                ui,
                &p,
                tr!("A site or social page where masternodes can see who you are · optional"),
            );
            text_field(
                ui,
                &p,
                "add-name-link",
                &mut form.link,
                "https://",
                FieldKind::Text,
                |_| {},
            );
            let (line, color) = match link_of(form) {
                Err(_) => (
                    tr!("A link https:// of at most 128 characters is needed"),
                    p.danger,
                ),
                Ok(_) => (
                    tr!("Everyone sees the link; voters in DashPay see it on the request's card."),
                    p.muted,
                ),
            };
            ui.add(
                crate::components::para(line, SMALL.size, SMALL.weight, color)
                    .line_height(SMALL.line),
            );
        });
    }
    notice(
        ui,
        a,
        "info",
        p.muted,
        &format!(
            tr!(
                "The masternode vote lasts {span}, result about {end}. Others may join the contest until {until}. The name becomes yours only if the vote is won."
            ),
            span = super::contest::span(wallet.fees.name_vote_secs),
            end = crate::text::date_time_in_line(r.ends_about, now),
            until = crate::text::date_day_in_line(r.join_until, now)
        ),
    );
    name_answer(ui, a, form);
}

/// BUG-249: N2's summary, pinned over the password: the write and the fund on lines of their
/// own (the fund that rose, was and now, R-2), the link's write, the total and the credits
/// before and after, and the fund «not returned» above the password in any window (AC-T6).
fn review_summary(ui: &mut Ui, a: &mut Assets, wallet: &Wallet, form: &NameForm, r: Review) {
    let p = a.p;
    let value = style(UI.weight(Weight::Medium).mono());
    // BUG-190: a contested domain costs more than a regular name: the bound the credits are
    // checked by (preorder + domain at its peak), never below what is charged. BUG-225: and
    // above the claims' writes as metered, the same figure as N2.
    let write = wallet.fees.name_claim_writes();
    kv_row(
        ui,
        &p,
        tr!("Name write"),
        &format!("≈ {}", dash(write)),
        value.clone(),
    );
    kv_row(
        ui,
        &p,
        tr!("Fund for the vote"),
        &dash(r.fund),
        value.clone(),
    );
    if let Some(was) = r.was {
        ui.add(
            crate::components::para(
                &format!(
                    tr!("The fund rose: it was {was}, now {now}"),
                    was = dash(was),
                    now = dash(r.fund)
                ),
                SMALL.size,
                SMALL.weight,
                p.accent,
            )
            .line_height(SMALL.line),
        );
    }
    // АРХ-6: the link's document, «≈» (a document's write has no proved bound).
    let mut link_cost = 0;
    if wallet.identity_links && matches!(link_of(form), Ok(Some(_))) {
        link_cost = wallet.fees.document_write;
        kv_row(
            ui,
            &p,
            tr!("Link for voters"),
            &format!("≈ {}", dash(link_cost)),
            value.clone(),
        );
    }
    divider(ui, &p);
    let total = wallet.fees.name_claim_total(r.fund, link_cost);
    kv_row(
        ui,
        &p,
        tr!("Total in credits"),
        &format!("≈ {}", dash(total)),
        style(UI.weight(Weight::SemiBold).mono()),
    );
    // FEAT-0057 П3 (R-3.1, R-3.4): the one line «Credits @me: now › after», after the total —
    // the vote's fund in it.
    crate::components::credits_after(
        ui,
        a,
        &crate::components::payer_name(wallet),
        crate::components::Credits::of(wallet),
        crate::components::Charge::About(total),
    );
    notice(
        ui,
        a,
        "warning",
        p.accent,
        &format!(
            tr!(
                "The fund {fund} is not returned — not if the name goes to someone else, nor if the masternodes block it."
            ),
            fund = dash(r.fund)
        ),
    );
}

/// The link typed in the review: none when empty, else checked (FEAT-0021 AC-T11).
fn link_of(form: &NameForm) -> Result<Option<String>, evolite_core::dpns::LinkProblem> {
    if form.link.trim().is_empty() {
        return Ok(None);
    }
    evolite_core::dpns::identity_link(&form.link).map(Some)
}

/// N2's footer: the password and «Back» / «Sign and send the request»; the fund «not returned»
/// is in the summary over it (BUG-249).
fn review_footer(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut NameForm,
    (register, back): (&mut bool, &mut bool),
) -> bool {
    let p = a.p;
    let entered = form.flow.password.field(ui, &p, "add-name-password", true);
    button_row(ui, |ui| {
        if button(ui, a, tr!("Back"), ButtonKind::Ghost, None).clicked() {
            *back = true;
        }
        // A link that does not pass holds the claim back, in words above (AC-T11).
        let ready = !form.flow.waiting() && form.flow.password.ready(true) && link_of(form).is_ok();
        let label = if form.flow.waiting() {
            tr!("Writing…")
        } else {
            tr!("Sign and send the request")
        };
        if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked()
            || (entered && ready)
        {
            *register = true;
        }
    });
    false
}

/// BUG-220: what the newest claim of `label` took from the credits, as its History line says.
fn claim_paid(wallet: &Wallet, label: &str) -> Option<evolite_api::Duffs> {
    wallet
        .activity
        .iter()
        .filter(|a| {
            matches!(&a.kind, evolite_api::ActivityKind::NameClaimed { label: l }
                if l.eq_ignore_ascii_case(label))
        })
        .max_by_key(|a| (a.at, a.id))
        .map(|a| a.amount.saturating_neg())
        .filter(|paid| *paid > 0)
}

/// N3: the claim is in; what it cost, when the result comes, and that the name is not yours yet.
fn sent_body(ui: &mut Ui, a: &mut Assets, wallet: &Wallet, me: &str, form: &NameForm) {
    let p = a.p;
    let Some(r) = form.review else {
        return;
    };
    // BUG-220: what the claim took, from its line in History (the credits before and after),
    // else the claim's own estimate with «≈»: never a regular name's price with the fund.
    let total = match claim_paid(wallet, &form.asked) {
        Some(paid) => dash(paid),
        None => {
            let link = if matches!(link_of(form), Ok(Some(_))) {
                wallet.fees.document_write
            } else {
                0
            };
            // BUG-225 (review S): N2t's own total, the claims' writes as metered.
            format!("≈ {}", dash(wallet.fees.name_claim_total(r.fund, link)))
        }
    };
    ui.add(
        crate::components::para(
            &if me.is_empty() {
                format!(
                    tr!("The fund and the write were paid from the identity's credits: {total}. Result about {end}. evolite tells you with a notification, even with the window minimized."),
                    total = total,
                    end = crate::text::date_time_in_line(r.ends_about, chrono::Local::now())
                )
            } else {
                format!(
                    tr!("The fund and the write were paid from the credits of {me}: {total}. Result about {end}. evolite tells you with a notification, even with the window minimized."),
                    me = me,
                    total = total,
                    end = crate::text::date_time_in_line(r.ends_about, chrono::Local::now())
                )
            },
            UI.size,
            UI.weight,
            p.text,
        )
        .line_height(UI.line),
    );
    notice(
        ui,
        a,
        "info",
        p.muted,
        tr!(
            "While the vote lasts the name is not yours yet: nobody finds you or pays you by it. Follow it in Identity › “DPNS names”."
        ),
    );
}

/// What the review (N2) shows and asks: the fund agreed to, the one before it if it rose since
/// (R-2), and the vote's window and estimated end.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Review {
    pub fund: evolite_api::Duffs,
    pub was: Option<evolite_api::Duffs>,
    pub join_until: evolite_api::Timestamp,
    pub ends_about: evolite_api::Timestamp,
}

/// The name, and what DPNS says of it. True when «Open the request ›» under «you are already in
/// the vote» was clicked (FEAT-0021 N1 answer 8, entry 5).
fn name_field(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    form: &mut NameForm,
    label: &str,
    check: &Option<Result<NameCheck, CoreError>>,
    (wallet, id): (&Wallet, &Identity),
) -> bool {
    let p = *p;
    let mut open = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, &p, tr!("Name"));
        text_field(
            ui,
            &p,
            "add-name",
            &mut form.name,
            tr!("e.g. satoshi-2026"),
            FieldKind::Text,
            |_| {},
        );
        let (line, color) = match &check {
            // Д11: as Send and «Add a contact» — after a while, that it is the network.
            None if label.len() >= 3 => {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs(1));
                (
                    Some(crate::text::name_waiting(form.asked_at).to_string()),
                    p.muted,
                )
            }
            None if !label.is_empty() => (
                Some(tr!("This is not a Dash name: 3–63 letters, digits and hyphens").to_string()),
                p.danger,
            ),
            None => (None, p.muted),
            // FEAT-0021 N1: every answer of the check, in its tone.
            Some(Ok(answer)) => {
                let (line, tone) = super::contest::check_line(
                    label,
                    *answer,
                    &wallet.fees,
                    &id.names,
                    &wallet.identities,
                    chrono::Local::now(),
                );
                (Some(line), super::contest::tone_color(tone, &p))
            }
            Some(Err(e)) => {
                let (why, typo) = text::name_problem(e);
                (Some(why), if typo { p.danger } else { p.muted })
            }
        };
        if let Some(line) = line {
            ui.add(
                crate::components::para(&line, SMALL.size, SMALL.weight, color)
                    .line_height(SMALL.line),
            );
        }
        if matches!(check, Some(Ok(NameCheck::AlreadyRequested { other: None })))
            && inline_action(ui, &p, tr!("Open the request ›")).clicked()
        {
            open = true;
        }
        // R-6: under an answer on a contested name, which letters count as one.
        if matches!(check, Some(Ok(_))) && evolite_core::dpns::is_contested(label) {
            ui.add(
                crate::components::para(
                    super::contest::similar_letters(),
                    SMALL.size,
                    SMALL.weight,
                    p.muted,
                )
                .line_height(SMALL.line),
            );
        }
    });
    open
}

/// Whether the identity's credits are enough, and the way to more.
fn credits_state(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    read: bool,
    short: bool,
    needed: evolite_api::Duffs,
    credits: evolite_api::Duffs,
    top_up: &mut bool,
) {
    let p = *p;
    if !read {
        ui.add(
            crate::components::para(
                &format!(
                    "{}.",
                    tr!(
                        "The identity's credits are not read yet: nothing is charged until they are"
                    )
                ),
                SMALL.size,
                SMALL.weight,
                p.muted,
            )
            .line_height(SMALL.line),
        );
    } else if short {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(text(
                &format!(
                    tr!("Not enough credits for the name: ≈ {} needed, the identity has {}."),
                    dash(needed),
                    crate::format::balance(credits)
                ),
                SMALL,
                p.muted,
            ));
            if inline_action(ui, &p, tr!("Top up ›")).clicked() {
                *top_up = true;
            }
        });
        ui.add(
                    crate::components::para(
                        tr!("Platform wants this much before it writes a name; what the name does not use stays on the identity."), SMALL.size, SMALL.weight, p.muted).line_height(SMALL.line),
                );
    }
}

/// While the name is written, and Platform's answer.
fn name_answer(ui: &mut Ui, a: &mut Assets, form: &NameForm) {
    let p = a.p;
    match &form.result {
        _ if form.flow.waiting() => notice(
            ui,
            a,
            "pending",
            p.muted,
            tr!("Writing the name to Platform: preorder and domain — usually under a minute."),
        ),
        Some(Ok(())) => notice(
            ui,
            a,
            "confirmed",
            p.accent,
            &format!(tr!("The name @{} is written."), form.asked),
        ),
        Some(Err(e)) => crate::components::error_notice(
            ui,
            a,
            &name_error(e),
            crate::text::error_details(e).as_deref(),
        ),
        None => {}
    }
}

/// Why the name was not registered, in words.
fn name_error(e: &CoreError) -> String {
    match e {
        // The core's reason is a catalog key: the catalog adds the language's own full stop.
        CoreError::NotAvailable(why) => format!("{}.", why.trim_end_matches('.')),
        CoreError::NotSynced => {
            tr!("Wait for the sync to finish: the masternode lists are needed.").into()
        }
        CoreError::NotEnough => tr!("Not enough credits: top up the identity.").into(),
        CoreError::BadName => {
            tr!("This is not a Dash name: 3–63 letters, digits and hyphens.").into()
        }
        other => crate::text::failed(tr!("The name is not written"), other),
    }
}

/// "Add a name" as typed: the label, the core's check of it, and the registration's answer.
#[derive(Default)]
pub struct NameForm {
    pub name: String,
    /// The label last sent to `CheckName`, and its answer.
    pub asked: String,
    pub check: Option<Result<NameCheck, CoreError>>,
    /// When [`Self::asked`] went to the core: a long silence is said in words (Д11, BUG-87).
    pub asked_at: Option<std::time::Instant>,
    pub result: Option<Result<(), CoreError>>,
    /// The name's spend (TECH-spend-flow): sent once, its own answer only, "password for every
    /// payment" in it — the name is paid from the credits.
    pub flow: crate::components::spend_flow::SpendFlow<()>,
    /// A contested name under review (FEAT-0021 N2), and after it the request sent (N3).
    pub review: Option<Review>,
    /// Opened from Identity's «Claim @x ›» (UC-5): straight to the review once the check says
    /// the name can be claimed.
    pub review_asap: bool,
    /// The link «where my identity is confirmed» typed in the review (АРХ-6).
    pub link: String,
}
