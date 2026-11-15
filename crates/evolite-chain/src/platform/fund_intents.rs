//! Where each funding of the Platform address was signed to go (BUG-171): `txid → address`,
//! written once the lock is built, read by «Continue» so that it credits the address the review
//! showed, removed once the lock is closed. The shown address is the next unused one, and it
//! moves when the last one receives something: «Continue» after a restart took the one shown
//! then, not the one signed for. No secrets: an address of the wallet's own and a txid that is
//! on chain anyway. After a restore from the phrase there is no record: «Continue» goes to the
//! address shown now, and its dialog says which.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::json_file;

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct File {
    /// By the lock's txid: the address (bech32m) it was signed to credit.
    intents: BTreeMap<String, String>,
}

#[derive(Default)]
enum State {
    /// Not read yet.
    #[default]
    Unread,
    Read(BTreeMap<String, String>),
    /// A file there that cannot be read: kept as it is (never written over), and nothing in it
    /// is known — «Continue» goes to the address shown now.
    Broken,
}

/// `fund-intents.json` in the wallet's folder.
pub struct FundIntents {
    path: PathBuf,
    state: Mutex<State>,
}

impl FundIntents {
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
                    Ok(file) => State::Read(file.intents),
                    Err(_) => State::Broken,
                },
            };
        }
        f(&mut state)
    }

    fn save(&self, intents: &BTreeMap<String, String>) {
        // A file that cannot be written only loses the record: «Continue» then goes to the
        // address shown and says so.
        let _ = json_file::save(
            &self.path,
            &File {
                intents: intents.clone(),
            },
        );
    }

    /// The address the lock `txid` was signed to credit, if it was recorded here.
    pub fn address(&self, txid: &str) -> Option<String> {
        self.with(|state| match state {
            State::Read(intents) => intents.get(txid).cloned(),
            _ => None,
        })
    }

    /// The lock `txid` was built to credit `address`.
    pub fn put(&self, txid: &str, address: &str) {
        self.with(|state| {
            if let State::Read(intents) = state {
                intents.insert(txid.to_owned(), address.to_owned());
                self.save(intents);
            }
        });
    }

    /// The lock `txid` is closed: its record goes.
    pub fn remove(&self, txid: &str) {
        self.with(|state| {
            if let State::Read(intents) = state
                && intents.remove(txid).is_some()
            {
                self.save(intents);
            }
        });
    }

    /// Keeps the records of `open` locks only: a lock taken by Platform or gone needs none.
    pub fn keep_only(&self, open: &[String]) {
        self.with(|state| {
            if let State::Read(intents) = state {
                let before = intents.len();
                intents.retain(|txid, _| open.contains(txid));
                if intents.len() != before {
                    self.save(intents);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evolite-intents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("fund-intents.json")
    }

    /// BUG-171 AC-T1: recorded when the lock is built, read back — also by a new run — and
    /// dropped once the lock is no longer open.
    #[test]
    fn an_intent_is_kept_until_its_lock_is_closed() {
        let path = folder("kept");
        let intents = FundIntents::new(path.clone());
        assert_eq!(intents.address("aa"), None);
        intents.put("aa", "tdash1kr95");
        intents.put("bb", "tdash1kpmx");
        let again = FundIntents::new(path.clone());
        assert_eq!(again.address("aa").as_deref(), Some("tdash1kr95"));
        again.keep_only(&["bb".to_owned()]);
        assert_eq!(again.address("aa"), None, "closed: dropped");
        let third = FundIntents::new(path);
        assert_eq!(third.address("aa"), None, "and from the file");
        assert_eq!(third.address("bb").as_deref(), Some("tdash1kpmx"));
    }

    /// BUG-171 AC-T1: a file that cannot be read is not written over, and gives no address.
    #[test]
    fn a_broken_file_is_left_alone() {
        let path = folder("broken");
        std::fs::write(&path, b"{ not json").unwrap();
        let intents = FundIntents::new(path.clone());
        assert_eq!(intents.address("aa"), None);
        intents.put("aa", "tdash1kr95");
        intents.keep_only(&[]);
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }
}
