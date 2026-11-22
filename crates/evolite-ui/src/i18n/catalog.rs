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
        // A whole sentence the screens ended with a full stop goes before a template that only
        // half fits: "{}: {}." would keep a core reason with a colon in it English (BUG-35).
        let found = self
            .by_template(text, false)
            .or_else(|| self.with_full_stop(text))
            .or_else(|| self.by_template(text, true));
        if self.seen.len() >= SEEN_MAX {
            self.seen.clear();
        }
        self.seen.insert(text.to_owned(), found.clone());
        found
    }

    /// A sentence the screens finished with a full stop after the key ("{reason}."): the key's
    /// translation, then the stop — unless the translation ends with its own.
    fn with_full_stop(&mut self, text: &str) -> Option<String> {
        let bare = text.strip_suffix('.')?;
        if bare.is_empty() || bare.ends_with('.') {
            return None;
        }
        let mut found = self.translate(bare)?;
        if !found.ends_with(['.', '。', '।', '!', '?']) {
            // The language's own full stop, as its catalog writes it.
            found.push(match self.lang {
                Some(Lang::ZhHans | Lang::Ja) => '。',
                Some(Lang::Hi) => '।',
                _ => '.',
            });
        }
        Some(found)
    }

    /// `loose`: a template whose places keep a sentence untranslated will do too.
    fn by_template(&mut self, text: &str, loose: bool) -> Option<String> {
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
                        // BUG-270 r1: a place that runs over a « · » took a piece's words with
                        // it — «Received{}» and «Received privately · 1.2 DASH» — and the pieces
                        // know better («Получено приватно · 1.2 DASH»), so it is not whole.
                        whole &= c.split_whitespace().count() <= MAX_UNTRANSLATED_WORDS
                            && !c.contains(" · ");
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
                .or(fallback.filter(|_| loose))
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
            // The first place the next text follows after a non-empty gap. A gap that opens
            // with words kept whole (`own`, FSI … PDI) takes them whole: a date «29. November»
            // never ends at its own «. » (BUG-250, B1). Marks are not nested: `own` of a text
            // already marked ends at the first PDI, which is still past the inner text.
            let skip = match at.strip_prefix('\u{2068}').and_then(|r| r.find('\u{2069}')) {
                Some(close) => '\u{2068}'.len_utf8() + close + '\u{2069}'.len_utf8(),
                None => at.chars().next()?.len_utf8(),
            };
            skip + at[skip..].find(part.as_str())?
        };
        let captured = &at[..end];
        if captured.is_empty() {
            return None;
        }
        // BUG-270: a place never cuts a word. «Received{}» (a notification's «Received» with its
        // amount after it) took «Received privately · …» as «Received» + «privately · …», and a
        // screen reader heard «Пришло privately…». A letter on both sides of a place's edge is one
        // word, not a template's text and its place.
        let before = if i == 0 {
            first.as_str()
        } else {
            rest[i - 1].as_str()
        };
        let letter = |c: Option<char>| c.is_some_and(char::is_alphabetic);
        if letter(before.chars().next_back()) && letter(captured.chars().next())
            || letter(part.chars().next()) && letter(captured.chars().next_back())
        {
            return None;
        }
        captures.push(captured.to_owned());
        at = &at[end + part.len()..];
    }
    Some(captures)
}

/// Text ending with one point after a letter: an abbreviation such as «Min.».
fn abbreviated(text: &str) -> bool {
    let mut back = text.chars().rev();
    back.next() == Some('.') && back.next().is_some_and(char::is_alphabetic)
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
                let put_in = captures.get(index).map_or("", String::as_str);
                out.push_str(put_in);
                rest = &after[close + 1..];
                // An abbreviation's point ends the sentence too: «… die nächste Pause 1 Min.»,
                // not «1 Min..» (BUG-70). An ellipsis on either side stays.
                if abbreviated(put_in) && rest.starts_with('.') && !rest.starts_with("..") {
                    rest = &rest[1..];
                }
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

#[cfg(test)]
mod fill_tests {
    use super::*;

    fn parts(form: &str) -> Vec<String> {
        form.split("{}").map(str::to_owned).collect()
    }

    /// BUG-270 r1 (the architect's Should): a template's place never cuts a word — «Received{}»
    /// (a notification's «Received» and its amount) does not take «Receivedprivately», and
    /// still takes the amount it was made for; the other edge of a place the same.
    #[test]
    fn a_place_never_cuts_a_word() {
        let received = parts("Received{}");
        // A space is an edge: `fill` takes «privately · …»; the catalog then prefers the
        // pieces (the next test).
        assert_eq!(
            fill(&received, "Received privately · 1.2 DASH"),
            Some(vec![" privately · 1.2 DASH".to_owned()])
        );
        assert_eq!(fill(&received, "Receivedprivately"), None);
        assert_eq!(
            fill(&received, "Received 1.2 DASH"),
            Some(vec![" 1.2 DASH".to_owned()])
        );
        assert_eq!(
            fill(&received, "Received: 1.2 DASH"),
            Some(vec![": 1.2 DASH".to_owned()])
        );
        // The place's other edge: a digit before a letter is not one word.
        assert_eq!(fill(&parts("{}s"), "5s"), Some(vec!["5".to_owned()]));
        assert_eq!(fill(&parts("{}ing"), "Sending"), None);
        assert_eq!(
            fill(&parts("@{} · credits"), "@satoshi · credits"),
            Some(vec!["satoshi".to_owned()])
        );
    }

    /// The same through a catalog: the whole key wins, the template keeps its amount.
    #[test]
    fn received_privately_is_its_own_key_not_received_and_a_word() {
        let mut ru = Catalog::parse(
            crate::i18n::Lang::Ru,
            r#"{"Received{}": "Пришло{}", "Received privately": "Получено приватно"}"#,
        )
        .expect("a catalog");
        assert_eq!(
            ru.translate("Received privately").as_deref(),
            Some("Получено приватно")
        );
        assert_eq!(
            ru.translate("Received 1.2 DASH").as_deref(),
            Some("Пришло 1.2 DASH")
        );
        // QA's line whole: the pieces, not «Пришло privately · 1.2 DASH».
        assert_eq!(
            ru.translate("Received privately · 1.2 DASH").as_deref(),
            Some("Получено приватно · 1.2 DASH")
        );
        assert_eq!(
            ru.translate("Receivedprivately"),
            None,
            "no place starts inside a word"
        );
    }
}
