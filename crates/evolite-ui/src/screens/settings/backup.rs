//! Backup: the phrase on paper, and 17 · Show the phrase behind the password.

use super::parts::{Control, group, heading};
use super::txt;
use super::{PhraseCheckForm, SettingsState};
use crate::components::{
    Assets, ButtonKind, FieldKind, Part, button, button_enabled, button_row, dialog,
    dialog_with_footer, field_label, notice, pill, text_field,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::Weight;
use egui::{Align, Layout, Ui};
use evolite_api::{Command, CoreError, Password, Timestamp, Wallet, WalletOrigin};
use std::time::Duration;
use zeroize::Zeroize;

/// How long a revealed phrase stays on screen.
const SHOWN_FOR: Duration = Duration::from_secs(60);

/// The note under "Recovery phrase" (BUG-58): what the phrase brings back, and what stays on
/// this computer only (also in "Delete the wallet from the computer"). The designer's text of
/// 2026-11-06 (BUG-58.md, «1a»), as it stands until FEAT-0012 (tax categories) and FEAT-0015
/// (Platform address operations) are in: without them, each its own key when they come.
const WHAT_COMES_BACK: &str = tr!("What the phrase brings back");
const COMES_BACK: &str = tr!(
    "Core money, credits and the private pool; identities and names; DashPay contacts and profile. evolite reads again from the network the history of Core payments, names, contact requests, the profile, withdrawals and the pool."
);
/// FEAT-0015 R-9 (frame 9): Platform keeps only the balance of an address, so a restore brings back
/// the money on the Platform addresses but not their earlier operations.
const ADDRESS_BACK: &str = tr!(
    "Its balance comes back, but not its earlier operations: the network keeps only the balance of an address."
);
/// BUG-130 (FEAT-0019 R-1, AC-A6): the token list is this computer's; a restore finds again only
/// the tokens with a balance.
const TOKENS_HERE: &str = tr!(
    "The token list is kept on this computer. After a restore from the phrase, the search finds the tokens with a balance again, hidden ones included: hide them again if needed. Tokens added by hand with a zero balance do not come back — add them again by id."
);
pub const KEPT_HERE: &str = tr!("Kept only on this computer");
pub const KEPT_HERE_TEXT: &str = tr!(
    "Payment notes, the address book and settings, as well as records of credit transfers, app sign-ins and exact Platform fees. The phrase does not bring them back."
);
/// Before [`KEPT_HERE`] in "Delete the wallet from the computer": what does come back.
pub const STAYS_ON_NETWORK: &str =
    tr!("Money, identities, names and contacts stay on the network: the phrase brings them back.");

/// The line under "Show the phrase" (BUG-59): its words and the one date the vault has — when
/// the wallet came to this computer. A phrase keeps no date of its own, so for a wallet restored
/// (or imported from DashPay) from a phrase made elsewhere that date is when it was restored,
/// never a "made" one. `origin`: from the wallet list; not found there, as before.
fn phrase_line(words: usize, at: Timestamp, origin: Option<WalletOrigin>) -> String {
    let date = text::date_long(at);
    // «24 слова», not «24 слов»: the number of words takes its own plural form (CLDR).
    let words = text::plural(words as u32, tr!("word"), tr!("words"));
    match origin {
        Some(WalletOrigin::Restored) => {
            format!(tr!("{} · restored on this computer {}"), words, date)
        }
        Some(WalletOrigin::DashPay) => {
            format!(tr!("{} · imported from DashPay {}"), words, date)
        }
        Some(WalletOrigin::Created) | None => format!(tr!("{} · made {}"), words, date),
    }
}

pub(super) fn backup(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut SettingsState,
    wallet: &Wallet,
    origin: Option<WalletOrigin>,
) {
    let phrase_line = phrase_line(wallet.phrase_words, wallet.created_at, origin);
    let p = a.p;
    heading(
        ui,
        &p,
        tr!("Backup"),
        Some(tr!(
            "The phrase is the only way to get the wallet back if the computer breaks or is lost."
        )),
    );
    let mut remind = *state.toggle("remind-check");
    let checked_line = match state.phrase_checked {
        Some(at) => format!(tr!("Checked {}"), text::date_long(at)),
        None => tr!("Pick the words by number — this shows the record is right").to_string(),
    };
    let clicked = group(
        ui,
        a,
        tr!("Recovery phrase"),
        vec![
            (
                tr!("Show the phrase"),
                &phrase_line,
                Control::Button(tr!("Show"), ButtonKind::Secondary),
            ),
            (
                tr!("Check the phrase"),
                &checked_line,
                // The same kind as "Show" beside it: two actions of one card look alike
                // (designer review №18).
                Control::Button(tr!("Check"), ButtonKind::Secondary),
            ),
            (
                tr!("Remind me to check"),
                tr!("Three months after the check — on the Overview"),
                Control::Toggle(&mut remind),
            ),
        ],
    );
    *state.toggle("remind-check") = remind;
    // What a restore brings back and what it does not (BUG-58, the designer's text; the one
    // without tax categories until FEAT-0012).
    ui.add_space(12.0);
    crate::components::notice_parts(
        ui,
        a,
        "info",
        p.muted,
        &[
            (WHAT_COMES_BACK, COMES_BACK),
            (tr!("Platform address"), ADDRESS_BACK),
            (tr!("Tokens"), TOKENS_HERE),
            (KEPT_HERE, KEPT_HERE_TEXT),
        ],
    );
    match clicked {
        Some(0) => state.phrase_dialog = true,
        Some(1) => state.check = Some(PhraseCheckForm::default()),
        _ => {}
    }
}

/// · "Remind me to check": on Overview, three months after the last right check (or the
/// wallet's creation), a line with "Check" and "Remind me later" (a week). `show`: the
/// `--screen remind` preset, whatever the dates.
pub fn check_reminder(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut SettingsState,
    show: bool,
) {
    let now = cx.now.timestamp();
    let wallet = cx.wallet();
    if !show && (crate::is_demo(cx.snap) || !state.remind_check(wallet.created_at, now)) {
        return;
    }
    let p = a.p;
    let line = match state.phrase_checked {
        Some(at) => format!(
            tr!("The phrase was last checked {}. Check it again: without it the wallet cannot be recovered."),
            text::date_long(at)
        ),
        None => tr!("The phrase has not been checked yet. Check your record: without it the wallet cannot be recovered.").into(),
    };
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        notice(ui, a, "seed", p.accent, &line);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if button(ui, a, tr!("Check the phrase"), ButtonKind::Secondary, None).clicked() {
                state.section = super::Section::Backup;
                state.check = Some(PhraseCheckForm::default());
                cx.go(crate::screens::Screen::Settings);
            }
            if button(ui, a, tr!("Remind me in a week"), ButtonKind::Ghost, None).clicked() {
                state.remind_snoozed = Some(now + super::SNOOZE);
            }
        });
        ui.add_space(6.0);
    });
}

/// · Check the phrase: the password, then four words of the phrase by number, each picked
/// from three. The core asks and checks; the phrase never comes here, and a wrong word is
/// named by its number only.
pub(super) fn check_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut SettingsState) {
    let p = a.p;
    let Some(form) = &mut state.check else {
        return;
    };
    // Words of the phrase are typed here.
    crate::capture::keep_out();
    let mut ask = std::mem::take(&mut form.ask_on_open) && !form.password.is_empty();
    let mut check = None;
    let closed = dialog(
        ui,
        a,
        "check-phrase",
        "seed",
        tr!("Check the phrase"),
        462.0,
        |ui, a| {
            let mut close = false;
            match (&form.verdict, &form.questions) {
                (Some(None), _) => {
                    notice(
                        ui,
                        a,
                        "confirmed",
                        p.accent,
                        tr!(
                            "All correct: the phrase is written down right. Keep the record where it is."
                        ),
                    );
                    button_row(ui, |ui| {
                        close = button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked();
                    });
                }
                (Some(Some(number)), _) => {
                    notice(
                        ui,
                        a,
                        "warning",
                        p.danger,
                        &format!(
                            tr!("Word no. {number} did not match. Find your record and try again."),
                            number = number
                        ),
                    );
                    button_row(ui, |ui| {
                        close = button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked();
                        if button(ui, a, tr!("Try again"), ButtonKind::Primary, None).clicked() {
                            form.verdict = None;
                        }
                    });
                }
                (None, Some(questions)) => {
                    ui.add(crate::components::para(
                        tr!("Pick the words at these numbers. evolite does not show the phrase itself."),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    for (q, question) in questions.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.allocate_ui_with_layout(
                                egui::vec2(92.0, 30.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.set_min_width(92.0);
                                    ui.label(txt(
                                        &format!(tr!("Word no. {}"), question.number),
                                        13.0,
                                        Weight::SemiBold,
                                        p.text,
                                    ));
                                },
                            );
                            for (i, word) in question.choices.iter().enumerate() {
                                let on = form.picks.get(q).copied().flatten() == Some(i);
                                if pill(ui, &p, word, on).clicked()
                                    && let Some(pick) = form.picks.get_mut(q)
                                {
                                    *pick = Some(i);
                                }
                            }
                        });
                    }
                    button_row(ui, |ui| {
                        close = button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked();
                        let all = form.picks.iter().all(Option::is_some);
                        let label = if form.waiting {
                            tr!("Checking…")
                        } else {
                            tr!("Check")
                        };
                        if button_enabled(
                            ui,
                            a,
                            label,
                            ButtonKind::Primary,
                            None,
                            all && !form.waiting,
                        )
                        .clicked()
                        {
                            check = Some(
                                questions
                                    .iter()
                                    .zip(&form.picks)
                                    .map(|(q, pick)| {
                                        pick.map(|i| q.choices[i].clone()).unwrap_or_default()
                                    })
                                    .collect::<Vec<_>>(),
                            );
                        }
                    });
                }
                (None, None) => {
                    notice(
                        ui,
                        a,
                        "info",
                        p.muted,
                        tr!(
                            "evolite asks for four words of your phrase by number — pick each from three choices. The phrase itself is never shown."
                        ),
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        field_label(ui, &p, tr!("Wallet password"));
                        if text_field(
                            ui,
                            &p,
                            "check-password",
                            &mut form.password,
                            "",
                            FieldKind::Password,
                            |_| {},
                        )
                        .changed()
                        {
                            form.error = None;
                        }
                        let line = match &form.error {
                            Some(CoreError::WrongPassword) => {
                                Some(tr!("Wrong password").to_string())
                            }
                            Some(CoreError::NotAvailable(why)) => Some(format!("{why}.")),
                            Some(_) => Some(tr!("The vault did not open").to_string()),
                            None => None,
                        };
                        if let Some(line) = line {
                            ui.label(txt(&line, 12.0, Weight::Regular, p.danger));
                        }
                    });
                    button_row(ui, |ui| {
                        close = button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked();
                        let label = if form.waiting {
                            tr!("Opening…")
                        } else {
                            tr!("Start")
                        };
                        ask |= button_enabled(
                            ui,
                            a,
                            label,
                            ButtonKind::Primary,
                            None,
                            !form.password.is_empty() && !form.waiting,
                        )
                        .clicked();
                    });
                }
            }
            close
        },
    );
    if ask {
        cx.send(Command::AskPhraseCheck {
            password: Password::new(form.password.to_string()),
        });
        form.password.zeroize();
        form.waiting = true;
        form.error = None;
    }
    if let Some(picks) = check {
        cx.send(Command::CheckPhraseWords { picks });
        form.waiting = true;
    }
    if closed && !form.waiting {
        form.password.zeroize();
        state.check = None;
    }
}

/// 17 · Show the phrase: a warning, the wallet password, "Show for 60 seconds".
pub(super) fn phrase_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut SettingsState) {
    let p = a.p;
    if words_shown(state) {
        crate::capture::keep_out();
    }
    // "For 60 seconds": then the phrase is gone from the screen and from memory here.
    let left = state
        .revealed_at
        .map(|at| SHOWN_FOR.saturating_sub(at.elapsed()));
    if left.is_some_and(|l| l.is_zero()) {
        state.revealed = None;
        state.revealed_at = None;
    } else if left.is_some() {
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }
    let closed = dialog_with_footer(
        ui,
        a,
        "phrase-dialog",
        "seed",
        tr!("Show the phrase"),
        442.0,
        |ui, a, part| {
            if part == Part::Footer {
                // The footer (BUG-122): always in the window.
                let mut close = false;
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    // another wallet's phrase asks that wallet's password.
                    let other = state
                        .phrase_wallet
                        .as_ref()
                        .and_then(|id| cx.snap.wallets.iter().find(|w| &w.id == id && !w.open));
                    match other {
                        Some(w) => field_label(
                            ui,
                            &p,
                            &format!(tr!("Password of “{}”"), crate::i18n::own(&w.name)),
                        ),
                        None => field_label(ui, &p, tr!("Wallet password")),
                    }
                    if text_field(
                        ui,
                        &p,
                        "phrase-password",
                        &mut state.password,
                        "",
                        FieldKind::Password,
                        |_| {},
                    )
                    .changed()
                    {
                        state.revealed = None;
                        state.revealed_at = None;
                    }
                    let answer = match &state.revealed {
                        Some(Err(CoreError::NotAvailable(why))) => {
                            Some((format!(tr!("{why}. No phrase."), why = why), p.muted))
                        }
                        Some(Err(CoreError::WrongPassword)) => {
                            Some((tr!("Wrong password").to_string(), p.danger))
                        }
                        Some(Err(e)) => {
                            Some((crate::text::failed(tr!("It did not work"), e), p.danger))
                        }
                        Some(Ok(phrase)) => Some((phrase.words().join(" "), p.text)),
                        None => None,
                    };
                    if let Some((line, color)) = answer {
                        ui.label(txt(&line, 12.0, Weight::Regular, color));
                    }
                    if let Some(left) = left.filter(|l| !l.is_zero()) {
                        ui.label(txt(
                            &format!(tr!("Hides in {} s"), left.as_secs() + 1),
                            12.0,
                            Weight::Regular,
                            p.muted,
                        ));
                    }
                });
                button_row(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let ready = !state.password.is_empty();
                    if button_enabled(
                        ui,
                        a,
                        tr!("Show for 60 seconds"),
                        ButtonKind::Primary,
                        None,
                        ready,
                    )
                    .clicked()
                    {
                        cx.send(Command::RevealPhrase {
                            password: Password::new(state.password.to_string()),
                            wallet: state.phrase_wallet.clone(),
                        });
                        state.revealed = None;
                        state.revealed_at = None;
                    }
                });
                return close;
            }
            notice(
                ui,
                a,
                "warning",
                p.muted,
                tr!(
                    "Make sure nobody sees or records the screen. evolite never asks for the phrase — not in a chat, by mail or in apps."
                ),
            );
            false
        },
    );
    if closed {
        state.phrase_dialog = false;
        state.phrase_wallet = None;
        // Neither the phrase nor the password outlives the dialog.
        state.revealed = None;
        state.revealed_at = None;
        state.password.zeroize();
    }
}

/// The words of the phrase are on screen: only then is the window kept out of captures. A
/// wrong password leaves an answer, not the phrase — the window stays in a screen share, so the
/// other side sees the error too (BUG-38).
fn words_shown(state: &SettingsState) -> bool {
    matches!(state.revealed, Some(Ok(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::Phrase;

    #[test]
    fn only_the_words_keep_the_window_out_of_captures() {
        let mut state = SettingsState::new(true, super::super::Section::Backup, false);
        assert!(!words_shown(&state));
        state.on_revealed(Err(CoreError::WrongPassword));
        assert!(!words_shown(&state), "a wrong password shows no words");
        state.on_revealed(Ok(Phrase::new(vec!["abandon".into(); 12])));
        assert!(words_shown(&state));
    }

    /// BUG-59: a restored wallet's date is when it was restored here; only a wallet made here
    /// says "made".
    #[test]
    fn a_restored_phrase_is_not_said_to_be_made_on_its_restore_day() {
        let at = 1_793_000_000;
        let day = text::date_long(at);
        let made = phrase_line(12, at, Some(WalletOrigin::Created));
        assert_eq!(made, format!("12 words · made {day}"));
        let restored = phrase_line(12, at, Some(WalletOrigin::Restored));
        assert_eq!(
            restored,
            format!("12 words · restored on this computer {day}")
        );
        let imported = phrase_line(24, at, Some(WalletOrigin::DashPay));
        assert_eq!(imported, format!("24 words · imported from DashPay {day}"));
        assert!(!restored.contains("made") && !imported.contains("made"));
        assert_eq!(
            phrase_line(12, at, None),
            made,
            "not in the list: as before"
        );
    }

    /// The number of words takes its plural form: «12 слов», «24 слова» in Russian, «12 слів»,
    /// «24 слова» in Ukrainian (it was «24 слов»), with the rest of the line translated.
    #[test]
    fn the_number_of_words_takes_its_plural_form() {
        use crate::i18n::{Lang, t_in};
        let at = 1_793_000_000;
        for (lang, made, twelve, twenty_four) in [
            (Lang::Ru, "создана", "12 слов", "24 слова"),
            (Lang::Uk, "створено", "12 слів", "24 слова"),
        ] {
            for (n, words) in [(12, twelve), (24, twenty_four)] {
                let english = phrase_line(n, at, Some(WalletOrigin::Created));
                let line = t_in(lang, &english);
                assert!(
                    line.starts_with(&format!("{words} · {made} ")),
                    "{lang:?} {n}: «{line}»"
                );
            }
        }
        assert!(
            t_in(Lang::Ru, &phrase_line(24, at, Some(WalletOrigin::Restored)))
                .starts_with("24 слова · восстановлена на этом компьютере ")
        );
    }
}
