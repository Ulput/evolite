//! 19 · Contacts: DashPay requests and contacts of the identity, with their profiles. A request
//! from someone is answered with "Accept" (ours back: the contact is made) or hidden; a
//! contact opens its profile (22) or the Send screen. "Add a contact" (20) finds a name and
//! sends our request. Everything is asked of the core; nothing is written here.

use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, SpendPassword, avatar, button, button_enabled, card, dialog,
    dialog_with_header, field_label, inner, kv_row, notice, page_header, text_field,
};
use crate::context::Ctx;
use crate::text::{self, name_problem, plural, short_id};
use crate::theme::{Weight, mono, sans};
use chrono::{DateTime, Local};
use egui::{Align, Align2, Layout, Margin, Rect, Sense, Ui, pos2, vec2};
use evolite_api::{
    Activity, ActivityKind, Command, Contact, ContactState, CoreError, Profile, ResolvedName,
    Spend, Wallet,
};

const HOW: [(&str, &str, &str); 3] = [
    (
        "contacts",
        tr!("Contact request"),
        tr!(
            "Inside is a key for paying you, encrypted for the recipient. The identity's credits pay for it."
        ),
    ),
    (
        "send",
        tr!("Payments to the contact"),
        tr!(
            "Each payment goes to a new address from the contact's key: the network cannot see that one person is being paid."
        ),
    ),
    (
        "identity",
        tr!("Profile"),
        tr!("The name, «About» and avatar are public; evolite shows them next to the @name."),
    ),
];

#[derive(Default)]
pub struct ContactsState {
    /// "Add a contact" is open.
    pub adding: Option<AddContact>,
    /// The identity whose profile is open (22).
    pub profile: Option<String>,
    /// A request, an answer or "Hide" went to the core, no answer yet.
    pub waiting: bool,
    pub result: Option<Result<(), CoreError>>,
    /// "Send" on a contact: the Send screen opens with this recipient.
    pub pay: Option<String>,
    /// "Password for every payment" for a request or an answer: both cost credits (BUG-42).
    pub password: SpendPassword,
    /// "Accept" on a request with the password asked: the identity whose request waits for it.
    pub accepting: Option<String>,
    /// This frame: whether the wallet asks for its password for every spend.
    ask: bool,
}

#[derive(Default)]
pub struct AddContact {
    pub name: String,
    /// The name last sent to the core, and its answer.
    pub asked: String,
    pub resolution: Option<Result<ResolvedName, CoreError>>,
}

impl ContactsState {
    /// The core's answer to a contact request, an answer or "Hide". Only one this screen
    /// sent: "Add to contacts" on the Send screen is answered the same way.
    pub fn on_done(&mut self, result: &Result<(), CoreError>) {
        if !self.waiting {
            return;
        }
        self.waiting = false;
        self.password.settle(Some(result));
        if result.is_ok() {
            self.adding = None;
            self.accepting = None;
        }
        self.result = Some(result.clone());
    }

    /// The wallet locked: a password typed for a request goes.
    pub fn on_lock(&mut self) {
        self.password.forget();
        self.accepting = None;
    }

    pub fn on_resolved(&mut self, name: &str, result: &Result<ResolvedName, CoreError>) {
        if let Some(adding) = &mut self.adding
            && adding.asked == name
        {
            adding.resolution = Some(result.clone());
        }
    }
}

/// Who "Send" pays: the @name, or the whole identity id for a contact without a name (the
/// short id on screen is not an address the core can pay).
fn pay_to(contact: &Contact) -> String {
    match &contact.name {
        Some(name) => format!("@{name}"),
        None => contact.identity_id.clone(),
    }
}

/// "Alice Chen", or the @name when the profile has no name.
pub fn shown_name(contact: &Contact) -> String {
    contact
        .profile
        .display
        .clone()
        .unwrap_or_else(|| contact.handle())
}

/// The line under a contact's name: who they are to us.
fn about_line(c: &Contact, now: DateTime<Local>) -> String {
    let display = c.profile.display.as_deref();
    match c.state {
        ContactState::Incoming => match display {
            Some(d) => format!(tr!("{d} · wants to add you to contacts"), d = d),
            None => tr!("wants to add you to contacts").into(),
        },
        ContactState::Outgoing => format!(
            tr!("your request is sent · you can pay once {} answers"),
            display.map_or_else(|| c.handle(), str::to_owned)
        ),
        ContactState::Established => {
            let payments = if c.payments == 0 {
                tr!("no payments yet").to_string()
            } else {
                plural(c.payments, tr!("payment"), tr!("payments"))
            };
            let since = format!(tr!("contact with {}"), text::date_day(c.since, now));
            match display {
                Some(d) => format!("{d} · {since} · {payments}"),
                None => format!("{since} · {payments}"),
            }
        }
    }
}

/// The small label and value on the right of a row.
fn when_line(c: &Contact, now: DateTime<Local>) -> (&'static str, String) {
    match (c.state, c.last_payment) {
        (ContactState::Incoming, _) => (tr!("received"), text::when(c.since, now)),
        (ContactState::Outgoing, _) => (tr!("request sent"), text::when(c.since, now)),
        (ContactState::Established, Some(last)) => (tr!("last payment"), text::when(last, now)),
        (ContactState::Established, None) => (tr!("in contacts"), text::when(c.since, now)),
    }
}

pub fn contacts(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut ContactsState) {
    let p = a.p;
    state.ask = crate::components::password_asked(cx.snap);
    let wallet = cx.wallet();
    let now = cx.now;
    let mut asked: Vec<Command> = Vec::new();
    ui.spacing_mut().item_spacing.y = 22.0;
    page_header(ui, a, tr!("Contacts"), None, |ui, a| {
        let has_identity = wallet.identity.is_some();
        if button_enabled(
            ui,
            a,
            tr!("Add a contact"),
            ButtonKind::Primary,
            None,
            has_identity,
        )
        .on_disabled_hover_text(
            crate::i18n::t(tr!("An identity is needed first: it signs the request")).into_owned(),
        )
        .clicked()
        {
            state.adding = Some(AddContact::default());
            state.result = None;
        }
        button_enabled(ui, a, tr!("My QR"), ButtonKind::Secondary, None, false)
            .on_disabled_hover_text(crate::i18n::t(tr!("Requests by QR come next")).into_owned());
    });

    let requests: Vec<&Contact> = wallet
        .contacts
        .iter()
        .filter(|c| c.state == ContactState::Incoming)
        .collect();
    let known: Vec<&Contact> = wallet
        .contacts
        .iter()
        .filter(|c| c.state != ContactState::Incoming)
        .collect();

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 16.0;
        let total = ui.available_width();
        ui.add(
            egui::Label::new(
                txt(
                    tr!("DashPay contacts: you exchanged encrypted requests, and each payment goes to a new address from the contact's keys. A request is a write to Platform, paid for by the identity's credits."),
                    14.0,
                    Weight::Regular,
                    p.muted,
                )
                .line_height(Some(21.0)),
            )
            .wrap(),
        );
        if let Some(Err(e)) = &state.result {
            crate::components::error_notice(ui, a, &contact_error(e), crate::text::error_details(e).as_deref());
        }

        if !requests.is_empty() {
            ui.label(txt(
                &format!(tr!("Requests · {}"), requests.len()),
                15.0,
                Weight::SemiBold,
                p.text,
            ));
            list(ui, a, total, &requests, now, state, &mut asked);
        }
        ui.label(txt(
            &format!(tr!("Contacts · {}"), known.len()),
            15.0,
            Weight::SemiBold,
            p.text,
        ));
        if known.is_empty() {
            card(&p, Margin::same(28)).show(ui, |ui| {
                ui.set_width(inner(total, 28));
                ui.vertical_centered(|ui| {
                    ui.label(txt(
                        tr!("No contacts yet. Add someone by their @name."),
                        14.0,
                        Weight::Regular,
                        p.muted,
                    ))
                });
            });
        } else {
            list(ui, a, total, &known, now, state, &mut asked);
        }

        // Three explanations side by side, or one under another when narrow.
        let per_row = if total >= 760.0 { 3 } else { 1 };
        let w = (total - 16.0 * (per_row - 1) as f32) / per_row as f32;
        for chunk in HOW.chunks(per_row) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                for (icon, title, body) in chunk {
                    card(&p, Margin::same(18)).show(ui, |ui| {
                        ui.set_width(inner(w, 18));
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 8.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.add(a.icons.icon(icon, p.text, &p, 18.0));
                                ui.label(txt(title, 14.0, Weight::SemiBold, p.text));
                            });
                            ui.add(
                                egui::Label::new(
                                    txt(body, 12.0, Weight::Regular, p.muted)
                                        .line_height(Some(18.0)),
                                )
                                .wrap(),
                            );
                        });
                    });
                }
            });
        }
    });

    if state.adding.is_some() {
        add_contact(ui, a, cx, wallet, state, &mut asked);
    }
    accept_dialog(ui, a, wallet, state, &mut asked);
    if let Some(id) = state.profile.clone() {
        match wallet.contacts.iter().find(|c| c.identity_id == id) {
            Some(contact) => {
                profile_dialog(ui, a, contact, &wallet.activity, now, state, &mut asked)
            }
            None => state.profile = None,
        }
    }
    for command in asked {
        cx.send(command);
    }
}

fn list(
    ui: &mut Ui,
    a: &mut Assets,
    total: f32,
    rows: &[&Contact],
    now: DateTime<Local>,
    state: &mut ContactsState,
    asked: &mut Vec<Command>,
) {
    let p = a.p;
    card(&p, Margin::ZERO).show(ui, |ui| {
        ui.set_width(inner(total, 0));
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, contact) in rows.iter().enumerate() {
            if i > 0 {
                let (line, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(line, 0, p.line);
            }
            row(ui, a, contact, now, state, asked);
        }
    });
}

/// One contact: avatar, @name and short id, who they are to us, when, and what can be done.
/// A click on the row opens the profile.
fn row(
    ui: &mut Ui,
    a: &mut Assets,
    c: &Contact,
    now: DateTime<Local>,
    state: &mut ContactsState,
    asked: &mut Vec<Command>,
) {
    let p = a.p;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 77.0), Sense::click());
    crate::components::named(
        &response,
        egui::WidgetType::Button,
        true,
        &format!("{} · {}", c.handle(), shown_name(c)),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(rect.shrink(1.0), 12, p.raised);
    }
    let cy = rect.center().y;
    let face = Rect::from_min_size(pos2(rect.left() + 20.0, cy - 22.0), vec2(44.0, 44.0));
    let picture = c.profile.avatar.as_deref();
    avatar(
        ui,
        a,
        face,
        &c.identity_id,
        &shown_name(c),
        picture,
        &mut |cmd| asked.push(cmd),
    );
    let painter = ui.painter();
    let x = face.right() + 16.0;
    let handle = painter.text(
        pos2(x, cy - 2.0),
        Align2::LEFT_BOTTOM,
        crate::i18n::t(&c.handle()),
        sans(15.0, Weight::SemiBold),
        p.text,
    );
    painter.text(
        pos2(handle.right() + 8.0, handle.center().y),
        Align2::LEFT_CENTER,
        crate::i18n::t(&short_id(&c.identity_id)),
        mono(12.0, Weight::Regular),
        p.muted,
    );

    let buttons: &[(&str, ButtonKind, f32)] = match c.state {
        ContactState::Incoming => &[
            (tr!("Accept"), ButtonKind::Primary, 112.0),
            (tr!("Hide"), ButtonKind::Ghost, 96.0),
        ],
        ContactState::Established => &[(tr!("Send"), ButtonKind::Ghost, 112.0)],
        ContactState::Outgoing => &[],
    };
    let buttons_w: f32 = buttons.iter().map(|b| b.2 + 8.0).sum();
    let right = rect.right() - 20.0 - buttons_w;
    let (label, when) = when_line(c, now);
    let label_rect = painter.text(
        pos2(right, cy - 1.0),
        Align2::RIGHT_BOTTOM,
        crate::i18n::t(label),
        sans(11.0, Weight::Regular),
        p.muted,
    );
    painter.text(
        pos2(right, cy + 1.0),
        Align2::RIGHT_TOP,
        crate::i18n::t(&when),
        sans(13.0, Weight::Medium),
        p.text,
    );
    // The line under the name stops before the right column.
    let room = (label_rect.left().min(right - 140.0) - x - 16.0).max(80.0);
    let line = ui.painter().layout(
        crate::i18n::t(&about_line(c, now)).into_owned(),
        sans(13.0, Weight::Regular),
        p.muted,
        room,
    );
    let first_row = line.rows.first().map(|r| r.text()).unwrap_or_default();
    let shown = if line.rows.len() > 1 {
        format!("{}…", first_row.trim_end())
    } else {
        first_row
    };
    ui.painter().text(
        pos2(x, cy + 2.0),
        Align2::LEFT_TOP,
        crate::i18n::t(&shown),
        sans(13.0, Weight::Regular),
        p.muted,
    );

    let mut bx = rect.right() - 20.0 - buttons_w + 8.0;
    let mut clicked_button = false;
    for (label, kind, w) in buttons {
        let r = Rect::from_min_size(pos2(bx, cy - 20.5), vec2(*w, 41.0));
        // A child UI: placing the button must not move the parent's cursor.
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        let enabled = !state.waiting;
        if button_enabled(&mut child, a, label, *kind, Some(*w), enabled).clicked() {
            clicked_button = true;
            match *label {
                // With "password for every payment" the answer asks for it first.
                tr!("Accept") if state.ask => {
                    state.password.forget();
                    state.accepting = Some(c.identity_id.clone());
                    state.result = None;
                }
                tr!("Accept") => {
                    asked.push(state.password.command(
                        Spend::AcceptContact {
                            identity_id: c.identity_id.clone(),
                        },
                        false,
                    ));
                    state.waiting = true;
                    state.result = None;
                }
                tr!("Hide") => {
                    asked.push(Command::HideRequest {
                        identity_id: c.identity_id.clone(),
                    });
                    state.waiting = true;
                    state.result = None;
                }
                _ => state.pay = Some(pay_to(c)),
            }
        }
        bx += w + 8.0;
    }
    if response.clicked() && !clicked_button {
        state.profile = Some(c.identity_id.clone());
    }
}

/// Core payments to and from `handle` (the core names a contact's payments by it), newest first.
fn paid<'a>(activity: &'a [Activity], handle: &'a str) -> impl Iterator<Item = &'a Activity> {
    activity.iter().filter(move |act| match &act.kind {
        ActivityKind::Sent { to } => to == handle,
        ActivityKind::Received { from: Some(from) } => from == handle,
        _ => false,
    })
}

/// 22 · Contact profile: the picture, name, "Bio" and what we know of the contact, with its
/// latest payments.
fn profile_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    c: &Contact,
    activity: &[Activity],
    now: DateTime<Local>,
    state: &mut ContactsState,
    asked: &mut Vec<Command>,
) {
    let name = shown_name(c);
    let mut pay = false;
    let close = dialog_with_header(
        ui,
        a,
        "contact-profile",
        464.0,
        |ui, a| {
            let p = a.p;
            let (face, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
            avatar(
                ui,
                a,
                face,
                &c.identity_id,
                &name,
                c.profile.avatar.as_deref(),
                &mut |cmd| asked.push(cmd),
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(txt(&name, 20.0, Weight::Bold, p.text));
                let handle = match &c.name {
                    Some(n) => format!(tr!("@{n} · name verified"), n = n),
                    None => tr!("no DPNS name").into(),
                };
                ui.label(txt(&handle, 14.0, Weight::Medium, p.muted));
                ui.label(
                    egui::RichText::new(crate::i18n::t(&short_id(&c.identity_id)))
                        .font(mono(12.0, Weight::Regular))
                        .color(p.muted),
                )
                .on_hover_text(crate::i18n::t(&c.identity_id).into_owned());
            });
        },
        |ui, a| {
            let p = a.p;
            about_block(ui, a, &c.profile);
            let value = mono(13.0, Weight::Medium);
            let state_line = match c.state {
                ContactState::Established => format!(tr!("since {}"), text::date_long(c.since)),
                ContactState::Outgoing => tr!("waiting for an answer to your request").into(),
                ContactState::Incoming => tr!("waiting for your answer").into(),
            };
            kv_row(ui, &p, tr!("In contacts"), &state_line, value.clone());
            if c.state == ContactState::Established {
                let count = plural(c.payments, tr!("payment"), tr!("payments"));
                let payments = match c.last_payment {
                    _ if c.payments == 0 => tr!("not yet").into(),
                    Some(last) => format!(
                        tr!("{count} · last {}"),
                        text::when(last, now),
                        count = count
                    ),
                    None => count,
                };
                kv_row(ui, &p, tr!("Payments"), &payments, value.clone());
                // The latest of them, as the history names them.
                for act in paid(activity, &c.handle()).take(5) {
                    kv_row(
                        ui,
                        &p,
                        &text::when(act.at, now),
                        &text::signed_amount(act.amount),
                        mono(12.0, Weight::Regular),
                    );
                }
            }
            if let Some(at) = c.profile.updated_at {
                kv_row(ui, &p, tr!("Profile updated"), &text::date_long(at), value);
            }
            notice(
                ui,
                a,
                "shield",
                p.accent,
                &format!(
                    tr!(
                        "The name and «About» are {}'s own words: Platform confirms only the @name. The avatar is a picture by link: evolite checks its fingerprint from the profile and draws it in a separate process with no access to keys."
                    ),
                    c.handle()
                ),
            );
            let mut close = false;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if c.state == ContactState::Established
                    && button(ui, a, tr!("Send"), ButtonKind::Primary, None).clicked()
                {
                    pay = true;
                    close = true;
                }
                if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close
        },
    );
    if pay {
        state.pay = Some(pay_to(c));
    }
    if close {
        state.profile = None;
    }
}

/// "Bio" and its text, when the profile has one.
pub fn about_block(ui: &mut Ui, a: &mut Assets, profile: &Profile) {
    let p = a.p;
    let Some(about) = profile.about.as_deref().filter(|s| !s.trim().is_empty()) else {
        ui.label(txt(
            tr!("A profile without «About»"),
            13.0,
            Weight::Regular,
            p.muted,
        ));
        return;
    };
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(txt(tr!("Bio"), 12.0, Weight::Regular, p.muted));
        ui.add(
            egui::Label::new(txt(about, 14.0, Weight::Regular, p.text).line_height(Some(20.0)))
                .wrap(),
        );
    });
}

/// 20 · Add a contact: a name, the profile it leads to, the cost, "Send the request".
fn add_contact(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    wallet: &Wallet,
    state: &mut ContactsState,
    asked: &mut Vec<Command>,
) {
    let Some(adding) = &mut state.adding else {
        return;
    };
    // A name goes to the core once per change; the answer comes back as an event.
    let typed = adding.name.trim().to_string();
    if typed.trim_start_matches('@').len() >= 3 && typed != adding.asked {
        asked.push(Command::ResolveName {
            name: typed.clone(),
        });
        adding.asked = typed.clone();
        adding.resolution = None;
    }
    let found = match &adding.resolution {
        Some(Ok(found)) if adding.asked == typed => Some(found.clone()),
        _ => None,
    };
    let me = wallet
        .identity
        .as_ref()
        .and_then(|i| i.name())
        .map_or_else(|| "identity".to_string(), |n| format!("@{n}"));
    let fee = wallet.fees.contact_request;
    let waiting = state.waiting;
    let result = state.result.clone();
    // The request costs credits: "password for every payment" asks for it here.
    let ask = crate::components::password_asked(cx.snap);
    let password = &mut state.password;
    let mut send = false;
    let close = dialog(
        ui,
        a,
        "add-contact",
        "contacts",
        tr!("Add a contact"),
        464.0,
        |ui, a| {
            let p = a.p;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("Name in DPNS"));
                text_field(
                    ui,
                    &p,
                    "add-contact-name",
                    &mut adding.name,
                    tr!("@name"),
                    FieldKind::Text,
                    |_| {},
                );
            });
            let mut ready = false;
            match (&adding.resolution, &found) {
                (_, Some(found)) => {
                    let is_self = wallet
                        .identity
                        .as_ref()
                        .is_some_and(|i| i.id == found.identity_id);
                    found_box(ui, a, found, asked);
                    match (is_self, found.contact) {
                        (true, _) => notice(ui, a, "info", p.muted, tr!("This is you.")),
                        (false, Some(ContactState::Established)) => notice(
                            ui,
                            a,
                            "confirmed",
                            p.accent,
                            tr!("Already in your contacts."),
                        ),
                        (false, Some(ContactState::Outgoing)) => notice(
                            ui,
                            a,
                            "pending",
                            p.muted,
                            tr!("Your request has been sent — waiting for the answer."),
                        ),
                        (false, Some(ContactState::Incoming)) => notice(
                            ui,
                            a,
                            "info",
                            p.muted,
                            tr!(
                                "This person already sent you a request: accept it in the «Requests» list."
                            ),
                        ),
                        (false, None) => ready = true,
                    }
                }
                (Some(Err(e)), None) if adding.asked == typed => {
                    let (why, typo) = name_problem(e);
                    ui.label(txt(
                        &why,
                        12.0,
                        Weight::Regular,
                        if typo { p.danger } else { p.muted },
                    ));
                }
                // One or two letters are not asked about: say why "Send" stays off, as the Send
                // screen does for `@ab` (BUG-39 №4).
                _ if (1..3).contains(&typed.trim_start_matches('@').len()) => {
                    ui.label(txt(
                        tr!("This is not a Dash name: 3–63 letters, digits and hyphens"),
                        12.0,
                        Weight::Regular,
                        p.danger,
                    ));
                }
                _ if typed.trim_start_matches('@').len() >= 3 => {
                    ui.label(txt(
                        tr!("Checking the name in DPNS…"),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
                _ => {}
            }
            let value = mono(13.0, Weight::Medium);
            kv_row(
                ui,
                &p,
                tr!("A write to Platform"),
                &format!("≈ {}", crate::format::amount(fee)),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Paid by"),
                &format!(tr!("{me} · credits"), me = me),
                value,
            );
            let who = found
                .as_ref()
                .and_then(|f| f.display.clone())
                .unwrap_or_else(|| tr!("recipient").into());
            notice(
                ui,
                a,
                "info",
                p.muted,
                &format!(
                    tr!(
                        "You can pay once {who} answers. The request carries a key for paying you, encrypted for the recipient; evolite notices the answer by itself while syncing."
                    ),
                    who = who
                ),
            );
            if let Some(Err(e)) = &result
                && !password.is_wrong()
            {
                crate::components::error_notice(
                    ui,
                    a,
                    &contact_error(e),
                    crate::text::error_details(e).as_deref(),
                );
            }
            let entered = password.field(ui, &p, "add-contact-password", ask);
            let ready = ready && !waiting && password.ready(ask);
            let mut close = false;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let label = if waiting {
                    tr!("Sending…")
                } else {
                    tr!("Send the request")
                };
                if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked()
                    || (entered && ready)
                {
                    send = true;
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close
        },
    );
    if send {
        cx.send(
            state
                .password
                .command(Spend::AddContact { name: typed }, ask),
        );
        state.waiting = true;
        state.result = None;
    } else if close && !waiting {
        state.adding = None;
        state.password.forget();
    }
}

/// "Accept" with "password for every payment" on: the answer costs credits, so the password
/// comes first, in a dialog of its own (the request row has no room for it).
fn accept_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    wallet: &Wallet,
    state: &mut ContactsState,
    asked: &mut Vec<Command>,
) {
    let Some(id) = state.accepting.clone() else {
        return;
    };
    let Some(contact) = wallet.contacts.iter().find(|c| c.identity_id == id) else {
        state.accepting = None;
        return;
    };
    let p = a.p;
    let who = shown_name(contact);
    let waiting = state.waiting;
    let mut accept = false;
    let closed = dialog(
        ui,
        a,
        "accept-contact",
        "contacts",
        tr!("Accept the request"),
        440.0,
        |ui, a| {
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.label(txt(
                &format!(
                    tr!("{who} becomes a contact: you can pay each other. The answer costs ≈ {fee} of the identity's credits."),
                    who = crate::i18n::own(&who),
                    fee = crate::format::amount(wallet.fees.contact_request)
                ),
                14.0,
                Weight::Regular,
                p.text,
            ));
            if let Some(Err(e)) = &state.result
                && !state.password.is_wrong()
            {
                crate::components::error_notice(
                    ui,
                    a,
                    &contact_error(e),
                    crate::text::error_details(e).as_deref(),
                );
            }
            let entered = state
                .password
                .field(ui, &p, "accept-contact-password", true);
            let ready = !waiting && state.password.ready(true);
            let mut close = false;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let label = if waiting {
                    tr!("Sending…")
                } else {
                    tr!("Accept")
                };
                if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked()
                    || (entered && ready)
                {
                    accept = true;
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close
        },
    );
    if accept {
        asked.push(
            state
                .password
                .command(Spend::AcceptContact { identity_id: id }, true),
        );
        state.waiting = true;
        state.result = None;
    } else if closed && !waiting {
        state.accepting = None;
        state.password.forget();
    }
}

/// The profile a name leads to: picture, name, "Bio", id.
fn found_box(ui: &mut Ui, a: &mut Assets, found: &ResolvedName, asked: &mut Vec<Command>) {
    let p = a.p;
    let display = found
        .display
        .clone()
        .unwrap_or_else(|| format!("@{}", found.name));
    let profile = found.profile.clone().unwrap_or_default();
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(12)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (face, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                avatar(
                    ui,
                    a,
                    face,
                    &found.identity_id,
                    &display,
                    profile.avatar.as_deref(),
                    &mut |cmd| asked.push(cmd),
                );
                ui.label(txt(&display, 14.0, Weight::SemiBold, p.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let note = if found.profile.is_some() {
                        tr!("profile")
                    } else {
                        tr!("no profile")
                    };
                    ui.label(txt(note, 12.0, Weight::Regular, p.muted));
                });
            });
            if let Some(about) = &profile.about {
                ui.add(
                    egui::Label::new(
                        txt(&format!("«{about}»"), 12.0, Weight::Regular, p.muted)
                            .line_height(Some(17.0)),
                    )
                    .wrap(),
                );
            }
            ui.add(
                egui::Label::new(
                    txt(
                        &format!(
                            tr!("identity {} · @{} verified by a Platform proof"),
                            short_id(&found.identity_id),
                            found.name
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(Some(17.0)),
                )
                .wrap(),
            );
        });
}

/// Why a contact action did not happen, in words.
pub fn contact_error(e: &CoreError) -> String {
    match e {
        CoreError::NotAvailable(why) => format!("{why}."),
        CoreError::NotSynced => {
            tr!("Wait for the sync: the masternode lists and the identity are needed.").into()
        }
        CoreError::NotEnough => {
            tr!("The identity does not have enough credits for the write.").into()
        }
        CoreError::NameNotFound => tr!("There is no such name in DPNS.").into(),
        CoreError::BadName => {
            tr!("This is not a Dash name: 3–63 letters, digits and hyphens.").into()
        }
        other => crate::text::failed(tr!("It did not work"), other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::{Chain, Proof};

    fn op(kind: ActivityKind) -> Activity {
        Activity {
            id: 1,
            kind,
            chain: Chain::Core,
            at: 0,
            amount: 1,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: None,
        }
    }

    #[test]
    fn lists_a_contacts_payments_both_ways() {
        let history = [
            op(ActivityKind::Sent { to: "@bob".into() }),
            op(ActivityKind::Received {
                from: Some("@bob".into()),
            }),
            op(ActivityKind::Sent {
                to: "@alice".into(),
            }),
            op(ActivityKind::CreditTransfer { to: "@bob".into() }),
        ];
        assert_eq!(paid(&history, "@bob").count(), 2, "Core payments only");
    }

    /// "Add to contacts" from the Send screen is answered as `contact` too: the open
    /// "Add a contact" here stays, and shows no result of a request it did not send.
    #[test]
    fn an_answer_to_the_send_screen_is_not_taken_here() {
        let mut state = ContactsState {
            adding: Some(AddContact::default()),
            ..Default::default()
        };
        state.on_done(&Ok(()));
        assert!(state.adding.is_some() && state.result.is_none());
        state.waiting = true;
        state.on_done(&Ok(()));
        assert!(state.adding.is_none() && state.result == Some(Ok(())));
    }

    #[test]
    fn a_picture_that_did_not_download_is_said_whole_in_every_language() {
        // BUG-15 (reopened): the core's reason has its own colon ("…: PNG, JPEG, …"), and
        // "{}: {}." took the whole sentence by that colon, leaving the reason English.
        use crate::i18n::{Lang, t_in};
        let reasons = [
            "the link is not a picture: PNG, JPEG, GIF, WebP or SVG is needed",
            "the picture is over 1 MB",
            "an address on this computer or the local network — such links are not opened",
        ];
        for reason in reasons {
            let why = format!(tr!("The picture did not download: {e}"), e = reason);
            let shown = contact_error(&CoreError::NotAvailable(why));
            for lang in Lang::ALL.into_iter().filter(|l| *l != Lang::En) {
                let t = t_in(lang, &shown);
                for english in ["did not download", reason] {
                    assert!(!t.contains(english), "{lang:?}: {t}");
                }
            }
        }
    }
}
