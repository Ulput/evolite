//! The addresses of contacts' keys this wallet has paid to, by contact. DIP-15 wants every
//! payment to a contact on a new address; platform-wallet marks the address it takes as used,
//! but after a restart the contact's account is built again from its request and comes back
//! with no used addresses (our SPV does not watch the contact's addresses, so nothing on chain
//! marks them either), and the next payment would go to an address already paid. This
//! file remembers them; before a payment they are marked used again. Public data only (identity
//! ids and addresses). A missing file is none remembered; one that does not read is set aside,
//! never written over (BUG-212).

use crate::json_file::{State, Store};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Contact identity (base58) → addresses paid.
pub type Paid = BTreeMap<String, Vec<String>>;

pub struct ContactAddresses {
    /// Read once, kept up to date.
    store: Store<Paid>,
    /// What the wallet's own sent payments show paid to contacts' keys ([`Self::saw`]): in
    /// memory too, so that a file set aside or started over never lets a payment go to one of
    /// them again (BUG-212, the architect).
    seen: std::sync::Mutex<Paid>,
}

impl ContactAddresses {
    pub fn new(path: PathBuf) -> Self {
        Self {
            store: Store::new(path),
            seen: Default::default(),
        }
    }

    /// The addresses of `contact`'s key already paid to: those kept, then those the sent
    /// payments show.
    pub fn paid(&self, contact: &str) -> Vec<String> {
        let mut paid = self
            .store
            .read(|all| all.get(contact).cloned())
            .unwrap_or_default();
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        for address in seen.get(contact).into_iter().flatten() {
            if !paid.contains(address) {
                paid.push(address.clone());
            }
        }
        paid
    }

    /// The wallet's sent Core payments to addresses of contacts' keys, as `(contact,
    /// address)` (BUG-212): kept in memory, and written to the file where it lacks them — so a
    /// file started over, or one written by a run that crashed before its write, is rebuilt from
    /// the history. Nothing is written while the file is set aside.
    pub fn saw(&self, payments: &[(String, String)]) {
        {
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            for (contact, address) in payments {
                let list = seen.entry(contact.clone()).or_default();
                if !list.contains(address) {
                    list.push(address.clone());
                }
            }
        }
        let _ = self.store.update_kept(|all| {
            let mut changed = false;
            for (contact, address) in payments {
                let list = all.entry(contact.clone()).or_default();
                if !list.contains(address) {
                    list.push(address.clone());
                    changed = true;
                }
            }
            ((), changed)
        });
    }

    /// Whether the file read or was set aside (BUG-212).
    pub fn state(&self) -> State {
        self.store.state()
    }

    /// The file, for «Try again» and «Start over».
    /// FEAT-0045 R-9: a backup's paid addresses added (a union: an address paid stays paid).
    /// How many were added.
    pub fn merge_backup(&self, from: &Paid) -> Result<usize, String> {
        self.store()
            .update(|all| {
                let mut added = 0;
                for (contact, addresses) in from {
                    let here = all.entry(contact.clone()).or_default();
                    for a in addresses {
                        if !here.contains(a) {
                            here.push(a.clone());
                            added += 1;
                        }
                    }
                }
                (added, added > 0)
            })
            .map_err(|e| e.to_string())
    }

    pub fn store(&self) -> &Store<Paid> {
        &self.store
    }

    /// `address` of `contact`'s key was paid (or may have been: the network gave no clear
    /// answer). Written beside and renamed over; a write that fails is retried with the next.
    pub fn add(&self, contact: &str, address: &str) {
        let _ = self.store.update_kept(|all| {
            let list = all.entry(contact.to_owned()).or_default();
            let new = !list.iter().any(|a| a == address);
            if new {
                list.push(address.to_owned());
            }
            ((), new)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_paid_addresses_across_loads() {
        let dir = std::env::temp_dir().join(format!("evolite-contact-paid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("contact-addresses.json");
        let paid = ContactAddresses::new(path.clone());
        assert!(paid.paid("alice").is_empty());
        paid.add("alice", "yA");
        paid.add("alice", "yB");
        paid.add("alice", "yA");
        paid.add("bob", "yC");
        let again = ContactAddresses::new(path);
        assert_eq!(again.paid("alice"), vec!["yA", "yB"], "once each, in order");
        assert_eq!(again.paid("bob"), vec!["yC"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
    fn store_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "evolite-store-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The bytes of each `<file>.unreadable-*` in `dir`.
    fn set_aside_files(dir: &std::path::Path, file: &str) -> Vec<Vec<u8>> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{file}.unreadable-"))
            })
            .map(|e| std::fs::read(e.path()).unwrap())
            .collect()
    }

    /// BUG-212: a file that does not read is set aside, not written over by the next address;
    /// «Start over» begins a new one and the old one stays.
    #[test]
    fn a_file_that_does_not_read_is_never_written_over() {
        let dir = store_dir("contact-paid");
        let path = dir.join("contact-addresses.json");
        std::fs::write(&path, b"{\"alice\": [\"yA\"").unwrap();
        let paid = ContactAddresses::new(path.clone());
        assert!(paid.paid("alice").is_empty());
        assert!(matches!(paid.state(), State::SetAside { .. }));
        paid.add("alice", "yB");
        assert!(!path.exists(), "nothing written in its place");
        assert_eq!(
            set_aside_files(&dir, "contact-addresses.json"),
            [b"{\"alice\": [\"yA\"".to_vec()]
        );
        paid.store().start_anew();
        paid.add("alice", "yC");
        assert_eq!(ContactAddresses::new(path).paid("alice"), ["yC"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-212 (the architect): what the sent payments show paid counts while the file is set
    /// aside, and fills a file started over.
    #[test]
    fn sent_payments_rebuild_the_paid_addresses() {
        let dir = store_dir("contact-seen");
        let path = dir.join("contact-addresses.json");
        std::fs::write(&path, b"{\"alice\"").unwrap();
        let paid = ContactAddresses::new(path.clone());
        let seen = [
            ("alice".to_owned(), "yA".to_owned()),
            ("alice".to_owned(), "yB".to_owned()),
        ];
        paid.saw(&seen);
        assert_eq!(
            paid.paid("alice"),
            ["yA", "yB"],
            "set aside: from the history"
        );
        assert!(!path.exists(), "and nothing written");
        paid.store().start_anew();
        paid.saw(&seen);
        assert_eq!(
            ContactAddresses::new(path.clone()).paid("alice"),
            ["yA", "yB"]
        );
        // Kept ones first, then what only the history shows; none twice.
        paid.add("alice", "yC");
        paid.saw(&[("alice".to_owned(), "yA".to_owned())]);
        assert_eq!(paid.paid("alice"), ["yA", "yB", "yC"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
