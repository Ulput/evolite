//! "Open "…"" (Figma 29): another wallet of this computer asked for from the menu or
//! Settings, by its own password, over the screen. The open one is locked only when the
//! password is right (the core checks it first).

use super::{pause, txt};
use crate::components::{
    Assets, ButtonKind, FieldKind, button, button_enabled, button_row, dialog_with_header,
    field_label, notice, paint_wallet_dot, text_field,
};
use crate::context::Ctx;
use crate::i18n::own;
use crate::theme::Weight;
use egui::{Rect, Sense, Ui, vec2};
use evolite_api::{Command, CoreError, Password};
use zeroize::{Zeroize, Zeroizing};

#[derive(Default)]
pub struct SwitchForm {
    /// The wallet to open.
    pub id: String,
    pub password: Zeroizing<String>,
    pub waiting: bool,
    pub error: Option<String>,
    /// The core refused the password unchecked: the wallet's pause ends then (FEAT-0002).
    pub refused: Option<evolite_api::Timestamp>,
    /// The end counted down to, cut down if the clock is set back meanwhile (AC-T3).
    pub cut: pause::Cut,
}

impl SwitchForm {
    pub fn new(id: String) -> Self {
        Self {
            id,
            ..Self::default()
        }
    }

    /// The core's answer to `SwitchWallet` with the password; `true` when it is open.
    pub fn on_done(&mut self, result: &Result<(), CoreError>) -> bool {
        self.waiting = false;
        match result {
            Ok(()) => true,
            Err(CoreError::WrongPassword) => {
                self.error = Some(tr!("Wrong password").into());
                false
            }
            // The pause's plate says it.
            Err(CoreError::TooManyTries { until }) => {
                self.error = None;
                self.refused = Some(*until);
                false
            }
            Err(e) => {
                self.error = Some(crate::text::error_text(e));
                false
            }
        }
    }
}

/// Shows the dialog; returns `true` when it should close (cancelled, or the wallet is gone).
pub fn switch_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut SwitchForm) -> bool {
    let snap = cx.snap;
    let Some(target) = snap.wallets.iter().find(|w| w.id == form.id) else {
        return true;
    };
    let current = snap.wallets.iter().find(|w| w.open);
    let closed = dialog_with_header(
        ui,
        a,
        "switch-wallet",
        460.0,
        |ui, a| {
            let p = a.p;
            let (boxr, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
            ui.painter()
                .rect_filled(boxr, egui::CornerRadius::same(14), p.raised);
            paint_wallet_dot(
                ui,
                Rect::from_center_size(boxr.center(), vec2(32.0, 32.0)),
                target,
            );
            ui.label(txt(
                &format!(tr!("Open “{}”"), own(&target.name)),
                20.0,
                Weight::Bold,
                p.text,
            ));
        },
        |ui, a| {
            let p = a.p;
            let mut close = false;
            let lead = match current {
                Some(current) => format!(
                    tr!(
                        "“{}” will be locked: its keys leave memory and its sync stops. “{}” continues from where it stopped last time."
                    ),
                    own(&current.name),
                    own(&target.name)
                ),
                None => format!(
                    tr!("“{}” continues from where it stopped last time."),
                    own(&target.name)
                ),
            };
            ui.add(
                crate::components::para(&lead, 13.0, Weight::Regular, p.muted).line_height(20.0),
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, &format!(tr!("Password of “{}”"), own(&target.name)));
                let field = text_field(
                    ui,
                    &p,
                    "switch-password",
                    &mut form.password,
                    "",
                    FieldKind::Password,
                    |_| {},
                );
                if field.changed() {
                    form.error = None;
                }
                if !form.waiting && !field.has_focus() && form.password.is_empty() {
                    field.request_focus();
                }
                // The wallet's pause (FEAT-0002): its plate in place of the catching-up note.
                let pause_end = form.cut.end(
                    pause::until(target, form.refused),
                    target.wrong_tries,
                    cx.now.timestamp(),
                );
                let paused = pause::left(pause_end, cx.now.timestamp());
                if paused.is_some() {
                    form.error = None;
                }
                super::unlock::pause_lines(
                    ui,
                    a,
                    form.error.as_deref(),
                    target.wrong_tries,
                    None,
                    None,
                );
                // The line kept for the error is the gap to the note, about 23 px, not added to
                // a gap of its own (designer, FEAT-0034 item 8 r1).
                ui.add_space(-4.0);
                match paused {
                    Some(secs) => {
                        pause::plate(ui, a, secs, pause_end.unwrap_or_default());
                        pause::tick(ui, cx.now);
                    }
                    None => notice(
                        ui,
                        a,
                        "pending",
                        p.muted,
                        tr!("After opening, about a minute to catch up: new blocks since it was last open."),
                    ),
                }
                ui.add_space(10.0);
                let ready = !form.password.is_empty() && !form.waiting && paused.is_none();
                button_row(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let label = if form.waiting { tr!("Opening…") } else { tr!("Open") };
                    let mut open = button_enabled(ui, a, label, ButtonKind::Primary, None, ready);
                    if let Some(secs) = paused {
                        open = pause::explain(ui, open, secs);
                    }
                    let open = open.clicked();
                    if ready && open {
                        // The password goes to the core and is wiped here at once.
                        cx.send(Command::SwitchWallet {
                            id: form.id.clone(),
                            password: Some(Password::new(form.password.to_string())),
                        });
                        form.password.zeroize();
                        form.waiting = true;
                    }
                });
            });
            close
        },
    );
    if closed {
        form.password.zeroize();
    }
    closed
}
