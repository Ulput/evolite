//! A paragraph: text that may take more than one line. The installer has it too
//! (`setup/main.rs`), so it needs only `i18n` and `theme`.

use crate::theme::{Weight, sans};
use egui::{Color32, FontId, Response, RichText, Ui};

/// Text in Inter at a size and weight, in a colour, wrapped to the width there is: what
/// `egui::Label::new(txt(..)).wrap()` draws, except in the Arabic interface. egui wraps a line
/// after [`crate::i18n::visual`] has put it in visual order, so a right-to-left paragraph came
/// out with its end in the first line (BUG-119); here [`crate::i18n::t_wrapped`] breaks it in
/// reading order first. Text without Arabic goes through the very same `Label` as before.
pub fn para(text: &str, size: f32, w: Weight, color: Color32) -> Para<'_> {
    Para {
        text,
        font: sans(size, w),
        color,
        line_height: None,
    }
}

#[must_use = "add it with `ui.add`"]
pub struct Para<'a> {
    text: &'a str,
    font: FontId,
    color: Color32,
    line_height: Option<f32>,
}

impl Para<'_> {
    /// Another font than Inter (mono for a link, an address).
    pub fn font(mut self, font: FontId) -> Self {
        self.font = font;
        self
    }

    pub fn line_height(mut self, height: f32) -> Self {
        self.line_height = Some(height);
        self
    }

    fn rich(&self, shown: String) -> RichText {
        RichText::new(shown)
            .font(self.font.clone())
            .color(self.color)
            .line_height(self.line_height)
    }
}

/// Whether `text` is Arabic in the Arabic interface: the only text [`lines`] breaks itself, and
/// the only text cut by words in reading order rather than by egui (BUG-118).
pub(crate) fn breaks_itself(text: &str) -> bool {
    crate::i18n::rtl()
        && crate::i18n::t_in(crate::i18n::current(), text)
            .chars()
            .any(crate::i18n::is_rtl_char)
}

/// What to give egui to lay out `text` in `font`, wrapped at `width`: [`crate::i18n::t`] of it,
/// or, for Arabic in the Arabic interface, its lines from [`crate::i18n::t_wrapped`], each
/// within `width`, so egui's own wrap finds nothing left to break (BUG-119).
pub fn lines(ctx: &egui::Context, text: &str, font: &FontId, width: f32) -> String {
    lines_measured(text, font, width, |line| {
        ctx.fonts_mut(|f| f.layout_no_wrap(line.to_owned(), font.clone(), Color32::WHITE))
            .size()
            .x
    })
}

/// [`lines`] for text drawn in more than one font: `measure` says how wide a line is drawn.
/// `font` is the main one, a part of what the lines are remembered by.
pub fn lines_measured(
    text: &str,
    font: &FontId,
    width: f32,
    measure: impl Fn(&str) -> f32,
) -> String {
    arranged(text, font, width, usize::MAX, measure)
}

/// What to give egui to lay out `text` in `font` in at most `rows` rows of `width`, the last
/// ending in «…» when the text is longer: [`crate::i18n::t`] of it, for egui's own cut, or, for
/// Arabic in the Arabic interface, its lines from [`crate::i18n::t_wrapped_rows`], cut at a word
/// from its end in reading order — egui's cut kept the left of a row, the end of an Arabic
/// phrase, and dropped its start (BUG-118).
pub fn cut_to(ctx: &egui::Context, text: &str, font: &FontId, width: f32, rows: usize) -> String {
    arranged(text, font, width, rows, |line| {
        ctx.fonts_mut(|f| f.layout_no_wrap(line.to_owned(), font.clone(), Color32::WHITE))
            .size()
            .x
    })
}

/// [`lines_measured`] in at most `rows` lines; remembered by text, width, font and rows.
fn arranged(
    text: &str,
    font: &FontId,
    width: f32,
    rows: usize,
    measure: impl Fn(&str) -> f32,
) -> String {
    if !breaks_itself(text) || !width.is_finite() || width <= 0.0 {
        return crate::i18n::t(text).into_owned();
    }
    // Whole points: the lines are remembered by width, and a line that fits the width
    // rounded down fits the width.
    let width = width.floor();
    // The font by its family and the bits of its size: `FontId` itself is no key (an `f32`).
    type Key = (&'static str, String, i32, u32, egui::FontFamily, usize);
    thread_local! {
        static SEEN: std::cell::RefCell<std::collections::HashMap<Key, String>> =
            Default::default();
    }
    let key: Key = (
        crate::i18n::current().code(),
        text.to_owned(),
        width as i32,
        font.size.to_bits(),
        font.family.clone(),
        rows,
    );
    if let Some(found) = SEEN.with(|s| s.borrow().get(&key).cloned()) {
        return found;
    }
    let fits = |line: &str| measure(line) <= width;
    let out = if rows == usize::MAX {
        crate::i18n::t_wrapped(text, fits)
    } else {
        crate::i18n::t_wrapped_rows(text, fits, rows)
    };
    SEEN.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() > 5_000 {
            s.clear();
        }
        s.insert(key, out.clone());
    });
    out
}

impl egui::Widget for Para<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let width = ui.available_width();
        let horizontal_wrap =
            ui.layout().main_dir() == egui::Direction::LeftToRight && ui.layout().main_wrap();
        if !breaks_itself(self.text) || horizontal_wrap || !width.is_finite() || width <= 0.0 {
            let shown = crate::i18n::t(self.text).into_owned();
            return ui.add(egui::Label::new(self.rich(shown)).wrap());
        }
        let shown = lines(ui.ctx(), self.text, &self.font, width);
        let broken = shown.matches('\n').count() > self.text.matches('\n').count();
        let response = ui.add(egui::Label::new(self.rich(shown)).wrap());
        if !broken {
            // One line, as a Label shows it: the same words for a screen reader too.
            return response;
        }
        // Lines each in visual order read nothing like the paragraph: a screen reader hears it
        // in reading order.
        let spoken = crate::i18n::spoken(self.text);
        let enabled = ui.is_enabled();
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, enabled, &spoken));
        response
    }
}
