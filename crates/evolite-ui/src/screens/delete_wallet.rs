//! Delete the wallet from the computer (Figma "Flows"): from Settings with the password, or
//! from the unlock screen as "Forgot the password?" without it. The money stays on the network; only
//! the phrase brings it back, so the dialog asks for proof the person has it and for the word
//! "delete". The balance is shown only when the wallet is open.

use super::txt;
use crate::components::secret_password;
use crate::components::{
    Assets, ButtonKind, CardAmount, FieldKind, Part, button, button_row, checkbox, field_label,
    notice, text_field,
};
use crate::context::Ctx;
use crate::theme::Weight;
use egui::{Align, Layout, Ui};
use evolite_api::{Balances, Command, CoreError, Password};
use zeroize::{Zeroize, Zeroizing};

/// The word typed to confirm.
const WORD: &str = tr!("delete");
/// N16: the payment notes are only here (FEAT-0003).
pub const DELETES_NOTES: &str = tr!(
    "Payment notes are kept only on this computer: they are deleted with the wallet and do not come back when you restore from the phrase."
);

/// The word as typed: in Russian or in the interface language (the hint shows the latter).
fn typed_word(typed: &str) -> bool {
    let typed = typed.trim().to_lowercase();
    typed == WORD || typed == crate::i18n::t_in(crate::i18n::current(), WORD).to_lowercase()
}

#[derive(Default)]
pub struct DeleteForm {
    /// "Forgot the password?": no password, no balance.
    pub forgot: bool,
    /// Another wallet of this computer (its id and name), deleted by its own password from
    /// Settings while the open one stays open (R-28, BUG-44); `None`: the open one.
    pub other: Option<(String, String)>,
    has_phrase: bool,
    password: Zeroizing<String>,
    word: String,
    waiting: bool,
    result: Option<Result<(), CoreError>>,
    /// FEAT-0004 B: the wallet's pause, cut as the core cuts it when the clock is set back.
    cut: crate::screens::pause::Cut,
}

impl DeleteForm {
    pub fn forgot() -> Self {
        Self {
            forgot: true,
            ..Self::default()
        }
    }

    /// The deletion of another wallet (`id`, `name`), asking that wallet's own password.
    pub fn other(id: String, name: String) -> Self {
        Self {
            other: Some((id, name)),
            ..Self::default()
        }
    }

    /// The core's answer; `true` when the wallet is gone and the dialog is done.
    pub fn on_done(&mut self, result: &Result<(), CoreError>) -> bool {
        self.waiting = false;
        self.result = Some(result.clone());
        result.is_ok()
    }

    /// Everything the dialog asks for is there.
    fn ready(&self) -> bool {
        self.has_phrase
            && typed_word(&self.word)
            && (self.forgot || !self.password.is_empty())
            && !self.waiting
    }
}

/// Shows the dialog; returns `true` when it should close (cancelled).
/// BUG-248: DASH locked on Core for an unfinished operation (the analyst's words, r2).
const UNFINISHED: &str = tr!(
    "Some DASH are locked for an unfinished operation: an identity creation or top-up, a top-up of the Platform address or a transfer to the pool. Finish it before deleting the wallet: after restoring from the phrase it may not be possible."
);

pub fn delete_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut DeleteForm,
    balance: Option<&Balances>,
    unfinished: bool,
    notes: bool,
) -> bool {
    let p = a.p;
    let now = cx.now;
    let title = if form.forgot {
        tr!("Forgot the password?")
    } else {
        tr!("Delete the wallet from the computer")
    };
    let closed = crate::components::dialog_with_summary(
        ui,
        a,
        "delete-wallet",
        "trash",
        title,
        470.0,
        |ui, a, part| {
            if part == Part::Footer {
                return delete_footer(ui, a, cx, form);
            }
            // BUG-249: what is agreed to, pinned over the fields: DASH still locked for an
            // operation, and the tick the button waits for (the architect's condition 2).
            if part == Part::Summary {
                if unfinished {
                    notice(ui, a, "warning", p.danger, UNFINISHED);
                }
                let has = if form.forgot {
                    tr!("I have this wallet's recovery phrase")
                } else {
                    tr!("The phrase is written down and checked")
                };
                checkbox(ui, a, &mut form.has_phrase, has);
                return false;
            }
            // What the phrase brings back, read before deleting rather than after (BUG-58): the
            // first line, or the second after the password's in «Forgot the password?» (designer,
            // 2026-11-06).
            let stays = crate::screens::settings::STAYS_ON_NETWORK;
            let lines: &[&str] = if form.forgot {
                &[
                    tr!(
                        "The password cannot be recovered: it encrypts the phrase on this computer. Delete the wallet from here and restore it — or another one — from the phrase."
                    ),
                    stays,
                ]
            } else {
                &[stays]
            };
            if let Some(balances) = balance.filter(|_| !form.forgot) {
                // The total counts Core: as sure as Core is (BUG-80), never a 0 not known yet.
                let total = CardAmount::total(balances);
                ui.horizontal(|ui| {
                    ui.label(txt(tr!("Balance"), 13.0, Weight::Regular, p.muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(crate::text::core_amount(balances, total, now))
                                .font(crate::theme::mono(13.0, Weight::Medium))
                                .color(total.color(&p)),
                        );
                    });
                });
            }
            for line in lines {
                ui.add(crate::components::para(line, 13.0, Weight::Regular, p.text));
            }
            // The wallet has payment notes: they go with it for good (FEAT-0003, AC-D7).
            if notes {
                notice(ui, a, "document", p.danger, DELETES_NOTES);
            }
            // And what it does not (BUG-58).
            crate::components::notice_parts(
                ui,
                a,
                "info",
                p.muted,
                &[(
                    crate::screens::settings::KEPT_HERE,
                    crate::screens::settings::KEPT_HERE_TEXT,
                )],
            );
            false
        },
    );
    if closed {
        form.password.zeroize();
    }
    closed && !form.waiting
}

/// The dialog's footer (BUG-122): the password, the word, what went wrong and the buttons, always
/// in the window. Whether it closed.
fn delete_footer(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut DeleteForm) -> bool {
    let p = a.p;
    let mut close = false;
    if !form.forgot {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            match &form.other {
                Some((_, name)) => field_label(
                    ui,
                    &p,
                    &format!(tr!("Password of “{}”"), crate::i18n::own(name)),
                ),
                None => field_label(ui, &p, tr!("Password")),
            }
            if text_field(
                ui,
                &p,
                "delete-password",
                &mut form.password,
                "",
                FieldKind::Password,
                |_| {},
            )
            .changed()
            {
                form.result = None;
            }
        });
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, &p, tr!("Type “delete”"));
        text_field(
            ui,
            &p,
            "delete-word",
            &mut form.word,
            WORD,
            FieldKind::Text,
            |_| {},
        );
    });
    // FEAT-0004 B: the pause of the wallet deleted — the open one, or the other one by its own
    // password; «Forgot the password?» asks none, so nothing holds it.
    let gate = if form.forgot {
        secret_password::Gate::default()
    } else {
        let wallet = match &form.other {
            Some((id, _)) => cx.snap.wallets.iter().find(|w| &w.id == id),
            None => cx.snap.wallets.iter().find(|w| w.open),
        };
        let refused = secret_password::refused(form.result.as_ref().and_then(|r| r.as_ref().err()));
        secret_password::Gate::of(wallet, refused, &mut form.cut, cx.now.timestamp())
    };
    let error = match &form.result {
        _ if gate.paused() => None,
        Some(Err(CoreError::WrongPassword)) => Some(tr!("Wrong password").to_owned()),
        Some(Err(CoreError::NotAvailable(why) | CoreError::Failed(why))) => Some(format!("{why}.")),
        Some(Err(_)) => Some(tr!("Could not delete the wallet").to_owned()),
        _ => None,
    };
    if gate.paused() {
        secret_password::lines(ui, a, &gate, None, cx.now);
    } else if let Some(error) = error {
        ui.add(crate::components::para(
            &error,
            12.0,
            Weight::Regular,
            p.danger,
        ));
    }
    button_row(ui, |ui| {
        if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
            close = true;
        }
        let label = if form.waiting {
            tr!("Deleting…")
        } else {
            tr!("Delete from the computer")
        };
        if secret_password::send_button(ui, a, label, ButtonKind::Danger, None, form.ready(), &gate)
            .clicked()
        {
            // The password goes to the core and is wiped here at once.
            let password = (!form.forgot).then(|| Password::new(form.password.to_string()));
            form.password.zeroize();
            cx.send(Command::DeleteWallet {
                password,
                wallet: form.other.as_ref().map(|(id, _)| id.clone()),
            });
            form.waiting = true;
            form.result = None;
        }
    });
    close
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_the_phrase_the_word_and_from_settings_the_password() {
        let mut form = DeleteForm {
            has_phrase: true,
            word: "Delete ".into(),
            ..DeleteForm::default()
        };
        assert!(!form.ready(), "from Settings the password is needed");
        form.password = Zeroizing::new("password".into());
        assert!(form.ready());
        form.has_phrase = false;
        assert!(!form.ready(), "and the phrase");

        let mut forgot = DeleteForm::forgot();
        forgot.has_phrase = true;
        forgot.word = "delet".into();
        assert!(!forgot.ready(), "the whole word");
        forgot.word = tr!("delete").into();
        assert!(forgot.ready(), "no password when it is forgotten");
    }

    #[test]
    fn done_only_when_the_core_deleted_it() {
        let mut form = DeleteForm::forgot();
        assert!(!form.on_done(&Err(CoreError::WrongPassword)));
        assert!(form.on_done(&Ok(())));
    }
}
