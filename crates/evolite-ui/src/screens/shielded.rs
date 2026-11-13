//! 08 · Private pool: balance in the pool (can be hidden), how it works, four actions and
//! operations that only the owner sees — all from the core's snapshot.

use super::txt;
use crate::components::{
    Assets, ButtonKind, CardAmount, GridSpec, Part, RowCell, SpendPassword, TxRow, amount_field,
    button, button_enabled, button_row, card, dialog_with_footer, error_notice, field_label, grid,
    grid_layout, inner, kv_row, list_card, notice, page_header, password_asked, progress_bar,
    segmented_with_off, select_row, tx_row,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::{TILE_MIN_W, Weight, mono, sans};
use egui::{
    Align, Color32, CornerRadius, CursorIcon, Layout, Margin, Rect, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use evolite_api::{CoreError, DoneWhat, Duffs, Fees, Pool, ShieldSource, Source, Spend, Wallet};

/// (icon, title, line under it).
const ACTIONS: [(&str, &str, &str); 4] = [
    (
        "shield",
        // Not "Hide": "Hide the amounts" on the same screen only hides digits.
        tr!("Shield"),
        tr!("from Core, an identity or a Platform address"),
    ),
    (
        "send",
        tr!("Send privately"),
        tr!("to an evolite private address"),
    ),
    (
        "receive",
        tr!("Private address"),
        tr!("to receive into the pool"),
    ),
    ("unshield", tr!("Unshield"), tr!("back to Platform")),
];

/// The hint on the pool's actions while the pool is not connected yet.
const POOL_CONNECTING: &str =
    tr!("The pool connects after syncing with Platform — usually a minute");

const HOW: [(&str, &str); 3] = [
    (
        "shield",
        tr!(
            "Shield — credits go into the pool; from then on the amount and the recipient are not visible on the network."
        ),
    ),
    (
        "transfer",
        tr!("Transfers inside the pool are visible only to those in them."),
    ),
    (
        "unshield",
        tr!("Unshield — return to a Platform address or an identity."),
    ),
];

/// Which dialog a move of the pool belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PoolMove {
    Shield,
    /// Out of the pool: to the Platform address or into the identity's credits.
    Unshield,
}

impl PoolMove {
    /// The move a core's `Done` answers.
    pub fn of(what: DoneWhat) -> Self {
        if what == DoneWhat::Shield {
            Self::Shield
        } else {
            Self::Unshield
        }
    }
}

#[derive(Default)]
pub struct PoolState {
    /// 09 · "Shield into the private pool" is open.
    pub shielding: bool,
    pub from: usize,
    pub sources_open: bool,
    pub amount: String,
    /// Empty when the dialog opens, «Max» beside it (design review №27: a 0.5 put in for the
    /// person opened the dialog with «More than there is» already on); an example only in the
    /// dev-tools presets, kept in the chosen format (`format::fill_suggested`).
    pub suggested: Option<i64>,
    /// The core is building the proof: how far it has got.
    pub progress: Option<f32>,
    /// The move running is «Continue» of a stopped shield, not a shield started now: what it
    /// moves is the lock, not the amount typed (BUG-101).
    pub continuing: bool,
    /// The core's last answer, and the move it answers: the dialogs share this, and one
    /// dialog's answer is not the other's (BUG-101).
    pub result: Option<(PoolMove, Result<(), CoreError>)>,
    /// "Unshield" is open.
    pub unshielding: bool,
    pub unshield_amount: String,
    /// An example for `unshield_amount` (dev-tools presets), kept the same way.
    pub unshield_suggested: Option<i64>,
    /// "Private address" is open.
    pub address_open: bool,
    /// "Unshield" goes into the identity's credits instead of the Platform address.
    pub unshield_to_identity: bool,
    /// "Password for every payment" in "Shield" and "Unshield" (one move at a time; BUG-42).
    pub password: SpendPassword,
}

impl PoolState {
    /// `move_`'s answer is gone; the other dialog's stays.
    fn forget_result(&mut self, move_: PoolMove) {
        if self.result.as_ref().is_some_and(|(of, _)| *of == move_) {
            self.result = None;
        }
    }
}

/// The answer to `move_`, if the last answer is to it.
fn result_of(
    result: &Option<(PoolMove, Result<(), CoreError>)>,
    move_: PoolMove,
) -> Option<&Result<(), CoreError>> {
    result
        .as_ref()
        .filter(|(of, _)| *of == move_)
        .map(|(_, r)| r)
}

/// Where funds can be shielded from: the identity's credits, the Platform address or Core.
fn shield_sources(w: &Wallet) -> Vec<(ShieldSource, &'static str, String, i64)> {
    let name = w.identity.as_ref().and_then(|i| i.name()).map_or_else(
        || tr!("Identity · credits").to_string(),
        |n| format!(tr!("@{n} · credits"), n = n),
    );
    vec![
        (ShieldSource::Credits, "identity", name, w.balances.credits),
        (
            ShieldSource::PlatformAddress,
            "platform-address",
            tr!("Platform address").to_string(),
            w.balances.platform_address,
        ),
        (
            ShieldSource::Core,
            "core",
            "Core".to_string(),
            w.balances.core,
        ),
    ]
}

pub fn shielded(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut PoolState) {
    let wallet = cx.wallet();
    let now = cx.now;
    // Moves need the pool connected: its private address is known then.
    let connected = wallet.pool.address.is_some();
    ui.spacing_mut().item_spacing.y = 22.0;
    page_header(ui, a, tr!("Private pool"), None, |ui, a| {
        if button_enabled(
            ui,
            a,
            tr!("Shield funds"),
            ButtonKind::Primary,
            None,
            connected,
        )
        .on_disabled_hover_text(crate::i18n::t(POOL_CONNECTING).into_owned())
        .clicked()
        {
            state.shielding = true;
        }
        let can_unshield = connected && wallet.balances.shielded > 0;
        if button_enabled(
            ui,
            a,
            tr!("Unshield"),
            ButtonKind::Secondary,
            None,
            can_unshield,
        )
        .on_disabled_hover_text(
            crate::i18n::t(if connected {
                tr!("The pool is empty for now")
            } else {
                POOL_CONNECTING
            })
            .into_owned(),
        )
        .clicked()
        {
            state.unshielding = true;
        }
    });

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 22.0;
            let total = ui.available_width();
            let wide = total >= 760.0;
            // One grid for the cards and the actions: halves with a gap of 20, the actions in
            // the same columns (design review 2026-11-05 №7).
            let half = if wide { (total - 20.0) / 2.0 } else { total };
            crate::components::two_columns(ui, wide, |ui, first, cell| {
                if first {
                    let pool = (wallet.balances.shielded, &wallet.pool);
                    balance(ui, a, pool, now, half, cell);
                } else {
                    how(ui, a, half, cell);
                }
            });

            // Four actions in a row while each is at least a tile wide, else two by two, else
            // one under another (responsive spec R4). The gap is the halves' 20: four split
            // the halves in two, two stand in them.
            let spec = GridSpec {
                min_w: TILE_MIN_W,
                gap: vec2(20.0, 12.0),
                max_cols: 4,
                balanced: true,
            };
            let (_, w) = grid_layout(total, ACTIONS.len(), spec);
            // One height for all: the tallest line under a title decides (one or two lines).
            let h = ACTIONS
                .iter()
                .map(|(_, title, sub)| action_height(ui, title, sub, w))
                .fold(62.0, f32::max);
            grid(ui, ACTIONS.len(), spec, |ui, i, w, _| {
                let (icon, title, sub) = ACTIONS[i];
                let resp = action(ui, a, icon, title, sub, vec2(w, h), connected);
                if !connected {
                    resp.on_hover_text(crate::i18n::t(POOL_CONNECTING).into_owned());
                    return;
                }
                if !resp.clicked() {
                    return;
                }
                match icon {
                    "shield" => state.shielding = true,
                    "send" => cx.pay(Source::Pool, String::new()),
                    "receive" => state.address_open = true,
                    _ => state.unshielding = true,
                }
            });

            list_card(
                ui,
                a,
                total,
                tr!("Operations in the pool"),
                tr!("visible only to you"),
                |ui, a| {
                    ui.add_space(-2.0);
                    // An empty card says so (BUG-39 №12).
                    if wallet.pool.operations.is_empty() {
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            ui.label(txt(
                                if wallet.pool.checked_at.is_some() {
                                    tr!("No operations in the pool yet")
                                } else {
                                    tr!("Still reading the operations — they appear here as soon as they are found")
                                },
                                14.0,
                                Weight::Regular,
                                a.p.muted,
                            ));
                        });
                        ui.add_space(14.0);
                    }
                    for op in &wallet.pool.operations {
                        // A note found by reading the pool has no time of its own.
                        let detail = if text::known(op.at) {
                            format!(
                                "{} · {}",
                                text::day(op.at, now),
                                text::time_of_day(op.at, now)
                            )
                        } else {
                            tr!("found in the pool · time unknown").to_string()
                        };
                        // «Hide amounts» masks it in `tx_row`.
                        let amount = if op.amount == 0 {
                            // Found by reading the pool: what left is not known.
                            "—".to_string()
                        } else {
                            crate::text::signed_amount(op.amount)
                        };
                        // Only the owner sees these, so a network status would say nothing.
                        tx_row(
                            ui,
                            a,
                            &TxRow {
                                icon: text::activity_icon(op),
                                title: &text::activity_title(op),
                                detail: &detail,
                                proof: None,
                                amount: &amount,
                                alert: None,
                                quiet: false,
 core_chip: false,
                                note: op.note.as_deref(),
                                fiat: None,
                            },
                        );
                    }
                },
            );
        });
    if state.shielding {
        shield_dialog(ui, a, cx, state);
    }
    if state.unshielding {
        unshield_dialog(ui, a, cx, state);
    }
    if state.address_open
        && let Some(address) = &wallet.pool.address
    {
        address_dialog(ui, a, state, address);
    }
}

fn balance(
    ui: &mut Ui,
    a: &mut Assets,
    (duffs, pool): (i64, &Pool),
    now: chrono::DateTime<chrono::Local>,
    width: f32,
    cell: &mut RowCell,
) {
    let p = a.p;
    cell.card(ui, card(&p, Margin::same(24)), |ui| {
        ui.set_width(inner(width, 24));
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add(a.icons.icon("shielded", p.muted, &p, 18.0));
            ui.label(txt(tr!("In the pool"), 13.0, Weight::Medium, p.muted));
            // The app's one «Hide amounts», not a switch of this card's own (FEAT-0007).
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                crate::components::amount_toggle(ui, a, false);
            });
        });
        // Not read in this run: the store's last amount dim, or «—» — a 0 before the first read
        // is not known (CS-4, as the Overview's pool tile, BUG-66).
        let amount = super::overview::pool_amount(duffs, pool.checked_at);
        let font = mono(36.0, Weight::Medium);
        match amount.known() {
            Some(_) => {
                let color = amount.color(&p);
                crate::components::card_amount_fitted(ui, amount, font, color, inner(width, 24));
            }
            None => {
                // «— DASH» (the spec, CS-4): what is not known, in the unit it will be in.
                let unknown = format!("— {}", crate::format::unit_name());
                ui.label(
                    egui::RichText::new(unknown)
                        .font(font)
                        .color(amount.color(&p)),
                );
            }
        }
        // Mono sets Arabic letters apart («3 سجلات · آخر تحقق…», BUG-118): sans in Arabic.
        ui.label(
            egui::RichText::new(crate::i18n::t(&pool_line(pool, now)))
                .font(crate::theme::figures(12.0, Weight::Regular))
                .color(p.muted),
        );
    });
}

fn how(ui: &mut Ui, a: &mut Assets, width: f32, cell: &mut RowCell) {
    let p = a.p;
    cell.card(ui, card(&p, Margin::same(24)), |ui| {
        ui.set_width(inner(width, 24));
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.label(txt(tr!("How it works"), 15.0, Weight::SemiBold, p.text));
        for (icon, text) in HOW {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(a.icons.icon(icon, p.text, &p, 18.0));
                ui.add(
                    crate::components::para(text, 13.0, Weight::Regular, p.text).line_height(19.0),
                );
            });
        }
    });
}

/// The words of a quick action laid out in a tile `width` wide: the title and the line under
/// it, each wrapped at the tile's inner right edge.
pub(crate) struct ActionText {
    pub title: std::sync::Arc<egui::Galley>,
    pub sub: std::sync::Arc<egui::Galley>,
    /// From the tile's left edge to where the words start.
    pub left: f32,
}

impl ActionText {
    pub fn new(
        ctx: &egui::Context,
        title: &str,
        sub: &str,
        width: f32,
        color: (Color32, Color32),
    ) -> Self {
        let left = 16.0 + 34.0 + 12.0;
        // 16 to the right edge, as on the left (FEAT-0034 item 5).
        let text_w = width - left - 16.0;
        // The lines are measured outside `fonts_mut`: `lines` takes the fonts itself.
        let laid = |text: &str, font: egui::FontId, color: Color32| {
            let shown = crate::components::lines(ctx, text, &font, text_w);
            ctx.fonts_mut(|f| f.layout(shown, font, color, text_w))
        };
        let title = laid(title, sans(14.0, Weight::SemiBold), color.0);
        let sub = laid(sub, sans(12.0, Weight::Regular), color.1);
        ActionText { title, sub, left }
    }

    /// Padding 14 + the title's line(s) + gap 2 + the wrapped line(s) + padding 14.
    pub fn height(&self) -> f32 {
        28.0 + self.title.size().y + 2.0 + self.sub.size().y
    }
}

/// The quick actions of the pool with their titles and lines, for a test.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub(crate) fn actions() -> impl Iterator<Item = (&'static str, &'static str)> {
    ACTIONS.iter().map(|(_, title, sub)| (*title, *sub))
}

/// A quick action: surface, radius 14, icon in a raised 34 px box, title and a line under it.
fn action_height(ui: &Ui, title: &str, sub: &str, width: f32) -> f32 {
    ActionText::new(
        ui.ctx(),
        title,
        sub,
        width,
        (Color32::WHITE, Color32::WHITE),
    )
    .height()
}

fn action(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    title: &str,
    sub: &str,
    size: egui::Vec2,
    enabled: bool,
) -> egui::Response {
    let p = a.p;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    crate::components::named(&resp, egui::WidgetType::Button, enabled, title);
    let fill = if enabled && resp.hovered() {
        p.raised
    } else {
        p.surface
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(14),
        fill,
        Stroke::new(1.0, p.line),
        StrokeKind::Inside,
    );
    let boxr = Rect::from_min_size(
        pos2(rect.left() + 16.0, rect.center().y - 17.0),
        vec2(34.0, 34.0),
    );
    crate::components::paint_icon_box(ui, a, boxr, 12, icon, 18.0, p.ink);
    let words = ActionText::new(ui.ctx(), title, sub, rect.width(), (p.text, p.muted));
    let x = rect.left() + words.left;
    let painter = ui.painter();
    let title_h = words.title.size().y;
    // The title and its line stand on the tile's middle as one block, as the icon does: in a
    // row as tall as its tallest tile a short one's words do not hug the top (designer's r1).
    let top = rect.center().y - (title_h + 2.0 + words.sub.size().y) / 2.0;
    painter.galley(pos2(x, top), words.title, p.text);
    painter.galley(pos2(x, top + title_h + 2.0), words.sub, p.muted);
    if !enabled {
        // Faded like a disabled button.
        ui.painter()
            .rect_filled(rect, CornerRadius::same(14), p.bg.gamma_multiply(0.45));
        return resp;
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// 09 · Shield into the private pool: where from, how much, the fee, and the proof being built.
fn shield_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut PoolState) {
    crate::format::fill_suggested(&mut state.amount, &mut state.suggested);
    let wallet = cx.wallet();
    let sources = shield_sources(wallet);
    let fees = &wallet.fees;
    // Core as sure as it is (BUG-80): «Not known yet» instead of a 0, and no «From … to …» to
    // measure an amount against a balance not known.
    let now = cx.now;
    let core = CardAmount::core(&wallet.balances);
    let core_unknown =
        |source: &ShieldSource| *source == ShieldSource::Core && core.known().is_none();
    // Platform not read yet (BUG-91): «—», never a 0, and such a row cannot be chosen.
    let unread = |source: &ShieldSource| match source {
        ShieldSource::Credits => !wallet.balances.credits_read,
        ShieldSource::PlatformAddress => !wallet.balances.platform_address_read,
        ShieldSource::Core => false,
    };
    let shown = |source: &ShieldSource, balance: Duffs| match source {
        ShieldSource::Core => text::core_amount(&wallet.balances, core, now),
        _ => CardAmount::platform(!unread(source), balance).balance(),
    };
    // What the middle planned, for the footer's button: whether it continues a stopped shield,
    // whether there is something to shield, how much and from where.
    let mut planned = None;
    let closed = dialog_with_footer(
        ui,
        a,
        "shield-dialog",
        "shield",
        tr!("Shield into the private pool"),
        462.0,
        |ui, a, part| {
            let p = a.p;
            let busy = state.progress.is_some();
            if part == Part::Footer {
                // The footer (BUG-122): the password and the buttons, always in the window.
                let Some((resume, can, amount, source)) = planned else {
                    return false;
                };
                let mut close = false;
                let ask = password_asked(cx.snap) && !busy;
                let entered = state.password.field(ui, &p, "shield-password", ask);
                crate::components::button_row_spend(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let ready = !busy && can && state.password.ready(ask);
                    let label = match (busy, resume) {
                        (true, _) => tr!("Shielding…"),
                        (false, true) => tr!("Continue"),
                        (false, false) => tr!("Shield"),
                    };
                    if (button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked()
                        || (entered && ready))
                        && let Some(amount) = amount
                    {
                        let op = Spend::Shield {
                            from: source,
                            amount,
                        };
                        state.continuing = resume;
                        cx.send(state.password.command(op, ask));
                        state.progress = Some(0.0);
                        state.result = None;
                    }
                });
                return close;
            }
            let (source, _, _, balance) = &sources[state.from.min(sources.len() - 1)];
            shield_source_rows(ui, a, state, &sources, busy, &shown, &unread);
            // A move running or just done has its coins gone already (BUG-101): its typed amount
            // is not measured against what is left, and a lock the core remembers for it is
            // not a stopped shield.
            let judged = can_judge(busy, result_of(&state.result, PoolMove::Shield));
            // A shield from Core cut short: its lock is finished, never a new one made.
            let resume = match source {
                ShieldSource::Core if judged || state.continuing => wallet.pool.unfinished_shield,
                _ => None,
            };
            let cost = shield_cost(fees, *source, *balance);
            let amount = match resume {
                Some(locked) => Some(locked),
                None => crate::format::parse_amount(&state.amount).filter(|d| *d > 0),
            };
            let preview = match (resume, amount) {
                // Already locked and paid for on Core: only Platform's fee is still to come.
                (Some(locked), _) => Some((*balance, locked - fees.shield_from_core)),
                (None, Some(d)) => shield_preview(fees, *source, *balance, d),
                _ => None,
            };
            if let Some(locked) = resume.filter(|_| !busy) {
                notice(
                    ui,
                    a,
                    "pending",
                    p.accent,
                    &format!(
                        tr!(
                            "A stopped shielding: {} are already locked on Core. “Continue” takes it to the pool — no new money is locked."
                        ),
                        crate::format::amount(locked)
                    ),
                );
            } else if resume.is_none() {
                let say_limits = judged
                    && amount.is_some()
                    && preview.is_none()
                    && !core_unknown(source)
                    && !unread(source);
                shield_amount_field(ui, &p, state, &cost, *source, say_limits);
            }
            // From credits not checked in this run, what is left is not said (BUG-116).
            let checked = wallet.balances.credits_checked;
            shield_cost_lines(ui, &p, &cost, source, resume, preview, checked);
            if let Some(fraction) = state.progress {
                proof_progress(ui, a, fraction);
            }
            shield_answer(ui, a, state);
            planned = Some((resume.is_some(), preview.is_some(), amount, *source));
            false
        },
    );
    if closed {
        state.shielding = false;
        state.sources_open = false;
        state.forget_result(PoolMove::Shield);
        state.password.forget();
    }
}

/// «From»: the source chosen and, opened, the others to choose from.
fn shield_source_rows(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut PoolState,
    sources: &[(ShieldSource, &'static str, String, i64)],
    busy: bool,
    shown: &impl Fn(&ShieldSource, Duffs) -> String,
    unread: &impl Fn(&ShieldSource) -> bool,
) {
    let p = a.p;
    let (source, icon, title, balance) = &sources[state.from.min(sources.len() - 1)];
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            field_label(ui, &p, tr!("From"));
            // The app's one «Hide amounts», at the sources (BUG-73).
            crate::components::amount_toggle(ui, a, true);
        });
        let value = shown(source, *balance);
        if select_row(ui, a, icon, title, Some(&value)).clicked() && !busy {
            state.sources_open = !state.sources_open;
        }
        if state.sources_open {
            for (i, (source, icon, title, balance)) in sources.iter().enumerate() {
                let value = shown(source, *balance);
                if i != state.from
                    && select_row(ui, a, icon, title, Some(&value)).clicked()
                    && !unread(source)
                {
                    state.from = i;
                    state.sources_open = false;
                    state.forget_result(PoolMove::Shield);
                }
            }
        }
    });
}

/// The amount to shield, and the limits it has to keep (`say_limits`: it does not keep them).
fn shield_amount_field(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    state: &mut PoolState,
    cost: &ShieldCost,
    source: ShieldSource,
    say_limits: bool,
) {
    let p = *p;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        field_label(ui, &p, tr!("Amount"));
        let field = amount_field(ui, &p, "shield-amount", &mut state.amount, true);
        if field.all {
            state.amount = crate::format::input_text(cost.max);
        }
        forget_success(state, PoolMove::Shield, &field);
        if say_limits {
            let why = match source {
                ShieldSource::Credits => tr!("More than there is in this balance").to_string(),
                // platform-wallet keeps a reserve for the fee on the first address.
                ShieldSource::PlatformAddress => format!(
                    tr!("At most {}: a reserve for the fee stays at the address"),
                    crate::format::amount(cost.max)
                ),
                ShieldSource::Core => format!(
                    tr!("From {} to {}: the Platform fee comes out of the amount"),
                    crate::format::amount(cost.min),
                    crate::format::amount(cost.max)
                ),
            };
            ui.label(txt(&why, 12.0, Weight::Regular, p.danger));
        }
    });
}

/// The fees, what is left and what arrives.
fn shield_cost_lines(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    cost: &ShieldCost,
    source: &ShieldSource,
    resume: Option<Duffs>,
    preview: Option<(Duffs, Duffs)>,
    credits_checked: bool,
) {
    let p = *p;
    let value = mono(13.0, Weight::Medium);
    kv_row(
        ui,
        &p,
        tr!("Platform fee"),
        &format!("≈ {}", crate::format::amount(cost.fee)),
        value.clone(),
    );
    // From Core the asset lock is a Core transaction with its own fee: a line of its own,
    // so "Platform fee" is the amount less what arrives in the pool. A stopped shield's lock
    // is already paid for.
    if cost.core_fee > 0 && resume.is_none() {
        kv_row(
            ui,
            &p,
            tr!("Core fee"),
            &format!("≈ {}", crate::format::amount(cost.core_fee)),
            value.clone(),
        );
    }
    let left_label = match source {
        ShieldSource::Credits => tr!("Left in credits"),
        ShieldSource::PlatformAddress => tr!("Left at the address"),
        ShieldSource::Core => tr!("Left on Core"),
    };
    // Platform meters a shield from addresses, and Core's fee depends on the coins: an
    // estimate there; from credits the rest is exact.
    let left_text = preview.map_or("—".to_string(), |(left, _)| match source {
        ShieldSource::Credits if !credits_checked => "—".to_string(),
        ShieldSource::Credits => crate::format::balance(left),
        _ => format!("≈ {}", crate::format::balance(left)),
    });
    kv_row(ui, &p, left_label, &left_text, value.clone());
    // From Core Platform's fee comes out of the lock: less reaches the pool.
    if *source == ShieldSource::Core {
        let arrives = preview.map_or("—".to_string(), |(_, arrives)| {
            crate::format::amount(arrives)
        });
        kv_row(ui, &p, tr!("Arrives in the pool"), &arrives, value);
    }
}

/// The answer to the shield, unless it was the password that was wrong.
fn shield_answer(ui: &mut Ui, a: &mut Assets, state: &mut PoolState) {
    let p = a.p;
    // A wrong password is said under its field, not as a failed move.
    state
        .password
        .settle(result_of(&state.result, PoolMove::Shield));
    let shown = result_of(&state.result, PoolMove::Shield).filter(|_| !state.password.is_wrong());
    match shown {
        Some(Err(CoreError::NotAvailable(why))) => notice(
            ui,
            a,
            "info",
            p.muted,
            &format!(tr!("{why}. Nothing went into the pool."), why = why),
        ),
        Some(Err(e)) => error_notice(
            ui,
            a,
            &crate::text::failed(tr!("Not shielded"), e),
            crate::text::error_details(e).as_deref(),
        ),
        Some(Ok(())) => notice(
            ui,
            a,
            "confirmed",
            p.accent,
            tr!("The funds are in the pool."),
        ),
        None => {}
    }
}

/// The proof being built: shared by "Hide" and "Unshield".
fn proof_progress(ui: &mut Ui, a: &mut Assets, fraction: f32) {
    let p = a.p;
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add(a.icons.icon("sync", p.text, &p, 16.0));
                ui.label(txt(
                    tr!("Building the proof on this computer"),
                    13.0,
                    Weight::SemiBold,
                    p.text,
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(crate::i18n::t(&format!(
                            "{} %",
                            (fraction * 100.0).round()
                        )))
                        .font(mono(12.0, Weight::Regular))
                        .color(p.muted),
                    );
                });
            });
            progress_bar(ui, &p, fraction);
            ui.add(crate::components::para(
                tr!("Usually 10–30 seconds. The keys and the amount never leave the device."),
                12.0,
                Weight::Regular,
                p.muted,
            ));
        });
}

/// "Unshield": out of the pool to the wallet's own Platform address, or into the credits of the
/// identity the interface shows.
fn unshield_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut PoolState) {
    crate::format::fill_suggested(&mut state.unshield_amount, &mut state.unshield_suggested);
    let wallet = cx.wallet();
    let balance = wallet.balances.shielded;
    let identity = wallet.identity.as_ref().map(|i| {
        i.name()
            .map_or_else(|| "identity".to_string(), |n| format!("@{n}"))
    });
    // Without an identity only the Platform address is there.
    let to_identity = state.unshield_to_identity && identity.is_some();
    let fee = unshield_fee(&wallet.fees, to_identity);
    // What the middle reckoned, for the footer's button: whether the amount fits, and which.
    let mut planned = None;
    let closed = dialog_with_footer(
        ui,
        a,
        "unshield-dialog",
        "unshield",
        tr!("Unshield from the pool"),
        462.0,
        |ui, a, part| {
            let p = a.p;
            let busy = state.progress.is_some();
            if part == Part::Footer {
                // The footer (BUG-122): the password and the buttons, always in the window.
                let Some((fits, amount)) = planned else {
                    return false;
                };
                let mut close = false;
                let ask = password_asked(cx.snap) && !busy;
                let entered = state.password.field(ui, &p, "unshield-password", ask);
                crate::components::button_row_spend(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let ready = !busy && fits && state.password.ready(ask);
                    let label = if busy {
                        tr!("Unshielding…")
                    } else {
                        tr!("Unshield")
                    };
                    if (button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked()
                        || (entered && ready))
                        && let Some(amount) = amount
                    {
                        let op = if to_identity {
                            Spend::TopUpFromPool { amount }
                        } else {
                            Spend::Unshield { amount }
                        };
                        cx.send(state.password.command(op, ask));
                        state.progress = Some(0.0);
                        state.result = None;
                    }
                });
                return close;
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                field_label(ui, &p, tr!("Where to"));
                let mut choice = usize::from(to_identity);
                segmented_with_off(
                    ui,
                    &p,
                    &[tr!("Platform address"), tr!("Identity credits")],
                    &mut choice,
                    true,
                    &[
                        None,
                        identity
                            .is_none()
                            .then_some(tr!("The wallet has no identity yet")),
                    ],
                );
                if !busy && (choice == 1) != state.unshield_to_identity {
                    state.unshield_to_identity = choice == 1;
                    state.forget_result(PoolMove::Unshield);
                }
                let target = match &identity {
                    Some(name) if to_identity => format!(tr!("{name} · credits"), name = name),
                    _ => format!(tr!("Your Platform address · {}"), wallet.platform_address),
                };
                ui.label(
                    egui::RichText::new(crate::i18n::t(&target))
                        .font(mono(12.0, Weight::Regular))
                        .color(p.text),
                );
            });
            let amount = crate::format::parse_amount(&state.unshield_amount).filter(|d| *d > 0);
            let left = amount.map(|d| crate::format::left_after(balance, d, fee));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                field_label(ui, &p, tr!("Amount"));
                let field =
                    amount_field(ui, &p, "unshield-amount", &mut state.unshield_amount, true);
                if field.all {
                    state.unshield_amount = crate::format::input_text((balance - fee).max(0));
                }
                forget_success(state, PoolMove::Unshield, &field);
                if can_judge(busy, result_of(&state.result, PoolMove::Unshield))
                    && left.is_some_and(|l| l < 0)
                {
                    ui.label(txt(
                        tr!("More than there is in the pool"),
                        12.0,
                        Weight::Regular,
                        p.danger,
                    ));
                }
            });
            kv_row(
                ui,
                &p,
                tr!("Platform fee"),
                &format!("≈ {}", crate::format::amount(fee)),
                mono(13.0, Weight::Medium),
            );
            let left_text = left
                .filter(|l| *l >= 0)
                .map_or("—".to_string(), crate::format::balance);
            // The app's one «Hide amounts», at the one balance shown (BUG-73).
            crate::components::kv_row_eye(
                ui,
                a,
                tr!("Left in the pool"),
                &left_text,
                mono(13.0, Weight::Medium),
            );
            notice(
                ui,
                a,
                "eye",
                p.muted,
                if to_identity {
                    tr!(
                        "The unshielded amount becomes visible on the network: the identity gets all of it, the fee comes from the pool on top."
                    )
                } else {
                    tr!(
                        "The unshielded amount becomes visible on the network, like any transfer to a Platform address."
                    )
                },
            );
            if let Some(fraction) = state.progress {
                proof_progress(ui, a, fraction);
            }
            // A wrong password is said under its field, not as a failed move.
            state
                .password
                .settle(result_of(&state.result, PoolMove::Unshield));
            let shown =
                result_of(&state.result, PoolMove::Unshield).filter(|_| !state.password.is_wrong());
            match shown {
                Some(Err(CoreError::NotAvailable(why))) => notice(
                    ui,
                    a,
                    "info",
                    p.muted,
                    &format!(tr!("{why}. Nothing left the pool."), why = why),
                ),
                Some(Err(e)) => error_notice(
                    ui,
                    a,
                    &crate::text::failed(tr!("Not unshielded"), e),
                    crate::text::error_details(e).as_deref(),
                ),
                Some(Ok(())) => notice(
                    ui,
                    a,
                    "confirmed",
                    p.accent,
                    if to_identity {
                        tr!("Unshielded: the amount is in the identity's credits.")
                    } else {
                        tr!("Unshielded: the amount is on your Platform address.")
                    },
                ),
                None => {}
            }
            planned = Some((left.is_some_and(|l| l >= 0), amount));
            false
        },
    );
    if closed {
        state.unshielding = false;
        state.forget_result(PoolMove::Unshield);
        state.password.forget();
    }
}

/// Whether a typed amount is measured against the balance (BUG-101): not while the move runs nor
/// after it succeeded, when the balance is what it left behind.
fn can_judge(busy: bool, result: Option<&Result<(), CoreError>>) -> bool {
    !busy && !matches!(result, Some(Ok(())))
}

/// A new amount typed after a success: the success is over, and the amount is judged again.
fn forget_success(
    state: &mut PoolState,
    move_: PoolMove,
    field: &crate::components::AmountResponse,
) {
    let succeeded = matches!(result_of(&state.result, move_), Some(Ok(())));
    if (field.response.changed() || field.all) && succeeded {
        state.result = None;
    }
}

/// What "Unshield" costs: into the identity's credits a top-up from the pool, else an unshield
/// to the Platform address.
fn unshield_fee(fees: &Fees, to_identity: bool) -> Duffs {
    if to_identity {
        fees.pool_top_up
    } else {
        fees.unshield
    }
}

/// "Private address": the wallet's Orchard address, as a QR code and text to copy.
fn address_dialog(ui: &mut Ui, a: &mut Assets, state: &mut PoolState, address: &str) {
    let closed = dialog_with_footer(
        ui,
        a,
        "pool-address-dialog",
        "receive",
        tr!("Private address"),
        462.0,
        |ui, a, part| {
            let p = a.p;
            if part == Part::Footer {
                let mut close = false;
                button_row(ui, |ui| {
                    if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    // Copied like any address (wiped later), with "Copied" for a moment
                    // (BUG-39 №1).
                    let copy_id = egui::Id::new("pool-address-copy");
                    let label = if crate::clipboard::just_copied(ui.ctx(), copy_id) {
                        tr!("Copied")
                    } else {
                        tr!("Copy")
                    };
                    if button(ui, a, label, ButtonKind::Primary, None).clicked() {
                        crate::clipboard::copy_answered(ui.ctx(), address.to_owned(), copy_id);
                    }
                });
                return close;
            }
            ui.vertical_centered(|ui| {
                super::receive::qr_card(ui, &p, address);
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(address)
                        .font(mono(12.0, Weight::Regular))
                        .color(p.text),
                )
                .wrap(),
            );
            notice(
                ui,
                a,
                "shield",
                p.muted,
                tr!(
                    "Transfers to this address go straight into the pool: only you see the amount and the sender."
                ),
            );
            false
        },
    );
    if closed {
        state.address_open = false;
    }
}

/// "3 note": the pool's notes, one plural key for every language.
fn notes(n: u32) -> String {
    crate::text::plural(n, tr!("note"), tr!("notes"))
}

/// What a shield from one source costs and how much it can move.
#[derive(Debug, PartialEq)]
struct ShieldCost {
    /// Platform's fee: the line "Platform fee".
    fee: Duffs,
    /// Core's fee for the asset lock, paid on top (from Core only): the line "Core fee".
    core_fee: Duffs,
    min: Duffs,
    max: Duffs,
}

/// What shielding from `source` costs, and the least and most it can move out of `balance`.
/// From credits: all but the fee. From addresses: all but the reserve platform-wallet keeps on
/// the first address for Platform's metered fee. From Core: the amount is locked, the Core fee
/// is paid on top and Platform's fee comes out of the lock, so it must be more than that fee.
fn shield_cost(fees: &Fees, source: ShieldSource, balance: Duffs) -> ShieldCost {
    match source {
        ShieldSource::Credits => ShieldCost {
            fee: fees.shield,
            core_fee: 0,
            min: 1,
            max: (balance - fees.shield).max(0),
        },
        ShieldSource::PlatformAddress => ShieldCost {
            fee: fees.shield_from_address,
            core_fee: 0,
            min: 1,
            max: (balance - fees.shield_reserve).max(0),
        },
        ShieldSource::Core => ShieldCost {
            fee: fees.shield_from_core,
            core_fee: fees.core_payment,
            min: fees.shield_from_core + 1,
            max: (balance - fees.core_payment).max(0),
        },
    }
}

/// What stays at the source and what reaches the pool when `amount` is shielded; `None` when
/// the amount does not fit.
fn shield_preview(
    fees: &Fees,
    source: ShieldSource,
    balance: Duffs,
    amount: Duffs,
) -> Option<(Duffs, Duffs)> {
    let cost = shield_cost(fees, source, balance);
    if amount < cost.min || amount > cost.max {
        return None;
    }
    Some(match source {
        ShieldSource::Core => (
            balance - amount - fees.core_payment,
            amount - fees.shield_from_core,
        ),
        _ => (balance - amount - cost.fee, amount),
    })
}

/// What a payment out of the pool to `to` costs: to a private address it is a private
/// transfer, to a Platform address an unshield, to anything else (a Core address) a withdrawal.
/// Nothing typed yet: a private transfer, the pool's own move.
/// A `tdash1…` that is an evolite private address, not a Platform address: it carries 43 bytes, a
/// Platform address 21, so about 83 characters against 46.
pub(crate) fn is_private_address(to: &str) -> bool {
    to.trim().len() > 64
}

pub(crate) fn pool_fee(fees: &Fees, to: &str) -> Duffs {
    let to = to.trim();
    if to.is_empty() || to.starts_with('@') {
        return fees.pool_transfer;
    }
    if to.to_ascii_lowercase().starts_with("tdash1") {
        return if is_private_address(to) {
            fees.pool_transfer
        } else {
            fees.unshield
        };
    }
    fees.pool_withdrawal
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{notes, pool_fee, shield_cost, shield_preview, unshield_fee};
    use evolite_api::{Fees, ShieldSource};

    /// The pool's fees as the core gives them at protocol 15 (its own test checks the formulas).
    pub(crate) fn fees() -> Fees {
        Fees {
            core_payment: 226,
            core_unsignable: 0,
            core_coins: Vec::new(),
            credit_transfer: 2_000,
            credit_transfer_to_address: 20_000,
            pool_transfer: 114_140,
            nft_transfer: 30_000,
            document_write: 20_000,
            shield: 149_760,
            identity_create: 2_000,
            identity_lock_min: 241_000,
            top_up_lock_min: 50_500,
            name_regular: 1_000_000,
            name_contested: 20_000_000,
            add_key: 10_000,
            contact_request: 10_000,
            contact_request_balance: 230_000,
            profile: 15_000,
            shield_from_address: 114_140,
            shield_reserve: 228_280,
            unshield: 120_223,
            pool_withdrawal: 226_480,
            shield_from_core: 164_140,
            pool_top_up: 114_360,
            credit_withdrawal: 400_000,
            credit_withdrawal_reserve: 400_000,
        }
    }

    #[test]
    fn each_way_out_of_the_pool_has_its_own_fee() {
        let f = fees();
        let private =
            "tdash1zpavu07x8e3puq0lwy2s8mhpd37hfnz5ymdc2wzr3jcyxrrz3cnm3a8qydrzckf3ksua4xs5jh6t0";
        assert_eq!(pool_fee(&f, private), f.pool_transfer);
        assert_eq!(
            pool_fee(&f, "tdash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq"),
            f.unshield
        );
        assert_eq!(
            pool_fee(&f, " yStwA1xNxEE2kv5ZVDNGSsPach8pBApWHZ "),
            f.pool_withdrawal
        );
        assert_eq!(pool_fee(&f, ""), f.pool_transfer);
    }

    #[test]
    fn shielding_leaves_the_fee_or_the_reserve() {
        let f = fees();
        // From credits: everything but the fee, and the rest is exact.
        let cost = shield_cost(&f, ShieldSource::Credits, 1_000_000);
        assert_eq!((cost.fee, cost.max), (f.shield, 1_000_000 - f.shield));
        assert_eq!(cost.core_fee, 0);
        assert_eq!(
            shield_preview(&f, ShieldSource::Credits, 1_000_000, 300_000),
            Some((1_000_000 - 300_000 - f.shield, 300_000))
        );
        // From an address: the reserve stays for Platform's metered fee.
        let cost = shield_cost(&f, ShieldSource::PlatformAddress, 1_000_000);
        assert_eq!(cost.fee, f.shield_from_address);
        assert_eq!(cost.max, 1_000_000 - f.shield_reserve);
        // Too little to move anything.
        assert_eq!(shield_cost(&f, ShieldSource::PlatformAddress, 1_000).max, 0);
    }

    #[test]
    fn a_shield_from_core_pays_platform_out_of_the_lock() {
        let f = fees();
        let cost = shield_cost(&f, ShieldSource::Core, 1_000_000);
        // Two fees, two lines: Platform's out of the lock, Core's on top of it.
        assert_eq!(
            (cost.fee, cost.core_fee),
            (f.shield_from_core, f.core_payment)
        );
        assert_eq!(cost.max, 1_000_000 - f.core_payment);
        // 0.003 DASH locked: Core pays it and its fee, the pool gets it less Platform's fee.
        assert_eq!(
            shield_preview(&f, ShieldSource::Core, 1_000_000, 300_000),
            Some((
                1_000_000 - 300_000 - f.core_payment,
                300_000 - f.shield_from_core
            ))
        );
        // "Platform fee" is exactly the amount less what arrives in the pool.
        let (_, arrives) = shield_preview(&f, ShieldSource::Core, 1_000_000, 300_000).unwrap();
        assert_eq!(300_000 - arrives, cost.fee);
        // A lock no bigger than Platform's fee would bring nothing.
        assert_eq!(
            shield_preview(&f, ShieldSource::Core, 1_000_000, f.shield_from_core),
            None
        );
    }

    #[test]
    fn out_of_the_pool_to_the_identity_costs_the_top_up_fee() {
        let f = fees();
        assert_eq!(unshield_fee(&f, false), f.unshield);
        assert_eq!(unshield_fee(&f, true), f.pool_top_up);
    }

    #[test]
    fn counts_notes_in_russian() {
        assert_eq!(notes(1), "1 note");
        assert_eq!(notes(3), "3 notes");
        assert_eq!(notes(5), "5 notes");
        assert_eq!(notes(11), "11 notes");
        assert_eq!(notes(22), "22 notes");
    }
}

/// Where the pool stands, under its balance.
fn pool_line(pool: &evolite_api::Pool, now: chrono::DateTime<chrono::Local>) -> String {
    match pool.checked_at {
        Some(at) => format!(
            tr!("{} · last checked {}"),
            notes(pool.notes),
            text::when(at, now)
        ),
        None if pool.address.is_some() => {
            tr!("Reading the pool on Platform — the first time takes a few minutes").to_string()
        }
        // Keys bound and notes read in the background: minutes the first time.
        None if pool.connecting => {
            tr!("Connecting the pool: preparing keys and reading notes — the first time up to a few minutes")
                .to_string()
        }
        // Not asked to connect yet (Platform still syncing): waiting, said neutrally (CS-4).
        None => POOL_CONNECTING.to_string(),
    }
}

#[cfg(test)]
mod pool_line_tests {
    use super::*;

    #[test]
    fn says_the_pool_is_being_connected() {
        let mut pool = evolite_api::Pool {
            notes: 0,
            checked_at: None,
            address: None,
            operations: Vec::new(),
            unfinished_shield: None,
            connecting: false,
        };
        let now = chrono::Local::now();
        assert_eq!(pool_line(&pool, now), POOL_CONNECTING);
        pool.connecting = true;
        assert!(pool_line(&pool, now).starts_with("Connecting the pool"));
        pool.address = Some("tdash1…".into());
        assert!(pool_line(&pool, now).starts_with("Reading the pool"));
    }

    /// TECH-price-check AC-T2: what the journal records as promised is what this screen shows.
    #[test]
    fn the_price_on_record_is_the_one_the_pool_screen_shows() {
        use super::tests::fees;
        use super::{shield_cost, unshield_fee};
        use evolite_api::ShieldSource;
        use evolite_api::Spend;
        use evolite_core::price::{Seen, declared};
        let f = fees();
        let seen = Seen {
            fees: f.clone(),
            core: 0,
            credits: 0,
            platform_address: 0,
            pool: 0,
            mixed: 0,
            platform_inputs: Vec::new(),
            first_name: false,
        };
        for source in [
            ShieldSource::Credits,
            ShieldSource::PlatformAddress,
            ShieldSource::Core,
        ] {
            let cost = shield_cost(&f, source, 1_000_000);
            let op = Spend::Shield {
                from: source,
                amount: 5,
            };
            assert_eq!(declared(&op, &seen, 0).declared, cost.fee + cost.core_fee);
        }
        assert_eq!(
            declared(&Spend::Unshield { amount: 5 }, &seen, 0).declared,
            unshield_fee(&f, false)
        );
        assert_eq!(
            declared(&Spend::TopUpFromPool { amount: 5 }, &seen, 0).declared,
            unshield_fee(&f, true)
        );
    }
}
