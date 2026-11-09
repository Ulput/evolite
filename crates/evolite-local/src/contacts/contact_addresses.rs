//! The addresses of contacts' keys this wallet has paid to, by contact. DIP-15 wants every
//! payment to a contact on a new address; platform-wallet marks the address it takes as used,
//! but after a restart the contact's account is built again from its request and comes back
//! with no used addresses (our SPV does not watch the contact's addresses, so nothing on chain
//! marks them either), and the next payment would go to an address already paid. This
//! file remembers them; before a payment they are marked used again. Public data only (identity
//! ids and addresses). A missing or unreadable file is none remembered.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct ContactAddresses {
    path: PathBuf,
    /// Contact identity (base58) → addresses paid; read once, kept up to date.
    paid: Mutex<Option<BTreeMap<String, Vec<String>>>>,
}

impl ContactAddresses {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            paid: Mutex::new(None),
        }
    }

    /// The addresses of `contact`'s key already paid to.
    pub fn paid(&self, contact: &str) -> Vec<String> {
        let mut paid = self.paid.lock().unwrap_or_else(|e| e.into_inner());
        paid.get_or_insert_with(|| self.load())
            .get(contact)
            .cloned()
            .unwrap_or_default()
    }

    /// `address` of `contact`'s key was paid (or may have been: the network gave no clear
    /// answer). Written beside and renamed over; a write that fails is retried with the next.
    pub fn add(&self, contact: &str, address: &str) {
        let mut paid = self.paid.lock().unwrap_or_else(|e| e.into_inner());
        let all = paid.get_or_insert_with(|| self.load());
        let list = all.entry(contact.to_owned()).or_default();
        if !list.iter().any(|a| a == address) {
            list.push(address.to_owned());
        }
        let _ = self.save(all);
    }

    fn load(&self) -> BTreeMap<String, Vec<String>> {
        crate::json_file::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, paid: &BTreeMap<String, Vec<String>>) -> Result<(), String> {
        crate::json_file::save(&self.path, paid)
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
}
