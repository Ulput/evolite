//! The DashPay receiving accounts whose rescan from the wallet's birth has run to its end, kept
//! in `contact-rescans.json` beside the store (public data: the identity pair of each account).
//!
//! BUG-150: a contact's receiving account is built only once Platform is ready, after the
//! filters have gone past the blocks of the coins the contact sent: nothing watched its
//! addresses then, and an account built later only watches new blocks. On a data folder whose
//! store does not hold such a coin, the coin was never found and the balance was short. Each
//! account built is rescanned for once from the wallet's birth; noted here only when that
//! rescan has run to its end, so one cut short is asked again at the next start.
//!
//! BUG-221 (live, qa-f27): a rescan in the run that built the account ran to its end and found
//! nothing, while the next start's found the contact's coins. One that adds no DashPay coin is
//! not final: its accounts are kept in `contact-rescans-empty.json` and asked once more after
//! the next start (never twice in one run); found empty again, they are done.
//!
//! BUG-232 review r2: the same rule, in `gap-rescans.json`, for outputs of ours that a rescan
//! is owed to so they get their coin (keyed by outpoint): one that gets none is not asked on
//! every start for ever.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct ContactScans {
    path: PathBuf,
    done: Mutex<Option<BTreeSet<String>>>,
    /// Accounts whose rescan ran to its end without a coin, once (BUG-221).
    empty_path: PathBuf,
    empty: Mutex<Option<BTreeSet<String>>>,
    /// Asked in this run: not again before the next start.
    this_run: Mutex<BTreeSet<String>>,
}

impl ContactScans {
    pub fn new(path: PathBuf) -> Self {
        let stem = path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        Self {
            empty_path: path.with_file_name(format!("{stem}-empty.json")),
            path,
            done: Mutex::new(None),
            empty: Mutex::new(None),
            this_run: Mutex::new(BTreeSet::new()),
        }
    }

    /// An account's name in the file: `<our identity>-<contact>-<index>`, hex.
    pub fn key(user: &[u8; 32], friend: &[u8; 32], index: u32) -> String {
        let hex = |b: &[u8; 32]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        format!("{}-{}-{index}", hex(user), hex(friend))
    }

    fn load(path: &std::path::Path) -> BTreeSet<String> {
        // BUG-235: one that does not read is set aside, not written over.
        // default-ok: a cache: unread means rescan, set aside
        crate::json_file::read_cache(path, |b| serde_json::from_slice(b).ok()).unwrap_or_default()
    }

    /// Of `accounts` (by [`Self::key`]), those not rescanned for yet, nor asked in this run.
    pub fn owed(&self, accounts: &[String]) -> Vec<String> {
        let mut guard = self.done.lock().unwrap_or_else(|e| e.into_inner());
        let done = guard.get_or_insert_with(|| Self::load(&self.path));
        let this_run = self.this_run.lock().unwrap_or_else(|e| e.into_inner());
        accounts
            .iter()
            .filter(|a| !done.contains(*a) && !this_run.contains(*a))
            .cloned()
            .collect()
    }

    /// Notes that the rescan for `accounts` ran to its end (kept in the files): done when it
    /// added a DashPay coin (`found`) or was found empty once before; else asked once more
    /// after the next start (BUG-221).
    pub fn finished(&self, accounts: &[String], found: bool) {
        self.this_run
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(accounts.iter().cloned());
        let mut guard = self.done.lock().unwrap_or_else(|e| e.into_inner());
        let done = guard.get_or_insert_with(|| Self::load(&self.path));
        let mut empty_guard = self.empty.lock().unwrap_or_else(|e| e.into_inner());
        let empty = empty_guard.get_or_insert_with(|| Self::load(&self.empty_path));
        let (done_before, empty_before) = (done.len(), empty.len());
        for account in accounts {
            let empty_once = empty.remove(account);
            if found || empty_once {
                done.insert(account.clone());
            } else {
                empty.insert(account.clone());
            }
        }
        if done.len() != done_before {
            let _ = crate::json_file::save(&self.path, done);
        }
        if empty.len() != empty_before {
            let _ = crate::json_file::save(&self.empty_path, empty);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("evolite-contact-scans")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("contact-rescans.json")
    }

    /// BUG-150: each account is owed one rescan until it has run to its end, also across a
    /// restart (a new instance on the same file); two accounts of one pair differ by index.
    #[test]
    fn an_account_is_rescanned_for_once_across_restarts() {
        let path = file("once");
        let a = ContactScans::key(&[1; 32], &[2; 32], 0);
        let b = ContactScans::key(&[1; 32], &[3; 32], 0);
        assert_ne!(a, b);
        assert_ne!(a, ContactScans::key(&[1; 32], &[2; 32], 1));
        let scans = ContactScans::new(path.clone());
        assert_eq!(
            scans.owed(&[a.clone(), b.clone()]),
            vec![a.clone(), b.clone()]
        );
        scans.finished(std::slice::from_ref(&a), true);
        assert_eq!(scans.owed(&[a.clone(), b.clone()]), vec![b.clone()]);
        let again = ContactScans::new(path);
        assert_eq!(again.owed(&[a, b.clone()]), vec![b], "kept in the file");
    }

    /// BUG-221: a rescan that found no coin is not final: not asked again in the same run,
    /// asked once more after the next start, and done when that one is empty too — or at once
    /// when it finds one.
    #[test]
    fn an_empty_rescan_is_asked_once_more_after_a_restart() {
        let path = file("empty");
        let a = ContactScans::key(&[1; 32], &[2; 32], 0);
        let b = ContactScans::key(&[1; 32], &[3; 32], 0);
        let both = [a.clone(), b.clone()];
        let first = ContactScans::new(path.clone());
        first.finished(&both, false);
        assert!(first.owed(&both).is_empty(), "not again in this run");
        let second = ContactScans::new(path.clone());
        assert_eq!(second.owed(&both), both.to_vec(), "once more after a start");
        second.finished(std::slice::from_ref(&a), false);
        second.finished(std::slice::from_ref(&b), true);
        let third = ContactScans::new(path);
        assert!(third.owed(&both).is_empty(), "empty twice, or found: done");
    }

    /// BUG-232 review r2: the outputs owed a rescan for their coin keep their own files (the
    /// empty one beside it), apart from the contacts': one that got no coin twice is done.
    #[test]
    fn the_gap_rescans_keep_their_own_files() {
        let contacts = file("gap");
        let gap = contacts.with_file_name("gap-rescans.json");
        let coin = "ab:1".to_string();
        let first = ContactScans::new(gap.clone());
        first.finished(std::slice::from_ref(&coin), false);
        assert!(gap.with_file_name("gap-rescans-empty.json").exists());
        assert!(
            !contacts
                .with_file_name("contact-rescans-empty.json")
                .exists()
        );
        assert_eq!(
            ContactScans::new(contacts).owed(std::slice::from_ref(&coin)),
            vec![coin.clone()]
        );
        let second = ContactScans::new(gap.clone());
        second.finished(std::slice::from_ref(&coin), false);
        assert!(ContactScans::new(gap).owed(&[coin]).is_empty());
    }
}
