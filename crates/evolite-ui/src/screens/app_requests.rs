//! 11–13 · Requests from apps, decoded by the core and shown as they will be signed: sign-in
//! with App Connect, signing one operation, and the key exchange over QR. Plus the dialog to
//! paste a link. The answer goes back to the core; nothing here holds a key.

use super::txt;
use crate::components::{
    Assets, BUTTON_HEIGHT, ButtonKind, FieldKind, Mark, SpendPassword, button, button_enabled,
    button_row, check_line, chip, dialog, divider, error_notice, field_label, kv_row, notice,
    paint_letter_box, password_asked, text_action, text_field,
};
use crate::context::Ctx;
use crate::text::{self, key_level, short_address};
use crate::theme::{Weight, mono, sans};
use egui::{Align, Align2, CornerRadius, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use evolite_api::{AppRequest, AppRequestKind, Command, CoreError, Spend};

#[derive(Default)]
pub struct AppRequests {
    /// The request the core decoded and the user has not answered yet.
    pub open: Option<AppRequest>,
    pub pasting: bool,
    pub link: String,
    pub link_error: Option<CoreError>,
    pub show_bytes: bool,
    /// The core's answer to "Allow" / "Sign" / "The code matches".
    pub result: Option<Result<(), CoreError>>,
    /// "Disconnect" asked about this app (its id and name), not confirmed yet; then the core's
    /// answer while the dialog is open.
    pub disconnecting: Option<(String, String)>,
    pub disconnect_waiting: bool,
    pub disconnect_result: Option<Result<(), CoreError>>,
    /// "Password for every payment" before "Allow" / "Sign": the app spends the identity's
    /// credits or adds a key to it (BUG-42).
    pub password: SpendPassword,
    /// Where the person's yes to [`Self::open`] is (BUG-30).
    pub consent: Consent,
}

/// The person's yes to a request ("Allow sign-in", "Sign", "The code matches"). The core takes
/// the request away at the first answer, so after a yes there is nothing left to answer: a
/// second yes would come back "out of date" and a "no" would not stop what is under way (BUG-30).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Consent {
    /// Not answered: both buttons work.
    #[default]
    Asking,
    /// The yes went to the core; the network has not answered yet.
    Sending,
    /// The app got the answer.
    Sent,
    /// The yes did not go through ([`AppRequests::result`] says why); the request is over. A
    /// wrong wallet password is not this: the core refused it before touching the request, so
    /// the dialog asks again (BUG-42).
    Failed,
}

/// What a click in the request dialog asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Yes,
    No,
    /// "Close" once the yes is answered, or ✕ / Esc.
    Close,
}

impl AppRequests {
    /// Takes the core's answer to a link or to the user's decision.
    pub fn on_request(&mut self, request: Result<AppRequest, CoreError>) {
        match request {
            Ok(r) => {
                self.open = Some(r);
                self.pasting = false;
                self.link.clear();
                self.link_error = None;
                self.result = None;
                self.show_bytes = false;
                self.consent = Consent::Asking;
            }
            // The link window shows why; a link from "Show an example" opens it for that.
            Err(e) => {
                self.link_error = Some(e);
                self.pasting = true;
            }
        }
    }

    /// The core's answer to "Disconnect": done closes the dialog, a refusal stays in it.
    pub fn on_disconnected(&mut self, result: Result<(), CoreError>) {
        if !self.disconnect_waiting {
            return;
        }
        self.disconnect_waiting = false;
        if result.is_ok() {
            self.disconnecting = None;
            self.disconnect_result = None;
        } else {
            self.disconnect_result = Some(result);
        }
    }

    pub fn on_answer(&mut self, result: Result<(), CoreError>) {
        self.password.on_answer(&result);
        if self.consent == Consent::Sending && result == Err(CoreError::WrongPassword) {
            // The core's gate refused the password before the request was touched: it is
            // still held, so the person retypes and says yes again. The line under the field
            // says what happened (BUG-42).
            self.consent = Consent::Asking;
            self.result = Some(result);
        } else if self.consent == Consent::Sending {
            // The yes is answered: the dialog says how it went and offers only "Close".
            self.consent = if result.is_ok() {
                Consent::Sent
            } else {
                Consent::Failed
            };
            self.result = result.err().map(Err);
        } else if result.is_ok() {
            // Declined: the request is over.
            self.open = None;
        } else {
            self.result = Some(result);
        }
    }

    /// The person's click on the open request, and the command it sends to the core. After a
    /// yes nothing goes to the core again; closing while it is sent keeps the dialog (BUG-30).
    /// `ask`: the wallet wants its password for every spend, and a yes is one (BUG-42).
    pub fn pick(&mut self, pick: Pick, ask: bool) -> Option<Command> {
        let id = self.open.as_ref()?.id;
        match (self.consent, pick) {
            (Consent::Asking, Pick::Yes) => {
                self.consent = Consent::Sending;
                self.result = None;
                Some(self.password.command(Spend::ApproveApp { id }, ask))
            }
            (Consent::Asking, Pick::No) => {
                self.result = None;
                Some(Command::DeclineApp { id })
            }
            // Closing is declining: the app hears "no".
            (Consent::Asking, Pick::Close) => {
                self.open = None;
                self.result = None;
                self.password.forget();
                Some(Command::DeclineApp { id })
            }
            (Consent::Sent | Consent::Failed, Pick::Close) => {
                self.open = None;
                self.result = None;
                self.consent = Consent::Asking;
                self.password.forget();
                None
            }
            (Consent::Sending, _) | (Consent::Sent | Consent::Failed, _) => None,
        }
    }
}

/// The app as the core names it: its owner's `@name` from Platform, or the app's own word in
/// «…» (`real/app_jobs.rs`). The word and whether it is the app's own.
fn app_word(app: &str) -> (&str, bool) {
    match app.strip_prefix('«').and_then(|w| w.strip_suffix('»')) {
        Some(word) => (word, true),
        None => (app, false),
    }
}

/// The app for the person: its own word in the quotes of the interface language (BUG-52;
/// «…» stays the core's marker).
pub fn app_label(app: &str) -> String {
    match app_word(app) {
        (word, true) => quote(word),
        (name, false) => name.to_owned(),
    }
}

/// A word in the quotes of the interface language. Translated here: it goes into other
/// sentences as a value.
fn quote(word: &str) -> String {
    crate::i18n::t(&format!(tr!("“{}”"), word)).into_owned()
}

/// The contract's name from the link, for a sentence: in the quotes of [`app_label`], with
/// control and invisible characters escaped as Debug does, so they show (BUG-52 r2 should 3).
fn quoted_name(name: &str) -> String {
    quote(&name.escape_debug().to_string())
}

fn dash(d: i64) -> String {
    crate::format::amount(d)
}

/// Draws whichever dialog is open.
pub fn show(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut AppRequests) {
    if state.pasting {
        paste_dialog(ui, a, cx, state);
    }
    let Some(request) = state.open.clone() else {
        return;
    };
    let (icon, title, width) = match request.kind {
        AppRequestKind::Connect { .. } => ("app-connect", tr!("Signing in to an app"), 482.0),
        AppRequestKind::Sign { .. } => ("sign", tr!("Sign the operation"), 502.0),
        AppRequestKind::KeyExchange { .. } => ("key", tr!("Sign in by code"), 502.0),
    };
    let ask = password_asked(cx.snap);
    // «Sign in as» is the identity of «Work as»: with more than one, say where it is chosen.
    let many = cx
        .snap
        .wallet
        .as_ref()
        .is_some_and(|w| w.identities.len() > 1);
    let mut answer = Answer {
        pick: None,
        consent: state.consent,
        password: &mut state.password,
        ask,
    };
    let closed = dialog(ui, a, "app-request", icon, title, width, |ui, a| {
        let p = a.p;
        match &request.kind {
            AppRequestKind::Connect { .. } => connect(ui, a, &request, many, &mut answer),
            AppRequestKind::Sign { .. } => sign(
                ui,
                a,
                &request,
                state.show_bytes,
                &mut state.show_bytes,
                &mut answer,
            ),
            AppRequestKind::KeyExchange { .. } => key_exchange(ui, a, &request, &mut answer),
        }
        if state.consent == Consent::Sent {
            notice(
                ui,
                a,
                "confirmed",
                p.accent,
                tr!("Done: the app got your answer."),
            );
        }
        // A wrong password is said under its field: the app got nothing.
        match state
            .result
            .as_ref()
            .filter(|_| !answer.password.is_wrong())
        {
            Some(Err(CoreError::NotAvailable(why))) => notice(
                ui,
                a,
                "info",
                p.muted,
                &format!(tr!("{why}. The app got nothing."), why = why),
            ),
            Some(Err(e)) => error_notice(
                ui,
                a,
                &text::failed(tr!("It did not work"), e),
                text::error_details(e).as_deref(),
            ),
            _ => {}
        }
        false
    });
    let pick = if closed {
        Some(Pick::Close)
    } else {
        answer.pick
    };
    if let Some(command) = pick.and_then(|pick| state.pick(pick, ask)) {
        cx.send(command);
    }
}

/// The person's click in the dialog, where the yes already is, and "password for every
/// payment" before a yes.
struct Answer<'a> {
    pick: Option<Pick>,
    consent: Consent,
    password: &'a mut SpendPassword,
    ask: bool,
}

/// Two buttons on the right: decline (ghost) and the primary answer; above them the wallet
/// password when every spend asks for it. While the yes is sent both are off and the primary
/// one says so; once it is answered only "Close" is left.
/// `asked_by_the_app`: the app asks for something the person did not start (a sign-in).
fn answers(
    ui: &mut Ui,
    a: &mut Assets,
    no: &str,
    yes: &str,
    asked_by_the_app: bool,
    answer: &mut Answer<'_>,
) {
    let p = a.p;
    let entered = answer.consent == Consent::Asking
        && answer
            .password
            .field(ui, &p, "app-request-password", answer.ask);
    let ready = answer.password.ready(answer.ask);
    // A sign-in, or an operation to sign, is asked by the site, not by the person: Enter must
    // not allow it (the designer, 2026-11-06, rule b′). The focus starts on «Decline», and
    // «Allow sign-in» or «Sign» goes only by a click, or by Tab and Enter on it.
    crate::components::button_row_destructive_if(ui, asked_by_the_app, |ui| match answer.consent {
        Consent::Asking => {
            if button(ui, a, no, ButtonKind::Ghost, None).clicked() {
                answer.pick = Some(Pick::No);
            }
            if button_enabled(ui, a, yes, ButtonKind::Primary, None, ready).clicked()
                || (entered && ready)
            {
                answer.pick = Some(Pick::Yes);
            }
        }
        Consent::Sending => {
            button_enabled(ui, a, no, ButtonKind::Ghost, None, false);
            button_enabled(ui, a, tr!("Sending…"), ButtonKind::Primary, None, false);
        }
        Consent::Sent | Consent::Failed => {
            if button(ui, a, tr!("Close"), ButtonKind::Primary, None).clicked() {
                answer.pick = Some(Pick::Close);
            }
        }
    });
}

/// 11 · Signing in to an app (App Connect).
fn connect(ui: &mut Ui, a: &mut Assets, r: &AppRequest, many: bool, answer: &mut Answer<'_>) {
    let p = a.p;
    let AppRequestKind::Connect {
        contract_id,
        contract_found,
        identity,
        key_id,
        level,
        contract_name,
        cost,
    } = &r.kind
    else {
        return;
    };
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let (logo, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                paint_letter_box(ui, &p, logo, app_word(&r.app).0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(txt(&app_label(&r.app), 15.0, Weight::SemiBold, p.text));
                    ui.label(
                        egui::RichText::new(crate::i18n::t(&format!(
                            tr!("contract {}"),
                            short_address(contract_id)
                        )))
                        .font(mono(12.0, Weight::Regular))
                        .color(p.muted),
                    )
                    .on_hover_text(contract_id.clone());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Muted, not the accent of "verified": the contract exists, which says
                    // nothing about who made the request.
                    if *contract_found {
                        chip(ui, a, "confirmed", tr!("Contract found"), false).on_hover_text(
                            crate::i18n::t(tr!("The contract is on Platform: verified by a proof"))
                                .into_owned(),
                        );
                    } else {
                        chip(ui, a, "warning", tr!("Contract not found"), false);
                    }
                });
            });
        });
    // Plain text, not a list (BUG-29, the designer 2026-11-06): the core signs in as the identity
    // of «Work as», and this row says which one — there is nothing to pick here.
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let font = if identity.starts_with('@') {
            sans(13.0, Weight::SemiBold)
        } else {
            mono(12.0, Weight::Regular)
        };
        kv_row(ui, &p, tr!("Sign in as"), identity, font);
        if many {
            ui.add(
                egui::Label::new(txt(
                    tr!("Sign-in uses the identity from “Work as”. You can change it on the Identity screen."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ))
                .wrap(),
            );
        }
    });
    // The name is the contract owner's `@name` from Platform; in quotes it is the app's own word.
    if app_word(&r.app).1 {
        // A risk the person decides on, not an error (calm states CS-1).
        crate::components::attention_notice(
            ui,
            a,
            &format!(
                tr!(
                    "The name {contract_name} is the app's own word; Platform does not confirm it. Allow only if you started the sign-in yourself."
                ),
                contract_name = quoted_name(contract_name)
            ),
        );
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        field_label(ui, &p, tr!("The app gets"));
        ui.add_space(4.0);
        check_line(ui, a, Mark::Yes, tr!("Your name and identity ID"));
        check_line(
            ui,
            a,
            Mark::Yes,
            &format!(
                tr!("Key #{key_id} · {} — by the second link"),
                key_level(*level),
                key_id = key_id
            ),
        );
        check_line(
            ui,
            a,
            Mark::Yes,
            tr!("Write documents in your name (fees come from the credits)"),
        );
        check_line(ui, a, Mark::No, tr!("Transfer credits and DASH"));
        check_line(ui, a, Mark::No, tr!("The phrase and other keys"));
    });
    divider(ui, &p);
    kv_row(
        ui,
        &p,
        tr!("Answer to the app"),
        &format!("≈ {}", dash(*cost)),
        mono(13.0, Weight::Medium),
    );
    answers(ui, a, tr!("Decline"), tr!("Allow sign-in"), true, answer);
}

/// 12 · Sign the operation: what the core decoded, not what the app says.
fn sign(
    ui: &mut Ui,
    a: &mut Assets,
    r: &AppRequest,
    showing_bytes: bool,
    show_bytes: &mut bool,
    answer: &mut Answer<'_>,
) {
    let p = a.p;
    let AppRequestKind::Sign {
        action,
        contract,
        contract_id,
        document_type,
        identity,
        key_id,
        level,
        fields,
        cost,
        credits_after,
        bytes_hex,
    } = &r.kind
    else {
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (logo, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
        paint_letter_box(ui, &p, logo, app_word(&r.app).0);
        ui.label(txt(
            &format!(tr!("{} asks you to sign one operation"), app_label(&r.app)),
            14.0,
            Weight::Medium,
            p.text,
        ));
    });
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            let value = sans(13.0, Weight::Medium);
            let code = mono(13.0, Weight::Medium);
            kv_row(ui, &p, tr!("Action"), action, value.clone());
            kv_row(
                ui,
                &p,
                tr!("Contract"),
                &format!("{contract} · {}", short_address(contract_id)),
                code.clone(),
            );
            kv_row(ui, &p, tr!("Type"), document_type, code);
            kv_row(
                ui,
                &p,
                tr!("Label"),
                &format!(
                    tr!("{identity} · key #{key_id} {}"),
                    key_level(*level),
                    identity = identity,
                    key_id = key_id
                ),
                value,
            );
            divider(ui, &p);
            for (name, value) in fields {
                ui.label(
                    egui::RichText::new(crate::i18n::t(name))
                        .font(mono(11.0, Weight::Regular))
                        .color(p.muted),
                );
                ui.add(
                    egui::Label::new(
                        txt(value, 14.0, Weight::Regular, p.text).line_height(Some(21.0)),
                    )
                    .wrap(),
                );
            }
            if showing_bytes {
                divider(ui, &p);
                ui.label(
                    egui::RichText::new(crate::i18n::t(bytes_hex))
                        .font(mono(12.0, Weight::Regular))
                        .color(p.muted),
                );
            }
        });
    let value = mono(13.0, Weight::Medium);
    kv_row(
        ui,
        &p,
        tr!("Cost"),
        &format!("≈ {}", dash(*cost)),
        value.clone(),
    );
    kv_row(
        ui,
        &p,
        tr!("Credits after"),
        &crate::format::balance(*credits_after),
        value,
    );
    notice(
        ui,
        a,
        "info",
        p.muted,
        tr!(
            "evolite decoded the operation itself and shows what it contains, not the app's description of it."
        ),
    );
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), BUTTON_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            let label = if showing_bytes {
                tr!("Hide the bytes")
            } else {
                tr!("Show the bytes")
            };
            if text_action(ui, a, "document", label).clicked() {
                *show_bytes = !showing_bytes;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Asked by the app, and it spends the identity's credits: Enter must not
                // sign it, as it must not allow a sign-in (rule b′, review r2): the focus
                // starts on «Decline», «Sign» goes by a click or Tab and Enter on it.
                answers(ui, a, tr!("Decline"), tr!("Sign"), true, answer);
            });
        },
    );
}

/// 13 · Sign in by code (`dash-kx:`): the same code on both screens.
fn key_exchange(ui: &mut Ui, a: &mut Assets, r: &AppRequest, answer: &mut Answer<'_>) {
    let p = a.p;
    let AppRequestKind::KeyExchange {
        identity,
        device,
        code,
        channel_encrypted,
        request_signed,
    } = &r.kind
    else {
        return;
    };
    ui.add(
        egui::Label::new(
            txt(
                &format!(
                    tr!("The app {device} asks for a key for {identity}. Make sure the code matches the code on its screen.")
                , device = device, identity = identity),
                14.0,
                Weight::Regular,
                p.muted,
            )
            .line_height(Some(21.0)),
        )
        .wrap(),
    );
    // Two groups of three digits, centred.
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::hover());
    let group_w = 108.0;
    let start = row.center().x - group_w - 5.0;
    for (i, group) in [&code[..3.min(code.len())], &code[3.min(code.len())..]]
        .iter()
        .enumerate()
    {
        let rect = Rect::from_min_size(
            pos2(start + i as f32 * (group_w + 10.0), row.top()),
            vec2(group_w, 66.0),
        );
        ui.painter()
            .rect_filled(rect, CornerRadius::same(12), p.raised);
        // One digit per 26 px, as in Figma: easy to compare digit by digit.
        for (k, digit) in group.chars().enumerate() {
            ui.painter().text(
                pos2(rect.center().x + (k as f32 - 1.0) * 26.0, rect.center().y),
                Align2::CENTER_CENTER,
                crate::i18n::t(&digit.to_string()),
                mono(30.0, Weight::Medium),
                p.text,
            );
        }
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mark = |ok: bool| if ok { Mark::Yes } else { Mark::No };
        check_line(
            ui,
            a,
            mark(*channel_encrypted),
            tr!("The channel is encrypted (ECDH, session key)"),
        );
        check_line(
            ui,
            a,
            mark(*request_signed),
            tr!("The request is signed by the app's identity"),
        );
        check_line(ui, a, Mark::Waiting, tr!("Waiting for your confirmation"));
    });
    notice(
        ui,
        a,
        "warning",
        p.muted,
        tr!("If the code does not match, decline: someone else is trying to get the key."),
    );
    answers(
        ui,
        a,
        tr!("The code does not match"),
        tr!("The code matches"),
        // Asked by the app as the sign-in is: «Someone else is trying to get the key».
        true,
        answer,
    );
}

/// "Disconnect": what it does and costs, before the core is asked.
pub fn disconnect_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut AppRequests) {
    let Some((id, name)) = state.disconnecting.clone() else {
        return;
    };
    let cost = cx.wallet().fees.add_key;
    let closed = dialog(
        ui,
        a,
        "app-disconnect",
        "app-connect",
        tr!("Disconnect the app"),
        462.0,
        |ui, a| {
            let p = a.p;
            let mut close = false;
            ui.add(
                egui::Label::new(
                    txt(
                        &format!(
                            tr!("The {name} keys on the identity will be disabled: the app can no longer sign with them. They cannot be turned back on — only signed in again.")
                        , name = name),
                        14.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(Some(21.0)),
                )
                .wrap(),
            );
            kv_row(
                ui,
                &p,
                tr!("Identity update"),
                &format!("≈ {}", dash(cost)),
                mono(13.0, Weight::Medium),
            );
            if let Some(Err(e)) = &state.disconnect_result {
                error_notice(
                    ui,
                    a,
                    &text::failed(tr!("Not disconnected"), e),
                    text::error_details(e).as_deref(),
                );
            }
            // The keys cannot be turned back on: «Cancel» first (the designer's rule b′).
            crate::components::button_row_destructive(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                let label = if state.disconnect_waiting {
                    tr!("Disconnecting…")
                } else {
                    tr!("Disconnect")
                };
                if button_enabled(
                    ui,
                    a,
                    label,
                    ButtonKind::Primary,
                    None,
                    !state.disconnect_waiting,
                )
                .clicked()
                {
                    cx.send(Command::DisconnectApp { id: id.clone() });
                    state.disconnect_waiting = true;
                    state.disconnect_result = None;
                }
            });
            close
        },
    );
    if closed && !state.disconnect_waiting {
        state.disconnecting = None;
        state.disconnect_result = None;
    }
}

/// "Paste a link": a `dash-key:`, `dash-st:` or key-exchange link from an app.
fn paste_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut AppRequests) {
    let closed = dialog(
        ui,
        a,
        "app-link",
        "link",
        tr!("Paste a link"),
        462.0,
        |ui, a| {
            let p = a.p;
            let mut close = false;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("Link from an app"));
                if text_field(
                    ui,
                    &p,
                    "app-link",
                    &mut state.link,
                    tr!("dash-key:… or dash-st:…"),
                    FieldKind::Text,
                    |_| {},
                )
                .changed()
                {
                    state.link_error = None;
                }
                if let Some(e) = &state.link_error {
                    let line = match e {
                        CoreError::BadLink => tr!("This is not a Dash app request").to_string(),
                        CoreError::NotAvailable(why) => why.clone(),
                        other => text::error_text(other),
                    };
                    ui.label(txt(&line, 12.0, Weight::Regular, p.danger));
                }
            });
            notice(
                ui,
                a,
                "info",
                p.muted,
                tr!("evolite decodes the link itself and shows exactly what the app asks for."),
            );
            button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                let ready = !state.link.trim().is_empty();
                if button_enabled(ui, a, tr!("Open"), ButtonKind::Primary, None, ready).clicked() {
                    cx.send(Command::OpenAppLink {
                        text: state.link.clone(),
                    });
                }
            });
            close
        },
    );
    if closed {
        state.pasting = false;
        state.link.clear();
        state.link_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::KeyLevel;

    fn asked(id: u64) -> AppRequests {
        let mut state = AppRequests::default();
        state.on_request(Ok(AppRequest {
            id,
            app: "yappr".into(),
            kind: AppRequestKind::Connect {
                contract_id: "c".into(),
                contract_found: true,
                identity: "@me".into(),
                key_id: 3,
                level: KeyLevel::High,
                contract_name: "yappr".into(),
                cost: 0,
            },
        }));
        state
    }

    fn out_of_date() -> CoreError {
        CoreError::NotAvailable("The request is out of date: open the app's link again".into())
    }

    fn is_yes(command: Option<Command>, want: u64) -> bool {
        matches!(
            command,
            Some(Command::Spend {
                op: Spend::ApproveApp { id },
                ..
            }) if id == want
        )
    }

    fn is_no(command: Option<Command>, want: u64) -> bool {
        matches!(command, Some(Command::DeclineApp { id }) if id == want)
    }

    /// BUG-30: after a yes, neither a second yes nor "Decline" nor ✕ goes to the core, and the
    /// dialog stays until the network answers.
    #[test]
    fn after_a_yes_nothing_more_goes_to_the_core() {
        let mut state = asked(7);
        assert!(is_yes(state.pick(Pick::Yes, false), 7));
        assert_eq!(state.consent, Consent::Sending);
        for pick in [Pick::Yes, Pick::No, Pick::Close] {
            assert!(state.pick(pick, false).is_none(), "{pick:?} while sent");
        }
        assert!(state.open.is_some(), "the dialog waits for the network");
    }

    /// BUG-30: the network's yes is shown as done, with "Close" only; closing sends nothing.
    #[test]
    fn a_yes_that_went_through_is_shown_as_done() {
        let mut state = asked(7);
        state.pick(Pick::Yes, false);
        state.on_answer(Ok(()));
        assert_eq!(state.consent, Consent::Sent);
        assert!(state.open.is_some(), "the dialog says it is done");
        assert!(state.pick(Pick::No, false).is_none());
        assert!(state.pick(Pick::Close, false).is_none());
        assert!(state.open.is_none());
    }

    /// BUG-30: a yes that failed says why and is over: no second try, closing sends no "no".
    #[test]
    fn a_failed_yes_says_why_and_is_over() {
        let mut state = asked(7);
        state.pick(Pick::Yes, false);
        state.on_answer(Err(out_of_date()));
        assert_eq!(state.consent, Consent::Failed);
        assert_eq!(state.result, Some(Err(out_of_date())));
        assert!(state.pick(Pick::Yes, false).is_none());
        assert!(state.pick(Pick::Close, false).is_none());
        assert!(state.open.is_none());
        assert_eq!(state.result, None);
    }

    /// BUG-30 with BUG-42: a wrong wallet password on "Allow" ends nothing. The core's gate
    /// refused it before the request was touched, so the dialog asks again: the line under the
    /// field says why, and the next yes goes to the core with the retyped password.
    #[test]
    fn a_wrong_password_on_yes_asks_again() {
        let mut state = asked(7);
        assert!(is_yes(state.pick(Pick::Yes, true), 7));
        state.on_answer(Err(CoreError::WrongPassword));
        assert_eq!(state.consent, Consent::Asking, "the request is still held");
        assert!(state.open.is_some());
        assert!(state.password.is_wrong(), "said under the field");
        assert_eq!(state.result, Some(Err(CoreError::WrongPassword)));
        assert!(is_yes(state.pick(Pick::Yes, true), 7), "a second try goes");
        assert_eq!(state.consent, Consent::Sending);
        state.on_answer(Ok(()));
        assert_eq!(state.consent, Consent::Sent);
        assert!(!state.password.is_wrong());
    }

    /// Before any yes, "Decline" and ✕ still say "no" to the app; a new request asks again.
    #[test]
    fn before_a_yes_declining_still_says_no() {
        let mut state = asked(7);
        assert!(is_no(state.pick(Pick::No, false), 7));
        state.on_answer(Ok(()));
        assert!(state.open.is_none(), "declined: the request is over");
        let mut state = asked(8);
        state.pick(Pick::Yes, false);
        state.on_answer(Ok(()));
        state.on_request(asked(9).open.ok_or_else(out_of_date));
        assert_eq!(state.consent, Consent::Asking, "a new request asks again");
        assert!(is_no(state.pick(Pick::Close, false), 9));
    }

    /// BUG-52, review should: the app's own word, which the core marks «…», is quoted as the
    /// interface language quotes; an `@name` from Platform is shown as it is.
    #[test]
    fn an_apps_own_word_is_quoted_in_the_interface_language() {
        use crate::i18n::{Lang, set};
        set(Lang::En);
        assert_eq!(app_label("«yappr»"), "“yappr”");
        assert_eq!(app_label("@yappr"), "@yappr");
        set(Lang::Ru);
        assert_eq!(app_label("«yappr»"), "«yappr»");
        set(Lang::En);
    }

    /// BUG-52 r2 should 3: the contract's name in the warning takes the same quotes as the
    /// app's label and keeps Debug's escaping, so a line break or a direction mark shows.
    #[test]
    fn the_contract_name_in_the_warning_is_quoted_and_escaped() {
        use crate::i18n::{Lang, set};
        set(Lang::En);
        assert_eq!(quoted_name("yappr"), "“yappr”");
        assert_eq!(quoted_name("a\nb\u{202e}"), "“a\\nb\\u{202e}”");
        set(Lang::De);
        assert_eq!(quoted_name("yappr"), "„yappr“");
        set(Lang::En);
    }
}
