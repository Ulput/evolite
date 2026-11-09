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
pub mod layers;
pub mod network;
pub mod note_text;
pub mod phrase_check;

use std::time::{SystemTime, UNIX_EPOCH};

/// A spend without the password while "password for every send" is on (the screen did not
/// ask): the same refusal from the demo and the real core.
pub const PASSWORD_NEEDED: &str =
    tr!("This wallet asks for the password for every send — enter it and try again");

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
