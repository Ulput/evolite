//! The search's [`Cache`] on disk: `wallets/<network>/token-search.json`, one for all the wallets
//! of the network, as the contracts are public and the same for everyone (card, «Кэш»). Ids as
//! hex. A missing, damaged or older file is a fresh cache: the next pass reads the network again,
//! which costs time and traffic and nothing else. Written beside and renamed over
//! ([`evolite_local::json_file`]), so a cut leaves the old cache or the new one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::crawl::{Cache, Id, Known, Pass};

/// The file's format; another one is read as no file.
const FORMAT: u32 = 1;

/// The file of the network's search in data folder `data_dir`.
pub fn path(data_dir: &Path) -> PathBuf {
    evolite_local::wallets::network_dir(data_dir).join("token-search.json")
}

#[derive(Serialize, Deserialize)]
struct Stored<P> {
    format: u32,
    /// Every id seen, with the version last read (`None`: not read).
    ids: Vec<(String, Option<u32>)>,
    tokens: Vec<StoredKnown<P>>,
    to_read: Vec<String>,
    pass: StoredPass,
    passes: u32,
}

#[derive(Serialize, Deserialize)]
struct StoredKnown<P> {
    id: String,
    version: u32,
    tokens: P,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case")]
enum StoredPass {
    Idle,
    Ids { after: Option<String> },
    Versions { after: Option<String> },
    Contracts,
}

/// The cache kept at `path`, or a fresh one.
pub fn load<P: DeserializeOwned>(path: &Path) -> Cache<P> {
    evolite_local::json_file::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Stored<P>>(&bytes).ok())
        .and_then(from_stored)
        .unwrap_or_default()
}

/// Keep `cache` at `path`.
pub fn save<P: Serialize + Clone>(path: &Path, cache: &Cache<P>) -> Result<(), String> {
    evolite_local::json_file::save(path, &to_stored(cache))
}

fn to_stored<P: Clone>(cache: &Cache<P>) -> Stored<P> {
    let hex = |id: &Id| to_hex(id);
    Stored {
        format: FORMAT,
        ids: cache.ids.iter().map(|(id, v)| (hex(id), *v)).collect(),
        tokens: cache
            .tokens
            .iter()
            .map(|(id, k)| StoredKnown {
                id: hex(id),
                version: k.version,
                tokens: k.tokens.clone(),
            })
            .collect(),
        to_read: cache.to_read.iter().map(hex).collect(),
        pass: match &cache.pass {
            Pass::Idle => StoredPass::Idle,
            Pass::Ids { after } => StoredPass::Ids {
                after: after.as_ref().map(hex),
            },
            Pass::Versions { after } => StoredPass::Versions {
                after: after.as_ref().map(hex),
            },
            Pass::Contracts => StoredPass::Contracts,
        },
        passes: cache.passes,
    }
}

/// `None` when anything in it is not what [`to_stored`] writes: a cache half read could skip
/// contracts for good, a fresh one only reads them again.
fn from_stored<P>(stored: Stored<P>) -> Option<Cache<P>> {
    if stored.format != FORMAT {
        return None;
    }
    let ids =
        |list: Vec<String>| -> Option<BTreeSet<Id>> { list.iter().map(|s| from_hex(s)).collect() };
    let after = |a: Option<String>| -> Option<Option<Id>> {
        match a {
            None => Some(None),
            Some(s) => from_hex(&s).map(Some),
        }
    };
    let mut tokens = BTreeMap::new();
    for k in stored.tokens {
        tokens.insert(
            from_hex(&k.id)?,
            Known {
                version: k.version,
                tokens: k.tokens,
            },
        );
    }
    Some(Cache {
        ids: stored
            .ids
            .iter()
            .map(|(s, v)| from_hex(s).map(|id| (id, *v)))
            .collect::<Option<_>>()?,
        tokens,
        to_read: ids(stored.to_read)?,
        pass: match stored.pass {
            StoredPass::Idle => Pass::Idle,
            StoredPass::Ids { after: a } => Pass::Ids { after: after(a)? },
            StoredPass::Versions { after: a } => Pass::Versions { after: after(a)? },
            StoredPass::Contracts => Pass::Contracts,
        },
        passes: stored.passes,
    })
}

fn to_hex(id: &Id) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Id> {
    if text.len() != 64 {
        return None;
    }
    let mut id = [0u8; 32];
    for (i, byte) in id.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> Id {
        [n; 32]
    }

    fn file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "evolite-token-search-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("token-search.json")
    }

    fn cache() -> Cache<Vec<u16>> {
        Cache {
            ids: [(id(1), Some(1)), (id(2), Some(3)), (id(0xfe), None)].into(),
            tokens: [(
                id(2),
                Known {
                    version: 3,
                    tokens: vec![0, 1],
                },
            )]
            .into(),
            to_read: [id(0xfe)].into(),
            pass: Pass::Ids {
                after: Some(id(0xfe)),
            },
            passes: 4,
        }
    }

    #[test]
    fn a_pass_cut_short_goes_on_after_a_restart() {
        let path = file("restart");
        save(&path, &cache()).unwrap();
        assert_eq!(load::<Vec<u16>>(&path), cache());
        for pass in [
            Pass::Idle,
            Pass::Ids { after: None },
            Pass::Versions { after: Some(id(2)) },
            Pass::Contracts,
        ] {
            let c = Cache {
                pass: pass.clone(),
                ..cache()
            };
            save(&path, &c).unwrap();
            assert_eq!(load::<Vec<u16>>(&path).pass, pass);
        }
    }

    #[test]
    fn a_missing_damaged_or_other_file_is_a_fresh_cache() {
        let path = file("damaged");
        assert_eq!(load::<Vec<u16>>(&path), Cache::default(), "no file");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(load::<Vec<u16>>(&path), Cache::default(), "not json");
        save(&path, &cache()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let short_id = text.replacen(&to_hex(&id(1)), "0101", 1);
        std::fs::write(&path, short_id).unwrap();
        assert_eq!(
            load::<Vec<u16>>(&path),
            Cache::default(),
            "an id not 32 bytes"
        );
        let other = text.replacen("\"format\": 1", "\"format\": 2", 1);
        assert_ne!(other, text);
        std::fs::write(&path, other).unwrap();
        assert_eq!(load::<Vec<u16>>(&path), Cache::default(), "another format");
    }

    #[test]
    fn the_file_is_in_the_network_folder_not_a_wallet_folder() {
        let data = Path::new("data");
        let p = path(data);
        // One for every wallet of the network: deleting a wallet's folder leaves it.
        assert_eq!(
            p.parent(),
            Some(evolite_local::wallets::network_dir(data).as_path())
        );
        assert!(!p.starts_with(evolite_local::wallets::wallet_dir(data, "0000abcd")));
    }
}
