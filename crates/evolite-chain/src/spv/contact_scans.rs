//! The DashPay receiving accounts whose rescan from the wallet's birth has run to its end, kept
//! in `contact-rescans.json` beside the store (public data: the identity pair of each account).
//!
//! BUG-150: a contact's receiving account is built only once Platform is ready, after the
//! filters have gone past the blocks of the coins the contact sent: nothing watched its
//! addresses then, and an account built later only watches new blocks. On a data folder whose
//! store does not hold such a coin, the coin was never found and the balance was short. Each
//! account built is rescanned for once from the wallet's birth; noted here only when that
//! rescan has run to its end, so one cut short is asked again at the next start.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct ContactScans {
    path: PathBuf,
    done: Mutex<Option<BTreeSet<String>>>,
}

impl ContactScans {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            done: Mutex::new(None),
        }
    }

    /// An account's name in the file: `<our identity>-<contact>-<index>`, hex.
    pub fn key(user: &[u8; 32], friend: &[u8; 32], index: u32) -> String {
        let hex = |b: &[u8; 32]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        format!("{}-{}-{index}", hex(user), hex(friend))
    }

    fn load(&self) -> BTreeSet<String> {
        crate::json_file::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Of `accounts` (by [`Self::key`]), those not rescanned for yet.
    pub fn owed(&self, accounts: &[String]) -> Vec<String> {
        let mut guard = self.done.lock().unwrap_or_else(|e| e.into_inner());
        let done = guard.get_or_insert_with(|| self.load());
        accounts
            .iter()
            .filter(|a| !done.contains(*a))
            .cloned()
            .collect()
    }

    /// Notes that the rescan for `accounts` ran to its end (kept in the file).
    pub fn done(&self, accounts: &[String]) {
        let mut guard = self.done.lock().unwrap_or_else(|e| e.into_inner());
        let done = guard.get_or_insert_with(|| self.load());
        let before = done.len();
        done.extend(accounts.iter().cloned());
        if done.len() != before {
            let _ = crate::json_file::save(&self.path, done);
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
        scans.done(std::slice::from_ref(&a));
        assert_eq!(scans.owed(&[a.clone(), b.clone()]), vec![b.clone()]);
        let again = ContactScans::new(path);
        assert_eq!(again.owed(&[a, b.clone()]), vec![b], "kept in the file");
    }
}
