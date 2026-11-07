//! platform-wallet's SQLite store with what it does not keep (BUG-77, R-34, workarounds (1) and
//! (2) of BUG-77.md «Обход в evolite»):
//!
//! 1. **DashPay's backfill record.** platform-wallet rewinds the filters to where contacts were
//!    first paid unless a stored record says those blocks were already scanned with the
//!    contacts' addresses watched. The SQLite backend keeps no such record, so the filters went
//!    back ~18 000 blocks at every start of the chain — and old payments came back on screen
//!    before the blocks that spent them. [`Store`] keeps the record in `backfill.json` beside the
//!    store and answers [`stores_dashpay_backfill`](PlatformWalletPersistence) `true`.
//! 2. **Spent coins.** The store does not load the transactions that spent the wallet's coins,
//!    so a rewound pass credited them again. Its spent coins (`core_utxos`, `spent = 1`) are
//!    claimed at the load ([`ManagedWalletInfo::restore_spent_outpoints`]): a claimed coin is
//!    never credited again.
//!
//! **What a record may never do is skip a contact's payment.** A record vouches that blocks
//! from each contact's `covered_from` up to the store's cursor were tested with that contact
//! watched; one that vouches for blocks never scanned hides the payments in them. So:
//! - it is written only after the SQLite round it came with is committed (a crash between the
//!   two leaves the older record: a contact it lacks is backfilled again);
//! - the SQLite backend keeps its cursor as a monotonic max and ignores the rewind the record
//!   comes with: a backfill cut short would not show in the cursor. The record is therefore
//!   marked complete only once the scan, watched here round by round, has climbed back to the
//!   height it was rewound from; an unfinished one is not taken back;
//! - at the load it is taken back only when it is this wallet's (its key and its check), whole,
//!   complete, and the store's cursor is at or above the one it was written with (an older
//!   store under a newer record — a copy, a backup — is not vouched for). Anything else is no
//!   record, i.e. the rewind as before: slow, never lossy. The journal says which.
//!
//! Claims only stop a credit: a coin wrongly claimed can make the balance lower, never higher.
//! Only coins whose spender the store holds in a ChainLocked block are claimed — a send seen in
//! the mempool alone may never be mined, and a block without a ChainLock may be reorganized out:
//! either way its coin must come back, not stay hidden until a resync.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dpp::prelude::Identifier;
use key_wallet::dashcore::{Network, OutPoint, Txid};
use key_wallet::managed_account::transaction_record::TransactionRecord;
use key_wallet_manager::WalletId;
use platform_wallet::changeset::{
    ClientStartState, DashPayBackfillRecord, DpnsNameStateEntry, ListedCoreTxid,
    PersistenceCapabilities, PersistenceError, PlatformWalletChangeSet, PlatformWalletPersistence,
};
use platform_wallet::masternode::TrackedMasternode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The live test's measure before the workarounds (`EVOLITE_B77_PLAIN`): the store as SQLite alone.
#[cfg(test)]
fn plain() -> bool {
    std::env::var_os("EVOLITE_B77_PLAIN").is_some()
}

#[cfg(not(test))]
fn plain() -> bool {
    false
}

/// The backfill record's file, beside the store.
pub fn backfill_file(store: &Path) -> PathBuf {
    store.with_file_name("backfill.json")
}

/// What the load did with a wallet's saved backfill record (the journal's `outcome`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Taken back: no rewind for the contacts it covers.
    Kept,
    /// None saved for this wallet.
    None,
    /// The file does not read (JSON, hex or the record's own layout).
    Damaged,
    /// Its check does not match what it says: edited, or another wallet's moved over.
    Tampered,
    /// Written with a higher cursor than the store has: an older store under a newer record.
    Ahead,
    /// The rewind it came with was not scanned back up before the app stopped.
    Unfinished,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Kept => "kept",
            Verdict::None => "none",
            Verdict::Damaged => "rejected_damaged",
            Verdict::Tampered => "rejected_tampered",
            Verdict::Ahead => "rejected_ahead",
            Verdict::Unfinished => "rejected_unfinished",
        }
    }
}

/// One wallet's record as saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    /// The store's cursor (its `synced_height`, a monotonic max) when this was written.
    cursor: u32,
    floor: u32,
    rewound_from: u32,
    /// [`DashPayBackfillRecord::covered_bytes`], hex.
    covered: String,
    /// The rewind it came with has been scanned back up to where it started.
    complete: bool,
    /// SHA-256 of the wallet's id and every field above, hex.
    check: String,
}

impl Entry {
    fn new(id: &WalletId, cursor: u32, record: &DashPayBackfillRecord, complete: bool) -> Self {
        let mut entry = Entry {
            cursor,
            floor: record.floor,
            rewound_from: record.rewound_from,
            covered: hex(&record.covered_bytes()),
            complete,
            check: String::new(),
        };
        entry.check = entry.digest(id);
        entry
    }

    fn digest(&self, id: &WalletId) -> String {
        let mut h = Sha256::new();
        h.update(b"evolite backfill v1");
        h.update(id);
        h.update(self.cursor.to_le_bytes());
        h.update(self.floor.to_le_bytes());
        h.update(self.rewound_from.to_le_bytes());
        h.update([u8::from(self.complete)]);
        h.update(self.covered.as_bytes());
        hex(&h.finalize())
    }

    fn record(&self) -> Option<DashPayBackfillRecord> {
        DashPayBackfillRecord::from_parts(self.floor, self.rewound_from, &unhex(&self.covered)?)
    }

    fn with_cursor(&self, id: &WalletId, cursor: u32, complete: bool) -> Self {
        let mut entry = self.clone();
        entry.cursor = cursor;
        entry.complete = complete;
        entry.check = entry.digest(id);
        entry
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    /// By wallet id, hex.
    wallets: BTreeMap<String, Entry>,
}

/// A rewind being scanned back up: the scan is at `at`, and done at `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Climb {
    at: u32,
    to: u32,
}

#[derive(Default)]
struct Tracked {
    /// The store's cursor as it has it: the max of every one committed.
    cursor: Option<u32>,
    climb: Option<Climb>,
    entry: Option<Entry>,
}

/// platform-wallet's persister: the SQLite one (`P`) for everything, plus the backfill record and
/// the spent-coin claims (see the module).
pub struct Store<P> {
    inner: P,
    db: PathBuf,
    file: PathBuf,
    wallets: Mutex<BTreeMap<WalletId, Tracked>>,
}

impl<P> Store<P> {
    /// `inner` is the store at `db`; the record goes beside it.
    pub fn new(inner: P, db: PathBuf) -> Self {
        Self {
            file: backfill_file(&db),
            inner,
            db,
            wallets: Mutex::new(BTreeMap::new()),
        }
    }

    fn tracked(&self) -> std::sync::MutexGuard<'_, BTreeMap<WalletId, Tracked>> {
        self.wallets.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The saved record of wallet `id`, judged against the store's cursor `loaded`: the record
    /// platform-wallet gets (empty unless [`Verdict::Kept`]) and why. Journaled; the wallet's
    /// tracking starts from here.
    pub fn take_back(&self, id: &WalletId, loaded: u32) -> (DashPayBackfillRecord, Verdict) {
        let (verdict, entry, record) = judge(super::json_file::read(&self.file).ok(), id, loaded);
        crate::diag::record(crate::diag::Record::Backfill {
            outcome: verdict.name(),
            contacts: record.covered().len() as u32,
        });
        self.tracked().insert(
            *id,
            Tracked {
                cursor: Some(loaded),
                climb: None,
                entry,
            },
        );
        (record, verdict)
    }

    /// What a committed round changes in the record (see the module): the cursor followed, a
    /// rewind's climb, the record replaced. Written when the saved entry changes.
    fn committed(&self, id: &WalletId, round: Round) {
        let mut wallets = self.tracked();
        let w = wallets.entry(*id).or_default();
        let before = w.cursor;
        if let Some(h) = round.height {
            w.cursor = Some(before.map_or(h, |c| c.max(h)));
            if round.rewind {
                let to = before.unwrap_or(h).max(w.climb.map_or(0, |c| c.to));
                w.climb = Some(Climb { at: h, to });
            } else if let Some(climb) = &mut w.climb {
                climb.at = climb.at.max(h);
            }
        }
        let mut changed = false;
        if let Some(record) = round.record {
            changed = true;
            w.entry = match (record.is_empty(), w.cursor) {
                (true, _) => None,
                // A record with no cursor to vouch for is not kept.
                (false, None) => None,
                (false, Some(cursor)) => {
                    if let Some(climb) = &mut w.climb
                        && record.has_extent()
                    {
                        climb.to = climb.to.max(record.rewound_from);
                    } else if w.climb.is_none()
                        && record.has_extent()
                        && record.rewound_from > cursor
                    {
                        w.climb = Some(Climb {
                            at: cursor,
                            to: record.rewound_from,
                        });
                    }
                    Some(Entry::new(id, cursor, &record, false))
                }
            };
        }
        if let Some(climb) = w.climb
            && climb.at >= climb.to
        {
            w.climb = None;
        }
        if w.climb.is_none()
            && let (Some(entry), Some(cursor)) = (&w.entry, w.cursor)
            && !entry.complete
        {
            w.entry = Some(entry.with_cursor(id, cursor, true));
            changed = true;
        }
        if changed {
            let saved = Saved {
                wallets: wallets
                    .iter()
                    .filter_map(|(id, w)| Some((hex(id), w.entry.clone()?)))
                    .collect(),
            };
            drop(wallets);
            self.save(&saved);
        }
    }

    /// Writes the file whole; a failure is journaled and the file goes: an older record must not
    /// stay beside a store it no longer describes.
    fn save(&self, saved: &Saved) {
        let result = if saved.wallets.is_empty() {
            match std::fs::remove_file(&self.file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        } else {
            super::json_file::save(&self.file, saved)
        };
        if result.is_err() {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

/// What [`Store::committed`] needs of a round, taken before the round goes to SQLite.
struct Round {
    height: Option<u32>,
    rewind: bool,
    record: Option<DashPayBackfillRecord>,
}

impl Round {
    fn of(changeset: &PlatformWalletChangeSet) -> Self {
        let core = changeset.core.as_ref();
        Round {
            height: core.and_then(|c| c.synced_height),
            rewind: core.is_some_and(|c| c.synced_height.is_some() && c.synced_height_is_rewind),
            record: changeset.dashpay_backfill.clone(),
        }
    }
}

/// The saved file judged for wallet `id` at the store's cursor `loaded`: the verdict, the entry
/// to go on from, and the record for platform-wallet.
fn judge(
    file: Option<Vec<u8>>,
    id: &WalletId,
    loaded: u32,
) -> (Verdict, Option<Entry>, DashPayBackfillRecord) {
    let none = DashPayBackfillRecord::default();
    let Some(bytes) = file else {
        return (Verdict::None, None, none);
    };
    let Ok(saved) = serde_json::from_slice::<Saved>(&bytes) else {
        return (Verdict::Damaged, None, none);
    };
    let Some(entry) = saved.wallets.get(&hex(id)) else {
        return (Verdict::None, None, none);
    };
    if entry.check != entry.digest(id) {
        return (Verdict::Tampered, None, none);
    }
    let Some(record) = entry.record() else {
        return (Verdict::Damaged, None, none);
    };
    if entry.cursor > loaded {
        return (Verdict::Ahead, None, none);
    }
    if !entry.complete {
        return (Verdict::Unfinished, None, none);
    }
    (Verdict::Kept, Some(entry.clone()), record)
}

/// Every method goes to the SQLite store — those with a default too, or the default would answer
/// instead of SQLite — except the backfill record's (see the module).
impl<P: PlatformWalletPersistence> PlatformWalletPersistence for Store<P> {
    fn store_commits_inline(&self) -> bool {
        self.inner.store_commits_inline()
    }

    fn persistence_capabilities(&self) -> PersistenceCapabilities {
        self.inner.persistence_capabilities()
    }

    fn store_transient_is_reissuable(&self) -> bool {
        self.inner.store_transient_is_reissuable()
    }

    fn stores_dashpay_backfill(&self) -> bool {
        !plain()
    }

    fn persists_durably(&self) -> bool {
        self.inner.persists_durably()
    }

    fn store(
        &self,
        wallet_id: WalletId,
        changeset: PlatformWalletChangeSet,
    ) -> Result<(), PersistenceError> {
        let round = Round::of(&changeset);
        // SQLite first: the record vouches for the cursor of a committed round only.
        self.inner.store(wallet_id, changeset)?;
        self.committed(&wallet_id, round);
        Ok(())
    }

    fn flush(&self, wallet_id: WalletId) -> Result<(), PersistenceError> {
        self.inner.flush(wallet_id)
    }

    fn persist_tracked_masternodes(
        &self,
        network: Network,
        records: &[TrackedMasternode],
    ) -> Result<(), PersistenceError> {
        self.inner.persist_tracked_masternodes(network, records)
    }

    fn load_tracked_masternodes(
        &self,
        network: Network,
    ) -> Result<Vec<TrackedMasternode>, PersistenceError> {
        self.inner.load_tracked_masternodes(network)
    }

    fn load(&self) -> Result<ClientStartState, PersistenceError> {
        let mut state = self.inner.load()?;
        if plain() {
            return Ok(state);
        }
        for (id, wallet) in state.wallets.iter_mut() {
            let loaded = wallet.wallet_info.metadata.synced_height;
            wallet.dashpay_backfill = self.take_back(id, loaded).0;
            // Before the SPV client starts: no block reaches the wallet before its claims.
            let claims = spent_claims(&self.db, id);
            let held = wallet
                .wallet_info
                .restore_spent_outpoints(&claims.claimed)
                .len();
            crate::diag::record(crate::diag::Record::SpentClaims {
                claimed: claims.claimed.len() as u32,
                not_in_block: claims.not_in_block as u32,
                still_held: held as u32,
            });
        }
        Ok(state)
    }

    fn get_core_tx_record(
        &self,
        wallet_id: WalletId,
        txid: &Txid,
    ) -> Result<Option<TransactionRecord>, PersistenceError> {
        self.inner.get_core_tx_record(wallet_id, txid)
    }

    fn list_wallet_core_txids(
        &self,
        wallet_id: WalletId,
    ) -> Result<Option<Vec<ListedCoreTxid>>, PersistenceError> {
        self.inner.list_wallet_core_txids(wallet_id)
    }

    fn get_dpns_name_state(
        &self,
        wallet_id: WalletId,
        wallet_identity_id: &Identifier,
        normalized_label: &str,
    ) -> Result<Option<DpnsNameStateEntry>, PersistenceError> {
        self.inner
            .get_dpns_name_state(wallet_id, wallet_identity_id, normalized_label)
    }
}

/// The store's spent coins to claim: each with the transaction that spent it in a ChainLocked
/// block.
#[derive(Debug, Default, PartialEq)]
pub struct Claims {
    pub claimed: Vec<(OutPoint, Option<Txid>)>,
    /// Spent in the store, but no spender of it in a ChainLocked block there: not claimed (the
    /// journal's `notInBlock`).
    pub not_in_block: usize,
}

/// The coins of wallet `wallet` the store at `db` has as spent (`core_utxos`, `spent = 1`) whose
/// spender it holds in a ChainLocked block (`core_transactions`), read only. A store this does not read
/// claims nothing: the pass as before, never a coin hidden for nothing.
pub fn spent_claims(db: &Path, wallet: &WalletId) -> Claims {
    let Ok(spent) = spent_outpoints(db, wallet) else {
        return Claims::default();
    };
    let records = super::chain::stored_history(db, wallet).unwrap_or_default();
    claims_of(&spent, &records)
}

fn claims_of(spent: &BTreeSet<OutPoint>, records: &[TransactionRecord]) -> Claims {
    let mut spender: BTreeMap<OutPoint, Txid> = BTreeMap::new();
    for record in records {
        // A block alone may still be reorganized out, and its spend with it: claimed at every
        // start, the coin would stay hidden until a resync. A ChainLock is final, and in Dash a
        // block gets one within seconds.
        if !record.context.is_chain_locked() {
            continue;
        }
        for input in &record.transaction.input {
            spender.insert(input.previous_output, record.txid);
        }
    }
    let mut claims = Claims::default();
    for outpoint in spent {
        match spender.get(outpoint) {
            Some(txid) => claims.claimed.push((*outpoint, Some(*txid))),
            None => claims.not_in_block += 1,
        }
    }
    claims
}

/// `core_utxos` rows with `spent = 1`; the outpoint is the store's bincode of it.
fn spent_outpoints(db: &Path, wallet: &WalletId) -> Result<BTreeSet<OutPoint>, String> {
    use rusqlite::OpenFlags;
    let conn = rusqlite::Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT outpoint FROM core_utxos WHERE wallet_id = ?1 AND spent = 1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![wallet.as_slice()], |row| {
            row.get::<_, Vec<u8>>(0)
        })
        .map_err(|e| e.to_string())?;
    let config = bincode::config::standard().with_limit::<64>();
    Ok(rows
        .filter_map(Result::ok)
        .filter_map(|blob| {
            bincode::serde::decode_from_slice::<OutPoint, _>(&blob, config)
                .ok()
                .filter(|(_, used)| *used == blob.len())
                .map(|(outpoint, _)| outpoint)
        })
        .collect())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests;
