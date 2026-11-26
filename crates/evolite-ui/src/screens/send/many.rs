//! FEAT-0056: one Core payment to several recipients — rows of «To» and «Amount», «All» on one
//! of them, «Paste a list», the totals, and the last look with one password
//! ([`evolite_api::Spend::SendMany`]). From Core only, with «Features for advanced users» on;
//! a payment to one stays [`evolite_api::Spend::Send`], untouched.

use super::*;
use crate::components::Part;
use std::collections::HashMap;

/// The rows of a payment to several, with what the core said of each recipient.
#[derive(Default)]
pub struct Many {
    pub rows: Vec<Row>,
    /// `PreviewPayment`'s answer for each recipient asked (by the text sent): the address the
    /// payment goes to, or why it cannot.
    pub previews: HashMap<String, Result<String, CoreError>>,
    /// «Paste a list» open, with what is typed, and the lines it could not take.
    pub paste: Option<String>,
    pub paste_errors: Vec<String>,
    /// The payment of these rows is with the core: they stay until its answer (cleared once it
    /// went, kept if refused).
    pub signed: bool,
}

/// One recipient as typed.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Row {
    pub to: String,
    pub amount: String,
    /// «All»: the rest after the other rows and the fee (UC-2).
    pub all: bool,
}

/// What one frame of the rows works out before they are drawn.
pub(super) struct ManyView {
    /// Each row's amount; `None` while it cannot be read.
    pub(super) amounts: Vec<Option<Duffs>>,
    /// Each row's problem, said under it.
    pub(super) problems: Vec<Option<String>>,
    /// Recipients typed twice, said once (R-4: allowed).
    pub(super) twice: Vec<String>,
    pub(super) to_recipients: Duffs,
    pub(super) fee: Duffs,
    /// The change back to this wallet, as the coins it takes make it; `None` when there is
    /// none (an «All» row, or dust going to the fee).
    pub(super) change: Option<Duffs>,
    /// Every row reads and nothing is short: «Check the payment» may open.
    pub(super) ready: bool,
    /// What the rows need more than the source has.
    pub(super) short: Option<Duffs>,
    pub(super) rate: Option<u64>,
}

/// Whether «+ Recipient» is offered: from Core, with the advanced features on (R-9, Н2), not
/// to the wallet's own Platform address.
pub(super) fn allowed(cx: &Ctx, source: Source) -> bool {
    cx.advanced() && source == Source::Core
}

impl SendForm {
    /// «+ Recipient»: the first time, the recipient and amount typed become the first row.
    pub(crate) fn add_recipient(&mut self) {
        match &mut self.many {
            Some(many) if many.rows.len() < evolite_api::MAX_RECIPIENTS => {
                many.rows.push(Row::default())
            }
            Some(_) => {}
            None => {
                let first = Row {
                    to: std::mem::take(&mut self.recipient),
                    amount: std::mem::take(&mut self.amount),
                    all: std::mem::take(&mut self.max),
                };
                self.many = Some(Many {
                    rows: vec![first, Row::default()],
                    ..Default::default()
                });
            }
        }
        self.result = None;
    }

    /// «×» on row `i`: down to one, the form is a payment to one again, with that row.
    pub(super) fn remove_row(&mut self, i: usize) {
        let Some(many) = &mut self.many else { return };
        if i < many.rows.len() {
            many.rows.remove(i);
        }
        if many.rows.len() <= 1 {
            let last = many.rows.pop().unwrap_or_default();
            self.many = None;
            self.recipient = last.to;
            self.amount = last.amount;
            self.max = last.all;
        }
    }

    /// FEAT-0056 UC-5: «Send again» of a payment to several — every recipient and amount, no
    /// coins chosen.
    pub fn paying_many(rows: Vec<(String, Duffs)>) -> Self {
        let mut form = Self::paying(Source::Core, String::new());
        form.many = Some(Many {
            rows: rows
                .into_iter()
                .map(|(to, amount)| Row {
                    to,
                    amount: crate::format::input_text(amount),
                    all: false,
                })
                .collect(),
            ..Default::default()
        });
        form
    }
}

/// A line of a pasted list split into the recipient and the amount. BUG-328 (a): a semicolon or
/// a tab, when the line has one, is the separator, so an amount may have a decimal comma
/// («yA1; 0,001»); else the first comma — an address or an @name has none («yA1, 0,001»).
fn split_line(line: &str) -> Vec<&str> {
    let parts: Vec<&str> = if line.contains([';', '\t']) {
        line.split([';', '\t']).collect()
    } else {
        line.splitn(2, ',').collect()
    };
    parts
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

/// UC-3: the rows of a pasted list — `address or @name, amount`, split by a semicolon, a tab or
/// a comma ([`split_line`]) — and the lines that are not one, by their number: an amount that
/// does not read, or a recipient that is none ([`recipient_problem`], BUG-328 (b)). An amount
/// goes into its row as the interface writes it (BUG-328 (c)).
pub(super) fn parse_list(text: &str, check: &Recipients) -> (Vec<Row>, Vec<String>) {
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let problem = match split_line(line).as_slice() {
            [to, amount] => match (
                recipient_problem(to, check),
                crate::format::parse_amount(amount),
            ) {
                (Some(why), _) => Some(why),
                (None, Some(d)) if d > 0 => {
                    rows.push(Row {
                        to: (*to).to_owned(),
                        amount: crate::format::input_text(d),
                        all: false,
                    });
                    None
                }
                (None, _) => Some(amount_problem(amount)),
            },
            _ => Some(tr!("One per line: an address or @name, then the amount").to_owned()),
        };
        if let Some(problem) = problem {
            errors.push(format!(tr!("Line {}: {}"), n + 1, crate::i18n::t(&problem)));
        }
    }
    (rows, errors)
}

/// Why amount `text` of a row or a pasted line does not read: the amount field's own reason
/// ([`crate::format::amount_hint`], BUG-161 — «1,500» asked which, too many digits), the
/// general line only when it has none (BUG-328 r1).
fn amount_problem(text: &str) -> String {
    crate::format::amount_hint(text).unwrap_or_else(|| tr!("Enter an amount, e.g. 0.25").to_owned())
}

/// What the recipients of a payment to several are checked by.
pub(super) struct Recipients {
    pub(super) has_identity: bool,
    /// The demo core's addresses are made up: only their checksum is not asked of them.
    pub(super) demo: bool,
}

/// The problem of recipient `to` in a payment to several, before the core is asked: one it can
/// never pay (R-1) — a Platform address, an address of the other network, or (BUG-328 (b)) no
/// Core address at all.
fn recipient_problem(to: &str, check: &Recipients) -> Option<String> {
    let to = to.trim();
    if to.is_empty() {
        return None;
    }
    let network = crate::context::network();
    if to.starts_with('@') {
        return (!check.has_identity).then(|| tr!("Paying by @name takes an identity").to_owned());
    }
    if evolite_api::address::is_platform(to, network) {
        return Some(
            tr!("A payment to several recipients goes only to Core addresses and DashPay contacts")
                .to_owned(),
        );
    }
    if let Some(why) = super::form::wrong_network_line(to, network) {
        return Some(why);
    }
    let name = |network| match network {
        evolite_api::Network::Testnet => "Testnet",
        evolite_api::Network::Mainnet => "Mainnet",
    };
    match super::core_address::check(to, network) {
        super::core_address::CoreAddress::Here => None,
        super::core_address::CoreAddress::Other(found) => Some(format!(
            tr!("This is a {found} address — it doesn't work on {here}."),
            found = name(found),
            here = name(network)
        )),
        super::core_address::CoreAddress::Checksum if check.demo => None,
        super::core_address::CoreAddress::Checksum => Some(
            tr!("This is not a Dash address: a typo or an extra character — the checksum does not match")
                .to_owned(),
        ),
        super::core_address::CoreAddress::Not => {
            Some(tr!("This is not a Dash address or @name").to_owned())
        }
    }
}

/// The fee of all the outputs at the form's rate, and what the source holds for them: the
/// chosen coins (with «All», all of them), else the wallet's Core coins.
pub(super) fn view(cx: &Ctx, form: &SendForm, source: &SourceView) -> Option<ManyView> {
    let many = form.many.as_ref()?;
    let n = many.rows.len();
    // FEAT-0040 with contacts too (А-1): the wallet's rate whatever the recipients are.
    let rate = super::fee::rate(cx, form, "");
    let fees = super::fee::rated(&cx.wallet().fees, rate);
    let extra = evolite_api::extra_outputs_fee(n.saturating_sub(1));
    let extra = rate.map_or(extra, |r| evolite_api::fee_choice::at_rate(extra, r));
    let check = Recipients {
        has_identity: cx.wallet().identity.is_some(),
        demo: cx.snap.demo,
    };
    let all_at = many.rows.iter().position(|r| r.all);
    let fixed: Vec<Option<Duffs>> = many
        .rows
        .iter()
        .map(|r| {
            if r.all {
                None
            } else {
                crate::format::parse_amount(&r.amount).filter(|d| *d > 0)
            }
        })
        .collect();
    let fixed_sum: Duffs = fixed.iter().flatten().sum();
    let chosen = form.coins.chosen_coins();
    let values: Vec<Duffs> = chosen.iter().map(|c| c.value).collect();
    let held: Duffs = if chosen.is_empty() {
        source.duffs
    } else {
        values.iter().sum()
    };
    let (fee, all_amount, change) = if all_at.is_some() {
        // Everything the coins hold goes: the fee of spending them all.
        let fee = if chosen.is_empty() {
            fees.core_payment.saturating_add(extra)
        } else {
            fees.core_fee_for_inputs(values.len()).saturating_add(extra)
        };
        let rest = held - fixed_sum - fee;
        (fee, Some(rest), None)
    } else if chosen.is_empty() {
        let fee = fees.core_payment_to(fixed_sum, n);
        let inputs = fees.core_inputs_for(fixed_sum, n);
        let taken: Duffs = fees.core_coins.iter().take(inputs).map(|c| c.value).sum();
        let change = taken - fixed_sum - fee;
        let change = (change >= evolite_api::coin_plan::DUST).then_some(change);
        (fee, None, change)
    } else {
        match evolite_api::coin_plan::coin_plan(&values, fixed_sum, false, |k| {
            fees.core_fee_for_inputs(k).saturating_add(extra)
        }) {
            Ok(plan) => (plan.fee, None, (plan.change > 0).then_some(plan.change)),
            Err(_) => (fees.core_payment_to(fixed_sum, n), None, None),
        }
    };
    let amounts: Vec<Option<Duffs>> = many
        .rows
        .iter()
        .zip(&fixed)
        .map(|(r, f)| {
            if r.all {
                all_amount.filter(|d| *d > 0)
            } else {
                *f
            }
        })
        .collect();
    let mut seen: Vec<String> = Vec::new();
    let mut twice: Vec<String> = Vec::new();
    for r in &many.rows {
        let to = r.to.trim().to_owned();
        if to.is_empty() {
            continue;
        }
        if seen.contains(&to) && !twice.contains(&to) {
            twice.push(to.clone());
        }
        seen.push(to);
    }
    let problems: Vec<Option<String>> = many
        .rows
        .iter()
        .zip(&amounts)
        .map(|(r, amount)| {
            recipient_problem(&r.to, &check).or_else(|| match amount {
                None if r.all => Some(tr!("Too little for a payment").to_owned()),
                None if !r.amount.trim().is_empty() => Some(amount_problem(&r.amount)),
                Some(d) if *d < evolite_api::coin_plan::DUST => {
                    Some(tr!("Too little for a payment").to_owned())
                }
                _ => None,
            })
        })
        .collect();
    let to_recipients: Duffs = amounts.iter().flatten().sum();
    let short = (to_recipients + fee - held).max(0);
    let short = (all_at.is_none() && short > 0).then_some(short);
    let ready = many.rows.iter().all(|r| !r.to.trim().is_empty())
        && amounts.iter().all(Option::is_some)
        && problems.iter().all(Option::is_none)
        && short.is_none()
        && many.rows.iter().filter(|r| r.all).count() <= 1;
    Some(ManyView {
        amounts,
        problems,
        twice,
        to_recipients,
        fee,
        change,
        ready,
        short,
        rate,
    })
}

/// The rows, «+ Recipient» and «Paste a list», in place of «To» and «Amount».
pub(super) fn rows_card(ui: &mut Ui, a: &mut Assets, form: &mut SendForm, mv: &ManyView) {
    let p = a.p;
    let Some(many) = form.many.as_mut() else {
        return;
    };
    let all_taken = many.rows.iter().position(|r| r.all);
    let mut remove = None;
    let mut changed = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        field_label(ui, &p, tr!("To"));
        for (i, row) in many.rows.iter_mut().enumerate() {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                changed |= text_field(
                    ui,
                    &p,
                    &format!("many-to-{i}"),
                    &mut row.to,
                    tr!("@name or Dash address"),
                    FieldKind::Text,
                    |_| {},
                )
                .changed();
                ui.horizontal(|ui| {
                    // The buttons as wide as their words in this language, the amount the rest.
                    let buttons =
                        crate::components::button_width(ui, &crate::i18n::t(tr!("All (the rest)")))
                            + crate::components::button_width(ui, "×")
                            + 2.0 * ui.spacing().item_spacing.x;
                    let w = (ui.available_width() - buttons).max(80.0);
                    ui.allocate_ui(vec2(w, 40.0), |ui| {
                        if row.all {
                            // «All»: the rest, worked out, not typed.
                            let shown = mv.amounts[i].map_or("—".into(), crate::format::amount);
                            ui.label(txt(&shown, 15.0, Weight::Medium, p.text));
                        } else {
                            changed |= text_field(
                                ui,
                                &p,
                                &format!("many-amount-{i}"),
                                &mut row.amount,
                                "0.00",
                                FieldKind::Amount,
                                |_| {},
                            )
                            .changed();
                        }
                    });
                    let other_all = all_taken.is_some_and(|at| at != i);
                    let all = button_enabled(
                        ui,
                        a,
                        tr!("All (the rest)"),
                        if row.all {
                            ButtonKind::Primary
                        } else {
                            ButtonKind::Secondary
                        },
                        None,
                        !other_all,
                    )
                    .on_disabled_hover_text(
                        crate::i18n::t(tr!("\"All\" is already set for another recipient"))
                            .into_owned(),
                    );
                    if all.clicked() {
                        row.all = !row.all;
                        changed = true;
                    }
                    if button(ui, a, "×", ButtonKind::Ghost, None).clicked() {
                        remove = Some(i);
                    }
                });
                if let Some(problem) = &mv.problems[i] {
                    ui.add(crate::components::para(
                        &crate::i18n::t(problem),
                        12.0,
                        Weight::Regular,
                        p.danger,
                    ));
                }
            });
        }
        for to in &mv.twice {
            ui.label(txt(
                &format!(tr!("{} is in the list twice"), crate::i18n::own(to)),
                12.0,
                Weight::Regular,
                p.accent,
            ));
        }
        ui.horizontal(|ui| {
            let room = many.rows.len() < evolite_api::MAX_RECIPIENTS;
            if button_enabled(ui, a, tr!("+ Recipient"), ButtonKind::Secondary, None, room)
                .on_disabled_hover_text(format!(
                    tr!("No more than {} recipients"),
                    evolite_api::MAX_RECIPIENTS
                ))
                .clicked()
            {
                many.rows.push(Row::default());
                changed = true;
            }
            if button(ui, a, tr!("Paste a list"), ButtonKind::Ghost, None).clicked() {
                many.paste = Some(String::new());
                many.paste_errors.clear();
            }
        });
        for error in &many.paste_errors {
            ui.label(txt(error, 12.0, Weight::Regular, p.danger));
        }
    });
    if changed {
        form.result = None;
    }
    if let Some(i) = remove {
        form.remove_row(i);
    }
}

/// FEAT-0056 R-12 (AC-A10): whether the rows say what a payment to several shows its
/// recipients — two or more recipients typed, a DashPay contact among them.
pub(super) fn says_privacy(form: &SendForm) -> bool {
    let Some(many) = &form.many else { return false };
    let typed: Vec<&str> = many
        .rows
        .iter()
        .map(|r| r.to.trim())
        .filter(|t| !t.is_empty())
        .collect();
    typed.len() >= 2 && typed.iter().any(|t| t.starts_with('@'))
}

/// R-12: one muted line under the rows, over the totals; no icon, no tick, nothing held.
pub(super) fn privacy_line(ui: &mut Ui, p: &crate::theme::Palette, form: &SendForm) {
    if !says_privacy(form) {
        return;
    }
    ui.add(
        crate::components::para(
            tr!(
                "Every recipient sees the whole transaction: the other recipients' addresses and amounts. Contacts know it is from you."
            ),
            12.0,
            Weight::Regular,
            p.muted,
        )
        .line_height(17.0),
    );
}

/// «Paste a list» (UC-3): what parses becomes rows, after the ones there; a line that does not
/// is said by its number; more than the limit is cut there and said.
pub(super) fn paste_dialog(ui: &mut Ui, a: &mut Assets, form: &mut SendForm, demo: bool) {
    let Some(many) = form.many.as_mut() else {
        return;
    };
    let Some(text) = many.paste.as_mut() else {
        return;
    };
    let p = a.p;
    let mut parse = false;
    let closed = crate::components::dialog(
        ui,
        a,
        "send-paste-list",
        "send",
        tr!("Paste a list"),
        460.0,
        |ui, a| {
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.label(txt(
                tr!("One per line: an address or @name, then the amount"),
                13.0,
                Weight::Regular,
                p.muted,
            ));
            ui.add(
                egui::TextEdit::multiline(text)
                    .id_salt("send-paste-text")
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .font(crate::theme::mono(13.0, Weight::Regular)),
            );
            let mut close = false;
            crate::components::button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                if button(ui, a, tr!("Parse"), ButtonKind::Primary, None).clicked() {
                    parse = true;
                    close = true;
                }
            });
            close
        },
    );
    if parse {
        let check = Recipients {
            // An @name's own check comes with the rows: the list takes it as it is.
            has_identity: true,
            demo,
        };
        let (rows, mut errors) = parse_list(text, &check);
        // The empty rows give way to the list.
        many.rows
            .retain(|r| !r.to.trim().is_empty() || !r.amount.trim().is_empty());
        let room = evolite_api::MAX_RECIPIENTS.saturating_sub(many.rows.len());
        if rows.len() > room {
            errors.push(format!(
                tr!("No more than {} recipients"),
                evolite_api::MAX_RECIPIENTS
            ));
        }
        many.rows.extend(rows.into_iter().take(room));
        while many.rows.len() < 2 {
            many.rows.push(Row::default());
        }
        many.paste_errors = errors;
        form.result = None;
    }
    if closed && let Some(many) = form.many.as_mut() {
        many.paste = None;
    }
}

/// The totals under the rows (UC-4): «To recipients», «Fee», «Change», «Total spent».
pub(super) fn totals(ui: &mut Ui, p: &crate::theme::Palette, mv: &ManyView) {
    let value = crate::theme::mono(13.0, Weight::Regular);
    kv_row(
        ui,
        p,
        tr!("To recipients"),
        &crate::format::amount(mv.to_recipients),
        value.clone(),
    );
    kv_row(
        ui,
        p,
        tr!("Fee"),
        &format!("≈ {}", crate::format::amount(mv.fee)),
        value.clone(),
    );
    if let Some(change) = mv.change {
        kv_row(
            ui,
            p,
            tr!("Change back"),
            &format!("≈ {}", crate::format::amount(change)),
            value.clone(),
        );
    }
    kv_row(
        ui,
        p,
        tr!("Total spent"),
        &crate::format::amount(mv.to_recipients + mv.fee),
        crate::theme::mono(13.0, Weight::Medium),
    );
    if let Some(short) = mv.short {
        ui.label(txt(
            &format!(tr!("{} short"), crate::format::amount(short)),
            12.0,
            Weight::Regular,
            p.danger,
        ));
    }
}

/// The rows as the core will be asked for them; `None` while a row's amount cannot be read (an
/// amount not read is no zero).
fn send_to(form: &SendForm, mv: &ManyView) -> Option<Vec<evolite_api::SendTo>> {
    let many = form.many.as_ref()?;
    many.rows
        .iter()
        .zip(&mv.amounts)
        .map(|(r, amount)| {
            Some(evolite_api::SendTo {
                to: r.to.trim().to_owned(),
                amount: (*amount)?,
                all: r.all,
            })
        })
        .collect()
}

/// «Check the payment» under the rows, with what is still missing.
pub(super) fn send_row(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    mv: &ManyView,
    synced: bool,
    asked: &mut Asked,
) {
    let p = a.p;
    let signed = form.many.as_ref().is_some_and(|m| m.signed);
    let ready = mv.ready && synced && !signed;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if button_enabled(
            ui,
            a,
            tr!("Check and send"),
            ButtonKind::Primary,
            None,
            ready,
        )
        .clicked()
        {
            if !form.coins.chosen.is_empty() {
                asked.commands.push(form.coins.ask_again());
            }
            check(form, asked);
        }
        if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
            form.replace(SendForm::empty());
            asked.back = true;
        }
    });
    if !synced {
        ui.add(crate::components::para(
            crate::text::WAIT_FOR_SYNC,
            12.0,
            Weight::Regular,
            p.muted,
        ));
    }
}

/// «Check the payment»: each recipient goes to the core once for its address (an @name: the
/// contact's next one), then the last look opens.
pub(super) fn check(form: &mut SendForm, asked: &mut Asked) {
    let Some(many) = form.many.as_mut() else {
        return;
    };
    many.previews.clear();
    let mut sent: Vec<String> = Vec::new();
    for r in &many.rows {
        let to = r.to.trim().to_owned();
        if !sent.contains(&to) {
            asked
                .commands
                .push(Command::PreviewPayment { to: to.clone() });
            sent.push(to);
        }
    }
    form.confirming = true;
    form.result = None;
}

/// The last look of a payment to several: each recipient with the address the core showed, the
/// totals and the tick pinned over the password (BUG-249), «Sign and send to {n} recipients».
pub(super) fn confirm(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut SendForm,
    mv: &ManyView,
    ask_password: bool,
) {
    let p = a.p;
    let Some(recipients) = send_to(form, mv) else {
        form.confirming = false;
        return;
    };
    let n = recipients.len();
    let preview = Spend::SendMany {
        id: 0,
        recipients: recipients.clone(),
        note: None,
        rate: None,
        coins: None,
        fee_rate: mv.rate,
        fee_shown: None,
    };
    // A6: the tick by the total; another total takes it off.
    let large = crate::components::large_spend(cx.snap, &preview);
    let key = format!("{}+{}", mv.to_recipients, n);
    if large != form.large || key != form.large_to {
        form.large_ticked = false;
    }
    form.large = large;
    form.large_to = key;
    let previews = form
        .many
        .as_ref()
        .map(|m| m.previews.clone())
        .unwrap_or_default();
    let addresses: Vec<Option<&Result<String, CoreError>>> =
        recipients.iter().map(|r| previews.get(&r.to)).collect();
    let all_shown = addresses.iter().all(|a| matches!(a, Some(Ok(_))));
    let signing = form.signing.is_some();
    let timed_out = form.signing.is_some_and(|at| at.elapsed() >= SIGNING_HOLD);
    if signing {
        ui.ctx().request_repaint_after(SIGNING_HOLD);
    }
    let mut send_now = false;
    let closed = crate::components::dialog_with_summary(
        ui,
        a,
        "send-many-confirm",
        "send",
        tr!("Check the payment"),
        460.0,
        |ui, a, part| {
            ui.spacing_mut().item_spacing.y = 12.0;
            if part == Part::Summary {
                totals(ui, &p, mv);
                if form.large.is_some() {
                    crate::components::checkbox(
                        ui,
                        a,
                        &mut form.large_ticked,
                        crate::components::tick_text(),
                    );
                }
                return false;
            }
            if part == Part::Footer {
                if ask_password {
                    password_field(ui, a, form);
                }
                let mut close = false;
                crate::components::button_row_spend(ui, |ui| {
                    if !signing && button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let paused = ask_password && crate::components::spend_pause(ui.ctx()).paused();
                    let ready = !signing
                        && !paused
                        && mv.ready
                        && all_shown
                        && (form.large.is_none() || form.large_ticked)
                        && (!ask_password || !form.password.is_empty());
                    let label = if signing {
                        tr!("Sending…").to_owned()
                    } else {
                        crate::text::plural_in(
                            n as u32,
                            tr!("Sign and send to {} recipient"),
                            tr!("Sign and send to {} recipients"),
                        )
                    };
                    if button_enabled(ui, a, &label, ButtonKind::Primary, None, ready).clicked()
                        && ready
                    {
                        send_now = true;
                        close = !ask_password || form.waiting > 0;
                    }
                });
                return close || timed_out;
            }
            crate::screens::stale_core_line(ui, &p, &cx.snap.sync);
            // FEAT-0051 UC-4: every address whole, a mono block without «Copy» — the one typed
            // and the one a contact is paid at alike; a name stays in its line.
            for (r, address) in recipients.iter().zip(&addresses) {
                let typed = r.to.trim();
                let named = typed.starts_with('@');
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        if named {
                            ui.label(txt(&crate::i18n::own(typed), 13.0, Weight::Medium, p.text));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(txt(
                                &crate::format::amount(r.amount),
                                13.0,
                                Weight::Medium,
                                p.text,
                            ));
                        });
                    });
                    if !named {
                        crate::components::full_id(ui, a, typed, false);
                    }
                    match address {
                        Some(Ok(address)) if address != typed => {
                            crate::components::full_id(ui, a, address, false);
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            ui.add(crate::components::para(
                                &send_error(e),
                                12.0,
                                Weight::Regular,
                                p.danger,
                            ));
                        }
                        None => {
                            ui.label(txt(
                                &format!(tr!("Looking for {}…"), crate::i18n::own(&r.to)),
                                12.0,
                                Weight::Regular,
                                p.muted,
                            ));
                        }
                    }
                });
            }
            if let Some(note) = form.note_shown() {
                confirm_note(ui, a, &note, form.note_from_link());
            }
            false
        },
    );
    if send_now {
        sign_and_send_many(cx, form, recipients, mv, ask_password);
    }
    if closed {
        form.confirming = false;
        form.signing = None;
        form.password_wrong = false;
        form.password.zeroize();
    }
}

/// The payment goes to the core; the rows stay until its answer.
fn sign_and_send_many(
    cx: &mut Ctx,
    form: &mut SendForm,
    recipients: Vec<evolite_api::SendTo>,
    mv: &ManyView,
    ask_password: bool,
) {
    let any_all = recipients.iter().any(|r| r.all);
    form.sent_to = String::new();
    form.last_payment += 1;
    cx.send(Command::Spend {
        op: Spend::SendMany {
            id: form.last_payment,
            recipients,
            note: form.note_to_send().map(evolite_api::NoteText),
            rate: None,
            coins: form.coins.choice(any_all),
            fee_rate: mv.rate,
            // R-7: never more than the fee shown (with dust change).
            fee_shown: Some(mv.fee),
        },
        password: ask_password.then(|| Password::new(form.password.to_string())),
        ask: 0,
        large_confirmed: form.large.is_some() && form.large_ticked,
    });
    form.large_ticked = false;
    form.waiting += 1;
    form.signed_note = std::mem::take(&mut form.note);
    if let Some(many) = form.many.as_mut() {
        many.signed = true;
    }
    form.signing = (ask_password && form.waiting == 1).then(std::time::Instant::now);
    form.password_wrong = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two real testnet addresses (BUG-328's screenshots).
    const QA_A: &str = "yMEwQ1PLRtUprLAKxJgYnWsGRmcSAtBdr7";
    const QA_B: &str = "yR5ADQsL4Jrj2rBukKpg5ZhoaGwFmHCJhM";

    fn real() -> Recipients {
        Recipients {
            has_identity: true,
            demo: false,
        }
    }

    /// UC-3 (A4): five lines, one wrong — four rows, and the wrong one said by its number;
    /// a comma, a semicolon or a tab between. Mutant: every line taken → red.
    #[test]
    fn a_pasted_list_gives_its_rows_and_says_each_wrong_line() {
        let (rows, errors) = parse_list(
            &format!("{QA_A}, 0.1\n@carol; 0.2\n{QA_B}\t0.3\n{QA_A}, lots\n\n{QA_B}, 0.05\n"),
            &real(),
        );
        assert_eq!(
            rows.iter().map(|r| r.to.as_str()).collect::<Vec<_>>(),
            vec![QA_A, "@carol", QA_B, QA_B]
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("Line 4: "), "{errors:?}");
        let (_, errors) = parse_list("just one thing", &real());
        assert!(errors[0].contains("One per line"), "{errors:?}");
    }

    /// BUG-328 (a): a decimal comma with «;», with a tab, and after the first comma; (b) a line
    /// whose recipient is no address is said by its number and not taken — in the demo too,
    /// where only a made-up address's checksum is let pass; (c) the amounts as the interface
    /// writes them. Mutants: the comma split everywhere → (a) red; no recipient check → (b) red.
    #[test]
    fn a_decimal_comma_reads_and_a_line_without_an_address_is_said() {
        let (rows, errors) = parse_list(
            &format!("{QA_A}; 0,001\n{QA_B}\t0,002\n{QA_A}, 0,003\nnot-an-address, 0.001\n"),
            &real(),
        );
        let amounts: Vec<Option<Duffs>> = rows
            .iter()
            .map(|r| crate::format::parse_amount(&r.amount))
            .collect();
        assert_eq!(amounts, [Some(100_000), Some(200_000), Some(300_000)]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].starts_with("Line 4: ") && errors[0].contains("not a Dash address"),
            "{errors:?}"
        );
        // (c): what the interface writes, not what was pasted.
        assert!(
            rows.iter().all(|r| r.amount
                == crate::format::input_text(crate::format::parse_amount(&r.amount).unwrap())),
            "{rows:?}"
        );
        // A typo in a real address: its checksum; in the demo, let pass.
        let typo = QA_A.replacen('W', "X", 1);
        let (rows, errors) = parse_list(&format!("{typo}; 0.1"), &real());
        assert!(
            rows.is_empty() && errors[0].contains("checksum"),
            "{errors:?}"
        );
        let demo = Recipients {
            has_identity: true,
            demo: true,
        };
        let (rows, _) = parse_list(&format!("{typo}; 0.1\nnot-an-address; 0.1"), &demo);
        assert_eq!(rows.len(), 1, "the made-up one, not the words");
    }

    /// BUG-328 r1 Should: an amount a line has but that does not read says why, as the amount
    /// field does — in English «1,500» is asked which, not «e.g. 0.25»; the general line only
    /// when there is no reason. Mutant: always the general line → red.
    #[test]
    fn a_pasted_amount_that_does_not_read_says_why() {
        crate::i18n::set(crate::i18n::Lang::En);
        let (rows, errors) = parse_list(&format!("{QA_A}, 1,500\n{QA_B}; 0.000000001"), &real());
        assert!(rows.is_empty(), "{rows:?}");
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert_eq!(
            errors[0],
            format!("Line 1: {}", crate::format::amount_hint("1,500").unwrap())
        );
        assert!(errors[0].contains("Unclear"), "{errors:?}");
        assert!(errors[1].contains("Too many digits"), "{errors:?}");
        assert!(
            errors.iter().all(|e| !e.contains("e.g. 0.25")),
            "{errors:?}"
        );
        crate::i18n::set(crate::i18n::Lang::Ru);
    }

    /// UC-1, UC-2: «+ Recipient» takes what was typed as the first row; «×» down to one gives
    /// the form back as a payment to one, with that row.
    #[test]
    fn rows_come_from_the_form_and_go_back_to_it() {
        let mut form = SendForm::empty();
        form.recipient = "yA1".into();
        form.amount = "0.1".into();
        form.add_recipient();
        let many = form.many.as_ref().unwrap();
        assert_eq!(many.rows.len(), 2);
        assert_eq!(
            (many.rows[0].to.as_str(), many.rows[0].amount.as_str()),
            ("yA1", "0.1")
        );
        assert!(form.recipient.is_empty() && form.amount.is_empty());
        form.remove_row(1);
        assert!(form.many.is_none());
        assert_eq!(
            (form.recipient.as_str(), form.amount.as_str()),
            ("yA1", "0.1")
        );
    }
}
