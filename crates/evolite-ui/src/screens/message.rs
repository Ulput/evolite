//! FEAT-0044: «Message» — sign with an address of this wallet, or check someone's signature, in
//! dash-qt's format. Opened from «Receive» and from «Settings › Advanced › Tools», behind the
//! advanced switch. Signing asks the wallet's password every time (R-13); checking needs neither
//! the password nor the wallet (`evolite_chain::message::verify_message`, called here). The lock
//! closes it and its texts go (R-10). The person's text is shown as it is (`i18n::own`, R-15).

use crate::components::{
    Assets, ButtonKind, FieldKind, button, button_enabled, button_row, dialog, field_label, notice,
    para, text_field, txt,
};
use crate::context::Ctx;
use crate::theme::Weight;
use egui::Ui;
use evolite_api::{Command, CoreError, Password};
use zeroize::Zeroizing;

/// R-9: the most characters a message may have.
const MESSAGE_MAX: usize = 10_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Sign,
    Check,
}

#[derive(Clone)]
struct State {
    tab: Tab,
    address: String,
    message: String,
    password: Zeroizing<String>,
    waiting: bool,
    signed: Option<Result<String, String>>,
    check_address: String,
    check_message: String,
    check_signature: String,
}

fn state_id() -> egui::Id {
    egui::Id::new("message-dialog")
}

fn get(ctx: &egui::Context) -> Option<State> {
    ctx.data(|d| d.get_temp::<Option<State>>(state_id()))
        .flatten()
}

fn put(ctx: &egui::Context, state: Option<State>) {
    ctx.data_mut(|d| d.insert_temp(state_id(), state));
}

/// Opens the window on `tab`, signing with `address` by default (the current receiving one).
pub fn open(ctx: &egui::Context, tab: Tab, address: &str) {
    put(
        ctx,
        Some(State {
            tab,
            address: address.to_owned(),
            message: String::new(),
            password: Zeroizing::new(String::new()),
            waiting: false,
            signed: None,
            check_address: String::new(),
            check_message: String::new(),
            check_signature: String::new(),
        }),
    );
}

/// R-10: the lock closes it; the texts and the signature go.
pub fn close(ctx: &egui::Context) {
    put(ctx, None);
}

/// Whether the window is open (tests).
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub fn is_open(ctx: &egui::Context) -> bool {
    get(ctx).is_some()
}

/// Tests: the texts of a tab, as if typed (`None` leaves one as it is).
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub fn type_in(
    ctx: &egui::Context,
    tab: Tab,
    (address, message, third): (Option<&str>, Option<&str>, Option<&str>),
) {
    if let Some(mut state) = get(ctx) {
        state.tab = tab;
        // As typing does: the last answer goes.
        if tab == Tab::Sign {
            state.signed = None;
        }
        let (a, m, t) = match tab {
            Tab::Sign => (&mut state.address, &mut state.message, &mut *state.password),
            Tab::Check => (
                &mut state.check_address,
                &mut state.check_message,
                &mut state.check_signature,
            ),
        };
        for (field, value) in [(a, address), (m, message), (t, third)] {
            if let Some(value) = value {
                *field = value.to_owned();
            }
        }
        put(ctx, Some(state));
    }
}

/// The core's answer to `SignMessage`.
pub fn on_signed(ctx: &egui::Context, result: &Result<String, CoreError>) {
    if let Some(mut state) = get(ctx) {
        state.waiting = false;
        state.signed = Some(match result {
            Ok(signature) => {
                state.password = Zeroizing::new(String::new());
                Ok(signature.clone())
            }
            Err(CoreError::WrongPassword) => Err(tr!("Wrong password").into()),
            Err(CoreError::NotAvailable(why)) => Err(why.clone()),
            Err(e) => Err(crate::text::error_text(e)),
        });
        put(ctx, Some(state));
    }
}

/// The window, while it is open.
pub fn window(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let Some(mut state) = get(ui.ctx()) else {
        return;
    };
    let p = a.p;
    let mut send = None;
    let closed = dialog(
        ui,
        a,
        "message-dialog",
        "edit",
        tr!("Message"),
        520.0,
        |ui, a| {
            let mut close = false;
            ui.horizontal(|ui| {
                for (tab, label) in [(Tab::Sign, tr!("Sign")), (Tab::Check, tr!("Check"))] {
                    let kind = if state.tab == tab {
                        ButtonKind::Secondary
                    } else {
                        ButtonKind::Ghost
                    };
                    if button(ui, a, label, kind, None).clicked() {
                        state.tab = tab;
                    }
                }
            });
            match state.tab {
                Tab::Sign => sign_tab(ui, a, cx, &mut state, &mut send, &mut close),
                Tab::Check => check_tab(ui, a, &mut state, &mut close),
            }
            close
        },
    );
    if let Some(command) = send {
        cx.send(command);
    }
    if closed {
        close(ui.ctx());
    } else {
        let _ = p;
        put(ui.ctx(), Some(state));
    }
}

fn limited(text: &mut String) {
    if text.chars().count() > MESSAGE_MAX {
        *text = text.chars().take(MESSAGE_MAX).collect();
    }
}

fn sign_tab(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut State,
    send: &mut Option<Command>,
    close: &mut bool,
) {
    let p = a.p;
    // R-5: before anything is typed.
    notice(ui, a, "warning", p.accent, WARNING);
    field_label(ui, &p, tr!("Address"));
    if text_field(
        ui,
        &p,
        "message-address",
        &mut state.address,
        "",
        FieldKind::Text,
        |_| {},
    )
    .changed()
    {
        state.signed = None;
    }
    field_label(ui, &p, tr!("Message"));
    let message = ui.add(
        egui::TextEdit::multiline(&mut state.message)
            .desired_rows(4)
            .desired_width(f32::INFINITY),
    );
    if message.changed() {
        limited(&mut state.message);
        state.signed = None;
    }
    if state.message.chars().count() >= MESSAGE_MAX {
        ui.label(txt(
            &crate::i18n::t(tr!("At most 10,000 characters")),
            12.0,
            Weight::Regular,
            p.muted,
        ));
    }
    match &state.signed {
        Some(Ok(signature)) => {
            field_label(ui, &p, tr!("Signature"));
            ui.add(egui::Label::new(txt(signature, 13.0, Weight::Medium, p.text)).wrap());
            let signature = signature.clone();
            button_row(ui, |ui| {
                if button(ui, a, tr!("Done"), ButtonKind::Ghost, None).clicked() {
                    *close = true;
                }
                if button(ui, a, tr!("Copy the signature"), ButtonKind::Primary, None).clicked() {
                    // R-14: not a secret — as an address is copied.
                    crate::clipboard::copy(ui.ctx(), signature.clone());
                }
            });
        }
        answer => {
            field_label(ui, &p, tr!("Wallet password"));
            text_field(
                ui,
                &p,
                "message-password",
                &mut state.password,
                "",
                FieldKind::Password,
                |_| {},
            );
            if let Some(Err(why)) = answer {
                ui.label(txt(&crate::i18n::t(why), 12.0, Weight::Regular, p.danger));
            }
            button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    *close = true;
                }
                // An empty message proves nothing: not signed (the card's edge case).
                let ready = !state.waiting
                    && !state.message.is_empty()
                    && !state.address.trim().is_empty()
                    && !state.password.is_empty();
                if button_enabled(ui, a, tr!("Sign"), ButtonKind::Primary, None, ready).clicked() {
                    *send = Some(Command::SignMessage {
                        address: state.address.trim().to_owned(),
                        message: state.message.clone(),
                        password: Password::new(state.password.to_string()),
                    });
                    state.waiting = true;
                    state.signed = None;
                }
            });
        }
    }
    let _ = cx;
}

fn check_tab(ui: &mut Ui, a: &mut Assets, state: &mut State, close: &mut bool) {
    let p = a.p;
    field_label(ui, &p, tr!("Address"));
    text_field(
        ui,
        &p,
        "check-address",
        &mut state.check_address,
        "",
        FieldKind::Text,
        |_| {},
    );
    field_label(ui, &p, tr!("Message"));
    if ui
        .add(
            egui::TextEdit::multiline(&mut state.check_message)
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        )
        .changed()
    {
        limited(&mut state.check_message);
    }
    field_label(ui, &p, tr!("Signature"));
    text_field(
        ui,
        &p,
        "check-signature",
        &mut state.check_signature,
        "",
        FieldKind::Text,
        |_| {},
    );
    // UC-3: the answer as it is typed, no button, no password.
    if !state.check_address.trim().is_empty() && !state.check_signature.trim().is_empty() {
        let (line, color) = said(
            &state.check_address,
            &state.check_message,
            &state.check_signature,
        );
        ui.add(para(
            &line,
            13.0,
            Weight::Medium,
            if color { p.accent } else { p.danger },
        ));
        if !color {
            ui.add(para(
                &crate::i18n::t(tr!(
                    "The message must match to the character, spaces and line breaks included"
                )),
                12.0,
                Weight::Regular,
                p.muted,
            ));
        }
    }
    button_row(ui, |ui| {
        if button(ui, a, tr!("Done"), ButtonKind::Ghost, None).clicked() {
            *close = true;
        }
    });
}

/// R-3, R-12: the check's answer in words, and whether it is the good one.
fn said(address: &str, message: &str, signature: &str) -> (String, bool) {
    use evolite_core::message::{Verified, VerifyError, verify};
    let t = |s: &str| crate::i18n::t(s).into_owned();
    match verify(address, message, signature) {
        Ok(Verified::Valid) => (
            t(tr!(
                "The signature is valid: the message was signed with this address's key"
            )),
            true,
        ),
        Ok(Verified::NotThisAddress) => {
            (t(tr!("The signature does not match this address")), false)
        }
        Ok(Verified::NotASignature) => (t(tr!("This is not a signature")), false),
        Err(VerifyError::NotAnAddress) => (t(tr!("This is not an address")), false),
        Err(VerifyError::OtherNetwork(network)) => (
            t(tr!("This is an address of another network ({})")).replacen("{}", &network, 1),
            false,
        ),
        Err(VerifyError::NotP2pkh) => {
            let start = match evolite_core::network::current() {
                evolite_api::Network::Mainnet => "X",
                evolite_api::Network::Testnet => "y",
            };
            (
                t(tr!("Such an address has no message signatures: an address starting with {} is needed"))
                    .replacen("{}", start, 1),
                false,
            )
        }
    }
}

/// R-5: what a signature means, on «Sign».
pub const WARNING: &str = tr!(
    "Sign only text you understand. The signature proves the address is yours, and anyone can be shown it."
);
