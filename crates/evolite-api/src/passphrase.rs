//! FEAT-0041: the BIP-39 passphrase («the 25th word») as BIP-39 salts it, and what the field
//! says about it. One place for the core (which refuses) and the screens (which warn and keep
//! «Continue» off), so both count the same bytes.
//!
//! - R-8: taken as typed — no trimming, no change of case; only NFKD, as BIP-39 says.
//! - R-10: at most [`PASSPHRASE_BYTES`] bytes of UTF-8 **after NFKD**, never cut: a cut
//!   passphrase is another key and another, empty wallet.

use std::fmt;

use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

/// The most bytes a passphrase takes after NFKD (R-10): 256 Latin letters, 128 Cyrillic.
pub const PASSPHRASE_BYTES: usize = 256;

/// Over this many characters Trezor does not take it (R-10's note, not a refusal).
pub const TREZOR_CHARS: usize = 50;

/// A BIP-39 passphrase on its way between the screens and the core. Wiped on drop; `Debug`
/// says nothing of it, not even whether it is empty (R-14, R-15).
#[derive(Clone, PartialEq, Default)]
pub struct Passphrase(Zeroizing<String>);

impl Passphrase {
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// As typed.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(hidden)")
    }
}

/// `text` as BIP-39 salts it: NFKD. What the vault keeps and what is compared.
pub fn normalized(text: &str) -> Zeroizing<String> {
    Zeroizing::new(text.nfkd().collect())
}

/// Its length as BIP-39 counts it: bytes of UTF-8 after NFKD.
pub fn bytes(text: &str) -> usize {
    text.nfkd().map(char::len_utf8).sum()
}

/// R-10: within [`PASSPHRASE_BYTES`].
pub fn fits(text: &str) -> bool {
    bytes(text) <= PASSPHRASE_BYTES
}

/// What the field says under a passphrase (R-8, R-9, R-10); none stops «Continue» but
/// [`Note::TooLong`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    /// R-10: over [`PASSPHRASE_BYTES`] — refused.
    TooLong,
    /// R-8: a space at either end or two in a row.
    Spaces,
    /// R-9: letters outside ASCII.
    NotLatin,
    /// R-10: longer than Trezor takes.
    Trezor,
}

/// The notes for `text`, most serious first; none for an empty one.
pub fn notes(text: &str) -> Vec<Note> {
    let mut notes = Vec::new();
    if text.is_empty() {
        return notes;
    }
    if !fits(text) {
        notes.push(Note::TooLong);
    }
    if text.starts_with(char::is_whitespace)
        || text.ends_with(char::is_whitespace)
        || text.contains("  ")
    {
        notes.push(Note::Spaces);
    }
    if text.chars().any(|c| c.is_alphabetic() && !c.is_ascii()) {
        notes.push(Note::NotLatin);
    }
    if fits(text) && text.chars().count() > TREZOR_CHARS {
        notes.push(Note::Trezor);
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-A15: 256 Latin letters fit, 257 do not; 129 Cyrillic letters do not (2 bytes each);
    /// 4-byte characters are counted in bytes, not characters.
    #[test]
    fn the_limit_is_bytes_after_nfkd() {
        assert!(fits(&"a".repeat(256)));
        assert!(!fits(&"a".repeat(257)));
        assert!(fits(&"ж".repeat(128)));
        assert!(!fits(&"ж".repeat(129)));
        assert!(fits(&"😀".repeat(64)));
        assert!(!fits(&"😀".repeat(65)));
        // «é» as one character is two after NFKD: 3 bytes, not 2.
        assert_eq!(bytes("\u{e9}"), 3);
        assert_eq!(bytes("e\u{301}"), 3);
        assert_eq!(notes(&"a".repeat(257))[0], Note::TooLong);
    }

    /// AC-A10: «é» typed as one character and as two is one passphrase.
    #[test]
    fn nfkd_makes_one_of_two_spellings() {
        assert_eq!(*normalized("caf\u{e9}"), *normalized("cafe\u{301}"));
        assert_eq!(*normalized(" abc"), " abc", "nothing is trimmed (R-8)");
    }

    #[test]
    fn the_field_notes() {
        assert!(notes("").is_empty());
        assert!(notes("correct horse").is_empty());
        assert_eq!(notes(" abc"), vec![Note::Spaces]);
        assert_eq!(notes("abc "), vec![Note::Spaces]);
        assert_eq!(notes("a  b"), vec![Note::Spaces]);
        assert_eq!(notes("пароль"), vec![Note::NotLatin]);
        assert_eq!(notes(&"a".repeat(51)), vec![Note::Trezor]);
        assert!(notes(&"a".repeat(50)).is_empty());
    }

    #[test]
    fn debug_shows_nothing() {
        assert_eq!(
            format!("{:?}", Passphrase::new("TREZOR".into())),
            "Passphrase(hidden)"
        );
        assert_eq!(format!("{:?}", Passphrase::default()), "Passphrase(hidden)");
    }
}
