//! UI kit from Figma (pages Components and UI kit), used by every screen. Everything is painted
//! by hand: egui's own widgets don't take this styling. Screens keep only what is theirs.

mod avatar;
mod button;
mod card;
mod chip;
mod controls;
mod dialog;
mod field;
mod notice;
mod page;
mod progress;
mod rows;
mod spend_password;

pub use avatar::*;
pub use button::*;
pub use card::*;
pub use chip::*;
pub use controls::*;
pub use dialog::*;
pub use field::*;
pub use notice::*;
pub use page::*;
pub use progress::*;
pub use rows::*;
pub use spend_password::*;

/// What the component files share; a glob import keeps their headers short.
mod prelude {
    pub(crate) use super::*;
    pub(crate) use crate::theme::{Palette, Weight, mono, sans};
    pub(crate) use egui::{
        Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Layout, Margin, Rect,
        Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
    };
}

use crate::icons::Icons;
use crate::theme::{Palette, Weight, sans};
use egui::{Align2, Color32, FontId, Rect, RichText, Ui};

/// Text in Inter at a size and weight, in a colour.
pub fn txt(s: &str, size: f32, w: Weight, color: Color32) -> RichText {
    RichText::new(crate::i18n::t(s))
        .font(sans(size, w))
        .color(color)
}

pub struct Assets {
    pub p: Palette,
    pub icons: Icons,
    /// Profile pictures, drawn by the core like NFT pictures (keys start with `avatar:`).
    pub avatars: crate::art::ArtCache,
}

/// Names a hand-painted widget for screen readers: AccessKit sees only what egui is told. The
/// name is in the interface language, in reading order.
pub(crate) fn named(resp: &egui::Response, kind: egui::WidgetType, enabled: bool, label: &str) {
    let label = crate::i18n::spoken(label);
    resp.widget_info(|| egui::WidgetInfo::labeled(kind, enabled, &label));
}

/// [`named`] for something that is on or off: a pill, a toggle, a checkbox, a tab.
pub(crate) fn named_choice(resp: &egui::Response, kind: egui::WidgetType, on: bool, label: &str) {
    let label = crate::i18n::spoken(label);
    resp.widget_info(|| egui::WidgetInfo::selected(kind, true, on, &label));
}

pub(crate) fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(crate::i18n::t(text).into_owned(), font, Color32::WHITE)
        .size()
        .x
}

pub(crate) fn paint_text(
    ui: &Ui,
    pos: egui::Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    color: Color32,
) -> Rect {
    ui.painter()
        .text(pos, anchor, crate::i18n::t(text), font, color)
}
