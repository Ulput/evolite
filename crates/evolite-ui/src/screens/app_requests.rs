//! 11–13 · Requests from apps, decoded by the core and shown as they will be signed: sign-in
//! with App Connect, signing one operation, and the key exchange over QR. Plus the dialog to
//! paste a link. The answer goes back to the core; nothing here holds a key.

use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, Mark, button, button_enabled, check_line, chip, dialog, divider,
    error_notice, field_label, kv_row, notice, paint_letter_box, select_row, text_action,
    text_field,
};
use crate::context::Ctx;
use crate::text::{self, key_level, short_address};
use crate::theme::{Weight, mono, sans};
use egui::{Align, Align2, CornerRadius, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use evolite_api::{AppRequest, AppRequestKind, Command, CoreError};

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
        if result.is_ok() {
            // Declined (or done): the request is over.
            self.open = None;
        } else {
            self.result = Some(result);
        }
    }
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
    let mut answer = None;
    let closed = dialog(ui, a, "app-request", icon, title, width, |ui, a| {
        let p = a.p;
        match &request.kind {
            AppRequestKind::Connect { .. } => connect(ui, a, &request, &mut answer),
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
        match &state.result {
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
    if let Some(approve) = answer {
        cx.send(Command::AnswerApp {
            id: request.id,
            approve,
        });
        state.result = None;
    }
    if closed {
        // Closing is declining: the app hears "no".
        cx.send(Command::AnswerApp {
            id: request.id,
            approve: false,
        });
        state.open = None;
        state.result = None;
    }
}

/// Two buttons on the right: decline (ghost) and the primary answer.
fn answers(ui: &mut Ui, a: &mut Assets, no: &str, yes: &str, answer: &mut Option<bool>) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if button(ui, a, yes, ButtonKind::Primary, None).clicked() {
            *answer = Some(true);
        }
        if button(ui, a, no, ButtonKind::Ghost, None).clicked() {
            *answer = Some(false);
        }
    });
}

/// 11 · Signing in to an app (App Connect).
fn connect(ui: &mut Ui, a: &mut Assets, r: &AppRequest, answer: &mut Option<bool>) {
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
                paint_letter_box(ui, &p, logo, &r.app);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(txt(&r.app, 15.0, Weight::SemiBold, p.text));
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
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        field_label(ui, &p, tr!("Sign in as"));
        select_row(ui, a, "identity", identity, None);
    });
    // The name is the contract owner's `@name` from Platform; in quotes it is the app's own word.
    if r.app.starts_with('«') {
        notice(
            ui,
            a,
            "warning",
            p.danger,
            &format!(
                tr!(
                    "The name {contract_name:?} is the app's own word; Platform does not confirm it. Allow only if you started the sign-in yourself."
                ),
                contract_name = contract_name
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
    answers(ui, a, tr!("Decline"), tr!("Allow sign-in"), answer);
}

/// 12 · Sign the operation: what the core decoded, not what the app says.
fn sign(
    ui: &mut Ui,
    a: &mut Assets,
    r: &AppRequest,
    showing_bytes: bool,
    show_bytes: &mut bool,
    answer: &mut Option<bool>,
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
        paint_letter_box(ui, &p, logo, &r.app);
        ui.label(txt(
            &format!(tr!("{} asks you to sign one operation"), r.app),
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
    kv_row(ui, &p, tr!("Credits after"), &dash(*credits_after), value);
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
        vec2(ui.available_width(), 41.0),
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
                answers(ui, a, tr!("Decline"), tr!("Sign"), answer);
            });
        },
    );
}

/// 13 · Sign in by code (`dash-kx:`): the same code on both screens.
fn key_exchange(ui: &mut Ui, a: &mut Assets, r: &AppRequest, answer: &mut Option<bool>) {
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
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
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
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
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
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let ready = !state.link.trim().is_empty();
                if button_enabled(ui, a, tr!("Open"), ButtonKind::Primary, None, ready).clicked() {
                    cx.send(Command::OpenAppLink {
                        text: state.link.clone(),
                    });
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
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
