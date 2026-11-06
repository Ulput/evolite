//! "Sync again" (Figma "Flows"): what goes and what stays, how long it takes,
//! and that evolite starts again for it. The core clears at the next start, before anything
//! opens; the app then starts itself again.

use crate::components::{Assets, ButtonKind, button, button_enabled, button_row, dialog, kv_row};
use crate::context::Ctx;
use crate::screens::txt;
use crate::text;
use crate::theme::{Weight, sans};
use egui::Ui;
use evolite_api::{Command, CoreError};

#[derive(Default)]
pub struct ResyncForm {
    waiting: bool,
    error: Option<CoreError>,
}

impl ResyncForm {
    /// The core's answer; `true` when the mark is left and the app should start again.
    pub fn on_done(&mut self, result: &Result<(), CoreError>) -> bool {
        self.waiting = false;
        self.error = result.clone().err();
        result.is_ok()
    }
}

/// Shows the dialog; returns `true` when it should close (cancelled).
pub(super) fn resync_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut ResyncForm,
) -> bool {
    let p = a.p;
    dialog(
        ui,
        a,
        "resync",
        "sync",
        tr!("Sync again"),
        470.0,
        |ui, a| {
            let mut close = false;
            ui.add(
            egui::Label::new(txt(
                tr!("evolite downloads the headers and block filters again and finds the coins and history from the wallet's creation date. Keys, identities, names and contacts are not touched."),
                13.0,
                Weight::Regular,
                p.text,
            ))
            .wrap(),
        );
            let value = sans(13.0, Weight::Medium);
            // What is read again and what stays (BUG-58, decision B: Platform too).
            kv_row(
                ui,
                &p,
                tr!("Read again"),
                tr!("Core and Platform: coins, names, contacts, profile, withdrawals, pool"),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Stays as it is"),
                tr!("notes, the address book, settings: nothing is erased"),
                value.clone(),
            );
            kv_row(ui, &p, tr!("Takes"), tr!("10–20 minutes"), value.clone());
            kv_row(
                ui,
                &p,
                tr!("Sending"),
                tr!("unavailable until it ends"),
                value,
            );
            ui.add(
                egui::Label::new(txt(
                    tr!("evolite restarts for this; enter your password after the restart."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ))
                .wrap(),
            );
            if let Some(error) = &form.error {
                ui.add(
                    egui::Label::new(txt(
                        &text::error_text(error),
                        12.0,
                        Weight::Regular,
                        p.danger,
                    ))
                    .wrap(),
                );
            }
            button_row(ui, |ui| {
                let label = if form.waiting {
                    tr!("Preparing…")
                } else {
                    tr!("Sync again")
                };
                if button_enabled(ui, a, label, ButtonKind::Primary, None, !form.waiting).clicked()
                {
                    cx.send(Command::Resync);
                    form.waiting = true;
                    form.error = None;
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close
        },
    ) && !form.waiting
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_again_only_when_the_core_left_the_mark() {
        let mut form = ResyncForm::default();
        assert!(!form.on_done(&Err(CoreError::NotAvailable("busy".into()))));
        assert!(form.error.is_some());
        assert!(form.on_done(&Ok(())));
        assert!(form.error.is_none());
    }
}
