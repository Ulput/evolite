//! Payment notes (FEAT-0003): what the person wrote to an operation of the history, kept only
//! on this computer, encrypted, in `wallets/<network>/<id>/notes.enc` (AC-T1).
//!
//! - The file: `{"format":"evolite/notes/v1","nonce":<hex, 12 bytes>,"ciphertext":<hex>}`;
//!   inside, JSON `{"<key>": {"text": "...", "rate"?: {...}, "updated": <unix>}, ...}`.
//!   `rate` is the rate at the time of the operation (FEAT-0008, АРХ-1), only while «Fiat
//!   rate» is on; an entry may have a rate and an empty text. `text` is always written, so
//!   an older evolite still reads the file.
//! - AES-256-GCM with a fresh nonce for every write; the associated data is the format and the
//!   wallet id, so one wallet's file does not open as another's.
//! - The key: HKDF-SHA256 of the phrase's BIP-39 seed, salted with the wallet id, info
//!   `evolite/notes/v1`. Not the vault's key: that one comes from the password, and changing
//!   the password must not lose the notes (AC-T8). Derived once when the wallet opens, kept in
//!   memory while it is open (zeroed when dropped), never written anywhere.
//! - A file that does not open (not JSON, another format, a wrong tag) is never written over:
//!   it is renamed to `notes.enc.unreadable-<unix>` and new notes go to a new file (AC-T5).
//! - A file that is there but could not be read now (held by another program, no access, too
//!   big) is left where it is, and nothing is written at all until a later open reads it:
//!   every change is refused ([`NoteError::NotRead`]), so its notes are never written over.
//!
//! The keys are the history's (`Activity::key`): `core:<txid>`, `platform:<line>`,
//! `pool:<id>`. A send whose line is not in the history yet (one out of the pool: its id comes
//! with the pool's store) keeps its note under `pending:<kind>:<amount>:<at>` until
//! [`Notes::bind`] finds its line, after a restart too.
//!
//! Nothing here goes to the journal but the kind of failure: no text, no length, no key.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use evolite_api::{Activity, ActivityKind, Duffs, FiatRate, Timestamp};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use zeroize::{Zeroize, Zeroizing};

pub const FORMAT: &str = "evolite/notes/v1";
/// The most notes one wallet keeps (~6 MB at most, AC-T2).
pub const MAX_NOTES: usize = 10_000;
/// How long a send's note waits for its line before it goes: a send the network never took.
const PENDING_WAIT: Timestamp = 7 * 24 * 60 * 60;
/// How far a line's time may be before the send's (clocks differ a little).
const CLOCK_SLACK: Timestamp = 2 * 60;

/// The notes key of a wallet: from its seed, never from the password.
pub fn key_of(seed: &[u8], wallet_id: &str) -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0u8; 32]);
    hkdf::Hkdf::<sha2::Sha256>::new(Some(wallet_id.as_bytes()), seed)
        .expand(FORMAT.as_bytes(), key.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 length");
    key
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Envelope {
    format: String,
    nonce: String,
    ciphertext: String,
}

/// One note. Its text is zeroed when dropped: a lock leaves none in memory.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Note {
    /// Empty where the entry only keeps a rate.
    #[serde(default)]
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rate: Option<Rate>,
    updated: Timestamp,
}

/// The rate at the time of an operation, as the file keeps it (FEAT-0008).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Rate {
    currency: String,
    price: i64,
    at: Timestamp,
}

impl From<&FiatRate> for Rate {
    fn from(r: &FiatRate) -> Self {
        Self {
            currency: r.currency.clone(),
            price: r.price,
            at: r.at,
        }
    }
}

impl From<&Rate> for FiatRate {
    fn from(r: &Rate) -> Self {
        Self {
            currency: r.currency.clone(),
            price: r.price,
            at: r.at,
        }
    }
}

impl Drop for Note {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}

/// Why a note was not saved: no text in it, ever.
#[derive(Debug, PartialEq, Eq)]
pub enum NoteError {
    TooMany,
    /// The file there was not read when the wallet opened: nothing is written over it.
    NotRead,
    /// The file could not be written: the io error's kind.
    Write(&'static str),
}

/// The open wallet's notes: read once when it opens, kept in memory, written whole on each
/// change. Dropped with the open wallet (on lock, a switch, "Add a wallet").
pub struct Notes {
    path: PathBuf,
    key: Zeroizing<[u8; 32]>,
    wallet_id: String,
    all: BTreeMap<String, Note>,
    /// The file there did not open and was put aside this time (`Wallet::notes_error`).
    unreadable: bool,
    /// The file there could not be read now and is left as it is: no write until a later open
    /// reads it (review r1 must 1).
    not_read: bool,
}

fn associated_data(wallet_id: &str) -> Vec<u8> {
    format!("{FORMAT}|{wallet_id}").into_bytes()
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

fn now() -> Timestamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as Timestamp)
}

/// What a damaged file goes to: `notes.enc.unreadable-<unix>`, the next free second.
fn aside(path: &Path, at: Timestamp) -> PathBuf {
    let mut at = at;
    loop {
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".unreadable-{at}"));
        let aside = PathBuf::from(name);
        if !aside.exists() {
            return aside;
        }
        at += 1;
    }
}

/// Whether a file put aside before has exactly `bytes`: the same damaged file again (copied
/// back, say) is not put aside twice.
fn already_aside(path: &Path, bytes: &[u8]) -> bool {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let mut prefix = name.to_os_string();
    prefix.push(".unreadable-");
    let prefix = prefix.to_string_lossy().into_owned();
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            e.file_name().to_string_lossy().starts_with(&prefix)
                && crate::json_file::read(&e.path()).is_ok_and(|b| b == bytes)
        })
}

impl Notes {
    /// The notes in `path` for the wallet `wallet_id` with this seed. A missing file is no
    /// notes; one that does not open is put aside (see the module).
    pub fn open(path: PathBuf, seed: &[u8], wallet_id: &str) -> Self {
        let mut notes = Self {
            key: key_of(seed, wallet_id),
            wallet_id: wallet_id.to_owned(),
            path,
            all: BTreeMap::new(),
            unreadable: false,
            not_read: false,
        };
        match crate::json_file::read(&notes.path) {
            Ok(bytes) => match notes.decrypt(&bytes) {
                Some(all) => notes.all = all,
                None => notes.put_aside(&bytes),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                // Too big or not readable now: left where it is, nothing written over it until
                // the next open reads it (`write` refuses).
                evolite_common::diag::storage_failed(&notes.path, "notes read", &e);
                notes.not_read = true;
            }
        }
        notes
    }

    fn put_aside(&mut self, bytes: &[u8]) {
        self.unreadable = true;
        evolite_common::diag::record(evolite_common::diag::Record::Storage {
            file: "notes.enc".into(),
            op: "notes open",
            error: "unreadable",
        });
        let moved = if already_aside(&self.path, bytes) {
            std::fs::remove_file(&self.path)
        } else {
            std::fs::rename(&self.path, aside(&self.path, now()))
        };
        if let Err(e) = moved {
            evolite_common::diag::storage_failed(&self.path, "notes aside", &e);
        }
    }

    fn decrypt(&self, bytes: &[u8]) -> Option<BTreeMap<String, Note>> {
        let envelope: Envelope = serde_json::from_slice(bytes).ok()?;
        if envelope.format != FORMAT {
            return None;
        }
        let nonce = unhex(&envelope.nonce).filter(|n| n.len() == 12)?;
        let ciphertext = unhex(&envelope.ciphertext)?;
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).ok()?;
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: &associated_data(&self.wallet_id),
                    },
                )
                .ok()?,
        );
        serde_json::from_slice(&plain).ok()
    }

    fn encrypt(&self, all: &BTreeMap<String, Note>) -> Vec<u8> {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).expect("the OS random generator is available");
        self.encrypt_with(all, nonce)
    }

    /// The file's bytes for `all` under `nonce` ([`Self::encrypt`] takes a fresh one each time).
    fn encrypt_with(&self, all: &BTreeMap<String, Note>, nonce: [u8; 12]) -> Vec<u8> {
        let plain = Zeroizing::new(serde_json::to_vec(all).expect("notes serialize"));
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).expect("a 32-byte key");
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: &associated_data(&self.wallet_id),
                },
            )
            .expect("AES-GCM encrypts any length the file can hold");
        serde_json::to_vec(&Envelope {
            format: FORMAT.into(),
            nonce: hex(&nonce),
            ciphertext: hex(&ciphertext),
        })
        .expect("the envelope serializes")
    }

    /// Whether the file was there and did not open this time: put aside.
    #[doc(hidden)]
    pub fn unreadable(&self) -> bool {
        self.unreadable
    }

    /// What was wrong with the file at this open, for the history's E2.
    pub fn trouble(&self) -> Option<evolite_api::NotesError> {
        if self.not_read {
            Some(evolite_api::NotesError::NotRead)
        } else if self.unreadable {
            Some(evolite_api::NotesError::PutAside)
        } else {
            None
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.all
            .get(key)
            .map(|n| n.text.as_str())
            .filter(|t| !t.is_empty())
    }

    /// The rate kept at the time of the operation `key` (FEAT-0008).
    pub fn rate(&self, key: &str) -> Option<FiatRate> {
        self.all.get(key)?.rate.as_ref().map(FiatRate::from)
    }

    /// Keeps `rate` for each of `keys` (one write for all): a payment's own rate
    /// (`replace`) takes the place of one kept before; an operation first seen keeps the first
    /// it got (AC-T6). The text of a note there stays.
    pub fn set_rates(
        &mut self,
        keys: &[String],
        rate: &FiatRate,
        replace: bool,
    ) -> Result<(), NoteError> {
        let mut all = self.all.clone();
        let mut changed = false;
        for key in keys.iter().filter(|k| !k.is_empty()) {
            if !all.contains_key(key) && all.len() >= MAX_NOTES {
                return Err(NoteError::TooMany);
            }
            let note = all.entry(key.clone()).or_insert_with(|| Note {
                text: String::new(),
                rate: None,
                updated: now(),
            });
            if replace || note.rate.is_none() {
                note.rate = Some(rate.into());
                changed = true;
            }
        }
        if changed { self.write(all) } else { Ok(()) }
    }

    /// Whether any operation has a note (a send's note waiting for its line too).
    #[doc(hidden)]
    pub fn any(&self) -> bool {
        !self.all.is_empty()
    }

    /// Keeps `text` as the note of `key` (`None`: no note), written to the file before it is
    /// kept in memory: a failed write changes nothing. The text is as
    /// [`crate::note_text::kept`] leaves it.
    pub fn set(&mut self, key: &str, text: Option<String>) -> Result<(), NoteError> {
        if key.is_empty() {
            return Err(NoteError::Write("no key"));
        }
        let mut all = self.all.clone();
        match text {
            Some(text) => {
                if !all.contains_key(key) && all.len() >= MAX_NOTES {
                    return Err(NoteError::TooMany);
                }
                let rate = all.get(key).and_then(|n| n.rate.clone());
                all.insert(
                    key.to_owned(),
                    Note {
                        text,
                        rate,
                        updated: now(),
                    },
                );
            }
            // No note: an entry that also keeps a rate stays with the rate alone.
            None => match all.get_mut(key) {
                None => return Ok(()),
                Some(note) if note.rate.is_some() => {
                    if note.text.is_empty() {
                        return Ok(());
                    }
                    note.text.zeroize();
                    note.text = String::new();
                    note.updated = now();
                }
                Some(_) => {
                    all.remove(key);
                }
            },
        }
        self.write(all)
    }

    fn write(&mut self, all: BTreeMap<String, Note>) -> Result<(), NoteError> {
        // Every change comes here: a file that was not read stays as it is (review r1 must 1).
        if self.not_read {
            return Err(NoteError::NotRead);
        }
        let bytes = self.encrypt(&all);
        crate::json_file::write(&self.path, &bytes)
            .map_err(|e| NoteError::Write(evolite_common::diag::io_error_kind(&e)))?;
        self.all = all;
        Ok(())
    }

    /// The note and the rate of a send out of the pool, kept until its line is in the history
    /// and gone with it if it never comes.
    pub fn set_pending(
        &mut self,
        kind: Pending,
        (amount, at): (Duffs, Timestamp),
        text: Option<String>,
        rate: Option<&FiatRate>,
    ) {
        let key = format!("pending:{}:{amount}:{at}", kind.name());
        let mut kept = match text {
            Some(text) => self.set(&key, Some(text)),
            None => Ok(()),
        };
        if let (Ok(()), Some(rate)) = (&kept, rate) {
            kept = self.set_rates(&[key], rate, true);
        }
        if let Err(e) = kept {
            evolite_common::diag::record(evolite_common::diag::Record::Storage {
                file: "notes.enc".into(),
                op: "notes pending",
                error: match e {
                    NoteError::Write(kind) => kind,
                    NoteError::TooMany => "too many",
                    NoteError::NotRead => "not read",
                },
            });
        }
    }

    /// The lines [`Self::bind`] needs out of the history: none when no note waits (the usual
    /// case), else only those made since the oldest waiting send. Taken on the loop and small,
    /// so that the binding and its write go off it (AC-T9).
    pub fn lines_to_bind(&self, history: &[Activity]) -> Option<Vec<Activity>> {
        let mut waiting = self
            .all
            .keys()
            .filter(|k| k.starts_with("pending:"))
            .peekable();
        waiting.peek()?;
        let since = waiting
            .filter_map(|k| parse_pending(k))
            .map(|(_, _, at)| at)
            .min()
            .unwrap_or(Timestamp::MAX);
        Some(
            history
                .iter()
                .filter(|a| a.at >= since.saturating_sub(CLOCK_SLACK))
                .cloned()
                .collect(),
        )
    }

    /// Moves each send's waiting note to the line it became: the oldest line of its kind made
    /// since it was signed that has no note, out by at least its amount. One that waited too
    /// long goes. Written once when anything moved; a failed write is tried at the next call.
    pub fn bind(&mut self, history: &[Activity]) {
        if !self.all.keys().any(|k| k.starts_with("pending:")) {
            return;
        }
        let mut all = self.all.clone();
        let mut changed = false;
        let waiting: Vec<String> = all
            .keys()
            .filter(|k| k.starts_with("pending:"))
            .cloned()
            .collect();
        let now = now();
        for key in waiting {
            let Some((kind, amount, at)) = parse_pending(&key) else {
                all.remove(&key);
                changed = true;
                continue;
            };
            let line = history
                .iter()
                .filter(|a| {
                    !a.key.is_empty()
                        && !a.key.starts_with("pending:")
                        && kind.is(&a.kind)
                        && a.at >= at - CLOCK_SLACK
                        && -a.amount >= amount
                        && all.get(&a.key).is_none_or(|n| n.text.is_empty())
                })
                .min_by_key(|a| a.at);
            if let Some(line) = line {
                if let Some(mut note) = all.remove(&key) {
                    // A rate it got as an operation first seen gives way to the payment's own.
                    if note.rate.is_none() {
                        note.rate = all.get(&line.key).and_then(|n| n.rate.clone());
                    }
                    all.insert(line.key.clone(), note);
                }
                changed = true;
            } else if now > at + PENDING_WAIT {
                all.remove(&key);
                changed = true;
            }
        }
        if changed {
            let _ = self.write(all);
        }
    }
}

/// The kind of send a waiting note is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    /// Out of the pool: to a private address, a Platform address or Core.
    Pool,
}

impl Pending {
    fn name(self) -> &'static str {
        match self {
            Pending::Pool => "pool",
        }
    }

    fn is(self, kind: &ActivityKind) -> bool {
        match self {
            // The pool names no payee; one to the wallet's own private address is a self
            // transfer there (as `platform_sending` matches it).
            Pending::Pool => matches!(
                kind,
                ActivityKind::PoolSent | ActivityKind::SelfTransfer { .. }
            ),
        }
    }
}

fn parse_pending(key: &str) -> Option<(Pending, Duffs, Timestamp)> {
    let mut parts = key.strip_prefix("pending:")?.split(':');
    let kind = match parts.next()? {
        "pool" => Pending::Pool,
        _ => return None,
    };
    let amount = parts.next()?.parse().ok()?;
    let at = parts.next()?.parse().ok()?;
    Some((kind, amount, at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::{Chain, Proof};

    const WALLET: &str = "ab12";

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "evolite-notes-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed(n: u8) -> [u8; 64] {
        [n; 64]
    }

    fn line(key: &str, kind: ActivityKind, at: Timestamp, amount: Duffs) -> Activity {
        Activity {
            id: 1,
            kind,
            chain: Chain::Platform,
            at,
            amount,
            proof: Proof::Proved,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers: evolite_api::layer::IDENTITY,
            flow: evolite_api::Flow::Out,
            key: key.into(),
            detail: None,
            note: None,
            rate: None,
        }
    }

    /// Every file of the folder, bytes: what anyone with the disk could read.
    fn all_bytes(dir: &Path) -> Vec<u8> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .flat_map(|e| std::fs::read(e.path()).unwrap())
            .collect()
    }

    #[test]
    fn a_note_is_written_encrypted_and_read_back() {
        let dir = dir("back");
        let path = dir.join("notes.enc");
        let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
        assert!(!notes.unreadable() && !notes.any());
        notes
            .set("core:aa", Some("За дизайн логотипа ZQX-7731".into()))
            .unwrap();
        let bytes = all_bytes(&dir);
        assert!(!String::from_utf8_lossy(&bytes).contains("ZQX-7731"));
        assert!(!String::from_utf8_lossy(&bytes).contains("core:aa"));
        let envelope: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(envelope["format"], FORMAT);
        assert_eq!(envelope["nonce"].as_str().unwrap().len(), 24);
        assert!(!path.with_extension("enc.tmp").exists());
        let back = Notes::open(path.clone(), &seed(1), WALLET);
        assert_eq!(back.get("core:aa"), Some("За дизайн логотипа ZQX-7731"));
        assert!(!back.unreadable());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_write_takes_a_fresh_nonce() {
        let dir = dir("nonce");
        let path = dir.join("notes.enc");
        let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
        notes.set("core:aa", Some("one".into())).unwrap();
        let first: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        notes.set("core:aa", Some("one".into())).unwrap();
        let second: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_ne!(first["nonce"], second["nonce"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_empty_note_deletes_it() {
        let dir = dir("delete");
        let path = dir.join("notes.enc");
        let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
        notes.set("core:aa", Some("one".into())).unwrap();
        notes.set("core:aa", None).unwrap();
        assert_eq!(notes.get("core:aa"), None);
        assert_eq!(Notes::open(path, &seed(1), WALLET).get("core:aa"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_key_comes_from_the_seed_not_the_password() {
        // The vault's password is not an input at all: the same seed and wallet give the same
        // key, so a changed password reads the same notes (AC-T8).
        assert_eq!(*key_of(&seed(1), WALLET), *key_of(&seed(1), WALLET));
        assert_ne!(*key_of(&seed(1), WALLET), *key_of(&seed(2), WALLET));
        assert_ne!(*key_of(&seed(1), WALLET), *key_of(&seed(1), "cd34"));
    }

    #[test]
    fn another_wallets_file_does_not_open_and_is_put_aside() {
        let dir = dir("other");
        let path = dir.join("notes.enc");
        Notes::open(path.clone(), &seed(1), WALLET)
            .set("core:aa", Some("mine".into()))
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        // Another wallet, or the same seed under another wallet id (the associated data).
        for (seed, wallet) in [(seed(2), WALLET), (seed(1), "cd34")] {
            std::fs::write(&path, &original).unwrap();
            let other = Notes::open(path.clone(), &seed, wallet);
            assert!(other.unreadable());
            assert_eq!(other.get("core:aa"), None);
            assert!(!path.exists(), "never written over, never left in the way");
        }
        // Put aside once, with its bytes as they were: the second time it was the same file.
        let aside: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("notes.enc.unreadable-")
            })
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read(aside[0].path()).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_damaged_file_is_kept_aside_and_new_notes_go_to_a_new_one() {
        let dir = dir("damaged");
        let path = dir.join("notes.enc");
        for (n, damage) in [
            b"not json".to_vec(),
            br#"{"format":"evolite/notes/v2","nonce":"00","ciphertext":"00"}"#.to_vec(),
            br#"{"format":"evolite/notes/v1","nonce":"zz","ciphertext":"00"}"#.to_vec(),
        ]
        .into_iter()
        .enumerate()
        {
            std::fs::write(&path, &damage).unwrap();
            let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
            assert!(notes.unreadable(), "damage {n}");
            notes.set("core:bb", Some("new".into())).unwrap();
            assert_eq!(
                Notes::open(path.clone(), &seed(1), WALLET).get("core:bb"),
                Some("new")
            );
            std::fs::remove_file(&path).unwrap();
        }
        // Each damaged file kept, none over another.
        let aside = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("notes.enc.unreadable-")
            })
            .count();
        assert_eq!(aside, 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_more_than_ten_thousand_notes() {
        let dir = dir("many");
        let mut notes = Notes::open(dir.join("notes.enc"), &seed(1), WALLET);
        // Filled in memory: ten thousand writes would take a while.
        for n in 0..MAX_NOTES {
            notes.all.insert(
                format!("core:{n}"),
                Note {
                    text: "x".into(),
                    updated: 0,
                    rate: None,
                },
            );
        }
        assert_eq!(
            notes.set("core:new", Some("x".into())),
            Err(NoteError::TooMany)
        );
        // Changing one that is there is no new note.
        notes.set("core:5", Some("y".into())).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_write_keeps_what_was_there() {
        let dir = dir("fail");
        // A folder where the file should be: the rename over it fails.
        let path = dir.join("notes.enc");
        std::fs::create_dir_all(path.join("in-the-way")).unwrap();
        let mut notes = Notes {
            path: path.clone(),
            key: key_of(&seed(1), WALLET),
            wallet_id: WALLET.into(),
            all: BTreeMap::new(),
            unreadable: false,
            not_read: false,
        };
        assert!(matches!(
            notes.set("core:aa", Some("x".into())),
            Err(NoteError::Write(_))
        ));
        assert_eq!(notes.get("core:aa"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_pool_sends_note_waits_for_its_line_after_a_restart_too() {
        let dir = dir("pending");
        let path = dir.join("notes.enc");
        let at = now();
        let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
        notes.set_pending(Pending::Pool, (50_000, at), Some("rent".into()), None);
        // Not there yet: an older pool send and a credit transfer are not it.
        notes.bind(&[
            line("pool:01", ActivityKind::PoolSent, at - 3600, -50_100),
            line(
                "platform:02",
                ActivityKind::CreditTransfer { to: "@bob".into() },
                at,
                -50_100,
            ),
        ]);
        assert!(notes.get("pool:01").is_none() && notes.get("platform:02").is_none());
        // Restarted: still waiting, and bound to its line when it comes.
        let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
        notes.bind(&[
            line("pool:03", ActivityKind::PoolSent, at + 20, -50_120),
            line("pool:04", ActivityKind::PoolSent, at + 40, -50_120),
        ]);
        assert_eq!(notes.get("pool:03"), Some("rent"));
        assert_eq!(notes.get("pool:04"), None);
        assert!(!notes.all.keys().any(|k| k.starts_with("pending:")));
        assert_eq!(
            Notes::open(path, &seed(1), WALLET).get("pool:03"),
            Some("rent")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Review r1 must 1: a file that is there but was not read (a folder in its place here: on
    /// Windows its open is "access denied", elsewhere its read "is a directory"; or too big)
    /// is never written over, by a note, a send's note or a binding, until an open reads it.
    #[test]
    fn a_file_that_was_not_read_is_not_written_over_until_it_is() {
        let dir = dir("not-read");
        let path = dir.join("notes.enc");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("inside"), b"kept").unwrap();
        let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
        assert_eq!(notes.trouble(), Some(evolite_api::NotesError::NotRead));
        assert!(!notes.unreadable(), "not put aside: it stays where it is");
        assert_eq!(
            notes.set("core:aa", Some("new".into())),
            Err(NoteError::NotRead)
        );
        notes.set_pending(Pending::Pool, (1, now()), Some("rent".into()), None);
        notes.bind(&[line("pool:01", ActivityKind::PoolSent, now(), -2)]);
        assert_eq!(std::fs::read(path.join("inside")).unwrap(), b"kept");
        assert!(!path.with_extension("enc.tmp").exists());
        std::fs::remove_dir_all(&path).unwrap();
        // Too big to read: the same, the file keeps every byte.
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(crate::json_file::MAX_READ + 1).unwrap();
        drop(file);
        let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
        assert_eq!(notes.trouble(), Some(evolite_api::NotesError::NotRead));
        assert!(notes.set("core:aa", Some("new".into())).is_err());
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            crate::json_file::MAX_READ + 1
        );
        // A later open that reads it writes again.
        std::fs::remove_file(&path).unwrap();
        let mut good = Notes::open(path.clone(), &seed(1), WALLET);
        good.set("core:bb", Some("old".into())).unwrap();
        let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
        assert_eq!(notes.trouble(), None);
        notes.set("core:aa", Some("new".into())).unwrap();
        let back = Notes::open(path, &seed(1), WALLET);
        assert_eq!(
            (back.get("core:aa"), back.get("core:bb")),
            (Some("new"), Some("old"))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Review r1 should 1: the key and the file are the same from build to build. A changed
    /// salt, info, associated data or envelope would make every wallet's `notes.enc` unreadable
    /// after an update. The key is checked against OpenSSL (`openssl kdf -keylen 32 -kdfopt
    /// digest:SHA256 -kdfopt hexkey:01…01 -kdfopt salt:ab12 -kdfopt info:evolite/notes/v1
    /// HKDF`); the file is one this build wrote, and must keep opening.
    #[test]
    fn the_key_and_the_file_are_known_answers() {
        assert_eq!(
            hex(key_of(&seed(1), WALLET).as_ref()),
            "2b9fd0ce92b81b3f26c12e5f9bd8f7ca3e9f114d4818ec7d7e033cadb3bd91c5"
        );
        let dir = dir("kat");
        let path = dir.join("notes.enc");
        let notes = Notes::open(path.clone(), &seed(1), WALLET);
        let all = BTreeMap::from([(
            "core:aa".to_owned(),
            Note {
                text: "rent".into(),
                updated: 1_700_000_000,
                rate: None,
            },
        )]);
        let file = String::from_utf8(notes.encrypt_with(&all, [7; 12])).unwrap();
        assert_eq!(file, KNOWN_FILE);
        std::fs::write(&path, KNOWN_FILE).unwrap();
        let back = Notes::open(path, &seed(1), WALLET);
        assert_eq!(back.trouble(), None);
        assert_eq!(back.get("core:aa"), Some("rent"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `notes.enc` v1 of [`WALLET`] with seed 01…01: `core:aa` → "rent", updated 1700000000,
    /// nonce 07…07.
    const KNOWN_FILE: &str = concat!(
        r#"{"format":"evolite/notes/v1","nonce":"070707070707070707070707","ciphertext":"#,
        r#""0b904b6ff14463c71b8c54a9c0dc587be04a9722715e5345a2fd1be3d3e9823a44acfcf3fb4906d3d11f6fa111085ad3b4acaff2eeb3033e2b1f80f8f7ec6c8a"}"#
    );

    /// Review r2 must 1: `wallet()` hands `bind` only the lines from the oldest waiting send on,
    /// a clock's slack before it included: none without a waiting note, and with them every
    /// line a waiting note may bind to, so the binding is the same as over the whole history.
    #[test]
    fn the_lines_to_bind_start_at_the_oldest_waiting_send_less_the_slack() {
        let dir = dir("lines-to-bind");
        let t = now();
        let history = [
            line("pool:early", ActivityKind::PoolSent, t - 3 * 60, -50_100),
            line("pool:slack", ActivityKind::PoolSent, t - 60, -50_100),
            line("pool:after", ActivityKind::PoolSent, t + 20, -50_100),
            line(
                "pool:late",
                ActivityKind::PoolSent,
                t + 10 * 60 + 20,
                -70_100,
            ),
        ];
        let mut notes = Notes::open(dir.join("notes.enc"), &seed(1), WALLET);
        assert_eq!(notes.lines_to_bind(&history), None, "no note waits");
        notes.set_pending(
            Pending::Pool,
            (70_000, t + 10 * 60),
            Some("later".into()),
            None,
        );
        notes.set_pending(Pending::Pool, (50_000, t), Some("first".into()), None);
        let lines = notes.lines_to_bind(&history).expect("notes wait");
        let keys: Vec<&str> = lines.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, ["pool:slack", "pool:after", "pool:late"]);
        // The same binding as over the whole history.
        let mut whole = Notes::open(dir.join("whole.enc"), &seed(1), WALLET);
        whole.set_pending(
            Pending::Pool,
            (70_000, t + 10 * 60),
            Some("later".into()),
            None,
        );
        whole.set_pending(Pending::Pool, (50_000, t), Some("first".into()), None);
        notes.bind(&lines);
        whole.bind(&history);
        assert_eq!(notes.get("pool:slack"), Some("first"));
        assert_eq!(notes.get("pool:late"), Some("later"));
        for key in ["pool:early", "pool:slack", "pool:after", "pool:late"] {
            assert_eq!(notes.get(key), whole.get(key), "{key}");
        }
        assert!(!notes.all.keys().any(|k| k.starts_with("pending:")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_send_that_never_came_lets_its_note_go_after_a_week() {
        let dir = dir("expired");
        let mut notes = Notes::open(dir.join("notes.enc"), &seed(1), WALLET);
        notes.set_pending(
            Pending::Pool,
            (1, now() - PENDING_WAIT - 1),
            Some("old".into()),
            None,
        );
        notes.bind(&[]);
        assert!(!notes.any());
        std::fs::remove_dir_all(dir).unwrap();
    }
    fn usd(price: i64) -> FiatRate {
        FiatRate {
            currency: "USD".into(),
            price,
            at: 1_700_000_000,
        }
    }

    /// The inner JSON as the file keeps it, for what is and is not in it.
    fn inner(path: &Path) -> serde_json::Value {
        let notes = Notes::open(path.to_owned(), &seed(1), WALLET);
        let bytes = std::fs::read(path).unwrap();
        let all = notes.decrypt(&bytes).unwrap();
        serde_json::to_value(&all).unwrap()
    }

    #[test]
    fn a_rate_is_kept_beside_the_note_and_read_back() {
        let dir = dir("rate");
        let path = dir.join("notes.enc");
        let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
        notes
            .set_rates(&["core:aa".into()], &usd(3_078_000_000), true)
            .unwrap();
        // A rate alone is no note.
        assert_eq!(notes.get("core:aa"), None);
        notes.set("core:aa", Some("rent".into())).unwrap();
        let back = Notes::open(path.clone(), &seed(1), WALLET);
        assert_eq!(back.get("core:aa"), Some("rent"));
        assert_eq!(back.rate("core:aa"), Some(usd(3_078_000_000)));
        // The note gone, the rate stays; `text` is always written, for an older evolite.
        notes.set("core:aa", None).unwrap();
        let json = inner(&path);
        assert_eq!(json["core:aa"]["text"], "");
        assert_eq!(json["core:aa"]["rate"]["price"], 3_078_000_000i64);
        assert_eq!(notes.rate("core:aa"), Some(usd(3_078_000_000)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_rate_first_seen_never_changes_but_a_payments_own_takes_its_place() {
        let dir = dir("rate-once");
        let mut notes = Notes::open(dir.join("notes.enc"), &seed(1), WALLET);
        let key = vec!["platform:01".to_owned()];
        notes.set_rates(&key, &usd(1), false).unwrap();
        notes.set_rates(&key, &usd(2), false).unwrap();
        assert_eq!(notes.rate("platform:01"), Some(usd(1)), "AC-T6: once");
        notes.set_rates(&key, &usd(3), true).unwrap();
        assert_eq!(notes.rate("platform:01"), Some(usd(3)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn notes_without_rates_have_no_rate_field() {
        // AC-T1: with «Fiat rate» off nothing writes a rate, and the file has none.
        let dir = dir("no-rate");
        let path = dir.join("notes.enc");
        let mut notes = Notes::open(path.clone(), &seed(1), WALLET);
        notes.set("core:aa", Some("one".into())).unwrap();
        assert!(inner(&path)["core:aa"].get("rate").is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_of_before_the_rate_still_opens() {
        // A v1 file written before FEAT-0008: `text` and `updated` only.
        let dir = dir("old-file");
        let path = dir.join("notes.enc");
        let notes = Notes::open(path.clone(), &seed(1), WALLET);
        let mut old = BTreeMap::new();
        old.insert(
            "core:aa".to_owned(),
            serde_json::json!({"text": "old", "updated": 1}),
        );
        let plain = serde_json::to_vec(&old).unwrap();
        let cipher = Aes256Gcm::new_from_slice(notes.key.as_ref()).unwrap();
        let nonce = [7u8; 12];
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: &associated_data(WALLET),
                },
            )
            .unwrap();
        let envelope = serde_json::to_vec(&Envelope {
            format: FORMAT.into(),
            nonce: hex(&nonce),
            ciphertext: hex(&ciphertext),
        })
        .unwrap();
        std::fs::write(&path, envelope).unwrap();
        let back = Notes::open(path, &seed(1), WALLET);
        assert!(!back.unreadable());
        assert_eq!(back.get("core:aa"), Some("old"));
        assert_eq!(back.rate("core:aa"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_pool_sends_rate_waits_with_it_and_goes_with_it() {
        let dir = dir("rate-pending");
        let mut notes = Notes::open(dir.join("notes.enc"), &seed(1), WALLET);
        let t = now();
        notes.set_pending(Pending::Pool, (50_000, t), None, Some(&usd(9)));
        // Its line came and got the rate of a first seen operation: the payment's own wins.
        let line = line("pool:7", ActivityKind::PoolSent, t + 5, -60_000);
        notes.set_rates(&["pool:7".into()], &usd(1), false).unwrap();
        notes.bind(std::slice::from_ref(&line));
        assert_eq!(notes.rate("pool:7"), Some(usd(9)));
        // One that never comes goes with its rate (AC-T5).
        notes.set_pending(
            Pending::Pool,
            (1, t - PENDING_WAIT - 1),
            None,
            Some(&usd(9)),
        );
        notes.bind(&[]);
        assert!(!notes.all.keys().any(|k| k.starts_with("pending:")));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
