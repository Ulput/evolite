//! "Open "…"" (Figma 29): another wallet of this computer asked for from the menu or
//! Settings, by its own password, over the screen. The open one is locked only when the
//! password is right (the core checks it first).

use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, button, button_enabled, dialog_with_header, field_label, notice,
    paint_wallet_dot, text_field,
};
use crate::context::Ctx;
use crate::i18n::own;
use crate::theme::Weight;
use egui::{Align, Layout, Rect, Sense, Ui, vec2};
use evolite_api::{Command, CoreError, Password};
use zeroize::{Zeroize, Zeroizing};

#[derive(Default)]
pub struct SwitchForm {
    /// The wallet to open.
    pub id: String,
    pub password: Zeroizing<String>,
    pub waiting: bool,
    pub error: Option<String>,
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
                egui::Label::new(
                    txt(&lead, 13.0, Weight::Regular, p.muted).line_height(Some(20.0)),
                )
                .wrap(),
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
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if let Some(error) = &form.error {
                    ui.label(txt(error, 12.0, Weight::Regular, p.danger));
                }
                ui.add_space(10.0);
                notice(
                    ui,
                    a,
                    "pending",
                    p.muted,
                    tr!("After opening, about a minute to catch up: new blocks since it was last open."),
                );
                ui.add_space(10.0);
                let ready = !form.password.is_empty() && !form.waiting;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let label = if form.waiting { tr!("Opening…") } else { tr!("Open") };
                    let open =
                        button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked();
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    if ready && (open || entered) {
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
