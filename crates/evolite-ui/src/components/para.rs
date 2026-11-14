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

    /// The words as a label shows them; an address in them in the monospace font (BUG-154).
    fn rich(&self, shown: String) -> egui::WidgetText {
        // A mono paragraph (an address, a txid) keeps a name or words in it in Inter (BUG-154).
        let mono = matches!(&self.font.family, egui::FontFamily::Monospace)
            || matches!(&self.font.family, egui::FontFamily::Name(n) if n.starts_with("mono"));
        if mono {
            return with_figures(&shown, self.font.clone(), self.color, self.line_height).into();
        }
        if address_runs(&shown).is_empty() {
            return RichText::new(shown)
                .font(self.font.clone())
                .color(self.color)
                .line_height(self.line_height)
                .into();
        }
        with_ids(&shown, self.font.clone(), self.color, self.line_height).into()
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

/// Whether `run` (letters, digits and «…», nothing else) is written like an address, a hash, an
/// id or a key (BUG-154): a Platform address, a long Base58 or hex string with both letters and
/// digits, or one shortened around «…» with a digit in it.
pub fn address_like(run: &str) -> bool {
    let lower = run.to_ascii_lowercase();
    if lower.starts_with("tdash1") || lower.starts_with("dash1") {
        return true;
    }
    let has_digit = run.chars().any(|c| c.is_ascii_digit());
    let has_letter = run.chars().any(|c| c.is_ascii_alphabetic());
    if let Some((head, tail)) = run.split_once('…') {
        return head.len() >= 3 && tail.len() >= 3 && has_digit && has_letter;
    }
    run.len() >= 26 && has_digit && has_letter
}

/// The runs of `text` that are addresses, hashes, ids or keys ([`address_like`]), as byte
/// ranges: drawn in the monospace font, as Inter's «x» between digits reads as «×» (BUG-154).
pub fn address_runs(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut found = Vec::new();
    let mut start = None;
    let piece = |c: char| c.is_ascii_alphanumeric() || c == '…';
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (start, piece(c)) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                if address_like(&text[s..i]) {
                    found.push(s..i);
                }
                start = None;
            }
            _ => {}
        }
    }
    found
}

/// BUG-154: `text` (already in the interface language) in `font`, but every address, hash, id
/// and key in it ([`address_runs`]) in the monospace font of the same size and
/// weight. In Inter a lowercase «x» between digits reads as «×», and an address is checked by
/// eye before signing.
pub fn with_ids(
    text: &str,
    font: FontId,
    color: Color32,
    line_height: Option<f32>,
) -> egui::text::LayoutJob {
    let mono_font = mono_like(&font);
    let format = |font: FontId| egui::TextFormat {
        font_id: font,
        color,
        line_height,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    let mut at = 0;
    for run in address_runs(text) {
        if run.start > at {
            job.append(&text[at..run.start], 0.0, format(font.clone()));
        }
        job.append(&text[run.clone()], 0.0, format(mono_font.clone()));
        at = run.end;
    }
    if at < text.len() {
        job.append(&text[at..], 0.0, format(font.clone()));
    }
    job
}

/// The monospace font as heavy as `font`: Regular for Inter 400, Medium otherwise.
fn mono_like(font: &FontId) -> FontId {
    match &font.family {
        egui::FontFamily::Monospace => font.clone(),
        egui::FontFamily::Name(name) if name.starts_with("mono") => font.clone(),
        egui::FontFamily::Proportional => crate::theme::mono(font.size, Weight::Regular),
        _ => crate::theme::mono(font.size, Weight::Medium),
    }
}

/// BUG-154, the designer: a line meant for the monospace font (`font`, mono) whose words are
/// set in Inter of the same size and weight — figures, amounts, ids and codes (DASH, USD: all
/// capitals, five letters at most) stay mono, a word or an @name does not: «@satoshi ·
/// credits», «yPq8…8e4k · this wallet · Nov 14». A line with no words is all mono, as before.
pub fn with_figures(
    text: &str,
    font: FontId,
    color: Color32,
    line_height: Option<f32>,
) -> egui::text::LayoutJob {
    let words_font = sans_like(&font);
    let ids = address_runs(text);
    let format = |font: FontId| egui::TextFormat {
        font_id: font,
        color,
        line_height,
        ..Default::default()
    };
    // Each char: whether it belongs to a word (Inter) rather than to the figures (mono).
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut is_word = vec![false; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        if ids.iter().any(|r| r.start <= at && at < r.end) {
            i += 1;
            continue;
        }
        // A run of letters (with an @ before it, an @name), cut where it ends.
        let starts_name = c == '@';
        if c.is_alphabetic() || starts_name {
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].1.is_alphabetic()
                    || (starts_name && (chars[j].1.is_alphanumeric() || chars[j].1 == '-')))
            {
                j += 1;
            }
            let word: String = chars[i..j].iter().map(|(_, c)| c).collect();
            let letters = word.trim_start_matches('@');
            let code = letters.chars().count() <= 5 && letters.chars().all(|c| c.is_uppercase());
            if starts_name || (!code && letters.chars().count() >= 2) {
                is_word[i..j].iter_mut().for_each(|w| *w = true);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    let mut job = egui::text::LayoutJob::default();
    let mut run_start = 0;
    for k in 1..=chars.len() {
        let end = chars.get(k).map_or(text.len(), |(at, _)| *at);
        if k == chars.len() || is_word[k] != is_word[k - 1] {
            let start = chars[run_start].0;
            let f = if is_word[k - 1] { &words_font } else { &font };
            job.append(&text[start..end], 0.0, format(f.clone()));
            run_start = k;
        }
    }
    job
}

/// Inter as heavy as the monospace `font`: Regular for mono 400, Medium otherwise.
fn sans_like(font: &FontId) -> FontId {
    match &font.family {
        egui::FontFamily::Monospace => sans(font.size, Weight::Regular),
        egui::FontFamily::Name(name) if name.starts_with("mono") => sans(font.size, Weight::Medium),
        _ => font.clone(),
    }
}
