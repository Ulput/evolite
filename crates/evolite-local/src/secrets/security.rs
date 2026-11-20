//! "Password for every send" (BUG-42, R-27): the wallet's own switch, kept by the core beside
//! its vault (`wallets/<network>/<id>/security.json`), so that the interface cannot turn it off
//! without the password. On by default, and on whenever the file is missing or damaged. The
//! interface's old toggle (`toggles["password-each-send"]` in a `settings.json`) is not read:
//! a file it can write must not turn the check off (review r1, must 1); a wallet that had it off
//! asks again until it is turned off here, with the password.

use crate::json_file;
use crate::wallets;
use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize)]
struct Security {
    password_each_send: bool,
    /// FEAT-0004 FR-7: the threshold of a large spend, duffs; `null`: off. A file without it
    /// (made before) gives the default, never «off».
    #[serde(rename = "largeSpendDuffs", default = "default_large")]
    large_spend_duffs: Option<evolite_api::Duffs>,
}

fn default_large() -> Option<evolite_api::Duffs> {
    Some(evolite_api::confirm::LARGE_SPEND_DEFAULT)
}

/// The wallet's file as it is, when it reads.
fn read(root: &Path, wallet: &str) -> Option<Security> {
    let bytes = json_file::read(&path(root, wallet)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn path(root: &Path, wallet: &str) -> PathBuf {
    wallets::wallet_dir(root, wallet).join("security.json")
}

/// Wallet `wallet`'s switch: off only when its own file says off.
pub fn load(root: &Path, wallet: &str) -> bool {
    let Ok(bytes) = json_file::read(&path(root, wallet)) else {
        return true;
    };
    // A damaged file asks for the password: the safe side.
    !matches!(
        serde_json::from_slice::<Security>(&bytes),
        Ok(Security {
            password_each_send: false,
            ..
        })
    )
}

/// Keeps the switch of wallet `wallet` (its threshold kept as it is).
pub fn save(root: &Path, wallet: &str, on: bool) -> Result<(), String> {
    json_file::save(
        &path(root, wallet),
        &Security {
            password_each_send: on,
            large_spend_duffs: load_large(root, wallet),
        },
    )
}

/// FEAT-0004 FR-7: wallet `wallet`'s threshold of a large spend; the default (1 DASH) when the
/// file is missing or does not read — the safe side, never «off».
pub fn load_large(root: &Path, wallet: &str) -> Option<evolite_api::Duffs> {
    match read(root, wallet) {
        Some(security) => security.large_spend_duffs,
        None => default_large(),
    }
}

/// FEAT-0004 FR-7: keeps wallet `wallet`'s threshold (`None`: off), its switch kept as it is.
pub fn save_large(
    root: &Path,
    wallet: &str,
    duffs: Option<evolite_api::Duffs>,
) -> Result<(), String> {
    json_file::save(
        &path(root, wallet),
        &Security {
            password_each_send: load(root, wallet),
            large_spend_duffs: duffs,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("evolite-security-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// FEAT-0004 AC-T6: the threshold is 1 DASH without a file, in an older file and in a
    /// damaged one; kept, and kept apart from the switch.
    #[test]
    fn the_threshold_is_one_dash_unless_set_and_survives_the_switch() {
        let root = root("large");
        let one = Some(evolite_api::confirm::LARGE_SPEND_DEFAULT);
        assert_eq!(load_large(&root, "w3"), one, "no file");
        let dir = wallets::wallet_dir(&root, "w3");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("security.json"), r#"{"password_each_send":false}"#).unwrap();
        assert_eq!(load_large(&root, "w3"), one, "a file from before");
        std::fs::write(dir.join("security.json"), b"{not json").unwrap();
        assert_eq!(load_large(&root, "w3"), one, "a damaged file: not off");
        save_large(&root, "w3", Some(500_000_000)).unwrap();
        assert_eq!(load_large(&root, "w3"), Some(500_000_000));
        save(&root, "w3", false).unwrap();
        assert_eq!(
            load_large(&root, "w3"),
            Some(500_000_000),
            "the switch keeps it"
        );
        save_large(&root, "w3", None).unwrap();
        assert_eq!(load_large(&root, "w3"), None, "off");
        assert!(!load(&root, "w3"), "and the switch kept");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn is_on_by_default_and_kept() {
        let root = root("default");
        assert!(load(&root, "w1"), "no file, no wallet folder: on");
        std::fs::create_dir_all(wallets::wallet_dir(&root, "w1")).unwrap();
        save(&root, "w1", false).unwrap();
        assert!(!load(&root, "w1"));
        save(&root, "w1", true).unwrap();
        assert!(load(&root, "w1"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    /// Review r1, must 1: with no `security.json`, a stale «off» the interface left in the
    /// wallet's or the app's `settings.json` does not turn the password off, now or later.
    fn a_missing_file_asks_for_the_password_whatever_the_settings_say() {
        let root = root("stale");
        let dir = wallets::wallet_dir(&root, "w2");
        std::fs::create_dir_all(&dir).unwrap();
        let stale = r#"{"toggles":{"password-each-send":false}}"#;
        std::fs::write(dir.join("settings.json"), stale).unwrap();
        std::fs::write(root.join("settings.json"), stale).unwrap();
        assert!(
            load(&root, "w2"),
            "no file of its own: the password is asked"
        );
        assert!(load(&root, "w2"), "and still asked the next time");
        assert!(!path(&root, "w2").exists(), "nothing is written for it");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_damaged_file_asks_for_the_password() {
        let root = root("damaged");
        let file = path(&root, "w3");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"{not json").unwrap();
        assert!(load(&root, "w3"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
