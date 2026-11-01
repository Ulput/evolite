//! Security: password change, auto-lock, a password for every send, privacy toggles,
//! "Lock now". The auto-lock runs in the core (it holds the keys); this screen only
//! tells it how long to wait.

use super::parts::{Control, group, heading};
use super::{PasswordChange, SettingsState};
use crate::components::{
    Assets, ButtonKind, FieldKind, button, button_enabled, dialog, field_label, notice, text_field,
};
use crate::context::Ctx;
use crate::screens::onboarding::{missing, missing_line, strength};
use crate::screens::txt;
use crate::theme::Weight;
use egui::{Align, Layout, Ui};
use evolite_api::{Command, CoreError, Password};
use zeroize::Zeroize;

pub(super) fn security(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut SettingsState) {
    let p = a.p;
    heading(
        ui,
        &p,
        tr!("Security"),
        Some(tr!(
            "The keys are kept on this computer, encrypted with your password."
        )),
    );
    let mut each = *state.toggle("password-each-send");
    let mut lock_after = state.lock_after;
    let lock_note = [
        tr!("After 5 minutes without activity"),
        tr!("After 15 minutes without activity"),
        tr!("After an hour without activity"),
        tr!("When the window is closed"),
    ][lock_after];
    let clicked = group(
        ui,
        a,
        tr!("Access"),
        vec![
            (
                tr!("Password"),
                tr!("Changes without changing the phrase. The old password is needed."),
                Control::Button(tr!("Change"), ButtonKind::Secondary),
            ),
            (
                tr!("Lock the wallet"),
                lock_note,
                Control::Segmented(
                    &[tr!("5 min"), tr!("15 min"), tr!("1 hour"), tr!("on close")],
                    &mut lock_after,
                ),
            ),
            (
                tr!("Password for every payment"),
                tr!("Even when the wallet is unlocked"),
                Control::Toggle(&mut each),
            ),
        ],
    );
    if clicked == Some(0) {
        state.change = Some(PasswordChange::default());
    }
    if lock_after != state.lock_after {
        state.lock_after = lock_after;
        cx.send(Command::AutoLock {
            after_secs: state.auto_lock_secs(),
        });
    }
    if !crate::is_demo(cx.snap)
        && button(ui, a, tr!("Lock now"), ButtonKind::Secondary, None).clicked()
    {
        cx.send(Command::Lock);
    }
    *state.toggle("password-each-send") = each;
    let mut hide = *state.toggle("hide-amounts");
    let mut fresh = *state.toggle("new-address");
    let mut clip = *state.toggle("clipboard");
    group(
        ui,
        a,
        tr!("Privacy"),
        vec![
            (
                tr!("Hide amounts"),
                tr!("Balances and history show •••• until you hover over them"),
                Control::Toggle(&mut hide),
            ),
            (
                tr!("A new address after each payment"),
                tr!(
                    "Old addresses keep working. Off — always the wallet's first address: simpler, but payments are easier to link"
                ),
                Control::Toggle(&mut fresh),
            ),
            (
                tr!("Clipboard"),
                tr!(
                    "Copied addresses, IDs and links are wiped after 60 seconds if nothing new is in the clipboard. The phrase — always"
                ),
                Control::Toggle(&mut clip),
            ),
        ],
    );
    *state.toggle("hide-amounts") = hide;
    *state.toggle("new-address") = fresh;
    *state.toggle("clipboard") = clip;
    let asked = group(
        ui,
        a,
        tr!("Danger zone"),
        vec![(
            tr!("Delete the wallet from this computer"),
            tr!("The funds stay on the network. Only the phrase brings the wallet back."),
            Control::Danger(tr!("Delete")),
        )],
    );
    if asked.is_some() && !crate::is_demo(cx.snap) {
        state.delete = Some(Default::default());
    }
}

/// "Change password": the old one, the new one twice. The phrase does not change; the core
/// encrypts it again under the new password.
pub(super) fn password_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut SettingsState,
) {
    let p = a.p;
    let Some(change) = &mut state.change else {
        return;
    };
    let closed = dialog(
        ui,
        a,
        "password-dialog",
        "lock",
        tr!("Change password"),
        442.0,
        |ui, a| {
            let mut close = false;
            if change.result == Some(Ok(())) {
                notice(
                    ui,
                    a,
                    "confirmed",
                    p.accent,
                    tr!(
                        "Password changed. The phrase is the same — your paper copy is still right."
                    ),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked() {
                        close = true;
                    }
                });
                return close;
            }
            let field = |ui: &mut Ui, id: &str, label: &str, value: &mut String| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    field_label(ui, &p, label);
                    text_field(ui, &p, id, value, "", FieldKind::Password, |_| {}).changed()
                })
                .inner
            };
            let mut edited = field(ui, "password-old", tr!("Current password"), &mut change.old);
            edited |= field(ui, "password-new", tr!("New password"), &mut change.new);
            let level = strength(&change.new);
            if level > 0 {
                let label = [
                    "",
                    tr!("Too short"),
                    tr!("Weak"),
                    tr!("Good"),
                    tr!("Strong"),
                ][level];
                ui.label(txt(label, 12.0, Weight::Regular, p.muted));
            }
            if let Some(line) = missing_line(&change.new) {
                ui.label(txt(&line, 12.0, Weight::Regular, p.muted));
            }
            edited |= field(
                ui,
                "password-repeat",
                tr!("Repeat the new one"),
                &mut change.repeat,
            );
            if edited {
                change.result = None;
            }
            let mismatch = !change.repeat.is_empty() && change.repeat != change.new;
            let answer = match &change.result {
                _ if mismatch => Some(tr!("The passwords do not match").to_string()),
                Some(Err(CoreError::WrongPassword)) => {
                    Some(tr!("The current password is wrong").into())
                }
                Some(Err(CoreError::NotAvailable(why))) => Some(format!("{why}.")),
                Some(Err(e)) => Some(crate::text::failed(tr!("Not changed"), e)),
                _ => None,
            };
            if let Some(line) = answer {
                ui.label(txt(&line, 12.0, Weight::Regular, p.danger));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let ready = !change.old.is_empty()
                    && missing(&change.new).is_empty()
                    && change.new == change.repeat
                    && change.new != change.old
                    && !change.waiting;
                let label = if change.waiting {
                    tr!("Encrypting…")
                } else {
                    tr!("Change password")
                };
                if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked() {
                    // The passwords go to the core and are wiped here at once.
                    cx.send(Command::ChangePassword {
                        old: Password::new(change.old.to_string()),
                        new: Password::new(change.new.to_string()),
                    });
                    change.old.zeroize();
                    change.new.zeroize();
                    change.repeat.zeroize();
                    change.waiting = true;
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close
        },
    );
    if closed && let Some(mut change) = state.change.take() {
        change.old.zeroize();
        change.new.zeroize();
        change.repeat.zeroize();
    }
}
