//! FEAT-0012: an operation's tax category in its card (`card/tax-category`, the designer's
//! frames 01–03) and the window that explains it (`dialog/tax-explainer`, frame 04).
//!
//! The row shows the pair the operation's direction allows — «Expense | Transfer out» for
//! what went out, «Income | Transfer in» for what came in (R-3) — and a click on its other half
//! asks the core at once ([`Command::SetTaxTransfer`]); there is no «Save». The core's snapshot
//! is what the row shows: the half asked for is shown only until the answer, and a refusal puts
//! the row back with the reason under it.

use crate::components::{Assets, ButtonKind, Part, button, dialog_with_footer, para};
use crate::csv_export::TaxCategory;
use crate::theme::{Weight, mono, sans};
use egui::{Align, Align2, CornerRadius, Layout, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use evolite_api::{Activity, Command, CoreError};

/// The category asked for and not answered yet: shown in place of the snapshot's.
#[derive(Clone)]
struct Asked {
    key: String,
    transfer: bool,
    /// The core said yes: kept until the snapshot carries it.
    done: bool,
}

/// Why the last change was not kept, under the row of `key`.
#[derive(Clone)]
struct Refused {
    key: String,
    why: String,
    details: Option<String>,
}

fn asked_id() -> egui::Id {
    egui::Id::new("activity-tax-asked")
}

fn refused_id() -> egui::Id {
    egui::Id::new("activity-tax-refused")
}

fn explainer_id() -> egui::Id {
    egui::Id::new("activity-tax-explainer")
}

/// Opens the window that explains the categories (the row's «?», `--screen
/// activity-card-tax-explainer`).
pub fn open_explainer(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(explainer_id(), true));
}

/// Whether that window is open: the card under it keeps Escape and clicks outside to it.
pub fn explainer_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(explainer_id())) == Some(true)
}

/// Forgets what the row asked and why it was refused, and closes the window: the card closed,
/// the wallet locked or another one opened.
pub fn forget_tax(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<Asked>(asked_id());
        d.remove::<Refused>(refused_id());
        d.remove::<bool>(explainer_id());
    });
}

/// The core's answer to [`Command::SetTaxTransfer`]: kept — the asked half stays until the
/// snapshot has it; refused — the row goes back to the snapshot's and says why.
pub fn on_tax_done(ctx: &egui::Context, result: &Result<(), CoreError>) {
    let Some(mut asked) = ctx.data(|d| d.get_temp::<Asked>(asked_id())) else {
        return;
    };
    match result {
        Ok(()) => {
            asked.done = true;
            ctx.data_mut(|d| d.insert_temp(asked_id(), asked));
        }
        Err(e) => {
            let refused = Refused {
                key: asked.key,
                why: crate::text::failed(tr!("The category was not saved"), e),
                details: crate::text::error_details(e),
            };
            ctx.data_mut(|d| {
                d.remove::<Asked>(asked_id());
                d.insert_temp(refused_id(), refused);
            });
        }
    }
}

/// `--screen activity-card-tax-error`: a change of the category of `key` the core refused, as
/// on a full disk.
#[cfg(feature = "dev-tools")]
pub fn tax_failed_example(ctx: &egui::Context, key: &str, transfer: bool) {
    ask(ctx, key, transfer);
    on_tax_done(
        ctx,
        &Err(CoreError::Failed("notes.enc: storage_full".into())),
    );
}

fn ask(ctx: &egui::Context, key: &str, transfer: bool) {
    ctx.data_mut(|d| {
        d.remove::<Refused>(refused_id());
        d.insert_temp(
            asked_id(),
            Asked {
                key: key.to_owned(),
                transfer,
                done: false,
            },
        );
    });
}

/// Whether the row of `act` shows the transfer: the half asked for until the snapshot agrees.
fn shown_transfer(ctx: &egui::Context, act: &Activity) -> bool {
    match ctx.data(|d| d.get_temp::<Asked>(asked_id())) {
        Some(asked) if asked.key == act.key => {
            if asked.done && asked.transfer == act.tax_transfer {
                ctx.data_mut(|d| d.remove::<Asked>(asked_id()));
            }
            asked.transfer
        }
        _ => act.tax_transfer,
    }
}

/// The pair's height: the row's, so its caption and «?» are centred on it.
const PAIR_HEIGHT: f32 = 36.0;

/// The row under the note: «Tax category», «?», the pair, and one line under them. The command
/// to send when the other half was chosen.
pub fn tax_row(
    ui: &mut Ui,
    a: &mut Assets,
    act: &Activity,
    by_sign: TaxCategory,
) -> Option<Command> {
    let p = a.p;
    let ctx = ui.ctx().clone();
    let pair = by_sign.pair();
    let transfer = shown_transfer(&ctx, act);
    let shown = pair[usize::from(transfer)];
    let mut chosen = None;
    // One block of the card (designer, 2026-11-18): the pair, 7 to the line's ink under it, and the
    // card's own gap after the whole — not after the pair, or the line reads as txid's caption.
    ui.vertical(|ui| {
        // 5 + the line's leading above its ink.
        ui.spacing_mut().item_spacing.y = 5.0;
        // As tall as the pair, so the caption and «?» stand on its middle, not its top.
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), PAIR_HEIGHT),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(crate::screens::txt(
                    tr!("Tax category"),
                    13.0,
                    Weight::Regular,
                    p.muted,
                ));
                if help_button(ui, a).clicked() {
                    open_explainer(&ctx);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    chosen = pair_toggle(ui, a, pair, shown);
                });
            },
        );
        let refused = ctx
            .data(|d| d.get_temp::<Refused>(refused_id()))
            .filter(|r| r.key == act.key);
        match &refused {
            Some(refused) => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add(a.icons.icon("warning", p.danger, &p, 14.0));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.add(para(&refused.why, 12.0, Weight::Medium, p.danger));
                        if let Some(details) = &refused.details {
                            crate::components::details_line(ui, &p, details);
                        }
                    });
                });
            }
            None => {
                ui.add(para(caption(shown), 12.0, Weight::Regular, p.muted));
            }
        }
    });
    // 16 from the line's ink to the next block's (the designer's measure): the card's gap is
    // between frames, and the line's leading and the txid caption's add 7 to it.
    ui.add_space(-7.0);
    let chosen = chosen.filter(|c| *c != shown)?;
    ask(&ctx, &act.key, chosen.is_transfer());
    Some(Command::SetTaxTransfer {
        key: act.key.clone(),
        transfer: chosen.is_transfer(),
    })
}

/// The line under the row: where the category goes, or what a transfer means and its code.
fn caption(shown: TaxCategory) -> &'static str {
    match shown {
        TaxCategory::Income | TaxCategory::Expense => {
            tr!("Only for the CSV export. Kept on this computer with the notes")
        }
        TaxCategory::TransferOut => tr!(
            "The money stayed with you: on an exchange or in another wallet of yours. In the CSV: Transfer-out"
        ),
        TaxCategory::TransferIn => tr!(
            "The money came from you: from an exchange or another wallet of yours. In the CSV: Transfer-in"
        ),
    }
}

/// «?»: 20 px, the `help` icon, named for a screen reader.
fn help_button(ui: &mut Ui, a: &mut Assets) -> egui::Response {
    let p = a.p;
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    crate::components::named(
        &resp,
        egui::WidgetType::Button,
        true,
        tr!("What the tax category is"),
    );
    let color = if resp.hovered() || resp.has_focus() {
        p.text
    } else {
        p.muted
    };
    a.icons
        .icon("help", color, &p, 16.0)
        .paint_at(ui, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The pair as one control (one Tab stop): a track with both halves, the shown one `ink`; a
/// click on a half or an arrow key picks it, Space switches. The half picked, if any.
fn pair_toggle(
    ui: &mut Ui,
    a: &mut Assets,
    pair: [TaxCategory; 2],
    shown: TaxCategory,
) -> Option<TaxCategory> {
    let p = a.p;
    let font = sans(13.0, Weight::SemiBold);
    let widths = pair.map(|c| crate::components::text_width(ui, c.label(), font.clone()) + 24.0);
    let size = vec2(8.0 + widths[0] + widths[1] + 4.0, PAIR_HEIGHT);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let id = resp.id;
    resp.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(
            egui::WidgetType::RadioGroup,
            true,
            crate::i18n::spoken(tr!("Tax category")),
        );
        info.current_text_value = Some(crate::i18n::spoken(shown.label()));
        info
    });
    // The arrows are this control's while it has the focus, not the next widget's.
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            );
        });
    }
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        p.bg,
        Stroke::new(1.0, p.line),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + 4.0;
    let halves = [0, 1].map(|i| {
        let half = Rect::from_min_size(pos2(x, rect.top() + 4.0), vec2(widths[i], 28.0));
        x += widths[i] + 4.0;
        half
    });
    let hover = resp.hover_pos();
    for (i, half) in halves.iter().enumerate() {
        let on = pair[i] == shown;
        if on {
            ui.painter()
                .rect_filled(*half, CornerRadius::same(7), p.ink);
        }
        let over = hover.is_some_and(|h| half.contains(h));
        let color = if on {
            p.on_ink
        } else if over {
            p.text
        } else {
            p.muted
        };
        crate::components::paint_text(
            ui,
            half.center(),
            Align2::CENTER_CENTER,
            pair[i].label(),
            font.clone(),
            color,
        );
    }
    let mut picked = None;
    if resp.clicked()
        && let Some(at) = resp.interact_pointer_pos()
    {
        picked = halves.iter().position(|h| h.contains(at));
    }
    if resp.has_focus() {
        // The frame is mirrored in Arabic: the right arrow goes to the half on the right.
        let (left, right) = if crate::i18n::rtl() { (1, 0) } else { (0, 1) };
        ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowLeft) {
                picked = Some(left);
            } else if i.key_pressed(egui::Key::ArrowRight) {
                picked = Some(right);
            } else if i.key_pressed(egui::Key::Space) {
                picked = Some(usize::from(!shown.is_transfer()));
            }
        });
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    picked.map(|i| pair[i])
}

/// `dialog/tax-explainer` (UC-4): what the category is, the four with their codes, and that
/// evolite is no tax adviser; «Got it» closes it. Drawn over the card while open.
pub fn explainer(ui: &mut Ui, a: &mut Assets) {
    if !explainer_open(ui.ctx()) {
        return;
    }
    let closed = dialog_with_footer(
        ui,
        a,
        "tax-explainer",
        "help",
        tr!("Tax category"),
        440.0,
        |ui, a, part| {
            let p = a.p;
            match part {
                // Not a dialog with a summary (BUG-249): never asked.
                Part::Summary => false,
                Part::Middle => {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    ui.add(para(
                        tr!(
                            "The category goes to the “Transaction Type” column of the CSV file, as in DashPay. A tax service uses it to tell income from moving your own money."
                        ),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    for (category, meaning) in [
                        (
                            TaxCategory::Income,
                            tr!("You were paid: for goods, work, a gift."),
                        ),
                        (TaxCategory::Expense, tr!("You paid someone else.")),
                        (
                            TaxCategory::TransferIn,
                            tr!(
                                "The money came from you: from your exchange or another wallet of yours."
                            ),
                        ),
                        (
                            TaxCategory::TransferOut,
                            tr!(
                                "The money went to you: to your exchange or another wallet of yours."
                            ),
                        ),
                    ] {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.label(crate::screens::txt(
                                    category.label(),
                                    14.0,
                                    Weight::SemiBold,
                                    p.text,
                                ));
                                // A format's code: never translated, never mirrored.
                                ui.label(
                                    egui::RichText::new(category.csv())
                                        .font(mono(12.0, Weight::Regular))
                                        .color(p.muted),
                                );
                            });
                            ui.add(para(meaning, 12.0, Weight::Regular, p.muted));
                        });
                    }
                    crate::components::notice(
                        ui,
                        a,
                        "warning",
                        p.muted,
                        tr!(
                            "evolite is not a tax adviser. If in doubt, ask a specialist in your country."
                        ),
                    );
                    false
                }
                Part::Footer => {
                    let mut close = false;
                    // The dialogs' own row of buttons: a click on it is the button's.
                    crate::components::button_row(ui, |ui| {
                        close = button(ui, a, tr!("Got it"), ButtonKind::Primary, None).clicked();
                    });
                    close
                }
            }
        },
    );
    // Over the card (which is held meanwhile), Escape is this window's: egui's own check of the
    // top modal does not see a window drawn inside another one's frame.
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if closed || escape {
        ui.ctx().data_mut(|d| d.remove::<bool>(explainer_id()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::{ActivityKind, Chain, Proof};

    fn line(transfer: bool) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Sent { to: "yX".into() },
            chain: Chain::Core,
            at: 100,
            amount: -1_000,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: "core:aa".into(),
            detail: None,
            note: None,
            rate: None,
            tax_transfer: transfer,
        }
    }

    /// The half asked for is shown until the snapshot carries it; a refusal puts the
    /// snapshot's back and keeps the reason for the row.
    #[test]
    fn the_asked_half_shows_until_the_snapshot_or_a_refusal() {
        let ctx = egui::Context::default();
        let before = line(false);
        ask(&ctx, "core:aa", true);
        assert!(shown_transfer(&ctx, &before), "asked: shown at once");
        on_tax_done(&ctx, &Ok(()));
        assert!(shown_transfer(&ctx, &before), "kept: until the snapshot");
        assert!(shown_transfer(&ctx, &line(true)));
        assert!(ctx.data(|d| d.get_temp::<Asked>(asked_id())).is_none());
        assert!(!shown_transfer(&ctx, &before), "the snapshot again");

        ask(&ctx, "core:aa", true);
        on_tax_done(
            &ctx,
            &Err(CoreError::Failed("notes.enc: storage_full".into())),
        );
        assert!(!shown_transfer(&ctx, &before), "refused: back");
        let refused = ctx.data(|d| d.get_temp::<Refused>(refused_id())).unwrap();
        assert_eq!(refused.key, "core:aa");
        // Another row does not show it, and the next change takes it away.
        ask(&ctx, "core:bb", true);
        assert!(ctx.data(|d| d.get_temp::<Refused>(refused_id())).is_none());
        forget_tax(&ctx);
        assert!(ctx.data(|d| d.get_temp::<Asked>(asked_id())).is_none());
    }
}
