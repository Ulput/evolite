//! What this wallet did on Platform: credit transfers and withdrawals, contact requests, profile
//! writes, DPNS names, shields into the pool. What each really cost is kept too, and the next
//! one of its kind is estimated from it (Platform meters these writes; no formula gives them).
//! Each is written here when Platform accepts it (`Source::Here`). What Platform keeps as
//! documents of the wallet's identities — DPNS names, contact requests, the profile,
//! withdrawals — is also read back from the network (`Source::Network`, `platform_history`)
//! and merged in by the document's key ([`merge`]), so a wallet restored elsewhere shows them
//! too (BUG-58, R-33). An entry written here is never removed.
//! Public data only (names, addresses, amounts, times). A missing or unreadable file is no
//! entries; it is never a reason to refuse anything. New fields are all `serde(default)`: an
//! older build reads a newer file and the other way round.

use evolite_api::{
    Activity, ActivityKind, Chain, Duffs, Proof, RequestState, Timestamp, WithdrawalState,
};
use std::path::PathBuf;
use std::sync::Mutex;

/// Where an entry comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Source {
    /// Written on this computer when Platform accepted it: amount and fee are known.
    #[default]
    Here,
    /// Read from a document on Platform (proved): no fee, an amount only when the document
    /// has one.
    Network,
}

/// A withdrawal document's `status` (the Withdrawals system contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WithdrawalStatus {
    Queued,
    Pooled,
    Broadcasted,
    Complete,
    Expired,
}

impl WithdrawalStatus {
    pub fn of(status: u64) -> Option<Self> {
        Some(match status {
            0 => Self::Queued,
            1 => Self::Pooled,
            2 => Self::Broadcasted,
            3 => Self::Complete,
            4 => Self::Expired,
            _ => return None,
        })
    }
}

impl WithdrawalStatus {
    fn shown(self) -> WithdrawalState {
        match self {
            Self::Queued => WithdrawalState::Queued,
            Self::Pooled => WithdrawalState::Pooled,
            Self::Broadcasted => WithdrawalState::Broadcasted,
            Self::Complete => WithdrawalState::Complete,
            Self::Expired => WithdrawalState::Expired,
        }
    }
}

/// Where a contact request stands between us and them, as DashPay says it now (BUG-58): on a
/// request to us, how it was answered; on ours, [`Reply::Accepted`] when ours answered theirs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Reply {
    /// Theirs is not answered.
    Waiting,
    /// Ours came after theirs: we accepted it.
    Accepted,
    /// Theirs came after ours: they accepted ours.
    TheyAccepted,
    /// We hid theirs.
    Hidden,
}

impl Reply {
    fn shown(self) -> RequestState {
        match self {
            Self::Waiting => RequestState::Waiting,
            Self::Accepted => RequestState::Accepted,
            Self::TheyAccepted => RequestState::TheyAccepted,
            Self::Hidden => RequestState::Hidden,
        }
    }
}

/// Entries further apart than this are different operations when matched without a key.
const SAME_TIME: Timestamp = 10 * 60;

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
    /// The wallet's identity it was done as (base58), when known.
    #[serde(default)]
    pub identity: Option<String>,
    /// The document's key on Platform (`dpns:<id>`, `profile:<id>:<revision>`, `wd:<id>`; a
    /// contact request ours to them `cr:<ours>:<to>`, theirs to us `cri:<from>:<ours>`, one
    /// per pair however often its key was rotated); `None` for an entry not matched to one yet.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub source: Source,
    /// A withdrawal's status, from its document.
    #[serde(default)]
    pub status: Option<WithdrawalStatus>,
    /// The Platform block height from the document, when it has one.
    #[serde(default)]
    pub block: Option<u64>,
    /// The line's id in the history, fixed once given ([`give_lines`]): matching an entry to its
    /// document later changes its key and time, not its id (the chosen line, notes on lines).
    #[serde(default)]
    pub line: Option<u64>,
    /// A contact request to us (`what` is `ContactRequest` naming the sender, key
    /// `cri:<from>:<ours>`), not ours. Kept as a flag, not a kind of its own: an older build
    /// reads the file still (it shows the line as a request of ours) instead of losing it.
    #[serde(default)]
    pub theirs: bool,
    /// How a contact request stands (both ways), from the last read of DashPay.
    #[serde(default)]
    pub reply: Option<Reply>,
}

impl Default for Entry {
    /// An entry written here, before its fields are set.
    fn default() -> Self {
        Self {
            at: 0,
            what: What::Profile,
            spent: 0,
            moved: 0,
            identity: None,
            key: None,
            source: Source::Here,
            status: None,
            block: None,
            line: None,
            theirs: false,
            reply: None,
        }
    }
}

/// Puts what was read from Platform (`network`) into the entries (`local`): by the document's
/// key; an entry without one (written before keys were kept, or before Platform was read)
/// matches the same kind and target within [`SAME_TIME`] of the document's time. A match takes
/// the document's key, identity, time and status and keeps its own amount and fee; a document
/// that matches nothing is added. Nothing is ever removed, and no line's id moves: each is fixed
/// before anything is matched. Returns whether anything changed.
pub fn merge(local: &mut Vec<Entry>, network: Vec<Entry>) -> bool {
    let mut changed = give_lines(local);
    for found in network {
        let Some(key) = found.key.clone() else {
            continue;
        };
        let by_key = local.iter().position(|e| e.key.as_deref() == Some(&key));
        let at = by_key.or_else(|| {
            local.iter().position(|e| {
                e.key.is_none()
                    && !found.theirs
                    && !e.theirs
                    && e.what == found.what
                    && (e.identity.is_none() || e.identity == found.identity)
                    && (e.at - found.at).abs() <= SAME_TIME
            })
        });
        match at {
            Some(i) => {
                let entry = &mut local[i];
                let before = entry.clone();
                entry.key = Some(key);
                entry.identity = entry.identity.take().or(found.identity);
                // The document's time is proved; a time of 0 is none. Matched by its key before,
                // the line keeps the earliest: a contact request's line is the first one sent
                // to them (later ones only rotate its key).
                if found.at > 0 {
                    entry.at = if by_key.is_some() && entry.at > 0 {
                        entry.at.min(found.at)
                    } else {
                        found.at
                    };
                }
                entry.status = found.status.or(entry.status);
                entry.reply = found.reply.or(entry.reply);
                entry.block = found.block.or(entry.block);
                // An entry read before keeps up with what the document says now.
                if entry.source == Source::Network {
                    entry.spent = found.spent;
                    entry.moved = found.moved;
                }
                changed |= *entry != before;
            }
            None => {
                local.push(found);
                give_lines(local);
                changed = true;
            }
        }
    }
    changed
}

/// Fixes the id of each entry that has none yet: from the document's key, else from the entry
/// itself (time, kind, amounts) — never from its place in the file, which a merged-in document
/// would shift. Two entries alike to the second take the next free id, in the file's order.
/// Above the txid-derived ids by half the range, as before. Returns whether any was given.
/// An id made from the entry uses `What`'s `Debug` form, which no build promises to keep: it
/// only has to last until the id is in the file, which the first [`merge`] after an update does.
fn give_lines(entries: &mut [Entry]) -> bool {
    if entries.iter().all(|e| e.line.is_some()) {
        return false;
    }
    let mut taken: std::collections::HashSet<u64> = entries.iter().filter_map(|e| e.line).collect();
    for entry in entries.iter_mut().filter(|e| e.line.is_none()) {
        let text = match &entry.key {
            Some(key) => key.clone(),
            None => format!(
                "{}:{:?}:{}:{}",
                entry.at, entry.what, entry.spent, entry.moved
            ),
        };
        // FNV-1a, 64 bits: stable across builds and runs, unlike the std hasher.
        let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        }) >> 2;
        let mut step = 0_u64;
        let line = loop {
            let line = u64::MAX / 2 + (hash.wrapping_add(step) & (u64::MAX >> 2));
            if taken.insert(line) {
                break line;
            }
            step += 1;
        };
        entry.line = Some(line);
    }
    true
}

pub struct Journal {
    path: PathBuf,
    /// Read once, then kept up to date with what is written.
    entries: Mutex<Option<Kept>>,
}

/// The entries in memory and whether the file lags behind them (a write that failed): the next
/// [`Journal::merge_network`] writes it even when it changes nothing (BUG-58 r2).
struct Kept {
    all: Vec<Entry>,
    unsaved: bool,
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
        entries.get_or_insert_with(|| self.kept()).all.clone()
    }

    fn kept(&self) -> Kept {
        Kept {
            all: self.load(),
            unsaved: false,
        }
    }

    /// Writes the entries and says whether the file now has them all.
    fn write(&self, kept: &mut Kept) -> Result<(), String> {
        let result = self.save(&kept.all);
        kept.unsaved = result.is_err();
        result
    }

    /// Keeps `entry`; written beside and renamed over, so a crash leaves the old file or the
    /// new one. A file that cannot be written loses only the history line.
    pub fn add(&self, entry: Entry) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let kept = entries.get_or_insert_with(|| self.kept());
        kept.all.push(entry);
        give_lines(&mut kept.all);
        let _ = self.write(kept);
    }

    /// [`merge`]s what was read from Platform and writes the file when anything changed, or
    /// when an earlier write failed (what it held is in memory only). Returns whether it
    /// changed; an error while the file does not have every entry, so that what was read is not
    /// counted as kept (the marks stay, R-33 p. 2) — this pass's or an earlier one's.
    pub fn merge_network(&self, network: Vec<Entry>) -> Result<bool, String> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let kept = entries.get_or_insert_with(|| self.kept());
        let changed = merge(&mut kept.all, network);
        if changed || kept.unsaved {
            self.write(kept)?;
        }
        Ok(changed)
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

    /// The file's entries, each with its line's id (given now to those written without one:
    /// by an older build, or before ids were kept).
    fn load(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = super::json_file::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        give_lines(&mut entries);
        entries
    }

    fn save(&self, entries: &[Entry]) -> Result<(), String> {
        super::json_file::save(&self.path, &entries)
    }
}

/// The line's id: the one fixed in the entry ([`give_lines`]; every entry the journal holds
/// has one).
fn line_id(entry: &Entry) -> u64 {
    entry.line.unwrap_or_else(|| {
        let mut one = [entry.clone()];
        give_lines(&mut one);
        one[0].line.unwrap_or_default()
    })
}

/// An entry as a line of the history: on Platform, proved (it was written only once Platform
/// accepted it, or read from a proved document).
pub fn activity(entry: &Entry) -> Activity {
    let kind = match &entry.what {
        What::Credits { to } => ActivityKind::CreditTransfer { to: to.clone() },
        What::Withdrawal { to } => ActivityKind::Withdrawal {
            to: to.clone(),
            status: entry.status.map(WithdrawalStatus::shown),
        },
        What::ContactRequest { to } if entry.theirs => ActivityKind::ContactRequestIn {
            from: to.clone(),
            ours: None,
            state: entry.reply.unwrap_or(Reply::Waiting).shown(),
        },
        What::ContactRequest { to } => ActivityKind::ContactRequest {
            to: to.clone(),
            accepting: entry.reply == Some(Reply::Accepted),
        },
        What::Profile => ActivityKind::ProfileSaved,
        What::Name { label } => ActivityKind::NameRegistered {
            label: label.clone(),
        },
        What::Shield { from } => ActivityKind::Shielded {
            from: Some(from.clone()),
        },
    };
    Activity {
        id: line_id(entry),
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
            ..Default::default()
        });
        journal.add(Entry {
            at: 1_001,
            what: What::Profile,
            spent: 0,
            moved: 0,
            ..Default::default()
        });
        let again = Journal::new(path);
        assert_eq!(again.entries().len(), 2);
        let line = activity(&again.entries()[0]);
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
            ..Default::default()
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
        assert_eq!(activity(&journal.entries()[2]).fee, None);
        // A name moves nothing: all it took is the fee.
        journal.add(entry(
            What::Name {
                label: "evolite".into(),
            },
            12_345,
            0,
        ));
        let line = activity(&journal.entries()[3]);
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
    fn here(at: Timestamp, what: What, spent: Duffs) -> Entry {
        Entry {
            at,
            what,
            spent,
            ..Default::default()
        }
    }

    fn read(at: Timestamp, what: What, key: &str) -> Entry {
        Entry {
            at,
            what,
            identity: Some("me".into()),
            key: Some(key.into()),
            source: Source::Network,
            ..Default::default()
        }
    }

    fn name(label: &str) -> What {
        What::Name {
            label: label.into(),
        }
    }

    /// BUG-58: reading the same documents again changes nothing and adds nothing.
    #[test]
    fn reading_the_network_twice_adds_nothing_the_second_time() {
        let mut local = Vec::new();
        let docs = || {
            vec![
                read(5_000, name("a"), "dpns:1"),
                read(6_000, name("b"), "dpns:2"),
            ]
        };
        assert!(merge(&mut local, docs()));
        assert_eq!(local.len(), 2);
        assert!(!merge(&mut local, docs()), "a second pass is no change");
        assert_eq!(local.len(), 2);
    }

    /// BUG-58: an entry written here before keys were kept is the document within 10 min: it
    /// takes the key and the document's time, and keeps what it cost.
    #[test]
    fn an_old_entry_within_ten_minutes_is_the_document() {
        let mut local = vec![here(10_000, name("a"), 1_234)];
        assert!(merge(
            &mut local,
            vec![read(10_000 + 590, name("a"), "dpns:1")]
        ));
        assert_eq!(local.len(), 1);
        let e = &local[0];
        assert_eq!((e.at, e.spent, e.source), (10_590, 1_234, Source::Here));
        assert_eq!(e.key.as_deref(), Some("dpns:1"));
        assert_eq!(e.identity.as_deref(), Some("me"));
        // Further than 10 min, or another name: a new line, the old one stays.
        let mut local = vec![here(10_000, name("a"), 1_234)];
        merge(&mut local, vec![read(10_000 + 601, name("a"), "dpns:1")]);
        merge(&mut local, vec![read(10_000, name("b"), "dpns:2")]);
        assert_eq!(local.len(), 3);
        assert_eq!(local[0].key, None);
    }

    /// BUG-58: Platform keeps only the profile's last revision: earlier "Profile saved" lines
    /// stay, the last one written here becomes the document.
    #[test]
    fn two_profile_saves_keep_both_lines() {
        let mut local = vec![
            here(1_000, What::Profile, 10),
            here(50_000, What::Profile, 11),
        ];
        merge(&mut local, vec![read(50_100, What::Profile, "profile:p:2")]);
        assert_eq!(local.len(), 2);
        assert_eq!(
            local[0].key, None,
            "the first save is not on Platform any more"
        );
        assert_eq!(local[1].key.as_deref(), Some("profile:p:2"));
        // The next revision is a line of its own.
        merge(&mut local, vec![read(90_000, What::Profile, "profile:p:3")]);
        assert_eq!(local.len(), 3);
    }

    /// BUG-58: a withdrawal read again follows its status; its line is not doubled.
    #[test]
    fn a_withdrawal_follows_its_status() {
        let to = || What::Withdrawal { to: "yAddr".into() };
        let doc = |status| Entry {
            status: Some(status),
            spent: 5_000,
            moved: 5_000,
            ..read(7_000, to(), "wd:9")
        };
        let mut local = vec![here(6_900, to(), 5_100)];
        assert!(merge(&mut local, vec![doc(WithdrawalStatus::Queued)]));
        assert!(merge(&mut local, vec![doc(WithdrawalStatus::Complete)]));
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].status, Some(WithdrawalStatus::Complete));
        assert_eq!(local[0].spent, 5_100, "what it cost here stays");
        // A withdrawal documented and then gone from Platform: the line stays.
        assert!(!merge(&mut local, Vec::new()));
        assert_eq!(local.len(), 1);
    }

    /// BUG-58: the file written by this build, read by the next run (and an older file without
    /// the new fields, read by this one); line ids follow the key, not the place in the file.
    #[test]
    fn merged_entries_survive_a_restart_with_the_same_line_ids() {
        let dir =
            std::env::temp_dir().join(format!("evolite-journal-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("platform-history.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            br#"[{"at":1000,"what":{"Name":{"label":"a"}},"spent":7}]"#,
        )
        .unwrap();
        let journal = Journal::new(path.clone());
        let old_id = activity(&journal.entries()[0]).id;
        assert!(
            journal
                .merge_network(vec![read(500, name("z"), "dpns:0")])
                .unwrap()
        );
        let again = Journal::new(path);
        let entries = again.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].spent, entries[0].source), (7, Source::Here));
        assert_eq!(activity(&entries[0]).id, old_id, "an id does not move");
        assert_eq!(entries[1].key.as_deref(), Some("dpns:0"));
        assert_ne!(activity(&entries[0]).id, activity(&entries[1]).id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-58 r1 should 1: an entry written before keys were kept keeps its line's id when it is
    /// first matched to its document (its key and time change), and after a restart.
    #[test]
    fn an_old_entry_keeps_its_line_id_when_it_is_matched() {
        let dir = std::env::temp_dir().join(format!("evolite-journal-line-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("platform-history.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            br#"[{"at":1000,"what":{"Name":{"label":"a"}},"spent":7}]"#,
        )
        .unwrap();
        let journal = Journal::new(path.clone());
        let old_id = activity(&journal.entries()[0]).id;
        assert!(
            journal
                .merge_network(vec![read(1_300, name("a"), "dpns:1")])
                .unwrap()
        );
        let entries = journal.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (entries[0].at, entries[0].key.as_deref()),
            (1_300, Some("dpns:1"))
        );
        assert_eq!(activity(&entries[0]).id, old_id, "matched, the id stays");
        let again = Journal::new(path);
        assert_eq!(
            activity(&again.entries()[0]).id,
            old_id,
            "and after a restart"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-58 r1 should 2: two entries alike to the second (two transfers to @bob at once) are
    /// two lines, each with its own id, the same after a restart.
    #[test]
    fn two_alike_entries_have_their_own_ids() {
        let dir =
            std::env::temp_dir().join(format!("evolite-journal-twins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("platform-history.json");
        let journal = Journal::new(path.clone());
        let bob = || What::Credits { to: "@bob".into() };
        journal.add(here(1_000, bob(), 100));
        journal.add(here(1_000, bob(), 100));
        let ids = |j: &Journal| {
            j.entries()
                .iter()
                .map(|e| activity(e).id)
                .collect::<Vec<_>>()
        };
        let first = ids(&journal);
        assert_ne!(first[0], first[1]);
        assert_eq!(ids(&Journal::new(path)), first);
        // Another amount is another id of its own, not the next one.
        let mut other = vec![here(1_000, bob(), 100), here(1_000, bob(), 200)];
        give_lines(&mut other);
        assert_eq!(other[0].line, Some(first[0]));
        assert_ne!(other[1].line, Some(first[1]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-58 r1 should 3: a journal that cannot be written says so, so the marks stay.
    #[test]
    fn a_journal_that_cannot_be_written_is_an_error() {
        let dir = std::env::temp_dir().join(format!("evolite-journal-ro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A file where the wallet's folder should be: nothing can be written under it.
        let blocker = dir.join("wallet");
        std::fs::write(&blocker, b"").unwrap();
        let journal = Journal::new(blocker.join("platform-history.json"));
        assert!(
            journal
                .merge_network(vec![read(1_000, name("a"), "dpns:1")])
                .is_err()
        );
        // Still not written: an error again, so the marks keep waiting.
        assert!(journal.merge_network(Vec::new()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Architect probe (BUG-58 r2): a failed write, then a pass that reads the same again once the
    /// folder is writable, must write the file before the marks may move.
    #[test]
    fn arch_probe_failed_write_is_written_by_the_next_pass() {
        let dir =
            std::env::temp_dir().join(format!("evolite-journal-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("wallet");
        std::fs::write(&blocker, b"").unwrap();
        let path = blocker.join("platform-history.json");
        let journal = Journal::new(path.clone());
        assert!(
            journal
                .merge_network(vec![read(1_000, name("a"), "dpns:1")])
                .is_err()
        );
        // The folder becomes writable (an antivirus lock let go).
        std::fs::remove_file(&blocker).unwrap();
        let second = journal.merge_network(vec![read(1_000, name("a"), "dpns:1")]);
        // The pass may move the marks only if what it read is on disk.
        assert!(second.is_ok(), "{second:?}");
        assert_eq!(
            Journal::new(path).entries().len(),
            1,
            "marks would move, file has nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-58 r1 must 1: a contact request's key is the pair; a later request to the same
    /// recipient (its key rotated) is the same line, at the first one's time.
    #[test]
    fn a_rotated_contact_request_is_the_same_line() {
        let to = || What::ContactRequest { to: "@bob".into() };
        let mut local = Vec::new();
        merge(&mut local, vec![read(1_000, to(), "cr:me:bob")]);
        let id = local[0].line;
        merge(&mut local, vec![read(9_000, to(), "cr:me:bob")]);
        assert_eq!(local.len(), 1);
        assert_eq!((local[0].at, local[0].line), (1_000, id));
    }

    /// BUG-58 part 2: a request to us never takes over a request of ours written here (the
    /// same person, within 10 min), and reading it again changes how it stands, not its line.
    #[test]
    fn a_request_to_us_is_a_line_of_its_own_that_follows_its_answer() {
        let bob = || What::ContactRequest { to: "@bob".into() };
        let theirs = |reply| Entry {
            theirs: true,
            reply: Some(reply),
            ..read(1_100, bob(), "cri:bob:me")
        };
        let mut local = vec![here(1_000, bob(), 50)];
        assert!(merge(&mut local, vec![theirs(Reply::Waiting)]));
        assert_eq!(local.len(), 2);
        assert_eq!(local[0].key, None, "ours stays ours");
        let line = local[1].line;
        assert!(matches!(
            activity(&local[1]).kind,
            ActivityKind::ContactRequestIn {
                state: RequestState::Waiting,
                ..
            }
        ));
        assert!(merge(&mut local, vec![theirs(Reply::Accepted)]));
        assert_eq!(local.len(), 2);
        assert_eq!(
            (local[1].reply, local[1].line),
            (Some(Reply::Accepted), line)
        );
        // An older build keeps reading the file: the flag is a field, not a new kind.
        let text = serde_json::to_string(&local).unwrap();
        assert!(
            text.contains(r#""what":{"ContactRequest":{"to":"@bob"}}"#),
            "{text}"
        );
    }
}
