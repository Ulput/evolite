//! The DPNS names asked for identities still being made, by identity slot. platform-wallet keeps
//! the asset lock of a cut-short registration, but not the name it was for: this file does, so
//! the identity can be finished with its name. Written before the lock, removed once the name is
//! in DPNS. Public data only (a slot number and a name).

use std::collections::BTreeMap;
use std::path::PathBuf;

pub struct PendingNames {
    path: PathBuf,
}

impl PendingNames {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Slot → name. A missing or unreadable file is no names.
    pub fn load(&self) -> BTreeMap<u32, String> {
        crate::json_file::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn set(&self, index: u32, label: &str) -> Result<(), String> {
        let mut names = self.load();
        names.insert(index, label.to_owned());
        self.save(&names)
    }

    pub fn remove(&self, index: u32) -> Result<(), String> {
        let mut names = self.load();
        if names.remove(&index).is_some() {
            self.save(&names)?;
        }
        Ok(())
    }

    fn save(&self, names: &BTreeMap<u32, String>) -> Result<(), String> {
        crate::json_file::save(&self.path, names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_names_by_slot_across_loads() {
        let dir = std::env::temp_dir().join(format!("evolite-pending-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pending = PendingNames::new(dir.join("pending-names.json"));
        assert!(pending.load().is_empty(), "no file: no names");
        pending.set(3, "evolite-2026").unwrap();
        pending.set(4, "other").unwrap();
        pending.remove(4).unwrap();
        let again = PendingNames::new(dir.join("pending-names.json"));
        assert_eq!(
            again.load(),
            BTreeMap::from([(3, "evolite-2026".to_string())])
        );
    }
}
