//! «Coins» on Send (FEAT-0013, stage 1, path 2): the person may choose the Core coins a
//! payment takes from; the wallet takes the largest of them it needs, «Max» all of them. The
//! choice lives with this form only (R-9) and goes to the core as `Spend::Send.coins`.

use super::*;
use evolite_api::{Coin, CoinChoice, CoinState, coin_plan};
use std::collections::BTreeSet;

/// Coins under this are hidden in the list unless ticked or «Show small ones» (R-13).
pub(super) const SMALL: Duffs = 100_000;

/// The most coins one payment takes: the core refuses more (R-12).
pub(super) const MAX_COINS: usize = 500;

/// The order of the list (DDW's four).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Order {
    #[default]
    Largest,
    Smallest,
    Newest,
    Oldest,
}

impl Order {
    pub(super) const ALL: [Order; 4] = [
        Order::Largest,
        Order::Smallest,
        Order::Newest,
        Order::Oldest,
    ];

    pub(super) fn words(self) -> &'static str {
        match self {
            Order::Largest => tr!("Largest first"),
            Order::Smallest => tr!("Smallest first"),
            Order::Newest => tr!("Newest first"),
            Order::Oldest => tr!("Oldest first"),
        }
    }
}

/// What the form knows of the coins and the person's choice.
#[derive(Default)]
pub struct CoinPick {
    /// The coins chosen and applied: the payment takes from these. Empty: the wallet chooses.
    pub chosen: BTreeSet<String>,
    /// The dialog is open, with its own ticks until «Apply».
    pub dialog: bool,
    pub manual: bool,
    pub ticked: BTreeSet<String>,
    pub order: Order,
    pub show_small: bool,
    /// The number of the last `ListCoins`: only its answer is taken.
    pub ask: u64,
    /// The core's answer to it.
    pub coins: Option<Result<Vec<Coin>, CoreError>>,
}

impl CoinPick {
    /// Forget the choice: after a payment, a lock, a wallet switch, a source other than Core,
    /// a recipient that is an @name (R-9).
    pub fn reset(&mut self) {
        self.chosen.clear();
        self.ticked.clear();
        self.manual = false;
        self.dialog = false;
    }

    /// Asks the core for the coins again (the dialog opens, or a payment is checked).
    pub fn ask_again(&mut self) -> Command {
        self.ask += 1;
        Command::ListCoins { ask: self.ask }
    }

    /// The coins the core last listed; none until it answers.
    pub(super) fn listed(&self) -> &[Coin] {
        match &self.coins {
            Some(Ok(coins)) => coins,
            _ => &[],
        }
    }

    /// The chosen coins as the core listed them: those it no longer has are not here.
    pub(super) fn chosen_coins(&self) -> Vec<&Coin> {
        self.listed()
            .iter()
            .filter(|c| self.chosen.contains(&c.outpoint))
            .collect()
    }

    /// A chosen coin that is gone or no longer spendable (UC-4, R-10): its outpoint.
    pub(super) fn gone(&self, set: &BTreeSet<String>) -> Option<String> {
        self.coins.as_ref()?.as_ref().ok()?;
        set.iter()
            .find(|o| {
                !self
                    .listed()
                    .iter()
                    .any(|c| &c.outpoint == *o && c.state == CoinState::Final)
            })
            .cloned()
    }

    /// A coin as a sentence names it: its short address, as the list rows show it (CC4); the
    /// outpoint only for a coin the list no longer has.
    pub(super) fn coin_name(&self, outpoint: &str) -> String {
        self.listed()
            .iter()
            .find(|c| c.outpoint == outpoint)
            .map_or_else(
                || short_outpoint(outpoint),
                |c| crate::text::short_address(&c.address),
            )
    }

    /// The choice for the core: `None` while the wallet chooses.
    pub(super) fn choice(&self, all: bool) -> Option<CoinChoice> {
        (!self.chosen.is_empty()).then(|| CoinChoice::Within {
            coins: self.chosen.iter().cloned().collect(),
            all,
        })
    }

    /// How many different addresses the chosen coins are on.
    pub(super) fn addresses(coins: &[&Coin]) -> usize {
        coins
            .iter()
            .map(|c| c.address.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

impl SendForm {
    /// «Max» taken and not changed since, with coins chosen: what they hold less the fee of
    /// spending them all (R-7).
    pub(super) fn follow_coins_max(&mut self, fees: &evolite_api::Fees) {
        if self.max && !self.coins.chosen.is_empty() {
            let values: Vec<Duffs> = self.coins.chosen_coins().iter().map(|c| c.value).collect();
            if !values.is_empty() {
                let max = coin_plan::max_of(&values, |k| fees.core_fee_for_inputs(k));
                self.amount = crate::format::input_text(max);
                self.suggested = None;
            }
        }
    }

    /// The core's answer to `ListCoins`: only that of the last ask (a switch, a lock, a dialog
    /// opened again ask anew).
    pub fn on_coins(&mut self, ask: u64, result: &Result<Vec<Coin>, CoreError>) {
        if ask == self.coins.ask {
            self.coins.coins = Some(result.clone());
        }
    }
}

/// «3 coins · 0,09 DASH · 3 addresses»: the chosen coins in a line.
fn chosen_line(coins: &[&Coin]) -> String {
    let total: Duffs = coins.iter().map(|c| c.value).sum();
    format!(
        "{} · {} · {}",
        crate::text::plural(coins.len() as u32, tr!("coin"), tr!("coins")),
        crate::format::amount(total),
        crate::text::plural(
            CoinPick::addresses(coins) as u32,
            tr!("address"),
            tr!("addresses")
        ),
    )
}

/// Where a coin came from: this wallet, or «from @contact», in the interface language (the
/// line it goes into is not looked up as a whole).
fn whose(coin: &Coin) -> String {
    let words = match &coin.from_contact {
        Some(who) => format!(tr!("from @{}"), who.trim_start_matches('@')),
        None => tr!("this wallet").into(),
    };
    crate::i18n::t(&words).into_owned()
}

/// A coin's state in words (R-4): its icon and words.
fn state_words(state: CoinState) -> (&'static str, &'static str) {
    match state {
        CoinState::Final => ("lock", tr!("Final")),
        CoinState::Unconfirmed => ("pending", tr!("Pending")),
        CoinState::Reserved => ("lock", tr!("sending")),
    }
}

/// «Coins» under «From» (F1–F3): from Core only (R-1); for an @name inactive with the reason;
/// a click opens the dialog and asks the core for the coins.
pub(super) fn coins_row(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    v: &FormView,
    asked: &mut Asked,
) {
    let p = a.p;
    if v.source.source != Source::Core {
        return;
    }
    let to_name = v.recipient_label.starts_with('@');
    let (value, action, any) = {
        let chosen = form.coins.chosen_coins();
        if chosen.is_empty() {
            (tr!("automatic").to_owned(), tr!("Choose"), false)
        } else {
            (chosen_line(&chosen), tr!("Edit"), true)
        }
    };
    let enabled = !to_name;
    let frame = egui::Frame::new()
        .fill(p.raised)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 10));
    let row = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !enabled {
                ui.disable();
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(a.icons.icon("filter", p.muted, &p, 16.0));
                ui.label(txt(tr!("Coins"), 13.0, Weight::Regular, p.muted));
                ui.label(txt(&value, 13.0, Weight::SemiBold, p.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(a.icons.icon("chevron-right", p.text, &p, 14.0));
                    ui.label(txt(action, 13.0, Weight::SemiBold, p.text));
                });
            });
        })
        .response;
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let resp = ui.interact(row.rect, ui.id().with("send-coins"), sense);
    crate::components::named(
        &resp,
        egui::WidgetType::Button,
        enabled,
        // Each part in reading order (`spoken`), not as drawn (`t`): Arabic is read forwards.
        &format!(
            "{}: {}",
            crate::i18n::spoken(tr!("Coins")),
            crate::i18n::spoken(&value)
        ),
    );
    if resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        form.coins.ticked = form.coins.chosen.clone();
        form.coins.manual = !form.coins.chosen.is_empty();
        form.coins.dialog = true;
        asked.commands.push(form.coins.ask_again());
    }
    let note = if to_name {
        Some(tr!(
            "The wallet chooses the coins for a payment to a contact."
        ))
    } else if any {
        Some(tr!(
            "The wallet takes only the ticked coins the amount and the fee need. “Max” spends every ticked coin."
        ))
    } else {
        None
    };
    if let Some(note) = note {
        ui.add_space(-12.0);
        ui.add(egui::Label::new(txt(note, 12.0, Weight::Regular, p.muted)).wrap());
    }
}

/// The dialog «Coins for the payment» (CC1–CC4): the mode, the order and small coins, what is
/// ticked and what it pays, the list, «Apply».
pub(super) fn coins_dialog(ui: &mut Ui, a: &mut Assets, form: &mut SendForm, v: &FormView) {
    if !form.coins.dialog {
        return;
    }
    let fees = v.fees.clone();
    let amount = v.amount.unwrap_or(0);
    let mut apply = false;
    let open = crate::components::dialog(
        ui,
        a,
        "send-coins",
        "filter",
        tr!("Coins for the payment"),
        640.0,
        |ui, a| {
            let p = a.p;
            let pick = &mut form.coins;
            ui.spacing_mut().item_spacing.y = 12.0;
            let mut mode = usize::from(pick.manual);
            crate::components::segmented(
                ui,
                &p,
                &[tr!("Automatic"), tr!("Choose by hand")],
                &mut mode,
                true,
            );
            // Back to the wallet's choice: the ticks go (UC-3).
            if (mode == 1) != pick.manual {
                pick.manual = mode == 1;
                if !pick.manual {
                    pick.ticked.clear();
                }
            }
            let mut reason: Option<String> = None;
            if !pick.manual {
                ui.add(
                    egui::Label::new(txt(
                        tr!("The wallet picks the coins itself: largest first, as many as the amount and the fee need. This is usually cheaper."),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    ))
                    .wrap(),
                );
            } else {
                reason = manual_part(ui, a, pick, amount, &fees);
            }
            ui.add_space(4.0);
            let mut cancel = false;
            ui.horizontal(|ui| {
                let note = reason.clone().unwrap_or_else(|| {
                    tr!("The wallet takes only the ticked coins it needs.").into()
                });
                ui.add(egui::Label::new(txt(&note, 12.0, Weight::Regular, p.muted)).wrap());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    apply = crate::components::button_enabled(
                        ui,
                        a,
                        tr!("Apply"),
                        ButtonKind::Primary,
                        None,
                        reason.is_none(),
                    )
                    .clicked();
                    cancel =
                        crate::components::button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None)
                            .clicked();
                });
            });
            !(apply || cancel)
        },
    );
    if apply {
        form.coins.chosen = if form.coins.manual {
            std::mem::take(&mut form.coins.ticked)
        } else {
            BTreeSet::new()
        };
    }
    if !open || apply {
        form.coins.dialog = false;
        form.coins.ticked.clear();
    }
}

/// The hand-picked part of the dialog: its bar, what is ticked, the list. Why «Apply» is off,
/// if it is.
fn manual_part(
    ui: &mut Ui,
    a: &mut Assets,
    pick: &mut CoinPick,
    amount: Duffs,
    fees: &evolite_api::Fees,
) -> Option<String> {
    let p = a.p;
    let listed: Vec<Coin> = pick.listed().to_vec();
    let small_hidden = listed
        .iter()
        .filter(|c| c.value < SMALL && !pick.ticked.contains(&c.outpoint))
        .count();
    ui.horizontal(|ui| {
        if crate::components::mini_button(ui, &p, pick.order.words()).clicked() {
            let at = Order::ALL
                .iter()
                .position(|o| *o == pick.order)
                .unwrap_or(0);
            pick.order = Order::ALL[(at + 1) % Order::ALL.len()];
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(txt(
                &format!(
                    tr!("under {} · {} hidden"),
                    crate::format::amount(SMALL),
                    if pick.show_small { 0 } else { small_hidden }
                ),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            crate::components::checkbox(ui, a, &mut pick.show_small, tr!("Show small ones"));
        });
    });
    // What is ticked and what it pays (R-5): short in `accent`, never red.
    let ticked: Vec<&Coin> = listed
        .iter()
        .filter(|c| pick.ticked.contains(&c.outpoint))
        .collect();
    let total: Duffs = ticked.iter().map(|c| c.value).sum();
    let planned = (!ticked.is_empty() && amount > 0).then(|| plan(&ticked, amount, false, fees));
    let gone = pick.gone(&pick.ticked);
    let mut reason = None;
    if let Some(outpoint) = &gone {
        spent_banner(ui, a, &pick.coin_name(outpoint));
        reason = Some(tr!("Check the selection.").to_owned());
    }
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(txt(
                    &format!(
                        tr!("Chosen {} of {} · {}"),
                        ticked.len(),
                        listed.len(),
                        crate::format::amount(total)
                    ),
                    13.0,
                    Weight::SemiBold,
                    p.text,
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &planned {
                    Some(Err(short)) => {
                        ui.label(txt(
                            &format!(tr!("{} short"), crate::format::amount(*short)),
                            12.0,
                            Weight::Medium,
                            p.accent,
                        ));
                    }
                    _ if !ticked.is_empty() => {
                        // «2 addresses», by the language's plural (CLDR); the payment only
                        // once there is an amount.
                        let addresses = crate::i18n::t(&crate::text::plural(
                            CoinPick::addresses(&ticked) as u32,
                            tr!("address"),
                            tr!("addresses"),
                        ))
                        .into_owned();
                        let line = if amount > 0 {
                            format!(
                                "{addresses} · {}",
                                crate::i18n::t(&format!(
                                    tr!("payment {} + fee"),
                                    crate::format::amount(amount)
                                ))
                            )
                        } else {
                            addresses
                        };
                        ui.label(
                            egui::RichText::new(line)
                                .font(sans(12.0, Weight::Regular))
                                .color(p.muted),
                        );
                    }
                    _ => {}
                });
            });
        });
    if reason.is_none() {
        if pick.ticked.len() > MAX_COINS {
            reason = Some(tr!("At most 500 coins in one payment: untick some.").into());
        } else if matches!(planned, Some(Err(_))) {
            reason = Some(tr!("Tick more coins or lower the amount.").into());
        } else if ticked.is_empty() {
            reason = Some(tr!("Check the selection.").into());
        }
    }
    // The list: largest first by default; small ones hidden unless ticked (R-13).
    let mut shown: Vec<&Coin> = listed
        .iter()
        .filter(|c| pick.show_small || c.value >= SMALL || pick.ticked.contains(&c.outpoint))
        .collect();
    match pick.order {
        Order::Largest => shown.sort_by_key(|c| std::cmp::Reverse(c.value)),
        Order::Smallest => shown.sort_by_key(|c| c.value),
        Order::Newest => shown.sort_by_key(|c| std::cmp::Reverse(c.at)),
        Order::Oldest => shown.sort_by_key(|c| c.at),
    }
    // A ticked coin that is gone comes first, where its mark is seen (CC4).
    if let Some(outpoint) = &gone {
        shown.sort_by_key(|c| &c.outpoint != outpoint);
    }
    let now = chrono::Local::now();
    egui::ScrollArea::vertical()
        // The list scrolls; the bar, the buttons stay in the window (AC-D8).
        .max_height((ui.ctx().content_rect().height() - 430.0).clamp(120.0, 320.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for coin in shown {
                coin_row(
                    ui,
                    a,
                    pick,
                    coin,
                    gone.as_deref() == Some(&coin.outpoint),
                    now,
                );
            }
        });
    reason
}

/// `ab12cd…:1`.
pub(super) fn short_outpoint(outpoint: &str) -> String {
    match outpoint.split_once(':') {
        Some((txid, vout)) if txid.len() > 10 => format!("{}…:{vout}", &txid[..8]),
        _ => outpoint.to_owned(),
    }
}

/// One coin of the list: a tick, the amount, where it is and when, its state. Only a final coin
/// can be ticked (R-4); a ticked one is on `raised`.
fn coin_row(
    ui: &mut Ui,
    a: &mut Assets,
    pick: &mut CoinPick,
    coin: &Coin,
    spent: bool,
    now: chrono::DateTime<chrono::Local>,
) {
    let p = a.p;
    let can = coin.state == CoinState::Final;
    let ticked = pick.ticked.contains(&coin.outpoint);
    let fill = if ticked {
        p.raised
    } else {
        egui::Color32::TRANSPARENT
    };
    let stroke = if spent {
        egui::Stroke::new(1.0, p.accent)
    } else {
        egui::Stroke::NONE
    };
    let frame = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let mut on = ticked;
                if !can {
                    ui.disable();
                }
                let text_color = if can { p.text } else { p.muted };
                let shown = format!(
                    "{} {}",
                    crate::format::number(coin.value, false),
                    crate::format::unit_name()
                );
                let shown = if crate::format::display().hide {
                    crate::format::mask(&shown)
                } else {
                    shown
                };
                // A screen reader hears the coin by its amount and address (TEAM 2.1a).
                let tick = tick_box(ui, a, &mut on, can);
                crate::components::named_choice(
                    &tick,
                    egui::WidgetType::Checkbox,
                    on,
                    &format!("{shown} · {}", crate::text::short_address(&coin.address)),
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(
                        egui::RichText::new(shown)
                            .font(mono(14.0, Weight::Medium))
                            .color(text_color),
                    );
                    let when = coin
                        .at
                        .map_or_else(|| "—".to_owned(), |at| crate::text::date_short(at, now));
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · {} · {when}",
                            crate::text::short_address(&coin.address),
                            whose(coin)
                        ))
                        .font(mono(12.0, Weight::Regular))
                        .color(p.muted),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (icon, words) = if spent {
                        ("warning", tr!("already spent"))
                    } else {
                        state_words(coin.state)
                    };
                    ui.label(txt(words, 12.0, Weight::Medium, p.muted));
                    ui.add(a.icons.icon(icon, p.muted, &p, 14.0));
                });
                if on != ticked && can {
                    if on {
                        pick.ticked.insert(coin.outpoint.clone());
                    } else {
                        pick.ticked.remove(&coin.outpoint);
                    }
                }
            });
        });
    let _ = frame;
}

/// The last look's «Inputs: k of n» block and what follows (P1): the coins taken, the ticked
/// ones not needed muted, the change, the addresses that become linked.
pub(super) fn confirm_inputs(
    ui: &mut Ui,
    a: &mut Assets,
    form: &SendForm,
    plan: &coin_plan::CoinPlan,
) {
    let p = a.p;
    let mut chosen = form.coins.chosen_coins();
    chosen.sort_by_key(|c| std::cmp::Reverse(c.value));
    let (taken, rest) = chosen.split_at(plan.taken.min(chosen.len()));
    egui::Frame::new()
        .fill(p.bg)
        .stroke(egui::Stroke::new(1.0, p.line))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            kv_row(
                ui,
                &p,
                &format!(tr!("Inputs: {} of {}"), taken.len(), chosen.len()),
                &crate::format::amount(plan.inputs),
                mono(13.0, Weight::Medium),
            );
            for coin in taken {
                input_line(ui, &p, coin, None);
            }
            for coin in rest {
                input_line(ui, &p, coin, Some(tr!("not needed, stays in the wallet")));
            }
        });
}

fn input_line(ui: &mut Ui, p: &crate::theme::Palette, coin: &Coin, note: Option<&str>) {
    let color = if note.is_some() {
        p.muted.gamma_multiply(0.7)
    } else {
        p.muted
    };
    // The amount first, on the right: the words take what is left and end in «…» (P1). One
    // line high: a layout given the rest of the dialog would take all of it.
    let size = egui::vec2(ui.available_width(), 18.0);
    ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
        ui.label(
            egui::RichText::new(crate::format::amount(coin.value))
                .font(mono(12.0, Weight::Regular))
                .color(color),
        );
        ui.add_space(8.0);
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "{} · {}",
                        crate::text::short_address(&coin.address),
                        note.map_or_else(|| whose(coin), |n| crate::i18n::t(n).into_owned())
                    ))
                    .font(mono(12.0, Weight::Regular))
                    .color(color),
                )
                .truncate(),
            );
        });
    });
}

/// A tick box as the app draws one (`components::checkbox`): an outline when off, `ink` with a
/// tick when on; dimmed when it cannot be ticked.
fn tick_box(ui: &mut Ui, a: &mut Assets, on: &mut bool, enabled: bool) -> egui::Response {
    let p = a.p;
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), sense);
    let dim = |c: egui::Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    if *on {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(6), dim(p.ink));
        a.icons.icon("check", p.on_ink, &p, 14.0).paint_at(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0)),
        );
    } else {
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(6),
            p.raised,
            egui::Stroke::new(1.0, dim(p.muted)),
            egui::StrokeKind::Inside,
        );
    }
    if resp.clicked() {
        *on = !*on;
    }
    resp.on_hover_cursor(if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::NotAllowed
    })
}

/// «The coin … is already spent» (CC4): the warning icon, the `accent` border.
pub(super) fn spent_banner(ui: &mut Ui, a: &mut Assets, coin: &str) {
    let p = a.p;
    egui::Frame::new()
        .fill(p.raised)
        .stroke(egui::Stroke::new(1.0, p.accent))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(a.icons.icon("warning", p.accent, &p, 18.0));
                ui.add(
                    egui::Label::new(txt(
                        &format!(
                            tr!(
                                "The coin {} is already spent, perhaps by a payment from another device. Untick it and check the selection before you go on."
                            ),
                            coin
                        ),
                        13.0,
                        Weight::Regular,
                        p.text,
                    ))
                    .wrap(),
                );
            });
        });
}

/// What the network learns from the coins taken (R-8): their addresses linked; a contact's
/// coin tells them the other address is ours. None for a single address.
pub(super) fn linked_note(coins: &[&Coin]) -> Option<String> {
    let addresses = CoinPick::addresses(coins);
    let mut lines = Vec::new();
    if addresses > 1 {
        // The count's own plural form (QA pass 21: «7 адресов», not «7 адреса»).
        lines.push(crate::text::plural(
            addresses as u32,
            tr!("address will be linked on the network: anyone will see it has one owner."),
            tr!("addresses will be linked on the network: anyone will see they have one owner."),
        ));
        if let Some(who) = coins.iter().find_map(|c| c.from_contact.as_deref()) {
            lines.push(format!(
                tr!("One coin came from @{}: they will learn that the other address is yours too."),
                who.trim_start_matches('@')
            ));
        }
    }
    // Each sentence in the interface language: the two together are no catalog key.
    (!lines.is_empty()).then(|| {
        lines
            .iter()
            .map(|l| crate::i18n::t(l).into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// The plan of a payment of `amount` from `coins` with the wallet's fees: how many it takes,
/// the fee and the change, or how much is short (R-5, R-7).
pub(super) fn plan(
    coins: &[&Coin],
    amount: Duffs,
    all: bool,
    fees: &evolite_api::Fees,
) -> Result<coin_plan::CoinPlan, Duffs> {
    let values: Vec<Duffs> = coins.iter().map(|c| c.value).collect();
    coin_plan::coin_plan(&values, amount, all, |k| fees.core_fee_for_inputs(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coin(n: u8, value: Duffs, state: CoinState) -> Coin {
        Coin {
            outpoint: format!("{n:02x}:0"),
            value,
            address: format!("y{n}"),
            from_contact: None,
            state,
            at: None,
        }
    }

    /// QA pass 21 (5): «Эти 7 адресов», not «7 адреса» — the count's own plural form.
    #[test]
    fn the_linked_addresses_take_the_plural_of_their_count() {
        let coins: Vec<Coin> = (1..=7).map(|n| coin(n, 10, CoinState::Final)).collect();
        let refs: Vec<&Coin> = coins.iter().collect();
        crate::i18n::set(crate::i18n::Lang::Ru);
        let seven = linked_note(&refs).unwrap();
        let two = linked_note(&refs[..2]).unwrap();
        crate::i18n::set(crate::i18n::Lang::En);
        assert!(seven.starts_with("Эти 7 адресов будут связаны"), "{seven}");
        assert!(two.starts_with("Эти 2 адреса будут связаны"), "{two}");
    }

    #[test]
    fn only_the_last_asks_answer_is_taken() {
        let mut form = SendForm::empty();
        let _ = form.coins.ask_again();
        let ask = form.coins.ask;
        form.on_coins(ask - 1, &Ok(vec![coin(1, 1, CoinState::Final)]));
        assert!(form.coins.coins.is_none(), "an earlier ask's answer");
        form.on_coins(ask, &Ok(vec![coin(1, 1, CoinState::Final)]));
        assert_eq!(form.coins.listed().len(), 1);
    }

    #[test]
    fn a_chosen_coin_spent_or_not_final_is_said() {
        let mut pick = CoinPick {
            coins: Some(Ok(vec![
                coin(1, 10, CoinState::Final),
                coin(2, 20, CoinState::Reserved),
            ])),
            ..Default::default()
        };
        pick.chosen = ["01:0".to_owned()].into();
        assert_eq!(pick.gone(&pick.chosen.clone()), None);
        pick.chosen = ["01:0".to_owned(), "02:0".to_owned()].into();
        assert_eq!(pick.gone(&pick.chosen.clone()), Some("02:0".into()));
        pick.chosen = ["03:0".to_owned()].into();
        assert_eq!(pick.gone(&pick.chosen.clone()), Some("03:0".into()));
        pick.reset();
        assert!(pick.choice(false).is_none());
    }
}
