//! FEAT-0020 R-10: one private key of the identity, by the password (the designer's K6, K6b).
//! Step 1: what the key gives, «I understand», the password. Step 2: WIF and hex, each with
//! «Copy» (`copy_secret` only), hidden 2 minutes after the last action, at once on a lock, on
//! leaving K1 and when the dialog closes. The window is kept out of captures while it shows.

use super::*;
use crate::components::{Part, dialog_with_footer, secret_password};
use evolite_api::{IdentityKey, KeyLevel, KeyPurpose, Network, Password, RevealedKey};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

/// How long a shown key stays after the last action (AC-T13).
pub(crate) const SHOWN_FOR: Duration = Duration::from_secs(120);

/// A private key asked for (K6) or shown (K6b).
#[derive(Default)]
pub struct Reveal {
    pub(crate) key_id: u32,
    pub(crate) understood: bool,
    pub(crate) password: Zeroizing<String>,
    /// Asked for this frame: the command goes as soon as K1 knows the identity.
    pub(crate) ask: bool,
    pub(crate) waiting: bool,
    pub(crate) refused: Option<CoreError>,
    /// Wiped when dropped.
    pub(crate) shown: Option<RevealedKey>,
    /// The last action while it is shown.
    pub(crate) touched: Option<Instant>,
    /// SEC-1 (BUG-228): «read the words to a screen reader», for one showing.
    pub(crate) reading: crate::secret_words::Reading,
    /// SEC-11: the wallet's pause after wrong passwords, counted down as the unlock does.
    pub(crate) cut: crate::screens::pause::Cut,
}

impl Reveal {
    pub(crate) fn of(key_id: u32) -> Self {
        Self {
            key_id,
            ..Default::default()
        }
    }

    /// The core's answer for this key: shown, with the password wiped; or why not, the
    /// password kept to be corrected.
    pub(crate) fn on_revealed(&mut self, result: Result<RevealedKey, CoreError>) {
        if !self.waiting {
            return;
        }
        self.waiting = false;
        match result {
            Ok(key) if key.key_id == self.key_id => {
                self.password.zeroize();
                self.refused = None;
                self.shown = Some(key);
                self.touched = Some(Instant::now());
            }
            Ok(_) => {}
            Err(e) => self.refused = Some(e),
        }
    }

    /// Hidden for good: 2 minutes without an action have passed.
    fn timed_out(&self) -> bool {
        self.touched.is_some_and(|at| at.elapsed() >= SHOWN_FOR)
    }
}

/// What a key lets whoever knows it do (the designer's five, by purpose and level).
fn gives(key: &IdentityKey) -> &'static str {
    match (&key.purpose, key.level) {
        (KeyPurpose::Identity, _) | (_, KeyLevel::Master) => tr!("change the identity's keys"),
        (KeyPurpose::Transfers, _) => tr!("transfer credits"),
        (KeyPurpose::Encryption, _) => tr!("read contacts' requests"),
        (KeyPurpose::Documents, KeyLevel::Critical) => tr!("send and burn tokens"),
        _ => tr!("sign in to apps"),
    }
}

/// «m:ss».
fn clock(left: Duration) -> String {
    let s = left.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// K6 / K6b over K1, for the identity `id` named `me`. Sends the command when asked; closes
/// (and drops the key) on «Cancel», «Hide», Esc and the timer.
pub(super) fn reveal_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    slot: &mut Option<Reveal>,
    id: &Identity,
    me: &str,
) {
    let Some(r) = slot.as_mut() else {
        return;
    };
    let Some(key) = id.keys.iter().find(|k| k.id == r.key_id).cloned() else {
        *slot = None;
        return;
    };
    if r.timed_out() {
        *slot = None;
        return;
    }
    if std::mem::take(&mut r.ask) && !r.waiting {
        cx.send(Command::RevealIdentityKey {
            identity: id.id.clone(),
            key_id: r.key_id,
            password: Password::new(r.password.to_string()),
        });
        r.waiting = true;
        r.refused = None;
    }
    let p = a.p;
    let closed = match r.shown.clone() {
        Some(shown) => {
            crate::capture::keep_out();
            ui.ctx().request_repaint_after(Duration::from_secs(1));
            let left = SHOWN_FOR.saturating_sub(r.touched.map_or(Duration::ZERO, |t| t.elapsed()));
            let title = format!(tr!("Private key {} · {}"), key.id, me);
            dialog_with_footer(ui, a, "key-shown", "key", &title, 480.0, |ui, a, part| {
                if part == Part::Footer {
                    let mut hide = false;
                    button_row(ui, |ui| {
                        hide = button(ui, a, tr!("Hide"), ButtonKind::Primary, None).clicked();
                    });
                    return hide;
                }
                ui.spacing_mut().item_spacing.y = 8.0;
                let network = match cx.snap.network {
                    Network::Testnet => tr!("testnet · compressed key"),
                    Network::Mainnet => tr!("mainnet · compressed key"),
                };
                // SEC-1 (BUG-228): the key reaches the tree only with reading on.
                let read = r.reading.showing(ui.ctx());
                let reader = crate::secret_words::screen_reader_running();
                if reader {
                    crate::secret_words::toggle(ui, a, &mut r.reading);
                }
                for (name, note, value, copy) in [
                    ("WIF", Some(network), &shown.wif, tr!("Copy WIF")),
                    ("hex", None, &shown.hex, tr!("Copy hex")),
                ] {
                    ui.horizontal(|ui| {
                        field_label(ui, &p, name);
                        if let Some(note) = note {
                            ui.label(text(note, SMALL, p.muted));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if text_action(ui, a, "copy", copy).clicked() {
                                // Wiped from the clipboard in 60 s, kept out of its history.
                                crate::clipboard::copy_secret(ui.ctx(), value.clone());
                                r.touched = Some(Instant::now());
                            }
                        });
                    });
                    egui::Frame::new()
                        .fill(p.raised)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            // Read whole, left to right in every language; in the tree only
                            // with reading on (SEC-1).
                            if read {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(value.as_str())
                                            .font(crate::theme::mono(13.0, Weight::Regular))
                                            .color(p.text),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Wrap)
                                    // No Ctrl+C past `copy_secret` (BUG-228 r1 M1, SEC-2 B2).
                                    .selectable(false),
                                );
                            } else {
                                crate::secret_words::painted(
                                    ui,
                                    value.as_str(),
                                    crate::theme::mono(13.0, Weight::Regular),
                                    p.text,
                                    &format!(tr!("{} key, shown on the screen"), name),
                                );
                            }
                        });
                }
                if !reader {
                    crate::secret_words::toggle(ui, a, &mut r.reading);
                }
                notice(
                    ui,
                    a,
                    "info",
                    p.muted,
                    &format!(
                        tr!(
                            "The key hides in {} — and at once on a lock or on leaving the screen. Copied, it is wiped from the clipboard in 60 s."
                        ),
                        clock(left)
                    ),
                );
                false
            })
        }
        None => {
            let title = format!(tr!("Show the private key {}"), key.id);
            let (purpose, _) = super::keys::key_words(&key);
            // SEC-11: the open wallet's password is counted with the unlock's; its pause is
            // shown and holds the button here too.
            let now = cx.now;
            let gate = secret_password::Gate::of(
                cx.snap.wallets.iter().find(|w| w.open),
                secret_password::refused(r.refused.as_ref()),
                &mut r.cut,
                now.timestamp(),
            );
            dialog_with_footer(
                ui,
                a,
                "key-reveal",
                "warning",
                &title,
                480.0,
                |ui, a, part| {
                    if part == Part::Footer {
                        let mut close = false;
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            field_label(ui, &p, tr!("Wallet password"));
                            if text_field(
                                ui,
                                &p,
                                "key-password",
                                &mut r.password,
                                "",
                                FieldKind::Password,
                                |_| {},
                            )
                            .changed()
                            {
                                r.refused = None;
                            }
                            // Not available is no fault of the password: said in grey above the
                            // lines every password before a secret has (SEC-11).
                            if let Some(CoreError::NotAvailable(why)) = &r.refused {
                                ui.label(text(why, SMALL, p.muted));
                            }
                            let error = match &r.refused {
                                Some(CoreError::WrongPassword) => {
                                    Some(tr!("Wrong password").to_string())
                                }
                                // The pause's plate says it.
                                Some(
                                    CoreError::TooManyTries { .. } | CoreError::NotAvailable(_),
                                )
                                | None => None,
                                Some(e) => Some(crate::text::failed(tr!("It did not work"), e)),
                            };
                            secret_password::lines(ui, a, &gate, error.as_deref(), now);
                        });
                        button_row(ui, |ui| {
                            if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                                close = true;
                            }
                            let ready = r.understood && !r.password.is_empty() && !r.waiting;
                            if secret_password::send_button(
                                ui,
                                a,
                                tr!("Show the key"),
                                ButtonKind::Primary,
                                None,
                                ready,
                                &gate,
                            )
                            .clicked()
                            {
                                r.ask = true;
                            }
                        });
                        return close;
                    }
                    ui.spacing_mut().item_spacing.y = 10.0;
                    let which = format!(tr!("Key {}"), key.id);
                    ui.label(text(
                        &format!(
                            "{which} · {purpose} · {}",
                            crate::text::key_level(key.level)
                        ),
                        BODY,
                        p.text,
                    ));
                    notice(
                        ui,
                        a,
                        "warning",
                        p.accent,
                        &format!(
                            tr!(
                                "Whoever knows this key can act as {} within its purpose: {}. evolite never asks you to enter it. Do not paste it on websites or send it in messages."
                            ),
                            me,
                            gives(&key)
                        ),
                    );
                    crate::components::checkbox(
                        ui,
                        a,
                        &mut r.understood,
                        tr!("I understand: this key lets one act as the identity"),
                    );
                    false
                },
            )
        }
    };
    if closed {
        // The key and the password go with the dialog.
        *slot = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(id: u32) -> RevealedKey {
        RevealedKey {
            identity: "Id".into(),
            key_id: id,
            wif: Zeroizing::new("w".into()),
            hex: Zeroizing::new("h".into()),
        }
    }

    /// AC-T13: a key shown wipes the password; a refusal keeps it; another key's answer or an
    /// answer nobody waits for changes nothing.
    #[test]
    fn the_answer_shows_the_key_and_wipes_the_password() {
        let mut r = Reveal::of(2);
        r.password = Zeroizing::new("pw".into());
        r.on_revealed(Ok(shown(2)));
        assert!(r.shown.is_none(), "not waited for");
        r.waiting = true;
        r.on_revealed(Err(CoreError::WrongPassword));
        assert_eq!(r.refused, Some(CoreError::WrongPassword));
        assert_eq!(r.password.as_str(), "pw");
        r.waiting = true;
        r.on_revealed(Ok(shown(3)));
        assert!(r.shown.is_none(), "another key's");
        r.waiting = true;
        r.on_revealed(Ok(shown(2)));
        assert!(r.shown.is_some() && r.password.is_empty() && r.touched.is_some());
    }

    /// AC-T13: hidden 2 minutes after the last action.
    #[test]
    fn a_shown_key_hides_after_two_idle_minutes() {
        let mut r = Reveal::of(2);
        assert!(!r.timed_out());
        r.touched = Some(Instant::now());
        assert!(!r.timed_out());
        r.touched = Instant::now().checked_sub(SHOWN_FOR);
        assert!(r.timed_out());
        assert_eq!(clock(Duration::from_secs(114)), "1:54");
    }

    /// AC-T14: the dialog copies a key only with `copy_secret` — never with the plain copy,
    /// which neither wipes it nor keeps it out of the clipboard's history.
    #[test]
    fn the_key_is_copied_only_as_a_secret() {
        let source = include_str!("key_reveal.rs");
        let plain = ["clipboard::", "copy("].concat();
        let plain_text = ["copy", "_text("].concat();
        assert!(!source.contains(&plain) && !source.contains(&plain_text));
        assert!(source.contains("copy_secret(ui.ctx(), value.clone())"));
    }
}
