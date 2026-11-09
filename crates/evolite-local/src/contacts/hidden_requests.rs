//! Senders whose contact requests the person hid ("Hide"). platform-wallet keeps them in its
//! store, which "Sync again" removes, and nothing on Platform says who was hidden;
//! without this file the hidden requests came back after a resync. Public data only
//! (identity ids). A missing or unreadable file is none hidden.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct HiddenRequests {
    path: PathBuf,
    /// Sender identities (base58); read once, kept up to date.
    hidden: Mutex<Option<BTreeSet<String>>>,
}

impl HiddenRequests {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            hidden: Mutex::new(None),
        }
    }

    pub fn contains(&self, sender: &str) -> bool {
        let mut hidden = self.hidden.lock().unwrap_or_else(|e| e.into_inner());
        hidden.get_or_insert_with(|| self.load()).contains(sender)
    }

    /// `sender`'s requests are hidden from now on: once the file has it, so a hide that did
    /// not save does not hide either. Written beside, flushed and renamed over.
    pub fn add(&self, sender: &str) -> std::io::Result<()> {
        let mut hidden = self.hidden.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = hidden.get_or_insert_with(|| self.load()).clone();
        if all.insert(sender.to_owned()) {
            crate::json_file::save(&self.path, &all).map_err(std::io::Error::other)?;
            *hidden = Some(all);
        }
        Ok(())
    }

    fn load(&self) -> BTreeSet<String> {
        crate::json_file::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_senders_stay_hidden_across_loads() {
        let dir = std::env::temp_dir().join(format!("evolite-hidden-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("hidden-requests.json");
        let first = HiddenRequests::new(path.clone());
        assert!(!first.contains("A"));
        first.add("A").unwrap();
        first.add("A").unwrap();
        assert!(first.contains("A"));
        // The next run of the app, after a resync removed the store.
        let next = HiddenRequests::new(path);
        assert!(next.contains("A") && !next.contains("B"));
    }

    #[test]
    fn a_hide_that_did_not_save_does_not_hide() {
        let dir = std::env::temp_dir().join(format!("evolite-hidden-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A folder where the file should be: the rename over it fails.
        let path = dir.join("hidden-requests.json");
        std::fs::create_dir_all(&path).unwrap();
        let hidden = HiddenRequests::new(path);
        assert!(hidden.add("A").is_err());
        assert!(
            !hidden.contains("A"),
            "the screen said it failed: the request stays"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
