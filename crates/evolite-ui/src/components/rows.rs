//! Rows and tiles: sidebar item, transaction row, balance card, icon box.

use super::prelude::*;
use evolite_api::Proof;

/// nav/default · nav/active: 40 px, radius 10, icon 20 + label 14. In the rail only the icon.
pub fn nav_item(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    label: &str,
    active: bool,
    rail: bool,
) -> Response {
    let p = a.p;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    named_choice(&resp, egui::WidgetType::SelectableLabel, active, label);
    let hovered = resp.hovered();
    if active || hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), p.raised);
    }
    let color = if active || hovered { p.text } else { p.muted };
    let icon_x = if rail {
        rect.center().x - 10.0
    } else {
        rect.left() + 12.0
    };
    let icon_rect = Rect::from_min_size(pos2(icon_x, rect.center().y - 10.0), vec2(20.0, 20.0));
    a.icons.icon(icon, color, &p, 20.0).paint_at(ui, icon_rect);
    if !rail {
        let font = sans(
            14.0,
            if active {
                Weight::SemiBold
            } else {
                Weight::Medium
            },
        );
        paint_text(
            ui,
            pos2(icon_rect.right() + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            font,
            color,
        );
    }
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if rail {
        resp.on_hover_text(crate::i18n::t(label).into_owned())
    } else {
        resp
    }
}

/// One operation, already in words. `proof` is `None` where a status means nothing (the
/// owner's own pool operations).
pub struct TxRow<'a> {
    pub icon: &'a str,
    pub title: &'a str,
    pub detail: &'a str,
    pub proof: Option<Proof>,
    pub amount: &'a str,
}

/// row/transaction: 54 px; icon box 36, title and line under it, status chip, amount in mono.
pub fn tx_row(ui: &mut Ui, a: &mut Assets, row: &TxRow) {
    let _ = tx_row_sensed(ui, a, row, Sense::hover());
}

/// [`tx_row`] that can be clicked (it opens the operation's card).
pub fn tx_row_clickable(ui: &mut Ui, a: &mut Assets, row: &TxRow) -> egui::Response {
    tx_row_sensed(ui, a, row, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn tx_row_sensed(ui: &mut Ui, a: &mut Assets, row: &TxRow, sense: Sense) -> egui::Response {
    let p = a.p;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), sense);
    named(
        &resp,
        if sense.senses_click() {
            egui::WidgetType::Button
        } else {
            egui::WidgetType::Label
        },
        true,
        &format!("{} · {} · {}", row.title, row.amount, row.detail),
    );
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(12), p.surface);
    }
    let cy = rect.center().y;
    let boxr = Rect::from_min_size(pos2(rect.left() + 16.0, cy - 18.0), vec2(36.0, 36.0));
    paint_icon_box(ui, a, boxr, 10, row.icon, 18.0, p.ink);

    let amount_right = rect.right() - 16.0;
    // "Hide amounts": the row under the pointer shows its amount.
    let hidden = crate::format::display().hide && !ui.rect_contains_pointer(rect);
    let masked = hidden.then(|| crate::format::mask(row.amount));
    paint_text(
        ui,
        pos2(amount_right, cy),
        Align2::RIGHT_CENTER,
        masked.as_deref().unwrap_or(row.amount),
        mono(14.0, Weight::Medium),
        p.text,
    );

    // The chip sits left of a 150 px amount column, as in Figma.
    let mut text_right = amount_right - 150.0;
    if let Some(proof) = row.proof {
        let (_, label) = crate::text::proof(proof);
        let chip_w = 8.0 + 14.0 + 6.0 + text_width(ui, label, sans(12.0, Weight::SemiBold)) + 10.0;
        let chip_rect = Rect::from_min_size(
            pos2(amount_right - 150.0 - 14.0 - chip_w, cy - 12.5),
            vec2(chip_w, 25.0),
        );
        let mut chip_ui = ui.new_child(egui::UiBuilder::new().max_rect(chip_rect));
        status_chip(&mut chip_ui, a, proof);
        text_right = chip_rect.left();
    }

    let tx = boxr.right() + 14.0;
    let title = ui.painter().layout(
        crate::i18n::t(row.title).into_owned(),
        sans(14.0, Weight::SemiBold),
        p.text,
        text_right - 14.0 - tx,
    );
    ui.painter().galley(pos2(tx, cy - 17.0), title, p.text);
    paint_text(
        ui,
        pos2(tx, cy + 9.0),
        Align2::LEFT_CENTER,
        row.detail,
        sans(12.0, Weight::Regular),
        p.muted,
    );
    resp
}

/// An amount as a label. With "Hide amounts" it reads "•••• DASH" until the pointer
/// is over it; whether it was is kept from the last frame.
pub fn amount_label(ui: &mut Ui, shown: &str, font: FontId, color: Color32) -> Response {
    if !crate::format::display().hide {
        return ui.label(
            egui::RichText::new(crate::i18n::t(shown))
                .font(font)
                .color(color),
        );
    }
    let id = ui.next_auto_id().with("amount-shown");
    let revealed = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let text = if revealed {
        shown.to_owned()
    } else {
        crate::format::mask(shown)
    };
    let resp = ui.label(
        egui::RichText::new(crate::i18n::t(&text))
            .font(font)
            .color(color),
    );
    let over = resp.contains_pointer();
    if over != revealed {
        ui.data_mut(|d| d.insert_temp(id, over));
        ui.ctx().request_repaint();
    }
    resp
}

/// card/balance: icon 18 + title 13 Medium, amount 24 mono, detail 12.
pub fn balance_card(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    title: &str,
    duffs: i64,
    detail: &str,
    width: f32,
) {
    let p = a.p;
    card(
        &p,
        Margin {
            left: 20,
            right: 20,
            top: 18,
            bottom: 18,
        },
    )
    .show(ui, |ui| {
        ui.set_width(width - 42.0);
        ui.vertical(|ui| balance_card_body(ui, a, icon, title, duffs, detail));
    });
}

fn balance_card_body(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    title: &str,
    duffs: i64,
    detail: &str,
) {
    let p = a.p;
    {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add(a.icons.icon(icon, p.muted, &p, 18.0));
            ui.label(
                egui::RichText::new(crate::i18n::t(title))
                    .font(sans(13.0, Weight::Medium))
                    .color(p.muted),
            );
        });
        amount_label(
            ui,
            &crate::format::number(duffs, false),
            mono(24.0, Weight::Medium),
            p.text,
        );
        ui.label(
            egui::RichText::new(crate::i18n::t(detail))
                .font(sans(12.0, Weight::Regular))
                .color(p.muted),
        );
    }
}

/// A raised rounded square with an icon in the middle (transaction rows, actions, dialogs).
pub fn paint_icon_box(
    ui: &Ui,
    a: &mut Assets,
    rect: Rect,
    radius: u8,
    icon: &str,
    icon_size: f32,
    color: Color32,
) {
    let p = a.p;
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius), p.raised);
    a.icons.icon(icon, color, &p, icon_size).paint_at(
        ui,
        Rect::from_center_size(rect.center(), vec2(icon_size, icon_size)),
    );
}

/// A wallet's circle (Figma 26 "wallet dot"): its colour and its name's first letter.
pub fn paint_wallet_dot(ui: &Ui, rect: Rect, wallet: &evolite_api::WalletEntry) {
    let color =
        crate::theme::WALLET_COLORS[wallet.color as usize % crate::theme::WALLET_COLORS.len()];
    ui.painter()
        .circle_filled(rect.center(), rect.width() / 2.0, color);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        wallet.letter(),
        sans((rect.width() * 0.46).round(), Weight::SemiBold),
        crate::theme::WALLET_LETTER,
    );
}

/// An app's logo until apps send theirs: its first letter on a raised square with a line.
pub fn paint_letter_box(ui: &Ui, p: &Palette, rect: Rect, name: &str) {
    let radius = (rect.width() * 0.28) as u8;
    ui.painter().rect(
        rect,
        CornerRadius::same(radius),
        p.raised,
        Stroke::new(1.0, p.line),
        StrokeKind::Inside,
    );
    let letter: String = name.chars().next().map(String::from).unwrap_or_default();
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        crate::i18n::t(&letter),
        sans((rect.width() * 0.41).round(), Weight::Bold),
        p.text,
    );
}
