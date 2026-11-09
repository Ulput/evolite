//! Apps the person let sign in as an identity ([`crate::app_connect`]): which app (its contract),
//! under which name it was shown, for which identity, the generation of its login key, and the
//! ids of its keys once they are on the identity. Platform holds the keys and the response; who
//! they belong to is only here. Public data only. A missing or unreadable file is no apps: the
//! keys stay on the identity, and the next sign-in finds its generation again from them.

use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct App {
    /// The app's contract, base58.
    pub contract: String,
    /// As the sign-in showed it: the contract owner's `@name`, else the app's own words.
    pub name: String,
    /// The identity signed in, base58.
    pub identity: String,
    pub generation: u32,
    /// The authentication and encryption key ids, once the app's keys are on the identity.
    pub keys: Option<(u32, u32)>,
    /// Unix seconds of the last sign-in.
    pub last_used: i64,
}

pub struct ConnectedApps {
    path: PathBuf,
    /// Read once, then kept up to date with what is written.
    apps: Mutex<Option<Vec<App>>>,
}

impl ConnectedApps {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            apps: Mutex::new(None),
        }
    }

    pub fn all(&self) -> Vec<App> {
        let mut apps = self.apps.lock().unwrap_or_else(|e| e.into_inner());
        apps.get_or_insert_with(|| self.load()).clone()
    }

    /// The app of `contract` for `identity`.
    pub fn get(&self, contract: &str, identity: &str) -> Option<App> {
        self.all()
            .into_iter()
            .find(|a| a.contract == contract && a.identity == identity)
    }

    /// Keeps `app` in place of the one of the same contract and identity.
    pub fn put(&self, app: App) -> Result<(), String> {
        self.change(|all| {
            all.retain(|a| !(a.contract == app.contract && a.identity == app.identity));
            all.push(app);
            all.sort_by_key(|a| std::cmp::Reverse(a.last_used));
        })
    }

    pub fn remove(&self, contract: &str, identity: &str) -> Result<(), String> {
        self.change(|all| all.retain(|a| !(a.contract == contract && a.identity == identity)))
    }

    /// Applies `edit` to a copy and keeps it once it is on disk.
    fn change(&self, edit: impl FnOnce(&mut Vec<App>)) -> Result<(), String> {
        let mut apps = self.apps.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = apps.get_or_insert_with(|| self.load()).clone();
        edit(&mut all);
        crate::json_file::save(&self.path, &all)?;
        *apps = Some(all);
        Ok(())
    }

    fn load(&self) -> Vec<App> {
        crate::json_file::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(contract: &str, identity: &str, last_used: i64) -> App {
        App {
            contract: contract.into(),
            name: "@yappr".into(),
            identity: identity.into(),
            generation: 0,
            keys: None,
            last_used,
        }
    }

    #[test]
    fn keeps_one_entry_per_app_and_identity_across_loads() {
        let dir = std::env::temp_dir().join(format!("evolite-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("connected-apps.json");
        let apps = ConnectedApps::new(path.clone());
        assert!(apps.all().is_empty(), "no file, no apps");
        apps.put(app("C1", "I1", 10)).unwrap();
        apps.put(app("C2", "I1", 20)).unwrap();
        apps.put(App {
            keys: Some((6, 7)),
            ..app("C1", "I1", 30)
        })
        .unwrap();
        apps.put(app("C1", "I2", 5)).unwrap();
        let again = ConnectedApps::new(path);
        let all = again.all();
        assert_eq!(all.len(), 3, "C1 for I1 once, newest first");
        assert_eq!(
            (all[0].contract.as_str(), all[0].keys),
            ("C1", Some((6, 7)))
        );
        assert!(again.get("C1", "I2").is_some());
        again.remove("C1", "I1").unwrap();
        assert!(
            ConnectedApps::new(dir.join("connected-apps.json"))
                .get("C1", "I1")
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_app_that_did_not_save_is_not_shown() {
        let dir = std::env::temp_dir().join(format!("evolite-apps-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("connected-apps.json");
        std::fs::create_dir_all(&path).unwrap();
        let apps = ConnectedApps::new(path);
        assert!(apps.put(app("C1", "I1", 1)).is_err());
        assert!(apps.all().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
