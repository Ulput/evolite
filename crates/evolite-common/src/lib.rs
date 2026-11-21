//! What every part of the evolite core shares and that needs no Platform stack: the network this
//! process runs on, the diagnostic journal, the rules of names, payment notes and recovery
//! phrases, which balances a line of history moves.

#![forbid(unsafe_code)]

/// Marks interface text (a reason the screens show): the literal as it is, in English, the key
/// of the catalogs in `evolite-ui/locales`, which `extract.pl` collects from these marks.
#[macro_export]
macro_rules! tr {
    ($text:literal) => {
        $text
    };
}

pub mod attempts;
pub mod bip39;
pub mod diag;
pub mod dpns;
pub mod key_refusal;
pub mod layers;
pub mod lock_words;
pub mod network;
pub mod note_text;
pub mod phrase_check;

use std::time::{SystemTime, UNIX_EPOCH};

/// A spend without the password while "password for every send" is on (the screen did not
/// ask): the same refusal from the demo and the real core.
pub const PASSWORD_NEEDED: &str =
    tr!("This wallet asks for the password for every send — enter it and try again");

/// What only the advanced features would ask, asked while they are off (FEAT-0040 «hidden is
/// off»): the same refusal from the demo and the real core.
pub const ADVANCED_OFF: &str = tr!("The features for advanced users are off");

/// FEAT-0041 R-10: a BIP-39 passphrase over 256 bytes after NFKD — refused, never cut.
pub const PASSPHRASE_TOO_LONG: &str = tr!("The BIP-39 passphrase is too long");

/// FEAT-0041 R-12: this phrase with this BIP-39 passphrase is a wallet of this computer already.
pub const SAME_WALLET: &str = tr!("This wallet is already on this computer");

/// The wall clock in Unix seconds, as the interface shows it.
pub fn now() -> evolite_api::Timestamp {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as evolite_api::Timestamp)
}

/// The wall clock in Unix seconds, as the wrong-password count keeps it (FEAT-0002).
pub fn wall_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
