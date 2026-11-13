//! The interface in 15 languages: `en`, `zh-Hans`, `hi`, `es`, `fr`, `ar`, `pt-BR`, `ru`, `id`,
//! `de`, `ja`, `tr`, `ko`, `vi`, `uk`.
//!
//! The screens are written in English (each text marked with `tr!`) and translated where text
//! reaches the screen — in `txt`, `paint_text` and the few other places that draw words — not
//! at every call site: the English text is the key. A catalog per language
//! (`locales/<code>.json`, built into the app) maps each English string to its translation.
//! Strings made with `format!` are found by their template: `"Hides in {} s"` matches
//! "Hides in 12 s", and what stood in for `{}` is translated too (a reason inside a sentence)
//! and put back. A key of two English forms joined by `|` is one word in its plural forms; its
//! translation is an object of CLDR categories (`one`, `few`, `many`, `other`, …), chosen by
//! the number found. Text without a translation stays English — never empty, never a key.

mod catalog;
mod plural;

pub use catalog::Catalog;

use std::borrow::Cow;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{OnceLock, RwLock};

/// The interface languages, in the order the language list shows them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Lang {
    Ru,
    En,
    ZhHans,
    Hi,
    Es,
    Fr,
    Ar,
    PtBr,
    Id,
    De,
    Ja,
    Tr,
    Ko,
    Vi,
    Uk,
}

impl Lang {
    pub const ALL: [Lang; 15] = [
        Lang::Ru,
        Lang::En,
        Lang::ZhHans,
        Lang::Hi,
        Lang::Es,
        Lang::Fr,
        Lang::Ar,
        Lang::PtBr,
        Lang::Id,
        Lang::De,
        Lang::Ja,
        Lang::Tr,
        Lang::Ko,
        Lang::Vi,
        Lang::Uk,
    ];

    /// BCP 47 code, as in `locales/<code>.json` and `settings.json`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
            Lang::ZhHans => "zh-Hans",
            Lang::Hi => "hi",
            Lang::Es => "es",
            Lang::Fr => "fr",
            Lang::Ar => "ar",
            Lang::PtBr => "pt-BR",
            Lang::Id => "id",
            Lang::De => "de",
            Lang::Ja => "ja",
            Lang::Tr => "tr",
            Lang::Ko => "ko",
            Lang::Vi => "vi",
            Lang::Uk => "uk",
        }
    }

    /// The language's own name, as the list shows it (`locales/languages.json`: each name is
    /// written in its own language, data rather than code).
    pub fn native(self) -> &'static str {
        static NAMES: OnceLock<serde_json::Map<String, serde_json::Value>> = OnceLock::new();
        NAMES
            .get_or_init(|| {
                serde_json::from_str(include_str!("../../locales/languages.json"))
                    .unwrap_or_default()
            })
            .get(self.code())
            .and_then(|v| v.as_str())
            .unwrap_or(self.code())
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL
            .into_iter()
            .find(|l| l.code().eq_ignore_ascii_case(code))
    }

    fn index(self) -> u8 {
        Lang::ALL.iter().position(|l| *l == self).unwrap_or(0) as u8
    }

    /// The catalog built into the app; English has none (it is the source).
    fn catalog_json(self) -> Option<&'static str> {
        Some(match self {
            Lang::En => return None,
            Lang::Ru => include_str!("../../locales/ru.json"),
            Lang::ZhHans => include_str!("../../locales/zh-Hans.json"),
            Lang::Hi => include_str!("../../locales/hi.json"),
            Lang::Es => include_str!("../../locales/es.json"),
            Lang::Fr => include_str!("../../locales/fr.json"),
            Lang::Ar => include_str!("../../locales/ar.json"),
            Lang::PtBr => include_str!("../../locales/pt-BR.json"),
            Lang::Id => include_str!("../../locales/id.json"),
            Lang::De => include_str!("../../locales/de.json"),
            Lang::Ja => include_str!("../../locales/ja.json"),
            Lang::Tr => include_str!("../../locales/tr.json"),
            Lang::Ko => include_str!("../../locales/ko.json"),
            Lang::Vi => include_str!("../../locales/vi.json"),
            Lang::Uk => include_str!("../../locales/uk.json"),
        })
    }
}

/// The interface language for a locale such as `ru-RU`, `zh-CN`, `pt_BR.UTF-8`: Chinese only
/// in its simplified form, Portuguese as Brazil's; English when the language is not one of the
/// fifteen.
pub fn for_locale(locale: &str) -> Lang {
    let locale = locale
        .split(['.', '@'])
        .next()
        .unwrap_or("")
        .replace('_', "-");
    let lower = locale.to_ascii_lowercase();
    let language = lower.split('-').next().unwrap_or("");
    match language {
        "zh" if lower.contains("hant") || lower.ends_with("-tw") || lower.ends_with("-hk") => {
            Lang::En
        }
        "zh" => Lang::ZhHans,
        "pt" => Lang::PtBr,
        other => Lang::ALL
            .into_iter()
            .find(|l| l.code().eq_ignore_ascii_case(other))
            .unwrap_or(Lang::En),
    }
}

/// The system's language (Windows: the user's locale; elsewhere `LC_ALL`, `LANG`), as the
/// interface starts before the person chose one.
pub fn system() -> Lang {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
        let mut buf = [0u16; 85];
        // SAFETY: the buffer is as long as the length passed; the call writes at most that.
        let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
        if len > 1 {
            return for_locale(&String::from_utf16_lossy(&buf[..len as usize - 1]));
        }
    }
    // An app started from the Finder gets no LANG: the system's region setting (`ru_RU`).
    #[cfg(target_os = "macos")]
    if let Some(locale) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .filter(|s| !s.trim().is_empty())
    {
        return for_locale(locale.trim());
    }
    ["LC_ALL", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .map_or(Lang::En, |l| for_locale(&l))
}

/// English (the source) until the person's or the system's language is set.
#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(1);

// One language per test thread: an app test (kittest, `--lang ru`) switches its own and no
// other test running beside it reads that one instead of English. The app draws on the thread
// that runs it, as egui_kittest does.
#[cfg(test)]
thread_local! {
    static CURRENT: AtomicU8 = const { AtomicU8::new(1) };
}

fn load() -> u8 {
    #[cfg(not(test))]
    return CURRENT.load(Ordering::Relaxed);
    #[cfg(test)]
    return CURRENT.with(|c| c.load(Ordering::Relaxed));
}

/// The language the interface is in now.
pub fn current() -> Lang {
    Lang::ALL[load() as usize % Lang::ALL.len()]
}

/// Whether the interface reads right to left (Arabic): the menu goes to the right and page
/// titles to the right edge.
pub fn rtl() -> bool {
    current() == Lang::Ar
}

/// Switches the interface language; screens follow on the next frame.
pub fn set(lang: Lang) {
    #[cfg(not(test))]
    CURRENT.store(lang.index(), Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.store(lang.index(), Ordering::Relaxed));
}

/// Where the catalogs come from: the app's own unless another program built on these modules
/// (the installer, with its own strings) set its source before the first translation.
static SOURCE: OnceLock<fn(Lang) -> Option<&'static str>> = OnceLock::new();

/// The catalogs of another program: `source` gives each language's JSON (none for English).
/// Takes effect only before the first translation; later calls are ignored.
#[allow(
    dead_code,
    reason = "only the installer (src/setup) calls it; the app has its own catalogs"
)]
pub fn use_catalogs(source: fn(Lang) -> Option<&'static str>) {
    let _ = SOURCE.set(source);
}

/// The catalogs, each read once on first use.
fn catalog(lang: Lang) -> Option<&'static RwLock<Catalog>> {
    static CATALOGS: OnceLock<Vec<Option<RwLock<Catalog>>>> = OnceLock::new();
    let all = CATALOGS.get_or_init(|| {
        let source = SOURCE.get().copied().unwrap_or(Lang::catalog_json);
        Lang::ALL
            .iter()
            .map(|l| {
                source(*l).map(|json| RwLock::new(Catalog::parse(*l, json).unwrap_or_default()))
            })
            .collect()
    });
    all[lang.index() as usize].as_ref()
}

/// `text` in the current language: its translation, or the English when there is none — and
/// any Arabic in it joined and in visual order, as egui draws left to right.
pub fn t(text: &str) -> Cow<'_, str> {
    let shown = match t_in(current(), text) {
        Cow::Borrowed(s) => visual(s),
        Cow::Owned(s) => Cow::Owned(visual(&s).into_owned()),
    };
    unmarked(shown)
}

/// `text` as [`t`] gives it, or, when `fits` says it is too long, cut at a word with "…": words
/// go from its end in reading order, before Arabic is put in visual order. Never cut text read
/// back from a laid-out row: its glyphs are in visual order, and epaint adds zero-width copies
/// of each right-to-left run, so such text turned again came out with letters doubled and
/// mixed (BUG-117).
pub fn t_fitted(text: &str, fits: impl Fn(&str) -> bool) -> String {
    let whole = t(text).into_owned();
    if fits(&whole) {
        return whole;
    }
    let said = t_in(current(), text).into_owned();
    let words: Vec<&str> = said.split(' ').collect();
    let cut = |n: usize| {
        let kept = words[..n].join(" ");
        unmarked(visual(&format!("{}…", kept.trim_end_matches([' ', '·'])))).into_owned()
    };
    (1..words.len())
        .rev()
        .map(cut)
        .find(|shown| fits(shown))
        .unwrap_or_else(|| cut(1))
}

/// `text` as [`t`] gives it, broken into lines that each satisfy `fits` (given a line as it is
/// drawn): words go into a line in reading order while it fits, and only then is each line put
/// in visual order. egui wraps a line after it is reordered, so a right-to-left paragraph
/// wrapped by egui put its end in the first line and its start in the last (BUG-119). A
/// person's words marked by [`own`] are kept on one line; a word wider than a line alone gets
/// a line of its own, for egui to break. Text that fits as it is comes back as [`t`] gives it.
pub fn t_wrapped(text: &str, fits: impl Fn(&str) -> bool) -> String {
    t_wrapped_rows(text, fits, usize::MAX)
}

/// [`t_wrapped`] in at most `rows` lines: when words are left over, the last line takes them and
/// is cut at a word with «…», as [`t_fitted`] cuts — so the cut drops the end of the text, never
/// its start (BUG-118: egui's own cut ended the row at its left, the start of an Arabic row).
pub fn t_wrapped_rows(text: &str, fits: impl Fn(&str) -> bool, rows: usize) -> String {
    if rows <= 1 {
        // One row is [`t_fitted`]'s cut, which also cuts inside a person's words (a note).
        return t_fitted(text, fits);
    }
    let whole = t(text).into_owned();
    if fits(&whole) {
        return whole;
    }
    let said = t_in(current(), text).into_owned();
    let shown = |line: &str| unmarked(visual(line)).into_owned();
    let mut lines = lines_in_reading_order(&said, |line| fits(&shown(line)));
    if lines.len() > rows.max(1) {
        let last = lines.split_off(rows.max(1) - 1).join(" ");
        let words: Vec<&str> = words_keeping_own(&last);
        let cut = |n: usize| {
            let kept = words[..n].join(" ");
            format!("{}…", kept.trim_end_matches([' ', '·']))
        };
        let fitted = (1..words.len())
            .rev()
            .map(cut)
            .find(|line| fits(&shown(line)))
            .unwrap_or_else(|| cut(1));
        lines.push(fitted);
    }
    shown(&lines.join("\n"))
}

/// `said` (in reading order) broken into lines at its spaces, each satisfying `fits`, its own
/// "\n" kept.
fn lines_in_reading_order(said: &str, fits: impl Fn(&str) -> bool) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for paragraph in said.split('\n') {
        let mut line: Option<String> = None;
        for word in words_keeping_own(paragraph) {
            let Some(so_far) = line.take() else {
                line = Some(word.to_owned());
                continue;
            };
            let longer = format!("{so_far} {word}");
            if fits(&longer) {
                line = Some(longer);
            } else {
                lines.push(so_far);
                line = Some(word.to_owned());
            }
        }
        lines.push(line.unwrap_or_default());
    }
    lines
}

/// `paragraph` cut at its spaces, except the spaces inside a person's words marked by [`own`]:
/// cut there, the isolate's start and end would land on different lines.
fn words_keeping_own(paragraph: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = 0;
    let mut inside = 0usize;
    for (i, c) in paragraph.char_indices() {
        match c {
            OWN_START => inside += 1,
            OWN_END => inside = inside.saturating_sub(1),
            ' ' if inside == 0 => {
                words.push(&paragraph[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    words.push(&paragraph[start..]);
    words
}

/// `text` in the current language for a screen reader: translated, in reading order.
pub fn spoken(text: &str) -> String {
    unmarked(t_in(current(), text)).into_owned()
}

const OWN_START: char = '\u{2068}';
const OWN_END: char = '\u{2069}';

/// A person's own words put into a sentence — a wallet's name, a label from the address book:
/// never translated, even when they happen to be a word of the interface ("Main"). The marks
/// (Unicode's first-strong isolate and its end) also keep the words' own direction inside
/// right-to-left text; [`t`] takes them off before drawing.
pub fn own(text: &str) -> String {
    format!("{OWN_START}{text}{OWN_END}")
}

/// Whether `text` is one person's words marked by [`own`] (a template's place may hold one).
fn is_own(text: &str) -> bool {
    text.starts_with(OWN_START) && text.ends_with(OWN_END)
}

fn unmarked(text: Cow<'_, str>) -> Cow<'_, str> {
    if text.contains([OWN_START, OWN_END]) {
        Cow::Owned(text.replace([OWN_START, OWN_END], ""))
    } else {
        text
    }
}

fn is_arabic(c: char) -> bool {
    matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}')
}

/// The mirror image of a paired bracket or quote (Unicode's Bidi_Mirroring_Glyph, the pairs
/// the interface's texts have).
fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '‹' => '›',
        '›' => '‹',
        other => other,
    }
}

/// A letter written right to left, before joining or after it (the presentation forms
/// [`visual`] puts on screen).
pub(crate) fn is_rtl_char(c: char) -> bool {
    is_arabic(c) || matches!(c, '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}

/// Each amount with its unit in `line` («0.0003 DASH», «1,250 mDASH», «30 000 duffs») as one
/// left-to-right isolate, as [`isolate_names`] does with @names: in right-to-left text the
/// number and the Latin unit stood as two runs, drawn «DASH 0.0003» (BUG-118, design review).
/// Amounts already inside an isolate are left as they are.
fn isolate_amounts(line: &str) -> Cow<'_, str> {
    const UNITS: [&str; 3] = ["DASH", "mDASH", "duffs"];
    if !UNITS.iter().any(|u| line.contains(u)) {
        return Cow::Borrowed(line);
    }
    let chars: Vec<char> = line.chars().collect();
    let separator = |c: char| matches!(c, '.' | ',' | '\u{a0}' | '\u{202f}');
    let mut out = String::with_capacity(line.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let fresh = i == 0 || !(chars[i - 1].is_ascii_digit() || separator(chars[i - 1]));
        let isolated = i > 0 && chars[i - 1] == OWN_START;
        if !(c.is_ascii_digit() && fresh && !isolated) {
            out.push(c);
            i += 1;
            continue;
        }
        // The number: digits, with separators between digits.
        let mut end = i + 1;
        while end < chars.len()
            && (chars[end].is_ascii_digit()
                || (separator(chars[end]) && chars.get(end + 1).is_some_and(char::is_ascii_digit)))
        {
            end += 1;
        }
        // Then a space and the unit, as a whole word.
        let unit = chars
            .get(end)
            .filter(|s| matches!(s, ' ' | '\u{a0}'))
            .and_then(|_| {
                UNITS.iter().find(|u| {
                    let after = end + 1 + u.chars().count();
                    chars
                        .get(end + 1..after)
                        .is_some_and(|w| w.iter().copied().eq(u.chars()))
                        && chars.get(after).is_none_or(|n| !n.is_alphanumeric())
                })
            });
        match unit {
            Some(u) => {
                let after = end + 1 + u.chars().count();
                out.push(OWN_START);
                out.extend(&chars[i..after]);
                out.push(OWN_END);
                i = after;
            }
            None => {
                out.extend(&chars[i..end]);
                i = end;
            }
        }
    }
    Cow::Owned(out)
}

/// Each `@name` in `line` (a DPNS name: Latin letters, digits, `-`, `_`, `.` inside) as one
/// left-to-right isolate, as [`own`] marks a person's words: in right-to-left text the bidi
/// algorithm would otherwise give the neutral `@` the paragraph's direction and draw
/// "alice@". Names already inside an isolate are left as they are. An `@` just before an
/// isolate — a template's «@{}» filled with [`own`] words — goes inside it, with the name it
/// belongs to: outside, it was as neutral as before (design review of BUG-119: «carol@»).
fn isolate_names(line: &str) -> Cow<'_, str> {
    if !line.contains('@') {
        return Cow::Borrowed(line);
    }
    let name_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let isolated = i > 0 && chars[i - 1] == OWN_START;
        if c == '@' && chars.get(i + 1) == Some(&OWN_START) {
            out.push(OWN_START);
            out.push('@');
            i += 2;
        } else if c == '@'
            && !isolated
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_alphanumeric())
        {
            let mut end = i + 1;
            while end < chars.len() && name_char(chars[end]) {
                end += 1;
            }
            // A full stop or a hyphen at the end belongs to the sentence, not the name.
            while matches!(chars[end - 1], '.' | '-' | '_') {
                end -= 1;
            }
            out.push(OWN_START);
            out.extend(&chars[i..end]);
            out.push(OWN_END);
            i = end;
        } else {
            out.push(c);
            i += 1;
        }
    }
    Cow::Owned(out)
}

/// Arabic as egui can draw it. egui does not reorder a line (no bidi), but it does shape: epaint
/// cuts a line into runs by the font each character comes from (`segment_into_runs`, by font
/// face, not by script) and gives each run to HarfBuzz, which joins the letters by the font's
/// own tables and, for an Arabic run, turns it around into visual order. So here each line is
/// reordered from logical to visual order by the Unicode bidi algorithm — an Arabic sentence
/// reads right to left with numbers, addresses and `@names` inside it still left to right —
/// and then every run of Arabic characters (the ones Noto Sans Arabic draws: Inter has none)
/// is put back in logical order, for HarfBuzz to turn once (BUG-117: turned twice, each word's
/// letters were drawn backwards). When egui does bidi itself, `the glyph order` tests in
/// `tests.rs` fail and this has to go. In the Arabic interface a line with Arabic is a
/// right-to-left paragraph even when it starts with a Latin word ("Platform في أرصدة DASH":
/// «Platform» rightmost); elsewhere (an Arabic name on an English screen) its first letter
/// decides. Text without Arabic is left as it is. Remembered, since a frame asks again.
pub fn visual(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_arabic) {
        return Cow::Borrowed(text);
    }
    let rtl = rtl();
    thread_local! {
        static SEEN: std::cell::RefCell<std::collections::HashMap<(bool, String), String>> =
            Default::default();
    }
    let key = (rtl, text.to_owned());
    if let Some(found) = SEEN.with(|s| s.borrow().get(&key).cloned()) {
        return Cow::Owned(found);
    }
    let level = rtl.then(unicode_bidi::Level::rtl);
    // Line by line: reordering a right-to-left paragraph whole would move its "\n" to the front.
    let out = text
        .split('\n')
        .map(|line| {
            let shaped = isolate_amounts(&isolate_names(line)).into_owned();
            let info = unicode_bidi::BidiInfo::new(&shaped, level);
            // A bracket or quote inside right-to-left text is drawn as its mirror image (bidi
            // rule L4), or "(Argon2)" would read ")Argon2("; mirrors are of one bidi class, so
            // the levels stay.
            let shaped: String = shaped
                .char_indices()
                .map(|(i, c)| {
                    if info.levels[i].is_rtl() {
                        mirror(c)
                    } else {
                        c
                    }
                })
                .collect();
            let info = unicode_bidi::BidiInfo::new(&shaped, level);
            info.paragraphs
                .iter()
                .map(|para| info.reorder_line(para, para.range.clone()).into_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    // The isolates have done their work in the reordering: none is drawn (egui has no glyph
    // for them), whoever draws the result.
    let out = arabic_runs_in_logical_order(&out.replace([OWN_START, OWN_END], ""));
    SEEN.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() > 5_000 {
            s.clear();
        }
        s.insert(key, out.clone());
    });
    Cow::Owned(out)
}

/// `shown` (in visual order) with each run of Arabic characters turned back to logical order:
/// such a run is one run of one font for epaint, and HarfBuzz turns it into visual order
/// itself. Bidi turned a right-to-left run character by character, marks included, so turning
/// it back character by character gives the logical order exactly.
fn arabic_runs_in_logical_order(shown: &str) -> String {
    let mut out = String::with_capacity(shown.len());
    let mut run: Vec<char> = Vec::new();
    for c in shown.chars() {
        if is_arabic(c) {
            run.push(c);
            continue;
        }
        out.extend(run.drain(..).rev());
        out.push(c);
    }
    out.extend(run.drain(..).rev());
    out
}

/// `text` in `lang`, before any reordering for display.
pub fn t_in(lang: Lang, text: &str) -> Cow<'_, str> {
    let Some(catalog) = catalog(lang) else {
        return Cow::Borrowed(text);
    };
    if text.is_empty() || !has_words(text) || is_own(text) {
        return Cow::Borrowed(text);
    }
    if let Ok(read) = catalog.read()
        && let Some(found) = read.exact(text)
    {
        return Cow::Owned(found.to_owned());
    }
    match catalog.write() {
        Ok(mut write) => write
            .translate(text)
            .map_or(Cow::Borrowed(text), Cow::Owned),
        Err(_) => Cow::Borrowed(text),
    }
}

/// Whether `text` has letters to translate (numbers, amounts and punctuation have none).
fn has_words(text: &str) -> bool {
    text.chars().any(char::is_alphabetic)
}

#[cfg(test)]
mod tests;

/// Between thousands and before decimals in `lang`: "1 234,5" in Russian, "1,234.5" in
/// English, "1.234,5" in German. Digits stay Western in every language (the wallet's amounts
/// are read side by side with addresses and explorers).
pub fn separators_in(lang: Lang) -> (char, char) {
    match lang {
        Lang::Ru | Lang::Uk | Lang::Fr => ('\u{a0}', ','),
        Lang::De | Lang::Es | Lang::PtBr | Lang::Id | Lang::Tr | Lang::Vi => ('.', ','),
        Lang::En | Lang::ZhHans | Lang::Ja | Lang::Ko | Lang::Hi | Lang::Ar => (',', '.'),
    }
}

/// What joins the items of a list in `lang` — a date and its time, word numbers «4–5, 11»:
/// Arabic writes its own comma «،» (BUG-118); every other language «, ».
pub fn list_comma(lang: Lang) -> &'static str {
    match lang {
        Lang::Ar => "، ",
        _ => ", ",
    }
}

/// [`separators_in`] the interface language.
pub fn separators() -> (char, char) {
    separators_in(current())
}

/// A date put together in `lang`'s order from the day, the month's name (already in `lang`)
/// and the year when it is shown: "14 October 2026", "October 14, 2026", "2026年10月14日".
pub fn date_in(lang: Lang, day: u32, month: &str, year: Option<i32>) -> String {
    match (lang, year) {
        (Lang::En, Some(y)) => format!("{month} {day}, {y}"),
        (Lang::En, None) => format!("{month} {day}"),
        (Lang::De, Some(y)) => format!("{day}. {month} {y}"),
        (Lang::De, None) => format!("{day}. {month}"),
        (Lang::Es | Lang::PtBr, Some(y)) => format!("{day} de {month} de {y}"),
        (Lang::ZhHans | Lang::Ja, Some(y)) => format!("{y}年{month}{day}日"),
        (Lang::ZhHans | Lang::Ja, None) => format!("{month}{day}日"),
        (Lang::Ko, Some(y)) => format!("{y}년 {month} {day}일"),
        (Lang::Ko, None) => format!("{month} {day}일"),
        (Lang::Vi, Some(y)) => format!("{day} {month}, {y}"),
        (_, Some(y)) => format!("{day} {month} {y}"),
        (_, None) => format!("{day} {month}"),
    }
}
