//! The wallet's own InstantLocks kept across a restart (BUG-173, the architect): `txid → isdlock`
//! bytes (hex) of the wallet's transactions not in a ChainLocked block yet, saved once this run
//! verified the lock. After a restart peers do not send an old transaction's lock again and
//! dash-spv keeps none, so a payment already shown «Confirmed» fell back to «Pending» until its
//! block. A kept lock shows it as «Confirmed · checking»; the coin is not final for spending
//! until the lock is verified again (R-1, part 2 of BUG-173). Nothing kept is trusted unchecked.
//!
//! `instant-locks.json` in the wallet's folder, written whole (`json_file`); at most [`MAX`], the
//! oldest go first; a file that cannot be read is never written over.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::json_file;

/// At most this many locks are kept.
pub const MAX: usize = 100;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
struct Kept {
    /// The `isdlock` message, hex.
    lock: String,
    /// Unix seconds it was kept.
    added: i64,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct File {
    #[serde(default)]
    locks: BTreeMap<String, Kept>,
}

#[derive(Default)]
enum State {
    #[default]
    Unread,
    Read(BTreeMap<String, Kept>),
    /// Cannot be read: left as it is, and nothing in it is known.
    Broken,
}

pub struct KeptLocks {
    path: PathBuf,
    state: Mutex<State>,
}

impl KeptLocks {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::default(),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*state, State::Unread) {
            *state = match json_file::read(&self.path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::Read(BTreeMap::new()),
                Err(_) => State::Broken,
                Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
                    Ok(file) => State::Read(file.locks),
                    Err(_) => State::Broken,
                },
            };
        }
        f(&mut state)
    }

    /// Whether a lock of `txid` is kept.
    pub fn contains(&self, txid: &str) -> bool {
        self.with(|state| matches!(state, State::Read(locks) if locks.contains_key(txid)))
    }

    /// The kept lock of `txid`, its bytes (for part 2: verified again).
    pub fn bytes(&self, txid: &str) -> Option<Vec<u8>> {
        self.with(|state| match state {
            State::Read(locks) => locks.get(txid).and_then(|k| from_hex(&k.lock)),
            _ => None,
        })
    }

    /// Keeps the locks `keep` (txid, bytes) and drops those of `drop` (now in a ChainLocked block,
    /// or not the wallet's any more), at `now`; written once, only when something changed.
    pub fn update(&self, keep: Vec<(String, Vec<u8>)>, drop: &[String], now: i64) {
        self.with(|state| {
            let State::Read(locks) = state else {
                return;
            };
            let before = locks.clone();
            for txid in drop {
                locks.remove(txid);
            }
            for (txid, bytes) in keep {
                locks.entry(txid).or_insert(Kept {
                    lock: to_hex(&bytes),
                    added: now,
                });
            }
            while locks.len() > MAX {
                let oldest = locks
                    .iter()
                    .min_by_key(|(_, k)| k.added)
                    .map(|(txid, _)| txid.clone());
                match oldest {
                    Some(txid) => locks.remove(&txid),
                    None => break,
                };
            }
            if *locks != before {
                // A file that cannot be written only loses what it would keep: «Pending» until
                // the block, as before.
                let _ = json_file::save(
                    &self.path,
                    &File {
                        locks: locks.clone(),
                    },
                );
            }
        });
    }
}

/// `bytes` as lowercase hex.
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex back to bytes; `None` for anything else.
fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evolite-kept-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("instant-locks.json")
    }

    /// BUG-173 AC-T1, AC-T4: a kept lock is read back by a new run, byte for byte; dropped once
    /// its transaction is ChainLocked; never more than 100, the oldest go.
    #[test]
    fn a_lock_is_kept_across_a_restart_until_its_block() {
        let path = folder("kept");
        let locks = KeptLocks::new(path.clone());
        locks.update(vec![("aa".into(), vec![1, 2, 3])], &[], 10);
        let again = KeptLocks::new(path.clone());
        assert!(again.contains("aa"));
        assert_eq!(again.bytes("aa"), Some(vec![1, 2, 3]));
        again.update(Vec::new(), &["aa".to_owned()], 20);
        assert!(
            !KeptLocks::new(path).contains("aa"),
            "in a ChainLocked block: gone"
        );
        let many = KeptLocks::new(folder("many"));
        many.update(
            (0..MAX as i64 + 5)
                .map(|n| (format!("{n:03}"), vec![n as u8]))
                .collect(),
            &[],
            0,
        );
        many.update(vec![("new".into(), vec![9])], &[], 1);
        assert!(many.contains("new"), "the newest stays");
        let count = (0..MAX as i64 + 5)
            .filter(|n| many.contains(&format!("{n:03}")))
            .count();
        assert_eq!(count, MAX - 1, "at most {MAX}");
    }

    /// A file that cannot be read is not written over.
    #[test]
    fn a_broken_file_is_left_alone() {
        let path = folder("broken");
        std::fs::write(&path, b"{ not json").unwrap();
        let locks = KeptLocks::new(path.clone());
        locks.update(vec![("aa".into(), vec![1])], &[], 1);
        assert!(!locks.contains("aa"));
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }
}
