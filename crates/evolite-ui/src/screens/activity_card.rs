//! 24 · Operation details: the card History and Overview open on a row (moved out of
//! `activity.rs` for FEAT-0005, so the card and the list change apart).

use super::txt;
use crate::components::{
    Assets, ButtonKind, button, button_enabled, button_row, dialog_with_header_held, field_label,
    kv_row, kv_row_in, notice, paint_icon_box, text_action,
};
use crate::context::Ctx;
use crate::text::{self};
use crate::theme::{Weight, mono, sans};
use chrono::{DateTime, Local};
use egui::{Align, Layout, Margin, Stroke, Ui};
use evolite_api::{
    Activity, ActivityKind, Chain, Command, Confirm, CoreError, Detail, Duffs, IdentityChoice,
    IdentityRef, Proof, SavedAddress, Source, WithdrawalState,
};
use zeroize::{Zeroize, Zeroizing};

/// The open operation's card, if one is open: History and Overview's latest operations show
/// the same one.
pub fn card_window(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let wallet = cx.wallet();
    let open = ui.data(|d| d.get_temp::<u64>(card_id()));
    if let Some(act) = open.and_then(|id| wallet.activity.iter().find(|a| a.id == id)) {
        let linked = act
            .detail
            .as_deref()
            .and_then(|d| d.link.as_deref())
            .and_then(|key| wallet.activity.iter().find(|a| a.key == key));
        let around = Around {
            now: cx.now,
            book: &wallet.address_book,
            linked,
            identities: &wallet.identities,
        };
        let answer = card(ui, a, act, &around);
        if let Some(command) = answer.note {
            cx.send(command);
        }
        if answer.closed {
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
        if let Some(screen) = answer.go {
            cx.go(screen);
        }
        // "Open in Identity": the same choice as the menu's, unless it is the one worked as.
        if let Some(id) = answer.act_as
            && wallet.identity.as_ref().map(|i| &i.id) != Some(&id)
        {
            cx.send(Command::ChooseIdentity { id });
        }
        // The related operation: its card in place of this one (AC-D4).
        if let Some(id) = answer.open {
            forget_note(ui.ctx());
            open_card(ui.ctx(), id);
        }
        // "Send again": the Send screen with the same recipient and amount.
        if let Some((to, amount)) = answer.again {
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

/// The card's detail (FEAT-0005) for the lines it is drawn for: on Core a payment in, out and a
/// transfer between own addresses; a Platform operation and an asset lock when the core knows
/// more than the row. Other lines keep the plain card.
fn card_detail(act: &Activity) -> Option<&Detail> {
    let drawn = act.chain != Chain::Core
        || matches!(
            act.kind,
            ActivityKind::Received { .. }
                | ActivityKind::Sent { .. }
                | ActivityKind::SelfTransfer { .. }
        );
    drawn.then_some(act.detail.as_deref()).flatten()
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

    /// Words, or a name, with a muted line.
    fn words(value: impl Into<String>, sub: Option<String>) -> Self {
        Side {
            value: value.into(),
            mono: false,
            sub,
        }
    }

    /// An identity of this wallet: its name when it has one, and whose credits (mockups 03-05).
    fn identity(r: &IdentityRef) -> Self {
        let short = text::short_address(&r.id);
        let your = tr!("your identity");
        let credits = tr!("credits");
        match &r.name {
            Some(name) => Side::words(
                format!("{} · {your}", crate::i18n::own(&format!("@{name}"))),
                Some(format!("{short} · {credits}")),
            ),
            None => Side::words(format!("{short} · {your}"), Some(credits.into())),
        }
    }

    /// Who was on the other side, as the row names them: an address in mono.
    fn payee(who: &str, sub: Option<String>) -> Self {
        Side {
            value: who.to_owned(),
            mono: !who.starts_with('@') && !who.contains('…'),
            sub,
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
        // A payout names the identity withdrawn from once it is paired with the withdrawal.
        ActivityKind::Received { .. } if detail.identity.is_some() => (
            detail.identity.iter().map(Side::identity).collect(),
            own_to(),
        ),
        ActivityKind::Received { from } => {
            let from = from.iter().map(|who| Side::words(who.clone(), None));
            (from.collect(), own_to())
        }
        _ => platform_sides(act, detail),
    }
}

/// «From» and «To» of a Platform operation or an asset lock (mockups 03-07): the identity it was
/// done as, when it is known, and where it went.
fn platform_sides(act: &Activity, detail: &Detail) -> (Vec<Side>, Vec<Side>) {
    let mine = || {
        detail
            .identity
            .iter()
            .map(Side::identity)
            .collect::<Vec<_>>()
    };
    let one = |side: Side| vec![side];
    let pool = || Side::words(tr!("This wallet's private pool"), None);
    match &act.kind {
        ActivityKind::NameRegistered { label } => (
            mine(),
            one(Side::words(
                "Dash Platform · DPNS",
                Some(format!(tr!("domain document “{}.dash”"), label)),
            )),
        ),
        ActivityKind::ProfileSaved => (
            mine(),
            one(Side::words(
                "Dash Platform · DashPay",
                Some(tr!("profile document").into()),
            )),
        ),
        ActivityKind::ContactRequest { to, .. } | ActivityKind::CreditTransfer { to } => {
            (mine(), one(Side::payee(to, None)))
        }
        ActivityKind::ContactRequestIn { from, .. } => (one(Side::payee(from, None)), mine()),
        // The Core address, ours when the core says it is (FEAT-0009).
        ActivityKind::Withdrawal { to, .. } if act.flow == evolite_api::Flow::Between => {
            (mine(), one(Side::own(to)))
        }
        ActivityKind::Withdrawal { to, .. } => (mine(), one(Side::payee(to, None))),
        ActivityKind::PoolSent => (one(pool()), Vec::new()),
        ActivityKind::PoolReceived => (Vec::new(), one(pool())),
        ActivityKind::Shielded { from } => {
            let source = if detail.identity.is_some() {
                mine()
            } else {
                from.as_deref()
                    .or_else(|| shield_source(act.layers))
                    .map(|f| Side::words(f, None))
                    .into_iter()
                    .collect()
            };
            (source, one(pool()))
        }
        ActivityKind::TopUp { .. } => {
            let from = Side::words(tr!("Core · this wallet"), Some(tr!("asset lock").into()));
            let to = if detail.identity.is_some() {
                mine()
            } else {
                counterparty(act)
                    .map(|(_, who, note)| Side::words(who, note))
                    .into_iter()
                    .collect()
            };
            (one(from), to)
        }
        _ => (Vec::new(), Vec::new()),
    }
}

/// Where a shield found by the pool's scan came from: the scan says it in the line's layers.
fn shield_source(layers: u8) -> Option<&'static str> {
    use evolite_api::layer;
    use evolite_core::layers::{FROM_CORE, FROM_CREDITS, FROM_PLATFORM_ADDRESS};
    if layers & layer::CORE != 0 {
        Some(FROM_CORE)
    } else if layers & layer::IDENTITY != 0 {
        Some(FROM_CREDITS)
    } else if layers & layer::PLATFORM_ADDRESS != 0 {
        Some(FROM_PLATFORM_ADDRESS)
    } else {
        None
    }
}

/// The line under the proof (FEAT-0005 item 7): what became of the operation elsewhere.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Related<'a> {
    /// Finished by Platform and its payout seen on Core: the payout's card.
    Paid {
        block: Option<u32>,
        payout: Option<&'a Activity>,
    },
    /// Finished by Platform; this wallet has not seen the payout (not one of its addresses, or
    /// not synced that far): nothing to open.
    PaidByPlatform,
    /// Not paid yet, as far as the last proved read knows.
    Waiting,
    /// Core did not confirm the payout in time: Platform signs it again by itself.
    Retrying,
    /// The Withdrawals contract's last word: too small for Core, the credits not returned.
    NotPaid,
    /// The withdrawal a payout came from.
    Withdrawal(&'a Activity),
    /// An asset lock Platform did not take.
    NotAdded { creates: bool },
}

/// Which line the operation gets, if any. «Paid» only as the core paired them: by the document's
/// index and script (AC-T4), never from the status alone once a payout is linked.
fn related<'a>(
    act: &Activity,
    detail: &Detail,
    linked: Option<&'a Activity>,
) -> Option<Related<'a>> {
    match &act.kind {
        ActivityKind::Withdrawal { status, .. } => match (status, detail.payout) {
            (Some(WithdrawalState::Complete), Some(p)) => Some(Related::Paid {
                block: p.block,
                payout: linked,
            }),
            (Some(WithdrawalState::Complete), None) => Some(Related::PaidByPlatform),
            (Some(WithdrawalState::Expired), _) => Some(Related::Retrying),
            (Some(WithdrawalState::Failed), _) => Some(Related::NotPaid),
            _ => Some(Related::Waiting),
        },
        ActivityKind::Received { .. } => linked.map(Related::Withdrawal),
        ActivityKind::TopUp {
            credits: evolite_api::Credits::NotAdded,
            creates,
            ..
        } => Some(Related::NotAdded { creates: *creates }),
        _ => None,
    }
}

/// The related line, drawn with a `line` outline; only one that leads somewhere is a button.
fn related_row(
    ui: &mut Ui,
    a: &mut Assets,
    (icon, color): (&str, egui::Color32),
    title: &str,
    sub: Option<&str>,
    opens: bool,
) -> egui::Response {
    const PAD: f32 = 14.0;
    const ICON: f32 = 16.0;
    const GAP: f32 = 10.0;
    let p = a.p;
    let width = ui.available_width();
    let wrap = (width - 2.0 * PAD - ICON - GAP - if opens { ICON + GAP } else { 0.0 }).max(40.0);
    let painter = ui.painter().clone();
    let title_galley = painter.layout(
        crate::i18n::t(title).into_owned(),
        sans(13.0, Weight::Medium),
        p.text,
        wrap,
    );
    let sub_galley = sub.map(|s| {
        painter.layout(
            crate::i18n::t(s).into_owned(),
            sans(12.0, Weight::Regular),
            p.muted,
            wrap,
        )
    });
    let text_height = title_galley.size().y + sub_galley.as_ref().map_or(0.0, |g| 2.0 + g.size().y);
    let sense = if opens {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, text_height + 24.0), sense);
    let spoken = match sub {
        Some(sub) => format!(
            "{} · {}",
            crate::i18n::spoken(title),
            crate::i18n::spoken(sub)
        ),
        None => crate::i18n::spoken(title),
    };
    if opens {
        crate::components::named(&resp, egui::WidgetType::Button, true, &spoken);
    } else {
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
    }
    let lit = opens && (resp.hovered() || resp.has_focus());
    painter.rect_stroke(
        rect.shrink(0.5),
        egui::CornerRadius::same(12),
        Stroke::new(1.0, if lit { p.text } else { p.line }),
        egui::StrokeKind::Inside,
    );
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + PAD + ICON / 2.0, rect.center().y),
        egui::vec2(ICON, ICON),
    );
    a.icons.icon(icon, color, &p, ICON).paint_at(ui, icon_rect);
    let x = icon_rect.right() + GAP;
    let top = rect.center().y - text_height / 2.0;
    let sub_y = top + title_galley.size().y + 2.0;
    painter.galley(egui::pos2(x, top), title_galley, p.text);
    if let Some(sub) = sub_galley {
        painter.galley(egui::pos2(x, sub_y), sub, p.muted);
    }
    if opens {
        let chevron = egui::Rect::from_center_size(
            egui::pos2(rect.right() - PAD - ICON / 2.0, rect.center().y),
            egui::vec2(ICON, ICON),
        );
        a.icons
            .icon("chevron-right", p.muted, &p, ICON)
            .paint_at(ui, chevron);
    }
    resp.on_hover_cursor(if opens {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::Default
    })
}

/// «Amount · when» of the operation a related line points at.
fn amount_and_when(act: &Activity, now: DateTime<Local>) -> String {
    format!(
        "{} · {}",
        text::signed_amount(act.amount),
        text::when(act.at, now)
    )
}

/// Draws [`Related`]: the identifier of the card to open, or that the Identity screen was asked.
fn related_block(
    ui: &mut Ui,
    a: &mut Assets,
    act: &Activity,
    related: Related,
    now: DateTime<Local>,
    answer: &mut CardAnswer,
) {
    let p = a.p;
    let to = match &act.kind {
        ActivityKind::Withdrawal { to, .. } => text::short_address(to),
        _ => String::new(),
    };
    match related {
        Related::Paid { block, payout } => {
            let title = match block {
                Some(b) => format!(tr!("Paid · block {}"), text::group_thousands(b as i64)),
                None => tr!("Paid out").into(),
            };
            let sub = payout
                .map(|l| format!("{} · {}", text::activity_title(l), amount_and_when(l, now)));
            let row = related_row(
                ui,
                a,
                ("receive", p.text),
                &title,
                sub.as_deref(),
                payout.is_some(),
            );
            if row.clicked() {
                answer.open = payout.map(|l| l.id);
            }
        }
        Related::PaidByPlatform => {
            related_row(
                ui,
                a,
                ("check", p.muted),
                tr!("Paid by Platform"),
                None,
                false,
            );
        }
        Related::Waiting => {
            let sub = format!(
                tr!("DASH arrive at {} as a separate Core operation, usually within an hour"),
                to
            );
            related_row(
                ui,
                a,
                ("pending", p.muted),
                tr!("Waiting for the Platform payout"),
                Some(&sub),
                false,
            );
        }
        Related::Retrying => {
            related_row(
                ui,
                a,
                ("pending", p.muted),
                tr!("The payout is being retried"),
                Some(tr!(
                    "Core did not confirm the payout in time. Platform signs it again on its own; you don't need to do anything."
                )),
                false,
            );
        }
        Related::NotPaid => {
            related_row(
                ui,
                a,
                ("warning", p.danger),
                tr!("Platform will not pay this out"),
                Some(tr!(
                    "The amount is too small for the Core network to accept. The credits are not returned."
                )),
                false,
            );
        }
        Related::Withdrawal(line) => {
            let title = text::activity_title(line);
            let sub = amount_and_when(line, now);
            if related_row(ui, a, ("send", p.text), &title, Some(&sub), true).clicked() {
                answer.open = Some(line.id);
            }
        }
        Related::NotAdded { creates } => {
            let color = crate::components::attention_color(&p);
            let sub = if creates {
                tr!(
                    "The identity's creation was interrupted. You can continue it on the Identity screen; the DASH are not lost"
                )
            } else {
                tr!(
                    "The top-up was interrupted. You can continue it on the Identity screen; the DASH are not lost"
                )
            };
            let row = related_row(
                ui,
                a,
                ("warning", color),
                tr!("The credits are not added yet"),
                Some(sub),
                true,
            );
            if row.clicked() {
                answer.go = Some(crate::screens::Screen::Identity);
                answer.closed = true;
            }
        }
    }
}

/// The part of the amount that went on, apart from the fee, under the word for it: what a
/// withdrawal takes out and what goes into the pool (mockups 04, 07).
fn amount_part(act: &Activity) -> Option<(&'static str, Duffs)> {
    let label = match act.kind {
        ActivityKind::Withdrawal { .. } => tr!("Withdrawn"),
        ActivityKind::Shielded { .. } => tr!("Into the pool"),
        _ => return None,
    };
    // A shield found by the scan is the amount itself, positive; the journal's has its fee in.
    let part = if act.amount > 0 {
        act.amount
    } else {
        -act.amount - act.fee.unwrap_or(0)
    };
    (part > 0).then_some((label, part))
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

/// The card's body under the amount (FEAT-0005, mockups 01-08): what a transfer between own
/// addresses did, «From → To» in a raised panel, the parts of the amount, when, the proof in
/// words, what became of it elsewhere and, when part of it was not kept, one line that says so
/// (FR-6).
fn detail_body(
    ui: &mut Ui,
    a: &mut Assets,
    act: &Activity,
    detail: &Detail,
    around: &Around,
    answer: &mut CardAnswer,
) {
    let (now, book) = (around.now, around.book);
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
    if let Some((label, part)) = amount_part(act) {
        kv_row(ui, &p, label, &crate::format::amount(part), value.clone());
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
    // An asset lock: the Core transaction's proof is the chip; the credits are apart, red when
    // Platform did not add them (BUG-57).
    credits_lines(ui, a, act, detail.credits_added);
    // Where a contact request stands; a withdrawal Platform gave up on says so here.
    if let Some(status) = shown_status(act) {
        kv_row(ui, &p, tr!("Status"), status, words.clone());
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
            proof_line(ui, a, icon, &said);
        }
        None if act.proof == Proof::Proved && act.chain != Chain::Core => {
            proof_line(
                ui,
                a,
                "confirmed",
                tr!("Proved by Platform: the answer matches the GroveDB proof"),
            );
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
    if let Some(line) = related(act, detail, around.linked) {
        related_block(ui, a, act, line, now, answer);
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

/// What proved it, in words: accent, this is what was checked (AC-D3).
fn proof_line(ui: &mut Ui, a: &mut Assets, icon: &str, said: &str) {
    let p = a.p;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add(a.icons.icon(icon, p.accent, &p, 16.0));
        ui.add(egui::Label::new(txt(said, 13.0, Weight::Regular, p.text)).wrap());
    });
}

/// «Credits» of an asset lock, red only when not added (BUG-57); `added`: how many, when this
/// computer made the top-up (FR-3). What Platform proved is explained under it (FEAT-0016).
fn credits_lines(ui: &mut Ui, a: &mut Assets, act: &Activity, added: Option<Duffs>) {
    let p = a.p;
    let Some((credits, tone)) = text::activity_credits(act) else {
        return;
    };
    let color = match tone {
        text::Tone::Plain => p.text,
        text::Tone::Quiet => p.muted,
        text::Tone::Bad => p.danger,
    };
    let said = match added.filter(|_| tone == text::Tone::Plain) {
        Some(added) => format!(tr!("{} were added"), crate::format::amount(added)),
        None => credits,
    };
    let words = sans(13.0, Weight::Medium);
    kv_row_in(ui, &p, tr!("Credits"), &said, words, color);
    if let Some(explained) = text::credits_explained(act) {
        notice(ui, a, "info", p.muted, &explained);
    }
}

/// The status said in words, where the card has no better line for it: a withdrawal's states
/// are the related line.
fn shown_status(act: &Activity) -> Option<&'static str> {
    match act.kind {
        ActivityKind::ContactRequest { .. } | ActivityKind::ContactRequestIn { .. } => {
            text::activity_status(act)
        }
        _ => None,
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
        crate::diagnostics::ask_scroll(ui.ctx());
        ui.scroll_to_rect(widget.rect.expand(AIR), None);
    }
}

/// What the card asks for.
#[derive(Default)]
struct CardAnswer {
    closed: bool,
    /// The payment to make again.
    again: Option<(String, Duffs)>,
    go: Option<crate::screens::Screen>,
    /// The note to write (FEAT-0003).
    note: Option<Command>,
    /// The related operation whose card to open instead (FEAT-0005).
    open: Option<u64>,
    /// The identity to work as (FEAT-0005 FR-7).
    act_as: Option<String>,
}

/// What the card reads besides its operation.
struct Around<'a> {
    now: DateTime<Local>,
    book: &'a [SavedAddress],
    /// The operation [`Detail::link`] points at, when it is in the history.
    linked: Option<&'a Activity>,
    identities: &'a [IdentityChoice],
}

/// The identity «Open in Identity» goes to: one of this wallet's, and not on a card that already
/// offers the Identity screen for a lock to continue.
fn identity_to_open<'a>(act: &'a Activity, identities: &[IdentityChoice]) -> Option<&'a str> {
    let not_added = matches!(
        act.kind,
        ActivityKind::TopUp {
            credits: evolite_api::Credits::NotAdded,
            ..
        }
    );
    let id = card_detail(act)?.identity.as_ref()?.id.as_str();
    (!not_added && !text::credits_quiet(act) && identities.iter().any(|c| c.id == id)).then_some(id)
}

/// 24 · Operation details: what and where, how much and its status, when, the fee and the block,
/// with whom, and the txid to copy or look up; "Send again" for a payment out, "Open in
/// Contacts" for a request to us not answered, "Continue on the Identity screen" for an asset
/// lock whose credits did not come (BUG-57). Whether it closed, the payment to make again when
/// asked, and the screen asked for.
fn card(ui: &mut Ui, a: &mut Assets, act: &Activity, around: &Around) -> CardAnswer {
    let (now, book) = (around.now, around.book);
    let repeat = again(act);
    let mut answer = CardAnswer::default();
    // A note edited in place: Escape and a click outside are its own, the card stays (AC-D4).
    let editing = note_edit(ui.ctx()).is_some_and(|e| !act.key.is_empty() && e.key == act.key);
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
    let footer = Footer {
        answer_there,
        not_added,
        repeat: &repeat,
        identity: identity_to_open(act, around.identities),
        open_identity: text::credits_quiet(act),
    };
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
                        crate::components::status_chip_of(ui, a, act.proof, text::core_chip(act));
                    });
                });
                if let Some(detail) = card_detail(act) {
                    // FEAT-0005 (mockups 01-08): from where to where, the parts of the amount,
                    // when, and what proved it in words. The chain is in the header and the block
                    // in the proof's line.
                    detail_body(ui, a, act, detail, around, &mut answer);
                } else {
                    plain_body(ui, a, act, now, book, not_added);
                }
                // The person's note, between the related operation and the txid (FEAT-0005 item 9,
                // FEAT-0003); none where no note can be kept.
                if !act.key.is_empty() {
                    answer.note = note_block(ui, a, act);
                }
                if let Some(txid) = &act.txid {
                    txid_block(ui, a, act, txid);
                }
                // The Platform document's id, in place of a state transition's (AC-A5).
                if let Some(id) = card_detail(act).and_then(|d| d.document.as_deref()) {
                    document_block(ui, a, act, id);
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
            card_footer(ui, a, &footer, &mut answer)
        },
    );
    answer.closed |= closed;
    answer
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
    // Up to the cross, which the dialog puts after the header, less 12 (BUG-114).
    let cross = crate::components::CROSS + ui.spacing().item_spacing.x;
    let width = (ui.available_width() - cross - TITLE_GAP).max(80.0);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add_space(4.0);
        card_title(ui, &p, &text::activity_title_in(act, book), width);
        ui.label(txt(
            &format!("{} · {}", text::chain(act.chain), text::when(act.at, now)),
            14.0,
            Weight::Medium,
            p.muted,
        ));
    });
}

/// Between the card's title and its close cross (BUG-114).
const TITLE_GAP: f32 = 12.0;

/// The card's title in at most two lines of `width`, read in full by a screen reader (BUG-114).
/// Lines are cut in reading order and each is shaped for display apart, so Arabic stays right.
fn card_title(ui: &mut Ui, p: &crate::theme::Palette, title: &str, width: f32) {
    let font = sans(20.0, Weight::Bold);
    let painter = ui.painter().clone();
    let shown = |s: &str| crate::i18n::visual(s).replace(['\u{2068}', '\u{2069}'], "");
    let galley = |s: &str| painter.layout_no_wrap(shown(s), font.clone(), p.text);
    let logical = crate::i18n::t_in(crate::i18n::current(), title).into_owned();
    let lines = fit_title(&logical, width, |s| galley(s).size().x);
    let galleys: Vec<_> = lines.iter().map(|l| galley(l)).collect();
    let size = galleys.iter().fold(egui::Vec2::ZERO, |s, g| {
        egui::vec2(s.x.max(g.size().x), s.y + g.size().y)
    });
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    let spoken = crate::i18n::spoken(title);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
    let mut y = rect.top();
    for g in galleys {
        let h = g.size().y;
        painter.galley(egui::pos2(rect.left(), y), g, p.text);
        y += h;
    }
}

/// Splits `title` into at most two lines no wider than `width` by `measure` (BUG-114): words
/// fill a line in order; an @name is never broken — it moves whole to the next line, and one
/// that does not fit a line alone keeps its start and end around «…» (the end at least
/// [`NAME_END`] characters). What is left after the second line ends it with «…».
fn fit_title(title: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let fits = |s: &str| measure(s) <= width;
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut rest = String::new();
    for word in title.split(' ').filter(|w| !w.is_empty()) {
        if lines.len() == 2 {
            rest.push(' ');
            rest.push_str(word);
            continue;
        }
        let joined = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if fits(&joined) {
            line = joined;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            if lines.len() == 2 {
                rest.push(' ');
                rest.push_str(word);
                continue;
            }
        }
        line = if fits(word) {
            word.to_owned()
        } else if is_name(word) {
            middle_fit(word, &fits)
        } else {
            end_fit(word, "", &fits)
        };
    }
    if !line.is_empty() {
        if lines.len() == 2 {
            rest = format!(" {line}{rest}");
        } else {
            lines.push(line);
        }
    }
    if !rest.is_empty()
        && let Some(last) = lines.last_mut()
    {
        *last = end_fit(last, &rest, &fits);
    }
    lines
}

/// The fewest characters kept at the end of a shortened @name: names often differ only there
/// (`-1108b`, `-1108c`), the designer's rule for BUG-114.
const NAME_END: usize = 6;

fn is_name(word: &str) -> bool {
    word.trim_start_matches(['\u{2068}', '\u{2069}'])
        .starts_with('@')
}

/// `name` with its middle replaced by «…» so that it fits: as much of both ends as fits, the
/// two halves balanced, the end never shorter than [`NAME_END`]. Its own-words marks stay.
fn middle_fit(name: &str, fits: &impl Fn(&str) -> bool) -> String {
    let (open, close) = (name.starts_with('\u{2068}'), name.ends_with('\u{2069}'));
    let bare: Vec<char> = name
        .chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect();
    let wrap = |s: String| {
        let mut out = String::new();
        if open {
            out.push('\u{2068}');
        }
        out.push_str(&s);
        if close {
            out.push('\u{2069}');
        }
        out
    };
    let cut = |kept: usize| {
        let tail = (kept / 2).max(NAME_END).min(kept);
        let head = kept - tail;
        let mut s: String = bare[..head].iter().collect();
        s.push('…');
        s.extend(&bare[bare.len() - tail..]);
        s
    };
    for kept in (1..bare.len()).rev() {
        let s = wrap(cut(kept));
        if fits(&s) {
            return s;
        }
    }
    wrap(cut(NAME_END.min(bare.len())))
}

/// `line` followed by what does not fit (`rest`), ended with «…» where it is cut.
fn end_fit(line: &str, rest: &str, fits: &impl Fn(&str) -> bool) -> String {
    let whole = format!("{line}{rest}");
    if rest.is_empty() && fits(&whole) {
        return whole;
    }
    let chars: Vec<char> = whole.chars().collect();
    for n in (1..=chars.len().min(line.chars().count() + rest.chars().count())).rev() {
        let s: String = chars[..n].iter().collect::<String>().trim_end().to_owned() + "…";
        if fits(&s) {
            return s;
        }
    }
    "…".into()
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
    credits_lines(ui, a, act, None);
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

/// The system contract and document type of the document a line wrote: what Platform Explorer's
/// page of a document asks for.
fn document_of(kind: &ActivityKind) -> Option<(&'static str, &'static str)> {
    match kind {
        ActivityKind::NameRegistered { .. } => {
            Some(("GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec", "domain"))
        }
        ActivityKind::ProfileSaved => {
            Some(("Bwr4WHCPz5rFVAD87RqTs3izo4zpzwsEdKPWUT1NS1C7", "profile"))
        }
        ActivityKind::Withdrawal { .. } => {
            Some(("4fJLR2GYTPFdomuTVvNy3VRrvWgvkKPzqehEBpNf2nk6", "withdrawal"))
        }
        _ => None,
    }
}

/// Platform Explorer's page of a document, on the network of this run.
fn platform_explorer(kind: &ActivityKind, id: &str) -> Option<String> {
    let (contract, doc_type) = document_of(kind)?;
    let site = match evolite_core::network::current() {
        evolite_api::Network::Testnet => "https://testnet.platform-explorer.com",
        evolite_api::Network::Mainnet => "https://platform-explorer.com",
    };
    Some(format!(
        "{site}/document/{id}?contract-id={contract}&document-type-name={doc_type}"
    ))
}

/// The Platform document's id (AC-A5, AC-T6): to copy, or to look up in the explorer, only by
/// the button.
fn document_block(ui: &mut Ui, a: &mut Assets, act: &Activity, id: &str) {
    let p = a.p;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        mono_block(ui, &p, tr!("Document ID"), id, None);
        ui.add_space(4.0);
        let copy_id = egui::Id::new("copy-document").with(act.id);
        let label = if crate::clipboard::just_copied(ui.ctx(), copy_id) {
            tr!("Copied")
        } else {
            tr!("Copy the ID")
        };
        let url = platform_explorer(&act.kind, id);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if button(ui, a, label, ButtonKind::Secondary, None).clicked() {
                crate::clipboard::copy_answered(ui.ctx(), id.to_string(), copy_id);
            }
            if let Some(url) = &url
                && button(
                    ui,
                    a,
                    tr!("Open in the Platform explorer"),
                    ButtonKind::Secondary,
                    None,
                )
                .on_hover_text(url.clone())
                .clicked()
            {
                crate::links::open(url);
            }
        });
        if url.is_some() {
            notice(
                ui,
                a,
                "shield",
                p.accent,
                tr!(
                    "The explorer opens only when you click. The explorer's site learns this computer's address and which operation you look at, so evolite opens it only by the button. You can simply copy the ID."
                ),
            );
        }
    });
}

/// What the buttons under the card offer.
struct Footer<'a> {
    answer_there: bool,
    not_added: bool,
    repeat: &'a Option<(String, Duffs)>,
    identity: Option<&'a str>,
    /// «Open Identity»: to see the credits, with no promise to continue (FEAT-0016).
    open_identity: bool,
}

/// The buttons under the card. Whether it closes.
fn card_footer(ui: &mut Ui, a: &mut Assets, footer: &Footer, answer: &mut CardAnswer) -> bool {
    let Footer {
        answer_there,
        not_added,
        repeat,
        identity,
        open_identity,
    } = *footer;
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
            answer.go = Some(crate::screens::Screen::Contacts);
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
            answer.go = Some(crate::screens::Screen::Identity);
            close = true;
        }
        if open_identity
            && button(ui, a, tr!("Open Identity"), ButtonKind::Secondary, None).clicked()
        {
            answer.go = Some(crate::screens::Screen::Identity);
            close = true;
        }
        if let Some(id) = identity
            && button(ui, a, tr!("Open in Identity"), ButtonKind::Secondary, None).clicked()
        {
            answer.act_as = Some(id.to_owned());
            answer.go = Some(crate::screens::Screen::Identity);
            close = true;
        }
        if let Some(repeat) = repeat
                    && button(ui, a, tr!("Send again"), ButtonKind::Secondary, None)
                        .on_hover_text(
                            crate::i18n::t(tr!("The same amount to the same recipient — we show everything again before signing"),)
                        .into_owned())
                        .clicked()
                {
                    answer.again = Some(repeat.clone());
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

    fn withdrawal(status: Option<WithdrawalState>) -> Activity {
        let mut act = sent("", None);
        act.kind = ActivityKind::Withdrawal {
            to: "yA".into(),
            status,
        };
        act.chain = Chain::Platform;
        act
    }

    /// АРХ-2: what a withdrawal's line says follows the document's status and whether the core
    /// paired a payout; a status alone never makes a link.
    #[test]
    fn a_withdrawal_is_paid_only_as_the_core_paired_it() {
        use WithdrawalState::*;
        let payout = sent("", None);
        let none = Detail::default();
        let paid = Detail {
            payout: Some(evolite_api::Payout { block: Some(9) }),
            ..Detail::default()
        };
        for s in [None, Some(Queued), Some(Pooled), Some(Broadcasted)] {
            assert_eq!(related(&withdrawal(s), &none, None), Some(Related::Waiting));
        }
        assert_eq!(
            related(&withdrawal(Some(Complete)), &none, None),
            Some(Related::PaidByPlatform)
        );
        assert_eq!(
            related(&withdrawal(Some(Complete)), &paid, Some(&payout)),
            Some(Related::Paid {
                block: Some(9),
                payout: Some(&payout)
            })
        );
        assert_eq!(
            related(&withdrawal(Some(Expired)), &none, None),
            Some(Related::Retrying)
        );
        assert_eq!(
            related(&withdrawal(Some(Failed)), &paid, Some(&payout)),
            Some(Related::NotPaid),
            "the last word is the document's"
        );
        assert!(shown_status(&withdrawal(Some(Failed))).is_none());
    }

    /// FR-9: the identity's name and id and the document's id find the line.
    #[test]
    fn the_search_finds_the_identity_and_the_document() {
        let mut act = withdrawal(None);
        act.detail = Some(Box::new(Detail {
            identity: Some(IdentityRef {
                id: "HFZQabc".into(),
                name: Some("werner555".into()),
            }),
            document: Some("9FpDoc".into()),
            ..Detail::default()
        }));
        for q in ["werner555", "hfzqabc", "9fpdoc"] {
            assert!(
                crate::screens::activity_filters::matches_query(&act, q, &[]),
                "{q}"
            );
        }
        assert!(!crate::screens::activity_filters::matches_query(
            &act,
            "nothing",
            &[]
        ));
    }

    /// FR-7: only one of this wallet's identities opens, and not over «Continue».
    #[test]
    fn open_in_identity_is_for_this_wallets_own() {
        let choice = IdentityChoice {
            id: "me".into(),
            name: None,
            slot: 0,
        };
        let mut act = withdrawal(None);
        act.detail = Some(Box::new(Detail {
            identity: Some(IdentityRef {
                id: "me".into(),
                name: None,
            }),
            ..Detail::default()
        }));
        assert_eq!(
            identity_to_open(&act, std::slice::from_ref(&choice)),
            Some("me")
        );
        assert_eq!(identity_to_open(&act, &[]), None);
        act.kind = ActivityKind::TopUp {
            identity: "@x".into(),
            creates: false,
            credits: evolite_api::Credits::NotAdded,
        };
        assert_eq!(identity_to_open(&act, &[choice]), None);
    }

    /// FEAT-0005a: «From → To» per kind.
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

        assert!(card_detail(&sent("@bob", None)).is_none(), "no detail");
        // A kind the card has no body for on Core stays plain.
        let mut other = with(sent("@bob", None), &[], &[]);
        other.kind = ActivityKind::PlatformReceived;
        assert!(card_detail(&other).is_none());
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
mod title_tests {
    use super::*;

    /// One unit per character: easy to reason about.
    fn width(s: &str) -> f32 {
        s.chars()
            .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
            .count() as f32
    }

    fn fit(title: &str, w: f32) -> Vec<String> {
        fit_title(title, w, width)
    }

    /// One line when it fits.
    #[test]
    fn a_short_title_is_one_line() {
        assert_eq!(fit("Top-up of @bob", 20.0), ["Top-up of @bob"]);
    }

    /// The @name moves whole to the second line rather than break.
    #[test]
    fn the_name_moves_whole_to_the_second_line() {
        assert_eq!(
            fit("Your request was accepted: @qa-kestrel-1108b", 30.0),
            ["Your request was accepted:", "@qa-kestrel-1108b"]
        );
    }

    /// A name too long for a line keeps its start and end; the end has at least six characters,
    /// so -1108b and -1108c stay apart (the designer's rule).
    #[test]
    fn a_name_too_long_keeps_its_start_and_its_end() {
        let lines = fit("From @qa-kestrel-1108b-with-a-name-long-enough", 14.0);
        assert_eq!(lines[0], "From");
        let name = &lines[1];
        assert!(width(name) <= 14.0, "{name}");
        assert!(name.starts_with("@qa") && name.contains('…'), "{name}");
        assert!(name.ends_with("enough"), "the end, six characters: {name}");
        // Even in a very narrow line the end keeps six characters.
        let narrow = fit("@qa-kestrel-1108c", 8.0);
        assert!(narrow[0].ends_with("-1108c"), "{narrow:?}");
    }

    /// At most two lines; what is left ends the second with «…».
    #[test]
    fn never_more_than_two_lines() {
        let lines = fit("one two three four five six seven eight nine ten", 10.0);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines.iter().all(|l| width(l) <= 10.0), "{lines:?}");
        assert!(lines[1].ends_with('…'), "{lines:?}");
    }

    /// The own-words marks around a name stay when it is shortened: Arabic keeps its order.
    #[test]
    fn a_marked_name_keeps_its_marks() {
        let lines = fit("\u{2068}@qa-kestrel-1108b-with-a-long-name\u{2069}", 12.0);
        assert!(
            lines[0].starts_with('\u{2068}') && lines[0].ends_with('\u{2069}'),
            "{lines:?}"
        );
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
