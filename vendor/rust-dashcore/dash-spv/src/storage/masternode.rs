use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::sml_engine::{qr_info_diffs, MasternodeListEngine, WORK_DIFF_DEPTH};
use dashcore::consensus::{deserialize, serialize, Decodable, Encodable};
use dashcore::network::message_qrinfo::QRInfo;
use dashcore::network::message_sml::MnListDiff;
use dashcore::prelude::CoreBlockHeight;
use dashcore::sml::llmq_type::network::NetworkLLMQExt;
use dashcore::sml::llmq_type::LLMQType;
use dashcore::sml::quorum_entry::qualified_quorum_entry::QualifiedQuorumEntry;
use dashcore::{BlockHash, Network, QuorumHash};

use crate::error::{StorageError, StorageResult};
use crate::storage::{io::atomic_write, BlockHeaderStorage};

type IndexMap = BTreeMap<CoreBlockHeight, PathBuf>;

/// evolite (TECH-spv-prune): more stored QRInfos than this, and the next one is
/// asked from genesis. Each QRInfo is asked on a list the one before built, so
/// without that the stored messages chain back to the first QRInfo and none can
/// go.
pub(crate) const QRINFO_CUT_AT: usize = 8;

/// evolite (TECH-spv-prune): what this process's masternode storage did, for
/// the embedder's journal: counts and milliseconds only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MasternodeStorageStats {
    /// Opening the storage (headers, filters, blocks, the masternode index).
    pub storage_ms: u64,
    /// Replaying the stored masternode messages into the engine.
    pub load_engine_ms: u64,
    /// Messages stored when the engine was loaded.
    pub messages: usize,
    /// Messages pruned since the process started.
    pub pruned: usize,
    /// The quorums of the newest list after the replay: verified, and not.
    pub quorums_at_start: (usize, usize),
    /// The same once the first masternode sync of the process completed.
    pub quorums_synced: Option<(usize, usize)>,
}

static STATS: std::sync::Mutex<MasternodeStorageStats> =
    std::sync::Mutex::new(MasternodeStorageStats {
        storage_ms: 0,
        load_engine_ms: 0,
        messages: 0,
        pruned: 0,
        quorums_at_start: (0, 0),
        quorums_synced: None,
    });

/// What this process's masternode storage did so far ([`MasternodeStorageStats`]).
pub fn masternode_storage_stats() -> MasternodeStorageStats {
    *STATS.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn note_storage_open(storage_ms: u64) {
    STATS.lock().unwrap_or_else(|e| e.into_inner()).storage_ms = storage_ms;
}

/// The newest list's quorums once the first masternode sync completed.
pub(crate) fn note_synced_quorums(counts: (usize, usize)) {
    STATS.lock().unwrap_or_else(|e| e.into_inner()).quorums_synced.get_or_insert(counts);
}

enum Message {
    Diff(Box<MnListDiff>),
    QrInfo(Box<QRInfo>),
}

/// At one height a diff replays before a QRInfo.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Diff,
    QrInfo,
}

#[async_trait]
pub trait MasternodeStorage: Send + Sync + 'static {
    async fn store_diff(&mut self, height: CoreBlockHeight, diff: &MnListDiff)
        -> StorageResult<()>;

    async fn store_qr_info(
        &mut self,
        height: CoreBlockHeight,
        qr_info: &QRInfo,
    ) -> StorageResult<()>;

    /// Drops the messages stored above `height`.
    async fn truncate_above(&mut self, height: CoreBlockHeight) -> StorageResult<()>;
}

pub struct PersistentMasternodeStorage<H: BlockHeaderStorage> {
    storage_path: PathBuf,
    headers: Arc<RwLock<H>>,
    network: Network,
    diffs: IndexMap,
    qr_infos: IndexMap,
    /// evolite (SPV P0): a replay checks every stored message against its block's
    /// coinbase, as the engine does live (#1200). Off only in tests of the replay's
    /// own mechanics, whose made-up messages prove nothing.
    #[cfg(test)]
    validate: bool,
}

/// The stored messages as of when it was taken. A replay runs on this, so the
/// caller can release the storage lock before it starts.
pub struct MessageLog<H: BlockHeaderStorage> {
    headers: Arc<RwLock<H>>,
    network: Network,
    diffs: IndexMap,
    qr_infos: IndexMap,
    #[cfg(test)]
    validate: bool,
}

impl<H: BlockHeaderStorage> PersistentMasternodeStorage<H> {
    const FOLDER_NAME: &str = "masternodes";
    const DIFF_PREFIX: &str = "diff_";
    const QRINFO_PREFIX: &str = "qrinfo_";
    const EXTENSION: &str = "dat";

    pub async fn open(
        storage_path: impl Into<PathBuf> + Send,
        headers: Arc<RwLock<H>>,
        network: Network,
    ) -> StorageResult<Self> {
        let storage_path = storage_path.into();
        let (diffs, qr_infos) = Self::index_folder(&storage_path.join(Self::FOLDER_NAME)).await?;

        Ok(PersistentMasternodeStorage {
            storage_path,
            headers,
            network,
            diffs,
            qr_infos,
            #[cfg(test)]
            validate: true,
        })
    }

    /// For tests of the replay's mechanics on made-up messages (evolite, SPV P0).
    #[cfg(test)]
    pub(crate) fn without_validation(mut self) -> Self {
        self.validate = false;
        self
    }

    fn folder(&self) -> PathBuf {
        self.storage_path.join(Self::FOLDER_NAME)
    }

    fn file_name(prefix: &str, height: CoreBlockHeight) -> String {
        format!("{prefix}{height}.{}", Self::EXTENSION)
    }

    fn height_from_file_name(name: &str, prefix: &str) -> Option<CoreBlockHeight> {
        name.strip_prefix(prefix)?.strip_suffix(&format!(".{}", Self::EXTENSION))?.parse().ok()
    }

    async fn index_folder(folder: &Path) -> StorageResult<(IndexMap, IndexMap)> {
        let mut diffs = BTreeMap::new();
        let mut qr_infos = BTreeMap::new();

        if !folder.exists() {
            return Ok((diffs, qr_infos));
        }

        let mut entries = tokio::fs::read_dir(folder).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(height) = Self::height_from_file_name(name, Self::DIFF_PREFIX) {
                diffs.insert(height, path);
            } else if let Some(height) = Self::height_from_file_name(name, Self::QRINFO_PREFIX) {
                qr_infos.insert(height, path);
            }
        }

        Ok((diffs, qr_infos))
    }

    async fn store_message<T: Encodable + Sync>(
        folder: &Path,
        index: &mut IndexMap,
        prefix: &str,
        height: CoreBlockHeight,
        message: &T,
    ) -> StorageResult<()> {
        tokio::fs::create_dir_all(folder).await?;
        let path = folder.join(Self::file_name(prefix, height));
        atomic_write(&path, &serialize(message)).await?;
        index.insert(height, path);
        Ok(())
    }

    async fn read_message<T: Decodable>(path: &Path) -> StorageResult<T> {
        let bytes = tokio::fs::read(path).await?;
        deserialize(&bytes).map_err(|e| {
            StorageError::Corruption(format!("Failed to decode {}: {e}", path.display()))
        })
    }

    async fn read_entry(path: &Path, kind: Kind) -> StorageResult<Message> {
        Ok(match kind {
            Kind::Diff => Message::Diff(Box::new(Self::read_message(path).await?)),
            Kind::QrInfo => Message::QrInfo(Box::new(Self::read_message(path).await?)),
        })
    }

    pub fn message_log(&self) -> MessageLog<H> {
        MessageLog {
            headers: Arc::clone(&self.headers),
            network: self.network,
            diffs: self.diffs.clone(),
            qr_infos: self.qr_infos.clone(),
            #[cfg(test)]
            validate: self.validate,
        }
    }

    pub(crate) async fn load_engine(&self) -> MasternodeListEngine<H> {
        let started = std::time::Instant::now();
        let engine = self.message_log().replay(CoreBlockHeight::MAX, |_| {}).await;
        let mut stats = STATS.lock().unwrap_or_else(|e| e.into_inner());
        stats.load_engine_ms = started.elapsed().as_millis() as u64;
        stats.messages = self.diffs.len() + self.qr_infos.len();
        stats.quorums_at_start = engine.quorum_counts();
        engine
    }

    /// evolite (TECH-spv-prune): the QRInfos stored ([`QRINFO_CUT_AT`]).
    #[cfg(test)]
    pub(crate) fn qr_info_count(&self) -> usize {
        self.qr_infos.len()
    }

    /// evolite (TECH-spv-prune, review r1 M1): whether the next QRInfo is asked
    /// from genesis: more than [`QRINFO_CUT_AT`] stored, and the newest stored
    /// one is not from genesis already (a cut whose pruning has not run asks no
    /// second one).
    pub(crate) async fn cut_due(&self) -> bool {
        if self.qr_infos.len() <= QRINFO_CUT_AT {
            return false;
        }
        let Some((_, path)) = self.qr_infos.last_key_value() else {
            return false;
        };
        let Ok(newest) = Self::read_message::<QRInfo>(path).await else {
            return true;
        };
        !self.from_genesis(&qr_info_bases(&newest).into_iter().collect::<Vec<_>>()).await
    }

    /// Whether every one of `bases` is genesis: the zero hash or a block at 0.
    async fn from_genesis(&self, bases: &[BlockHash]) -> bool {
        use dashcore_hashes::Hash;
        let headers = self.headers.read().await;
        for base in bases {
            if *base == BlockHash::all_zeros() {
                continue;
            }
            if headers.get_header_height_by_hash(base).await.ok().flatten() != Some(0) {
                return false;
            }
        }
        true
    }

    /// evolite (TECH-spv-prune): deletes the stored messages a replay no
    /// longer needs, answering how many. Once a QRInfo from genesis is stored
    /// (the cut), only it and the messages applied on what it and they build
    /// (or on genesis) can stay: everything from before the cut goes at once,
    /// whatever its height (review r1 M1). Of those, a list is needed when it
    /// lies in the engine's retention window below the newest list (QRInfo
    /// bases, the work blocks of active quorums, Platform's walk-back), or when
    /// a kept message is applied on it; it is kept through the newest message
    /// that builds it, and so are that message's own bases. Everything else
    /// goes. A message whose block is not in the header chain, or a file that
    /// does not read, stops the pruning: nothing it cannot judge is deleted. A
    /// file goes before its index entry, so a failure leaves more stored, never
    /// less.
    pub(crate) async fn prune(&mut self) -> StorageResult<usize> {
        struct Entry {
            height: CoreBlockHeight,
            kind: Kind,
            built: Vec<BlockHash>,
            bases: Vec<BlockHash>,
        }
        let mut entries = Vec::new();
        for (kind, index) in [(Kind::Diff, &self.diffs), (Kind::QrInfo, &self.qr_infos)] {
            for (height, path) in index {
                let Ok(message) = Self::read_entry(path, kind).await else {
                    return Ok(0);
                };
                entries.push(Entry {
                    height: *height,
                    kind,
                    built: message.built_hashes(),
                    bases: message.base_hashes(),
                });
            }
        }
        // Newest first: the first message building a list is the one kept for it.
        entries.sort_by_key(|e| std::cmp::Reverse((e.height, e.kind)));

        let mut height_of: HashMap<BlockHash, CoreBlockHeight> = HashMap::new();
        {
            let headers = self.headers.read().await;
            for entry in &entries {
                for hash in &entry.built {
                    if height_of.contains_key(hash) {
                        continue;
                    }
                    match headers.get_header_height_by_hash(hash).await {
                        Ok(Some(height)) => {
                            height_of.insert(*hash, height);
                        }
                        _ => return Ok(0),
                    }
                }
            }
        }
        let Some(newest) = height_of.values().copied().max() else {
            return Ok(0);
        };
        let floor = newest.saturating_sub(crate::sml_engine::retention_window(self.network));

        // The newest QRInfo from genesis, and what is applied on it forward.
        let mut genesis = Vec::with_capacity(entries.len());
        for entry in &entries {
            genesis.push(self.from_genesis(&entry.bases).await);
        }
        let mut eligible = vec![true; entries.len()];
        if let Some(cut) =
            entries.iter().zip(&genesis).position(|(e, g)| e.kind == Kind::QrInfo && *g)
        {
            eligible = vec![false; entries.len()];
            eligible[cut] = true;
            let mut built: HashSet<BlockHash> = entries[cut].built.iter().copied().collect();
            loop {
                let mut grew = false;
                for (i, entry) in entries.iter().enumerate() {
                    // Another QRInfo from genesis is an older cut, not applied on this one.
                    let older_cut = entry.kind == Kind::QrInfo && genesis[i];
                    if eligible[i] || older_cut {
                        continue;
                    }
                    let on_kept = genesis[i] || entry.bases.iter().all(|b| built.contains(b));
                    if on_kept {
                        eligible[i] = true;
                        built.extend(entry.built.iter().copied());
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
        }

        let mut provider: HashMap<BlockHash, usize> = HashMap::new();
        for (i, entry) in entries.iter().enumerate() {
            if !eligible[i] {
                continue;
            }
            for hash in &entry.built {
                provider.entry(*hash).or_insert(i);
            }
        }
        let mut kept = vec![false; entries.len()];
        let mut needed: Vec<BlockHash> = height_of
            .iter()
            .filter(|(_, height)| **height >= floor)
            .map(|(hash, _)| *hash)
            .collect();
        while let Some(hash) = needed.pop() {
            let Some(&i) = provider.get(&hash) else {
                continue;
            };
            if !kept[i] {
                kept[i] = true;
                needed.extend(entries[i].bases.iter().copied());
            }
        }

        let mut pruned = 0;
        for (entry, keep) in entries.iter().zip(kept) {
            if keep {
                continue;
            }
            let index = match entry.kind {
                Kind::Diff => &mut self.diffs,
                Kind::QrInfo => &mut self.qr_infos,
            };
            if let Some(path) = index.get(&entry.height) {
                match tokio::fs::remove_file(path).await {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
                index.remove(&entry.height);
                pruned += 1;
            }
        }
        if pruned > 0 {
            STATS.lock().unwrap_or_else(|e| e.into_inner()).pruned += pruned;
            tracing::info!(
                "Pruned {pruned} masternode messages the replay no longer needs, {} kept",
                self.diffs.len() + self.qr_infos.len()
            );
        }
        Ok(pruned)
    }
}

impl<H: BlockHeaderStorage> MessageLog<H> {
    /// Rebuilds the engine from the messages stored up to `until`, calling
    /// `visit` after each one. Messages apply in the order of their newest
    /// base, so each comes after the one that built the list it extends. They
    /// are read once to plan that order and again to apply, so only one is held
    /// in memory at a time, and the header storage is locked per message.
    async fn replay(
        &self,
        until: CoreBlockHeight,
        mut visit: impl FnMut(&MasternodeListEngine<H>),
    ) -> MasternodeListEngine<H> {
        // Checked as live (#1200): a stored message that fails is «does not apply»
        // below, and the client asks the network for the lists again.
        let mut engine = MasternodeListEngine::new(self.network, Arc::clone(&self.headers));
        #[cfg(test)]
        if !self.validate {
            engine = MasternodeListEngine::new_without_validation(
                self.network,
                Arc::clone(&self.headers),
            );
        }

        let entries =
            self.diffs.range(..=until).map(|(height, path)| (*height, Kind::Diff, path)).chain(
                self.qr_infos.range(..=until).map(|(height, path)| (*height, Kind::QrInfo, path)),
            );

        let mut plan = Vec::new();
        for (height, kind, path) in entries {
            let Some(message) = self.read_entry(height, kind, path).await else {
                continue;
            };
            let mut newest_base = None;
            for base in message.base_hashes() {
                newest_base = newest_base.max(engine.height_if_known(&base).await);
            }
            plan.push((newest_base.unwrap_or(height), height, kind, path));
        }
        plan.sort_by_key(|(newest_base, height, kind, _)| (*newest_base, *height, *kind));

        let total = plan.len();
        let mut applied = 0;
        for (_, height, kind, path) in plan {
            if let Some(message) = self.read_entry(height, kind, path).await {
                match message.apply(&mut engine).await {
                    Ok(()) => applied += 1,
                    Err(e) => tracing::warn!("Masternode message at {height} does not apply: {e}"),
                }
            }
            visit(&engine);
        }

        tracing::debug!(
            "Replayed {applied}/{total} masternode messages into {} masternode lists",
            engine.masternode_lists.len()
        );

        engine
    }

    /// Reads the message stored at `height` while its block is still in the
    /// header chain there, or logs why it is skipped.
    async fn read_entry(
        &self,
        height: CoreBlockHeight,
        kind: Kind,
        path: &Path,
    ) -> Option<Message> {
        let message = match PersistentMasternodeStorage::<H>::read_entry(path, kind).await {
            Ok(message) => message,
            Err(e) => {
                tracing::warn!("Skipping unreadable masternode message at {height}: {e}");
                return None;
            }
        };
        let block_hash = message.block_hash();
        let stored_height =
            self.headers.read().await.get_header_height_by_hash(&block_hash).await.ok().flatten();
        if stored_height != Some(height) {
            tracing::warn!(
                "Skipping masternode message at {height}: {block_hash} is no longer in the \
                 header chain"
            );
            return None;
        }
        Some(message)
    }

    /// The quorum as the engine would have resolved it at `height` before its
    /// retention window dropped the lists holding it. A QRInfo builds lists
    /// back to its h-4c work block, so messages stored up to six rotation
    /// cycles above `height` still count.
    pub async fn quorum_entry_at_or_before(
        &self,
        llmq_type: LLMQType,
        quorum_hash: QuorumHash,
        height: CoreBlockHeight,
    ) -> Option<QualifiedQuorumEntry> {
        let cycle = self.network.isd_llmq_type().params().dkg_params.interval;
        let until = height.saturating_add(cycle.saturating_mul(6)).saturating_add(WORK_DIFF_DEPTH);
        let mut found: Option<(CoreBlockHeight, QualifiedQuorumEntry)> = None;
        self.replay(until, |engine| {
            let hit =
                engine.quorum_entry_for_hash_at_or_before_height(llmq_type, quorum_hash, height);
            if let Some((list_height, quorum)) = hit {
                if found.as_ref().is_none_or(|(best, _)| list_height >= *best) {
                    found = Some((list_height, quorum.clone()));
                }
            }
        })
        .await;
        found.map(|(_, quorum)| quorum)
    }
}

impl Message {
    /// Feeds the message to `engine`, failing with why it was not taken.
    async fn apply<H: BlockHeaderStorage>(
        self,
        engine: &mut MasternodeListEngine<H>,
    ) -> Result<(), String> {
        match self {
            Message::QrInfo(qr_info) => {
                engine.feed_qr_info(*qr_info).await.map(drop).map_err(|e| e.to_string())
            }
            Message::Diff(diff) => engine.apply_diff(*diff).await.map_err(|e| e.to_string()),
        }
    }

    fn block_hash(&self) -> BlockHash {
        match self {
            Message::Diff(diff) => diff.block_hash,
            Message::QrInfo(qr_info) => qr_info.mn_list_diff_tip.block_hash,
        }
    }

    /// evolite (TECH-spv-prune): block hashes of the lists this message builds.
    fn built_hashes(&self) -> Vec<BlockHash> {
        match self {
            Message::Diff(diff) if diff.block_hash != diff.base_block_hash => vec![diff.block_hash],
            Message::Diff(_) => Vec::new(),
            Message::QrInfo(qr_info) => qr_info_built(qr_info).into_iter().collect(),
        }
    }

    /// Block hashes of the lists this message is applied on top of. A QRInfo's
    /// diffs chain from its oldest one, which Core sends as an empty diff from
    /// the base to itself, so only bases no other diff of it produces count.
    fn base_hashes(&self) -> Vec<BlockHash> {
        match self {
            Message::Diff(diff) => vec![diff.base_block_hash],
            Message::QrInfo(qr_info) => qr_info_bases(qr_info).into_iter().collect(),
        }
    }
}

/// Block hashes of the lists a QRInfo's diffs build.
pub(crate) fn qr_info_built(qr_info: &QRInfo) -> HashSet<BlockHash> {
    qr_info_diffs(qr_info)
        .iter()
        .filter(|d| d.block_hash != d.base_block_hash)
        .map(|d| d.block_hash)
        .collect()
}

/// Block hashes of the lists a QRInfo is applied on top of.
pub(crate) fn qr_info_bases(qr_info: &QRInfo) -> HashSet<BlockHash> {
    let built = qr_info_built(qr_info);
    qr_info_diffs(qr_info)
        .iter()
        .map(|d| d.base_block_hash)
        .filter(|base| !built.contains(base))
        .collect()
}

#[async_trait]
impl<H: BlockHeaderStorage> MasternodeStorage for PersistentMasternodeStorage<H> {
    async fn store_diff(
        &mut self,
        height: CoreBlockHeight,
        diff: &MnListDiff,
    ) -> StorageResult<()> {
        let folder = self.folder();
        Self::store_message(&folder, &mut self.diffs, Self::DIFF_PREFIX, height, diff).await
    }

    async fn store_qr_info(
        &mut self,
        height: CoreBlockHeight,
        qr_info: &QRInfo,
    ) -> StorageResult<()> {
        // A QRInfo for the tip already stored, requested from a list the stored
        // one built, would replace the message it is applied on top of.
        if let Some(path) = self.qr_infos.get(&height) {
            if let Ok(stored) = Self::read_message::<QRInfo>(path).await {
                if stored.mn_list_diff_tip.block_hash == qr_info.mn_list_diff_tip.block_hash
                    && !qr_info_bases(qr_info).is_disjoint(&qr_info_built(&stored))
                {
                    return Ok(());
                }
            }
        }
        let folder = self.folder();
        Self::store_message(&folder, &mut self.qr_infos, Self::QRINFO_PREFIX, height, qr_info)
            .await?;
        // evolite (TECH-spv-prune): once it is written, what it makes redundant goes.
        if let Err(e) = self.prune().await {
            tracing::warn!("Could not prune the stored masternode messages: {e}");
        }
        Ok(())
    }

    async fn truncate_above(&mut self, height: CoreBlockHeight) -> StorageResult<()> {
        // A message leaves its index only once its file is gone, so a retry
        // finds what a failed deletion left.
        for index in [&mut self.diffs, &mut self.qr_infos] {
            while let Some(entry) = index.last_entry().filter(|entry| *entry.key() > height) {
                match tokio::fs::remove_file(entry.get()).await {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                    _ => entry.remove(),
                };
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::MockHeaderStorage;
    use dashcore_hashes::Hash;

    use tempfile::TempDir;
    use test_case::test_case;

    fn hash(byte: u8) -> BlockHash {
        BlockHash::from_slice(&[byte; 32]).unwrap()
    }

    async fn open_storage(
        dir: &TempDir,
        heights: &[(u8, u32)],
    ) -> PersistentMasternodeStorage<MockHeaderStorage> {
        let map = heights.iter().map(|(b, h)| (hash(*b), *h)).collect();
        PersistentMasternodeStorage::open(
            dir.path(),
            Arc::new(RwLock::new(MockHeaderStorage::new(map))),
            Network::Regtest,
        )
        .await
        .expect("open")
        .without_validation()
    }

    #[tokio::test]
    async fn truncating_drops_the_messages_stored_above() {
        let dir = TempDir::new().unwrap();
        let heights = [(0x00, 0), (0xAA, 100), (0xBB, 200)];
        let mut storage = open_storage(&dir, &heights).await;

        storage.store_diff(100, &MnListDiff::dummy(0x00, 0xAA)).await.unwrap();
        storage.store_diff(200, &MnListDiff::dummy(0xAA, 0xBB)).await.unwrap();
        storage.store_qr_info(200, &QRInfo::dummy(0xBB)).await.unwrap();

        storage.truncate_above(100).await.unwrap();

        let reopened = open_storage(&dir, &heights).await;
        assert_eq!(reopened.diffs.keys().copied().collect::<Vec<_>>(), vec![100]);
        assert!(reopened.qr_infos.is_empty());
    }

    #[tokio::test]
    async fn a_reopened_storage_replays_what_was_stored() {
        let dir = TempDir::new().unwrap();
        let heights = [(0x00, 0), (0xAA, 100), (0xBB, 200)];
        {
            let mut storage = open_storage(&dir, &heights).await;
            storage.store_diff(100, &MnListDiff::dummy(0x00, 0xAA)).await.unwrap();
            storage.store_diff(200, &MnListDiff::dummy(0xAA, 0xBB)).await.unwrap();
        }

        let engine = open_storage(&dir, &heights).await.load_engine().await;

        assert_eq!(engine.masternode_lists.keys().copied().collect::<Vec<_>>(), vec![100, 200]);
        assert_eq!(engine.masternode_lists[&200].block_hash, hash(0xBB));
        assert_eq!(engine.masternode_lists[&200].masternodes.len(), 2);
    }

    #[tokio::test]
    async fn replay_verifies_the_newest_lists_non_rotating_quorums() {
        use dashcore::bls_sig_utils::{BLSPublicKey, BLSSignature};
        use dashcore::hash_types::QuorumVVecHash;
        use dashcore::network::message_sml::QuorumCLSigObject;
        use dashcore::sml::llmq_entry_verification::{
            LLMQEntryVerificationSkipStatus, LLMQEntryVerificationStatus,
        };
        use dashcore::sml::llmq_type::LLMQType;
        use dashcore::transaction::special_transaction::quorum_commitment::QuorumEntry;
        use dashcore::QuorumHash;

        let dir = TempDir::new().unwrap();
        let quorum_hash = QuorumHash::from_byte_array([0xAB; 32]);
        let diff = MnListDiff {
            new_quorums: vec![QuorumEntry {
                version: 1,
                llmq_type: LLMQType::LlmqtypeTest,
                quorum_hash,
                quorum_index: None,
                signers: vec![true; 3],
                valid_members: vec![true; 3],
                quorum_public_key: BLSPublicKey::from([7; 48]),
                quorum_vvec_hash: QuorumVVecHash::all_zeros(),
                threshold_sig: BLSSignature::from([1; 96]),
                all_commitment_aggregated_signature: BLSSignature::from([1; 96]),
            }],
            quorums_chainlock_signatures: vec![QuorumCLSigObject {
                signature: BLSSignature::from([1; 96]),
                index_set: vec![0],
            }],
            ..MnListDiff::dummy(0x00, 0xAA)
        };
        let mut storage = open_storage(&dir, &[(0x00, 0), (0xAA, 100)]).await;
        storage.store_diff(100, &diff).await.unwrap();

        let engine = storage.load_engine().await;

        let status =
            &engine.masternode_lists[&100].quorums[&LLMQType::LlmqtypeTest][&quorum_hash].verified;
        assert_ne!(
            *status,
            LLMQEntryVerificationStatus::Skipped(
                LLMQEntryVerificationSkipStatus::NotMarkedForVerification
            ),
            "the replay never tried to verify it"
        );
    }

    /// A reorg truncates the headers above 100, but the diff stored for the
    /// orphaned block at 200 stays on disk until a new one replaces it.
    #[tokio::test]
    async fn replay_skips_a_message_whose_block_left_the_chain() {
        let dir = TempDir::new().unwrap();
        let mut storage = open_storage(&dir, &[(0x00, 0), (0xAA, 100)]).await;
        storage.store_diff(100, &MnListDiff::dummy(0x00, 0xAA)).await.unwrap();
        storage.store_diff(200, &MnListDiff::dummy(0xAA, 0xBB)).await.unwrap();

        let engine = storage.load_engine().await;

        assert_eq!(engine.masternode_lists.keys().copied().collect::<Vec<_>>(), vec![100]);
    }

    #[tokio::test]
    async fn replay_skips_an_orphan_and_keeps_going() {
        let dir = TempDir::new().unwrap();
        let mut storage =
            open_storage(&dir, &[(0x00, 0), (0xAA, 100), (0xBB, 150), (0xDD, 120)]).await;

        storage.store_diff(100, &MnListDiff::dummy(0x00, 0xAA)).await.unwrap();
        storage.store_diff(120, &MnListDiff::dummy(0xEE, 0xDD)).await.unwrap();
        storage.store_diff(150, &MnListDiff::dummy(0xAA, 0xBB)).await.unwrap();

        let engine = storage.load_engine().await;

        assert!(
            !engine.masternode_lists.contains_key(&120),
            "the orphan has no base to apply on and is left to the network"
        );
        assert!(engine.masternode_lists.contains_key(&150), "the diff after it still applies");
    }

    #[tokio::test]
    async fn storing_the_same_height_twice_keeps_the_newer_message() {
        let dir = TempDir::new().unwrap();
        let mut storage = open_storage(&dir, &[]).await;

        storage.store_diff(100, &MnListDiff::dummy(0x00, 0xAA)).await.unwrap();
        storage.store_diff(100, &MnListDiff::dummy(0x00, 0xBB)).await.unwrap();

        assert_eq!(storage.diffs.len(), 1, "one file per height");
        let stored: MnListDiff =
            PersistentMasternodeStorage::<MockHeaderStorage>::read_message(&storage.diffs[&100])
                .await
                .expect("read back");
        assert_eq!(stored.block_hash, hash(0xBB), "the second write wins");
    }

    /// A QRInfo chaining `start`, `start + 1`, `start + 2`, `start + 3` and
    /// `tip`, applied on top of the list at `start`.
    fn chained_qr_info(start: u8, tip: u8) -> QRInfo {
        QRInfo {
            mn_list_diff_at_h_minus_3c: MnListDiff::dummy_empty(start, start),
            mn_list_diff_at_h_minus_2c: MnListDiff::dummy_empty(start, start + 1),
            mn_list_diff_at_h_minus_c: MnListDiff::dummy_empty(start + 1, start + 2),
            mn_list_diff_h: MnListDiff::dummy_empty(start + 2, start + 3),
            mn_list_diff_tip: MnListDiff::dummy_empty(start + 3, tip),
            ..QRInfo::dummy(tip)
        }
    }

    /// The QRInfo stored at `height`, `None` when the file does not decode.
    async fn stored_qr_info(
        storage: &PersistentMasternodeStorage<MockHeaderStorage>,
        height: CoreBlockHeight,
    ) -> Option<QRInfo> {
        PersistentMasternodeStorage::<MockHeaderStorage>::read_message(&storage.qr_infos[&height])
            .await
            .ok()
    }

    #[test_case(chained_qr_info(0xA2, 0xE0), false, false; "keeps the stored one it builds on")]
    #[test_case(chained_qr_info(0x10, 0xE0), false, true; "replaces it when unrelated")]
    #[test_case(chained_qr_info(0xA2, 0xEF), false, true; "replaces it for another tip after a reorg")]
    #[test_case(chained_qr_info(0xA2, 0xE0), true, true; "replaces a stored file that does not decode")]
    #[tokio::test]
    async fn a_same_tip_qr_info_replaces_the_stored_one_unless_built_on_it(
        incoming: QRInfo,
        corrupt_stored: bool,
        replaces: bool,
    ) {
        let height = 100;
        let stored = chained_qr_info(0xA0, 0xE0);
        let dir = TempDir::new().unwrap();
        let mut storage = open_storage(&dir, &[]).await;
        storage.store_qr_info(height, &stored).await.unwrap();
        if corrupt_stored {
            tokio::fs::write(&storage.qr_infos[&height], b"not a qrinfo").await.unwrap();
        }

        storage.store_qr_info(height, &incoming).await.unwrap();

        let expected = if replaces {
            incoming
        } else {
            stored
        };
        assert_eq!(stored_qr_info(&storage, height).await, Some(expected));
    }

    #[tokio::test]
    async fn replay_survives_unreadable_files_and_ignores_foreign_names() {
        let dir = TempDir::new().unwrap();
        let folder = dir.path().join(PersistentMasternodeStorage::<MockHeaderStorage>::FOLDER_NAME);
        tokio::fs::create_dir_all(&folder).await.unwrap();

        {
            let mut storage = open_storage(&dir, &[(0x00, 0), (0xAA, 100)]).await;
            storage.store_diff(100, &MnListDiff::dummy(0x00, 0xAA)).await.unwrap();
        }

        tokio::fs::write(folder.join("diff_50.dat"), b"not a diff").await.unwrap();
        tokio::fs::write(folder.join("diff_abc.dat"), b"x").await.unwrap();

        let storage = open_storage(&dir, &[(0x00, 0), (0xAA, 100)]).await;
        assert_eq!(
            storage.diffs.keys().copied().collect::<Vec<_>>(),
            vec![50, 100],
            "only well-formed names are indexed"
        );
        assert!(storage.qr_infos.is_empty());

        let engine = storage.load_engine().await;
        assert!(
            engine.masternode_lists.contains_key(&100),
            "the readable message still rebuilds its list"
        );
    }

    /// A Platform quorum mined at 1 and retired at 2, then a diff per block up to
    /// past the retention window, written without fsync to keep the setup fast.
    #[tokio::test]
    async fn a_lookup_rebuilds_a_quorum_only_lists_out_of_the_window_held() {
        use dashcore::bls_sig_utils::{BLSPublicKey, BLSSignature};
        use dashcore::hash_types::QuorumVVecHash;
        use dashcore::network::message_sml::{DeletedQuorum, QuorumCLSigObject};
        use dashcore::transaction::special_transaction::quorum_commitment::QuorumEntry;
        type S = PersistentMasternodeStorage<MockHeaderStorage>;

        let tip = 2_500;
        let llmq_type = Network::Regtest.platform_type();
        let quorum_hash = QuorumHash::from_byte_array([0xAB; 32]);
        let mined = MnListDiff {
            block_hash: BlockHash::dummy(1),
            new_quorums: vec![QuorumEntry {
                version: 1,
                llmq_type,
                quorum_hash,
                quorum_index: None,
                signers: vec![true; 3],
                valid_members: vec![true; 3],
                quorum_public_key: BLSPublicKey::from([7; 48]),
                quorum_vvec_hash: QuorumVVecHash::all_zeros(),
                threshold_sig: BLSSignature::from([1; 96]),
                all_commitment_aggregated_signature: BLSSignature::from([1; 96]),
            }],
            quorums_chainlock_signatures: vec![QuorumCLSigObject {
                signature: BLSSignature::from([1; 96]),
                index_set: vec![0],
            }],
            ..MnListDiff::dummy(0x00, 0x01)
        };
        let retired = MnListDiff {
            deleted_quorums: vec![DeletedQuorum {
                llmq_type,
                quorum_hash,
            }],
            ..MnListDiff::dummy_between(1, 2)
        };

        let dir = TempDir::new().unwrap();
        let folder = dir.path().join(S::FOLDER_NAME);
        std::fs::create_dir_all(&folder).unwrap();
        let write = |height, diff: &MnListDiff| {
            std::fs::write(folder.join(S::file_name(S::DIFF_PREFIX, height)), serialize(diff))
                .unwrap()
        };
        write(1, &mined);
        write(2, &retired);
        for height in 3..=tip {
            write(height, &MnListDiff::dummy_between(height - 1, height));
        }
        let heights = (1..=tip).map(|height| (BlockHash::dummy(height), height)).collect();
        let storage = S::open(
            dir.path(),
            Arc::new(RwLock::new(MockHeaderStorage::new(heights))),
            Network::Regtest,
        )
        .await
        .unwrap()
        .without_validation();

        let engine = storage.load_engine().await;
        assert!(engine
            .quorum_entry_for_hash_at_or_before_height(llmq_type, quorum_hash, 100)
            .is_none());

        let log = storage.message_log();
        let quorum = log.quorum_entry_at_or_before(llmq_type, quorum_hash, 100).await;
        assert_eq!(quorum.map(|quorum| quorum.quorum_entry.quorum_hash), Some(quorum_hash));
        let unknown = QuorumHash::from_byte_array([0xCD; 32]);
        assert!(log.quorum_entry_at_or_before(llmq_type, unknown, 100).await.is_none());
    }

    #[test]
    fn qr_info_bases_are_what_its_chain_starts_from() {
        let mut qr_info = QRInfo::dummy(0x00);
        qr_info.mn_list_diff_at_h_minus_3c = MnListDiff::dummy_empty(0xA0, 0xA0);
        qr_info.mn_list_diff_at_h_minus_2c = MnListDiff::dummy_empty(0xA0, 0xB0);
        qr_info.mn_list_diff_at_h_minus_c = MnListDiff::dummy_empty(0xB0, 0xC0);
        qr_info.mn_list_diff_h = MnListDiff::dummy_empty(0xC0, 0xD0);
        qr_info.mn_list_diff_tip = MnListDiff::dummy_empty(0xD0, 0xE0);

        assert_eq!(
            Message::QrInfo(Box::new(qr_info)).base_hashes(),
            vec![hash(0xA0)],
            "the empty diff at the start of the chain needs its list to exist already"
        );
    }

    /// Mainnet headers start at a checkpoint, so the genesis a first QRInfo
    /// is built from has no height. A work-block diff requested from a list
    /// that QRInfo built still has to replay after it.
    #[tokio::test]
    async fn replay_applies_a_diff_on_a_list_of_a_qr_info_built_from_genesis() {
        use crate::sml_engine::test_support::fixture_heights;
        use dashcore::network::constants::NetworkExt;

        let qr_info: QRInfo = bincode::decode_from_slice(
            include_bytes!("../../../dash/tests/data/test_DML_diffs/qrinfo_2518986.dat"),
            bincode::config::standard(),
        )
        .unwrap()
        .0;

        let mut heights = fixture_heights(include_bytes!(
            "../../../dash/tests/data/test_DML_diffs/block_container_2518986.dat"
        ));
        heights.remove(&Network::Mainnet.known_genesis_block_hash().unwrap());
        let tip_height = heights[&qr_info.mn_list_diff_tip.block_hash];
        let base = qr_info.mn_list_diff_at_h_minus_c.block_hash;
        let work_block_height = heights[&base] + 24;
        heights.insert(hash(0xEE), work_block_height);

        let dir = TempDir::new().unwrap();
        let mut storage = PersistentMasternodeStorage::open(
            dir.path(),
            Arc::new(RwLock::new(MockHeaderStorage::new(heights))),
            Network::Mainnet,
        )
        .await
        .unwrap()
        .without_validation();
        storage.store_qr_info(tip_height, &qr_info).await.unwrap();
        let diff = MnListDiff {
            base_block_hash: base,
            ..MnListDiff::dummy(0x00, 0xEE)
        };
        storage.store_diff(work_block_height, &diff).await.unwrap();

        let engine = storage.load_engine().await;

        assert!(engine.masternode_lists.contains_key(&tip_height), "the QRInfo replays");
        assert!(
            engine.masternode_lists.contains_key(&work_block_height),
            "the diff on the QRInfo's h-c list replays after the QRInfo"
        );
    }

    /// evolite (SPV P0, #1200 on our storage): a replay checks every stored message
    /// against its block's coinbase, as the engine does live. The real QRInfo replays;
    /// the same one with a masternode added to its tip list «does not apply» — no list
    /// for it, no panic — and the client asks the network again.
    #[test_case(true; "as mined")]
    #[test_case(false; "with a forged masternode")]
    #[tokio::test]
    async fn replay_checks_stored_messages_against_their_coinbase(honest: bool) {
        use crate::sml_engine::qr_info_diffs;
        use crate::sml_engine::test_support::fixture_heights;
        use dashcore::network::constants::NetworkExt;
        use dashcore::sml::masternode_list_entry::MasternodeListEntry;

        let mut qr_info: QRInfo = bincode::decode_from_slice(
            include_bytes!("../../../dash/tests/data/test_DML_diffs/qrinfo_2518986.dat"),
            bincode::config::standard(),
        )
        .unwrap()
        .0;
        let mut heights = fixture_heights(include_bytes!(
            "../../../dash/tests/data/test_DML_diffs/block_container_2518986.dat"
        ));
        heights.remove(&Network::Mainnet.known_genesis_block_hash().unwrap());
        let tip_height = heights[&qr_info.mn_list_diff_tip.block_hash];
        // The headers a synced client holds: each diff's block and its merkle root.
        let mut headers = MockHeaderStorage::new(heights);
        for diff in qr_info_diffs(&qr_info) {
            headers.merkle_roots.insert(diff.block_hash, diff.coinbase_merkle_root().unwrap());
        }
        if !honest {
            qr_info.mn_list_diff_tip.new_masternodes.push(MasternodeListEntry::dummy(0x11));
        }

        let dir = TempDir::new().unwrap();
        let mut storage = PersistentMasternodeStorage::open(
            dir.path(),
            Arc::new(RwLock::new(headers)),
            Network::Mainnet,
        )
        .await
        .unwrap();
        storage.store_qr_info(tip_height, &qr_info).await.unwrap();

        let engine = storage.load_engine().await;

        assert_eq!(
            engine.masternode_lists.contains_key(&tip_height),
            honest,
            "a stored QRInfo replays only as mined"
        );
    }

    // evolite (TECH-spv-prune).

    /// The lists at or above `floor`, as (height, block hash).
    fn window_lists(
        engine: &MasternodeListEngine<MockHeaderStorage>,
        floor: CoreBlockHeight,
    ) -> Vec<(CoreBlockHeight, BlockHash)> {
        engine.masternode_lists.range(floor..).map(|(h, list)| (*h, list.block_hash)).collect()
    }

    /// An old chain of 12 diffs from genesis, an older work-block list in the
    /// retention window chained to it, a diff from genesis (the cut) and three
    /// after it. While the window's list chains back, nothing goes; once it is
    /// asked again from genesis, the 12 go, and a reopened storage replays the
    /// same lists in the window, and the next diff applies on them.
    #[tokio::test]
    async fn a_cut_lets_the_messages_before_it_go_but_keeps_the_window() {
        let window = crate::sml_engine::retention_window(Network::Regtest);
        let tip = 3 * window + 100;
        let floor = tip - window;
        let cut = floor + 10;
        let mut heights: Vec<(u8, u32)> = vec![(0x00, 0)];
        heights.extend((1..=12u8).map(|b| (b, u32::from(b))));
        heights.extend([
            (0x30, floor + 5),
            (0x40, cut),
            (0x41, cut + 1),
            (0x42, cut + 2),
            (0x43, tip),
            (0x44, tip + 1),
        ]);
        let dir = TempDir::new().unwrap();
        let mut storage = open_storage(&dir, &heights).await;
        for b in 1..=12u8 {
            storage.store_diff(u32::from(b), &MnListDiff::dummy(b - 1, b)).await.unwrap();
        }
        storage.store_diff(floor + 5, &MnListDiff::dummy(0x0C, 0x30)).await.unwrap();
        storage.store_diff(cut, &MnListDiff::dummy(0x00, 0x40)).await.unwrap();
        storage.store_diff(cut + 1, &MnListDiff::dummy(0x40, 0x41)).await.unwrap();
        storage.store_diff(cut + 2, &MnListDiff::dummy(0x41, 0x42)).await.unwrap();
        storage.store_diff(tip, &MnListDiff::dummy(0x42, 0x43)).await.unwrap();
        assert_eq!(storage.prune().await.unwrap(), 0, "the window's list chains back to genesis");

        // After the cut its list is asked again, from genesis.
        storage.store_diff(floor + 5, &MnListDiff::dummy(0x00, 0x30)).await.unwrap();
        let before = window_lists(&storage.load_engine().await, floor);
        assert_eq!(before.len(), 5);
        assert_eq!(storage.prune().await.unwrap(), 12);

        let mut reopened = open_storage(&dir, &heights).await;
        assert_eq!(
            reopened.diffs.keys().copied().collect::<Vec<_>>(),
            vec![floor + 5, cut, cut + 1, cut + 2, tip]
        );
        assert_eq!(window_lists(&reopened.load_engine().await, floor), before);
        reopened.store_diff(tip + 1, &MnListDiff::dummy(0x43, 0x44)).await.unwrap();
        let engine = reopened.load_engine().await;
        assert_eq!(engine.masternode_lists[&(tip + 1)].block_hash, hash(0x44), "the next applies");
    }

    /// Storing a QRInfo prunes once it is written: the old chain, which no
    /// list in the window needs, goes; the QRInfo stays.
    #[tokio::test]
    async fn storing_a_qr_info_prunes_what_it_makes_redundant() {
        let window = crate::sml_engine::retention_window(Network::Regtest);
        let tip = 3 * window + 100;
        let mut heights: Vec<(u8, u32)> = vec![(0x00, 0)];
        heights.extend((1..=16u8).map(|b| (b, u32::from(b))));
        heights.extend([(0x50, 0), (0x51, tip - 4), (0x52, tip - 3), (0x53, tip - 2), (0x54, tip)]);
        let dir = TempDir::new().unwrap();
        let mut storage = open_storage(&dir, &heights).await;
        for b in 1..=16u8 {
            storage.store_diff(u32::from(b), &MnListDiff::dummy(b - 1, b)).await.unwrap();
        }
        storage.store_qr_info(tip, &chained_qr_info(0x50, 0x54)).await.unwrap();
        let reopened = open_storage(&dir, &heights).await;
        assert!(reopened.diffs.is_empty());
        assert_eq!(reopened.qr_infos.keys().copied().collect::<Vec<_>>(), vec![tip]);
    }

    /// A message the pruning cannot judge stops it: a file gone behind the
    /// index, or a block not in the header chain. The replay skips the missing
    /// file and applies the rest.
    #[tokio::test]
    async fn what_the_pruning_cannot_judge_stops_it() {
        let mut heights: Vec<(u8, u32)> = vec![(0x00, 0)];
        heights.extend((1..=20u8).map(|b| (b, u32::from(b) * 1000)));
        let dir = TempDir::new().unwrap();
        let mut storage = open_storage(&dir, &heights).await;
        for b in 1..=20u8 {
            storage.store_diff(u32::from(b) * 1000, &MnListDiff::dummy(0x00, b)).await.unwrap();
        }
        tokio::fs::remove_file(&storage.diffs[&1000]).await.unwrap();
        assert_eq!(storage.prune().await.unwrap(), 0, "a file gone");
        assert_eq!(storage.load_engine().await.masternode_lists.len(), 19);

        let dir = TempDir::new().unwrap();
        let mut storage = open_storage(&dir, &heights[..20]).await;
        for b in 1..=20u8 {
            storage.store_diff(u32::from(b) * 1000, &MnListDiff::dummy(0x00, b)).await.unwrap();
        }
        assert_eq!(storage.prune().await.unwrap(), 0, "a block not in the header chain");
    }

    /// A QRInfo from genesis makes the engine forget the lists it did not
    /// build, so the next QRInfo's base is one of its lists; one on a list
    /// leaves the engine as it was.
    #[tokio::test]
    async fn a_qr_info_from_genesis_leaves_only_its_lists_to_build_on() {
        let mut heights: Vec<(u8, u32)> = vec![(0x00, 0)];
        heights.extend((1..=12u8).map(|b| (b, u32::from(b) * 100)));
        heights.push((0x20, 1300));
        let dir = TempDir::new().unwrap();
        let mut storage = open_storage(&dir, &heights).await;
        for b in 1..=12u8 {
            storage.store_diff(u32::from(b) * 100, &MnListDiff::dummy(b - 1, b)).await.unwrap();
        }
        let mut engine = storage.load_engine().await;
        let lists = engine.masternode_lists.len();

        assert!(!engine.cut_at_qr_info_from_genesis(&chained_qr_info(0x05, 0x20)).await);
        assert_eq!(engine.masternode_lists.len(), lists, "on a list: no cut");

        // From genesis, building only the newest list the engine holds; the
        // engine holds an older one too.
        assert!(engine.masternode_lists.values().any(|list| list.block_hash == hash(0x0B)));
        let from_genesis = QRInfo {
            mn_list_diff_at_h_minus_3c: MnListDiff::dummy_empty(0x00, 0x00),
            mn_list_diff_at_h_minus_2c: MnListDiff::dummy_empty(0x00, 0x00),
            mn_list_diff_at_h_minus_c: MnListDiff::dummy_empty(0x00, 0x00),
            mn_list_diff_h: MnListDiff::dummy_empty(0x00, 0x0C),
            mn_list_diff_tip: MnListDiff::dummy_empty(0x0C, 0x20),
            ..QRInfo::dummy(0x20)
        };
        assert!(engine.cut_at_qr_info_from_genesis(&from_genesis).await);
        let built = qr_info_built(&from_genesis);
        assert!(!engine.masternode_lists.is_empty());
        assert!(engine.masternode_lists.values().all(|list| built.contains(&list.block_hash)));
        assert!(engine
            .qr_info_base_list(CoreBlockHeight::MAX)
            .is_none_or(|list| built.contains(&list.block_hash)));
    }

    /// Review r1 M1: nine QRInfos chained from the first one from genesis, an
    /// older work-block diff on one of their lists, a work-block diff asked
    /// again from genesis after the cut (below the cut's height), then the cut:
    /// a QRInfo from genesis. All of it lies in the retention window. Storing
    /// the cut prunes everything from before it at once; a restart then
    /// replays only lists the cut and the diff after it built, the next
    /// QRInfo's base is one of the cut's lists, and no second cut is due.
    #[tokio::test]
    async fn a_restart_right_after_the_cut_keeps_nothing_from_before_it() {
        let window = crate::sml_engine::retention_window(Network::Regtest);
        assert!(window > 0x50, "the whole scene lies in the window");
        let mut heights: Vec<(u8, u32)> = (0x00..=0x44u8).map(|b| (b, u32::from(b))).collect();
        heights.push((0x50, 0x50));
        let dir = TempDir::new().unwrap();
        let mut storage = open_storage(&dir, &heights).await;
        // q1 on genesis, then each on the list the one before built.
        for i in 0..9u8 {
            let (start, tip) = (i * 4, i * 4 + 4);
            storage.store_qr_info(u32::from(tip), &chained_qr_info(start, tip)).await.unwrap();
        }
        assert_eq!(storage.qr_info_count(), 9);
        assert!(storage.cut_due().await);
        storage.store_diff(0x2F, &MnListDiff::dummy(0x10, 0x2F)).await.unwrap();
        // After the cut: a work-block list asked again from genesis, below the cut's height.
        storage.store_diff(0x30, &MnListDiff::dummy(0x00, 0x30)).await.unwrap();
        let cut = QRInfo {
            mn_list_diff_at_h_minus_3c: MnListDiff::dummy_empty(0x00, 0x00),
            mn_list_diff_at_h_minus_2c: MnListDiff::dummy_empty(0x00, 0x41),
            mn_list_diff_at_h_minus_c: MnListDiff::dummy_empty(0x41, 0x42),
            mn_list_diff_h: MnListDiff::dummy_empty(0x42, 0x43),
            mn_list_diff_tip: MnListDiff::dummy_empty(0x43, 0x44),
            ..QRInfo::dummy(0x44)
        };
        storage.store_qr_info(0x44, &cut).await.unwrap();

        let reopened = open_storage(&dir, &heights).await;
        assert_eq!(reopened.qr_infos.keys().copied().collect::<Vec<_>>(), vec![0x44]);
        assert_eq!(reopened.diffs.keys().copied().collect::<Vec<_>>(), vec![0x30]);
        assert!(reopened.qr_info_count() <= QRINFO_CUT_AT);
        assert!(!reopened.cut_due().await);
        let mut built = qr_info_built(&cut);
        built.insert(hash(0x30));
        let engine = reopened.load_engine().await;
        assert!(engine.masternode_lists.values().all(|list| built.contains(&list.block_hash)));
        let base = engine.qr_info_base_list(0x50 + 4 * 24);
        assert!(base.is_none_or(|list| built.contains(&list.block_hash)));
    }

    /// Review r1 M1: a cut stored and not pruned yet asks no second one.
    #[tokio::test]
    async fn a_stored_cut_asks_no_second_one() {
        let dir = TempDir::new().unwrap();
        let mut storage = open_storage(&dir, &[(0x00, 0)]).await;
        for n in 1..=9u8 {
            // On a list of 0xF0's, not genesis.
            let on_a_base = QRInfo {
                mn_list_diff_at_h_minus_3c: MnListDiff::dummy_empty(0xF0, 0xF0),
                ..QRInfo::dummy(n)
            };
            storage.store_qr_info(u32::from(n), &on_a_base).await.unwrap();
        }
        assert!(storage.cut_due().await, "nine on bases");
        storage.store_qr_info(10, &chained_qr_info(0x00, 0x60)).await.unwrap();
        assert!(!storage.cut_due().await, "the newest is from genesis");
    }
}
