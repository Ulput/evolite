//! The backup file (FEAT-0045): what the phrase does not bring back — notes, the address book,
//! contacts' names, hidden requests, signed-in apps, the wallet's settings — in one file the
//! person keeps. No phrase, no key, no password in it (R-1, AC-A2).
//!
//! - The file: JSON `{"format":"evolite/backup","version":1,"network":"testnet",
//!   "wallet":<hex, 8 bytes>,"nonce":<hex, 12 bytes>,"ciphertext":<hex>}`. The open part says
//!   what the file is, for which network and for which wallet, before anything is decrypted:
//!   another wallet's file or another network's is refused by name (AC-T-B1).
//! - `wallet` is the wallet's fingerprint, not its id: `HKDF-SHA256(seed, salt =
//!   "evolite/backup", info = "id/v1")[..8]`. It tells only that two files are of one wallet,
//!   and only to whoever holds both.
//! - The key: `HKDF-SHA256(seed, salt = "evolite/backup", info = "key/v1")`, AES-256-GCM with a
//!   fresh nonce, the open part as the associated data. From the seed, so only the same
//!   phrase (with the same BIP-39 password) opens it; no password of its own (R-2, R-8).
//! - Inside: [`Contents`], one section per kind of record, each merged by its own rule
//!   ([`Contents::merge_into`] and the stores' own merges, R-9).
//!
//! Nothing here goes to the journal: the core says only that a copy was saved or restored.

use crate::address_book::Entry;
use crate::connected_apps::App;
use crate::contact_names::Kept;
use crate::notes::Note;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

pub const FORMAT: &str = "evolite/backup";
/// The version this build writes and the newest it reads (R-11).
pub const VERSION: u32 = 1;
pub use evolite_api::backup::EXTENSION;
/// The largest backup file read (10 000 notes and the rest fit well under it).
pub const MAX_FILE: u64 = 64 << 20;

const SALT: &[u8] = b"evolite/backup";

/// A wallet's backup key and fingerprint, from its seed: derived at the unlock, kept in memory
/// while it is open (the key zeroed when dropped), never written.
pub struct BackupKey {
    key: Zeroizing<[u8; 32]>,
    fingerprint: [u8; 8],
}

impl BackupKey {
    pub fn of(seed: &[u8]) -> Self {
        let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(SALT), seed);
        let mut key = Zeroizing::new([0u8; 32]);
        hk.expand(b"key/v1", key.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 length");
        let mut fingerprint = [0u8; 8];
        hk.expand(b"id/v1", &mut fingerprint)
            .expect("8 bytes is a valid HKDF-SHA256 length");
        Self { key, fingerprint }
    }

    /// The fingerprint as the file's open part has it.
    pub fn fingerprint(&self) -> String {
        hex(&self.fingerprint)
    }
}

/// What the file holds, one section per kind of record (AC-T-B2). A section this build does not
/// know (a newer one's, of the same version) is skipped.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Contents {
    /// `notes/v1`, the deletions' marks too (FEAT-0024 shares the record).
    #[serde(default)]
    pub notes: BTreeMap<String, Note>,
    #[serde(default)]
    pub address_book: Vec<Entry>,
    /// Contacts' names as last read, by identity.
    #[serde(default)]
    pub contact_names: BTreeMap<String, Kept>,
    /// Senders whose requests were hidden.
    #[serde(default)]
    pub hidden_requests: BTreeSet<String>,
    #[serde(default)]
    pub connected_apps: Vec<App>,
    /// The wallet's half of the app's settings, as the interface keeps it: the core does not
    /// read it, it carries it (the architect, answer 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
    /// Reserved for the NFT decisions (FEAT-0031 R-10), once they have a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nft: Option<serde_json::Value>,
    /// The journal's finished lines: exact Platform fees, credit transfers, contact requests…
    #[serde(default)]
    pub platform_history: Vec<crate::journal::Entry>,
    /// Credits each identity top-up added, by its lock's txid.
    #[serde(default)]
    pub top_up_credits: std::collections::HashMap<String, evolite_api::Duffs>,
    /// Claims on contested names.
    #[serde(default)]
    pub name_requests: Vec<crate::name_requests::NameRequest>,
    /// The pool's operations with the amounts the scan loses.
    #[serde(default)]
    pub pool_history: Vec<crate::pool_history::Saved>,
    /// Tokens added by hand (the search finds the rest).
    #[serde(default)]
    pub tokens: Vec<crate::wallet_tokens::Tracked>,
    /// Token × identity pairs hidden.
    #[serde(default)]
    pub hidden_tokens: BTreeSet<(String, String)>,
    /// Contacts' addresses paid to, so the next payment goes to a new one (DIP-15).
    #[serde(default)]
    pub contact_addresses: crate::contact_addresses::Paid,
}

/// The sections a file has, by name: every field of [`Contents`] (the guard of AC-T-B2 holds
/// it to the wallet folder's files).
pub const SECTIONS: [&str; 14] = [
    "notes",
    "address_book",
    "contact_names",
    "hidden_requests",
    "connected_apps",
    "settings",
    "nft",
    "platform_history",
    "top_up_credits",
    "name_requests",
    "pool_history",
    "tokens",
    "hidden_tokens",
    "contact_addresses",
];

/// AC-A6 (AC-T-B2′): every file of a wallet's folder, by name, and whether the backup takes it —
/// with why not. A file new in the folder without a decision here fails the guard in the core,
/// so no file is forgotten. `token-history.json` is the token reads' cursors and a balance cache:
/// the token lines themselves are the journal's, which the file takes.
pub const FOLDER_FILES: &[(&str, bool, &str)] = &[
    (
        "wallet.json",
        false,
        "the vault: the phrase never goes into a backup",
    ),
    (
        "settings.json",
        true,
        "carried by the interface, section settings",
    ),
    ("notes.enc", true, "notes"),
    ("address-book.json", true, "address_book"),
    ("contact-names.json", true, "contact_names"),
    ("hidden-requests.json", true, "hidden_requests"),
    ("connected-apps.json", true, "connected_apps"),
    (
        "tokens.json",
        true,
        "tokens and hidden_tokens, the ones added by hand",
    ),
    (
        "platform-history.json",
        true,
        "platform_history, finished lines",
    ),
    ("top-up-credits.json", true, "top_up_credits"),
    ("name-requests.json", true, "name_requests"),
    ("pool-history.json", true, "pool_history"),
    ("contact-addresses.json", true, "contact_addresses"),
    (
        "token-history.json",
        false,
        "read cursors and a cache: the network gives them again",
    ),
    (
        "pending-names.json",
        false,
        "an operation not finished: the network goes on with it",
    ),
    (
        "fund-intents.json",
        false,
        "an operation not finished: the network goes on with it",
    ),
    ("platform-wallet.db", false, "a cache of the network"),
    ("shielded-pool.db", false, "a cache of the network"),
    ("instant-locks.json", false, "a cache of the network"),
    ("platform-sync.json", false, "a cache of the network"),
    ("lock-status.json", false, "a cache of the network"),
    ("mempool-seen.json", false, "a cache of the network"),
    ("mined.json", false, "a cache of the network"),
    ("contact-rescans.json", false, "a scan's state"),
    ("gap-rescans.json", false, "a scan's state"),
    ("core-balance.json", false, "a cache of the network"),
    (
        "contact-resend.json",
        false,
        "a resend's state: the network goes on with it",
    ),
    ("resync-on-start", false, "a flag for the next start"),
    (
        "auto-backup.json",
        false,
        "the automatic copies' folder and times: a new computer chooses its own",
    ),
    (
        "nft-series.json",
        false,
        "a Workshop series' sends and their state: an operation that goes on from the network",
    ),
];

/// Why a file was not opened. None of them changes anything (AC-A3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Not a backup, cut short or changed: «The file is damaged or changed».
    Damaged,
    /// Made by a newer evolite (R-11).
    Newer,
    /// For this network (R-4).
    OtherNetwork(String),
    /// Another wallet's — or this phrase with another BIP-39 password (AC-A7).
    OtherWallet,
    /// The file could not be read: the io error's kind.
    Read(&'static str),
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    network: String,
    wallet: String,
    nonce: String,
    ciphertext: String,
}

/// The open part of a file, read without the key: what the rotation of the automatic copies
/// checks before it deletes one (R-12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub network: String,
    pub wallet: String,
}

fn associated_data(version: u32, network: &str, wallet: &str) -> Vec<u8> {
    format!("{FORMAT}|{version}|{network}|{wallet}").into_bytes()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() || !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// The file's bytes for `contents` of the wallet `key` on `network`; an error only when the
/// system gives no random bytes for the nonce (review r1, Could).
pub fn seal(key: &BackupKey, network: &str, contents: &Contents) -> Result<Vec<u8>, &'static str> {
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| "no random bytes")?;
    let wallet = key.fingerprint();
    let plain = Zeroizing::new(serde_json::to_vec(contents).expect("the contents serialize"));
    let cipher = Aes256Gcm::new_from_slice(key.key.as_ref()).expect("a 32-byte key");
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &associated_data(VERSION, network, &wallet),
            },
        )
        .expect("AES-GCM encrypts any length the file can hold");
    Ok(serde_json::to_vec(&Envelope {
        format: FORMAT.into(),
        version: VERSION,
        network: network.into(),
        wallet,
        nonce: hex(&nonce),
        ciphertext: hex(&ciphertext),
    })
    .expect("the envelope serializes"))
}

fn envelope(bytes: &[u8]) -> Option<Envelope> {
    serde_json::from_slice::<Envelope>(bytes)
        .ok()
        .filter(|e| e.format == FORMAT)
}

/// The open part of `bytes`, if they are a backup file.
pub fn header(bytes: &[u8]) -> Option<Header> {
    envelope(bytes).map(|e| Header {
        version: e.version,
        network: e.network,
        wallet: e.wallet,
    })
}

/// What `bytes` hold, if they are a file of the wallet `key` on `network`: checked in the order
/// a person can act on — not a backup, too new, another network, another wallet, then the tag.
pub fn open(key: &BackupKey, network: &str, bytes: &[u8]) -> Result<Contents, OpenError> {
    let e = envelope(bytes).ok_or(OpenError::Damaged)?;
    if e.version > VERSION {
        return Err(OpenError::Newer);
    }
    if e.network != network {
        return Err(OpenError::OtherNetwork(e.network));
    }
    if e.wallet != key.fingerprint() {
        return Err(OpenError::OtherWallet);
    }
    let nonce = unhex(&e.nonce)
        .filter(|n| n.len() == 12)
        .ok_or(OpenError::Damaged)?;
    let ciphertext = unhex(&e.ciphertext).ok_or(OpenError::Damaged)?;
    let cipher = Aes256Gcm::new_from_slice(key.key.as_ref()).expect("a 32-byte key");
    let plain = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &associated_data(e.version, &e.network, &e.wallet),
                },
            )
            .map_err(|_| OpenError::Damaged)?,
    );
    serde_json::from_slice(&plain).map_err(|_| OpenError::Damaged)
}

/// Reads the file at `path` whole, refusing one bigger than [`MAX_FILE`].
pub fn read(path: &std::path::Path) -> Result<Vec<u8>, OpenError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE + 1).read_to_end(&mut bytes))
        .map_err(|e| OpenError::Read(evolite_common::diag::io_error_kind(&e)))?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(OpenError::Damaged);
    }
    Ok(bytes)
}

/// The address book's merge (the architect, answer 4): by address; an address here keeps its
/// own name. The book has no times, so a name deleted here may come back from an older copy.
/// How many entries were added.
pub fn merge_book(here: &mut Vec<Entry>, from: &[Entry]) -> usize {
    let mut added = 0;
    for entry in from {
        if !here.iter().any(|e| e.address == entry.address) {
            here.push(entry.clone());
            added += 1;
        }
    }
    added
}

/// Contacts' names: by identity, the later read wins. How many changed.
pub fn merge_names(here: &mut BTreeMap<String, Kept>, from: &BTreeMap<String, Kept>) -> usize {
    let mut changed = 0;
    for (id, theirs) in from {
        if here.get(id).is_none_or(|ours| theirs.at > ours.at) {
            here.insert(id.clone(), theirs.clone());
            changed += 1;
        }
    }
    changed
}

/// Hidden requests: a union (nothing is ever unhidden). How many were added.
pub fn merge_hidden(here: &mut BTreeSet<String>, from: &BTreeSet<String>) -> usize {
    let before = here.len();
    here.extend(from.iter().cloned());
    here.len() - before
}

/// Signed-in apps: by app and identity, the later sign-in wins. How many changed.
pub fn merge_apps(here: &mut Vec<App>, from: &[App]) -> usize {
    let mut changed = 0;
    for theirs in from {
        match here
            .iter_mut()
            .find(|a| a.contract == theirs.contract && a.identity == theirs.identity)
        {
            Some(ours) if theirs.last_used > ours.last_used => {
                *ours = theirs.clone();
                changed += 1;
            }
            Some(_) => {}
            None => {
                here.push(theirs.clone());
                changed += 1;
            }
        }
    }
    changed
}

pub mod auto;

#[cfg(test)]
mod tests;
