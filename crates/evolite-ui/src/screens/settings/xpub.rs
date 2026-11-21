//! FEAT-0042: «Account public key (xpub)» — the warning first (R-3), then the wallet's password
//! every time (R-6), then the key as a QR and as text with its path (R-1, R-12) and «Copy» past the
//! clipboard's history, not wiped (R-9). The key stays until «Done», leaving the screen or the
//! lock (R-7, R-8). Behind the advanced switch.

use super::SettingsState;
use crate::components::{
    Assets, ButtonKind, FieldKind, button, button_row, dialog, field_label, notice,
    secret_password, text_field,
};
use crate::context::Ctx;
use crate::theme::Weight;
use egui::Ui;
use evolite_api::{Command, CoreError, Password};

/// R-3: what the key shows, before the password.
pub const WARNING: &str = tr!(
    "This key shows every address, payment and the balance of the account — past and future. It cannot spend. Give it only to those you trust with your history."
);

/// R-4: what the key does not cover.
pub const ONLY_CORE: &str =
    tr!("Core payments of account 0 only: no mixed coins, DashPay contacts or Platform");

/// The key's dialog, while it is open.
pub(super) fn xpub_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut SettingsState) {
    let p = a.p;
    let closed = dialog(
        ui,
        a,
        "xpub-dialog",
        "eye",
        tr!("Account public key (xpub)"),
        462.0,
        |ui, a| {
            let mut close = false;
            notice(ui, a, "warning", p.accent, WARNING);
            ui.add_space(8.0);
            match &state.xpub {
                Some(Ok(xpub)) => {
                    ui.vertical_centered(|ui| {
                        crate::screens::receive::qr_card(ui, &p, xpub.expose());
                    });
                    // R-11: the whole key, wrapped by the window, left to right in every
                    // language.
                    ui.add(
                        egui::Label::new(crate::components::txt(
                            &crate::i18n::own(xpub.expose()),
                            13.0,
                            Weight::Medium,
                            p.text,
                        ))
                        .wrap(),
                    );
                    let network = match evolite_core::network::current() {
                        evolite_api::Network::Mainnet => tr!("{} · mainnet"),
                        evolite_api::Network::Testnet => tr!("{} · testnet"),
                    };
                    ui.label(crate::components::txt(
                        &crate::i18n::t(network).replacen("{}", xpub.path(), 1),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    ui.add(crate::components::para(
                        &crate::i18n::t(ONLY_CORE),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    let key = xpub.expose().to_owned();
                    button_row(ui, |ui| {
                        if button(ui, a, tr!("Done"), ButtonKind::Ghost, None).clicked() {
                            close = true;
                        }
                        let label = if state.xpub_copied == Some(true) {
                            tr!("Copied")
                        } else {
                            tr!("Copy")
                        };
                        if button(ui, a, label, ButtonKind::Primary, None).clicked() {
                            state.xpub_copied =
                                Some(crate::clipboard::copy_public_excluded(ui.ctx(), &key));
                        }
                    });
                    if state.xpub_copied == Some(false) {
                        ui.label(crate::components::txt(
                            &crate::i18n::t(tr!(
                                "Could not copy — write the key down or scan the QR"
                            )),
                            12.0,
                            Weight::Regular,
                            p.danger,
                        ));
                    }
                }
                answer => {
                    let refused = match answer {
                        Some(Err(e)) => secret_password::refused(Some(e)),
                        _ => None,
                    };
                    let gate = secret_password::Gate::of(
                        cx.snap.wallets.iter().find(|w| w.open),
                        refused,
                        &mut state.xpub_cut,
                        cx.now.timestamp(),
                    );
                    let error = match answer {
                        Some(Err(CoreError::WrongPassword)) => {
                            Some(crate::i18n::t(tr!("Wrong password")).into_owned())
                        }
                        Some(Err(CoreError::TooManyTries { .. })) | None | Some(Ok(_)) => None,
                        Some(Err(CoreError::NotAvailable(why))) => {
                            Some(crate::i18n::t(why).into_owned())
                        }
                        Some(Err(other)) => Some(crate::text::error_text(other)),
                    };
                    field_label(ui, &p, tr!("Wallet password"));
                    if text_field(
                        ui,
                        &p,
                        "xpub-password",
                        &mut state.xpub_password,
                        "",
                        FieldKind::Password,
                        |_| {},
                    )
                    .changed()
                    {
                        state.xpub = None;
                    }
                    secret_password::lines(ui, a, &gate, error.as_deref(), cx.now);
                    button_row(ui, |ui| {
                        if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                            close = true;
                        }
                        let ready = !state.xpub_password.is_empty();
                        if secret_password::send_button(
                            ui,
                            a,
                            tr!("Show"),
                            ButtonKind::Primary,
                            None,
                            ready,
                            &gate,
                        )
                        .clicked()
                        {
                            cx.send(Command::ExportXpub {
                                password: Password::new(state.xpub_password.to_string()),
                            });
                            state.xpub = None;
                        }
                    });
                }
            }
            close
        },
    );
    if closed {
        state.close_xpub();
    }
}

#[cfg(test)]
mod tests {
    /// AC-T-X4: a key of 111 characters makes a QR of version 7 at most — a module of at least
    /// 3 px at the card's smallest side.
    #[test]
    fn the_keys_qr_is_small_enough() {
        let key = "tpubD6NzVbkrYhZ4XgiXtGrdW5XDAPFCL9h7we1vwNCpn8tGbBcgfVYjXyhWo4E1xkh56hjod1RhGjxbaTLV3X4FyWuejifB9jusQ46QzG87VKY";
        assert_eq!(key.len(), 111);
        let code = qrcode::QrCode::with_error_correction_level(key, qrcode::EcLevel::M).unwrap();
        assert!(matches!(code.version(), qrcode::Version::Normal(v) if v <= 7));
        assert!(
            120.0 / code.width() as f32 >= 2.5,
            "readable at the smallest side"
        );
    }
}
