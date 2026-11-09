//! BIP39 English wordlist: suggestions while typing and the checksum of a whole phrase.
//! Only checks words; deriving keys is the wallet core's job and is not in the prototype.

use sha2::{Digest, Sha256};

const WORDLIST: &str = include_str!("../assets/bip39-english.txt");

pub fn words() -> impl Iterator<Item = &'static str> {
    WORDLIST.lines().map(str::trim).filter(|w| !w.is_empty())
}

pub fn index(word: &str) -> Option<usize> {
    words().position(|w| w == word)
}

/// Up to `limit` words that start with `prefix` (lowercase).
pub fn suggest(prefix: &str, limit: usize) -> Vec<&'static str> {
    if prefix.is_empty() {
        return Vec::new();
    }
    words()
        .filter(|w| w.starts_with(prefix))
        .take(limit)
        .collect()
}

#[derive(Debug, PartialEq)]
pub enum PhraseError {
    /// Not 12 or 24 words.
    Length,
    /// A word that is not in the list (its position, from 0).
    UnknownWord(usize),
    /// All words are known, but the last bits do not match.
    Checksum,
}

/// Checks a 12- or 24-word phrase: every word in the list and the checksum (BIP39).
pub fn check(phrase: &[&str]) -> Result<(), PhraseError> {
    if phrase.len() != 12 && phrase.len() != 24 {
        return Err(PhraseError::Length);
    }
    let mut bits = Vec::with_capacity(phrase.len() * 11);
    for (i, word) in phrase.iter().enumerate() {
        let n = index(word).ok_or(PhraseError::UnknownWord(i))?;
        bits.extend((0..11).rev().map(|b| (n >> b) & 1 == 1));
    }
    let entropy_bits = bits.len() * 32 / 33;
    let entropy: Vec<u8> = bits[..entropy_bits]
        .chunks(8)
        .map(|byte| byte.iter().fold(0u8, |acc, &b| (acc << 1) | b as u8))
        .collect();
    let hash = Sha256::digest(&entropy);
    let checksum = &bits[entropy_bits..];
    let ok = checksum
        .iter()
        .enumerate()
        .all(|(i, &b)| ((hash[i / 8] >> (7 - i % 8)) & 1 == 1) == b);
    if ok {
        Ok(())
    } else {
        Err(PhraseError::Checksum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wordlist_is_bip39_english() {
        assert_eq!(words().count(), 2048);
        assert_eq!(index("abandon"), Some(0));
        assert_eq!(index("zoo"), Some(2047));
    }

    #[test]
    fn accepts_known_vectors() {
        // Test vectors from the BIP39 reference (Trezor).
        let mut twelve = vec!["abandon"; 11];
        twelve.push("about");
        assert_eq!(check(&twelve), Ok(()));
        let mut twenty_four = vec!["abandon"; 23];
        twenty_four.push("art");
        assert_eq!(check(&twenty_four), Ok(()));
        let zoo: Vec<&str> = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong"
            .split(' ')
            .collect();
        assert_eq!(check(&zoo), Ok(()));
    }

    #[test]
    fn rejects_bad_phrases() {
        assert_eq!(check(&["abandon"; 12]), Err(PhraseError::Checksum));
        assert_eq!(check(&["abandon"; 11]), Err(PhraseError::Length));
        let mut typo = vec!["abandon"; 11];
        typo.push("abuot");
        assert_eq!(check(&typo), Err(PhraseError::UnknownWord(11)));
    }

    #[test]
    fn suggests_by_prefix() {
        assert_eq!(suggest("fro", 3), vec!["frog", "front", "frost"]);
        assert!(suggest("", 3).is_empty());
    }
}
