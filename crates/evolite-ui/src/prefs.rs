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
    /// FEAT-0041 R-13: the empty restored wallet's banner was closed, or money came: not again.
    pub empty_hint_closed: Option<bool>,
    /// The interface language (`ru`, `en`, `zh-Hans`…); none: the system's.
    pub lang: Option<String>,
    /// FEAT-0040: the wallet's fee rate for a Core payment, duffs per kB; none: «Normal» (R-12).
    pub fee_rate: Option<u64>,
    /// BUG-286, BUG-298: the identities (base58) whose «keys other wallets need» warning got
    /// «Not now», with what was missing then (r1 Should 1): the warning is hidden on Contacts
    /// and Identity while nothing else is missing; the keys screen still says it.
    pub keys_not_now: NotNow,
}

/// «Not now» on the missing keys' warning (BUG-286 r1): by identity, the names of what was
/// missing when it was said (`screens::identity::keys::missing_names`).
pub type NotNow = BTreeMap<String, BTreeSet<String>>;

/// Whether «Not now» for `id` covers `missing` (by name): everything missing now was missing
/// when it was said. Something new missing shows the warning again.
pub fn not_now_covers(not_now: &NotNow, id: &str, missing: &BTreeSet<String>) -> bool {
    not_now.get(id).is_some_and(|said| missing.is_subset(said))
}

/// Joins `from` into `into`, identity by identity.
pub fn join_not_now(into: &mut NotNow, from: &NotNow) {
    for (id, names) in from {
        into.entry(id.clone())
            .or_default()
            .extend(names.iter().cloned());
    }
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
pub const APP_TOGGLES: [&str; 7] = [
    "fiat-rate",
    "hide-amounts",
    "own-node",
    "proxy",
    // Р1: the update check is the app's, not a wallet's.
    "update-check",
    // FEAT-0040…0046 (analyst-149): «an advanced user» is a person, not a wallet.
    "advanced",
    // FEAT-0038 W0: an author is a person too; the Workshop shows for every wallet.
    "workshop",
];

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
        // FEAT-0040 (R-12): the wallet's, never the app's.
        fee_rate: prefs.fee_rate,
        empty_hint_closed: prefs.empty_hint_closed,
        keys_not_now: prefs.keys_not_now.clone(),
        ..Prefs::default()
    };
    (app, wallet)
}

/// FEAT-0045 (review r1, Must 1): the wallet's half of `from` (a backup's) fills what `now` does
/// not have — a setting made here stays, so an old copy never turns a weaker one back on
/// (`lock_after`, a switch such as the clipboard's). The trusted issuers are both sides', the
/// phrase's last check the later one; the app's half of `now` is not touched.
pub fn take_wallet_half(now: &mut Prefs, from: &Prefs) {
    let (_, from) = split(from);
    for (key, on) in from.toggles {
        now.toggles.entry(key).or_insert(on);
    }
    now.trusted_issuers.extend(from.trusted_issuers);
    join_not_now(&mut now.keys_not_now, &from.keys_not_now);
    now.lock_after = now.lock_after.or(from.lock_after);
    now.identity = now.identity.take().or(from.identity);
    now.remind_snoozed = now.remind_snoozed.or(from.remind_snoozed);
    now.fee_rate = now.fee_rate.or(from.fee_rate);
    now.empty_hint_closed = now.empty_hint_closed.or(from.empty_hint_closed);
    now.phrase_checked = now.phrase_checked.max(from.phrase_checked);
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

/// The app's settings, from the common folder `common`, the same for both networks (FEAT-0037).
/// With none there yet, the app's part of a network folder's file is taken there once — this
/// network's (`network_dir`), else the other's (`other_dir`): a first start of this build on
/// mainnet keeps what testnet had (review r1, Should 2). That file is left as it was, for an
/// older build. With neither, nothing is written: the common file comes with the first change.
/// With `--data-dir` the common folder is the network's (`common == network_dir`): its file is
/// read whole, as before.
pub fn app_prefs(common: &Path, network_dir: &Path, other_dir: Option<&Path>) -> Prefs {
    let file = path(common);
    if common == network_dir || file.exists() {
        return load(&file);
    }
    let Some(old) = std::iter::once(network_dir)
        .chain(other_dir)
        .map(path)
        .find(|p| p.exists())
    else {
        return Prefs::default();
    };
    let (app, _) = split(&load(&old));
    if let Err(e) = save(&file, &app) {
        eprintln!("settings not moved to the common folder: {e}");
    }
    app
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
            toggles: [("new-address".to_owned(), true)].into(),
            trusted_issuers: ["neon-forge".to_owned()].into(),
            identity: Some("H5ttoybh42z3HukXBjA4Sb7qudXvRAfziTREdhPposwB".into()),
            lang: Some("en".into()),
            phrase_checked: Some(1_793_404_800),
            number_style: Some(2),
            fiat_currency: Some(1),
            remind_snoozed: Some(1_794_009_600),
            fee_rate: Some(2_000),
            empty_hint_closed: Some(true),
            keys_not_now: [(
                "H5ttoybh42z3HukXBjA4Sb7qudXvRAfziTREdhPposwB".to_owned(),
                ["dashpay-decryption".to_owned()].into(),
            )]
            .into(),
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path), prefs);
        // FEAT-0040 (R-12): the fee rate is the wallet's, never the app's.
        let (app, wallet) = split(&prefs);
        assert_eq!((app.fee_rate, wallet.fee_rate), (None, Some(2_000)));
        // BUG-286, BUG-298: «Not now» is the wallet's, by identity.
        assert!(app.keys_not_now.is_empty());
        assert_eq!(wallet.keys_not_now, prefs.keys_not_now);
    }

    /// BUG-286 r1 Should 1: «Not now» covers what was missing then; something new missing
    /// (the sign-in key after the decryption one) is not covered. Mutant: any entry covers.
    #[test]
    fn not_now_covers_only_what_was_missing_then() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        let mut not_now = NotNow::new();
        join_not_now(
            &mut not_now,
            &[("id".to_owned(), names(&["dashpay-decryption"]))].into(),
        );
        assert!(not_now_covers(
            &not_now,
            "id",
            &names(&["dashpay-decryption"])
        ));
        assert!(!not_now_covers(
            &not_now,
            "id",
            &names(&["dashpay-decryption", "sign-in"])
        ));
        assert!(!not_now_covers(
            &not_now,
            "other",
            &names(&["dashpay-decryption"])
        ));
        join_not_now(
            &mut not_now,
            &[("id".to_owned(), names(&["sign-in"]))].into(),
        );
        assert!(not_now_covers(
            &not_now,
            "id",
            &names(&["dashpay-decryption", "sign-in"])
        ));
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
                ("new-address".to_owned(), true),
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
        assert_eq!(wallet.toggles, [("new-address".to_owned(), true)].into());
        assert!(wallet.theme.is_none() && wallet.lang.is_none() && wallet.unit.is_none());
        assert_eq!(wallet.phrase_checked, Some(1_793_404_800));
        // FEAT-0041 R-13: the empty wallet's banner closed is the wallet's.
        let (app, wallet) = split(&Prefs {
            empty_hint_closed: Some(true),
            ..Prefs::default()
        });
        assert_eq!(
            (app.empty_hint_closed, wallet.empty_hint_closed),
            (None, Some(true))
        );
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

    /// FEAT-0037 AC-T6д: the app's settings are in the common folder, the same for both networks.
    /// The first run of this build takes the app's part of the network folder's old file there,
    /// and leaves that file as it was (an older build still reads it).
    #[test]
    fn the_app_settings_move_to_the_common_folder_once() {
        let root = std::env::temp_dir().join(format!("evolite-prefs-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (common, testnet, mainnet) = (
            root.join("evolite-app"),
            root.join("evolite"),
            root.join("evolite-mainnet"),
        );
        let old = Prefs {
            theme: Some("light".into()),
            lang: Some("en".into()),
            lock_after: Some(5),
            ..Prefs::default()
        };
        save(&path(&testnet), &old).unwrap();
        let before = std::fs::read(path(&testnet)).unwrap();
        let app = app_prefs(&common, &testnet, Some(&mainnet));
        assert_eq!(
            (app.theme.as_deref(), app.lang.as_deref()),
            (Some("light"), Some("en"))
        );
        assert!(
            app.lock_after.is_none(),
            "the wallet's part stays with the wallet"
        );
        assert!(path(&common).exists(), "kept in the common folder");
        assert_eq!(
            std::fs::read(path(&testnet)).unwrap(),
            before,
            "the old file as it was"
        );
        // The other network reads the same: the theme and the language do not jump.
        let dark = Prefs {
            theme: Some("dark".into()),
            ..Prefs::default()
        };
        save(&path(&mainnet), &dark).unwrap();
        assert_eq!(
            app_prefs(&common, &mainnet, Some(&testnet))
                .theme
                .as_deref(),
            Some("light")
        );
        // With --data-dir the common folder is that folder: the file is the one it had.
        assert_eq!(
            app_prefs(&testnet, &testnet, None).lock_after,
            Some(5),
            "read whole, as before"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review r1, Should 2: the first start of this build on mainnet, with the settings in the
    /// testnet folder only, keeps them (the theme does not go back to the default); with no
    /// settings anywhere nothing is written.
    #[test]
    fn a_first_start_on_mainnet_keeps_testnet_s_settings() {
        let root = std::env::temp_dir().join(format!(
            "evolite-prefs-first-mainnet-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (common, testnet, mainnet) = (
            root.join("evolite-app"),
            root.join("evolite"),
            root.join("evolite-mainnet"),
        );
        assert_eq!(
            app_prefs(&common, &mainnet, Some(&testnet)),
            Prefs::default()
        );
        assert!(!path(&common).exists(), "nothing found: nothing written");
        let old = Prefs {
            theme: Some("light".into()),
            ..Prefs::default()
        };
        save(&path(&testnet), &old).unwrap();
        assert_eq!(
            app_prefs(&common, &mainnet, Some(&testnet))
                .theme
                .as_deref(),
            Some("light"),
            "testnet's theme"
        );
        assert!(path(&common).exists(), "kept in the common folder");
        let _ = std::fs::remove_dir_all(&root);
    }
    /// FEAT-0045 review r1, Must 1: a backup's settings fill only what is not set here — one
    /// made here stays (a switch, the lock's time, the fee); in a wallet with none, the file's
    /// come; the trusted issuers are both sides', the phrase's check the later one; the app's
    /// own settings are not taken from a backup.
    #[test]
    fn a_backups_settings_never_win_over_the_ones_here() {
        let file = Prefs {
            lock_after: Some(4),
            fee_rate: Some(2_000),
            identity: Some("IdFile".into()),
            remind_snoozed: Some(50),
            phrase_checked: Some(10),
            toggles: [
                ("clipboard-wipe".to_owned(), false),
                ("only-file".to_owned(), true),
            ]
            .into(),
            trusted_issuers: ["Ifile".to_owned()].into(),
            theme: Some("light".into()),
            ..Prefs::default()
        };
        let mut here = Prefs {
            lock_after: Some(1),
            fee_rate: Some(1_000),
            identity: Some("IdHere".into()),
            phrase_checked: Some(20),
            toggles: [("clipboard-wipe".to_owned(), true)].into(),
            trusted_issuers: ["Ihere".to_owned()].into(),
            ..Prefs::default()
        };
        take_wallet_half(&mut here, &file);
        assert_eq!(here.lock_after, Some(1));
        assert_eq!(here.fee_rate, Some(1_000));
        assert_eq!(here.identity.as_deref(), Some("IdHere"));
        assert!(
            here.toggles["clipboard-wipe"],
            "the stronger one here stays"
        );
        assert!(here.toggles["only-file"], "one not set here comes");
        assert_eq!(here.remind_snoozed, Some(50));
        assert_eq!(here.phrase_checked, Some(20));
        assert_eq!(here.trusted_issuers.len(), 2);
        assert_eq!(here.theme, None, "the app's half is not the backup's");
        // An empty wallet takes the file's wallet half.
        let mut empty = Prefs::default();
        take_wallet_half(&mut empty, &file);
        assert_eq!(empty.lock_after, Some(4));
        assert_eq!(empty.fee_rate, Some(2_000));
        assert!(!empty.toggles["clipboard-wipe"]);
    }
}
