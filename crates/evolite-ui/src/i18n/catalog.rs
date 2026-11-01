//! One language's translations: exact strings, templates with `{}` and words in all their
//! plural forms (see the module doc). Built from the language's JSON once; what a template
//! translated is remembered, so a frame does the work only for text it has not seen.

use super::{Lang, plural};
use std::collections::HashMap;

/// What a template turns into.
#[derive(Clone, Debug)]
enum Out {
    Text(String),
    /// By CLDR category name; `other` (or the last given) when the category is missing.
    Plural(Vec<(String, String)>),
}

#[derive(Clone, Debug)]
struct Template {
    /// The English text around the `{}`: one part more than there are places.
    parts: Vec<String>,
    out: Out,
}

#[derive(Default)]
pub struct Catalog {
    lang: Option<Lang>,
    exact: HashMap<String, String>,
    /// Longest literal text first: the most specific template wins.
    templates: Vec<Template>,
    seen: HashMap<String, Option<String>>,
}

/// Remembered translations at most; the cache starts again past it (live text such as
/// counters and times would grow it without end).
const SEEN_MAX: usize = 20_000;

/// Words a place may hold untranslated: a name or a label ("Anna", "workshop rent").
/// Longer untranslated text in a place means the template matched by accident.
const MAX_UNTRANSLATED_WORDS: usize = 3;

impl Catalog {
    pub fn parse(lang: Lang, json: &str) -> Result<Catalog, String> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let object = value.as_object().ok_or("a catalog is a JSON object")?;
        let mut catalog = Catalog {
            lang: Some(lang),
            ..Catalog::default()
        };
        for (key, value) in object {
            if key.starts_with('_') {
                continue; // `_comment` and the like
            }
            let out = match value {
                serde_json::Value::String(s) => Out::Text(s.clone()),
                serde_json::Value::Object(forms) => Out::Plural(
                    forms
                        .iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                        .collect(),
                ),
                _ => continue,
            };
            let is_template = key.contains("{}") || key.contains('|');
            match (&out, is_template) {
                (Out::Text(text), false) => {
                    catalog.exact.insert(key.clone(), text.clone());
                }
                _ => {
                    for form in key.split('|') {
                        catalog.templates.push(Template {
                            parts: form.split("{}").map(str::to_owned).collect(),
                            out: out.clone(),
                        });
                    }
                }
            }
        }
        catalog
            .templates
            .sort_by_key(|t| std::cmp::Reverse(t.parts.iter().map(String::len).sum::<usize>()));
        Ok(catalog)
    }

    /// The translation of `text` as it is, when there is one.
    pub fn exact(&self, text: &str) -> Option<&str> {
        self.exact
            .get(text)
            .map(String::as_str)
            .or_else(|| self.seen.get(text).and_then(|s| s.as_deref()))
    }

    /// The translation of `text` by its exact form or a template; `None`: there is none.
    pub fn translate(&mut self, text: &str) -> Option<String> {
        if let Some(found) = self.exact.get(text) {
            return Some(found.clone());
        }
        if let Some(seen) = self.seen.get(text) {
            return seen.clone();
        }
        let found = self.by_template(text);
        if self.seen.len() >= SEEN_MAX {
            self.seen.clear();
        }
        self.seen.insert(text.to_owned(), found.clone());
        found
    }

    fn by_template(&mut self, text: &str) -> Option<String> {
        let lang = self.lang?;
        // The most specific template whose places hold no untranslated sentence: a name or a
        // label may stay as it is, but text that only looks like "{} of {}" is left to the next
        // template or to its pieces. Words from outside (a library's error, an address) have no
        // translation at all: when no template takes the text otherwise, the first one that
        // fits keeps them as they are.
        let mut found = None;
        let mut fallback = None;
        for i in 0..self.templates.len() {
            let Some(captures) = fill(&self.templates[i].parts, text) else {
                continue;
            };
            let out = self.templates[i].out.clone();
            // A plural form counts something: "new address" is not "{} address".
            if matches!(out, Out::Plural(_)) && !captures.iter().any(|c| number(c).is_some()) {
                continue;
            }
            // What stood in for `{}` may be words too (a reason inside a sentence).
            let mut translated = Vec::with_capacity(captures.len());
            let mut whole = true;
            for c in captures {
                // Numbers, and a person's own words (`own`), stay as they are.
                if !super::has_words(&c) || super::is_own(&c) {
                    translated.push(c);
                    continue;
                }
                match self.translate(&c) {
                    Some(t) => translated.push(t),
                    None => {
                        whole &= c.split_whitespace().count() <= MAX_UNTRANSLATED_WORDS;
                        translated.push(c);
                    }
                }
            }
            if whole {
                found = Some((out, translated));
                break;
            }
            if fallback.is_none() {
                fallback = Some((out, translated));
            }
        }
        let (out, captures) = found.or_else(|| {
            self.by_pieces(text)
                .map(|t| (Out::Text("{}".into()), vec![t]))
                .or(fallback)
        })?;
        let pattern = match out {
            Out::Text(text) => text,
            Out::Plural(forms) => {
                let n = captures.iter().find_map(|c| number(c)).unwrap_or(0);
                let want = plural::category(lang, n).name();
                forms
                    .iter()
                    .find(|(k, _)| k == want)
                    .or_else(|| forms.iter().find(|(k, _)| k == "other"))
                    .or(forms.last())
                    .map(|(_, v)| v.clone())?
            }
        };
        Some(put(&pattern, &captures))
    }

    /// Text put together from pieces with " · " (a name, a status, a time): each piece on its
    /// own; a piece without a translation (a name, an address) stays as it is. `None` when no
    /// piece was translated.
    fn by_pieces(&mut self, text: &str) -> Option<String> {
        if !text.contains(" · ") {
            return None;
        }
        let mut out = Vec::new();
        let mut any = false;
        for piece in text.split(" · ") {
            match (super::has_words(piece) && !super::is_own(piece))
                .then(|| self.translate(piece))
                .flatten()
            {
                Some(t) => {
                    any = true;
                    out.push(t);
                }
                None => out.push(piece.to_owned()),
            }
        }
        any.then(|| out.join(" · "))
    }
}

/// The text standing in for each `{}` when `text` is `parts` with the gaps filled; `None`:
/// it is not this template. Each gap takes the shortest text that lets the rest follow.
fn fill(parts: &[String], text: &str) -> Option<Vec<String>> {
    let (first, rest) = parts.split_first()?;
    if rest.is_empty() {
        return None; // not a template
    }
    let mut at = text.strip_prefix(first.as_str())?;
    let mut captures = Vec::with_capacity(rest.len());
    for (i, part) in rest.iter().enumerate() {
        let last = i + 1 == rest.len();
        let end = if last {
            if !at.ends_with(part.as_str()) || at.len() < part.len() {
                return None;
            }
            at.len() - part.len()
        } else if part.is_empty() {
            return None; // two gaps side by side cannot be told apart
        } else {
            // The first place the next text follows after a non-empty gap.
            let skip = at.chars().next()?.len_utf8();
            skip + at[skip..].find(part.as_str())?
        };
        let captured = &at[..end];
        if captured.is_empty() {
            return None;
        }
        captures.push(captured.to_owned());
        at = &at[end + part.len()..];
    }
    Some(captures)
}

/// `pattern` with its places filled: `{}` in order, or `{0}`, `{1}`… by number (a language
/// may need another order of words).
fn put(pattern: &str, captures: &[String]) -> String {
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut next = 0;
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) if after[..close].chars().all(|c| c.is_ascii_digit()) => {
                let index = if close == 0 {
                    next += 1;
                    next - 1
                } else {
                    after[..close].parse().unwrap_or(usize::MAX)
                };
                out.push_str(captures.get(index).map_or("", String::as_str));
                rest = &after[close + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A whole number written in `text`, for choosing a plural form. Counts are written with the
/// interface language's thousands separator (a space, a comma or a point), so all of them
/// may stand between digit groups.
fn number(text: &str) -> Option<u64> {
    let digits: String = text
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{a0}' | '\u{202f}' | ',' | '.' | '\''))
        .collect();
    digits.parse().ok()
}
