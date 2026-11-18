//! Contacts' DPNS names kept on this computer (SYNC-4): shown at once after an unlock, before the
//! network answers. Only for the first paint: every name is still read from the network once a
//! run, and what it says is kept here. Public data (identity ids and names). A file that does not
//! read is set aside (`json_file::Store`); then the names come from the network only.

use crate::json_file::Store;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One contact's name as last read, and when (Unix seconds).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Kept {
    /// `None`: the contact has no name.
    pub name: Option<String>,
    pub at: i64,
}

pub struct ContactNames {
    store: Store<BTreeMap<String, Kept>>,
}

impl ContactNames {
    pub fn new(path: PathBuf) -> Self {
        Self {
            store: Store::new(path),
        }
    }

    /// Every name kept, by the contact's identity (base58).
    pub fn all(&self) -> BTreeMap<String, Option<String>> {
        self.store.read(|all| {
            all.iter()
                .map(|(id, kept)| (id.clone(), kept.name.clone()))
                .collect()
        })
    }

    /// Keeps what the network said of `id`'s name at `now`; written only when the name changed or
    /// was not kept.
    pub fn keep(&self, id: &str, name: Option<String>, now: i64) {
        let _ = self.store.update(|all| {
            let changed = all.get(id).is_none_or(|kept| kept.name != name);
            if changed {
                all.insert(id.to_owned(), Kept { name, at: now });
            }
            ((), changed)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "evolite-contact-names-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// SYNC-4: a name read is there after a restart; one the network now says otherwise is
    /// replaced; the same name again writes nothing.
    #[test]
    fn names_read_are_kept_and_follow_the_network() {
        let dir = dir("kept");
        let path = dir.join("contact-names.json");
        let names = ContactNames::new(path.clone());
        assert!(names.all().is_empty());
        names.keep("ID1", Some("alice".into()), 10);
        names.keep("ID2", None, 10);
        let back = ContactNames::new(path.clone());
        assert_eq!(back.all().get("ID1"), Some(&Some("alice".to_owned())));
        assert_eq!(back.all().get("ID2"), Some(&None));
        let before = std::fs::read(&path).unwrap();
        back.keep("ID1", Some("alice".into()), 99);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "the same: nothing written"
        );
        back.keep("ID1", Some("alice2".into()), 99);
        assert_eq!(
            ContactNames::new(path).all().get("ID1"),
            Some(&Some("alice2".to_owned()))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
