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
    let (icon, label) = crate::text::proof(proof);
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

/// A filter pill: ink when on, surface with a 1 px line when off. 33 px high.
pub fn pill(ui: &mut Ui, p: &Palette, label: &str, on: bool) -> Response {
    let font = sans(13.0, Weight::SemiBold);
    let w = text_width(ui, label, font.clone()) + 28.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 33.0), Sense::click());
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
