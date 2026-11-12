//! 19 · Contacts: DashPay requests and contacts of the identity, with their profiles. A request
//! from someone is answered with "Accept" (ours back: the contact is made) or hidden; a
//! contact opens its profile (22) or the Send screen. "Add a contact" (20) finds a name and
//! sends our request. Everything is asked of the core; nothing is written here.

use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, SpendPassword, avatar, button, button_enabled, button_row, card,
    dialog, dialog_with_header, field_label, inner, kv_row, notice, page_header, text_field,
};
use crate::context::Ctx;
use crate::text::{self, name_problem, plural, short_id};
use crate::theme::{Weight, mono, sans};
use chrono::{DateTime, Local};
use egui::{Align, Align2, Layout, Margin, Rect, Sense, Ui, pos2, vec2};
use evolite_api::{
    Activity, ActivityKind, Command, Contact, ContactState, CoreError, Duffs, Profile,
    ResolvedName, Spend, Wallet,
};

const HOW: [(&str, &str, &str); 3] = [
    (
        "contacts",
        tr!("Contact request"),
        tr!(
            "Visible to everyone, forever: who, to whom and when. Inside is a key for paying you, encrypted for the recipient. The identity's credits pay for it."
        ),
    ),
    (
        "send",
        tr!("Payments to the contact"),
        tr!(
            "Each payment goes to a new address from the contact's key: the request does not link the payments. Blockchain analysis may still link them."
        ),
    ),
    (
        "identity",
        tr!("Profile"),
        tr!("The name, “About” and avatar are public; evolite shows them next to the @name."),
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
    /// "Accept" on a request: the identity whose request waits in "Accept the request" (what
    /// the answer makes public, and the password when asked).
    pub accepting: Option<String>,
    /// "Contact privacy" is open (PRIV-1).
    pub privacy: bool,
    /// "Contact privacy" closed: its link takes the focus back.
    privacy_closed: bool,
    /// The credits and identity when "not enough credits" was first shown: it goes once they change.
    refused_with: Option<(Duffs, Option<String>)>,
    /// This frame: whether the wallet asks for its password for every spend.
    ask: bool,
    /// «My QR» is open (FR-8).
    pub my_qr: bool,
}

#[derive(Default)]
pub struct AddContact {
    pub name: String,
    /// The name last sent to the core, and its answer.
    pub asked: String,
    pub resolution: Option<Result<ResolvedName, CoreError>>,
    /// «QR» in the name field (FEAT-0006), and what it said until the name is typed again.
    pub scanner: crate::qr_scan::Scanner,
    pub qr_error: Option<&'static str>,
    /// When [`Self::asked`] went to the core: a long silence is said in words (BUG-87).
    pub asked_at: Option<std::time::Instant>,
}

impl ContactsState {
    /// BUG-110: "Not enough credits" stays until the identity has more or another identity is
    /// worked as. `credits`: `None` while the balance is not read yet.
    fn forget_not_enough_after_top_up(&mut self, credits: Option<Duffs>, identity: Option<&str>) {
        if !matches!(self.result, Some(Err(CoreError::NotEnough))) {
            self.refused_with = None;
            return;
        }
        let Some(credits) = credits else {
            return;
        };
        match &self.refused_with {
            None => self.refused_with = Some((credits, identity.map(str::to_owned))),
            Some((at, who)) if credits > *at || who.as_deref() != identity => {
                self.result = None;
                self.refused_with = None;
            }
            Some(_) => {}
        }
    }

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
        // A new answer is measured from the balance it comes at (BUG-110 r1).
        self.refused_with = None;
    }

    /// The wallet locked: a password typed for a request goes, and the dialogs close —
    /// «Contact privacy» too, so it does not open again after unlocking, nor its link take the
    /// focus on the next visit.
    pub fn on_lock(&mut self) {
        self.password.forget();
        self.accepting = None;
        self.privacy = false;
        self.privacy_closed = false;
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
pub(crate) fn about_line(c: &Contact, now: DateTime<Local>) -> String {
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

/// The line under a contact's name as drawn, at most `room` wide: whole, or cut at a word with
/// "…" (in reading order, so in Arabic the left end goes: BUG-117).
pub(crate) fn about_shown(
    ctx: &egui::Context,
    about: &str,
    font: egui::FontId,
    room: f32,
) -> String {
    crate::i18n::t_fitted(about, |shown| {
        let width = ctx.fonts_mut(|f| {
            f.layout_no_wrap(shown.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
                .size()
                .x
        });
        width <= room
    })
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
        let named = wallet.identity.as_ref().and_then(|i| i.name()).is_some();
        if button_enabled(ui, a, tr!("My QR"), ButtonKind::Secondary, None, named)
            .on_disabled_hover_text(
                crate::i18n::t(tr!("An identity with a name is needed first")).into_owned(),
            )
            .clicked()
        {
            state.my_qr = true;
        }
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
                    tr!("DashPay contacts: you exchanged requests. The requests themselves are public; the payment keys inside them are encrypted. Each payment goes to a new address from the contact's keys. A request is a write to Platform paid for by the identity's credits."),
                    14.0,
                    Weight::Regular,
                    p.muted,
                )
                .line_height(Some(21.0)),
            )
            .wrap(),
        );
        state.forget_not_enough_after_top_up(
            known_credits(wallet),
            wallet.identity.as_ref().map(|i| i.id.as_str()),
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

        // Three explanations side by side from about 760 px, else one under another. The
        // cards of a row are as tall as the tallest (FEAT-0034 item 6), as on Apps.
        let spec = crate::components::GridSpec {
            // 3 × 242 + 2 × 16 = 758: three a row from about 760 px, as before.
            min_w: 242.0,
            gap: egui::vec2(16.0, 16.0),
            max_cols: 3,
            even: true,
        };
        crate::components::grid(ui, HOW.len(), spec, |ui, i, w, cell| {
            let (icon, title, body) = HOW[i];
            cell.card(ui, card(&p, Margin::same(18)), |ui| {
                ui.set_width(inner(w, 18));
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.add(a.icons.icon(icon, p.text, &p, 18.0));
                    ui.label(txt(title, 14.0, Weight::SemiBold, p.text));
                });
                ui.add(
                    egui::Label::new(
                        txt(body, 12.0, Weight::Regular, p.muted).line_height(Some(18.0)),
                    )
                    .wrap(),
                );
            });
        });
        privacy_link(ui, a, state);
    });

    if state.adding.is_some() {
        add_contact(ui, a, cx, wallet, state, &mut asked);
    }
    if state.my_qr {
        my_qr(ui, a, wallet, &mut state.my_qr);
    }
    accept_dialog(ui, a, wallet, state, &mut asked);
    privacy_dialog(ui, a, state);
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
    ui.painter().text(
        pos2(x, cy + 2.0),
        Align2::LEFT_TOP,
        about_shown(
            ui.ctx(),
            &about_line(c, now),
            sans(13.0, Weight::Regular),
            room,
        ),
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
        let mut resp = button_enabled(&mut child, a, label, *kind, Some(*w), enabled);
        if *label == tr!("Hide") {
            resp = hide_hint(&child, resp);
        }
        if resp.clicked() {
            clicked_button = true;
            match *label {
                // The answer is public for good: "Accept the request" says so first, and asks
                // for the password when the wallet wants it (PRIV-1, BUG-42).
                tr!("Accept") => {
                    state.password.forget();
                    state.accepting = Some(c.identity_id.clone());
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
                // The latest of them, as the history names them — and hides them (BUG-73):
                // the app's one «Hide amounts» beside «Payments» when there are some.
                let handle = c.handle();
                let latest: Vec<_> = paid(activity, &handle).take(5).collect();
                if latest.is_empty() {
                    kv_row(ui, &p, tr!("Payments"), &payments, value.clone());
                } else {
                    crate::components::kv_row_eye(ui, a, tr!("Payments"), &payments, value.clone());
                }
                for act in latest {
                    kv_row(
                        ui,
                        &p,
                        &text::when(act.at, now),
                        &crate::format::signed_balance(act.amount),
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
                        "The name and “About” are {}'s own words: Platform confirms only the @name. The avatar is a picture by link: evolite checks its fingerprint from the profile and draws it in a separate process with no access to keys."
                    ),
                    c.handle()
                ),
            );
            let mut close = false;
            button_row(ui, |ui| {
                if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                if c.state == ContactState::Established
                    && button(ui, a, tr!("Send"), ButtonKind::Primary, None).clicked()
                {
                    pay = true;
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
            tr!("A profile without “About”"),
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
    // A picture dropped on the dialog, then what a scan brought: a name goes into the field as
    // if typed (UC-5); anything else says why and leaves what was typed.
    if let Some(picture) = crate::qr_scan::dropped(ui.ctx()) {
        adding.scanner.start(ui.ctx(), picture);
    }
    if let Some(scanned) = adding.scanner.poll() {
        match name_from_qr(&scanned) {
            Ok(name) => {
                adding.name = name;
                adding.qr_error = None;
            }
            Err(why) => adding.qr_error = why,
        }
    }
    // A name goes to the core once per change; the answer comes back as an event.
    let typed = adding.name.trim().to_string();
    if typed.trim_start_matches('@').len() >= 3 && typed != adding.asked {
        asked.push(Command::ResolveName {
            name: typed.clone(),
        });
        adding.asked = typed.clone();
        adding.resolution = None;
        adding.asked_at = Some(std::time::Instant::now());
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
                let scanner = &mut adding.scanner;
                let typed = text_field(
                    ui,
                    &p,
                    "add-contact-name",
                    &mut adding.name,
                    tr!("@name"),
                    FieldKind::Text,
                    |ui| {
                        crate::components::qr_button(ui, a, "add-contact", scanner);
                    },
                );
                if typed.changed() {
                    adding.qr_error = None;
                }
                if let Some(why) = adding.qr_error {
                    crate::components::qr_note(ui, a, &crate::components::QrNote::Error(why));
                }
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
                                "This person already sent you a request: accept it in the “Requests” list."
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
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs(1));
                    ui.label(txt(
                        text::name_waiting(adding.asked_at),
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
            reserve_notice(ui, &p, wallet);
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
            public_line(
                ui,
                a,
                tr!(
                    "The request, and the fact that you are contacts, are visible to everyone and can't be deleted. The addresses for payments between you are hidden."
                ),
            );
            let ready = ready && !waiting && password.ready(ask);
            let mut close = false;
            button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
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
            });
            // A picture dragged over the dialog: the overlay covers it (Figma 10).
            if crate::qr_scan::dragging_picture(ui.ctx()) {
                crate::components::drop_overlay(ui.ctx(), a, ui.min_rect().expand(12.0));
            }
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

/// What a QR brings to «Name in DPNS» (UC-5): `@name`, or the name of a `dash:` link's `user`;
/// else why not (`None`: the file dialog was closed, nothing to say).
fn name_from_qr(scanned: &crate::qr_scan::Scanned) -> Result<String, Option<&'static str>> {
    use crate::qr_scan::{Kind, Scanned};
    let no_name = tr!("There is no @name in this QR code");
    match scanned {
        Scanned::Found {
            text,
            kind: Kind::Name,
        } => Ok(text.clone()),
        Scanned::Found {
            text,
            kind: Kind::Uri,
        } => match crate::payment_uri::parse(text) {
            Some(Ok(link)) if link.user.is_some() => Ok(link.recipient()),
            Some(Ok(_)) => Err(Some(no_name)),
            _ => Err(Some(tr!("This is not a Dash address or an app link"))),
        },
        Scanned::Found {
            kind: Kind::Address,
            ..
        } => Err(Some(no_name)),
        Scanned::Found {
            kind: Kind::AppLink,
            ..
        } => Err(Some(tr!(
            "This is an app link: open it on the “Apps” screen"
        ))),
        other => Err(crate::components::qr_error(other)),
    }
}

/// «My QR» (FR-8, Figma `dialog/my-qr`): the identity's name and the address to receive on now
/// as `dash:<address>?user=<name>`, as DashPay shows it; no amount.
fn my_qr(ui: &mut Ui, a: &mut Assets, wallet: &Wallet, open: &mut bool) {
    let Some(name) = wallet.identity.as_ref().and_then(|i| i.name()) else {
        *open = false;
        return;
    };
    let link = crate::payment_uri::my_link(&wallet.core_address, name);
    let title = format!(tr!("My QR · @{}"), name);
    let copy_id = egui::Id::new("my-qr-copy").with(&link);
    let closed = dialog(ui, a, "my-qr", "scan", &title, 440.0, |ui, a| {
        let p = a.p;
        ui.add(
            egui::Label::new(txt(
                tr!("Show this code: DashPay or evolite opens a payment to you. It holds your name and your current receiving address."),
                13.0,
                Weight::Regular,
                p.muted,
            ))
            .wrap(),
        );
        if wallet.core_address.is_empty() {
            super::receive::qr_waiting(ui, &p);
        } else {
            super::receive::qr_card(ui, &p, &link);
        }
        // The link whole, in Mono; in Arabic it keeps its own direction (`own`).
        ui.add(
            egui::Label::new(
                txt(&crate::i18n::own(&link), 12.0, Weight::Regular, p.text)
                    .font(mono(12.0, Weight::Regular)),
            )
            .wrap(),
        );
        kv_row(
            ui,
            &p,
            tr!("Work as"),
            &crate::i18n::own(&format!("@{name}")),
            sans(13.0, Weight::SemiBold),
        );
        let mut close = false;
        button_row(ui, |ui| {
            if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
                close = true;
            }
            let copied = crate::clipboard::just_copied(ui.ctx(), copy_id);
            let label = if copied {
                tr!("Copied")
            } else {
                tr!("Copy the link")
            };
            if button(ui, a, label, ButtonKind::Secondary, None).clicked() {
                crate::clipboard::copy_answered(ui.ctx(), link.clone(), copy_id);
            }
        });
        close
    });
    if closed {
        *open = false;
    }
}

/// "Accept the request": what the answer makes public, before it goes (PRIV-1), and the password
/// when the wallet asks for it for every payment: the answer costs credits (BUG-42). The request
/// row has no room for either.
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
    let ask = state.ask;
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
            reserve_notice(ui, &p, wallet);
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
            let entered = state.password.field(ui, &p, "accept-contact-password", ask);
            public_line(
                ui,
                a,
                tr!(
                    "Your answer makes you contacts: everyone can see it, and it can't be deleted. The addresses for payments between you are hidden."
                ),
            );
            let ready = !waiting && state.password.ready(ask);
            let mut close = false;
            button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
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
            });
            close
        },
    );
    if accept {
        asked.push(
            state
                .password
                .command(Spend::AcceptContact { identity_id: id }, ask),
        );
        state.waiting = true;
        state.result = None;
    } else if closed && !waiting {
        state.accepting = None;
        state.password.forget();
    }
}

/// The last line before a dialog's buttons when it sends a contact request or an answer: what
/// becomes public for good and what stays hidden (PRIV-1). Always there, not only on an error.
fn public_line(ui: &mut Ui, a: &mut Assets, text: &str) {
    let p = a.p;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add(a.icons.icon("info", p.muted, &p, 16.0));
        ui.add(
            egui::Label::new(txt(text, 12.0, Weight::Regular, p.muted).line_height(Some(17.0)))
                .wrap(),
        );
    });
}

/// What "Hide" does: a request is public and stays so; hiding is local (PRIV-1).
const HIDE_HINT: &str =
    tr!("Hides the request on this computer only. On the network it stays, visible to everyone.");

/// "Hide" says what it does on hover and on keyboard focus, and the same words to a screen
/// reader as its description.
fn hide_hint(ui: &Ui, resp: egui::Response) -> egui::Response {
    let words = crate::i18n::t(HIDE_HINT).into_owned();
    let spoken = crate::i18n::spoken(HIDE_HINT);
    ui.ctx()
        .accesskit_node_builder(resp.id, |node| node.set_description(spoken));
    if resp.has_focus() && !resp.hovered() {
        resp.show_tooltip_text(words.clone());
    }
    resp.on_hover_text(words)
}

/// "Learn more about privacy ›" under the explanations: opens "Contact privacy".
fn privacy_link(ui: &mut Ui, a: &mut Assets, state: &mut ContactsState) {
    let p = a.p;
    let label = tr!("Learn more about privacy ›");
    let resp = ui
        .add(egui::Button::new(txt(label, 13.0, Weight::SemiBold, p.accent)).frame(false))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    crate::components::named(&resp, egui::WidgetType::Button, true, label);
    // The dialog closed: the focus comes back here once the page takes input again (egui drops
    // a focus given to a widget still under last frame's modal layer).
    if state.privacy_closed && ui.memory(|m| m.allows_interaction(ui.layer_id())) {
        state.privacy_closed = false;
        resp.request_focus();
    }
    if resp.clicked() {
        state.privacy = true;
    }
}

/// "Contact privacy": what DashPay makes public and what it keeps hidden, as verified in
/// docs/analysis/dashpay-privacy-2026-11-07.md. «Got it», the cross or Escape close it, and the
/// focus goes back to the link.
fn privacy_dialog(ui: &mut Ui, a: &mut Assets, state: &mut ContactsState) {
    if !state.privacy {
        return;
    }
    const POINTS: [&str; 5] = [
        tr!(
            "Who sent a request to whom, and when, is visible to everyone, forever. A request can't be deleted or withdrawn."
        ),
        tr!(
            "Accepting a request means sending one back, so everyone can see who is in contact with whom. Unanswered requests are visible too."
        ),
        tr!(
            "Anyone can send you a request without your consent. “Hide” removes it from this computer only."
        ),
        tr!(
            "The profile (name, “About”, avatar) and @names are public. The avatar loads from someone else's server, which sees your IP."
        ),
        tr!(
            "The payment keys are encrypted, and each payment goes to a new address: the requests do not link the payments. Blockchain analysis may still link them."
        ),
    ];
    let closed = dialog(
        ui,
        a,
        "contacts-privacy",
        "info",
        tr!("Contact privacy"),
        520.0,
        |ui, a| {
            let p = a.p;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                for point in POINTS {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(txt("•", 14.0, Weight::Regular, p.muted));
                        ui.add(
                            egui::Label::new(
                                txt(point, 14.0, Weight::Regular, p.text).line_height(Some(20.0)),
                            )
                            .wrap(),
                        );
                    });
                }
            });
            let mut close = false;
            button_row(ui, |ui| {
                if button(ui, a, tr!("Got it"), ButtonKind::Primary, None).clicked() {
                    close = true;
                }
            });
            close
        },
    );
    if closed {
        state.privacy = false;
        state.privacy_closed = true;
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

/// What a dialog says when the identity has less than the network wants before it writes (BUG-107):
/// the price is not the whole of it, and the write is refused below this. Nothing while the balance
/// is not read yet.
fn balance_warning(credits: Option<Duffs>, needed: Duffs) -> Option<String> {
    (credits? < needed).then(|| {
        format!(
            tr!("The network checks a reserve before it writes: with less than ≈ {amount} on the identity it may refuse. Less is charged."),
            amount = crate::format::amount(needed)
        )
    })
}

/// The identity's credits, once read and checked in this run: kept ones claim no lack
/// (BUG-116).
fn known_credits(wallet: &Wallet) -> Option<Duffs> {
    (wallet.balances.credits_read && wallet.balances.credits_checked)
        .then_some(wallet.balances.credits)
}

/// [`balance_warning`] for whatever sends a contact request or an answer.
pub(super) fn reserve_warning(wallet: &Wallet) -> Option<String> {
    balance_warning(known_credits(wallet), wallet.fees.contact_request_balance)
}

/// [`reserve_warning`] as a line of text.
pub(super) fn reserve_line(ui: &mut Ui, p: &crate::theme::Palette, warning: &str) {
    ui.add(egui::Label::new(txt(warning, 13.0, Weight::Regular, p.muted)).wrap());
}

/// [`reserve_line`] in a dialog, if the identity has too little.
fn reserve_notice(ui: &mut Ui, p: &crate::theme::Palette, wallet: &Wallet) {
    if let Some(warning) = reserve_warning(wallet) {
        reserve_line(ui, p, &warning);
    }
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

    /// BUG-107: below what the network wants the dialog says so, at it or above nothing is added.
    #[test]
    fn the_dialog_warns_only_below_the_balance_the_network_wants() {
        assert!(balance_warning(Some(126_042), 230_000).is_some());
        assert!(balance_warning(Some(230_000), 230_000).is_none());
        assert!(
            balance_warning(None, 230_000).is_none(),
            "a balance not read yet says nothing"
        );
    }

    /// BUG-110.
    #[test]
    fn not_enough_credits_goes_when_the_credits_or_identity_change() {
        let mut state = ContactsState {
            result: Some(Err(CoreError::NotEnough)),
            ..Default::default()
        };
        state.forget_not_enough_after_top_up(Some(100), Some("a"));
        assert!(state.result.is_some(), "as it was: it stays");
        state.forget_not_enough_after_top_up(Some(100), Some("a"));
        assert!(state.result.is_some(), "the same credits: it stays");
        state.forget_not_enough_after_top_up(Some(300), Some("a"));
        assert!(state.result.is_none(), "topped up: gone");

        state.result = Some(Err(CoreError::NotEnough));
        state.forget_not_enough_after_top_up(Some(100), Some("a"));
        state.forget_not_enough_after_top_up(Some(100), Some("b"));
        assert!(state.result.is_none(), "another identity: gone");

        state.result = Some(Err(CoreError::WrongPassword));
        state.forget_not_enough_after_top_up(Some(100), Some("a"));
        state.forget_not_enough_after_top_up(Some(900), Some("a"));
        assert!(state.result.is_some(), "another refusal is left alone");

        state.result = Some(Err(CoreError::NotEnough));
        state.forget_not_enough_after_top_up(None, Some("a"));
        state.forget_not_enough_after_top_up(Some(5), Some("a"));
        state.forget_not_enough_after_top_up(Some(5), Some("a"));
        assert!(
            state.result.is_some(),
            "a balance read only now is no top-up"
        );
    }

    /// BUG-110 r1 P3: a new refusal starts from the balance it came at, not the first one's.
    #[test]
    fn a_second_refusal_is_measured_from_its_own_balance() {
        let mut state = ContactsState {
            waiting: true,
            ..Default::default()
        };
        state.on_done(&Err(CoreError::NotEnough));
        state.forget_not_enough_after_top_up(Some(100), Some("a"));
        state.waiting = true;
        state.on_done(&Err(CoreError::NotEnough));
        state.forget_not_enough_after_top_up(Some(50), Some("a"));
        state.forget_not_enough_after_top_up(Some(80), Some("a"));
        assert!(state.result.is_none(), "topped up from 50 to 80: gone");
    }

    /// PRIV-1 r1 P3: a lock closes «Contact privacy» with the other dialogs, and its link does
    /// not take the focus on the next visit.
    #[test]
    fn a_lock_closes_the_privacy_dialog() {
        let mut state = ContactsState {
            accepting: Some("id".into()),
            privacy: true,
            privacy_closed: true,
            ..Default::default()
        };
        state.on_lock();
        assert!(state.accepting.is_none());
        assert!(!state.privacy);
        assert!(!state.privacy_closed);
    }

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
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            detail: None,
            note: None,
            rate: None,
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
