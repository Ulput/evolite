//! What this wallet did on Platform: credit transfers and withdrawals, contact requests, profile
//! writes, DPNS names, shields into the pool. What each really cost is kept too, and the next
//! one of its kind is estimated from it (Platform meters these writes; no formula gives them).
//! Platform keeps no history an identity can ask for, and platform-wallet keeps only
//! the Core side, so the history shows these from this file, written when each is accepted.
//! Public data only (names, addresses, amounts, times). A missing or unreadable file is no
//! entries; it is never a reason to refuse anything.

use evolite_api::{Activity, ActivityKind, Chain, Duffs, Proof, Timestamp};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum What {
    /// Credits to an identity (`@name`) or a Platform address.
    Credits {
        to: String,
    },
    Withdrawal {
        to: String,
    },
    ContactRequest {
        to: String,
    },
    Profile,
    /// A DPNS name registered for the identity.
    Name {
        label: String,
    },
    /// Credits (or a Platform address's) moved into the shielded pool; `from` says which.
    Shield {
        from: String,
    },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub at: Timestamp,
    pub what: What,
    /// What left the identity's credits, in duffs; 0 when not known.
    pub spent: Duffs,
    /// The part of `spent` that went to the payee (or into the pool), not to Platform: what is
    /// left is the fee. 0 in entries written before it was kept.
    #[serde(default)]
    pub moved: Duffs,
}

pub struct Journal {
    path: PathBuf,
    /// Read once, then kept up to date with what is written.
    entries: Mutex<Option<Vec<Entry>>>,
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            entries: Mutex::new(None),
        }
    }

    pub fn entries(&self) -> Vec<Entry> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.get_or_insert_with(|| self.load()).clone()
    }

    /// Keeps `entry`; written beside and renamed over, so a crash leaves the old file or the
    /// new one. A file that cannot be written loses only the history line.
    pub fn add(&self, entry: Entry) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let all = entries.get_or_insert_with(|| self.load());
        all.push(entry);
        let _ = self.save(all);
    }

    /// What the last entry of a kind (`kind`) cost in fees, in duffs; `None` before the first
    /// one whose fee is known.
    pub fn last_fee(&self, kind: impl Fn(&What) -> bool) -> Option<Duffs> {
        self.entries()
            .iter()
            .rev()
            .filter(|e| kind(&e.what))
            .find_map(fee_of)
    }

    fn load(&self) -> Vec<Entry> {
        super::json_file::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, entries: &[Entry]) -> Result<(), String> {
        super::json_file::save(&self.path, &entries)
    }
}

/// An entry as a line of the history: on Platform, proved (it was written only once Platform
/// accepted it). `n`: its place in the file, for an id of its own.
pub fn activity(entry: &Entry, n: usize) -> Activity {
    let kind = match &entry.what {
        What::Credits { to } => ActivityKind::CreditTransfer { to: to.clone() },
        What::Withdrawal { to } => ActivityKind::Withdrawal { to: to.clone() },
        What::ContactRequest { to } => ActivityKind::ContactRequest { to: to.clone() },
        What::Profile => ActivityKind::ProfileSaved,
        What::Name { label } => ActivityKind::NameRegistered {
            label: label.clone(),
        },
        What::Shield { from } => ActivityKind::Shielded {
            from: Some(from.clone()),
        },
    };
    Activity {
        // Below the ids of this run's pending payments, out of the txid range by chance only.
        id: u64::MAX / 2 + n as u64,
        kind,
        chain: Chain::Platform,
        at: entry.at,
        amount: -entry.spent,
        proof: Proof::Proved,
        txid: None,
        fee: fee_of(entry),
        block: None,
        address: None,
    }
}

/// What Platform took for an entry, when that is known: the balance was read before and after
/// (`spent` > 0), and for a kind that sends money, what it sent is kept (`moved`) and less.
fn fee_of(entry: &Entry) -> Option<Duffs> {
    if entry.spent <= 0 {
        return None;
    }
    if moves_money(&entry.what) {
        (entry.moved > 0 && entry.spent > entry.moved).then(|| entry.spent - entry.moved)
    } else {
        Some(entry.spent)
    }
}

/// Whether a kind of entry sends money somewhere (so `spent` is more than the fee).
fn moves_money(what: &What) -> bool {
    matches!(
        what,
        What::Credits { .. } | What::Withdrawal { .. } | What::Shield { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_entries_across_loads() {
        let dir = std::env::temp_dir().join(format!("evolite-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("platform-history.json");
        let journal = Journal::new(path.clone());
        assert!(journal.entries().is_empty(), "no file, no entries");
        journal.add(Entry {
            at: 1_000,
            what: What::Credits { to: "@bob".into() },
            spent: 100_000,
            moved: 0,
        });
        journal.add(Entry {
            at: 1_001,
            what: What::Profile,
            spent: 0,
            moved: 0,
        });
        let again = Journal::new(path);
        assert_eq!(again.entries().len(), 2);
        let line = activity(&again.entries()[0], 0);
        assert_eq!(
            line.kind,
            ActivityKind::CreditTransfer { to: "@bob".into() }
        );
        assert_eq!((line.amount, line.chain), (-100_000, Chain::Platform));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_next_fee_is_what_the_last_one_cost() {
        let dir = std::env::temp_dir().join(format!("evolite-journal-fee-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = Journal::new(dir.join("platform-history.json"));
        let credits = |w: &What| matches!(w, What::Credits { .. });
        assert_eq!(journal.last_fee(credits), None, "nothing written yet");
        let entry = |what, spent, moved| Entry {
            at: 1_000,
            what,
            spent,
            moved,
        };
        // Written before `moved` was kept: the fee cannot be told from the amount.
        journal.add(entry(What::Credits { to: "@a".into() }, 100_000, 0));
        assert_eq!(journal.last_fee(credits), None);
        journal.add(entry(What::Credits { to: "@b".into() }, 100_150, 100_000));
        assert_eq!(journal.last_fee(credits), Some(150));
        // A shield from a Platform address before its sync: the fee is not known, the line
        // shows none and the estimate keeps the last known one.
        journal.add(entry(
            What::Shield {
                from: tr!("a Platform address").into(),
            },
            300_000,
            300_000,
        ));
        let shield = |w: &What| matches!(w, What::Shield { .. });
        assert_eq!(journal.last_fee(shield), None);
        assert_eq!(activity(&journal.entries()[2], 2).fee, None);
        // A name moves nothing: all it took is the fee.
        journal.add(entry(
            What::Name {
                label: "evolite".into(),
            },
            12_345,
            0,
        ));
        let line = activity(&journal.entries()[3], 3);
        assert_eq!(
            (line.kind, line.fee),
            (
                ActivityKind::NameRegistered {
                    label: "evolite".into()
                },
                Some(12_345)
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
