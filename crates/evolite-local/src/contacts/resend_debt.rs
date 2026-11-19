//! FEAT-0020-resend: the contact requests an identity of this wallet still owes after its DashPay
//! pair was replaced. A contact whose request to us points to our disabled key cannot pay us
//! until we send a new one; the batch after the key change sends them, and what it could not
//! is kept here, to be sent with «Send requests again» (and a password). Public data only
//! (identity ids and a key number); a missing or unreadable file is nothing owed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where an owed request stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Owed {
    /// To send, by the button.
    Due,
    /// After a change of keys not known to have gone through: shown, and so sendable, once a
    /// read shows the change; dropped if it did not land.
    Waiting,
    /// Platform refused it by the unique index at the last version of the reference (R-11): no
    /// new request can go to this contact. Not counted, no button.
    Blocked,
}

/// One owed request: from `identity` to `contact`, to be signed with our encryption key `key`
/// (the one of the new pair).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub identity: String,
    pub contact: String,
    pub key: u32,
    pub state: Owed,
}

pub struct ResendDebt {
    store: crate::json_file::Store<crate::json_file::Entries<Row>>,
}

impl ResendDebt {
    pub fn new(path: PathBuf) -> Self {
        Self {
            store: crate::json_file::Store::new(path),
        }
    }

    /// The file, for «Local files»: one that does not read is set aside and said so, not read
    /// as nothing owed in silence (review r1 S2).
    pub fn store(&self) -> &crate::json_file::Store<crate::json_file::Entries<Row>> {
        &self.store
    }

    /// Every owed request.
    pub fn load(&self) -> Vec<Row> {
        self.store.read(|e| e.known.clone())
    }

    /// The owed requests of `identity`.
    pub fn of(&self, identity: &str) -> Vec<Row> {
        self.store.read(|e| {
            e.known
                .iter()
                .filter(|r| r.identity == identity)
                .cloned()
                .collect()
        })
    }

    /// A new change of `identity`'s keys: whatever it owed is replaced by `rows` (the key in
    /// them changed).
    pub fn replace(&self, identity: &str, rows: Vec<Row>) -> Result<(), String> {
        self.change(|all| {
            all.retain(|r| r.identity != identity);
            all.extend(rows);
            ((), true)
        })
    }

    /// Sets the state of `identity`'s request to `contact`, adding it if it is not there.
    pub fn put(&self, row: Row) -> Result<(), String> {
        self.change(|all| {
            all.retain(|r| !(r.identity == row.identity && r.contact == row.contact));
            all.push(row);
            ((), true)
        })
    }

    /// `identity`'s request to `contact` is sent (or was already there): no longer owed.
    pub fn settle(&self, identity: &str, contact: &str) -> Result<(), String> {
        self.change(|all| {
            let before = all.len();
            all.retain(|r| !(r.identity == identity && r.contact == contact));
            ((), all.len() != before)
        })
    }

    /// `identity`'s rows waiting for its change of keys: `Due` once a read showed the change,
    /// dropped when it did not land.
    pub fn after_change(&self, identity: &str, landed: bool) -> Result<(), String> {
        self.change(|all| {
            let before = all.clone();
            if landed {
                for r in all.iter_mut() {
                    if r.identity == identity && r.state == Owed::Waiting {
                        r.state = Owed::Due;
                    }
                }
            } else {
                all.retain(|r| !(r.identity == identity && r.state == Owed::Waiting));
            }
            let changed = *all != before;
            ((), changed)
        })
    }

    fn change<R>(&self, edit: impl FnOnce(&mut Vec<Row>) -> (R, bool)) -> Result<R, String> {
        self.store
            .update(|e| edit(&mut e.known))
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(contact: &str, state: Owed) -> Row {
        Row {
            identity: "A".into(),
            contact: contact.into(),
            key: 8,
            state,
        }
    }

    /// FEAT-0020-resend AC-T3: what was owed is on disk — a new instance on the same file sees
    /// it — and a request sent is no longer owed.
    #[test]
    fn what_is_owed_outlives_the_run() {
        let dir = std::env::temp_dir().join(format!("evolite-resend-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("contact-resend.json");
        let debt = ResendDebt::new(path.clone());
        debt.replace("A", vec![row("C2", Owed::Due)]).unwrap();
        debt.put(Row {
            identity: "B".into(),
            ..row("C9", Owed::Due)
        })
        .unwrap();
        let again = ResendDebt::new(path.clone());
        assert_eq!(again.of("A"), vec![row("C2", Owed::Due)]);
        again.settle("A", "C2").unwrap();
        assert!(ResendDebt::new(path).of("A").is_empty());
        assert_eq!(again.load().len(), 1, "another identity's is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review r1 S2: a file that does not read is set aside — said, not «nothing owed» in
    /// silence — and nothing is written over it.
    #[test]
    fn an_unreadable_file_is_set_aside() {
        let dir = std::env::temp_dir().join(format!("evolite-resend-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("contact-resend.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let debt = ResendDebt::new(path);
        assert!(debt.load().is_empty());
        assert!(matches!(
            debt.store().state(),
            crate::json_file::State::SetAside { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A change of keys not known yet: its rows wait; shown, they are due; not landed, gone.
    /// A new change replaces the identity's rows.
    #[test]
    fn rows_after_an_unclear_change_wait_for_it() {
        let dir = std::env::temp_dir().join(format!("evolite-resend-w-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let debt = ResendDebt::new(dir.join("contact-resend.json"));
        debt.replace(
            "A",
            vec![row("C1", Owed::Waiting), row("C2", Owed::Blocked)],
        )
        .unwrap();
        debt.after_change("A", true).unwrap();
        assert_eq!(debt.of("A")[0].state, Owed::Due);
        debt.replace("A", vec![row("C1", Owed::Waiting)]).unwrap();
        debt.after_change("A", false).unwrap();
        assert!(debt.of("A").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
