//! Where each funding of the Platform address was signed to go (BUG-171): `txid → address`,
//! written once the lock is built, read by «Continue» so that it credits the address the review
//! showed. The shown address is the next unused one, and it moves when the last one receives
//! something: «Continue» after a restart took the one shown then, not the one signed for. After a
//! restore from the phrase there is no record: «Continue» goes to the address shown now, and its
//! dialog says which.
//!
//! A record outlives its lock (BUG-173, the architect): it also names the funding's line in the
//! history from the first frame of a start, before platform-wallet's store is read. It is marked
//! taken when its lock is no longer open, and goes 90 days after that; at most 200 are kept, the
//! oldest go first. Only the wallet's own addresses and txids that are on chain anyway — never in
//! the journal. Written whole (`json_file`); a file that cannot be read is never written over.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::json_file;

/// At most this many records; the oldest go first.
pub const MAX: usize = 200;
/// A record goes this long after its lock was taken.
pub const KEEP_TAKEN_SECS: i64 = 90 * 24 * 60 * 60;

/// Unix seconds now: when a record is added or its lock is seen taken.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// One funding: where it goes, when it was recorded, when its lock was taken (unix seconds).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
struct Record {
    address: String,
    added: i64,
    #[serde(default)]
    taken: Option<i64>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct File {
    /// By the lock's txid.
    #[serde(default)]
    records: BTreeMap<String, Record>,
    /// BUG-171's first form, `txid → address`: read, written as `records`.
    #[serde(default, skip_serializing)]
    intents: BTreeMap<String, String>,
}

#[derive(Default)]
enum State {
    /// Not read yet.
    #[default]
    Unread,
    Read(BTreeMap<String, Record>),
    /// A file there that cannot be read: kept as it is (never written over), and nothing in it
    /// is known — «Continue» goes to the address shown now.
    Broken,
}

/// `fund-intents.json` in the wallet's folder.
pub struct FundIntents {
    path: PathBuf,
    state: Mutex<State>,
}

impl FundIntents {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::default(),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*state, State::Unread) {
            *state = match json_file::read(&self.path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::Read(BTreeMap::new()),
                Err(_) => State::Broken,
                Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
                    Ok(mut file) => {
                        for (txid, address) in std::mem::take(&mut file.intents) {
                            file.records.entry(txid).or_insert(Record {
                                address,
                                added: 0,
                                taken: None,
                            });
                        }
                        State::Read(file.records)
                    }
                    Err(_) => State::Broken,
                },
            };
        }
        f(&mut state)
    }

    fn save(&self, records: &BTreeMap<String, Record>) {
        // A file that cannot be written only loses the record: «Continue» then goes to the
        // address shown and says so, and the line is named once the store is read.
        let _ = json_file::save(
            &self.path,
            &File {
                records: records.clone(),
                intents: BTreeMap::new(),
            },
        );
    }

    /// The address the lock `txid` was signed to credit, if it was recorded here.
    pub fn address(&self, txid: &str) -> Option<String> {
        self.with(|state| match state {
            State::Read(records) => records.get(txid).map(|r| r.address.clone()),
            _ => None,
        })
    }

    /// Whether `txid` is a funding of the Platform address recorded here, taken or not
    /// (BUG-173: its line is named so from the first frame).
    pub fn is_funding(&self, txid: &str) -> bool {
        self.with(|state| matches!(state, State::Read(records) if records.contains_key(txid)))
    }

    /// The lock `txid` was built to credit `address`, at `now`.
    pub fn put(&self, txid: &str, address: &str, now: i64) {
        self.with(|state| {
            if let State::Read(records) = state {
                records.insert(
                    txid.to_owned(),
                    Record {
                        address: address.to_owned(),
                        added: now,
                        taken: None,
                    },
                );
                prune(records, now);
                self.save(records);
            }
        });
    }

    /// The locks `open` are open now (`now`): every other record's lock is taken (by Platform,
    /// here or elsewhere) from now on, if it was not already; old and surplus records go.
    pub fn settle(&self, open: &[String], now: i64) {
        self.with(|state| {
            if let State::Read(records) = state {
                let before = records.clone();
                for (txid, record) in records.iter_mut() {
                    if record.taken.is_none() && !open.contains(txid) {
                        record.taken = Some(now);
                    }
                }
                prune(records, now);
                if *records != before {
                    self.save(records);
                }
            }
        });
    }
}

/// Drops records taken more than [`KEEP_TAKEN_SECS`] before `now`, then the oldest beyond
/// [`MAX`] (taken ones before open ones).
fn prune(records: &mut BTreeMap<String, Record>, now: i64) {
    records.retain(|_, r| r.taken.is_none_or(|at| now - at <= KEEP_TAKEN_SECS));
    while records.len() > MAX {
        let oldest = records
            .iter()
            .min_by_key(|(_, r)| (r.taken.is_none(), r.added))
            .map(|(txid, _)| txid.clone());
        match oldest {
            Some(txid) => records.remove(&txid),
            None => break,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evolite-intents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("fund-intents.json")
    }

    /// BUG-171 AC-T1, BUG-173: recorded when the lock is built, read back — also by a new run;
    /// once the lock is no longer open the record stays, taken, and still names the funding.
    #[test]
    fn a_record_outlives_its_lock() {
        let path = folder("kept");
        let intents = FundIntents::new(path.clone());
        assert_eq!(intents.address("aa"), None);
        intents.put("aa", "tdash1kr95", 100);
        intents.put("bb", "tdash1kpmx", 100);
        let again = FundIntents::new(path.clone());
        assert_eq!(again.address("aa").as_deref(), Some("tdash1kr95"));
        again.settle(&["bb".to_owned()], 200);
        let third = FundIntents::new(path);
        assert!(third.is_funding("aa"), "taken, still named");
        assert_eq!(third.address("bb").as_deref(), Some("tdash1kpmx"));
        assert!(!third.is_funding("cc"));
    }

    /// BUG-173 (the architect): 90 days after its lock was taken a record goes; beyond 200 the
    /// oldest go, taken ones first.
    #[test]
    fn records_are_kept_90_days_and_200_at_most() {
        let intents = FundIntents::new(folder("bounds"));
        intents.put("old", "tdash1a", 0);
        intents.settle(&[], 10);
        intents.settle(&[], 10 + KEEP_TAKEN_SECS);
        assert!(intents.is_funding("old"), "90 days: still there");
        intents.settle(&[], 11 + KEEP_TAKEN_SECS);
        assert!(!intents.is_funding("old"), "a day later: gone");
        let mut records = BTreeMap::new();
        for n in 0..(MAX + 3) as i64 {
            records.insert(
                format!("{n:03}"),
                Record {
                    address: "tdash1".into(),
                    added: n,
                    taken: (n < 2).then_some(n),
                },
            );
        }
        prune(&mut records, 50);
        assert_eq!(records.len(), MAX);
        assert!(
            !records.contains_key("000") && !records.contains_key("001"),
            "taken first"
        );
        assert!(!records.contains_key("002"), "then the oldest open");
    }

    /// BUG-171's first form, `txid → address`, reads as records.
    #[test]
    fn the_first_form_still_reads() {
        let path = folder("first");
        std::fs::write(&path, br#"{"intents":{"aa":"tdash1kr95"}}"#).unwrap();
        let intents = FundIntents::new(path);
        assert_eq!(intents.address("aa").as_deref(), Some("tdash1kr95"));
    }

    /// BUG-171 AC-T1: a file that cannot be read is not written over, and gives no address.
    #[test]
    fn a_broken_file_is_left_alone() {
        let path = folder("broken");
        std::fs::write(&path, b"{ not json").unwrap();
        let intents = FundIntents::new(path.clone());
        assert_eq!(intents.address("aa"), None);
        intents.put("aa", "tdash1kr95", 1);
        intents.settle(&[], 2);
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }
}
