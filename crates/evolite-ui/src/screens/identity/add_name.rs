//! Registering another name: the dialog and its errors.

use super::*;

/// "Add a name": one more DPNS name for this identity, checked live like on "Create an
/// identity" and paid from its credits. A contested name cannot be taken here: masternodes
/// vote on it.
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
    let me = id
        .name()
        .map_or_else(|| "identity".to_string(), |n| format!("@{n}"));
    let fee = wallet.fees.name_price(id.names.is_empty());
    // BUG-97: what the name wants on the identity up front, not its price.
    let needed = wallet.fees.name_credits_needed();
    let credits = wallet.balances.credits;
    let read = wallet.balances.credits_read;
    // The name goes to the core once per change; the answer comes back as an event.
    let label = form.name.trim().trim_start_matches('@').to_lowercase();
    if label.len() >= 3 && label != form.asked {
        asked.push(Command::CheckName {
            label: label.clone(),
        });
        form.asked = label.clone();
        form.check = None;
        form.result = None;
    }
    let check = if label.is_empty() || label != form.asked {
        None
    } else {
        form.check.clone()
    };
    let mut register = false;
    let mut top_up = false;
    let close = dialog(
        ui,
        a,
        "add-name",
        "username",
        tr!("Add a name"),
        464.0,
        |ui, a| {
            let p = a.p;
            ui.add(
                egui::Label::new(
                    txt(
                        &format!(
                            tr!("One more DPNS name for {me}: people can find you and pay you by it too.")
                        , me = me),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(Some(20.0)),
                )
                .wrap(),
            );
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
                    None if label.len() >= 3 => {
                        (Some(tr!("Checking the name in DPNS…").to_string()), p.muted)
                    }
                    None if !label.is_empty() => (
                        Some(tr!("This is not a Dash name: 3–63 letters, digits and hyphens").to_string()),
                        p.danger,
                    ),
                    None => (None, p.muted),
                    Some(Ok(NameCheck::Available)) => (
                        Some(format!(tr!("@{label} is free · a regular name, no vote"), label = label)),
                        p.text,
                    ),
                    Some(Ok(NameCheck::Contested)) => (
                        Some(format!(
                            tr!("@{label} is a contested name: masternodes award it by vote, which evolite cannot do yet. Add a digit from 2 to 9 or make the name longer than 19 characters.")
                        , label = label)),
                        p.danger,
                    ),
                    Some(Ok(NameCheck::Taken)) => (Some(format!(tr!("@{label} is already taken"), label = label)), p.danger),
                    Some(Err(e)) => {
                        let (why, typo) = text::name_problem(e);
                        (Some(why), if typo { p.danger } else { p.muted })
                    }
                };
                if let Some(line) = line {
                    ui.add(
                        egui::Label::new(txt(&line, 12.0, Weight::Regular, color)).wrap(),
                    );
                }
            });
            let value = mono(13.0, Weight::Medium);
            kv_row(
                ui,
                &p,
                tr!("Name registration"),
                &format!("≈ {}", dash(fee)),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Paid by"),
                &format!(tr!("{me} · credits"), me = me),
                value,
            );
            // Not an error before anything was tried (calm states CS-1): what is missing, muted,
            // and the way to it.
            let short = read && credits < needed;
            if !read {
                ui.add(
                    egui::Label::new(txt(
                        &format!(
                            "{}.",
                            tr!("The identity's credits are not read yet: nothing is charged until they are")
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ))
                    .wrap(),
                );
            } else if short {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(txt(
                        &format!(
                            tr!(
                                "Not enough credits for the name: ≈ {} needed, the identity has {}."
                            ),
                            dash(needed),
                            crate::format::balance(credits)
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    if inline_action(ui, &p, tr!("Top up ›")).clicked() {
                        top_up = true;
                    }
                });
                ui.add(
                    egui::Label::new(txt(
                        tr!("Platform wants this much before it writes a name; what the name does not use stays on the identity."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ))
                    .wrap(),
                );
            }
            match &form.result {
                _ if form.waiting => notice(
                    ui,
                    a,
                    "pending",
                    p.muted,
                    tr!(
                        "Writing the name to Platform: preorder and domain — usually under a minute."
                    ),
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
            let ask = wallet.password_each_send && form.result.is_none();
            let entered = form.password.field(ui, &p, "add-name-password", ask);
            let mut close = false;
            button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                let ready = matches!(check, Some(Ok(NameCheck::Available)))
                    && read
                    && !short
                    && !form.waiting
                    && form.result.is_none()
                    && form.password.ready(ask);
                let label = if form.waiting {
                    tr!("Writing…")
                } else {
                    tr!("Register")
                };
                if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked()
                    || (entered && ready)
                {
                    register = true;
                }
            });
            close
        },
    );
    if register {
        let op = Spend::AddName {
            name: form.asked.clone(),
        };
        asked.push(form.password.command(op, wallet.password_each_send));
        form.waiting = true;
        form.result = None;
    } else if top_up && !form.waiting {
        // «Top up ›»: the name waits, the top-up opens (or says why it cannot now).
        state.adding_name = None;
        state.top_up_clicked(wallet);
    } else if close && !form.waiting {
        state.adding_name = None;
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
    /// Sent to the core, no answer yet.
    pub waiting: bool,
    pub result: Option<Result<(), CoreError>>,
    /// "Password for every payment": the name is paid from the credits.
    pub password: SpendPassword,
}
