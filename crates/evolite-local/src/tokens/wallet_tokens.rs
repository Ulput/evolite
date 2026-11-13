//! The wallet's tokens (FEAT-0019 АРХ-5): `tokens.json` in the wallet's folder. Which tokens the
//! wallet follows, how each came in (found by the search or added by hand), the proved contract
//! of each so the provider can check a token's transition after a restart without the network
//! (АРХ-2, AC-T2), and the "token × identity" pairs the person hid. Public data only. A missing or
//! damaged file is no tokens: the search finds the found ones again and the person adds the rest.
//! Not opened on mainnet (R-9).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;

/// How a token came into the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The wallet's search found a balance on one of its identities.
    Found,
    /// Added by its id or its keyword.
    ByHand,
}

/// A token the wallet follows.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tracked {
    /// Base58, 32 bytes.
    pub token_id: String,
    /// Base58 id of its contract.
    pub contract_id: String,
    pub position: u16,
    pub origin: Origin,
    /// The contract as last read with a proof, in Platform's own serialization, hex; `None`
    /// until read. Handed to the provider at start.
    #[serde(default)]
    pub contract: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct File {
    #[serde(default)]
    tokens: Vec<Tracked>,
    /// `(token_id, identity_id)`, both base58.
    #[serde(default)]
    hidden: BTreeSet<(String, String)>,
}

pub struct WalletTokens {
    path: PathBuf,
    /// Read once, kept up to date; changed only once the file has the change.
    file: Mutex<Option<File>>,
}

impl WalletTokens {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            file: Mutex::new(None),
        }
    }

    /// The tokens followed, in the order they came in.
    pub fn all(&self) -> Vec<Tracked> {
        self.read(|f| f.tokens.clone())
    }

    pub fn get(&self, token_id: &str) -> Option<Tracked> {
        self.read(|f| f.tokens.iter().find(|t| t.token_id == token_id).cloned())
    }

    /// Follow `token`. Already followed: its place stays; a token added by hand stays "by hand"
    /// when the search finds it too, and a contract already read is kept unless `token` brings
    /// a newer one.
    pub fn track(&self, token: Tracked) -> std::io::Result<()> {
        self.change(|f| {
            match f.tokens.iter_mut().find(|t| t.token_id == token.token_id) {
                Some(old) => {
                    let origin = if old.origin == Origin::ByHand {
                        Origin::ByHand
                    } else {
                        token.origin
                    };
                    let contract = token.contract.or_else(|| old.contract.take());
                    *old = Tracked {
                        origin,
                        contract,
                        ..token
                    };
                }
                None => f.tokens.push(token),
            }
            true
        })
    }

    /// Stop following `token_id`; its hidden pairs go with it.
    pub fn untrack(&self, token_id: &str) -> std::io::Result<()> {
        self.change(|f| {
            let before = (f.tokens.len(), f.hidden.len());
            f.tokens.retain(|t| t.token_id != token_id);
            f.hidden.retain(|(t, _)| t != token_id);
            before != (f.tokens.len(), f.hidden.len())
        })
    }

    /// Keep `contract` (proved, serialized) for `token_id`, if it is followed.
    pub fn set_contract(&self, token_id: &str, contract: &[u8]) -> std::io::Result<()> {
        let hex = to_hex(contract);
        self.change(
            |f| match f.tokens.iter_mut().find(|t| t.token_id == token_id) {
                Some(t) if t.contract.as_deref() != Some(hex.as_str()) => {
                    t.contract = Some(hex.clone());
                    true
                }
                _ => false,
            },
        )
    }

    /// The contracts kept, as bytes: what the provider gets at start. A damaged one is left out.
    pub fn contracts(&self) -> Vec<(String, Vec<u8>)> {
        self.read(|f| {
            f.tokens
                .iter()
                .filter_map(|t| {
                    let bytes = from_hex(t.contract.as_deref()?)?;
                    Some((t.contract_id.clone(), bytes))
                })
                .collect()
        })
    }

    /// Hide (`on`) or show again `token_id` on `identity`.
    pub fn hide(&self, token_id: &str, identity: &str, on: bool) -> std::io::Result<()> {
        let pair = (token_id.to_owned(), identity.to_owned());
        self.change(|f| {
            if on {
                f.hidden.insert(pair.clone())
            } else {
                f.hidden.remove(&pair)
            }
        })
    }

    pub fn is_hidden(&self, token_id: &str, identity: &str) -> bool {
        self.read(|f| {
            f.hidden
                .contains(&(token_id.to_owned(), identity.to_owned()))
        })
    }

    fn read<T>(&self, f: impl FnOnce(&File) -> T) -> T {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        f(file.get_or_insert_with(|| self.load()))
    }

    /// Applies `edit` to a copy; when it changed something, the file is written and only then the
    /// copy kept, so a change that did not save did not happen.
    fn change(&self, edit: impl FnOnce(&mut File) -> bool) -> std::io::Result<()> {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = file.get_or_insert_with(|| self.load()).clone();
        if edit(&mut next) {
            crate::json_file::save(&self.path, &next).map_err(std::io::Error::other)?;
            *file = Some(next);
        }
        Ok(())
    }

    fn load(&self) -> File {
        crate::json_file::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evolite-tokens-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn token(id: &str, origin: Origin) -> Tracked {
        Tracked {
            token_id: id.into(),
            contract_id: format!("C{id}"),
            position: 0,
            origin,
            contract: None,
        }
    }

    #[test]
    fn tokens_contracts_and_hidden_pairs_stay_across_loads() {
        let path = dir("loads").join("tokens.json");
        let first = WalletTokens::new(path.clone());
        assert!(first.all().is_empty(), "no file, no tokens");
        first.track(token("A", Origin::ByHand)).unwrap();
        first.track(token("B", Origin::Found)).unwrap();
        first.set_contract("A", &[0, 1, 0xab, 0xff]).unwrap();
        first.hide("B", "I1", true).unwrap();
        // The next run of the app.
        let next = WalletTokens::new(path);
        let ids: Vec<_> = next.all().into_iter().map(|t| t.token_id).collect();
        assert_eq!(ids, ["A", "B"], "in the order they came in");
        assert_eq!(
            next.contracts(),
            [("CA".to_owned(), vec![0, 1, 0xab, 0xff])]
        );
        assert!(next.is_hidden("B", "I1") && !next.is_hidden("B", "I2"));
        next.hide("B", "I1", false).unwrap();
        assert!(!next.is_hidden("B", "I1"));
    }

    #[test]
    fn found_again_keeps_by_hand_and_the_contract() {
        let tokens = WalletTokens::new(dir("again").join("tokens.json"));
        tokens.track(token("A", Origin::ByHand)).unwrap();
        tokens.set_contract("A", &[7]).unwrap();
        tokens.track(token("A", Origin::Found)).unwrap();
        let a = tokens.get("A").unwrap();
        assert_eq!(a.origin, Origin::ByHand);
        assert_eq!(a.contract.as_deref(), Some("07"));
        assert_eq!(tokens.all().len(), 1, "followed once");
        // A newer contract replaces the kept one.
        tokens
            .track(Tracked {
                contract: Some("08".into()),
                ..token("A", Origin::Found)
            })
            .unwrap();
        assert_eq!(tokens.get("A").unwrap().contract.as_deref(), Some("08"));
    }

    #[test]
    fn untrack_takes_its_hidden_pairs_with_it() {
        let tokens = WalletTokens::new(dir("untrack").join("tokens.json"));
        tokens.track(token("A", Origin::Found)).unwrap();
        tokens.hide("A", "I1", true).unwrap();
        tokens.untrack("A").unwrap();
        assert!(tokens.get("A").is_none() && !tokens.is_hidden("A", "I1"));
    }

    #[test]
    fn a_damaged_file_or_contract_is_nothing_not_a_panic() {
        let dir = dir("damaged");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokens.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(WalletTokens::new(path.clone()).all().is_empty());
        let file = r#"{"tokens":[{"token_id":"A","contract_id":"CA","position":0,"origin":"found","contract":"0g"},
            {"token_id":"B","contract_id":"CB","position":1,"origin":"by_hand","contract":"abc"}]}"#;
        std::fs::write(&path, file).unwrap();
        let tokens = WalletTokens::new(path);
        assert_eq!(tokens.all().len(), 2);
        assert!(tokens.contracts().is_empty(), "not hex: left out");
    }

    #[test]
    fn a_change_that_did_not_save_did_not_happen() {
        let dir = dir("fail");
        // A folder where the file should be: the rename over it fails.
        let path = dir.join("tokens.json");
        std::fs::create_dir_all(&path).unwrap();
        let tokens = WalletTokens::new(path);
        assert!(tokens.track(token("A", Origin::ByHand)).is_err());
        assert!(tokens.all().is_empty());
        assert!(tokens.hide("A", "I1", true).is_err());
        assert!(!tokens.is_hidden("A", "I1"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
