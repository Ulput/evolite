//! 24 · Operation details: the card History and Overview open on a row (moved out of
//! `activity.rs` for FEAT-0005, so the card and the list change apart).

use super::txt;
use crate::components::{
    Assets, ButtonKind, button, button_enabled, button_row, dialog_with_header_held, field_label,
    kv_row, kv_row_in, notice, paint_icon_box, status_chip, text_action,
};
use crate::context::Ctx;
use crate::text::{self};
use crate::theme::{Weight, mono, sans};
use chrono::{DateTime, Local};
use egui::{Align, Layout, Margin, Stroke, Ui};
use evolite_api::{
    Activity, ActivityKind, Chain, Command, Confirm, CoreError, Detail, Duffs, SavedAddress, Source,
};
use zeroize::{Zeroize, Zeroizing};

/// The open operation's card, if one is open: History and Overview's latest operations show
/// the same one.
pub fn card_window(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let wallet = cx.wallet();
    let open = ui.data(|d| d.get_temp::<u64>(card_id()));
    if let Some(act) = open.and_then(|id| wallet.activity.iter().find(|a| a.id == id)) {
        let (closed, again, go, note) = card(ui, a, act, cx.now, &wallet.address_book);
        if let Some(command) = note {
            cx.send(command);
        }
        if closed {
            ui.data_mut(|d| {
                d.remove::<u64>(card_id());
                d.remove::<u64>(all_sides_id());
                d.remove::<u64>(body_of_id());
            });
            // A note half edited goes with its card.
            forget_note(ui.ctx());
        }
        // "Open in Contacts": accepting is there, with the password (BUG-42, BUG-58).
        // "Continue on the Identity screen": the cut-short top-up or creation is finished
        // there, with the password (BUG-57).
        if let Some(screen) = go {
            cx.go(screen);
        }
        // "Send again": the Send screen with the same recipient and amount.
        if let Some((to, amount)) = again {
            cx.pay_again(Source::Core, to, amount);
        }
    }
}

/// Where the open card's operation is kept between frames.
fn card_id() -> egui::Id {
    egui::Id::new("activity-card")
}

/// The operation whose card was asked to list all its addresses («Show all», BUG-90); only
/// that card, and only while it stays open.
fn all_sides_id() -> egui::Id {
    card_id().with("all-sides")
}

/// Opens the card of operation `id` (a row clicked; `--screen activity-card`).
pub fn open_card(ctx: &egui::Context, id: u64) {
    ctx.data_mut(|d| d.insert_temp(card_id(), id));
}

/// The note being edited in the open card (FEAT-0003, AC-D4): its operation's key, the text as
/// typed, whether the core is writing it and why the last write failed.
#[derive(Clone, Default)]
pub struct NoteEdit {
    pub key: String,
    pub text: Zeroizing<String>,
    pub waiting: bool,
    /// N10 with the core's reason, and its details.
    pub error: Option<(String, Option<String>)>,
    /// The focus was put at the end of the text.
    placed: bool,
}

/// A note just written, shown until the core's next snapshot carries it.
#[derive(Clone)]
struct NoteSaved {
    key: String,
    note: Option<Zeroizing<String>>,
}

fn note_edit_id() -> egui::Id {
    egui::Id::new("activity-note-edit")
}

fn note_saved_id() -> egui::Id {
    egui::Id::new("activity-note-saved")
}

/// The note being edited, if any.
pub fn note_edit(ctx: &egui::Context) -> Option<NoteEdit> {
    ctx.data(|d| d.get_temp::<NoteEdit>(note_edit_id()))
}

/// Opens the note of operation `key` for editing with `text` in it (a click on «Edit» or «Add a
/// note»; the presets).
pub fn edit_note(ctx: &egui::Context, key: &str, text: &str) {
    let edit = NoteEdit {
        key: key.to_owned(),
        text: Zeroizing::new(text.to_owned()),
        ..Default::default()
    };
    ctx.data_mut(|d| d.insert_temp(note_edit_id(), edit));
}

fn put_note_edit(ctx: &egui::Context, edit: NoteEdit) {
    ctx.data_mut(|d| d.insert_temp(note_edit_id(), edit));
}

/// The core's answer to [`Command::SetNote`] (`Done { what: DoneWhat::Note }`): written — back to the
/// note as shown, with the new text; not — the reason under the field, the text kept for
/// another try (AC-D5).
pub fn on_note_done(ctx: &egui::Context, result: &Result<(), CoreError>) {
    let Some(mut edit) = note_edit(ctx).filter(|e| e.waiting) else {
        return;
    };
    match result {
        Ok(()) => {
            let note = evolite_core::note_text::kept(&edit.text)
                .ok()
                .flatten()
                .map(Zeroizing::new);
            let saved = NoteSaved {
                key: std::mem::take(&mut edit.key),
                note,
            };
            ctx.data_mut(|d| {
                d.remove::<NoteEdit>(note_edit_id());
                d.insert_temp(note_saved_id(), saved);
            });
        }
        Err(e) => {
            edit.waiting = false;
            edit.error = Some((
                text::failed(tr!("The note was not saved"), e),
                text::error_details(e),
            ));
            put_note_edit(ctx, edit);
        }
    }
}

/// `--screen activity-card-note-error`: the note of `key` edited to `text`, and its write failed
/// as on a full disk (the designer's frame 7).
#[cfg(feature = "dev-tools")]
pub fn note_failed_example(ctx: &egui::Context, key: &str, text: &str) {
    edit_note(ctx, key, text);
    if let Some(mut edit) = note_edit(ctx) {
        edit.waiting = true;
        put_note_edit(ctx, edit);
    }
    on_note_done(
        ctx,
        &Err(CoreError::Failed("notes.enc: storage_full".into())),
    );
}
/// Forgets a note being edited or just written: the card closed, the wallet locked or another
/// one opened (AC-T6).
pub fn forget_note(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<NoteEdit>(note_edit_id());
        d.remove::<NoteSaved>(note_saved_id());
    });
}

/// The note the card shows for `act`: one just written until the snapshot has it.
fn shown_note(ctx: &egui::Context, act: &Activity) -> Option<String> {
    let saved = ctx.data(|d| d.get_temp::<NoteSaved>(note_saved_id()));
    match saved {
        Some(saved) if saved.key == act.key => {
            let note = saved.note.as_deref().map(|n| n.to_string());
            if note == act.note {
                ctx.data_mut(|d| d.remove::<NoteSaved>(note_saved_id()));
            }
            note
        }
        _ => act.note.clone(),
    }
}

/// The note's block in the card (FEAT-0003, AC-D4, AC-D5): the note with «Edit», a dashed «+ Add
/// a note» when there is none, or the note edited in place — Enter saves, Escape cancels, an
/// emptied note is deleted. The command to send when saved.
fn note_block(ui: &mut Ui, a: &mut Assets, act: &Activity) -> Option<Command> {
    let p = a.p;
    let ctx = ui.ctx().clone();
    let note = shown_note(&ctx, act);
    let Some(mut edit) = note_edit(&ctx).filter(|e| e.key == act.key) else {
        match &note {
            Some(note) => {
                note_frame(&p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.horizontal(|ui| {
                        note_heading(ui, a);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if text_action(ui, a, "edit", tr!("Edit")).clicked() {
                                edit_note(ui.ctx(), &act.key, note);
                            }
                        });
                    });
                    ui.add(
                        egui::Label::new(txt(
                            &crate::i18n::own(note),
                            14.0,
                            Weight::Regular,
                            p.text,
                        ))
                        .wrap(),
                    );
                });
            }
            None => {
                if add_note_line(ui, a).clicked() {
                    edit_note(ui.ctx(), &act.key, "");
                }
            }
        }
        return None;
    };
    let mut command = None;
    let mut cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
    note_frame(&p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        let full =
            evolite_core::note_text::graphemes(&edit.text) >= evolite_api::NOTE_MAX_GRAPHEMES;
        ui.horizontal(|ui| {
            note_heading(ui, a);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(crate::screens::send::note_counter(&edit.text))
                        .font(mono(12.0, Weight::Regular))
                        .color(if full { p.danger } else { p.muted }),
                );
            });
        });
        let id = ui.make_persistent_id("note-edit-field");
        let focused = ui.memory(|m| m.has_focus(id));
        let field = egui::Frame::new()
            .fill(p.raised)
            .stroke(Stroke::new(
                1.0,
                if focused { p.text } else { p.field_line },
            ))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                crate::a11y::name_next_field(ui, tr!("Note · only you see it"));
                let mut typed = edit.text.to_string();
                let resp = ui.add_enabled(
                    !edit.waiting,
                    egui::TextEdit::multiline(&mut typed)
                        .id(id)
                        .font(sans(14.0, Weight::Regular))
                        .text_color(p.text)
                        .frame(egui::Frame::NONE)
                        // Enter saves: no line breaks in a note.
                        .return_key(None)
                        .desired_rows(1)
                        .desired_width(f32::INFINITY),
                );
                crate::a11y::take_field_name(ui, &resp);
                if resp.changed() {
                    *edit.text = evolite_core::note_text::fit(&typed).0;
                    edit.error = None;
                }
                typed.zeroize();
                resp
            })
            .inner;
        // Opened: the focus in the field, the cursor after the last character.
        if !edit.placed && !ui.is_sizing_pass() {
            edit.placed = true;
            ui.memory_mut(|m| m.request_focus(id));
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let end = egui::text::CCursor::new(edit.text.chars().count());
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), id);
            } else {
                let mut state = egui::text_edit::TextEditState::default();
                let end = egui::text::CCursor::new(edit.text.chars().count());
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), id);
            }
        }
        let enter = field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if let Some((why, details)) = &edit.error {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(a.icons.icon("warning", p.danger, &p, 14.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.add(egui::Label::new(txt(why, 12.0, Weight::Medium, p.danger)).wrap());
                    if let Some(details) = details {
                        crate::components::details_line(ui, &p, details);
                    }
                });
            });
        } else {
            ui.add(
                egui::Label::new(txt(
                    tr!("Enter saves, Esc cancels. An empty note is deleted."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ))
                .wrap(),
            );
        }
        let kept = evolite_core::note_text::kept(&edit.text).ok().flatten();
        let deletes = kept.is_none() && note.is_some();
        let can_save = !edit.waiting && (kept.is_some() || note.is_some());
        let label = if edit.waiting {
            tr!("Saving…")
        } else if deletes {
            tr!("Delete the note")
        } else {
            tr!("Save")
        };
        let mut save = enter && can_save;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if button_enabled(ui, a, label, ButtonKind::Primary, None, can_save).clicked() {
                save = true;
            }
            if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                cancel = true;
            }
        });
        if save && !cancel {
            if kept == note {
                // Nothing changed: nothing to write.
                cancel = true;
            } else {
                edit.waiting = true;
                edit.error = None;
                command = Some(Command::SetNote {
                    key: act.key.clone(),
                    text: evolite_api::NoteText(kept.unwrap_or_default()),
                });
            }
        }
    });
    if cancel {
        ctx.data_mut(|d| d.remove::<NoteEdit>(note_edit_id()));
    } else {
        put_note_edit(&ctx, edit);
    }
    command
}

/// The note's block: `raised`, r12 (Figma 24).
fn note_frame(p: &crate::theme::Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.raised.gamma_multiply(0.7))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 12))
}

/// N5 with the `document` icon.
fn note_heading(ui: &mut Ui, a: &mut Assets) {
    let p = a.p;
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.add(a.icons.icon("document", p.muted, &p, 14.0));
    ui.label(txt(
        tr!("Note · only you see it"),
        12.0,
        Weight::Medium,
        p.muted,
    ));
}

/// «+ Add a note»: a dashed line, 44 px (Figma, card-add).
fn add_note_line(ui: &mut Ui, a: &mut Assets) -> egui::Response {
    let p = a.p;
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::click());
    crate::components::named(&resp, egui::WidgetType::Button, true, tr!("Add a note"));
    let over = resp.hovered() || resp.has_focus();
    let color = if over { p.text } else { p.field_line };
    let r = rect.shrink(0.5);
    let corners = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    ui.painter().extend(egui::Shape::dashed_line(
        &corners,
        Stroke::new(1.0, color),
        5.0,
        4.0,
    ));
    let text_color = if over { p.text } else { p.muted };
    let icon = a.icons.icon("plus", text_color, &p, 14.0);
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 14.0 + 7.0, rect.center().y),
        egui::vec2(14.0, 14.0),
    );
    icon.paint_at(ui, icon_rect);
    crate::components::paint_text(
        ui,
        egui::pos2(icon_rect.right() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        tr!("Add a note"),
        sans(13.0, Weight::Medium),
        text_color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The block explorer's page of a transaction, on the network of this run.
fn explorer(txid: &str) -> String {
    match evolite_core::network::current() {
        evolite_api::Network::Testnet => {
            format!("https://insight.testnet.networks.dash.org/insight/tx/{txid}")
        }
        evolite_api::Network::Mainnet => format!("https://insight.dash.org/insight/tx/{txid}"),
    }
}

/// Who an operation was with, as its card says it: the label, the value (the whole address
/// when the wallet knows it) and, for a contact paid at an address of its key, a line naming it.
fn counterparty(act: &Activity) -> Option<(&'static str, String, Option<String>)> {
    match &act.kind {
        ActivityKind::Received { from: Some(who) } => Some((tr!("Sender"), who.clone(), None)),
        ActivityKind::Sent { to } => match &act.address {
            // A contact (by name or short id) is paid at a new address of its key.
            Some(address) if to.starts_with('@') || to.contains('…') => Some((
                tr!("To"),
                address.clone(),
                Some(format!(
                    tr!("{to} · a new address from the contact's key"),
                    to = to
                )),
            )),
            Some(address) => Some((tr!("To"), address.clone(), None)),
            None if !to.is_empty() => Some((tr!("To"), to.clone(), None)),
            None => None,
        },
        ActivityKind::CreditTransfer { to } | ActivityKind::Withdrawal { to, .. } => {
            Some((tr!("To"), to.clone(), None))
        }
        // Which identity of ours sent it, when the wallet has several (BUG-69).
        ActivityKind::ContactRequest { to, ours, .. } => Some((
            tr!("To"),
            to.clone(),
            ours.as_ref().map(|o| format!(tr!("from {name}"), name = o)),
        )),
        ActivityKind::ContactRequestIn { from, ours, .. } => Some((
            tr!("Sender"),
            from.clone(),
            ours.as_ref().map(|o| format!(tr!("for {name}"), name = o)),
        )),
        // An asset lock: the identity it registered or tops up, when the core knows it — one of
        // this wallet's (BUG-57).
        ActivityKind::TopUp { identity, .. } if !identity.is_empty() => Some((
            tr!("To"),
            identity.clone(),
            Some(tr!("your identity").into()),
        )),
        // A creation cut short: the identity is not there yet.
        ActivityKind::TopUp { creates: true, .. } => Some((
            tr!("To"),
            "Dash Platform".into(),
            Some(tr!("A new identity of this wallet").into()),
        )),
        // The lock itself does not name it: one of this wallet's identities or its Platform
        // address, and nothing else.
        ActivityKind::TopUp { .. } => Some((
            tr!("To"),
            "Dash Platform".into(),
            Some(tr!("An identity or the Platform address of this wallet").into()),
        )),
        _ => None,
    }
}

/// The card's own Core detail (FEAT-0005a), for the Core lines it is drawn for: a payment in,
/// out, and a transfer between own addresses. Other lines keep their card as it was until 5b.
fn core_detail(act: &Activity) -> Option<&Detail> {
    let core = matches!(
        act.kind,
        ActivityKind::Received { .. }
            | ActivityKind::Sent { .. }
            | ActivityKind::SelfTransfer { .. }
    );
    (core && act.chain == Chain::Core)
        .then_some(act.detail.as_deref())
        .flatten()
}

/// One line of «From» or «To»: the value (an address in mono, or words) and a muted line under
/// it.
#[derive(Clone, PartialEq, Debug)]
struct Side {
    value: String,
    mono: bool,
    sub: Option<String>,
}

impl Side {
    fn own(address: &str) -> Self {
        Side {
            value: address.to_owned(),
            mono: true,
            sub: Some(tr!("this wallet's address").into()),
        }
    }
}

/// «From» and «To» of a Core line (FEAT-0005a, mockups 01, 02): a payment out comes from this
/// wallet's Core coins («3 inputs»), the change not shown (designer-16); a transfer between own
/// addresses names each address of ours once; a payment in names its sender when known and our
/// address it came to.
fn sides(act: &Activity, detail: &Detail, book: &[SavedAddress]) -> (Vec<Side>, Vec<Side>) {
    let own_to = || -> Vec<Side> {
        if detail.own_to.is_empty() {
            act.address.iter().map(|a| Side::own(a)).collect()
        } else {
            detail.own_to.iter().map(|(a, _)| Side::own(a)).collect()
        }
    };
    match &act.kind {
        ActivityKind::Sent { .. } => {
            let inputs = detail.own_from.len() as u32;
            let from = Side {
                value: tr!("Core · this wallet").into(),
                mono: false,
                sub: (inputs > 0).then(|| text::plural(inputs, tr!("input"), tr!("inputs"))),
            };
            let to = counterparty(act).map(|(_, who, note)| Side {
                mono: !who.starts_with('@'),
                // An address the person named says so.
                sub: note.or_else(|| {
                    text::book_label(act, book)
                        .map(|l| format!(tr!("“{l}” · from the address book"), l = l))
                }),
                value: who,
            });
            (vec![from], to.into_iter().collect())
        }
        ActivityKind::SelfTransfer { .. } => {
            let mut from: Vec<Side> = Vec::new();
            for address in &detail.own_from {
                if !from.iter().any(|s| &s.value == address) {
                    from.push(Side::own(address));
                }
            }
            (from, own_to())
        }
        ActivityKind::Received { from } => {
            let from = from.iter().map(|who| Side {
                value: who.clone(),
                mono: false,
                sub: None,
            });
            (from.collect(), own_to())
        }
        _ => (Vec::new(), Vec::new()),
    }
}

/// The label column of the «From → To» panel, and the gap after it.
const SIDE_LABEL: f32 = 64.0;
const SIDE_GAP: f32 = 12.0;

/// A side of more addresses than this is one line of how many, its addresses behind «Show
/// all» (BUG-90: 25 inputs took the card past the window).
const SIDE_ROWS: usize = 3;

/// One line of the «From → To» panel, painted and named «label: value · line» for a screen
/// reader (BUG-41); `label` only on a side's first line. The value wraps beside the label.
fn side_row(ui: &mut Ui, p: &crate::theme::Palette, label: &str, side: &Side) {
    let width = ui.available_width();
    let wrap = (width - SIDE_LABEL - SIDE_GAP).max(40.0);
    let font = if side.mono {
        mono(13.0, Weight::Medium)
    } else {
        sans(13.0, Weight::Medium)
    };
    let painter = ui.painter().clone();
    let value = painter.layout(crate::i18n::t(&side.value).into_owned(), font, p.text, wrap);
    let sub = side.sub.as_deref().map(|s| {
        painter.layout(
            crate::i18n::t(s).into_owned(),
            sans(12.0, Weight::Regular),
            p.muted,
            wrap,
        )
    });
    let height = value.size().y + sub.as_ref().map_or(0.0, |g| 2.0 + g.size().y);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(width, height.max(18.0)), egui::Sense::hover());
    // Addresses and @names as they are; the words in the language heard (review r1 should 1).
    let mut spoken = if side.mono {
        side.value.clone()
    } else {
        crate::i18n::spoken(&side.value)
    };
    if let Some(sub) = &side.sub {
        spoken = format!("{spoken} · {}", crate::i18n::spoken(sub));
    }
    let spoken = if label.is_empty() {
        spoken
    } else {
        format!("{}: {spoken}", crate::i18n::spoken(label))
    };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
    if !label.is_empty() {
        crate::components::paint_text(
            ui,
            rect.left_top(),
            egui::Align2::LEFT_TOP,
            label,
            sans(12.0, Weight::Regular),
            p.muted,
        );
    }
    let x = rect.left() + SIDE_LABEL + SIDE_GAP;
    let sub_y = rect.top() + value.size().y + 2.0;
    painter.galley(egui::pos2(x, rect.top()), value, p.text);
    if let Some(sub) = sub {
        painter.galley(egui::pos2(x, sub_y), sub, p.muted);
    }
}

/// The one line that stands for a side too long to list (BUG-90): the wallet's inputs and
/// its addresses, in the words of a payment's «Core · this wallet · N inputs».
fn side_summaries(detail: &Detail, to: &[Side]) -> [Side; 2] {
    let line = |sub: String| Side {
        value: tr!("Core · this wallet").into(),
        mono: false,
        sub: Some(sub),
    };
    let addresses = to
        .iter()
        .map(|s| &s.value)
        .collect::<std::collections::HashSet<_>>()
        .len() as u32;
    [
        line(text::plural(
            detail.own_from.len() as u32,
            tr!("input"),
            tr!("inputs"),
        )),
        line(text::plural(addresses, tr!("address"), tr!("addresses"))),
    ]
}

/// The Core card's body under the amount (FEAT-0005a, mockups 01, 02): what a transfer between
/// own addresses did, «From → To» in a raised panel, the parts of the amount, when, the proof
/// in words and, when part of it was not kept, one line that says so (FR-6).
fn core_body(
    ui: &mut Ui,
    a: &mut Assets,
    act: &Activity,
    detail: &Detail,
    now: DateTime<Local>,
    book: &[SavedAddress],
) {
    let p = a.p;
    let words = sans(13.0, Weight::Medium);
    let value = mono(13.0, Weight::Medium);
    let moved = match act.kind {
        ActivityKind::SelfTransfer { moved } if moved > 0 => Some(moved),
        _ => None,
    };
    if let Some(moved) = moved {
        ui.add(
            egui::Label::new(txt(
                &format!(
                    tr!("Moved {} inside the wallet: only the fee left it."),
                    crate::format::balance(moved)
                ),
                13.0,
                Weight::Regular,
                p.muted,
            ))
            .wrap(),
        );
    }
    let (from, to) = sides(act, detail, book);
    if !from.is_empty() || !to.is_empty() {
        egui::Frame::new()
            .fill(p.raised)
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                let all = ui.data(|d| d.get_temp::<u64>(all_sides_id())) == Some(act.id);
                let [from_line, to_line] = side_summaries(detail, &to);
                let mut held_back = false;
                for (label, side, line) in
                    [(tr!("From"), &from, from_line), (tr!("To"), &to, to_line)]
                {
                    if !all && side.len() > SIDE_ROWS {
                        held_back = true;
                        side_row(ui, &p, label, &line);
                        continue;
                    }
                    for (i, s) in side.iter().enumerate() {
                        side_row(ui, &p, if i == 0 { label } else { "" }, s);
                    }
                }
                if held_back {
                    ui.horizontal(|ui| {
                        ui.add_space(SIDE_LABEL + SIDE_GAP);
                        if text_action(ui, a, "chevron-down", tr!("Show all")).clicked() {
                            ui.data_mut(|d| d.insert_temp(all_sides_id(), act.id));
                        }
                    });
                }
            });
    }
    if let Some(moved) = moved {
        kv_row(
            ui,
            &p,
            tr!("Moved"),
            &crate::format::balance(moved),
            value.clone(),
        );
    }
    if let Some(fee) = act.fee {
        // Already inside the amount above: shown as a size, not a second debit.
        kv_row(
            ui,
            &p,
            tr!("Fee"),
            &crate::format::amount(fee),
            value.clone(),
        );
    }
    kv_row(ui, &p, tr!("When"), &text::date_time(act.at, now), words);
    match detail.confirm {
        Some(confirm) => {
            let (icon, said) = match confirm {
                Confirm::InstantLock => (
                    "instantsend",
                    tr!("InstantSend: the quorum signature is checked").to_owned(),
                ),
                Confirm::ChainLock { block } => (
                    "chainlock",
                    format!(
                        tr!("ChainLock in block {}: the quorum signature is checked"),
                        text::group_thousands(block as i64)
                    ),
                ),
            };
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                // Accent: this is what was checked (AC-D3).
                ui.add(a.icons.icon(icon, p.accent, &p, 16.0));
                ui.add(egui::Label::new(txt(&said, 13.0, Weight::Regular, p.text)).wrap());
            });
        }
        // In a block not locked yet: the chip says so, the block is still worth a line.
        None => {
            if let Some(block) = act.block {
                kv_row(
                    ui,
                    &p,
                    tr!("Block"),
                    &text::group_thousands(block as i64),
                    value,
                );
            }
        }
    }
    if detail.incomplete {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add(a.icons.icon("info", p.muted, &p, 16.0));
            ui.add(
                egui::Label::new(txt(
                    tr!("Part of the details of this operation was not saved"),
                    13.0,
                    Weight::Regular,
                    p.muted,
                ))
                .wrap(),
            );
        });
    }
}

/// A label over a value in mono that wraps (an address, a txid), with an optional muted line.
fn mono_block(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    label: &str,
    value: &str,
    note: Option<&str>,
) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, p, label);
        ui.add(
            egui::Label::new(
                egui::RichText::new(crate::i18n::t(value))
                    .font(mono(12.0, Weight::Regular))
                    .color(p.text)
                    .line_height(Some(18.0)),
            )
            .wrap(),
        );
        if let Some(note) = note {
            ui.label(txt(note, 12.0, Weight::Regular, p.muted));
        }
    });
}

/// The height of the card's row of buttons, measured last frame.
fn footer_id() -> egui::Id {
    card_id().with("footer")
}

/// The operation the card's scrolled part last showed: another one, or a new opening, starts at
/// the top.
fn body_of_id() -> egui::Id {
    card_id().with("body-of")
}

/// The widget that had the focus when the card's scrolled part was last drawn.
fn focus_seen_id() -> egui::Id {
    card_id().with("focus-seen")
}

/// The most the card's scrolled part can take: the window less the dialog's margins, the header
/// drawn above it and the buttons under it. `footer` already has the gap between the two.
fn body_room(ui: &Ui) -> f32 {
    // From the edges of the window. A card that fits must not scroll: the ordinary one at
    // 1024×768 stands 14 from them in English, 5 in ru, uk, es, fr and pt-BR.
    const MARGIN: f32 = 4.0;
    const PADDING: f32 = 50.0; // the dialog's 24 above and below, and its stroke
    const FIRST_FRAME_FOOTER: f32 = 56.0;
    const AT_LEAST: f32 = 96.0;
    let footer = ui
        .data(|d| d.get_temp::<f32>(footer_id()))
        .unwrap_or(FIRST_FRAME_FOOTER);
    let above = ui.cursor().top() - ui.min_rect().top();
    let window = ui.ctx().content_rect().height();
    (window - 2.0 * MARGIN - PADDING - above - footer).max(AT_LEAST)
}

/// Brings into the visible part what the focus has just come to, so that Tab never lands on a
/// button below the fold and Enter never presses it blind. Called at the end of the scrolled
/// part; only a widget cut by its clip moves it, which the header's cross and the row of buttons
/// never are. Only on a change of focus, so the wheel can still leave a focused widget behind.
fn follow_focus(ui: &Ui) {
    const AIR: f32 = 12.0; // the note field's frame comes into view with it
    let focus = ui.memory(|m| m.focused());
    let seen = ui
        .data(|d| d.get_temp::<Option<egui::Id>>(focus_seen_id()))
        .flatten();
    if focus == seen {
        return;
    }
    ui.data_mut(|d| d.insert_temp(focus_seen_id(), focus));
    if let Some(widget) = focus.and_then(|id| ui.ctx().read_response(id))
        && widget.interact_rect != widget.rect
    {
        ui.scroll_to_rect(widget.rect.expand(AIR), None);
    }
}

/// What the card asks for: closed, the payment to make again, the screen to go to, the note
/// to write (FEAT-0003).
type CardAnswer = (
    bool,
    Option<(String, Duffs)>,
    Option<crate::screens::Screen>,
    Option<Command>,
);

/// 24 · Operation details: what and where, how much and its status, when, the fee and the block,
/// with whom, and the txid to copy or look up; "Send again" for a payment out, "Open in
/// Contacts" for a request to us not answered, "Continue on the Identity screen" for an asset
/// lock whose credits did not come (BUG-57). Whether it closed, the payment to make again when
/// asked, and the screen asked for.
fn card(
    ui: &mut Ui,
    a: &mut Assets,
    act: &Activity,
    now: DateTime<Local>,
    book: &[SavedAddress],
) -> CardAnswer {
    let repeat = again(act);
    let mut note = None;
    // A note edited in place: Escape and a click outside are its own, the card stays (AC-D4).
    let editing = note_edit(ui.ctx()).is_some_and(|e| !act.key.is_empty() && e.key == act.key);
    let mut paid_again = None;
    let mut go = None;
    let answer_there = matches!(
        act.kind,
        ActivityKind::ContactRequestIn {
            state: evolite_api::RequestState::Waiting,
            ..
        }
    );
    let not_added = matches!(
        act.kind,
        ActivityKind::TopUp {
            credits: evolite_api::Credits::NotAdded,
            ..
        }
    );
    let closed = dialog_with_header_held(
        ui,
        a,
        "activity-card",
        464.0,
        editing,
        |ui, a| {
            card_header(ui, a, act, now, book);
        },
        |ui, a| {
            let p = a.p;
            // Everything but the header and the buttons scrolls, so neither is ever out of the
            // window, however long the operation is (AC-D7, BUG-90).
            let room = body_room(ui);
            let mut content = |ui: &mut Ui, a: &mut Assets| {
                ui.horizontal(|ui| {
                    crate::components::amount_label(
                        ui,
                        &text::signed_amount(act.amount),
                        mono(28.0, Weight::Medium),
                        p.text,
                    );
                    // The app's one «Hide amounts», as an icon right after the sum (BUG-73): a
                    // click hides the list under the card too.
                    ui.add_space(8.0 - ui.spacing().item_spacing.x);
                    crate::components::amount_toggle(ui, a, true);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        status_chip(ui, a, act.proof);
                    });
                });
                if let Some(detail) = core_detail(act) {
                    // FEAT-0005a (mockups 01, 02): from where to where, the parts of the amount,
                    // when, and what proved it in words. The chain is in the header and the block
                    // in the proof's line.
                    core_body(ui, a, act, detail, now, book);
                } else {
                    plain_body(ui, a, act, now, book, not_added);
                }
                // The person's note, between the related operation and the txid (FEAT-0005 item 9,
                // FEAT-0003); none where no note can be kept.
                if !act.key.is_empty() {
                    note = note_block(ui, a, act);
                }
                if let Some(txid) = &act.txid {
                    txid_block(ui, a, act, txid);
                }
            };
            // The first frame only measures: without the area, or its height there is wrong.
            if ui.is_sizing_pass() {
                content(ui, a);
            } else {
                // The lines are as wide with a bar as without: egui widens an area to its
                // content, so the bar's gutter goes into the dialog's side padding. In a child
                // [`Ui`], or the dialog would grow by the gutter.
                let width = ui.available_width();
                let mut wider = ui.new_child(egui::UiBuilder::new().id_salt("activity-card-place"));
                let mut area = egui::ScrollArea::vertical()
                    .id_salt("activity-card-body")
                    .auto_shrink([false, true])
                    .max_height(room);
                if ui.data(|d| d.get_temp::<u64>(body_of_id())) != Some(act.id) {
                    ui.data_mut(|d| d.insert_temp(body_of_id(), act.id));
                    area = area.vertical_scroll_offset(0.0);
                }
                area.show(&mut wider, |ui| {
                    ui.set_max_width(width);
                    content(ui, a);
                    follow_focus(ui);
                });
                let used = wider.min_rect();
                ui.advance_cursor_after_rect(egui::Rect::from_min_size(
                    used.min,
                    egui::vec2(width, used.height()),
                ));
            }
            card_footer(
                ui,
                a,
                answer_there,
                not_added,
                &repeat,
                &mut go,
                &mut paid_again,
            )
        },
    );
    (closed, paid_again, go, note)
}

/// The card's header: the operation's icon, what it is, where and when.
fn card_header(
    ui: &mut Ui,
    a: &mut Assets,
    act: &Activity,
    now: DateTime<Local>,
    book: &[SavedAddress],
) {
    let p = a.p;
    let (boxr, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
    paint_icon_box(ui, a, boxr, 12, text::activity_icon(act), 22.0, p.ink);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add_space(4.0);
        ui.label(txt(
            &text::activity_title_in(act, book),
            20.0,
            Weight::Bold,
            p.text,
        ));
        ui.label(txt(
            &format!("{} · {}", text::chain(act.chain), text::when(act.at, now)),
            14.0,
            Weight::Medium,
            p.muted,
        ));
    });
}

/// The lines of an operation that is not a Core payment.
fn plain_body(
    ui: &mut Ui,
    a: &mut Assets,
    act: &Activity,
    now: DateTime<Local>,
    book: &[SavedAddress],
    not_added: bool,
) {
    let p = a.p;
    // Mono is for amounts, addresses and hashes; the date and the chain are words.
    let words = sans(13.0, Weight::Medium);
    let value = mono(13.0, Weight::Medium);
    kv_row(
        ui,
        &p,
        tr!("When"),
        &text::date_time(act.at, now),
        words.clone(),
    );
    kv_row(ui, &p, tr!("Where"), text::chain(act.chain), words.clone());
    if let Some(status) = text::activity_status(act) {
        kv_row(ui, &p, tr!("Status"), status, words.clone());
    }
    // An asset lock: the chip above is the Core transaction's; whether Platform added
    // the credits is said apart, in red when it did not (BUG-57, customer: red).
    if let Some((credits, bad)) = text::activity_credits(act) {
        let color = if bad { p.danger } else { p.text };
        kv_row_in(ui, &p, tr!("Credits"), credits, words, color);
    }
    if let Some(fee) = act.fee {
        kv_row(
            ui,
            &p,
            tr!("Fee"),
            // Already inside the amount above: shown as a size, not a second debit
            //.
            &crate::format::amount(fee),
            value.clone(),
        );
    }
    if let Some(block) = act.block {
        kv_row(
            ui,
            &p,
            tr!("Block"),
            &text::group_thousands(block as i64),
            value,
        );
    }
    if let Some((label, who, note)) = counterparty(act) {
        // An address the person named says so.
        let note = note.or_else(|| {
            text::book_label(act, book)
                .map(|l| format!(tr!("“{l}” · from the address book"), l = l))
        });
        mono_block(ui, &p, label, &who, note.as_deref());
    }
    if not_added {
        let creates = matches!(act.kind, ActivityKind::TopUp { creates: true, .. });
        notice(
            ui,
            a,
            "warning",
            crate::components::attention_color(&p),
            if creates {
                tr!(
                    "The identity's creation was interrupted. You can continue it on the Identity screen; the DASH are not lost"
                )
            } else {
                tr!(
                    "The top-up was interrupted. You can continue it on the Identity screen; the DASH are not lost"
                )
            },
        );
    }
}

/// The txid: to copy, or to look up in the explorer.
fn txid_block(ui: &mut Ui, a: &mut Assets, act: &Activity, txid: &str) {
    let p = a.p;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        mono_block(ui, &p, "txid", txid, None);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            // Two outlined buttons with a gap: a ghost one beside an outlined one read as
            // one control.
            ui.spacing_mut().item_spacing.x = 12.0;
            // "Copied" in its place for a moment, as on "Receive" (BUG-39 №1).
            let copy_id = egui::Id::new("copy-txid").with(act.id);
            let label = if crate::clipboard::just_copied(ui.ctx(), copy_id) {
                tr!("Copied")
            } else {
                tr!("Copy txid")
            };
            if button(ui, a, label, ButtonKind::Secondary, None).clicked() {
                crate::clipboard::copy_answered(ui.ctx(), txid.to_string(), copy_id);
            }
            if button(
                ui,
                a,
                tr!("Open in the explorer"),
                ButtonKind::Secondary,
                None,
            )
            .on_hover_text(crate::i18n::t(&explorer(txid)).into_owned())
            .clicked()
            {
                // egui's open_url does nothing without eframe's `links`.
                crate::links::open(&explorer(txid));
            }
        });
    });
    notice(
        ui,
        a,
        "shield",
        p.accent,
        tr!(
            "The explorer opens only when you click. The explorer's site learns this computer's address and which operation you look at, so evolite opens it only by the button. You can simply copy the txid."
        ),
    );
}

/// The buttons under the card. Whether it closes.
fn card_footer(
    ui: &mut Ui,
    a: &mut Assets,
    answer_there: bool,
    not_added: bool,
    repeat: &Option<(String, Duffs)>,
    go: &mut Option<crate::screens::Screen>,
    paid_again: &mut Option<(String, Duffs)>,
) -> bool {
    let mut close = false;
    // The action on the right, «Close» on its left: as in every dialog (designer review №8).
    let footer_top = ui.cursor().top();
    button_row(ui, |ui| {
        if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
            close = true;
        }
        if answer_there
            && button(ui, a, tr!("Open in Contacts"), ButtonKind::Secondary, None).clicked()
        {
            *go = Some(crate::screens::Screen::Contacts);
            close = true;
        }
        if not_added
            && button(
                ui,
                a,
                tr!("Continue on the Identity screen"),
                ButtonKind::Secondary,
                None,
            )
            .clicked()
        {
            *go = Some(crate::screens::Screen::Identity);
            close = true;
        }
        if let Some(repeat) = &repeat
                    && button(ui, a, tr!("Send again"), ButtonKind::Secondary, None)
                        .on_hover_text(
                            crate::i18n::t(tr!("The same amount to the same recipient — we show everything again before signing"),)
                        .into_owned())
                        .clicked()
                {
                    *paid_again = Some(repeat.clone());
                    close = true;
                }
    });
    let footer = ui.cursor().top() - footer_top;
    ui.data_mut(|d| d.insert_temp(footer_id(), footer));
    close
}

/// What "Send again" pays: a Core payment out, to its @name or address, the amount
/// without the fee. Not for a contact without a name (its short id cannot be paid) or with
/// nothing known of the amount.
fn again(act: &Activity) -> Option<(String, Duffs)> {
    let ActivityKind::Sent { to } = &act.kind else {
        return None;
    };
    let recipient = if to.starts_with('@') {
        to.clone()
    } else {
        act.address.clone().unwrap_or_else(|| to.clone())
    };
    let amount = -act.amount - act.fee.unwrap_or(0);
    (act.chain == Chain::Core && !recipient.is_empty() && !recipient.contains('…') && amount > 0)
        .then_some((recipient, amount))
}

#[cfg(test)]
mod card_tests {
    use super::*;
    use evolite_api::Proof;

    fn sent(to: &str, address: Option<&str>) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Sent { to: to.into() },
            chain: Chain::Core,
            at: 0,
            amount: -1,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: address.map(Into::into),
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            detail: None,
            note: None,
        }
    }

    fn with(mut act: Activity, from: &[&str], to: &[&str]) -> Activity {
        act.detail = Some(Box::new(Detail {
            own_from: from.iter().map(|a| a.to_string()).collect(),
            own_to: to.iter().map(|a| (a.to_string(), 5)).collect(),
            ..Detail::default()
        }));
        act
    }

    fn values(sides: &[Side]) -> Vec<(&str, Option<&str>)> {
        sides
            .iter()
            .map(|s| (s.value.as_str(), s.sub.as_deref()))
            .collect()
    }

    /// FEAT-0005a: «From → To» per kind. A payment out — this wallet's coins and how many
    /// inputs, then the payee as before (a contact's line); a transfer between own addresses —
    /// each address of ours once; a payment in — its sender, then our address it came to; no
    /// detail or another kind — no panel.
    #[test]
    fn from_and_to_follow_the_kind() {
        let out = with(sent("@bob", Some("yLcBz")), &["yA", "yB", "yA"], &[]);
        let d = out.detail.as_deref().unwrap();
        let (from, to) = sides(&out, d, &[]);
        assert_eq!(values(&from), [("Core · this wallet", Some("3 inputs"))]);
        assert!(!from[0].mono && to[0].mono);
        assert_eq!(
            values(&to),
            [("yLcBz", Some("@bob · a new address from the contact's key"))]
        );
        let one = with(sent("yLcBz", Some("yLcBz")), &["yA"], &[]);
        let (from, _) = sides(&one, one.detail.as_deref().unwrap(), &[]);
        assert_eq!(from[0].sub.as_deref(), Some("1 input"));

        let mut moved = with(sent("", None), &["yA", "yA", "yB"], &["yC"]);
        moved.kind = ActivityKind::SelfTransfer { moved: 5 };
        let (from, to) = sides(&moved, moved.detail.as_deref().unwrap(), &[]);
        let own = Some("this wallet's address");
        assert_eq!(values(&from), [("yA", own), ("yB", own)]);
        assert_eq!(values(&to), [("yC", own)]);

        let mut came = with(sent("", Some("yD")), &[], &[]);
        came.kind = ActivityKind::Received {
            from: Some("@alice".into()),
        };
        let (from, to) = sides(&came, came.detail.as_deref().unwrap(), &[]);
        assert_eq!(values(&from), [("@alice", None)]);
        // No receiving output listed: the line's own address.
        assert_eq!(values(&to), [("yD", own)]);

        assert!(core_detail(&sent("@bob", None)).is_none(), "no detail");
        let mut platform = with(sent("@bob", None), &[], &[]);
        platform.chain = Chain::Platform;
        assert!(core_detail(&platform).is_none(), "5a draws Core lines only");
    }

    #[test]
    fn says_whom_with_the_address_and_the_contact() {
        // A contact: its new address, and its name under it.
        assert_eq!(
            counterparty(&sent("@bob", Some("yLcBz"))),
            Some((
                tr!("To"),
                "yLcBz".to_string(),
                Some("@bob · a new address from the contact's key".to_string())
            ))
        );
        // An address alone, or a name with no address known.
        assert_eq!(
            counterparty(&sent("yLcBz", Some("yLcBz"))),
            Some((tr!("To"), "yLcBz".to_string(), None))
        );
        assert_eq!(
            counterparty(&sent("@bob", None)),
            Some((tr!("To"), "@bob".to_string(), None))
        );
        assert_eq!(counterparty(&sent("", None)), None);
    }
}

#[cfg(test)]
mod again_tests {
    use super::*;
    use evolite_api::Proof;

    fn out(to: &str, address: Option<&str>, amount: Duffs, fee: Option<Duffs>) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Sent { to: to.into() },
            chain: Chain::Core,
            at: 0,
            amount,
            proof: Proof::ChainLock,
            txid: None,
            fee,
            block: None,
            address: address.map(Into::into),
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            detail: None,
            note: None,
        }
    }

    #[test]
    fn pays_the_same_recipient_the_same_amount_again() {
        // To an address: the address, the amount without the fee.
        assert_eq!(
            again(&out("yXk3", Some("yXk3"), -100_000_226, Some(226))),
            Some(("yXk3".to_string(), 100_000_000))
        );
        // To a contact: by its name, so it goes to the next address of its key.
        assert_eq!(
            again(&out("@bob", Some("yLc"), -10_226, Some(226))),
            Some(("@bob".to_string(), 10_000))
        );
        // A contact without a name, or nothing that went out: nothing to repeat.
        assert_eq!(again(&out("6G7V…Qck", None, -10_000, None)), None);
        let mut received = out("yX", None, 5, None);
        received.kind = ActivityKind::Received { from: None };
        assert_eq!(again(&received), None);
    }

    #[test]
    fn offers_saved_addresses_while_typing() {
        let book = [
            SavedAddress {
                address: "yAAA".into(),
                label: "Rent".into(),
            },
            SavedAddress {
                address: "yBBB".into(),
                label: "Exchange".into(),
            },
        ];
        // `book_matches` lives with the Send screen.
        assert_eq!(
            crate::text::book_label(&out("yBBB", None, -1, None), &book),
            Some("Exchange")
        );
        assert_eq!(
            crate::text::activity_title_in(&out("yBBB", Some("yBBB"), -1, None), &book),
            "Sent · Exchange"
        );
    }
}
