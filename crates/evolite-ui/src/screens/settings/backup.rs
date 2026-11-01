//! Backup: the phrase on paper, and 17 · Show the phrase behind the password.

use super::parts::{Control, group, heading};
use super::txt;
use super::{PhraseCheckForm, SettingsState};
use crate::components::{
    Assets, ButtonKind, FieldKind, button, button_enabled, dialog, field_label, notice, pill,
    text_field,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::Weight;
use egui::{Align, Layout, Ui};
use evolite_api::{Command, CoreError, Password, Wallet};
use std::time::Duration;
use zeroize::Zeroize;

/// How long a revealed phrase stays on screen.
const SHOWN_FOR: Duration = Duration::from_secs(60);

pub(super) fn backup(ui: &mut Ui, a: &mut Assets, state: &mut SettingsState, wallet: &Wallet) {
    let phrase_line = format!(
        tr!("{} words · made {}"),
        wallet.phrase_words,
        text::date_long(wallet.created_at)
    );
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
                Control::Button(tr!("Check"), ButtonKind::Ghost),
            ),
            (
                tr!("Remind me to check"),
                tr!("Three months after the check — on the Overview"),
                Control::Toggle(&mut remind),
            ),
        ],
    );
    *state.toggle("remind-check") = remind;
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
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        if button(ui, a, tr!("Try again"), ButtonKind::Primary, None).clicked() {
                            form.verdict = None;
                        }
                        close = button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked();
                    });
                }
                (None, Some(questions)) => {
                    ui.label(txt(
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
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
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
                        close = button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked();
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
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
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
                        close = button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked();
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
    if state.revealed.is_some() {
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
    let closed = dialog(
        ui,
        a,
        "phrase-dialog",
        "seed",
        tr!("Show the phrase"),
        442.0,
        |ui, a| {
            let mut close = false;
            notice(
                ui,
                a,
                "warning",
                p.muted,
                tr!(
                    "Make sure nobody sees or records the screen. evolite never asks for the phrase — not in a chat, by mail or in apps."
                ),
            );
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
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
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
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close
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
