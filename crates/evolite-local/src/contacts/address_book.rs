//! The address book: Core addresses the person named, to pay again — an exchange, a shop, a
//! friend without DashPay. Kept in the wallet's folder, in a file of its own. Public data only
//! (addresses and the names given them); a missing or unreadable file is an empty book.

use crate::json_file::{Entries, State, Store};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub address: String,
    pub label: String,
}

/// The longest name an address can be given.
pub const LABEL_MAX: usize = 40;

pub struct AddressBook {
    /// Read once, then kept up to date with what is written. A file that does not read is set
    /// aside (`address-book.json.unreadable-<unix>`) before the next save could write over it:
    /// the names in it are not lost silently (BUG-212, in place of the `.bak` before).
    store: Store<Entries<Entry>>,
}

impl AddressBook {
    pub fn new(path: PathBuf) -> Self {
        Self {
            store: Store::new(path),
        }
    }

    /// The book, by name.
    pub fn entries(&self) -> Vec<Entry> {
        self.store.read(|e| e.known.clone())
    }

    /// Whether the file read or was set aside (BUG-212).
    pub fn state(&self) -> State {
        self.store.state()
    }

    /// The file, for «Try again» and «Start over».
    pub fn store(&self) -> &Store<Entries<Entry>> {
        &self.store
    }

    /// Keeps `address` under `label`, or renames it. The label is trimmed; it must have 1 to
    /// [`LABEL_MAX`] characters. The address is checked by the caller.
    pub fn save(&self, address: &str, label: &str) -> Result<(), String> {
        let label = label.trim();
        let count = label.chars().count();
        if count == 0 || count > LABEL_MAX {
            return Err(format!(
                tr!("A label of 1 to {LABEL_MAX} characters"),
                LABEL_MAX = LABEL_MAX
            ));
        }
        self.change(|all| {
            match all.iter_mut().find(|e| e.address == address) {
                Some(e) => e.label = label.to_owned(),
                None => all.push(Entry {
                    address: address.to_owned(),
                    label: label.to_owned(),
                }),
            }
            all.sort_by_key(|e| e.label.to_lowercase());
        })
    }

    /// FEAT-0045 R-9: a backup's entries merged in by address, a name here kept; written
    /// once, only when one was added. How many were added.
    pub fn merge(&self, from: &[Entry]) -> Result<usize, String> {
        self.store
            .update(|e| {
                let n = crate::backup::merge_book(&mut e.known, from);
                e.known.sort_by_key(|e| e.label.to_lowercase());
                (n, n > 0)
            })
            .map_err(|e| e.to_string())
    }

    pub fn forget(&self, address: &str) -> Result<(), String> {
        self.change(|all| all.retain(|e| e.address != address))
    }

    /// Applies `edit` to a copy of the book and keeps it only once it is on disk: a book that
    /// did not save does not show the change either. Written beside, flushed to disk and
    /// renamed over: a crash or a power cut leaves the old file or the new one. Refused while
    /// the file is set aside.
    fn change(&self, edit: impl FnOnce(&mut Vec<Entry>)) -> Result<(), String> {
        self.store
            .update(|e| {
                edit(&mut e.known);
                ((), true)
            })
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_named_addresses_across_loads() {
        let dir = std::env::temp_dir().join(format!("evolite-book-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("address-book.json");
        let book = AddressBook::new(path.clone());
        assert!(book.entries().is_empty(), "no file, an empty book");
        book.save("yB", "  Exchange ").unwrap();
        book.save("yA", "Apartment").unwrap();
        book.save("yB", "Exchange testnet").unwrap();
        assert!(book.save("yC", "   ").is_err(), "a name is needed");
        assert!(
            book.save("yC", &"ω".repeat(LABEL_MAX + 1)).is_err(),
            "and not too long"
        );
        let again = AddressBook::new(path);
        assert_eq!(
            again.entries(),
            vec![
                Entry {
                    address: "yA".into(),
                    label: "Apartment".into()
                },
                Entry {
                    address: "yB".into(),
                    label: "Exchange testnet".into()
                },
            ],
            "by name, renamed in place, trimmed"
        );
        again.forget("yA").unwrap();
        assert_eq!(
            AddressBook::new(dir.join("address-book.json"))
                .entries()
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_book_that_did_not_save_does_not_show_the_change() {
        let dir = std::env::temp_dir().join(format!("evolite-book-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A folder where the file should be: the rename over it fails.
        let path = dir.join("address-book.json");
        std::fs::create_dir_all(&path).unwrap();
        let book = AddressBook::new(path);
        assert!(book.save("yA", "Apartment").is_err());
        assert!(book.entries().is_empty(), "not saved, not shown");
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

    /// BUG-212: a damaged book is set aside with what it held (in place of the `.bak` before)
    /// and a change is refused until «Start over».
    #[test]
    fn a_damaged_book_is_set_aside_and_not_written_over() {
        let dir = store_dir("book");
        let path = dir.join("address-book.json");
        std::fs::write(&path, b"[{\"address\": \"yB\"").unwrap();
        let book = AddressBook::new(path.clone());
        assert!(book.entries().is_empty());
        assert!(book.save("yA", "Exchange").is_err());
        assert_eq!(
            set_aside_files(&dir, "address-book.json"),
            [b"[{\"address\": \"yB\"".to_vec()]
        );
        assert!(!path.exists());
        book.store().start_anew();
        book.save("yA", "Exchange").unwrap();
        assert_eq!(AddressBook::new(path).entries().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
