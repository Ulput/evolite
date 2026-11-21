//! "Password for every send" (BUG-42): the wallet password field in the last step of every
//! spend — a payment, a top-up, the pool, a name, a contact, a profile, an app's request. The
//! core keeps the setting ([`evolite_api::Wallet::password_each_send`]) and checks the password
//! before anything else ([`Command::Spend`]); a screen only asks for it and hands it over.

use super::prelude::*;
use super::secret_password::Gate;
use crate::screens::pause;
use evolite_api::{Command, CoreError, DoneWhat, Password, Snapshot, Spend, Timestamp};
use zeroize::{Zeroize, Zeroizing};

/// FEAT-0004 B: the open wallet's pause, set by the app once a frame for every spend's field.
pub fn set_pause(ctx: &egui::Context, gate: Gate) {
    ctx.data_mut(|d| d.insert_temp(pause_id(), gate));
}

/// The open wallet's pause as the app set it this frame — for a spend's own password field
/// too (the Core payment's last look builds its `Spend` itself).
pub fn spend_pause(ctx: &egui::Context) -> Gate {
    ctx.data(|d| d.get_temp(pause_id())).unwrap_or_default()
}

fn pause_id() -> egui::Id {
    egui::Id::new("spend-password-pause")
}

/// FEAT-0004 B: the open wallet's pause for the spends' fields — its snapshot's end, cut as the
/// core cuts it when the clock is set back (`cut`, kept by the app).
pub fn open_wallet_pause(snap: &Snapshot, cut: &mut pause::Cut, now: Timestamp) -> Gate {
    let open = snap.wallets.iter().find(|w| w.open);
    Gate::of(open, None, cut, now)
}

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
    /// The operation of the last [`Self::command`] ([`Spend::what`]), named in the "wrong
    /// password" line; none for the setting itself.
    what: Option<DoneWhat>,
    /// FEAT-0004 B: the end of the pause the core's last answer gave (its snapshot may come a
    /// frame later).
    refused: Option<Timestamp>,
    /// The wallet's password is paused (set by [`Self::field`] each frame it is asked).
    paused: bool,
    /// FEAT-0004 FR-7: what the watched spend sends, when it is more than the wallet's
    /// threshold on mainnet ([`Self::watch`]); the tick is asked then.
    large: Option<evolite_api::Duffs>,
    /// «I checked the amount and where it goes», for the spend watched as it is.
    ticked: bool,
    /// BUG-249: the pass the tick was drawn in a dialog's summary ([`Self::summary_tick`]): the
    /// field under it then leaves it out.
    tick_pass: Option<u64>,
    /// The spend watched last (its whole form: another amount or recipient takes the tick off,
    /// review r1 Should 1).
    watched: Option<String>,
    /// The last command's spend and what it sends, for a refusal by the core's own fees.
    last: Option<(String, evolite_api::Duffs)>,
    /// Review r1 Should 2: that spend, refused by the core for want of the tick though the
    /// form's fees put it under the threshold — the tick shown for it all the same.
    forced: Option<(String, evolite_api::Duffs)>,
}

/// FEAT-0004 FR-7: the words of the tick before a large spend.
pub fn tick_text() -> &'static str {
    tr!("I checked the amount and where it goes")
}

/// FEAT-0004 FR-7: what `op` sends when it asks the tick in `snap` — more than the wallet's
/// threshold, on mainnet — by the very function the core refuses by
/// ([`evolite_api::confirm::asks_tick`]).
pub fn large_spend(snap: &Snapshot, op: &Spend) -> Option<evolite_api::Duffs> {
    let wallet = snap.wallet.as_ref()?;
    let threshold = (snap.network == evolite_api::Network::Mainnet)
        .then_some(wallet.large_spend)
        .flatten();
    large_by(op, &wallet.fees, threshold)
}

fn large_by(
    op: &Spend,
    fees: &evolite_api::Fees,
    threshold: Option<evolite_api::Duffs>,
) -> Option<evolite_api::Duffs> {
    evolite_api::confirm::asks_tick(op, fees, threshold)
        .then(|| evolite_api::confirm::outgoing(op, fees))
        .flatten()
}

/// FEAT-0004 FR-7: the threshold that applies now (`None` off testnet or turned off) and the
/// wallet's fees, set by the app once a frame for every spend's flow; one per thread, as the
/// interface's language (a test's app is its own).
#[derive(Clone)]
pub struct LargeRule {
    pub threshold: Option<evolite_api::Duffs>,
    pub fees: evolite_api::Fees,
}

thread_local! {
    static RULE: std::cell::RefCell<Option<LargeRule>> = const { std::cell::RefCell::new(None) };
}

/// The rule of `snap`'s wallet, for this frame.
pub fn set_large_rule(snap: &Snapshot) {
    let rule = snap.wallet.as_ref().map(|w| LargeRule {
        threshold: (snap.network == evolite_api::Network::Mainnet)
            .then_some(w.large_spend)
            .flatten(),
        fees: w.fees.clone(),
    });
    RULE.with(|r| *r.borrow_mut() = rule);
}

/// Sets the rule directly (tests: a test's process is on testnet, where there is no threshold).
#[cfg(test)]
pub fn set_rule_for_test(rule: Option<LargeRule>) {
    RULE.with(|r| *r.borrow_mut() = rule);
}

/// The rule set this frame (`None` before a wallet, and in unit tests: no tick).
pub fn large_rule() -> Option<LargeRule> {
    RULE.with(|r| r.borrow().clone())
}

impl SpendPassword {
    /// The field under its label, when asked; `true` when Enter was pressed in it (the step's
    /// main button, as in "Check the payment"). In a dialog it is never `true`: the dialog's
    /// row of buttons presses its primary itself ([`super::button_row`]). `id`: unique on the
    /// screen.
    /// BUG-249 (the architect's condition 2): the FEAT-0004 tick in a dialog's summary, with
    /// what it agrees to, instead of over the password in the footer. [`Self::field`] then
    /// draws none.
    pub fn summary_tick(&mut self, ui: &mut Ui, p: &Palette) {
        self.tick_pass = Some(ui.ctx().cumulative_pass_nr());
        if self.large.is_some() {
            ui.checkbox(
                &mut self.ticked,
                egui::RichText::new(crate::i18n::t(tick_text()))
                    .font(sans(13.0, Weight::Regular))
                    .color(p.text),
            );
        }
    }

    pub fn field(&mut self, ui: &mut Ui, p: &Palette, id: &str, asked: bool) -> bool {
        // A spend in a dialog: the dialog has seen it (BUG-249's test of every spend dialog).
        super::note_spend(ui.ctx());
        // FEAT-0004 FR-7 (AC-T7): above the password, whether one is asked or not — unless the
        // summary over it has drawn the tick this pass.
        let in_summary = self.tick_pass == Some(ui.ctx().cumulative_pass_nr());
        if self.large.is_some() && !in_summary {
            ui.checkbox(
                &mut self.ticked,
                egui::RichText::new(crate::i18n::t(tick_text()))
                    .font(sans(13.0, Weight::Regular))
                    .color(p.text),
            );
        }
        if !asked {
            // AC-T2: a spend without its password is not held by a pause.
            self.paused = false;
            return false;
        }
        // FEAT-0004 B (AC-T3): the wallet's pause, as on the unlock screen: the plate with its
        // countdown under the field, the step's button not ready until it is over.
        let now = chrono::Local::now();
        let gate = spend_pause(ui.ctx());
        let own = pause::left(self.refused, now.timestamp());
        let left = gate.left.max(own);
        self.paused = left.is_some();
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
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && left.is_none();
            if let Some(secs) = left {
                let end = gate.end.max(self.refused).unwrap_or_default();
                pause::plate_plain(ui, *p, secs, end);
                pause::tick(ui, now);
            } else {
                if self.wrong {
                    ui.label(
                        egui::RichText::new(crate::i18n::t(wrong_text(self.what)))
                            .font(sans(12.0, Weight::Regular))
                            .color(p.danger),
                    );
                }
                // FEAT-0004: the warning before a pause and the line after one, as on the
                // unlock screen — the count is the wallet's, whichever form it was made in.
                super::secret_password::tries_lines_plain(ui, p, gate.wrong_tries);
            }
        });
        entered
    }

    /// Types `text` into the field (tests).
    #[cfg(test)]
    pub fn type_in(&mut self, text: &str) {
        self.typed.push_str(text);
    }

    /// The step's main button may go: nothing is asked, or something is typed and the wallet's
    /// password is not paused (FEAT-0004 B); a large spend ticked (FR-7).
    pub fn ready(&self, asked: bool) -> bool {
        (!asked || (!self.typed.is_empty() && !self.paused))
            && (self.large.is_none() || self.ticked)
    }

    /// FEAT-0004 FR-7 (AC-T7): the spend the step would send now, each frame before
    /// [`Self::field`] — the tick shows for one of more than the threshold. Another amount takes
    /// the tick off; one under the threshold hides it.
    pub fn watch(&mut self, op: Option<&Spend>) {
        let key = op.map(|op| format!("{op:?}"));
        let mut large = op
            .zip(large_rule())
            .and_then(|(op, rule)| large_by(op, &rule.fees, rule.threshold));
        // Should 2: the spend the core refused for want of the tick asks it, whatever the
        // form's fees say.
        if let Some((forced, out)) = &self.forced
            && key.as_ref() == Some(forced)
        {
            large = large.or(Some(*out));
        }
        // Should 1: another spend — amount or recipient — takes the tick off.
        if large != self.large || key != self.watched {
            self.ticked = false;
        }
        self.watched = key;
        self.large = large;
    }

    /// FEAT-0004 FR-7: a large spend watched and not ticked: it is not sent.
    pub fn waits_for_tick(&self) -> bool {
        self.large.is_some() && !self.ticked
    }

    /// Ticks the tick (tests).
    #[cfg(test)]
    pub fn tick(&mut self) {
        self.ticked = true;
    }

    /// Whether the tick is shown (tests).
    #[cfg(test)]
    pub fn asks_tick(&self) -> bool {
        self.large.is_some()
    }

    /// The command for `op`, with a copy of the password when asked (the copy is wiped by the
    /// core with the command).
    pub fn command(&mut self, op: Spend, asked: bool) -> Command {
        self.what = Some(op.what());
        let out = large_rule()
            .and_then(|rule| evolite_api::confirm::outgoing(&op, &rule.fees))
            .unwrap_or_default();
        self.last = Some((format!("{op:?}"), out));
        Command::Spend {
            op,
            password: self.password(asked),
            ask: 0,
            large_confirmed: self.large.is_some() && self.ticked,
        }
    }

    /// A copy of the typed password for a command, when asked (turning the setting off takes
    /// it too); its answer is awaited from now on.
    pub fn password(&mut self, asked: bool) -> Option<Password> {
        asked.then(|| Password::new(self.typed.to_string()))
    }

    /// The core's answer to the spend: a wrong password empties the field and says so under it
    /// (TECH-spend-flow review U3: typed again, never sent twice as it was); anything else
    /// forgets it.
    pub fn on_answer<T>(&mut self, result: &Result<T, CoreError>) {
        match result {
            Err(CoreError::WrongPassword) => self.on_wrong(),
            // FEAT-0004 B: the pause, at once — its snapshot may come a frame later.
            Err(CoreError::TooManyTries { until }) => {
                self.forget();
                self.refused = Some(*until);
            }
            // Review r1 Should 2: the core counts with its own fees; refused for want of the
            // tick, the last spend asks it now (the password kept, nothing was checked).
            Err(CoreError::LargeSpendNotConfirmed) => {
                self.forced = self.last.clone();
                self.large = self.last.as_ref().map(|(_, out)| *out);
                self.watched = self.last.as_ref().map(|(key, _)| key.clone());
                self.ticked = false;
            }
            _ => self.forget(),
        }
    }

    /// The core said the password was wrong: the field empty, the line under it shown.
    pub fn on_wrong(&mut self) {
        self.typed.zeroize();
        self.wrong = true;
    }

    /// Wipes what was typed (cancel, lock, the step closed), and the tick.
    pub fn forget(&mut self) {
        self.typed.zeroize();
        self.wrong = false;
        self.ticked = false;
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
pub fn wrong_text(what: Option<DoneWhat>) -> &'static str {
    match what {
        Some(DoneWhat::Send) => tr!("Wrong password — the payment was not sent."),
        Some(DoneWhat::TopUp) => tr!("Wrong password — the top-up did not happen."),
        Some(DoneWhat::FundAddress) => {
            tr!("Wrong password — the Platform address was not topped up.")
        }
        Some(DoneWhat::Identity) => tr!("Wrong password — the identity was not created."),
        Some(DoneWhat::Name) => tr!("Wrong password — the name was not registered."),
        Some(DoneWhat::Shield) => tr!("Wrong password — nothing went into the pool."),
        Some(DoneWhat::Unshield | DoneWhat::PoolTopUp) => {
            tr!("Wrong password — nothing came out of the pool.")
        }
        Some(DoneWhat::Contact) => tr!("Wrong password — the contact request was not sent."),
        Some(DoneWhat::Profile) => tr!("Wrong password — the profile was not saved."),
        Some(DoneWhat::App) => tr!("Wrong password — the app's request was not signed."),
        Some(DoneWhat::Gift) => tr!("Wrong password — the NFT was not given."),
        Some(DoneWhat::Disconnect) => tr!("Wrong password — the app was not disconnected."),
        Some(DoneWhat::Keys) => tr!("Wrong password — the keys did not change."),
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
            coins: None,
            fee_rate: None,
            fee_shown: None,
            id: 0,
            from: Source::Core,
            to: "@alice".into(),
            amount: 1,
            note: None,
            rate: None,
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
        let Command::Spend { password, op, .. } = form.command(send(), true) else {
            panic!()
        };
        assert_eq!(password.unwrap().expose(), "hunter2");
        assert_eq!(op, send());
    }

    /// TECH-spend-flow review U3: a wrong password empties the field (typed again, never sent
    /// twice as it was) and says so; anything else forgets it.
    #[test]
    fn a_wrong_password_is_typed_again_and_anything_else_forgets_it() {
        let mut form = SpendPassword::default();
        form.typed.push_str("hunter3");
        form.on_answer::<()>(&Err(CoreError::WrongPassword));
        assert!(form.is_wrong() && !form.ready(true));
        form.typed.push_str("hunter2");
        form.on_answer::<()>(&Err(CoreError::NotEnough));
        assert!(!form.is_wrong() && !form.ready(true));
        form.typed.push_str("hunter2");
        form.on_answer(&Ok(()));
        assert!(!form.ready(true));
    }

    /// FEAT-0004 B: «too many wrong passwords» keeps its end for the plate (its snapshot may
    /// come a frame later) and empties the field; in a pause the step's button is not ready even
    /// with a password typed (AC-T3); a spend that asks none is never held (AC-T2).
    #[test]
    fn a_pause_holds_only_a_spend_that_asks_the_password() {
        let mut form = SpendPassword::default();
        form.typed.push_str("hunter2");
        form.on_answer::<()>(&Err(CoreError::TooManyTries {
            until: 1_800_000_030,
        }));
        assert_eq!(form.refused, Some(1_800_000_030));
        assert!(!form.ready(true), "the field emptied");
        form.typed.push_str("hunter2");
        form.paused = true;
        assert!(!form.ready(true), "paused");
        assert!(form.ready(false), "no password asked: not held");
        form.paused = false;
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
            Spend::FundAddress { amount: 1 },
            Spend::ResumeFundAddress,
            Spend::CreateIdentity {
                name: Some("a".into()),
                top_up: 1,
            },
            Spend::ResumeIdentity,
            Spend::AddName {
                name: "a".into(),
                contest_fund: None,
                link: None,
            },
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
            Spend::ApproveApp { id: 1, grant: None },
            Spend::GiftNft {
                id: "a".into(),
                to: "@a".into(),
            },
            Spend::DisconnectApp { id: "app".into() },
        ];
        let general = wrong_text(Some(DoneWhat::Lock));
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
