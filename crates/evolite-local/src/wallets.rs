//! The wallets on this computer: one folder per wallet in `wallets/<network>/<id>/`, and
//! `wallets/<network>/wallets.json`, the index — each wallet's name, colour, how it came, when it
//! was made and last opened, and which was open last. Nothing secret, no balances: the phrase is
//! in the wallet's own `wallet.json`, under its own password.
//!
//! The index never holds anything alone: it is checked against the folders every time it is
//! read — a folder with a `wallet.json` and no entry gets one ("Wallet 2"), an entry with no
//! folder is dropped. So a crash between writing a wallet and writing the index loses nothing.
//!
//! A data folder from before wallet folders kept its one wallet right in `wallets/<network>/`. It is moved
//! into a folder of its own by renames only (see [`migrate`]): nothing copied, nothing deleted,
//! and a move cut short is finished at the next start.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How a wallet came to this computer, for its line in Settings → Wallets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Origin {
    #[default]
    Created,
    Restored,
    DashPay,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The folder's name: 8 random hex digits, not the phrase's `WalletId`, so the name of the
    /// folder does not tie the wallet to its addresses.
    pub id: String,
    /// Shown on this computer only; never sent anywhere.
    pub name: String,
    /// One of the palette's wallet colours, by the order wallets were added.
    pub color: u8,
    #[serde(default)]
    pub origin: Origin,
    /// Unix seconds.
    pub created_at: i64,
    #[serde(default)]
    pub last_opened: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub wallets: Vec<Record>,
    /// The wallet open last: opened (its unlock screen) at the next start.
    #[serde(default)]
    pub last: Option<String>,
}

/// How many wallet colours the palette has.
pub const COLORS: u8 = 6;

impl Index {
    pub fn get(&self, id: &str) -> Option<&Record> {
        self.wallets.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Record> {
        self.wallets.iter_mut().find(|r| r.id == id)
    }

    /// The wallet to open: the one open last while it is there, else the first.
    pub fn current(&self) -> Option<&Record> {
        self.last
            .as_deref()
            .and_then(|id| self.get(id))
            .or_else(|| self.wallets.first())
    }

    /// A record for a wallet just added: no name yet and the next colour. The core knows no
    /// interface language: the app names it ("Main", "Wallet 2" in the language of the
    /// moment) with `RenameWallet`, and from then on it is a name like any the person gives.
    pub fn add(&mut self, id: &str, origin: Origin, created_at: i64) -> &Record {
        let n = self.wallets.len();
        self.wallets.push(Record {
            id: id.to_owned(),
            name: String::new(),
            color: (n % COLORS as usize) as u8,
            origin,
            created_at,
            last_opened: None,
        });
        &self.wallets[n]
    }
}

/// `wallets/<network>/` in the data folder: the index and a folder per wallet.
pub fn network_dir(data_dir: &Path) -> PathBuf {
    data_dir
        .join("wallets")
        .join(evolite_common::network::name())
}

/// Whether `id` is a wallet folder's name as [`new_id`] makes it: 8 lowercase hex digits. Any
/// other name in the index — `..`, a path, a drive — would put the wallet, and "Delete wallet"
/// with it, outside `wallets/<network>/`.
pub fn is_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The folder of wallet `id`.
pub fn wallet_dir(data_dir: &Path, id: &str) -> PathBuf {
    network_dir(data_dir).join(id)
}

fn index_path(data_dir: &Path) -> PathBuf {
    network_dir(data_dir).join("wallets.json")
}

/// A new wallet's folder name: 8 random hex digits, not one of the folders there already.
pub fn new_id(data_dir: &Path) -> String {
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes).expect("the OS random generator is available");
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        if !wallet_dir(data_dir, &id).exists() {
            return id;
        }
    }
}

/// Whether `dir` is a wallet's folder: it holds its vault.
fn has_wallet(dir: &Path) -> bool {
    dir.join("wallet.json").is_file()
}

/// Moves a data folder from before wallet folders to the layout with a folder per wallet; nothing to do
/// when it is already there. Renames only, each one at a time:
///
/// 1. `wallets/<network>` → `wallets/<network>.moving`;
/// 2. a new empty `wallets/<network>/`;
/// 3. `wallets/<network>.moving` → `wallets/<network>/<id>/`;
/// 4. the index ([`load`] writes it from the folders).
///
/// A run cut short after any step is finished by the next call: a `.moving` folder is taken
/// from step 2 on.
pub fn migrate(data_dir: &Path) -> std::io::Result<()> {
    let dir = network_dir(data_dir);
    let moving = dir.with_extension("moving");
    if !moving.exists() {
        // Step 1: only a wallet kept right in wallets/<network>/ is moved.
        if !has_wallet(&dir) {
            return Ok(());
        }
        std::fs::rename(&dir, &moving)?;
    }
    // Steps 2 and 3.
    super::folders::create(&dir)?;
    let target = dir.join(new_id(data_dir));
    std::fs::rename(&moving, &target)?;
    Ok(())
}

/// The index, checked against the folders (and written back when they differ): a wallet's
/// folder without an entry gets one, an entry without its folder is dropped.
pub fn load(data_dir: &Path) -> Index {
    let path = index_path(data_dir);
    let saved: Index = super::json_file::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let mut index = saved.clone();
    index
        .wallets
        .retain(|r| is_id(&r.id) && has_wallet(&wallet_dir(data_dir, &r.id)));
    let mut found: Vec<(String, i64)> = std::fs::read_dir(network_dir(data_dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir() && has_wallet(&e.path()))
        .filter_map(|e| {
            let id = e.file_name().to_str().filter(|id| is_id(id))?.to_owned();
            let created = std::fs::metadata(e.path().join("wallet.json"))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            Some((id, created))
        })
        .collect();
    oldest_first(&mut found);
    for (id, created) in found {
        if index.get(&id).is_none() {
            index.add(&id, Origin::Created, created);
        }
    }
    if index
        .last
        .as_deref()
        .is_some_and(|id| index.get(id).is_none())
    {
        index.last = None;
    }
    if index != saved {
        let _ = save(data_dir, &index);
    }
    index
}

/// Oldest first, and by id within one second: the time is in whole seconds, and the order
/// `read_dir` gives the rest differs from one file system to another.
fn oldest_first(found: &mut [(String, i64)]) {
    found.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
}

/// Writes the index beside and renames it over: the old one or the new one, never half.
pub fn save(data_dir: &Path, index: &Index) -> Result<(), String> {
    super::json_file::save(&index_path(data_dir), index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evolite-wallets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A data folder from before wallet folders: the wallet's files right in wallets/<network>/.
    fn old_layout(data: &Path) {
        let dir = network_dir(data);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("wallet.json"), b"{vault}").unwrap();
        std::fs::write(dir.join("platform-history.json"), b"[1]").unwrap();
    }

    fn only_wallet(data: &Path) -> PathBuf {
        let index = load(data);
        assert_eq!(index.wallets.len(), 1, "{index:?}");
        let dir = wallet_dir(data, &index.wallets[0].id);
        assert_eq!(std::fs::read(dir.join("wallet.json")).unwrap(), b"{vault}");
        assert_eq!(
            std::fs::read(dir.join("platform-history.json")).unwrap(),
            b"[1]"
        );
        dir
    }

    #[test]
    fn an_old_folder_moves_into_a_wallet_of_its_own() {
        let data = folder("move");
        old_layout(&data);
        migrate(&data).unwrap();
        only_wallet(&data);
        assert_eq!(load(&data).wallets[0].name, "", "the app names it");
        // Run again: nothing more to move.
        migrate(&data).unwrap();
        only_wallet(&data);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_move_cut_short_is_finished_at_the_next_start() {
        // Cut after step 1: only the .moving folder.
        let data = folder("cut1");
        old_layout(&data);
        let dir = network_dir(&data);
        std::fs::rename(&dir, dir.with_extension("moving")).unwrap();
        migrate(&data).unwrap();
        only_wallet(&data);
        let _ = std::fs::remove_dir_all(&data);

        // Cut after step 2: .moving and an empty wallets/<network>/.
        let data = folder("cut2");
        old_layout(&data);
        let dir = network_dir(&data);
        std::fs::rename(&dir, dir.with_extension("moving")).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        migrate(&data).unwrap();
        only_wallet(&data);
        let _ = std::fs::remove_dir_all(&data);

        // Cut after step 3: the wallet's folder, no index yet — the index comes from it.
        let data = folder("cut3");
        old_layout(&data);
        let dir = network_dir(&data);
        std::fs::rename(&dir, dir.with_extension("moving")).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::rename(dir.with_extension("moving"), dir.join("0badc0de")).unwrap();
        migrate(&data).unwrap();
        assert_eq!(only_wallet(&data), dir.join("0badc0de"));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn an_index_cannot_point_outside_the_wallets_folder() {
        let data = folder("escape");
        // A folder outside wallets/<network>/ with a vault in it, and an index naming it.
        let outside = data.join("Documents");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("wallet.json"), b"{}").unwrap();
        let ok = wallet_dir(&data, "aaaa0001");
        std::fs::create_dir_all(&ok).unwrap();
        std::fs::write(ok.join("wallet.json"), b"{}").unwrap();
        let named = |id: &str| Record {
            id: id.into(),
            name: String::new(),
            color: 0,
            origin: Origin::Created,
            created_at: 0,
            last_opened: None,
        };
        let planted = Index {
            wallets: vec![
                named("../../Documents"),
                named(&outside.display().to_string()),
                named("aaaa0001"),
            ],
            last: Some("../../Documents".into()),
        };
        save(&data, &planted).unwrap();
        // A folder with an odd name inside is no wallet either.
        let odd = network_dir(&data).join("Wallet backup");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(odd.join("wallet.json"), b"{}").unwrap();
        let index = load(&data);
        let ids: Vec<&str> = index.wallets.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["aaaa0001"]);
        assert_eq!(index.current().unwrap().id, "aaaa0001");
        assert!(is_id("0badc0de") && !is_id("0BADC0DE") && !is_id("..") && !is_id("aaaa000/"));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn wallets_of_one_second_go_by_id() {
        let mut found = [("bbbb0002", 5), ("cccc0003", 1), ("aaaa0001", 5)]
            .map(|(id, created)| (id.to_owned(), created));
        oldest_first(&mut found);
        let ids = found.each_ref().map(|(id, _)| id.as_str());
        assert_eq!(ids, ["cccc0003", "aaaa0001", "bbbb0002"]);
    }

    #[test]
    fn the_index_follows_the_folders() {
        let data = folder("index");
        let make = |id: &str| {
            let dir = wallet_dir(&data, id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("wallet.json"), b"{}").unwrap();
        };
        make("aaaa0001");
        make("bbbb0002");
        let mut index = load(&data);
        assert_eq!(index.wallets.len(), 2);
        index.get_mut("bbbb0002").unwrap().name = tr!("Savings").into();
        index.last = Some("bbbb0002".into());
        save(&data, &index).unwrap();
        // A deleted wallet's entry goes; a folder without its vault (an onboarding left
        // halfway) is no wallet.
        std::fs::remove_dir_all(wallet_dir(&data, "aaaa0001")).unwrap();
        std::fs::create_dir_all(wallet_dir(&data, "cccc0003")).unwrap();
        let index = load(&data);
        assert_eq!(index.wallets.len(), 1);
        assert_eq!(index.current().unwrap().name, tr!("Savings"));
        let _ = std::fs::remove_dir_all(&data);
    }
}
