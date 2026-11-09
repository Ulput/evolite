//! What a payment note may hold (FEAT-0003, AC-T2): the same rules for the form that takes it
//! and for the core that keeps it. A note is one line of 1–140 extended grapheme clusters (what
//! a person sees as one character: an emoji with its modifiers, an Arabic letter with its
//! marks) and at most 1024 bytes of UTF-8. Control characters and the characters that override
//! the text's direction are taken out, so a note cannot make a history line or a file read
//! other than it is.

use evolite_api::{NOTE_MAX_BYTES, NOTE_MAX_GRAPHEMES};
use unicode_segmentation::UnicodeSegmentation;

/// Embeddings, overrides and isolates (U+202A–U+202E, U+2066–U+2069): they could turn the rest
/// of a line around. The text is isolated by whoever draws it instead.
fn turns_direction(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// `text` as a note keeps it: a line break or a tab becomes a space, other control characters
/// (C0, DEL, C1) and direction overrides go. Not trimmed: the form keeps what is being typed.
pub fn clean(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() || turns_direction(c) => None,
            c => Some(c),
        })
        .collect()
}

/// How many characters (extended grapheme clusters) `text` has.
pub fn graphemes(text: &str) -> usize {
    text.graphemes(true).count()
}

/// `text` cleaned and cut to what a note holds: [`NOTE_MAX_GRAPHEMES`] characters and
/// [`NOTE_MAX_BYTES`] bytes, never inside a character. The flag says whether anything was cut.
pub fn fit(text: &str) -> (String, bool) {
    let clean = clean(text);
    let mut end = 0;
    for (count, (at, g)) in clean.grapheme_indices(true).enumerate() {
        if count == NOTE_MAX_GRAPHEMES || at + g.len() > NOTE_MAX_BYTES {
            return (clean[..end].to_owned(), true);
        }
        end = at + g.len();
    }
    (clean, false)
}

/// The core's refusals, for "Details" (no text of the note in them, nor its length).
pub const TOO_LONG: &str = tr!("The note is too long");
pub const TOO_MANY: &str = tr!("This wallet keeps no more notes: delete some first");

/// Why a note cannot be kept as given.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    TooLong,
}

/// The note to keep for `text`: cleaned and trimmed; `None` for an empty one (no note). Too
/// long — more than a note holds — is refused, never cut here: the form cuts and says so.
pub fn kept(text: &str) -> Result<Option<String>, Refused> {
    let clean = clean(text);
    let text = clean.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.len() > NOTE_MAX_BYTES || graphemes(text) > NOTE_MAX_GRAPHEMES {
        return Err(Refused::TooLong);
    }
    Ok(Some(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_counts_what_a_person_sees_as_characters() {
        // A family emoji (7 code points), a flag (2), an Arabic letter with its mark (2).
        assert_eq!(graphemes("👨‍👩‍👧‍👦🇺🇦بُ"), 3);
        assert_eq!(graphemes("За дизайн"), 9);
    }

    #[test]
    fn direction_overrides_and_control_characters_are_taken_out() {
        assert_eq!(clean("a\u{202E}b\u{2066}c\u{2069}d"), "abcd");
        assert_eq!(
            clean("line\none\tand\r\u{7}\u{85}\u{9F}x"),
            "line one and x"
        );
        // Marks that only hint the direction stay: they reorder nothing on their own.
        assert_eq!(clean("@alice\u{200E}"), "@alice\u{200E}");
    }

    #[test]
    fn a_note_is_cut_at_140_characters_and_1024_bytes() {
        let long = "ж".repeat(141);
        let (cut, was) = fit(&long);
        assert!(was);
        assert_eq!(graphemes(&cut), 140);
        assert_eq!(fit("short"), ("short".to_owned(), false));
        // 140 emoji of 4 bytes are 560 bytes: within both limits.
        let emoji = "🙂".repeat(140);
        assert_eq!(fit(&emoji), (emoji.clone(), false));
        // A family emoji is 25 bytes: the bytes run out first, never inside one.
        let families = "👨‍👩‍👧‍👦".repeat(60);
        let (cut, was) = fit(&families);
        assert!(was && cut.len() <= NOTE_MAX_BYTES);
        assert_eq!(cut.len() % "👨‍👩‍👧‍👦".len(), 0);
    }

    #[test]
    fn the_core_refuses_what_is_too_long_and_takes_empty_for_no_note() {
        assert_eq!(kept("  \n "), Ok(None));
        assert_eq!(kept(" За дизайн \u{202E}"), Ok(Some("За дизайн".into())));
        assert_eq!(kept(&"x".repeat(141)), Err(Refused::TooLong));
        assert_eq!(kept(&"x".repeat(140)), Ok(Some("x".repeat(140))));
        assert_eq!(kept(&"👨‍👩‍👧‍👦".repeat(41)), Err(Refused::TooLong));
    }
}
