//! Choices: option cards, segmented control, toggle, checkbox.

use super::prelude::*;

/// One option of a segmented choice: icon 18, title 13 SemiBold and a mono line 11 (none when
/// `detail` is empty: then the option is 38 px high).
/// Selected — raised fill and an ink border; the rest — background fill and a line border.
pub fn option_card(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    title: &str,
    detail: &str,
    selected: bool,
) -> Response {
    option_card_in(
        ui,
        a,
        icon,
        title,
        detail,
        selected,
        CardColumn {
            width: 0.0,
            icon: true,
        },
    )
}

/// The padding of an option card left of its icon and right of its words.
const CARD_LEFT: f32 = 14.0;
const CARD_RIGHT: f32 = 16.0;
/// The icon and the gap after it.
const CARD_ICON: f32 = 18.0 + 10.0;

/// How wide an [`option_card`] must be to hold its words with the full padding on the right:
/// with its icon, or without it (the least a column may be, [`option_card_in`]).
pub fn option_card_width(ui: &Ui, title: &str, detail: &str, with_icon: bool) -> f32 {
    let words = text_width(ui, title, sans(13.0, Weight::SemiBold)).max(text_width(
        ui,
        detail,
        mono(11.0, Weight::Regular),
    ));
    CARD_LEFT + if with_icon { CARD_ICON } else { 0.0 } + words + CARD_RIGHT
}

/// The column an [`option_card_in`] stands in: its width, and whether the row's cards show their
/// icons — one answer for the whole row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardColumn {
    pub width: f32,
    pub icon: bool,
}

/// [`option_card`] in a column `width` wide (a row of cards in equal columns, BUG-67): the
/// words keep the full padding on the right (BUG-81). `icon` is decided for the whole row
/// by the caller (the designer, BUG-81: icons on every card of a row or on none), never card by
/// card. Never narrower than its words: the caller picks columns that hold them
/// (`option_card_width(.., false)`). 0: as wide as its icon and words.
pub fn option_card_in(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    title: &str,
    detail: &str,
    selected: bool,
    column: CardColumn,
) -> Response {
    let CardColumn {
        width,
        icon: with_icon,
    } = column;
    let p = a.p;
    let title_font = sans(13.0, Weight::SemiBold);
    let detail_font = mono(11.0, Weight::Regular);
    let full = option_card_width(ui, title, detail, true);
    let width = if width > 0.0 {
        width.max(option_card_width(ui, title, detail, false))
    } else {
        full
    };
    let (rect, resp) = ui.allocate_exact_size(
        vec2(width, if detail.is_empty() { 38.0 } else { 52.0 }),
        Sense::click(),
    );
    named_choice(&resp, egui::WidgetType::RadioButton, selected, title);
    let hovered = resp.hovered();
    let (fill, stroke) = if selected {
        (p.raised, p.ink)
    } else if hovered {
        (p.raised, p.line)
    } else {
        (p.bg, p.line)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(12),
        fill,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    let color = if selected { p.text } else { p.muted };
    let mut x = rect.left() + CARD_LEFT;
    if with_icon {
        let icon_rect = Rect::from_min_size(pos2(x, rect.center().y - 9.0), vec2(18.0, 18.0));
        a.icons.icon(icon, color, &p, 18.0).paint_at(ui, icon_rect);
        x += CARD_ICON;
    }
    if detail.is_empty() {
        paint_text(
            ui,
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            title,
            title_font,
            color,
        );
    } else {
        paint_text(
            ui,
            pos2(x, rect.center().y - 1.0),
            Align2::LEFT_BOTTOM,
            title,
            title_font,
            color,
        );
        paint_text(
            ui,
            pos2(x, rect.center().y + 1.0),
            Align2::LEFT_TOP,
            detail,
            detail_font,
            p.muted,
        );
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Toggle 40 × 22, `label` read out by screen readers: on — ink track with an on-ink knob on the right; off — raised track, muted
/// knob on the left.
pub fn toggle(ui: &mut Ui, p: &Palette, on: bool, label: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    named_choice(&resp, egui::WidgetType::Checkbox, on, label);
    let (track, stroke, knob, x) = if on {
        (p.ink, p.ink, p.on_ink, rect.right() - 11.0)
    } else {
        (p.raised, p.field_line, p.muted, rect.left() + 11.0)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(11),
        track,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    ui.painter()
        .circle_filled(pos2(x, rect.center().y), 7.0, knob);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Segmented control: a track with a 1 px line, the chosen item raised. On a card the track is
/// the page colour (items padded 12), on the page it is the card colour (items padded 14).
pub fn segmented(ui: &mut Ui, p: &Palette, items: &[&str], selected: &mut usize, on_card: bool) {
    segmented_with_off(ui, p, items, selected, on_card, &[]);
}

/// [`segmented`] where an item with `Some(hint)` in `off` (by position) cannot be chosen: it
/// is drawn faded and says why on hover.
pub fn segmented_with_off(
    ui: &mut Ui,
    p: &Palette,
    items: &[&str],
    selected: &mut usize,
    on_card: bool,
    off: &[Option<&str>],
) {
    let font = sans(13.0, Weight::SemiBold);
    let pad = if on_card { 24.0 } else { 28.0 };
    let widths: Vec<f32> = items
        .iter()
        .map(|s| text_width(ui, s, font.clone()) + pad)
        .collect();
    let total = 8.0 + widths.iter().sum::<f32>() + 4.0 * (items.len() - 1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 36.0), Sense::hover());
    let track = if on_card { p.bg } else { p.surface };
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        track,
        Stroke::new(1.0, p.line),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + 4.0;
    for (i, (label, w)) in items.iter().zip(widths).enumerate() {
        let item = Rect::from_min_size(pos2(x, rect.top() + 4.0), vec2(w, 28.0));
        let hint = off.get(i).copied().flatten();
        let sense = if hint.is_some() {
            Sense::hover()
        } else {
            Sense::click()
        };
        let resp = ui.interact(item, ui.id().with(("segmented", label)), sense);
        let on = *selected == i;
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::RadioButton, hint.is_none(), on, *label)
        });
        if on {
            ui.painter()
                .rect_filled(item, CornerRadius::same(7), p.raised);
        }
        let color = if hint.is_some() {
            p.muted.gamma_multiply(0.5)
        } else if on || resp.hovered() {
            p.text
        } else {
            p.muted
        };
        ui.painter().text(
            item.center(),
            Align2::CENTER_CENTER,
            crate::i18n::t(label),
            font.clone(),
            color,
        );
        if let Some(hint) = hint {
            resp.on_hover_cursor(CursorIcon::NotAllowed)
                .on_hover_text(crate::i18n::t(hint).into_owned());
        } else if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            *selected = i;
        }
        x += w + 4.0;
    }
}

/// Checkbox 20 px with a label: ink box with a tick when on.
pub fn checkbox(ui: &mut Ui, a: &mut Assets, on: &mut bool, label: &str) {
    let p = a.p;
    let row = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            // The words are part of the checkbox: a selectable label would take their clicks.
            ui.style_mut().interaction.selectable_labels = false;
            let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            if *on {
                ui.painter().rect_filled(rect, CornerRadius::same(6), p.ink);
                a.icons
                    .icon("check", p.on_ink, &p, 14.0)
                    .paint_at(ui, Rect::from_center_size(rect.center(), vec2(14.0, 14.0)));
            } else {
                ui.painter().rect(
                    rect,
                    CornerRadius::same(6),
                    p.raised,
                    Stroke::new(1.0, p.muted),
                    StrokeKind::Inside,
                );
            }
            ui.label(txt(label, 14.0, Weight::Regular, p.text));
        })
        .response;
    // On top of the box and the words: a click on either ticks it.
    let resp = crate::a11y::row_click(ui, &row, "checkbox");
    named_choice(&resp, egui::WidgetType::Checkbox, *on, label);
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if resp.clicked() {
        *on = !*on;
    }
}

/// A select row: raised, 1 px line, icon 18, a label, an optional mono value on the right and a
/// chevron. Clicking it is the caller's to answer (open a list, cycle the choice).
pub fn select_row(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    label: &str,
    value: Option<&str>,
) -> Response {
    let p = a.p;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    match value {
        Some(value) => named(
            &resp,
            egui::WidgetType::ComboBox,
            true,
            &format!("{label}: {value}"),
        ),
        None => named(&resp, egui::WidgetType::ComboBox, true, label),
    }
    let stroke = if resp.hovered() { p.muted } else { p.line };
    ui.painter().rect(
        rect,
        CornerRadius::same(12),
        p.bg,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    let cy = rect.center().y;
    a.icons.icon(icon, p.text, &p, 18.0).paint_at(
        ui,
        Rect::from_min_size(pos2(rect.left() + 16.0, cy - 9.0), vec2(18.0, 18.0)),
    );
    paint_text(
        ui,
        pos2(rect.left() + 46.0, cy),
        Align2::LEFT_CENTER,
        label,
        sans(14.0, Weight::SemiBold),
        p.text,
    );
    let chevron = Rect::from_center_size(pos2(rect.right() - 24.0, cy), vec2(16.0, 16.0));
    a.icons
        .icon("chevron-down", p.muted, &p, 16.0)
        .paint_at(ui, chevron);
    if let Some(value) = value {
        paint_text(
            ui,
            pos2(chevron.left() - 12.0, cy),
            Align2::RIGHT_CENTER,
            value,
            mono(13.0, Weight::Regular),
            p.muted,
        );
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}
