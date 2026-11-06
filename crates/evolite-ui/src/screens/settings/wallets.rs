//! Settings → Wallets (Figma 27): the wallets of this computer — open one, rename or
//! delete one from its "…" — and "Create new" / "Restore from phrase".

use super::SettingsState;
use super::parts::heading;
use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, button, button_enabled, button_row, card, dialog_with_header,
    field_label, inner, notice, paint_text, paint_wallet_dot, section_label, text_field,
};
use crate::context::{Ctx, WalletRequest};
use crate::screens::onboarding::Flow;
use crate::theme::{Weight, sans};
use egui::{Align2, CornerRadius, CursorIcon, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use evolite_api::{Command, CoreError};

/// A wallet's name: what the core takes.
pub const NAME_MAX: usize = 24;

/// "Rename": the wallet, the name typed, the core's answer while it is awaited.
#[derive(Default)]
pub struct RenameForm {
    pub id: String,
    pub name: String,
    /// One of the six wallet colours.
    pub color: u8,
    pub waiting: bool,
    pub error: Option<String>,
}

impl RenameForm {
    /// The core's answer to `RenameWallet`; `true` when the dialog is done.
    pub fn on_done(&mut self, result: &Result<(), CoreError>) -> bool {
        self.waiting = false;
        match result {
            Ok(()) => true,
            Err(e) => {
                self.error = Some(crate::text::error_text(e));
                false
            }
        }
    }

    /// The saved name and colour in `wallets` at once (BUG-23): the core has them when it says
    /// so, and the list need not wait for its next snapshot.
    pub fn show_saved(&self, wallets: &mut [evolite_api::WalletEntry]) {
        if let Some(w) = wallets.iter_mut().find(|w| w.id == self.id) {
            w.name = self.name.trim().to_owned();
            w.color = self.color;
        }
    }
}

pub(super) fn wallets(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut SettingsState) {
    let p = a.p;
    let snap = cx.snap;
    heading(
        ui,
        &p,
        tr!("Wallets"),
        Some(tr!(
            "Each wallet has its own recovery phrase, password and history. One is open: the others do not sync until you open them."
        )),
    );
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        section_label(ui, &p, tr!("On this computer"));
        let width = ui.available_width();
        card(&p, Margin::ZERO).show(ui, |ui| {
            ui.set_width(inner(width, 0));
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, w) in snap.wallets.iter().enumerate() {
                if i > 0 {
                    let (line, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().rect_filled(line, 0, p.line);
                }
                let (row, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 73.0), Sense::hover());
                let dot = Rect::from_min_size(
                    pos2(row.left() + 20.0, row.center().y - 16.0),
                    vec2(32.0, 32.0),
                );
                paint_wallet_dot(ui, dot, w);
                // The right side first: what is left is the text's.
                let more = Rect::from_center_size(
                    pos2(row.right() - 32.0, row.center().y),
                    vec2(32.0, 32.0),
                );
                let more_resp =
                    ui.interact(more, ui.id().with(("wallet-more", &w.id)), Sense::click());
                crate::components::named(&more_resp, egui::WidgetType::Button, true, tr!("More"));
                if more_resp.hovered() || state.wallet_more.as_deref() == Some(w.id.as_str()) {
                    ui.painter()
                        .rect_filled(more, CornerRadius::same(8), p.raised);
                }
                for dx in [-6.0, 0.0, 6.0] {
                    ui.painter().circle_filled(
                        pos2(more.center().x + dx, more.center().y),
                        1.8,
                        p.text,
                    );
                }
                if more_resp
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
                {
                    state.wallet_more = if state.wallet_more.as_deref() == Some(w.id.as_str()) {
                        None
                    } else {
                        Some(w.id.clone())
                    };
                }
                let status_right = more.left() - 12.0;
                let text_right = if w.open {
                    let font = sans(14.0, Weight::Medium);
                    let label = paint_text(
                        ui,
                        pos2(status_right, row.center().y),
                        Align2::RIGHT_CENTER,
                        tr!("Is open"),
                        font,
                        p.accent,
                    );
                    a.icons.icon("check", p.accent, &p, 16.0).paint_at(
                        ui,
                        Rect::from_center_size(
                            pos2(label.left() - 12.0, row.center().y),
                            vec2(16.0, 16.0),
                        ),
                    );
                    label.left() - 28.0
                } else {
                    let w_btn = crate::components::text_width(
                        ui,
                        tr!("Open"),
                        sans(14.0, Weight::SemiBold),
                    ) + 36.0;
                    let btn = Rect::from_min_size(
                        pos2(status_right - w_btn, row.center().y - 20.5),
                        vec2(w_btn, 41.0),
                    );
                    // A child of its own: the row's height stays the row's.
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btn));
                    let open = button(
                        &mut child,
                        a,
                        tr!("Open"),
                        ButtonKind::Secondary,
                        Some(w_btn),
                    );
                    if open.clicked() {
                        cx.wallets(WalletRequest::Open(w.id.clone()));
                    }
                    btn.left() - 12.0
                };
                let x = dot.right() + 14.0;
                let max = (text_right - x).max(40.0);
                let name =
                    ui.painter()
                        .layout(w.name.clone(), sans(15.0, Weight::SemiBold), p.text, max);
                ui.painter().galley(pos2(x, row.top() + 17.0), name, p.text);
                let line = crate::i18n::t(&crate::screens::wallets::settings_line(w, snap, cx.now))
                    .into_owned();
                let line = ui
                    .painter()
                    .layout(line, sans(12.0, Weight::Regular), p.muted, max);
                ui.painter()
                    .galley(pos2(x, row.top() + 40.0), line, p.muted);
                if state.wallet_more.as_deref() == Some(w.id.as_str()) {
                    more_menu(ui, a, cx, state, more, w);
                }
            }
        });
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        section_label(ui, &p, tr!("Add wallet"));
        let width = ui.available_width();
        card(&p, Margin::ZERO).show(ui, |ui| {
            ui.set_width(inner(width, 0));
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, (icon, title, sub, flow)) in [
                (
                    "plus",
                    tr!("Create new"),
                    tr!("A new 12-word phrase and a password — as on the first start"),
                    Flow::Create,
                ),
                (
                    "import",
                    tr!("Restore from phrase"),
                    tr!("12 or 24 words — for example from Dash Core or DashPay"),
                    Flow::Restore,
                ),
            ]
            .into_iter()
            .enumerate()
            {
                if i > 0 {
                    let (line, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().rect_filled(line, 0, p.line);
                }
                let (row, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 69.0), Sense::click());
                crate::components::named(&resp, egui::WidgetType::Button, true, title);
                if resp.hovered() {
                    ui.painter().rect_filled(
                        row,
                        CornerRadius::same(14),
                        p.raised.gamma_multiply(0.6),
                    );
                }
                let boxr = Rect::from_min_size(
                    pos2(row.left() + 20.0, row.center().y - 16.0),
                    vec2(32.0, 32.0),
                );
                ui.painter().circle_filled(boxr.center(), 16.0, p.raised);
                a.icons
                    .icon(icon, p.text, &p, 18.0)
                    .paint_at(ui, Rect::from_center_size(boxr.center(), vec2(18.0, 18.0)));
                let x = boxr.right() + 14.0;
                paint_text(
                    ui,
                    pos2(x, row.top() + 16.0),
                    Align2::LEFT_TOP,
                    title,
                    sans(15.0, Weight::SemiBold),
                    p.text,
                );
                paint_text(
                    ui,
                    pos2(x, row.top() + 38.0),
                    Align2::LEFT_TOP,
                    sub,
                    sans(12.0, Weight::Regular),
                    p.muted,
                );
                a.icons.icon("chevron-right", p.muted, &p, 18.0).paint_at(
                    ui,
                    Rect::from_center_size(
                        pos2(row.right() - 28.0, row.center().y),
                        vec2(18.0, 18.0),
                    ),
                );
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    cx.wallets(WalletRequest::Add(Some(flow)));
                }
            }
        });
    });
    notice(
        ui,
        a,
        "shield",
        p.muted,
        tr!(
            "Delete a wallet from its “…” menu: evolite asks for that wallet’s own password. The other wallets’ phrases are neither needed nor shown."
        ),
    );
}

/// A wallet's "…" (Figma 32): "Open" (another wallet), "Rename and colour", "Show
/// phrase", "Delete from this computer".
fn more_menu(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut SettingsState,
    anchor: Rect,
    w: &evolite_api::WalletEntry,
) {
    let p = a.p;
    let mut open = false;
    let mut rename = false;
    let mut phrase = false;
    let mut delete = false;
    let area = egui::Area::new(egui::Id::new("wallet-more-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(anchor.right() - 250.0, anchor.bottom() + 6.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.line))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.set_width(238.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let mut items = Vec::new();
                    if !w.open {
                        items.push(("lock", tr!("Open"), p.text, &mut open));
                    }
                    items.push(("edit", tr!("Rename and colour"), p.text, &mut rename));
                    items.push(("seed", tr!("Show the phrase"), p.text, &mut phrase));
                    items.push((
                        "trash",
                        tr!("Delete from this computer"),
                        p.danger,
                        &mut delete,
                    ));
                    for (icon, label, color, slot) in items {
                        let (row, resp) = ui.allocate_exact_size(vec2(238.0, 34.0), Sense::click());
                        crate::components::named(&resp, egui::WidgetType::Button, true, label);
                        if resp.hovered() {
                            ui.painter()
                                .rect_filled(row, CornerRadius::same(8), p.raised);
                        }
                        a.icons.icon(icon, color, &p, 16.0).paint_at(
                            ui,
                            Rect::from_min_size(
                                pos2(row.left() + 12.0, row.center().y - 8.0),
                                vec2(16.0, 16.0),
                            ),
                        );
                        paint_text(
                            ui,
                            pos2(row.left() + 36.0, row.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            sans(13.0, Weight::Medium),
                            color,
                        );
                        *slot = resp.on_hover_cursor(CursorIcon::PointingHand).clicked();
                    }
                });
        });
    let outside = ui.input(|i| {
        i.pointer.any_click()
            && i.pointer
                .interact_pos()
                .is_some_and(|at| !area.response.rect.contains(at) && !anchor.contains(at))
    });
    if outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.wallet_more = None;
    }
    if open {
        cx.wallets(WalletRequest::Open(w.id.clone()));
        state.wallet_more = None;
    }
    if rename {
        state.rename = Some(RenameForm {
            id: w.id.clone(),
            name: w.name.clone(),
            color: w.color,
            ..Default::default()
        });
        state.wallet_more = None;
    }
    if phrase {
        // The phrase dialog, by this wallet's own password.
        state.phrase_wallet = (!w.open).then(|| w.id.clone());
        state.phrase_dialog = true;
        state.wallet_more = None;
    }
    if delete {
        // Another wallet goes right here by its own password; the open one stays open (R-28).
        state.delete = Some(if w.open {
            crate::screens::delete_wallet::DeleteForm::default()
        } else {
            crate::screens::delete_wallet::DeleteForm::other(w.id.clone(), w.name.clone())
        });
        state.wallet_more = None;
    }
}

/// "Wallet "…"" (Figma 30): the name, up to [`NAME_MAX`] characters, and one of six colours;
/// both seen only on this computer.
pub(super) fn rename_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut RenameForm,
) -> bool {
    let Some(wallet) = cx.snap.wallets.iter().find(|w| w.id == form.id) else {
        return true;
    };
    // The circle in the header follows what is typed and chosen.
    let preview = evolite_api::WalletEntry {
        name: if form.name.trim().is_empty() {
            wallet.name.clone()
        } else {
            form.name.trim().to_owned()
        },
        color: form.color,
        ..wallet.clone()
    };
    let title = format!(tr!("Wallet “{}”"), crate::i18n::own(&wallet.name));
    dialog_with_header(
        ui,
        a,
        "rename-wallet",
        460.0,
        |ui, a| {
            let p = a.p;
            let (boxr, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
            ui.painter()
                .rect_filled(boxr, CornerRadius::same(14), p.raised);
            paint_wallet_dot(
                ui,
                Rect::from_center_size(boxr.center(), vec2(32.0, 32.0)),
                &preview,
            );
            ui.label(txt(&title, 20.0, Weight::Bold, p.text));
        },
        |ui, a| {
            let p = a.p;
            let mut close = false;
            ui.spacing_mut().item_spacing.y = 6.0;
            field_label(ui, &p, tr!("Title"));
            let field = text_field(
                ui,
                &p,
                "wallet-name",
                &mut form.name,
                "",
                FieldKind::Text,
                |_| {},
            );
            if field.changed() {
                form.error = None;
            }
            let len = form.name.trim().chars().count();
            if let Some(error) = &form.error {
                ui.label(txt(error, 12.0, Weight::Regular, p.danger));
            } else if len > NAME_MAX {
                ui.label(txt(
                    &format!(
                        tr!("No longer than {NAME_MAX} characters"),
                        NAME_MAX = NAME_MAX
                    ),
                    12.0,
                    Weight::Regular,
                    p.danger,
                ));
            }
            ui.add_space(12.0);
            field_label(ui, &p, tr!("Colour"));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for (i, color) in crate::theme::WALLET_COLORS.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                    let chosen = form.color as usize == i;
                    crate::components::named_choice(
                        &resp,
                        egui::WidgetType::RadioButton,
                        chosen,
                        COLOR_NAMES[i],
                    );
                    if chosen {
                        ui.painter()
                            .circle_stroke(rect.center(), 16.0, Stroke::new(2.0, p.text));
                    }
                    ui.painter().circle_filled(rect.center(), 13.0, *color);
                    if resp
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(crate::i18n::t(COLOR_NAMES[i]).into_owned())
                        .clicked()
                    {
                        form.color = i as u8;
                    }
                }
            });
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(txt(
                    tr!("Only you see the name and colour: they are kept on this computer and never sent to the network."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ))
                .wrap(),
            );
            ui.add_space(10.0);
            let ready = (1..=NAME_MAX).contains(&len) && !form.waiting;
            button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                let save =
                    button_enabled(ui, a, tr!("Save"), ButtonKind::Primary, None, ready).clicked();
                if ready && save {
                    cx.send(Command::RenameWallet {
                        id: form.id.clone(),
                        name: form.name.trim().to_owned(),
                        color: form.color,
                    });
                    form.waiting = true;
                }
            });
            close
        },
    )
}

/// The six wallet colours by name, for screen readers and the hover hint.
const COLOR_NAMES: [&str; 6] = [
    tr!("Yellow"),
    tr!("Blue"),
    tr!("Green"),
    tr!("Red"),
    tr!("Purple"),
    tr!("Grey"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::{WalletEntry, WalletOrigin};

    fn entry(id: &str, name: &str, color: u8) -> WalletEntry {
        WalletEntry {
            id: id.into(),
            name: name.into(),
            color,
            origin: WalletOrigin::Created,
            created_at: 0,
            last_opened: None,
            open: false,
            wrong_tries: 0,
            wait_until: None,
        }
    }

    /// BUG-23: a saved name is in the list as soon as the core says so, before its next
    /// snapshot; the other wallets are left as they are.
    #[test]
    fn a_saved_name_shows_before_the_next_snapshot() {
        let mut wallets = vec![entry("a", "Main", 0), entry("b", "Savings", 1)];
        let mut form = RenameForm {
            id: "b".into(),
            name: "  Holiday ".into(),
            color: 4,
            waiting: true,
            error: None,
        };
        assert!(form.on_done(&Ok(())));
        form.show_saved(&mut wallets);
        assert_eq!(wallets[1].name, "Holiday");
        assert_eq!(wallets[1].color, 4);
        assert_eq!((wallets[0].name.as_str(), wallets[0].color), ("Main", 0));
    }
}
