//! "Password for every send" (BUG-42): the wallet password field in the last step of every
//! spend — a payment, a top-up, the pool, a name, a contact, a profile, an app's request. The
//! core keeps the setting ([`evolite_api::Wallet::password_each_send`]) and checks the password
//! before anything else ([`Command::Spend`]); a screen only asks for it and hands it over.

use super::prelude::*;
use evolite_api::{Command, CoreError, Password, Snapshot, Spend};
use zeroize::{Zeroize, Zeroizing};

/// Whether the shown wallet wants its password for every spend.
pub fn password_asked(snap: &Snapshot) -> bool {
    snap.wallet.as_ref().is_some_and(|w| w.password_each_send)
}

/// The typed password of one spend form. Kept until the core answers, so a wrong one can be
/// corrected rather than typed again; wiped on success, on cancel and on lock.
#[derive(Default)]
pub struct SpendPassword {
    typed: Zeroizing<String>,
    /// The core said the last one was wrong.
    wrong: bool,
    /// A spend with this password went to the core; its answer is awaited ([`Self::settle`]).
    pending: bool,
    /// The operation of the last [`Self::command`] ([`Spend::what`]), named in the "wrong
    /// password" line; none for the setting itself.
    what: Option<&'static str>,
}

impl SpendPassword {
    /// The field under its label, when asked; `true` when Enter was pressed in it (the step's
    /// main button, as in "Check the payment"). In a dialog it is never `true`: the dialog's
    /// row of buttons presses its primary itself ([`super::button_row`]). `id`: unique on the
    /// screen.
    pub fn field(&mut self, ui: &mut Ui, p: &Palette, id: &str, asked: bool) -> bool {
        if !asked {
            return false;
        }
        let mut entered = false;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field_label(ui, p, tr!("Wallet password"));
            let field = text_field(ui, p, id, &mut self.typed, "", FieldKind::Password, |_| {});
            if field.changed() {
                self.wrong = false;
            }
            entered = !crate::a11y::in_dialog(ui.ctx())
                && field.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if self.wrong {
                ui.label(
                    egui::RichText::new(crate::i18n::t(wrong_text(self.what)))
                        .font(sans(12.0, Weight::Regular))
                        .color(p.danger),
                );
            }
        });
        entered
    }

    /// The step's main button may go: nothing is asked, or something is typed.
    pub fn ready(&self, asked: bool) -> bool {
        !asked || !self.typed.is_empty()
    }

    /// The command for `op`, with a copy of the password when asked (the copy is wiped by the
    /// core with the command).
    pub fn command(&mut self, op: Spend, asked: bool) -> Command {
        self.what = Some(op.what());
        Command::Spend {
            op,
            password: self.password(asked),
        }
    }

    /// A copy of the typed password for a command, when asked (turning the setting off takes
    /// it too); its answer is awaited from now on.
    pub fn password(&mut self, asked: bool) -> Option<Password> {
        self.pending = true;
        asked.then(|| Password::new(self.typed.to_string()))
    }

    /// Each frame, with the answer the screen keeps for its spend: the first answer after
    /// [`Self::command`] goes to [`Self::on_answer`]. A screen whose answer is routed to its
    /// state may call that directly instead.
    pub fn settle<T>(&mut self, answer: Option<&Result<T, CoreError>>) {
        if self.pending
            && let Some(result) = answer
        {
            self.on_answer(result);
        }
    }

    /// The core's answer to the spend: a wrong password keeps the typing for a correction and
    /// says so under the field; anything else forgets it.
    pub fn on_answer<T>(&mut self, result: &Result<T, CoreError>) {
        self.pending = false;
        if matches!(result, Err(CoreError::WrongPassword)) {
            self.wrong = true;
        } else {
            self.forget();
        }
    }

    /// Wipes what was typed (cancel, lock, the step closed).
    pub fn forget(&mut self) {
        self.typed.zeroize();
        self.wrong = false;
        self.pending = false;
    }

    /// The last answer was a wrong password (shown under the field; a screen leaves out its
    /// own error line then).
    pub fn is_wrong(&self) -> bool {
        self.wrong
    }
}

/// Under the field after a wrong password: nothing was spent — said of the operation itself
/// (BUG-42, the analyst: «Неверный пароль — <операция> не выполнена.»), a whole sentence each,
/// so that every language can agree its words. `what`: [`Spend::what`]; none — the setting.
pub fn wrong_text(what: Option<&str>) -> &'static str {
    match what {
        Some("send") => tr!("Wrong password — the payment was not sent."),
        Some("top-up") => tr!("Wrong password — the top-up did not happen."),
        Some("identity") => tr!("Wrong password — the identity was not created."),
        Some("name") => tr!("Wrong password — the name was not registered."),
        Some("shield") => tr!("Wrong password — nothing went into the pool."),
        Some("unshield" | "pool-top-up") => tr!("Wrong password — nothing came out of the pool."),
        Some("contact") => tr!("Wrong password — the contact request was not sent."),
        Some("profile") => tr!("Wrong password — the profile was not saved."),
        Some("app") => tr!("Wrong password — the app's request was not signed."),
        Some("gift") => tr!("Wrong password — the NFT was not given."),
        None => tr!("Wrong password — the setting stays on."),
        Some(_) => tr!("Wrong password — the operation did not happen."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::Source;

    fn send() -> Spend {
        Spend::Send {
            from: Source::Core,
            to: "@alice".into(),
            amount: 1,
            note: None,
        }
    }

    #[test]
    fn asks_only_when_the_setting_is_on() {
        let mut form = SpendPassword::default();
        assert!(form.ready(false) && !form.ready(true));
        let Command::Spend { password, .. } = form.command(send(), false) else {
            panic!()
        };
        assert!(password.is_none());
        form.typed.push_str("hunter2");
        assert!(form.ready(true));
        let Command::Spend { password, op } = form.command(send(), true) else {
            panic!()
        };
        assert_eq!(password.unwrap().expose(), "hunter2");
        assert_eq!(op, send());
    }

    #[test]
    fn a_wrong_password_stays_to_be_corrected_and_anything_else_forgets_it() {
        let mut form = SpendPassword::default();
        form.typed.push_str("hunter3");
        form.on_answer::<()>(&Err(CoreError::WrongPassword));
        assert!(form.is_wrong() && form.ready(true));
        form.on_answer::<()>(&Err(CoreError::NotEnough));
        assert!(!form.is_wrong() && !form.ready(true));
        form.typed.push_str("hunter2");
        form.on_answer(&Ok(()));
        assert!(!form.ready(true));
    }

    #[test]
    fn only_the_answer_after_a_spend_settles_it() {
        let mut form = SpendPassword::default();
        form.typed.push_str("hunter2");
        // An answer from before (a result still on screen) leaves the typing alone.
        form.settle::<()>(Some(&Err(CoreError::WrongPassword)));
        assert!(!form.is_wrong() && form.ready(true));
        let _ = form.command(send(), true);
        form.settle::<()>(None);
        assert!(form.ready(true));
        form.settle::<()>(Some(&Err(CoreError::WrongPassword)));
        assert!(form.is_wrong());
        // Settled once: the same answer next frame changes nothing.
        form.typed.push('x');
        form.settle::<()>(Some(&Ok(())));
        assert!(form.ready(true));
    }

    /// The analyst: «Неверный пароль — <операция> не выполнена.» — every spend is named, none
    /// falls to the general line, and the line says the operation of the last command.
    #[test]
    fn a_wrong_password_names_the_operation() {
        let every = [
            send(),
            Spend::TopUpIdentity { amount: 1 },
            Spend::ResumeTopUp,
            Spend::CreateIdentity {
                name: "a".into(),
                top_up: 1,
            },
            Spend::ResumeIdentity,
            Spend::AddName { name: "a".into() },
            Spend::Shield {
                from: evolite_api::ShieldSource::Core,
                amount: 1,
            },
            Spend::Unshield { amount: 1 },
            Spend::TopUpFromPool { amount: 1 },
            Spend::AddContact { name: "a".into() },
            Spend::AcceptContact {
                identity_id: "a".into(),
            },
            Spend::SaveProfile {
                display: "a".into(),
                about: String::new(),
                avatar_url: None,
            },
            Spend::ApproveApp { id: 1 },
            Spend::GiftNft {
                id: "a".into(),
                to: "@a".into(),
            },
        ];
        let general = wrong_text(Some("?"));
        for op in every {
            let text = wrong_text(Some(op.what()));
            assert_ne!(text, general, "{} is not named", op.what());
            assert_ne!(text, wrong_text(None), "{}", op.what());
            let mut form = SpendPassword::default();
            let _ = form.command(op, true);
            assert_eq!(form.what.map(|w| wrong_text(Some(w))), Some(text));
        }
        let mut setting = SpendPassword::default();
        let _ = setting.password(true);
        assert_eq!(wrong_text(setting.what), wrong_text(None));
    }
}
