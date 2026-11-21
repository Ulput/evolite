//! FEAT-0041 · the BIP-39 passphrase on «Restore» (P1: «Have a BIP-39 passphrase? ⌄», always
//! there, the switch on or off — analyst-158) and on «Create» (P2: «Advanced ⌄», only with the
//! advanced features on; with a repeat). Folded by default; open, the field with «Show», the
//! warning R-5 and the line about DashPay Android (R-7), and what the field notes (R-8…R-10).

use super::Onboarding;
use super::txt;
use crate::components::{Assets, FieldKind, field_label, notice, text_field};
use crate::theme::Weight;
use egui::{CursorIcon, Ui};
use evolite_api::passphrase::{self, Note};

/// R-5, said wherever the passphrase is typed.
pub(super) const WARNING: &str = tr!(
    "The BIP-39 passphrase is part of the key. Without it the same phrase opens another, empty wallet. It cannot be recovered or reset. Write it down apart from the phrase."
);

/// R-7.
const DASHPAY: &str = tr!(
    "DashPay Android does not support a BIP-39 passphrase: a wallet with one opens only in evolite, Dash Core and wallets that have it."
);

/// The folded row, then, open, the passphrase's fields. `repeat`: «Create» asks it twice.
pub(super) fn block(ui: &mut Ui, a: &mut Assets, o: &mut Onboarding, repeat: bool) {
    let p = a.p;
    let title = if repeat {
        tr!("Advanced")
    } else {
        tr!("Have a BIP-39 passphrase?")
    };
    let icon = if o.bip39_open {
        "chevron-up"
    } else {
        "chevron-down"
    };
    let row = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // The words are part of the row: a selectable label would take their clicks.
            ui.style_mut().interaction.selectable_labels = false;
            ui.label(txt(title, 14.0, Weight::Medium, p.text));
            ui.add(a.icons.icon(icon, p.muted, &p, 16.0));
        })
        .response;
    let row =
        crate::a11y::row_click(ui, &row, "bip39-fold").on_hover_cursor(CursorIcon::PointingHand);
    crate::components::named_choice(
        &row,
        egui::WidgetType::CollapsingHeader,
        o.bip39_open,
        title,
    );
    if row.clicked() {
        o.bip39_open = !o.bip39_open;
    }
    if !o.bip39_open {
        return;
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, &p, tr!("BIP-39 passphrase (optional)"));
        text_field(
            ui,
            &p,
            "ob-bip39",
            &mut o.bip39,
            "",
            FieldKind::Password,
            |_| {},
        );
        for note in passphrase::notes(&o.bip39) {
            let (line, color) = note_line(note);
            ui.add(crate::components::para(
                line,
                12.0,
                Weight::Regular,
                if color { p.danger } else { p.muted },
            ));
        }
    });
    if repeat {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field_label(ui, &p, tr!("Repeat the BIP-39 passphrase"));
            text_field(
                ui,
                &p,
                "ob-bip39-repeat",
                &mut o.bip39_repeat,
                "",
                FieldKind::Password,
                |_| {},
            );
            if mismatch(o) {
                ui.label(txt(
                    tr!("The passphrases do not match"),
                    12.0,
                    Weight::Regular,
                    p.danger,
                ));
            }
        });
    }
    notice(ui, a, "info", p.accent, WARNING);
    ui.add(crate::components::para(
        DASHPAY,
        12.0,
        Weight::Regular,
        p.muted,
    ));
}

/// What a note says, and whether it is a refusal (`danger`).
fn note_line(note: Note) -> (&'static str, bool) {
    match note {
        Note::TooLong => (tr!("The BIP-39 passphrase is too long"), true),
        Note::Spaces => (
            tr!("The passphrase has spaces at the ends or double spaces — they are part of it too"),
            false,
        ),
        Note::NotLatin => (
            tr!(
                "The passphrase has letters outside the Latin alphabet. Other wallets and keyboard layouts may type them differently — check that you can type it there"
            ),
            false,
        ),
        Note::Trezor => (
            tr!("Trezor takes a passphrase of at most 50 characters"),
            false,
        ),
    }
}

/// «Create»: a repeat typed that is not the passphrase.
fn mismatch(o: &Onboarding) -> bool {
    !o.bip39_repeat.is_empty() && *o.bip39_repeat != *o.bip39
}

/// Whether «Continue» may go on as far as the passphrase goes: not too long (R-10), and on
/// «Create» repeated exactly (UC-3).
pub(super) fn ready(o: &Onboarding, repeat: bool) -> bool {
    passphrase::fits(&o.bip39) && (!repeat || *o.bip39_repeat == *o.bip39)
}

/// The passphrase to send, `None` when none was typed.
pub(super) fn typed(o: &Onboarding) -> Option<passphrase::Passphrase> {
    (!o.bip39.is_empty()).then(|| passphrase::Passphrase::new(o.bip39.to_string()))
}

#[cfg(test)]
mod tests {
    use super::super::{Flow, Onboarding};
    use super::{ready, typed};

    /// AC-A15: 257 Latin letters keep «Continue» off and stay in the field whole; on «Create» a
    /// repeat that differs does too; nothing typed is no passphrase.
    #[test]
    fn continue_waits_for_a_passphrase_that_fits_and_matches() {
        let mut o = Onboarding::new(Flow::Create);
        assert!(ready(&o, true) && typed(&o).is_none());
        o.bip39 = zeroize::Zeroizing::new("a".repeat(256));
        assert!(!ready(&o, true), "not repeated yet");
        o.bip39_repeat = o.bip39.clone();
        assert!(ready(&o, true));
        o.bip39 = zeroize::Zeroizing::new("a".repeat(257));
        o.bip39_repeat = o.bip39.clone();
        assert!(!ready(&o, true) && !ready(&o, false));
        assert_eq!(o.bip39.len(), 257, "not cut");
        o.bip39 = zeroize::Zeroizing::new("TREZOR".into());
        assert!(ready(&o, false), "«Restore» asks no repeat");
        assert_eq!(typed(&o).unwrap().expose(), "TREZOR");
    }
}
