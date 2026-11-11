//! status/* chips (InstantLock, ChainLock, Proved, In the mempool, Testnet, Mainnet) and filter pills.

use super::prelude::*;
use evolite_api::Proof;

/// status/*: pill, raised fill; verified states in accent with an accent border.
pub fn chip(ui: &mut Ui, a: &mut Assets, icon: &str, label: &str, verified: bool) -> Response {
    let p = a.p;
    let color = if verified { p.accent } else { p.muted };
    chip_in(ui, a, icon, label, color, verified)
}

/// Which network the app is on: "Testnet" muted; "Mainnet" in the danger colour with a border,
/// so real money is never mistaken for test coins.
pub fn network_chip(ui: &mut Ui, a: &mut Assets, network: evolite_api::Network) -> Response {
    match network {
        evolite_api::Network::Testnet => chip(ui, a, "testnet", tr!("Testnet"), false),
        evolite_api::Network::Mainnet => {
            let danger = a.p.danger;
            chip_in(ui, a, "warning", tr!("Mainnet"), danger, true)
        }
    }
}

fn chip_in(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    label: &str,
    color: Color32,
    bordered: bool,
) -> Response {
    let p = a.p;
    let font = sans(12.0, Weight::SemiBold);
    let w = 8.0 + 14.0 + 6.0 + text_width(ui, label, font.clone()) + 10.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 25.0), Sense::hover());
    let stroke = if bordered {
        Stroke::new(1.0, color)
    } else {
        Stroke::NONE
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(255),
        p.raised,
        stroke,
        StrokeKind::Inside,
    );
    let icon_rect = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.center().y - 7.0),
        vec2(14.0, 14.0),
    );
    a.icons.icon(icon, color, &p, 14.0).paint_at(ui, icon_rect);
    paint_text(
        ui,
        pos2(icon_rect.right() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        color,
    );
    resp
}

/// How far a payment is proved: accent for the checked ones, muted for the mempool and for a
/// payment still on its way.
pub fn status_chip(ui: &mut Ui, a: &mut Assets, proof: Proof) -> Response {
    status_chip_of(ui, a, proof, false)
}

/// The words of a status chip; `core`: it speaks only of the Core transaction (FEAT-0016).
pub fn status_words(proof: Proof, core: bool) -> String {
    let (_, label) = crate::text::proof(proof);
    if core {
        format!("Core · {label}")
    } else {
        label.to_owned()
    }
}

/// [`status_chip`] that can say it is only the Core transaction's.
pub fn status_chip_of(ui: &mut Ui, a: &mut Assets, proof: Proof, core: bool) -> Response {
    let (icon, _) = crate::text::proof(proof);
    let label = status_words(proof, core);
    let label = label.as_str();
    let hint = crate::text::proof_hint(proof);
    let resp = chip(
        ui,
        a,
        icon,
        label,
        !matches!(proof, Proof::Mempool | Proof::Sending | Proof::Unconfirmed),
    );
    let both = format!(
        "{} · {}",
        crate::i18n::spoken(label),
        crate::i18n::spoken(hint)
    );
    named(
        &resp,
        egui::WidgetType::Label,
        true,
        &crate::i18n::own(&both),
    );
    resp.on_hover_text(crate::i18n::t(hint).into_owned())
}

/// The height of a pill and of the buttons that stand in a row with pills («Filters»): one
/// height, so a row of them stands on one axis (FEAT-0034 item 2).
pub const PILL_HEIGHT: f32 = 34.0;

/// How wide [`pill`] is with `label`.
pub fn pill_width(ui: &Ui, label: &str) -> f32 {
    text_width(ui, label, sans(13.0, Weight::SemiBold)) + 28.0
}

/// A filter pill: ink when on, surface with a 1 px line when off. [`PILL_HEIGHT`] high.
pub fn pill(ui: &mut Ui, p: &Palette, label: &str, on: bool) -> Response {
    let font = sans(13.0, Weight::SemiBold);
    let w = pill_width(ui, label);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, PILL_HEIGHT), Sense::click());
    let (fill, stroke, color) = if on {
        (p.ink, Stroke::NONE, p.on_ink)
    } else {
        let color = if resp.hovered() { p.text } else { p.muted };
        (p.surface, Stroke::new(1.0, p.line), color)
    };
    ui.painter()
        .rect(rect, 255, fill, stroke, StrokeKind::Inside);
    let shown = crate::i18n::t(label);
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, &shown, font, color);
    named_choice(&resp, egui::WidgetType::SelectableLabel, on, label);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// The pill that adds one more to a row of [`pill`]s (FEAT-0014, «+ New identity»): a dashed
/// muted line, a plus and the words 12 SemiBold, as high as a pill. `name` is what a screen
/// reader hears. Off (`enabled` false): at 45 %, no click and no Tab stop; the caller says why
/// in words beside it.
pub fn add_pill(ui: &mut Ui, a: &mut Assets, label: &str, name: &str, enabled: bool) -> Response {
    let p = a.p;
    let font = sans(12.0, Weight::SemiBold);
    // Padding 14, the plus 14 and a gap of 6 before the words.
    let w = 14.0 + 14.0 + 6.0 + text_width(ui, label, font.clone()) + 14.0;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, PILL_HEIGHT), sense);
    named(&resp, egui::WidgetType::Button, enabled, name);
    let over = enabled && (resp.hovered() || resp.has_focus());
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let line = fade(if over { p.text } else { p.muted });
    let color = fade(if over { p.text } else { p.muted });
    // A dashed line around the rounded pill: its straight sides and, at each end, a half
    // circle of short arcs.
    let r = rect.shrink(0.5);
    let radius = r.height() / 2.0;
    let mut points = vec![
        pos2(r.left() + radius, r.top()),
        pos2(r.right() - radius, r.top()),
    ];
    let arc = |center: egui::Pos2, from: f32, points: &mut Vec<egui::Pos2>| {
        for i in 1..=12 {
            let t = from + std::f32::consts::PI * i as f32 / 12.0;
            points.push(center + radius * vec2(t.cos(), t.sin()));
        }
    };
    arc(
        pos2(r.right() - radius, r.center().y),
        -std::f32::consts::FRAC_PI_2,
        &mut points,
    );
    points.push(pos2(r.left() + radius, r.bottom()));
    arc(
        pos2(r.left() + radius, r.center().y),
        std::f32::consts::FRAC_PI_2,
        &mut points,
    );
    ui.painter().extend(egui::Shape::dashed_line(
        &points,
        Stroke::new(1.0, line),
        4.0,
        3.0,
    ));
    let plus = Rect::from_center_size(
        pos2(rect.left() + 14.0 + 7.0, rect.center().y),
        vec2(14.0, 14.0),
    );
    a.icons.icon("plus", color, &p, 14.0).paint_at(ui, plus);
    paint_text(
        ui,
        pos2(plus.right() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        color,
    );
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}
