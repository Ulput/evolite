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
            password_each_send: false
        })
    )
}

/// Keeps the switch of wallet `wallet`.
pub fn save(root: &Path, wallet: &str, on: bool) -> Result<(), String> {
    json_file::save(
        &path(root, wallet),
        &Security {
            password_each_send: on,
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
