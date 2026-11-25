//! UI kit from Figma (pages Components and UI kit), used by every screen. Everything is painted
//! by hand: egui's own widgets don't take this styling. Screens keep only what is theirs.

mod avatar;
mod button;
mod card;
mod chip;
mod controls;
mod dialog;
mod fade;
mod field;
mod full_id;
mod identity_picker;
pub mod name_suggest;
mod notice;
mod page;
mod para;
mod progress;
mod qr;
mod rows;
pub mod secret_password;
pub mod spend_flow;
mod spend_password;

pub use avatar::*;
pub use button::*;
pub use card::*;
pub use chip::*;
pub use controls::*;
pub use dialog::*;
pub use fade::*;
pub use field::*;
pub use full_id::*;
pub use identity_picker::*;
pub use notice::*;
pub use page::*;
pub use para::*;
pub use progress::*;
pub use qr::*;
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

/// Text in Inter at a size and weight, in a colour; an address, a hash, an id or a key in it in
/// the monospace font (BUG-154, [`with_ids`]).
pub fn txt(s: &str, size: f32, w: Weight, color: Color32) -> Txt {
    Txt {
        text: crate::i18n::t(s).into_owned(),
        font: sans(size, w),
        color,
        line_height: None,
        underline: false,
    }
}

/// [`txt`] in a step of the type scale (FEAT-0033 step 2): its size, weight and font (a mono step
/// stays mono), an address in it in mono as [`txt`] draws it (BUG-154). Here and not in `theme`:
/// the installer builds `theme.rs` without the components.
pub fn text(s: &str, step: crate::theme::type_scale::Step, color: Color32) -> Txt {
    txt(s, step.size, step.weight, color).font(crate::theme::style(step))
}

/// What [`txt`] gives: a label's words, drawn by [`with_ids`].
#[must_use = "give it to a label"]
pub struct Txt {
    text: String,
    font: FontId,
    color: Color32,
    line_height: Option<f32>,
    underline: bool,
}

impl Txt {
    pub fn line_height(mut self, height: Option<f32>) -> Self {
        self.line_height = height;
        self
    }

    /// Underlined, as a link in words.
    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    /// Another font than Inter for the words.
    pub fn font(mut self, font: FontId) -> Self {
        self.font = font;
        self
    }
}

impl From<Txt> for egui::WidgetText {
    fn from(t: Txt) -> Self {
        if crate::text::address_runs(&t.text).is_empty() {
            let rich = RichText::new(t.text)
                .font(t.font)
                .color(t.color)
                .line_height(t.line_height);
            let rich = if t.underline { rich.underline() } else { rich };
            return rich.into();
        }
        let mut job = with_ids(&t.text, t.font, t.color, t.line_height);
        if t.underline {
            for section in &mut job.sections {
                section.format.underline = egui::Stroke::new(1.0, t.color);
            }
        }
        job.into()
    }
}

/// A label's text by [`with_figures`]: mono, its words in Inter (BUG-154).
pub fn fig_txt(text: &str, font: FontId, color: Color32) -> egui::WidgetText {
    with_figures(&crate::i18n::t(text), font, color, None).into()
}

/// [`with_ids`] laid out by `painter`, wrapped at `wrap` (infinite: one line): for text a screen
/// lays out itself.
pub fn ids_galley(
    painter: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    wrap: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = with_ids(text, font, color, None);
    job.wrap.max_width = wrap;
    painter.layout_job(job)
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
    let shown = crate::i18n::t(text);
    if crate::text::address_runs(&shown).is_empty() {
        return ui.painter().text(pos, anchor, shown, font, color);
    }
    // An address in the words: in the monospace font (BUG-154).
    let galley = ui.painter().layout_job(with_ids(&shown, font, color, None));
    let rect = anchor.anchor_size(pos, galley.size());
    ui.painter().galley(rect.min, galley, color);
    rect
}
