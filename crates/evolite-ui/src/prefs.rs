//! What the person chose in Settings, kept between runs: theme, auto-lock, the amount unit, NFT
//! animations, the switches and the trusted NFT issuers. Nothing secret. Written whole to a
//! temporary file and renamed, so a crash leaves the old file or the new one. The demo keeps
//! none: its screens stay as in Figma.
//!
//! the app's own choices (language, theme, unit, number format, the rate, NFT motion,
//! the Core node) are in `settings.json` of the data folder, the same for every wallet; each
//! wallet's (auto-lock, its switches, trusted issuers, the identity chosen, the phrase check)
//! in `settings.json` of its own folder. A file from before holds both: its wallet part
//! is taken by the wallet that has no file of its own yet, and goes from the app's file at the
//! next write ([`split`]).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// "dark", "light" or "system".
    pub theme: Option<String>,
    /// Position in Settings → Security → "Lock the wallet".
    pub lock_after: Option<usize>,
    pub unit: Option<usize>,
    pub nft_motion: Option<usize>,
    pub toggles: BTreeMap<String, bool>,
    pub trusted_issuers: BTreeSet<String>,
    /// The identity (base58) chosen on the Identity screen when the wallet has several.
    pub identity: Option<String>,
    /// When "Check the phrase" last went right (unix seconds).
    pub phrase_checked: Option<i64>,
    /// Settings → "Number format": 0 as the language writes them, 1 "1 234,56", 2 "1,234.56".
    pub number_style: Option<usize>,
    /// Settings → "Fiat rate": 0 USD, 1 EUR, 2 RUB.
    pub fiat_currency: Option<usize>,
    /// "Remind me later" on the phrase-check reminder: not before this (unix seconds).
    pub remind_snoozed: Option<i64>,
    /// The interface language (`ru`, `en`, `zh-Hans`…); none: the system's.
    pub lang: Option<String>,
}

/// Where the app's settings in `data_dir` live.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("settings.json")
}

/// Where wallet `id`'s own settings live: beside its vault (the core's
/// `wallets/<network>/<id>/`), so they go with a deleted wallet.
pub fn wallet_path(data_dir: &Path, id: &str) -> PathBuf {
    data_dir
        .join("wallets")
        .join(evolite_core::network::name())
        .join(id)
        .join("settings.json")
}

/// Switches that are the app's, not a wallet's: how amounts look and how evolite connects.
pub const APP_TOGGLES: [&str; 4] = ["fiat-rate", "hide-amounts", "own-node", "proxy"];

/// The app's part and the wallet's part of `prefs`.
pub fn split(prefs: &Prefs) -> (Prefs, Prefs) {
    let (app_toggles, wallet_toggles) = prefs
        .toggles
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .partition(|(k, _)| APP_TOGGLES.contains(&k.as_str()));
    let app = Prefs {
        theme: prefs.theme.clone(),
        unit: prefs.unit,
        nft_motion: prefs.nft_motion,
        number_style: prefs.number_style,
        fiat_currency: prefs.fiat_currency,
        lang: prefs.lang.clone(),
        toggles: app_toggles,
        ..Prefs::default()
    };
    let wallet = Prefs {
        lock_after: prefs.lock_after,
        toggles: wallet_toggles,
        trusted_issuers: prefs.trusted_issuers.clone(),
        identity: prefs.identity.clone(),
        phrase_checked: prefs.phrase_checked,
        remind_snoozed: prefs.remind_snoozed,
        ..Prefs::default()
    };
    (app, wallet)
}

/// A wallet's settings: its own file; without one, the wallet part of a file from before
/// (`app`), if that is what `app` is; else none.
pub fn load_wallet(data_dir: &Path, id: &str, app: &Prefs) -> Option<Prefs> {
    let path = wallet_path(data_dir, id);
    if path.exists() {
        return Some(load(&path));
    }
    let (_, old) = split(app);
    (old != Prefs::default()).then_some(old)
}

/// The saved settings; none (all defaults) when there is no file yet or it does not read —
/// a broken file never stops the wallet from opening. A file over 1 MB (settings take a few
/// hundred bytes) is not read into memory.
pub fn load(path: &Path) -> Prefs {
    use std::io::Read;
    const MAX: u64 = 1 << 20;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX + 1).read_to_end(&mut bytes))
        .ok()
        .filter(|&n| n as u64 <= MAX)
        .and_then(|_| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, prefs: &Prefs) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(prefs).map_err(std::io::Error::other)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evolite-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn what_is_saved_comes_back() {
        let path = path(&dir("round"));
        let prefs = Prefs {
            theme: Some("light".into()),
            lock_after: Some(0),
            unit: Some(2),
            nft_motion: Some(2),
            toggles: [("password-each-send".to_owned(), true)].into(),
            trusted_issuers: ["neon-forge".to_owned()].into(),
            identity: Some("H5ttoybh42z3HukXBjA4Sb7qudXvRAfziTREdhPposwB".into()),
            lang: Some("en".into()),
            phrase_checked: Some(1_793_404_800),
            number_style: Some(2),
            fiat_currency: Some(1),
            remind_snoozed: Some(1_794_009_600),
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path), prefs);
    }

    #[test]
    fn no_file_or_a_broken_one_means_defaults() {
        let dir = dir("broken");
        let path = path(&dir);
        assert_eq!(load(&path), Prefs::default());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(load(&path), Prefs::default());
    }

    fn old_file() -> Prefs {
        Prefs {
            theme: Some("light".into()),
            lang: Some("en".into()),
            unit: Some(1),
            lock_after: Some(0),
            toggles: [
                ("hide-amounts".to_owned(), true),
                ("password-each-send".to_owned(), true),
            ]
            .into(),
            trusted_issuers: ["neon-forge".to_owned()].into(),
            identity: Some("H5ttoybh42z3HukXBjA4Sb7qudXvRAfziTREdhPposwB".into()),
            phrase_checked: Some(1_793_404_800),
            ..Prefs::default()
        }
    }

    /// the app's choices and a wallet's go apart, and come back whole.
    #[test]
    fn the_app_and_a_wallet_each_keep_their_own() {
        let (app, wallet) = split(&old_file());
        assert_eq!(app.theme.as_deref(), Some("light"));
        assert_eq!(app.toggles, [("hide-amounts".to_owned(), true)].into());
        assert!(app.identity.is_none() && app.phrase_checked.is_none());
        assert!(app.lock_after.is_none() && app.trusted_issuers.is_empty());
        assert_eq!(wallet.lock_after, Some(0));
        assert_eq!(
            wallet.toggles,
            [("password-each-send".to_owned(), true)].into()
        );
        assert!(wallet.theme.is_none() && wallet.lang.is_none() && wallet.unit.is_none());
        assert_eq!(wallet.phrase_checked, Some(1_793_404_800));
    }

    /// A file from before is the first wallet's until it has its own; then each wallet
    /// reads only its own file, and a wallet without one gets nothing of another's.
    #[test]
    fn an_old_file_goes_to_its_wallet_only() {
        let data = dir("split");
        let old = old_file();
        let taken = load_wallet(&data, "aaaa0001", &old).unwrap();
        assert_eq!(taken.identity, old.identity);
        let (app, wallet) = split(&old);
        save(&path(&data), &app).unwrap();
        save(&wallet_path(&data, "aaaa0001"), &wallet).unwrap();
        let app = load(&path(&data));
        assert_eq!(load_wallet(&data, "aaaa0001", &app), Some(wallet));
        assert_eq!(load_wallet(&data, "bbbb0002", &app), None);
        let _ = std::fs::remove_dir_all(&data);
    }
}
