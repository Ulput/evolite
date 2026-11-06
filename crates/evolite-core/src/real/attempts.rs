//! The wrong-password count of each wallet of this computer, kept in the wallet's folder
//! (`unlock-attempts.json`, FEAT-0002 AC-T1): `{"failures": n, "waitUntil": unix-seconds|null}`.
//! Not a secret; it goes with the folder when the wallet is deleted. A missing file is no
//! count; a damaged one too (one line in the journal) — whoever can edit it can take the vault
//! itself, so failing open costs nothing (Q-T2).
//!
//! Only `Unlock` and `SwitchWallet` count here (R-31): the vault functions do not, or a wrong
//! payment password (BUG-42's `spend_gate`) or a deletion's would delay unlocking. FEAT-0004
//! may call [`Attempts::check`] / [`Attempts::begin`] / [`Attempts::record`] from more places.

use super::{json_file, wallets};
use crate::attempts::{Ledger, Outcome, Tries};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const FILE: &str = "unlock-attempts.json";

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Stored {
    failures: u32,
    wait_until: Option<u64>,
}

/// The counts of this computer's wallets, read once, written on every change.
pub struct Attempts {
    root: PathBuf,
    ledger: Ledger,
}

impl Attempts {
    /// Reads the counts of `ids`, the wallets of the data folder `root`.
    pub fn read<'a>(root: &Path, ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut ledger = Ledger::default();
        for id in ids {
            if let Some(tries) = read_file(&path(root, id)) {
                ledger.set(id, tries);
            }
        }
        Self {
            root: root.to_owned(),
            ledger,
        }
    }

    /// Wrong passwords in a row for wallet `id`.
    pub fn failures(&self, id: &str) -> u32 {
        self.ledger.get(id).failures
    }

    /// When the pause of wallet `id` ends, if one lasts at `now`. An end cut down by a clock set
    /// back is written to the file at once (R-31 item 4), so after a restart it is not longer.
    pub fn check(&mut self, id: &str, now: u64) -> Option<u64> {
        let kept = self.ledger.get(id);
        let until = self.ledger.until(id, now, Instant::now());
        if self.ledger.get(id) != kept {
            self.save(id);
        }
        until
    }

    /// Before the password of wallet `id` is checked: `Err(until)` in a pause; else the try is
    /// written down as wrong already (before Argon2, AC-T2) and the count before it returned.
    ///
    /// If that write fails, the try is counted in memory only and the check goes on: a fail-open
    /// on purpose. A process killed during Argon2 then loses that one try, but whoever can make
    /// the folder unwritable can take the vault in it anyway (Q-T2; review r1, should 1).
    pub fn begin(&mut self, id: &str, now: u64) -> Result<Tries, u64> {
        let kept = self.ledger.get(id);
        match self.ledger.begin(id, now, Instant::now()) {
            Ok(before) => {
                self.save(id);
                Ok(before)
            }
            Err(until) => {
                if self.ledger.get(id) != kept {
                    self.save(id);
                }
                Err(until)
            }
        }
    }

    /// After the check (`before` from [`Self::begin`]): written before the answer goes.
    pub fn record(&mut self, id: &str, before: Tries, outcome: Outcome) {
        self.ledger.end(id, before, outcome, Instant::now());
        if outcome != Outcome::Wrong {
            self.save(id);
        }
    }

    /// Wallet `id` was deleted: its count goes with it.
    pub fn forget(&mut self, id: &str) {
        self.ledger.forget(id);
        let _ = std::fs::remove_file(path(&self.root, id));
    }

    fn save(&self, id: &str) {
        let tries = self.ledger.get(id);
        // A failed write is in the journal already ([`json_file::write`]); the unlock goes on.
        let _ = json_file::save(
            &path(&self.root, id),
            &Stored {
                failures: tries.failures,
                wait_until: tries.wait_until,
            },
        );
    }
}

fn path(root: &Path, id: &str) -> PathBuf {
    wallets::wallet_dir(root, id).join(FILE)
}

/// The count in `path`; `None` when there is none or it cannot be read.
fn read_file(path: &Path) -> Option<Tries> {
    let bytes = match json_file::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            crate::diag::storage_failed(path, "read", &e);
            return None;
        }
    };
    match serde_json::from_slice::<Stored>(&bytes) {
        Ok(s) => Some(Tries {
            failures: s.failures,
            wait_until: s.wait_until,
        }),
        Err(_) => {
            let e =
                std::io::Error::new(std::io::ErrorKind::InvalidData, "attempts file unreadable");
            crate::diag::storage_failed(path, "read", &e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evolite-attempts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn fail(a: &mut Attempts, id: &str, now: u64) {
        let before = a.begin(id, now).expect("no pause");
        a.record(id, before, Outcome::Wrong);
    }

    #[test]
    fn the_count_is_written_before_the_check_and_read_after_a_restart() {
        let root = folder("restart");
        let mut a = Attempts::read(&root, ["w1"]);
        for _ in 0..4 {
            fail(&mut a, "w1", NOW);
        }
        // The 5th: counted on the disk before the password is checked (AC-T2).
        let before = a.begin("w1", NOW).unwrap();
        let mut again = Attempts::read(&root, ["w1"]);
        assert_eq!(again.failures("w1"), 5);
        assert_eq!(again.check("w1", NOW + 5), Some(NOW + 30));
        a.record("w1", before, Outcome::Wrong);
        let text = std::fs::read_to_string(path(&root, "w1")).unwrap();
        assert!(
            text.contains("\"failures\": 5") && text.contains("\"waitUntil\""),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_damaged_or_missing_file_is_no_pause() {
        let root = folder("damaged");
        let file = path(&root, "w1");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"{\"failures\": \"many\"").unwrap();
        let mut a = Attempts::read(&root, ["w1", "w2"]);
        assert_eq!((a.failures("w1"), a.check("w1", NOW)), (0, None));
        assert_eq!((a.failures("w2"), a.check("w2", NOW)), (0, None));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_right_password_and_a_deletion_clear_the_file() {
        let root = folder("clear");
        let mut a = Attempts::read(&root, ["w1"]);
        fail(&mut a, "w1", NOW);
        let before = a.begin("w1", NOW).unwrap();
        a.record("w1", before, Outcome::Right);
        assert_eq!(Attempts::read(&root, ["w1"]).failures("w1"), 0);
        fail(&mut a, "w1", NOW);
        a.forget("w1");
        assert!(!path(&root, "w1").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review r1, must 1: the clock set back a day in a 5-minute pause. The end cut to one step
    /// is written to the file, so a restart one step (and a second) later finds no pause.
    #[test]
    fn a_pause_cut_by_a_clock_set_back_stays_cut_after_a_restart() {
        let root = folder("back");
        let mut a = Attempts::read(&root, ["w1"]);
        // After the 8th wrong password at NOW, as a restart reads it (no monotonic clock).
        a.ledger.set(
            "w1",
            Tries {
                failures: 8,
                wait_until: Some(NOW + 300),
            },
        );
        a.save("w1");
        let mut a = Attempts::read(&root, ["w1"]);
        assert_eq!(a.check("w1", NOW), Some(NOW + 300));
        let back = NOW - 24 * 60 * 60;
        assert_eq!(a.check("w1", back), Some(back + 300));
        let mut again = Attempts::read(&root, ["w1"]);
        assert_eq!(
            again.check("w1", back + 100),
            Some(back + 300),
            "cut on disk"
        );
        let mut later = Attempts::read(&root, ["w1"]);
        assert_eq!(later.check("w1", back + 301), None, "one step waited: over");
        assert_eq!(later.failures("w1"), 8, "the count is kept");
        let _ = std::fs::remove_dir_all(&root);
    }
}
