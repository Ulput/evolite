//! The address book: Core addresses the person named, to pay again — an exchange, a shop, a
//! friend without DashPay. Kept in the wallet's folder, in a file of its own. Public data only
//! (addresses and the names given them); a missing or unreadable file is an empty book.

use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub address: String,
    pub label: String,
}

/// The longest name an address can be given.
pub const LABEL_MAX: usize = 40;

pub struct AddressBook {
    path: PathBuf,
    /// Read once, then kept up to date with what is written.
    entries: Mutex<Option<Vec<Entry>>>,
}

impl AddressBook {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            entries: Mutex::new(None),
        }
    }

    /// The book, by name.
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.get_or_insert_with(|| self.load()).clone()
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

    pub fn forget(&self, address: &str) -> Result<(), String> {
        self.change(|all| all.retain(|e| e.address != address))
    }

    /// Applies `edit` to a copy of the book and keeps it only once it is on disk: a book that
    /// did not save does not show the change either.
    fn change(&self, edit: impl FnOnce(&mut Vec<Entry>)) -> Result<(), String> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = entries.get_or_insert_with(|| self.load()).clone();
        edit(&mut all);
        self.store(&all)?;
        *entries = Some(all);
        Ok(())
    }

    /// The book from its file. A file that does not read is put aside as `.bak` before the
    /// next save could write over it: the names in it are not lost silently.
    fn load(&self) -> Vec<Entry> {
        let Ok(bytes) = crate::json_file::read(&self.path) else {
            return Vec::new();
        };
        match serde_json::from_slice(&bytes) {
            Ok(entries) => entries,
            Err(_) => {
                let _ = std::fs::rename(&self.path, self.path.with_extension("json.bak"));
                Vec::new()
            }
        }
    }

    /// Written beside, flushed to disk and renamed over: a crash or a power cut leaves the
    /// old file or the new one.
    fn store(&self, entries: &[Entry]) -> Result<(), String> {
        crate::json_file::save(&self.path, &entries)
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

    /// A file that does not read is kept as `.bak`, not written over by the next save.
    #[test]
    fn a_broken_book_is_put_aside_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("evolite-book-bak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("address-book.json");
        std::fs::write(&path, b"{ broken").unwrap();
        let book = AddressBook::new(path.clone());
        assert!(book.entries().is_empty());
        book.save("yA", "Apartment").unwrap();
        assert_eq!(
            std::fs::read(path.with_extension("json.bak")).unwrap(),
            b"{ broken"
        );
        assert_eq!(AddressBook::new(path).entries().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
