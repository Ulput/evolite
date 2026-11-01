//! The password step: it encrypts the phrase on this computer; with a strength hint.

use super::txt;
use super::{Flow, Onboarding, actions, heading, steps};
use crate::components::{Assets, FieldKind, field_label, notice, pill, text_field};
use crate::context::Ctx;
use crate::theme::{Weight, sans};
use egui::{Align2, CornerRadius, Rect, Sense, Ui, pos2, vec2};
use evolite_api::{Command, Password};
use zeroize::Zeroize;

/// Password strength 0–4: under 8 characters too short, then by length once every kind of
/// character the rules ask for is there.
pub fn strength(password: &str) -> usize {
    let len = password.chars().count();
    if len < 8 {
        return usize::from(len > 0);
    }
    let kinds = [
        password.chars().any(|c| c.is_lowercase()),
        password.chars().any(|c| c.is_uppercase()),
        password.chars().any(|c| c.is_ascii_digit()),
        password.chars().any(|c| !c.is_alphanumeric()),
    ]
    .iter()
    .filter(|k| **k)
    .count();
    // A password the rules (`missing`) do not take is never shown as more than "Weak".
    match (len, kinds) {
        (12.., 4) => 4,
        (8.., 4) => 3,
        _ => 2,
    }
}

/// What a new password still lacks, as in other wallets and services: at least 8
/// characters, an upper- and a lowercase letter, a digit and a symbol. Empty: it will do.
/// Only new passwords are held to this; one set earlier still opens the wallet.
pub fn missing(password: &str) -> Vec<&'static str> {
    let has = |f: fn(char) -> bool| password.chars().any(f);
    let mut missing = Vec::new();
    if password.chars().count() < 8 {
        missing.push(tr!("8 characters"));
    }
    if !has(char::is_uppercase) {
        missing.push(tr!("a capital letter"));
    }
    if !has(char::is_lowercase) {
        missing.push(tr!("a lowercase letter"));
    }
    if !has(|c| c.is_ascii_digit()) {
        missing.push(tr!("a digit"));
    }
    if !has(|c| !c.is_alphanumeric()) {
        missing.push(tr!("a symbol, e.g. ! or -"));
    }
    missing
}

/// The line under a new password: what is still needed, or nothing.
pub fn missing_line(password: &str) -> Option<String> {
    let missing = missing(password);
    (!password.is_empty() && !missing.is_empty())
        .then(|| format!(tr!("Still needed: {}"), missing.join(", ")))
}

/// O3 · A password for this computer.
pub(super) fn password(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, o: &mut Onboarding) {
    let p = a.p;
    if o.flow == Flow::Create {
        steps(ui, &p, 3);
    }
    heading(
        ui,
        &p,
        tr!("A password for this computer"),
        tr!(
            "The password encrypts the phrase on disk and is needed for every signature. It does not restore the wallet — the phrase does."
        ),
    );
    let level = strength(&o.password);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, &p, tr!("Password"));
        text_field(
            ui,
            &p,
            "ob-password",
            &mut o.password,
            "",
            FieldKind::Password,
            |_| {},
        );
        ui.add_space(6.0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
        for i in 0..4 {
            let r = Rect::from_min_size(
                pos2(rect.left() + i as f32 * 46.0, rect.center().y - 1.5),
                vec2(40.0, 3.0),
            );
            let color = if i < level { p.ink } else { p.line };
            ui.painter().rect_filled(r, CornerRadius::same(2), color);
        }
        let label = [
            "",
            tr!("Too short"),
            tr!("Weak"),
            tr!("Good"),
            tr!("Strong"),
        ][level];
        ui.painter().text(
            pos2(rect.left() + 4.0 * 46.0 + 6.0, rect.center().y),
            Align2::LEFT_CENTER,
            crate::i18n::t(label),
            sans(12.0, Weight::Regular),
            p.muted,
        );
        if let Some(line) = missing_line(&o.password) {
            ui.label(txt(&line, 12.0, Weight::Regular, p.muted));
        }
    });
    let mismatch = !o.repeat.is_empty() && o.repeat != o.password;
    let mut entered = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, &p, tr!("Repeat the password"));
        let field = text_field(
            ui,
            &p,
            "ob-repeat",
            &mut o.repeat,
            "",
            FieldKind::Password,
            |_| {},
        );
        // Enter after the repeat creates the wallet, as the button does.
        entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if mismatch {
            ui.label(txt(
                tr!("The passwords do not match"),
                12.0,
                Weight::Regular,
                p.danger,
            ));
        }
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        field_label(ui, &p, tr!("Lock the wallet"));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (i, label) in [
                tr!("after 5 min"),
                tr!("15 min"),
                tr!("1 hour"),
                tr!("on close"),
            ]
            .iter()
            .enumerate()
            {
                if pill(ui, &p, label, o.lock_after == i).clicked() {
                    o.lock_after = i;
                }
            }
        });
    });
    notice(
        ui,
        a,
        "lock",
        p.muted,
        tr!(
            "A forgotten password cannot be reset. If it happens, restore the wallet from the phrase and set a new password."
        ),
    );
    if let Some(error) = &o.error {
        ui.label(txt(error, 12.0, Weight::Regular, p.danger));
    }
    let ready = missing(&o.password).is_empty() && o.password == o.repeat && !o.waiting;
    let label = if o.waiting {
        tr!("Creating…")
    } else if o.flow == Flow::Create {
        tr!("Create a wallet")
    } else {
        tr!("Open the wallet")
    };
    let pressed = actions(ui, a, None, label, ready).1 || ((o.submit_on_open || entered) && ready);
    if pressed {
        // The password goes to the core and is wiped here at once.
        cx.send(Command::CreateWallet {
            password: Password::new(o.password.to_string()),
        });
        o.password.zeroize();
        o.repeat.zeroize();
        // "Lock the wallet" becomes the auto-lock in Settings.
        o.lock_choice = Some(o.lock_after);
        o.waiting = true;
        o.error = None;
        o.submit_on_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::{missing, missing_line, strength};

    #[test]
    fn rates_passwords() {
        assert_eq!(strength(""), 0);
        assert_eq!(strength("abc"), 1);
        assert_eq!(strength("abcdefgh"), 2);
        assert_eq!(strength("abcd1234!"), 2, "no capital: weak");
        assert_eq!(strength("Abcd1234!"), 3);
        assert_eq!(strength("Correct-horse-7"), 4);
    }

    #[test]
    fn a_new_password_needs_every_rule() {
        assert_eq!(
            missing("abc"),
            [
                tr!("8 characters"),
                tr!("a capital letter"),
                tr!("a digit"),
                tr!("a symbol, e.g. ! or -")
            ]
        );
        assert_eq!(missing("abcdefgh1!"), [tr!("a capital letter")]);
        assert_eq!(missing("ABCDEFGH1!"), [tr!("a lowercase letter")]);
        assert!(missing("Abcdefg1!").is_empty());
        assert!(
            missing("Password-2026").is_empty(),
            "Cyrillic letters count too"
        );
    }

    #[test]
    fn the_hint_says_what_is_left_and_nothing_when_empty_or_done() {
        assert_eq!(missing_line(""), None);
        assert_eq!(missing_line("Abcdefg1!"), None);
        assert_eq!(
            missing_line("Abcdefgh!").as_deref(),
            Some("Still needed: a digit")
        );
    }

    #[test]
    fn strength_starts_at_eight_characters() {
        assert_eq!(strength("Ab1!xyz"), 1, "7 characters: too short");
        assert!(strength("Ab1!xyzw") >= 2);
    }
}
