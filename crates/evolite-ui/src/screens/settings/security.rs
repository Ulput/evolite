//! Security: password change, auto-lock, a password for every send, privacy toggles,
//! "Lock now". The auto-lock runs in the core (it holds the keys); this screen only
//! tells it how long to wait.

use super::parts::{Control, group, heading};
use super::{PasswordChange, SettingsState};
use crate::components::secret_password;
use crate::components::text;
use crate::components::{
    Assets, ButtonKind, FieldKind, Part, button, button_enabled, button_row, dialog_with_footer,
    field_label, notice, text_field,
};
use crate::context::Ctx;
use crate::screens::onboarding::{missing, missing_line, strength};
use crate::theme::type_scale::{BODY, SMALL};
use egui::Ui;
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
    // The core keeps "password for every payment" for the wallet (BUG-42): the switch shows
    // its answer, and turning it off asks for the password first.
    let was_each = crate::components::password_asked(cx.snap);
    let mut each = was_each;
    // FEAT-0004 FR-7: the wallet's threshold as the core keeps it.
    let current_large = cx.snap.wallet.as_ref().and_then(|w| w.large_spend);
    let was_large = large_choice(current_large);
    let mut large = was_large;
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
            // FEAT-0004 FR-7: on mainnet, a tick before a spend of more than this.
            (
                tr!("Confirm spends over"),
                tr!("On mainnet, a spend over this amount asks you to tick that you checked it"),
                Control::Segmented(
                    &[tr!("0.1 DASH"), tr!("1 DASH"), tr!("10 DASH"), tr!("Off")],
                    &mut large,
                ),
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
    if each != was_each {
        if each {
            cx.send(Command::SetPasswordEachSend {
                on: true,
                password: None,
            });
        } else {
            state.each_off = Some(Default::default());
        }
    }
    each_off_dialog(ui, a, cx, state);
    // FEAT-0004 FR-7 (AC-T6): lower or on at once; higher or off with the password.
    if large != was_large {
        let now = LARGE_CHOICES[large];
        if evolite_api::confirm::weakens(current_large, now) {
            state.large_weaker = Some((now, Default::default()));
        } else {
            cx.send(Command::SetLargeSpend {
                duffs: now,
                password: None,
            });
        }
    }
    large_weaker_dialog(ui, a, cx, state);
    // FEAT-0020 (AC-D10): where people look when something happened — the shown identity's keys.
    let me = cx
        .wallet()
        .identity
        .as_ref()
        .map(crate::screens::identity::identity_named);
    if let Some(me) = me {
        let title = format!(tr!("Keys {}"), me);
        let opened = group(
            ui,
            a,
            tr!("Identity keys"),
            vec![(
                &title,
                tr!(
                    "If a key could have leaked, disable it or replace the main one. A private key can be shown there too, with the password."
                ),
                Control::Value(tr!("Open"), false),
            )],
        );
        if opened == Some(0) {
            cx.go(crate::screens::Screen::IdentityKeys);
        }
    }
    let mut hide = *state.toggle("hide-amounts");
    let mut strangers = *state.toggle("stranger-pictures");
    let mut updates = state.is_on("update-check");
    let mut fresh = *state.toggle("new-address");
    let mut clip = *state.toggle("clipboard");
    group(
        ui,
        a,
        tr!("Privacy"),
        vec![
            (
                tr!("Hide amounts"),
                tr!(
                    "Balances and history show ••••. The eye button next to an amount switches this too."
                ),
                Control::Toggle(&mut hide),
            ),
            // FEAT-0039 AC-A10, frame N7: right under «Hide amounts», off by default, the
            // wallet's own.
            (
                tr!("Strangers' pictures in name suggestions"),
                tr!(
                    "Off: pictures only of your contacts. On: evolite loads strangers' avatars from their servers, and those servers see your IP and which names you look at."
                ),
                Control::Toggle(&mut strangers),
            ),
            // Р1 (design REL-verify, review r1): what it asks and who sees it, said where it is
            // turned off.
            (
                tr!("Check for a new version"),
                tr!(
                    "Once a day evolite asks GitHub whether a newer version is out; GitHub sees your IP. It only believes a signed answer and never downloads anything by itself."
                ),
                Control::Toggle(&mut updates),
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
    *state.toggle("stranger-pictures") = strangers;
    *state.toggle("update-check") = updates;
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
    // FEAT-0004 B (batch 52): the fields, the pause's plate and the buttons in the footer, which
    // stays in the window however low it is — the plate made the dialog too tall for 800×440,
    // and without a footer its passwords and buttons scrolled away with it.
    let closed = dialog_with_footer(
        ui,
        a,
        "password-dialog",
        "lock",
        tr!("Change password"),
        442.0,
        |ui, a, part| {
            let mut close = false;
            let done = change.result == Some(Ok(()));
            if part != Part::Footer {
                if done {
                    notice(
                        ui,
                        a,
                        "confirmed",
                        p.accent,
                        tr!(
                            "Password changed. The phrase is the same — your paper copy is still right."
                        ),
                    );
                }
                return false;
            }
            if done {
                button_row(ui, |ui| {
                    if button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked() {
                        close = true;
                    }
                });
                return close;
            }
            // FEAT-0004 B (AC-T3): the wallet's pause, as on the unlock screen — its plate and
            // countdown, the button disabled until it is over. Nothing can be changed meanwhile,
            // so the fields give way to the plate (what is typed stays): the footer fits the
            // smallest window (batch 52).
            let refused =
                secret_password::refused(change.result.as_ref().and_then(|r| r.as_ref().err()));
            let open = cx.snap.wallets.iter().find(|w| w.open);
            let gate =
                secret_password::Gate::of(open, refused, &mut change.cut, cx.now.timestamp());
            if gate.paused() {
                secret_password::lines(ui, a, &gate, None, cx.now);
            } else {
                let field = |ui: &mut Ui, id: &str, label: &str, value: &mut String| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        field_label(ui, &p, label);
                        text_field(ui, &p, id, value, "", FieldKind::Password, |_| {}).changed()
                    })
                    .inner
                };
                let mut edited =
                    field(ui, "password-old", tr!("Current password"), &mut change.old);
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
                    ui.label(text(label, SMALL, p.muted));
                }
                if let Some(line) = missing_line(&change.new) {
                    ui.label(text(&line, SMALL, p.muted));
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
            if let Some(line) = answer.filter(|_| !gate.paused()) {
                ui.label(text(&line, SMALL, p.danger));
            }
            button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
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
                if secret_password::send_button(
                    ui,
                    a,
                    label,
                    ButtonKind::Primary,
                    None,
                    ready,
                    &gate,
                )
                .clicked()
                {
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

/// FEAT-0004 FR-7: the thresholds Settings offers, in the order of its segments (the last: off).
const LARGE_CHOICES: [Option<evolite_api::Duffs>; 4] = [
    Some(10_000_000),
    Some(100_000_000),
    Some(1_000_000_000),
    None,
];

/// The segment of `threshold`: its own, else the one nearest above it (1 DASH when none is).
fn large_choice(threshold: Option<evolite_api::Duffs>) -> usize {
    match threshold {
        None => 3,
        Some(t) => LARGE_CHOICES
            .iter()
            .position(|c| c.is_some_and(|c| c >= t))
            .unwrap_or(1),
    }
}

/// FEAT-0004 FR-7 (AC-T6): raising the threshold or turning it off takes the wallet's password,
/// counted as every password; anyone at the unlocked computer could weaken it otherwise.
fn large_weaker_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut SettingsState) {
    let Some((duffs, form)) = &mut state.large_weaker else {
        return;
    };
    let p = a.p;
    let mut send = false;
    let closed = dialog_with_footer(
        ui,
        a,
        "large-spend-weaker",
        "lock",
        tr!("Confirm spends over"),
        440.0,
        |ui, a, part| {
            if part == Part::Footer {
                let entered = form.field(ui, &p, "large-spend-password", true);
                let ready = form.ready(true);
                let mut close = false;
                button_row(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    if button_enabled(ui, a, tr!("Change"), ButtonKind::Primary, None, ready)
                        .clicked()
                        || (entered && ready)
                    {
                        send = true;
                    }
                });
                return close;
            }
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.label(text(
                tr!("Then more spends will go without the tick. Enter the wallet password to change it."),
                BODY,
                p.text,
            ));
            false
        },
    );
    if send {
        cx.send(Command::SetLargeSpend {
            duffs: *duffs,
            password: form.password(true),
        });
    } else if closed {
        state.large_weaker = None;
    }
}

/// Turning "password for every payment" off: the wallet password first, checked by the core
/// (anyone at the unlocked computer could switch it off and spend otherwise).
fn each_off_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut SettingsState) {
    let Some(form) = &mut state.each_off else {
        return;
    };
    let p = a.p;
    let mut turn_off = false;
    let closed = dialog_with_footer(
        ui,
        a,
        "password-each-send-off",
        "lock",
        tr!("Payments without the password"),
        440.0,
        |ui, a, part| {
            if part == Part::Footer {
                // The footer (BUG-122): always in the window.
                let entered = form.field(ui, &p, "password-each-send-off", true);
                let ready = form.ready(true);
                let mut close = false;
                button_row(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    if button_enabled(ui, a, tr!("Turn off"), ButtonKind::Primary, None, ready)
                        .clicked()
                        || (entered && ready)
                    {
                        turn_off = true;
                    }
                });
                return close;
            }
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.label(text(
                tr!(
                    "While the wallet is unlocked, anyone at this computer could then send its money. Enter the wallet password to turn the check off."
                ), BODY, p.text));
            false
        },
    );
    if turn_off {
        cx.send(Command::SetPasswordEachSend {
            on: false,
            password: form.password(true),
        });
    } else if closed {
        state.each_off = None;
    }
}
