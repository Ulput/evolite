//! The tokens screen's dialogs: «Send a token» (T6, T6b), a dialog of its own and not in
//! `send/*` (architect: FEAT-0013 and FEAT-0035 hold it), with the refusals before the password
//! (R-3, AC-T3); and «Add a token» (UC-1: by a token's or a contract's id, or a word), in the
//! spirit of the other dialogs (design review, «Добавить токен»).

use super::{TokensState, id_line, issuer, sender, shown_name, state};
use crate::components::{
    Assets, ButtonKind, FieldKind, SpendPassword, button, button_enabled, button_row,
    button_row_spend, card, dialog, field_label, kv_row, notice, password_asked, text_field, txt,
};
use crate::context::Ctx;
use crate::theme::{Weight, mono, sans};
use egui::{Align, Layout, Margin, Sense, Ui, vec2};
use evolite_api::tokens::{TokenInfo, TokenRefusal, TokenRow};
use evolite_api::{Command, CoreError, ResolvedName, Spend};

/// «Send a token»: one token, two steps.
pub struct SendState {
    pub token_id: String,
    pub to: String,
    pub amount: String,
    /// The second step, «Check the transfer», with the password.
    pub review: bool,
    pub password: SpendPassword,
    /// Sent, waiting for the core's answer.
    pub waiting: bool,
    pub result: Option<Result<(), CoreError>>,
    /// The `@name` asked of the network on «Check», until its answer.
    pub checking: Option<String>,
    /// Whom it goes to, as checked: «@alice» (or the short id) and the identity id.
    pub recipient: Option<(String, String)>,
    /// Why «To» cannot be used, said under it: not found on the network, or the lookup failed.
    pub to_problem: Option<String>,
    /// «To» as it was when `to_problem` was said: typing again takes it away.
    to_checked: String,
}

impl SendState {
    pub fn new(token_id: &str) -> Self {
        SendState {
            token_id: token_id.to_owned(),
            to: String::new(),
            amount: String::new(),
            review: false,
            password: SpendPassword::default(),
            waiting: false,
            result: None,
            checking: None,
            recipient: None,
            to_problem: None,
            to_checked: String::new(),
        }
    }

    /// `Done { what: SendToken }`: only an answer this dialog waits for.
    pub fn on_sent(&mut self, result: &Result<(), CoreError>) {
        if !self.waiting {
            return;
        }
        self.waiting = false;
        self.password.on_answer(result);
        self.result = Some(result.clone());
    }

    /// The network's answer about the `@name` in «To»: on to the review, or the reason under the
    /// field («The recipient is not found on the network.», R-3).
    pub fn on_resolved(&mut self, name: &str, result: &Result<ResolvedName, CoreError>) {
        if self.checking.as_deref() != Some(name) {
            return;
        }
        self.checking = None;
        self.to_checked = self.to.clone();
        match result {
            Ok(found) => {
                self.recipient = Some((format!("@{}", found.name), found.identity_id.clone()));
                self.to_problem = None;
                self.review = true;
            }
            Err(CoreError::NameNotFound) => {
                self.to_problem = Some(tr!("The recipient is not found on the network.").into());
            }
            Err(e) => self.to_problem = Some(crate::text::failed(tr!("Not checked"), e)),
        }
    }
}

/// «100,00 · Evo Coffee»: an amount outside its row (design §4).
fn with_name(amount: u64, info: &TokenInfo) -> String {
    format!(
        "{} · {}",
        state::written(amount, info.decimals),
        crate::i18n::own(state::name(info))
    )
}

/// The words of a refusal before the password (R-3).
fn problem_text(problem: &state::FormProblem, info: &TokenInfo, who: &str) -> String {
    use state::FormProblem::*;
    match problem {
        Address => tr!("A token goes to an identity — a Dash address will not do here.").into(),
        Amount => tr!("Not an amount of this token").into(),
        MoreThan(balance) => format!(
            tr!("More than you have: {} available."),
            with_name(*balance, info)
        ),
        NoCredits => format!(
            tr!("Not enough credits for the fee: top up {}."),
            crate::i18n::own(who)
        ),
        Token(TokenRefusal::Paused) => {
            tr!("The issuer paused the token — transfers are not available now.").into()
        }
        Token(TokenRefusal::Frozen) => {
            tr!("The issuer froze this balance — it cannot be sent.").into()
        }
        Token(TokenRefusal::NotTransferable) => {
            tr!("This token cannot be transferred: its issuer decided so.").into()
        }
        Token(TokenRefusal::NoKey) => tr!("This identity has no key that may send tokens.").into(),
        Token(TokenRefusal::NotKnown) => tr!("The balance is not known yet: wait for it.").into(),
        Token(TokenRefusal::Zero) => tr!("Not an amount of this token").into(),
        Token(TokenRefusal::MoreThanBalance(balance)) => format!(
            tr!("More than you have: {} available."),
            with_name(*balance, info)
        ),
        Token(TokenRefusal::NoCredits) => format!(
            tr!("Not enough credits for the fee: top up {}."),
            crate::i18n::own(who)
        ),
    }
}

/// One line under a field, always taken (FEAT-0034): the refusal in `danger`, else `hint` in
/// `muted`, else empty.
fn under_field(ui: &mut Ui, p: &crate::theme::Palette, problem: Option<&str>, hint: Option<&str>) {
    let (words, color) = match (problem, hint) {
        (Some(problem), _) => (problem, p.danger),
        (None, Some(hint)) => (hint, p.muted),
        (None, None) => (" ", p.muted),
    };
    ui.add(egui::Label::new(txt(words, 12.0, Weight::Regular, color)).wrap());
}

/// The dialog's width in a window this wide: 460 in 800×600, 500 above (design §8, review 6d).
fn dialog_width(ui: &Ui) -> f32 {
    // `dialog` adds its margins, 28 on each side.
    if ui.ctx().content_rect().width() < 900.0 {
        460.0 - 56.0
    } else {
        500.0 - 56.0
    }
}

pub(super) fn send_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, tokens: &mut TokensState) {
    let p = a.p;
    let Some(form) = &mut tokens.sending else {
        return;
    };
    let wallet = cx.wallet();
    let Some(row): Option<TokenRow> = wallet
        .tokens
        .rows
        .iter()
        .find(|r| r.info.token_id == form.token_id)
        .cloned()
    else {
        tokens.sending = None;
        return;
    };
    let who = sender(wallet);
    let from = shown_name(wallet);
    // As the core declares it until AC-T9 measures it: twice a document write (part 3 review r1).
    let fee = wallet.fees.document_write.saturating_mul(2);
    let balance = row.balance.unwrap_or(0);
    if form.to != form.to_checked {
        form.to_problem = None;
    }
    let problem = state::form_problem(&row, &form.to, &form.amount, who);
    let amount = state::typed(&form.amount, row.info.decimals).ok();
    let ask = password_asked(cx.snap);
    let mut send = None;
    let mut resolve = None;
    let mut top_up = false;
    let title = if form.review {
        tr!("Check the transfer")
    } else {
        tr!("Send a token")
    };
    let closed = dialog(
        ui,
        a,
        "token-send",
        "token",
        title,
        dialog_width(ui),
        |ui, a| {
            let mut close = false;
            ui.spacing_mut().item_spacing.y = 10.0;
            let value = sans(13.0, Weight::Medium);
            let number = mono(13.0, Weight::Medium);
            if !form.review {
                card(&p, Margin::same(14)).fill(p.raised).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(txt(
                                &crate::i18n::own(state::name(&row.info)),
                                14.0,
                                Weight::SemiBold,
                                p.text,
                            ));
                            ui.label(txt(
                                &id_line(&row.info, false),
                                12.0,
                                Weight::Regular,
                                p.muted,
                            ));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(state::written(balance, row.info.decimals))
                                    .font(mono(13.0, Weight::Medium))
                                    .color(p.text),
                            );
                            ui.label(txt(tr!("available"), 12.0, Weight::Regular, p.muted));
                        });
                    });
                });
                // «From identity» on the left, the name on the right: one axis with the fee below.
                kv_row(
                    ui,
                    &p,
                    tr!("From identity"),
                    &crate::i18n::own(&from),
                    value.clone(),
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    field_label(ui, &p, tr!("Send to"));
                    text_field(
                        ui,
                        &p,
                        "token-to",
                        &mut form.to,
                        // An example, not the hint again: the hint is under the field (design r3,
                        // R3-1).
                        "@alice",
                        FieldKind::Text,
                        |_| {},
                    );
                    let to_problem = form.to_problem.clone().or_else(|| {
                        (problem == Some(state::FormProblem::Address))
                            .then(|| problem_text(&state::FormProblem::Address, &row.info, &from))
                    });
                    // The hint stays under the field: the placeholder goes as one types (review 6b).
                    under_field(
                        ui,
                        &p,
                        to_problem.as_deref(),
                        Some(tr!("@name, contact or identity id")),
                    );
                });
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    field_label(ui, &p, tr!("How many"));
                    let mut all = false;
                    text_field(
                        ui,
                        &p,
                        "token-amount",
                        &mut form.amount,
                        "0",
                        FieldKind::Amount,
                        |ui| {
                            all = crate::components::mini_button(ui, &p, tr!("Max")).clicked();
                            ui.label(txt(
                                &crate::i18n::own(state::name(&row.info)),
                                13.0,
                                Weight::Regular,
                                p.muted,
                            ));
                        },
                    );
                    if all {
                        form.amount = state::written(balance, row.info.decimals);
                    }
                    let amount_problem = match &problem {
                        Some(state::FormProblem::Address | state::FormProblem::NoCredits)
                        | None => None,
                        Some(other) => Some(problem_text(other, &row.info, &from)),
                    };
                    under_field(ui, &p, amount_problem.as_deref(), None);
                });
                if problem == Some(state::FormProblem::NoCredits) {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(txt(
                            &problem_text(&state::FormProblem::NoCredits, &row.info, &from),
                            12.0,
                            Weight::Regular,
                            p.danger,
                        ));
                        let link = ui.add(
                            egui::Label::new(
                                txt(tr!("Top up"), 12.0, Weight::Medium, p.text).underline(),
                            )
                            .sense(Sense::click()),
                        );
                        crate::components::named(
                            &link,
                            egui::WidgetType::Button,
                            true,
                            tr!("Top up"),
                        );
                        top_up = link.clicked();
                    });
                } else {
                    kv_row(
                        ui,
                        &p,
                        &format!(tr!("Fee, in credits of {}"), crate::i18n::own(&from)),
                        &format!("≈ {}", crate::format::amount(fee)),
                        number.clone(),
                    );
                }
                notice(
                    ui,
                    a,
                    "eye",
                    p.muted,
                    tr!("The transfer and token balances are visible to everyone on the network."),
                );
                button_row(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let ready = problem.is_none()
                        && form.to_problem.is_none()
                        && form.checking.is_none()
                        && !form.to.trim().is_empty()
                        && amount.is_some_and(|a| a > 0);
                    if button_enabled(ui, a, tr!("Check"), ButtonKind::Primary, None, ready)
                        .clicked()
                    {
                        let to = form.to.trim().to_owned();
                        if to.starts_with('@') {
                            // The name is asked of the network before the review (R-3).
                            resolve = Some(to);
                        } else {
                            form.recipient = Some((state::short(&to), to));
                            form.review = true;
                        }
                    }
                });
                if form.checking.is_some() {
                    ui.label(txt(
                        tr!("Checking the recipient…"),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
            } else {
                let amount = amount.unwrap_or(0);
                let (to_name, to_id) = form.recipient.clone().unwrap_or_default();
                let to_line = if to_name == state::short(&to_id) {
                    to_name.clone()
                } else {
                    format!("{to_name} · {}", state::short(&to_id))
                };
                kv_row(
                    ui,
                    &p,
                    tr!("Token"),
                    &crate::i18n::own(state::name(&row.info)),
                    value.clone(),
                );
                kv_row(
                    ui,
                    &p,
                    tr!("Token id"),
                    &crate::i18n::own(&format!(
                        "{} · {}",
                        state::short(&row.info.token_id),
                        issuer(&row.info)
                    )),
                    value.clone(),
                );
                kv_row(
                    ui,
                    &p,
                    tr!("From identity"),
                    &crate::i18n::own(&from),
                    value.clone(),
                );
                kv_row(
                    ui,
                    &p,
                    tr!("Send to"),
                    &crate::i18n::own(&to_line),
                    value.clone(),
                );
                kv_row(
                    ui,
                    &p,
                    tr!("Amount"),
                    &crate::i18n::own(&with_name(amount, &row.info)),
                    value.clone(),
                );
                kv_row(
                    ui,
                    &p,
                    &format!(tr!("Fee, in credits of {}"), crate::i18n::own(&from)),
                    &format!("≈ {}", crate::format::amount(fee)),
                    number,
                );
                kv_row(
                    ui,
                    &p,
                    tr!("Left after"),
                    &crate::i18n::own(&with_name(balance.saturating_sub(amount), &row.info)),
                    value,
                );
                notice(
                    ui,
                    a,
                    "info",
                    p.muted,
                    tr!(
                        "The transfer cannot be undone. It and token balances are visible to everyone on the network."
                    ),
                );
                // The wallet's password, as for every spend (R-6, BUG-42).
                form.password.field(ui, &p, "token-password", ask);
                match form.result.as_ref().filter(|_| !form.password.is_wrong()) {
                    Some(Ok(())) => {
                        notice(ui, a, "confirmed", p.accent, tr!("The token is sent."));
                        button_row(ui, |ui| {
                            if button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked() {
                                close = true;
                            }
                        });
                        return close;
                    }
                    Some(Err(e)) => crate::components::error_notice(
                        ui,
                        a,
                        &crate::text::failed(tr!("Not sent"), e),
                        crate::text::error_details(e).as_deref(),
                    ),
                    None => {}
                }
                button_row_spend(ui, |ui| {
                    if button(ui, a, tr!("Back"), ButtonKind::Ghost, None).clicked() {
                        form.review = false;
                        form.result = None;
                    }
                    let ready = !form.waiting && form.password.ready(ask) && problem.is_none();
                    if button_enabled(
                        ui,
                        a,
                        tr!("Sign and send"),
                        ButtonKind::Primary,
                        None,
                        ready,
                    )
                    .clicked()
                    {
                        send = Some(Spend::SendToken {
                            token_id: row.info.token_id.clone(),
                            to: to_id.clone(),
                            amount,
                        });
                    }
                });
            }
            close
        },
    );
    if let Some(name) = resolve {
        form.checking = Some(name.clone());
        form.to_problem = None;
        cx.send(Command::ResolveName { name });
    }
    if top_up {
        cx.top_up();
    }
    if let Some(op) = send {
        cx.send(form.password.command(op, ask));
        form.waiting = true;
        form.result = None;
    }
    if closed {
        form.password.forget();
        tokens.sending = None;
    }
}

/// «Add a token»: by id or by a word, then «Add» on what was found.
#[derive(Default)]
pub struct AddState {
    pub query: String,
    /// The query asked of the core, until its answer.
    asked: Option<String>,
    found: Option<Result<Vec<TokenInfo>, CoreError>>,
    /// The token «Add» was clicked for, until the answer.
    adding: Option<String>,
    added: Vec<String>,
    failed: Option<CoreError>,
}

impl AddState {
    /// The dialog asking `query` at once, and the command that asks it (the `tokens-add-*`
    /// presets).
    #[cfg(feature = "dev-tools")]
    pub fn asking(query: &str) -> Option<(Self, Command)> {
        let command = add_command(query)?;
        let state = AddState {
            query: query.to_owned(),
            asked: Some(asked_of(&command)),
            ..Default::default()
        };
        Some((state, command))
    }

    pub fn on_found(&mut self, query: &str, result: &Result<Vec<TokenInfo>, CoreError>) {
        // The answer to the last question only; «not synced yet» is followed by the answer.
        if self.asked.as_deref() != Some(query) {
            return;
        }
        if matches!(result, Err(CoreError::NotSynced)) {
            self.found = Some(result.clone());
            return;
        }
        self.asked = None;
        self.found = Some(result.clone());
    }

    pub fn on_tracked(&mut self, result: &Result<(), CoreError>) {
        let Some(id) = self.adding.take() else {
            return;
        };
        match result {
            Ok(()) => self.added.push(id),
            Err(e) => self.failed = Some(e.clone()),
        }
    }
}

/// An id is Base58 of 32 bytes: 43 or 44 characters; anything else is a word.
fn is_id(text: &str) -> bool {
    let t = text.trim();
    (43..=44).contains(&t.len())
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() && !matches!(c, '0' | 'O' | 'I' | 'l'))
}

/// The command a query asks: by id, or by a word (lower case, as Platform keeps keywords).
pub fn add_command(query: &str) -> Option<Command> {
    let q = query.trim();
    if q.is_empty() {
        None
    } else if is_id(q) {
        Some(Command::FindToken { id: q.to_owned() })
    } else {
        Some(Command::SearchTokens {
            keyword: q.to_lowercase(),
        })
    }
}

/// What the core's answer to `command` names as its query.
fn asked_of(command: &Command) -> String {
    match command {
        Command::FindToken { id } => id.clone(),
        Command::SearchTokens { keyword } => keyword.clone(),
        _ => String::new(),
    }
}

/// Where a found token stands in the wallet's list.
#[derive(Clone, Copy, PartialEq)]
enum Listed {
    No,
    Shown,
    Hidden,
}

/// A found token as a row of the list without «Send» (review, «Добавить токен»).
fn found_row(
    ui: &mut Ui,
    a: &mut Assets,
    info: &TokenInfo,
    listed: Listed,
    added: bool,
    busy: bool,
) -> bool {
    let p = a.p;
    let mut add = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (circle, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
        ui.painter().circle_filled(circle.center(), 18.0, p.raised);
        a.icons.icon("token", p.text, &p, 18.0).paint_at(
            ui,
            egui::Rect::from_center_size(circle.center(), vec2(18.0, 18.0)),
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(txt(
                &crate::i18n::own(state::name(info)),
                14.0,
                Weight::SemiBold,
                p.text,
            ));
            ui.label(txt(&id_line(info, false), 12.0, Weight::Regular, p.muted));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if added {
                ui.label(txt(tr!("Added"), 13.0, Weight::Medium, p.accent));
            } else {
                match listed {
                    Listed::Shown => {
                        ui.label(txt(
                            tr!("Already in the list"),
                            13.0,
                            Weight::Regular,
                            p.muted,
                        ));
                    }
                    // Hidden by the person: «Hide» is not for good (architect, part 4).
                    Listed::Hidden => {
                        add = button_enabled(
                            ui,
                            a,
                            tr!("Return to the list"),
                            ButtonKind::Secondary,
                            None,
                            !busy,
                        )
                        .clicked();
                    }
                    Listed::No => {
                        add = button_enabled(ui, a, tr!("Add"), ButtonKind::Secondary, None, !busy)
                            .clicked();
                    }
                }
            }
        });
    });
    add
}

pub(super) fn add_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, tokens: &mut TokensState) {
    let p = a.p;
    let Some(add) = &mut tokens.adding else {
        return;
    };
    let wallet = cx.wallet();
    let listed: Vec<String> = wallet
        .tokens
        .rows
        .iter()
        .map(|r| r.info.token_id.clone())
        .collect();
    let hidden = wallet.tokens.hidden.clone();
    let standing = |id: &String| {
        if hidden.contains(id) {
            Listed::Hidden
        } else if listed.contains(id) {
            Listed::Shown
        } else {
            Listed::No
        }
    };
    let mut ask = None;
    let mut track = None;
    let closed = dialog(
        ui,
        a,
        "token-add",
        "token",
        tr!("Add a token"),
        dialog_width(ui),
        |ui, a| {
            let mut close = false;
            ui.spacing_mut().item_spacing.y = 10.0;
            let mut enter = false;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("Token id, contract id or a word"));
                let typed = text_field(
                    ui,
                    &p,
                    "token-query",
                    &mut add.query,
                    tr!("e.g. 7Hq2… or coffee"),
                    FieldKind::Text,
                    |_| {},
                );
                enter = typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let nothing = matches!(&add.found, Some(Ok(list)) if list.is_empty())
                    .then_some(tr!("Nothing found. Check the id or try another word."));
                match nothing {
                    Some(words) => under_field(ui, &p, None, Some(words)),
                    None => under_field(
                        ui,
                        &p,
                        None,
                        Some(tr!("You can add a token with a zero balance too.")),
                    ),
                }
            });
            match &add.found {
                None if add.asked.is_some() => {
                    ui.label(txt(tr!("Searching…"), 13.0, Weight::Regular, p.muted));
                }
                Some(Ok(list)) => {
                    for (i, info) in list.iter().enumerate() {
                        if i > 0 {
                            crate::components::divider(ui, &p);
                        }
                        let busy = add.adding.is_some();
                        if found_row(
                            ui,
                            a,
                            info,
                            standing(&info.token_id),
                            add.added.contains(&info.token_id),
                            busy,
                        ) {
                            track = Some(info.token_id.clone());
                        }
                    }
                }
                Some(Err(e)) => crate::components::error_notice(
                    ui,
                    a,
                    &crate::text::failed(tr!("Not found"), e),
                    crate::text::error_details(e).as_deref(),
                ),
                None => {}
            }
            if let Some(e) = &add.failed {
                crate::components::error_notice(
                    ui,
                    a,
                    &crate::text::failed(tr!("Not added"), e),
                    crate::text::error_details(e).as_deref(),
                );
            }
            button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                let ready = !add.query.trim().is_empty() && add.asked.is_none();
                if button_enabled(ui, a, tr!("Find"), ButtonKind::Primary, None, ready).clicked()
                    || (enter && ready)
                {
                    ask = add_command(&add.query);
                }
            });
            close
        },
    );
    if let Some(command) = ask {
        add.asked = Some(asked_of(&command));
        add.found = None;
        add.failed = None;
        cx.send(command);
    }
    if let Some(token_id) = track {
        add.adding = Some(token_id.clone());
        add.failed = None;
        cx.send(Command::TrackToken { token_id, on: true });
    }
    if closed {
        tokens.adding = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_found_by_id_and_anything_else_by_a_word() {
        let id = "5TKGz2Jm9wDqYp4rHcV8sNbE1xUa7LfQ3kRt6WhXoP2d";
        assert!(matches!(add_command(id), Some(Command::FindToken { id: i }) if i == id));
        assert!(matches!(
            add_command(" Coffee "),
            Some(Command::SearchTokens { keyword }) if keyword == "coffee"
        ));
        assert!(add_command("  ").is_none());
    }

    #[test]
    fn only_the_answer_to_the_last_question_is_taken() {
        let mut add = AddState {
            asked: Some("coffee".into()),
            ..Default::default()
        };
        add.on_found("gold", &Ok(Vec::new()));
        assert!(add.found.is_none(), "an answer to another question");
        add.on_found("coffee", &Err(CoreError::NotSynced));
        assert!(
            add.asked.is_some(),
            "not synced yet: the answer still comes"
        );
        add.on_found("coffee", &Ok(Vec::new()));
        assert!(add.asked.is_none() && matches!(add.found, Some(Ok(_))));
    }

    #[test]
    fn a_send_answer_counts_only_while_waited_for() {
        let mut send = SendState::new("T");
        send.on_sent(&Ok(()));
        assert!(send.result.is_none(), "nobody asked");
        send.waiting = true;
        send.on_sent(&Err(CoreError::NotEnough));
        assert_eq!(send.result, Some(Err(CoreError::NotEnough)));
        assert!(!send.waiting);
    }

    /// R-3: a name not on the network is said under «To» before the review; a found one goes to
    /// the review with its id.
    #[test]
    fn the_recipients_name_is_checked_before_the_review() {
        let mut send = SendState::new("T");
        send.to = "@nobody".into();
        send.checking = Some("@nobody".into());
        send.on_resolved("@other", &Err(CoreError::NameNotFound));
        assert!(send.to_problem.is_none(), "an answer about another name");
        send.on_resolved("@nobody", &Err(CoreError::NameNotFound));
        assert_eq!(
            send.to_problem.as_deref(),
            Some("The recipient is not found on the network.")
        );
        assert!(!send.review);
        send.checking = Some("@alice".into());
        send.on_resolved(
            "@alice",
            &Ok(ResolvedName {
                name: "alice".into(),
                display: None,
                identity_id: "3GkPxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxWq1R".into(),
                profile: None,
                contact: None,
            }),
        );
        assert!(send.review);
        assert_eq!(
            send.recipient.as_ref().map(|(n, _)| n.as_str()),
            Some("@alice")
        );
    }
}
