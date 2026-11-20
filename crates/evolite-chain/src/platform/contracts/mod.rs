//! The network's contracts, kept on this computer (FEAT-0031 AC-T-C5, design
//! `docs/tasks/features/FEAT-0031/C5-contracts-cache-design.md`, review
//! `docs/tasks/reviews/FEAT-0031-C5-design.md`): one SQLite file per network,
//! `<data folder>/contracts/<network>/contracts.db`, shared by its wallets — contracts are public.
//!
//! - **One row per contract**, its latest version proved: Platform's own serialization of it, the
//!   protocol it was decoded at, when its version was last checked. Only proved answers are put in
//!   ([`ContractCache::put`]); no proofs are kept.
//! - **The disk is not a proof** (review, condition 1): a row read from the file is trusted for
//!   reading, not for a write — [`Cached::proved`] says whether the body came with a proof in this
//!   run. A write, and anything that counts money or amounts, takes a proved one
//!   (`Chain::fresh_proved`).
//! - **Derived at the write:** the contract's tokens (`token`: id → contract, position) and, from
//!   FEAT-0031 stage 1, the document types that fit the NFT profile (`nft_match`).
//! - **The provider** finds a contract here after the system ones and those registered in this
//!   run (`spv/provider.rs`); it never waits for a write ([`ContractCache::try_get`]).
//! - **A file that does not open** is set aside as `contracts.db.unreadable-<unix>` with a journal
//!   line, as BUG-235's caches: the network fills a new one.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dpp::data_contract::DataContract;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::accessors::v1::DataContractV1Getters;
use dpp::prelude::Identifier;
use rusqlite::{Connection, OptionalExtension, params};

pub mod fresh;

/// How many decoded contracts are kept in memory (review, condition 4); the rest stay as bytes.
pub const DECODED: usize = 256;
/// The version of the NFT profile ([`crate::platform::workshop_read::profile`]) the `nft_match`
/// rows were checked by: rows of another are checked again.
pub const PROFILE_VERSION: i64 = 1;

/// How long a read waits for a write before it gives up (review, condition 2): the provider is on
/// the proof path and never waits; the SDK reads the contract itself then.
const BUSY: Duration = Duration::from_millis(50);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS contract (
    id BLOB PRIMARY KEY,
    version INTEGER NOT NULL,
    bytes BLOB NOT NULL,
    protocol INTEGER NOT NULL,
    checked_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS token (
    token_id BLOB PRIMARY KEY,
    contract_id BLOB NOT NULL,
    position INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS token_by_contract ON token(contract_id);
CREATE TABLE IF NOT EXISTS nft_match (
    contract_id BLOB NOT NULL,
    document_type TEXT NOT NULL,
    profile_version INTEGER NOT NULL,
    PRIMARY KEY (contract_id, document_type)
);
CREATE TABLE IF NOT EXISTS pass (
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL
);
";

/// The cache's file for the network whose SPV data is in `spv` (`<data>/spv/<network>`):
/// `<data>/contracts/<network>/contracts.db`.
pub fn path_beside_spv(spv: &Path) -> PathBuf {
    let network = spv.file_name().map(|n| n.to_owned()).unwrap_or_default();
    spv.parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
        .join("contracts")
        .join(network)
        .join("contracts.db")
}

/// A contract as the cache has it.
#[derive(Clone, Debug)]
pub struct Cached {
    pub version: u32,
    pub contract: Arc<DataContract>,
    /// When its version was last checked against Platform (unix seconds); 0: to check.
    pub checked_at: i64,
    /// Its body came with a proof in this run. A row read from the disk does not: it may be read
    /// by, never written with (review, condition 1).
    pub proved: bool,
    /// Its version was checked against Platform for this answer (or within the window asked).
    /// Not the same as `proved` (review condition 8): a check that failed leaves a body proved
    /// earlier in the run, and a body from the disk may have its version checked.
    pub version_checked: bool,
}

impl Cached {
    /// The configuration of `token_id`, only when the contract says that token is at its
    /// position: a position changed on the disk cannot make another token's settings this
    /// one's (review, condition 1).
    pub fn token_configuration(
        &self,
        token_id: &Identifier,
    ) -> Option<dpp::data_contract::TokenConfiguration> {
        self.contract
            .tokens()
            .iter()
            .find(|(position, _)| self.contract.token_id(**position).as_ref() == Some(token_id))
            .map(|(_, config)| config.clone())
    }
}

/// What a contract needs before it is used (design «fresh», review condition 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// The one kept: checked within the window, and proved when that is asked.
    Use,
    /// One request for its latest version; the body only if it grew.
    CheckVersion,
    /// The body, with its proof: not kept, or kept only on the disk and a write needs it proved.
    Fetch,
}

/// The rule before use: `within` seconds since the last check is fresh enough (0: always check);
/// `proved` (a write, or anything that counts money) never takes a body read from the disk.
pub fn decide(cached: Option<&Cached>, now: i64, within: i64, proved: bool) -> Next {
    match cached {
        None => Next::Fetch,
        Some(c) if proved && !c.proved => Next::Fetch,
        Some(c) if now.saturating_sub(c.checked_at) < within => Next::Use,
        Some(_) => Next::CheckVersion,
    }
}

/// The decoded contracts, the most recently used last, at most [`DECODED`].
#[derive(Default)]
struct Decoded {
    map: HashMap<Identifier, Cached>,
    order: VecDeque<Identifier>,
}

impl Decoded {
    fn get(&mut self, id: &Identifier) -> Option<Cached> {
        let found = self.map.get(id).cloned()?;
        self.order.retain(|o| o != id);
        self.order.push_back(*id);
        Some(found)
    }

    fn peek(&self, id: &Identifier) -> Option<Cached> {
        self.map.get(id).cloned()
    }

    fn put(&mut self, id: Identifier, cached: Cached) {
        self.order.retain(|o| *o != id);
        self.order.push_back(id);
        self.map.insert(id, cached);
        while self.order.len() > DECODED {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    fn update(&mut self, id: &Identifier, change: impl FnOnce(&mut Cached)) {
        if let Some(c) = self.map.get_mut(id) {
            change(c);
        }
    }
}

/// The network's contracts (see the module). Cheap to share (`Arc`); every method is sync.
pub struct ContractCache {
    path: PathBuf,
    /// Writes, one at a time.
    writer: Mutex<Option<Connection>>,
    /// Reads, the provider's among them; a write never holds it (WAL).
    reader: Mutex<Option<Connection>>,
    decoded: Mutex<Decoded>,
    /// Contracts whose body came with a proof in this run.
    proved: Mutex<HashSet<Identifier>>,
    /// One flight per contract while the network is asked about it ([`fresh`], condition 7).
    flights: Mutex<HashMap<Identifier, Arc<tokio::sync::Mutex<()>>>>,
}

impl ContractCache {
    /// The cache at `path`, opened at its first use.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            writer: Mutex::new(None),
            reader: Mutex::new(None),
            decoded: Mutex::new(Decoded::default()),
            proved: Mutex::new(HashSet::new()),
            flights: Mutex::new(HashMap::new()),
        }
    }

    /// The flight of contract `id`: held while the network is asked about it.
    pub(crate) fn flight(&self, id: &Identifier) -> Arc<tokio::sync::Mutex<()>> {
        self.flights
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(*id)
            .or_default()
            .clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The contract `id` as kept: from memory, else from the file (not proved in this run).
    pub fn get(&self, id: &Identifier) -> Option<Cached> {
        if let Some(found) = self
            .decoded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
        {
            return Some(found);
        }
        let mut reader = self.reader.lock().unwrap_or_else(|e| e.into_inner());
        self.read_row(&mut reader, id)
    }

    /// [`Self::get`] that never waits (the provider, on the proof path): `None` when the memory
    /// or the file is busy — the SDK then reads the contract itself.
    pub fn try_get(&self, id: &Identifier) -> Option<Cached> {
        if let Ok(decoded) = self.decoded.try_lock()
            && let Some(found) = decoded.peek(id)
        {
            return Some(found);
        }
        let mut reader = self.reader.try_lock().ok()?;
        self.read_row(&mut reader, id)
    }

    fn read_row(&self, reader: &mut Option<Connection>, id: &Identifier) -> Option<Cached> {
        let conn = self.connection(reader)?;
        let row: Option<(u32, Vec<u8>, i64)> = conn
            .query_row(
                "SELECT version, bytes, checked_at FROM contract WHERE id = ?1",
                params![id.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .ok()
            .flatten();
        let (version, bytes, checked_at) = row?;
        // Decoded untrusted and validated in full (`contract_from_bytes`): the file is on the
        // disk, not a proof. One that does not decode is read from the network again.
        let contract = crate::platform::tokens::info::contract_from_bytes(&bytes)?;
        if contract.id() != *id || contract.version() != version {
            return None;
        }
        let cached = Cached {
            version,
            contract: Arc::new(contract),
            checked_at,
            proved: self
                .proved
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(id),
            version_checked: false,
        };
        if let Ok(mut decoded) = self.decoded.try_lock() {
            decoded.put(*id, cached.clone());
        }
        Some(cached)
    }

    /// Keeps a contract that came with a proof, checked `now` (unix seconds): its row, its tokens,
    /// proved for this run. The only way a contract gets in.
    pub fn put(&self, contract: Arc<DataContract>, now: i64) -> Result<Cached, String> {
        self.put_with_pass(vec![contract], now, None)
            .map(|mut kept| kept.pop().expect("one put, one kept"))
    }

    /// Keeps `contracts` and, in the same transaction, the pass row `pass` (key, value): a crawl
    /// cut between a page's contracts and where it stands keeps neither, and goes on from the
    /// last page kept whole (review condition 10, BUG-238).
    pub fn put_with_pass(
        &self,
        contracts: Vec<Arc<DataContract>>,
        now: i64,
        pass: Option<(&str, &[u8])>,
    ) -> Result<Vec<Cached>, String> {
        let protocol = dpp::version::PlatformVersion::latest().protocol_version;
        let mut rows = Vec::with_capacity(contracts.len());
        for contract in &contracts {
            let bytes = crate::platform::tokens::info::contract_bytes(contract)?;
            let tokens: Vec<(Identifier, u16)> = contract
                .tokens()
                .keys()
                .filter_map(|p| Some((contract.token_id(*p)?, *p)))
                .collect();
            rows.push((contract.id(), contract.version(), bytes, tokens));
        }
        {
            let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
            let conn = self
                .connection(&mut writer)
                .ok_or("the contracts' file does not open")?;
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            for (id, version, bytes, tokens) in &rows {
                tx.execute(
                    "INSERT INTO contract (id, version, bytes, protocol, checked_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(id) DO UPDATE SET version = excluded.version,
                        bytes = excluded.bytes, protocol = excluded.protocol,
                        checked_at = excluded.checked_at",
                    params![id.as_slice(), version, bytes, protocol, now],
                )
                .map_err(|e| e.to_string())?;
                tx.execute(
                    "DELETE FROM token WHERE contract_id = ?1",
                    params![id.as_slice()],
                )
                .map_err(|e| e.to_string())?;
                // A new body is checked against the profile again ([`Self::nft_contracts`]).
                tx.execute(
                    "DELETE FROM nft_match WHERE contract_id = ?1",
                    params![id.as_slice()],
                )
                .map_err(|e| e.to_string())?;
                for (token_id, position) in tokens {
                    tx.execute(
                        "INSERT OR REPLACE INTO token (token_id, contract_id, position)
                         VALUES (?1, ?2, ?3)",
                        params![token_id.as_slice(), id.as_slice(), position],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            if let Some((key, value)) = pass {
                tx.execute(
                    "INSERT OR REPLACE INTO pass (key, value) VALUES (?1, ?2)",
                    params![key, value],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
        }
        let mut kept = Vec::with_capacity(contracts.len());
        let mut proved = self.proved.lock().unwrap_or_else(|e| e.into_inner());
        let mut decoded = self.decoded.lock().unwrap_or_else(|e| e.into_inner());
        for contract in contracts {
            let id = contract.id();
            proved.insert(id);
            let cached = Cached {
                version: contract.version(),
                contract,
                checked_at: now,
                proved: true,
                version_checked: true,
            };
            decoded.put(id, cached.clone());
            kept.push(cached);
        }
        Ok(kept)
    }

    /// The pass row `key` as last kept with its page ([`Self::put_with_pass`]).
    pub fn pass(&self, key: &str) -> Option<Vec<u8>> {
        let mut reader = self.reader.lock().unwrap_or_else(|e| e.into_inner());
        self.connection(&mut reader)?
            .query_row("SELECT value FROM pass WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()
            .ok()
            .flatten()
    }

    /// Platform said `id` is still at the version kept, at `now`.
    pub fn checked(&self, id: &Identifier, now: i64) {
        self.set_checked_at(id, now);
    }

    /// A write changed `id` (a token's settings, an update): the next use reads it again.
    pub fn stale(&self, id: &Identifier) {
        self.proved
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.decoded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .update(id, |c| c.proved = false);
        self.set_checked_at(id, 0);
    }

    fn set_checked_at(&self, id: &Identifier, at: i64) {
        self.decoded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .update(id, |c| c.checked_at = at);
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(conn) = self.connection(&mut writer) {
            let _ = conn.execute(
                "UPDATE contract SET checked_at = ?2 WHERE id = ?1",
                params![id.as_slice(), at],
            );
        }
    }

    /// The contract and position of `token_id` as kept, without waiting for a write (review
    /// condition 9). For a write the pair is checked against the contract proved
    /// ([`fresh::token_for_write`]).
    pub fn token(&self, token_id: &Identifier) -> Option<(Identifier, u16)> {
        let mut reader = self.reader.try_lock().ok()?;
        let conn = self.connection(&mut reader)?;
        let (contract, position): (Vec<u8>, u16) = conn
            .query_row(
                "SELECT contract_id, position FROM token WHERE token_id = ?1",
                params![token_id.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten()?;
        Some((Identifier::from_bytes(&contract).ok()?, position))
    }

    /// The configuration of `token_id` from the contract kept, checked against the contract's own
    /// token id at that position (the provider; review, condition 2).
    pub fn token_configuration(
        &self,
        token_id: &Identifier,
    ) -> Option<dpp::data_contract::TokenConfiguration> {
        let (contract, _) = self.token(token_id)?;
        self.try_get(&contract)?.token_configuration(token_id)
    }

    /// FEAT-0031 stage 1: the contracts kept that fit the NFT profile, with their instance
    /// type. Each contract is checked once per [`PROFILE_VERSION`] and its verdict kept in
    /// `nft_match` (an empty type: it does not fit), so only those not checked yet are decoded.
    pub fn nft_contracts(&self) -> Vec<(Identifier, String)> {
        let unchecked: Vec<Vec<u8>> = {
            let mut reader = self.reader.lock().unwrap_or_else(|e| e.into_inner());
            let Some(conn) = self.connection(&mut reader) else {
                return Vec::new();
            };
            conn.prepare(
                "SELECT id FROM contract WHERE id NOT IN
                 (SELECT contract_id FROM nft_match WHERE profile_version = ?1)",
            )
            .and_then(|mut s| {
                s.query_map(params![PROFILE_VERSION], |r| r.get(0))?
                    .collect::<Result<_, _>>()
            })
            .unwrap_or_default()
        };
        let verdicts: Vec<(Vec<u8>, String)> = unchecked
            .into_iter()
            .filter_map(|bytes| {
                let id = Identifier::from_bytes(&bytes).ok()?;
                let kept = self.get(&id)?;
                let instance = crate::platform::workshop_read::profile(&kept.contract)
                    .map(|p| p.instance)
                    .unwrap_or_default();
                Some((bytes, instance))
            })
            .collect();
        if !verdicts.is_empty() {
            let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(conn) = self.connection(&mut writer)
                && let Ok(tx) = conn.transaction()
            {
                for (id, instance) in &verdicts {
                    let _ = tx.execute("DELETE FROM nft_match WHERE contract_id = ?1", params![id]);
                    let _ = tx.execute(
                        "INSERT INTO nft_match (contract_id, document_type, profile_version)
                         VALUES (?1, ?2, ?3)",
                        params![id, instance, PROFILE_VERSION],
                    );
                }
                let _ = tx.commit();
            }
        }
        let mut reader = self.reader.lock().unwrap_or_else(|e| e.into_inner());
        let Some(conn) = self.connection(&mut reader) else {
            return Vec::new();
        };
        conn.prepare(
            "SELECT contract_id, document_type FROM nft_match
             WHERE profile_version = ?1 AND document_type != ''",
        )
        .and_then(|mut s| {
            s.query_map(params![PROFILE_VERSION], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, t)| Some((Identifier::from_bytes(&id).ok()?, t)))
        .collect()
    }

    /// The contracts kept and the bytes of their bodies (the journal, review condition 4).
    pub fn counts(&self) -> (u64, u64) {
        let mut reader = self.reader.lock().unwrap_or_else(|e| e.into_inner());
        self.connection(&mut reader)
            .and_then(|c| {
                c.query_row(
                    "SELECT COUNT(*), COALESCE(SUM(LENGTH(bytes)), 0) FROM contract",
                    [],
                    |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
                )
                .ok()
            })
            .unwrap_or((0, 0))
    }

    /// Lets the file go (before «Sync again» deletes it, review condition 5): the next use opens
    /// it again.
    pub fn close(&self) {
        *self.writer.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.reader.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The open connection in `slot`, opened now if it is not; a file that does not open is set
    /// aside once and a new one made.
    fn connection<'c>(&self, slot: &'c mut Option<Connection>) -> Option<&'c mut Connection> {
        if slot.is_none() {
            *slot = open(&self.path)
                .or_else(|_| {
                    set_aside(&self.path);
                    open(&self.path)
                })
                .ok();
        }
        slot.as_mut()
    }
}

/// The file, made with its tables if new, in WAL mode (reads never wait for a write).
fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = crate::folders::create(dir);
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// Moves a file that does not open, with its `-wal` and `-shm`, to `<name>.unreadable-<unix>`.
fn set_aside(path: &Path) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    crate::diag::record(crate::diag::Record::Storage {
        file: "contracts.db".into(),
        op: "set aside",
        error: "open",
    });
    for suffix in ["", "-wal", "-shm"] {
        let mut from = path.as_os_str().to_owned();
        from.push(suffix);
        let mut to = from.clone();
        to.push(format!(".unreadable-{at}"));
        let _ = std::fs::rename(&from, &to);
    }
}

#[cfg(test)]
mod tests;
