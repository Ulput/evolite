//! Headers manager for parallel sync.
//!
//! Downloads and validates block headers from peers. Handles both initial sync
//! and post-sync header updates. Emits BlockHeadersStored events for other managers.
//!
//! Uses HeadersPipeline for parallel downloads across checkpoint-defined segments
//! during initial sync. The same pipeline is reused for post-sync updates.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use crate::chain::{ChainWork, CheckpointManager};
use crate::error::{SyncError, SyncResult};
use crate::network::RequestSender;
use crate::storage::{BlockHeaderStorage, BlockHeaderTip, MetadataStorage};
use crate::sync::block_headers::HeadersPipeline;
use crate::sync::{
    BlockHeadersProgress, ProgressPercentage, SyncEvent, SyncManager, SyncState,
    BEST_CHAINLOCK_METADATA_STORAGE_KEY,
};
use crate::types::HashedBlockHeader;
use crate::validation::{BlockHeaderValidator, Validator};
use dashcore::block::Header;
use dashcore::ephemerealdata::chain_lock::ChainLock;
use dashcore::network::message_blockdata::Inventory;
use dashcore::pow::DGW_PAST_BLOCKS;
use dashcore::{BlockHash, Network};
use tokio::sync::RwLock;

/// evolite (SPV item 1): the most headers a branch is held in memory for while it has
/// no more work than ours. Past that it is dropped and its peer disconnected.
pub(super) const MAX_FORK_HEADERS: usize = 2016;

/// evolite (SPV item 1): a valid branch off the stored chain below its tip, held until
/// it has more work than ours from the fork point. Nothing is trimmed meanwhile.
#[derive(Debug)]
pub(super) struct ForkBranch {
    /// The peer that sent it.
    peer: SocketAddr,
    /// Height of the stored block it branches off.
    fork_height: u32,
    /// Its headers, from the one on the fork point on, all validated.
    headers: Vec<HashedBlockHeader>,
}

/// Total proof of work of `headers`.
fn work(headers: &[HashedBlockHeader]) -> ChainWork {
    headers.iter().fold(ChainWork::zero(), |work, header| work.add_header(header.header()))
}

/// Headers manager for downloading and validating block headers.
///
/// This manager handles:
/// - Initial header sync using parallel pipeline (checkpoint-based segments)
/// - Post-sync header updates via inventory announcements
///
/// Generic over `H: BlockHeaderStorage` to allow different storage implementations.
pub struct BlockHeadersManager<H: BlockHeaderStorage, M: MetadataStorage> {
    /// Current progress of the manager.
    pub(super) progress: BlockHeadersProgress,
    /// Block header storage.
    pub(super) header_storage: Arc<RwLock<H>>,
    /// Metadata storage for persisting the best peer tip height.
    pub(super) metadata_storage: Arc<RwLock<M>>,
    /// Pipeline for parallel header downloads (used for both initial sync and post-sync).
    pub(super) pipeline: HeadersPipeline,
    /// Compiled-in checkpoints, which no fork may branch off below.
    checkpoint_manager: &'static CheckpointManager,
    /// Pending block announcements waiting for headers message (post-sync).
    pub(super) pending_announcements: HashMap<BlockHash, Instant>,
    /// Peers we've sent a GetHeaders to after sync, so Dash Core knows our tip
    /// and can send us header announcements instead of inv.
    pub(super) announced_peers: HashSet<SocketAddr>,
    /// Network whose difficulty rules new headers are checked against.
    network: Network,
    /// evolite (SPV item 1): a branch that doesn't have more work than ours yet.
    pub(super) fork: Option<ForkBranch>,
}

impl<H: BlockHeaderStorage, M: MetadataStorage> std::fmt::Debug for BlockHeadersManager<H, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockHeadersManager")
            .field("progress", &self.progress)
            .field("pipeline", &self.pipeline)
            .finish_non_exhaustive()
    }
}

impl<H: BlockHeaderStorage, M: MetadataStorage> BlockHeadersManager<H, M> {
    /// Create a new headers manager with the given storage and checkpoint manager.
    pub async fn new(
        header_storage: Arc<RwLock<H>>,
        metadata_storage: Arc<RwLock<M>>,
        checkpoint_manager: &'static CheckpointManager,
        network: Network,
    ) -> SyncResult<Self> {
        let tip = header_storage
            .read()
            .await
            .get_tip()
            .await
            .ok_or_else(|| SyncError::MissingDependency("No tip in storage".to_string()))?;

        // Restore persisted target height, fall back to tip height
        let target_height =
            metadata_storage.read().await.load_last_target_height().await.unwrap_or(tip.height());

        let mut initial_progress = BlockHeadersProgress::default();
        initial_progress.set_state(SyncState::WaitingForConnections);
        initial_progress.update_tip_height(tip.height());
        initial_progress.update_target_height(target_height);

        tracing::info!("BlockHeadersManager initialized at height {}", tip.height());

        Ok(Self {
            progress: initial_progress,
            header_storage,
            metadata_storage,
            pipeline: HeadersPipeline::new(checkpoint_manager),
            checkpoint_manager,
            pending_announcements: HashMap::new(),
            announced_peers: HashSet::new(),
            network,
            fork: None,
        })
    }

    pub(super) async fn tip(&self) -> SyncResult<BlockHeaderTip> {
        self.header_storage
            .read()
            .await
            .get_tip()
            .await
            .ok_or_else(|| SyncError::MissingDependency("storage not initialized".to_string()))
    }

    /// The stored headers up to `height` the next one's difficulty is computed from.
    async fn ancestors(&self, height: u32) -> SyncResult<Vec<Header>> {
        let storage = self.header_storage.read().await;

        // A storage started at a checkpoint holds less history than a full window.
        let first = height
            .saturating_sub(DGW_PAST_BLOCKS as u32 - 1)
            .max(storage.get_start_height().await.unwrap_or(0));

        let headers = storage.load_headers(first..height + 1).await?;
        Ok(headers.iter().map(|header| *header.header()).collect())
    }

    /// Height and ancestors of the block `headers` extend, a stored block or a
    /// segment's tip. `None` if it's neither, as then nothing follows from them.
    async fn attach_point(
        &self,
        headers: &[HashedBlockHeader],
    ) -> SyncResult<Option<(u32, Vec<Header>)>> {
        let Some(first) = headers.first() else {
            return Ok(None);
        };
        let parent = first.header().prev_blockhash;
        let height = self.header_storage.read().await.get_header_height_by_hash(&parent).await?;
        match height {
            Some(height) => Ok(Some((height, self.ancestors(height).await?))),
            None => Ok(self.pipeline.segment_ancestors(&parent)),
        }
    }

    /// Store an already validated headers batch.
    async fn store_headers(&mut self, headers: &[HashedBlockHeader]) -> SyncResult<BlockHeaderTip> {
        debug_assert!(!headers.is_empty());

        // Store headers
        self.header_storage.write().await.store_headers(headers).await?;

        let tip = self.tip().await?;

        // Update state
        self.progress.update_tip_height(tip.height());
        self.progress.add_processed(headers.len() as u32);

        Ok(tip)
    }

    /// Handle incoming headers message (used for both initial sync and post-sync).
    pub(super) async fn handle_headers_pipeline(
        &mut self,
        peer: SocketAddr,
        headers: &[HashedBlockHeader],
        requests: &RequestSender,
    ) -> SyncResult<Vec<SyncEvent>> {
        if !self.pipeline.is_initialized() {
            // Pipeline not initialized (shouldn't happen in normal flow)
            tracing::warn!("Received headers but pipeline not initialized");
            return Ok(vec![]);
        }

        let was_syncing = self.state() == SyncState::Syncing;
        let tip_was_complete = self.pipeline.is_tip_complete();

        // evolite (SPV P0): validated against the stored block or the segment tip they extend
        // before routing (#1304), so a fork is never detected on headers nobody checked.
        if let Some((height, ancestors)) = self.attach_point(headers).await? {
            if let Err(e) = BlockHeaderValidator::new(self.network, ancestors, height).validate(headers)
            {
                // The peer that sent them is banned, as #1304 does — here an evolite
                // adaptation through `RequestSender` (#1304 bans through the #902 network).
                // Only for headers that fail validation: never on a timeout or a disconnect.
                let _ = requests.ban_peer(peer, &format!("invalid block headers: {e}"));
                return Err(e.into());
            }
        }

        // evolite (SPV item 1): the next headers of a branch we hold.
        if let Some(events) = self.extend_fork(peer, headers, requests).await? {
            return Ok(events);
        }

        // Route headers to the pipeline, validates checkpoint match.
        let matched = self.pipeline.receive_headers(headers)?;

        if matched.is_none() && !headers.is_empty() {
            if let Some(fork_height) = self.fork_height(headers).await? {
                // evolite (SPV item 1): a fork is followed only once it has more work than
                // our chain from the fork point; until then it is held, nothing trimmed.
                let new = self.unstored(headers).await?;
                if new.len() > MAX_FORK_HEADERS {
                    self.drop_fork_peer(peer, requests);
                    return Ok(vec![]);
                }
                self.fork = Some(ForkBranch {
                    peer,
                    fork_height,
                    headers: new,
                });
                return self.weigh_fork(requests).await;
            }
            tracing::debug!(
                "Headers not matched by pipeline (prev_hash: {}), may be post-sync update",
                headers[0].header().prev_blockhash
            );
        }

        // Process ready-to-store segments
        let mut events = Vec::new();
        let ready_batches = self.pipeline.take_ready_to_store();

        // Draining can expose new segments at the end of the active window, so
        // refill only after next_to_store has advanced. Skip unsolicited headers.
        if was_syncing || !tip_was_complete {
            let sent = self.pipeline.send_pending(requests)?;
            if sent > 0 {
                tracing::debug!("Pipeline sent {} more requests", sent);
            }
        }

        events.extend(self.store_ready_batches(ready_batches).await?);

        // After storing unsolicited post-sync headers, mark the tip complete so the next header goes through
        // the clean reset path. Don't mark complete during active catch-up.
        if !was_syncing && tip_was_complete && !events.is_empty() {
            self.pipeline.mark_tip_complete();
        }

        if was_syncing {
            events.extend(self.finalize_sync_if_complete(requests).await?);
        }

        if matched.is_some() {
            self.progress.bump_last_activity();
        }
        Ok(events)
    }

    /// evolite (SPV item 1): `headers` past the ones we already store.
    async fn unstored(&self, headers: &[HashedBlockHeader]) -> SyncResult<Vec<HashedBlockHeader>> {
        let storage = self.header_storage.read().await;
        let mut stored = 0;
        for header in headers {
            if storage.get_header_height_by_hash(header.hash()).await?.is_none() {
                break;
            }
            stored += 1;
        }
        Ok(headers[stored..].to_vec())
    }

    /// evolite (SPV item 1): takes `headers` that continue the branch we hold. `None`
    /// if they don't continue it, and the usual routing goes on.
    ///
    /// They are validated against the branch below them (continuity, proof of work,
    /// Dark Gravity Wave over stored ancestors and the branch). Invalid ones drop the
    /// branch and ban the peer; a branch past [`MAX_FORK_HEADERS`] is dropped and its
    /// peer disconnected.
    async fn extend_fork(
        &mut self,
        peer: SocketAddr,
        headers: &[HashedBlockHeader],
        requests: &RequestSender,
    ) -> SyncResult<Option<Vec<SyncEvent>>> {
        let (Some(branch), Some(first)) = (&self.fork, headers.first()) else {
            return Ok(None);
        };
        let Some(branch_tip) = branch.headers.last() else {
            return Ok(None);
        };
        if first.header().prev_blockhash != *branch_tip.hash() {
            return Ok(None);
        }

        let fork_height = branch.fork_height;
        let held = branch.headers.len();
        let mut ancestors = self.ancestors(fork_height).await?;
        if let Some(branch) = &self.fork {
            ancestors.extend(branch.headers.iter().map(|header| *header.header()));
        }
        let ancestors = ancestors.split_off(ancestors.len().saturating_sub(DGW_PAST_BLOCKS));
        let height = fork_height + held as u32;
        if let Err(e) = BlockHeaderValidator::new(self.network, ancestors, height).validate(headers)
        {
            self.fork = None;
            let _ = requests.ban_peer(peer, &format!("invalid fork headers: {e}"));
            return Err(e.into());
        }

        let Some(branch) = self.fork.as_mut() else {
            return Ok(None);
        };
        branch.headers.extend_from_slice(headers);
        if branch.headers.len() > MAX_FORK_HEADERS {
            self.drop_fork_peer(peer, requests);
            return Ok(Some(vec![]));
        }
        self.weigh_fork(requests).await.map(Some)
    }

    /// evolite (SPV item 1): drops the branch we hold and disconnects `peer`, whose
    /// branch got longer than we keep without having more work.
    fn drop_fork_peer(&mut self, peer: SocketAddr, requests: &RequestSender) {
        tracing::warn!(
            "Dropping a fork of more than {} headers without more work than ours, from {}",
            MAX_FORK_HEADERS,
            peer
        );
        self.fork = None;
        let _ = requests.disconnect_peer(peer, "fork too long without more work");
    }

    /// evolite (SPV item 1): `ForkDetected` once the branch we hold has strictly more
    /// work than our chain from the fork point to the tip. On equal work we stay on
    /// the chain we saw first, as Dash Core does. Below a ChainLock or checkpoint that
    /// arrived since, the branch is dropped. Otherwise the peer is asked for more of it.
    async fn weigh_fork(&mut self, requests: &RequestSender) -> SyncResult<Vec<SyncEvent>> {
        let Some(fork_height) = self.fork.as_ref().map(|branch| branch.fork_height) else {
            return Ok(vec![]);
        };
        let tip = self.tip().await?.height();
        if fork_height >= tip || self.is_final(fork_height, tip).await? {
            self.fork = None;
            return Ok(vec![]);
        }

        let ours = self.header_storage.read().await.load_headers(fork_height + 1..tip + 1).await?;
        let our_work = work(&ours);
        let Some(branch) = &self.fork else {
            return Ok(vec![]);
        };
        if work(&branch.headers) > our_work {
            tracing::info!(
                "Fork at height {} has more work than ours ({} headers against {}), switching",
                fork_height,
                branch.headers.len(),
                ours.len()
            );
            self.fork = None;
            return Ok(vec![SyncEvent::ForkDetected {
                fork_height,
            }]);
        }

        tracing::debug!(
            "Holding a fork at height {} ({} headers) until it has more work than ours",
            fork_height,
            branch.headers.len()
        );
        if let Some(branch_tip) = branch.headers.last() {
            let _ = requests.request_block_headers_from_peer(*branch_tip.hash(), branch.peer);
        }
        Ok(vec![])
    }

    /// Whether blocks above `fork_height` are final: below the newest checkpoint the
    /// stored chain (tip `tip`) reaches, or below the best ChainLock.
    async fn is_final(&self, fork_height: u32, tip: u32) -> SyncResult<bool> {
        if let Some(checkpoint) = self
            .checkpoint_manager
            .last_checkpoint_before_height(tip)
            .filter(|checkpoint| fork_height < checkpoint.height())
        {
            tracing::warn!(
                "Ignoring a fork at height {}, below the checkpoint at {}",
                fork_height,
                checkpoint.height()
            );
            return Ok(true);
        }

        let chain_lock = self
            .metadata_storage
            .read()
            .await
            .load_metadata::<ChainLock>(BEST_CHAINLOCK_METADATA_STORAGE_KEY)
            .await?;
        if let Some(chain_lock) = chain_lock.filter(|cl| fork_height < cl.block_height) {
            tracing::warn!(
                "Ignoring a fork at height {}, below the ChainLock at {}",
                fork_height,
                chain_lock.block_height
            );
            return Ok(true);
        }
        Ok(false)
    }

    /// Height where `headers` branch off the stored chain below its tip, if they
    /// do. A branch off below the best ChainLock, or below the newest checkpoint
    /// the stored chain reaches, is not followed: the blocks they fix are final.
    async fn fork_height(&self, headers: &[HashedBlockHeader]) -> SyncResult<Option<u32>> {
        let storage = self.header_storage.read().await;

        // Skip the headers we already store: a peer may resend part of our chain.
        for header in headers {
            if storage.get_header_height_by_hash(header.hash()).await?.is_some() {
                continue;
            }

            let parent = storage.get_header_height_by_hash(&header.header().prev_blockhash).await?;
            let tip = storage.get_tip_height().await;
            let (Some(fork_height), Some(tip)) =
                (parent.filter(|&parent| tip.is_some_and(|tip| parent < tip)), tip)
            else {
                return Ok(None);
            };
            drop(storage);

            // evolite (SPV item 1): the same rule decides again before a held branch is
            // followed, as a ChainLock may come meanwhile (`is_final`).
            if self.is_final(fork_height, tip).await? {
                return Ok(None);
            }
            return Ok(Some(fork_height));
        }

        Ok(None)
    }

    /// Write segments that finished downloading into storage.
    ///
    /// Split out of [`Self::handle_headers_pipeline`] so `tick` can promote
    /// headers too. Promotion used to be reachable only from there — i.e. only
    /// when a `Headers` message arrived — which leaves a sync no way to finish
    /// itself: once the last segment completes, no further `Headers` will ever
    /// come, and the already-downloaded, already-validated tail simply stays in
    /// memory. A testnet restore was found frozen exactly there, with 1,046,289
    /// headers buffered and the stored tip stuck at 1,474,000 while peers
    /// stayed connected and chain locks kept arriving.
    ///
    /// Takes the batches rather than draining them itself, so callers keep
    /// `take_ready_to_store` → `send_pending` → store ordering: draining can
    /// expose a new segment at the end of the active window, and the refill
    /// should pick it up in the same pass.
    pub(super) async fn store_ready_batches(
        &mut self,
        ready_batches: Vec<(u32, Vec<HashedBlockHeader>)>,
    ) -> SyncResult<Vec<SyncEvent>> {
        let mut events = Vec::new();

        for (_start_height, batch_headers) in ready_batches {
            if !batch_headers.is_empty() {
                // Validate chain continuity with current tip
                let tip = self.tip().await?;
                if batch_headers[0].header().prev_blockhash != *tip.hash() {
                    return Err(SyncError::Validation(format!(
                        "Segment chain break: expected prev {}, got {}",
                        tip.hash(),
                        batch_headers[0].header().prev_blockhash
                    )));
                }

                // Clear any pending announcements for headers we're storing
                for header in &batch_headers {
                    self.pending_announcements.remove(header.hash());
                }

                let new_tip = self.store_headers(&batch_headers).await?;
                // Update target if we've exceeded it (post-sync case)
                if new_tip.height() > self.progress.target_height() {
                    self.progress.update_target_height(new_tip.height());
                }
                events.push(SyncEvent::BlockHeadersStored {
                    tip_height: new_tip.height(),
                });
            }
        }

        Ok(events)
    }

    /// Close out an initial sync whose pipeline has nothing left to download.
    ///
    /// Extracted alongside [`Self::store_ready_batches`] and for the same
    /// reason: a promotion driven by `tick` has to be able to reach the
    /// completion it just made possible, or the manager stores the last
    /// segment and then sits in `Syncing` forever.
    pub(super) async fn finalize_sync_if_complete(
        &mut self,
        requests: &RequestSender,
    ) -> SyncResult<Vec<SyncEvent>> {
        if !self.pipeline.is_complete() {
            return Ok(Vec::new());
        }

        // If blocks were announced during sync, request them before finalizing the sync
        if !self.pending_announcements.is_empty() {
            tracing::info!(
                "Pipeline complete but {} blocks announced during sync, requesting headers",
                self.pending_announcements.len()
            );
            self.pipeline.reset_tip_segment();
            self.pipeline.send_pending(requests)?;
            return Ok(Vec::new());
        }

        // Synced to the tip and no pending announcements, finalize and emit event
        let tip = self.tip().await?;
        self.progress.update_target_height(tip.height());
        self.progress.set_state(SyncState::Synced);
        tracing::info!("Headers sync complete at height {}", tip.height());
        Ok(vec![SyncEvent::BlockHeaderSyncComplete {
            tip_height: tip.height(),
        }])
    }

    /// Handle inventory announcements for new blocks.
    ///
    /// During initial sync, Dash Core sends inv (not header announcements) because
    /// it doesn't think we have the parent block. We track these announcements so
    /// we can request headers after sync completes.
    ///
    /// When synced, we expect unsolicited header announcements. The tick handler
    /// uses a timeout to send fallback GetHeaders if headers don't arrive.
    pub(super) async fn handle_inventory(
        &mut self,
        inv: &[Inventory],
        _requests: &RequestSender,
    ) -> SyncResult<()> {
        for inv_item in inv {
            if let Inventory::Block(block_hash) = inv_item {
                // Check if already pending
                if self.pending_announcements.contains_key(block_hash) {
                    continue;
                }

                // Check if we already have this block
                if let Ok(Some(_)) =
                    self.header_storage.read().await.get_header_height_by_hash(block_hash).await
                {
                    continue;
                }

                tracing::info!("New block announced via inv: {}", block_hash);
                self.pending_announcements.insert(*block_hash, Instant::now());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{MessageType, NetworkEvent, NetworkRequest, RequestSender};
    use crate::storage::{
        DiskStorageManager, PersistentBlockHeaderStorage, PersistentMetadataStorage, StorageManager,
    };
    use crate::sync::block_headers::pipeline::ACTIVE_SEGMENT_WINDOW;
    use crate::sync::block_headers::segment_state::SegmentState;
    use crate::sync::{ManagerIdentifier, SyncManager, SyncManagerProgress};
    use dashcore::network::message::NetworkMessage;
    use test_case::test_case;
    use tokio::sync::mpsc::unbounded_channel;

    type TestBlockHeadersManager =
        BlockHeadersManager<PersistentBlockHeaderStorage, PersistentMetadataStorage>;

    fn create_test_checkpoint_manager() -> &'static CheckpointManager {
        CheckpointManager::for_network(dashcore::Network::Testnet)
    }

    async fn create_test_manager() -> TestBlockHeadersManager {
        let mut storage = DiskStorageManager::with_temp_dir().await.unwrap();
        // Store a genesis header so the manager can initialize
        let genesis = Header::dummy_batch(0..1);
        storage
            .store_headers(
                &genesis.iter().map(crate::types::HashedBlockHeader::from).collect::<Vec<_>>(),
            )
            .await
            .unwrap();
        let checkpoint_manager = create_test_checkpoint_manager();
        BlockHeadersManager::new(
            storage.block_headers(),
            storage.metadata(),
            checkpoint_manager,
            Network::Regtest,
        )
        .await
        .expect("Failed to create BlockHeadersManager")
    }

    #[test_case(None, 5, Some(5); "no ChainLock")]
    #[test_case(Some(6), 5, None; "below the ChainLock")]
    #[test_case(Some(5), 5, Some(5); "at the ChainLock")]
    #[test_case(Some(3), 5, Some(5); "above the ChainLock")]
    #[tokio::test]
    async fn a_branch_off_below_the_best_chain_lock_is_no_fork(
        chain_locked: Option<u32>,
        branch_off: u32,
        expected: Option<u32>,
    ) {
        let mut storage = DiskStorageManager::with_temp_dir().await.unwrap();
        let stored = Header::dummy_batch(0..10);
        storage
            .store_headers(&stored.iter().map(HashedBlockHeader::from).collect::<Vec<_>>())
            .await
            .unwrap();
        if let Some(height) = chain_locked {
            let bytes = serde_json::to_vec(&ChainLock::dummy(height)).unwrap();
            storage.store_metadata(BEST_CHAINLOCK_METADATA_STORAGE_KEY, &bytes).await.unwrap();
        }
        let manager = BlockHeadersManager::new(
            storage.block_headers(),
            storage.metadata(),
            create_test_checkpoint_manager(),
            Network::Regtest,
        )
        .await
        .unwrap();

        let branch = Header {
            prev_blockhash: stored[branch_off as usize].block_hash(),
            ..Header::dummy(1_000)
        };

        assert_eq!(
            manager.fork_height(&[HashedBlockHeader::from(&branch)]).await.unwrap(),
            expected
        );
    }

    /// Blocks below the newest checkpoint the stored chain reaches are final, as
    /// below the best ChainLock. A checkpoint above the tip fixes nothing yet,
    /// or no fork could ever be followed before the chain reached it.
    #[test_case(49_999, None; "below the checkpoint")]
    #[test_case(50_000, Some(50_000); "at the checkpoint")]
    #[test_case(50_001, Some(50_001); "above the checkpoint")]
    #[tokio::test]
    async fn a_branch_off_below_the_newest_reached_checkpoint_is_no_fork(
        branch_off: u32,
        expected: Option<u32>,
    ) {
        let mut storage = DiskStorageManager::with_temp_dir().await.unwrap();
        let start = 49_990;
        let stored: Vec<_> =
            Header::dummy_batch(start..50_010).iter().map(HashedBlockHeader::from).collect();
        storage.store_headers_at_height(&stored, start).await.unwrap();
        let manager = BlockHeadersManager::new(
            storage.block_headers(),
            storage.metadata(),
            create_test_checkpoint_manager(),
            Network::Regtest,
        )
        .await
        .unwrap();

        let branch = Header {
            prev_blockhash: *stored[(branch_off - start) as usize].hash(),
            ..Header::dummy(1_000_000)
        };

        assert_eq!(
            manager.fork_height(&[HashedBlockHeader::from(&branch)]).await.unwrap(),
            expected
        );
    }

    /// Headers are validated as they arrive, on the tip or branching off below it:
    /// invalid ones are neither stored nor followed, and their peer is banned.
    #[test_case(4, true; "valid on the tip")]
    #[test_case(4, false; "invalid on the tip")]
    #[test_case(2, true; "valid fork")]
    #[test_case(2, false; "invalid fork")]
    #[tokio::test]
    async fn test_received_headers_are_validated(parent: usize, valid: bool) {
        let mut storage = DiskStorageManager::with_temp_dir().await.unwrap();
        let chain: Vec<HashedBlockHeader> = Header::dummy_chain(5, BlockHash::dummy(0))
            .iter()
            .map(HashedBlockHeader::from)
            .collect();
        storage.store_headers(&chain).await.unwrap();
        let mut manager = BlockHeadersManager::new(
            storage.block_headers(),
            storage.metadata(),
            create_test_checkpoint_manager(),
            Network::Regtest,
        )
        .await
        .unwrap();
        manager.pipeline.init(4, *chain[4].hash(), 4);
        // evolite (SPV P0): synced as our pipeline has it, so a header on the tip is taken unasked.
        manager.pipeline.mark_tip_complete();
        manager.progress.set_state(SyncState::Synced);

        let parent_hash = *chain[parent].hash();
        let header = if valid {
            Header::dummy_chain(1, parent_hash)[0]
        } else {
            Header {
                prev_blockhash: parent_hash,
                ..Header::dummy(5)
            }
        };
        // evolite (SPV P0): our network is not #902's; the requests, the ban among them
        // (evolite-fork-ban.patch), go through a `RequestSender`.
        let (requests, mut rx) = create_test_request_sender();

        let result = manager.handle_headers_pipeline(peer(), &[header.into()], &requests).await;

        assert_eq!(result.is_ok(), valid, "{result:?}");
        let mut banned = Vec::new();
        while let Ok(request) = rx.try_recv() {
            if let NetworkRequest::BanPeer(address, _) = request {
                banned.push(address);
            }
        }
        assert_eq!(banned, if valid { vec![] } else { vec![peer()] });
        let forked = result.is_ok_and(|events| {
            events.iter().any(|event| matches!(event, SyncEvent::ForkDetected { .. }))
        });
        // evolite (SPV item 1): one header off 2 has less work than ours above it, so
        // it is held, never followed (`a_valid_fork_with_more_work_is_followed`).
        assert!(!forked, "one header has no more work than ours");
        let tip = if valid && parent == 4 {
            5
        } else {
            4
        };
        assert_eq!(manager.tip().await.unwrap().height(), tip);
    }

    /// evolite (SPV P0, AC-T5): on the 30 real mainnet headers 1999976–2000005, a fork
    /// off block 2000000 (the first with Dark Gravity Wave's 24 headers of history
    /// stored below it) whose header has its own proof of work but a difficulty made
    /// easier is refused before routing: no ForkDetected, the tip stays, the peer is
    /// banned. Its header as mined (the real 2000001, already stored) is no fork.
    #[tokio::test]
    async fn a_mainnet_fork_easier_than_dgw_is_refused_and_its_peer_banned() {
        let chain: Vec<HashedBlockHeader> =
            Header::mainnet_fixture().iter().map(HashedBlockHeader::from).collect();
        let first = 1_999_976;
        let mut storage = DiskStorageManager::with_temp_dir().await.unwrap();
        storage.store_headers_at_height(&chain, first).await.unwrap();
        let mut manager = BlockHeadersManager::new(
            storage.block_headers(),
            storage.metadata(),
            create_test_checkpoint_manager(),
            Network::Mainnet,
        )
        .await
        .unwrap();
        let tip = first + chain.len() as u32 - 1;
        manager.pipeline.init(tip, *chain[chain.len() - 1].hash(), tip);
        manager.pipeline.mark_tip_complete();
        manager.progress.set_state(SyncState::Synced);

        // A sibling of the real 2000001: regtest-easy bits and a nonce found for them,
        // so its proof of work holds and only the difficulty check can refuse it.
        let mut forged = *chain[25].header();
        forged.bits = dashcore::CompactTarget::from_consensus(0x207f_ffff);
        while !forged.target().is_met_by(forged.block_hash()) {
            forged.nonce = forged.nonce.wrapping_add(1);
        }
        assert_eq!(forged.prev_blockhash, *chain[24].hash(), "forked off 2000000");
        let (requests, mut rx) = create_test_request_sender();

        let result = manager.handle_headers_pipeline(peer(), &[forged.into()], &requests).await;

        assert!(
            format!("{result:?}").contains("ifficulty"),
            "refused for its difficulty (DGW): {result:?}"
        );
        let mut banned = Vec::new();
        while let Ok(request) = rx.try_recv() {
            if let NetworkRequest::BanPeer(address, _) = request {
                banned.push(address);
            }
        }
        assert_eq!(banned, vec![peer()], "its peer banned");
        assert_eq!(manager.tip().await.unwrap().height(), tip, "the tip stays");
    }

    /// evolite (SPV item 1): heights where regtest checks a header's bits against its
    /// limit (past Dark Gravity Wave's height, 34140), with the newest testnet
    /// checkpoint below them at 50000. Fork points in the tests are above it.
    const FORK_BASE: u32 = 50_000;

    /// Regtest's proof-of-work limit, the bits required at [`FORK_BASE`].
    const EASY: u32 = 0x207f_ffff;

    /// A header on `prev` with `bits` whose proof of work holds; `salt` sets it apart
    /// from a sibling.
    fn mined(prev: BlockHash, bits: u32, salt: u32) -> Header {
        let mut header = Header {
            prev_blockhash: prev,
            bits: dashcore::CompactTarget::from_consensus(bits),
            time: salt,
            nonce: 0,
            ..Header::dummy(0)
        };
        while !header.target().is_met_by(header.block_hash()) {
            header.nonce += 1;
        }
        header
    }

    /// `count` mined headers on `prev`, each with `bits`.
    fn mined_chain(prev: BlockHash, count: usize, bits: u32, salt: u32) -> Vec<HashedBlockHeader> {
        let mut chain: Vec<HashedBlockHeader> = Vec::with_capacity(count);
        for i in 0..count {
            let parent = chain.last().map_or(prev, |header| *header.hash());
            chain.push(mined(parent, bits, salt + i as u32).into());
        }
        chain
    }

    /// A synced regtest manager whose stored chain is `ours`, from height `start`.
    async fn fork_manager(
        start: u32,
        ours: &[HashedBlockHeader],
    ) -> (TestBlockHeadersManager, DiskStorageManager) {
        let mut storage = DiskStorageManager::with_temp_dir().await.unwrap();
        storage.store_headers_at_height(ours, start).await.unwrap();
        let mut manager = BlockHeadersManager::new(
            storage.block_headers(),
            storage.metadata(),
            create_test_checkpoint_manager(),
            Network::Regtest,
        )
        .await
        .unwrap();
        let tip = start + ours.len() as u32 - 1;
        manager.pipeline.init(tip, *ours[ours.len() - 1].hash(), tip);
        manager.pipeline.mark_tip_complete();
        manager.progress.set_state(SyncState::Synced);
        (manager, storage)
    }

    /// What the manager asked the network for.
    #[derive(Debug, Default)]
    struct Sent {
        banned: Vec<SocketAddr>,
        disconnected: Vec<SocketAddr>,
        /// Peers asked for headers, and the hash they were asked to go on from.
        asked: Vec<(SocketAddr, BlockHash)>,
    }

    fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkRequest>) -> Sent {
        let mut sent = Sent::default();
        while let Ok(request) = rx.try_recv() {
            match request {
                NetworkRequest::BanPeer(address, _) => sent.banned.push(address),
                NetworkRequest::DisconnectPeer(address, _) => sent.disconnected.push(address),
                NetworkRequest::SendMessageToPeer(NetworkMessage::GetHeaders(message), address) => {
                    sent.asked.push((address, message.locator_hashes[0]))
                }
                _ => {}
            }
        }
        sent
    }

    fn fork_detected(result: &SyncResult<Vec<SyncEvent>>) -> Option<u32> {
        result.as_ref().ok()?.iter().find_map(|event| match event {
            SyncEvent::ForkDetected {
                fork_height,
            } => Some(*fork_height),
            _ => None,
        })
    }

    async fn store_chain_lock(manager: &TestBlockHeadersManager, height: u32) {
        let bytes = serde_json::to_vec(&ChainLock::dummy(height)).unwrap();
        manager
            .metadata_storage
            .write()
            .await
            .store_metadata(BEST_CHAINLOCK_METADATA_STORAGE_KEY, &bytes)
            .await
            .unwrap();
    }

    /// How a header in the tests below is bad.
    #[derive(Debug, Clone, Copy)]
    enum Bad {
        /// Its hash doesn't meet its own target.
        ProofOfWork,
        /// Its proof of work holds but its bits are easier than required.
        Difficulty,
    }

    fn bad_header(prev: BlockHash, bad: Bad) -> HashedBlockHeader {
        match bad {
            Bad::ProofOfWork => {
                let mut header = mined(prev, EASY, 77);
                while header.target().is_met_by(header.block_hash()) {
                    header.nonce += 1;
                }
                header.into()
            }
            Bad::Difficulty => mined(prev, 0x2100_ffff, 77).into(),
        }
    }

    /// evolite (SPV item 1, AC-T1.1, AC-T1.2): a fork header with a bad proof of work,
    /// or with bits easier than required, above the ChainLock, is refused: no
    /// ForkDetected, nothing trimmed or held, its peer banned. Both as the first
    /// headers of a fork and as the next ones of a branch already held.
    #[test_case(Bad::ProofOfWork, false; "bad proof of work, first headers")]
    #[test_case(Bad::ProofOfWork, true; "bad proof of work, continuing a held branch")]
    #[test_case(Bad::Difficulty, false; "easier difficulty, first headers")]
    #[test_case(Bad::Difficulty, true; "easier difficulty, continuing a held branch")]
    #[tokio::test]
    async fn an_invalid_fork_is_refused_and_its_peer_banned(bad: Bad, held: bool) {
        let ours = mined_chain(BlockHash::dummy(0), 5, EASY, 0);
        let (mut manager, _storage) = fork_manager(FORK_BASE, &ours).await;
        store_chain_lock(&manager, FORK_BASE + 1).await;
        let (requests, mut rx) = create_test_request_sender();

        let mut parent = *ours[2].hash();
        if held {
            let branch = mined_chain(parent, 1, EASY, 100);
            let result = manager.handle_headers_pipeline(peer(), &branch, &requests).await;
            assert_eq!(fork_detected(&result), None, "{result:?}");
            assert!(manager.fork.is_some(), "the valid header is held");
            parent = *branch[0].hash();
            sent(&mut rx);
        }
        let result =
            manager.handle_headers_pipeline(peer(), &[bad_header(parent, bad)], &requests).await;

        let expected = match bad {
            Bad::ProofOfWork => "proof of work",
            Bad::Difficulty => "ifficulty",
        };
        assert!(format!("{result:?}").contains(expected), "{result:?}");
        assert_eq!(fork_detected(&result), None);
        assert!(manager.fork.is_none(), "the branch is dropped");
        assert_eq!(sent(&mut rx).banned, vec![peer()]);
        assert_eq!(manager.tip().await.unwrap().height(), FORK_BASE + 4, "nothing trimmed");
    }

    /// evolite (SPV item 1, AC-T1.3): a valid fork with less or as much work as our
    /// chain from the fork point is not followed: no ForkDetected, nothing trimmed. It
    /// is held, and its peer asked for what comes after it. Nobody is banned.
    #[test_case(1; "less work")]
    #[test_case(2; "equal work, the chain seen first stays")]
    #[tokio::test]
    async fn a_valid_fork_without_more_work_is_held_not_followed(length: usize) {
        let ours = mined_chain(BlockHash::dummy(0), 5, EASY, 0);
        let (mut manager, _storage) = fork_manager(FORK_BASE, &ours).await;
        let (requests, mut rx) = create_test_request_sender();
        let branch = mined_chain(*ours[2].hash(), length, EASY, 100);

        let result = manager.handle_headers_pipeline(peer(), &branch, &requests).await;

        assert_eq!(fork_detected(&result), None, "{result:?}");
        assert_eq!(manager.tip().await.unwrap().height(), FORK_BASE + 4);
        assert_eq!(manager.fork.as_ref().map(|fork| fork.headers.len()), Some(length));
        let sent = sent(&mut rx);
        assert_eq!(sent.asked, vec![(peer(), *branch[length - 1].hash())]);
        assert!(sent.banned.is_empty() && sent.disconnected.is_empty(), "{sent:?}");
    }

    /// evolite (SPV item 1, AC-T1.4): a valid fork is followed (ForkDetected at its
    /// fork point) once it has more work than ours from there, whether it comes in one
    /// message or grows over several. Before that, nothing.
    #[test_case(&[3]; "in one message")]
    #[test_case(&[2, 1]; "equal, then one more")]
    #[test_case(&[1, 1, 1]; "a header at a time")]
    #[tokio::test]
    async fn a_valid_fork_with_more_work_is_followed(messages: &[usize]) {
        let ours = mined_chain(BlockHash::dummy(0), 5, EASY, 0);
        let (mut manager, _storage) = fork_manager(FORK_BASE, &ours).await;
        let (requests, mut rx) = create_test_request_sender();
        let branch = mined_chain(*ours[2].hash(), messages.iter().sum(), EASY, 100);

        let mut sent_so_far = 0;
        for (i, &count) in messages.iter().enumerate() {
            let part = &branch[sent_so_far..sent_so_far + count];
            sent_so_far += count;
            let result = manager.handle_headers_pipeline(peer(), part, &requests).await;
            let last = i == messages.len() - 1;
            let expected = last.then_some(FORK_BASE + 2);
            assert_eq!(fork_detected(&result), expected, "message {i}: {result:?}");
        }

        assert!(manager.fork.is_none(), "followed, no longer held");
        assert!(sent(&mut rx).banned.is_empty());
    }

    /// evolite (SPV item 1, AC-T1.4): work decides, not length. On regtest heights
    /// with no difficulty rule yet, one header four thousand times harder outweighs
    /// two of ours; three easy ones don't outweigh two hard ones of ours.
    #[test_case(EASY, 0x1f00_ffff, 1, Some(2); "one harder header outweighs two")]
    #[test_case(0x1f00_ffff, EASY, 3, None; "three easier headers don't")]
    #[tokio::test]
    async fn work_not_length_decides(
        our_bits: u32,
        their_bits: u32,
        length: usize,
        expected: Option<u32>,
    ) {
        let mut ours = mined_chain(BlockHash::dummy(0), 3, EASY, 0);
        let above = mined_chain(*ours[2].hash(), 2, our_bits, 10);
        ours.extend(above);
        let (mut manager, _storage) = fork_manager(0, &ours).await;
        let (requests, _rx) = create_test_request_sender();
        let branch = mined_chain(*ours[2].hash(), length, their_bits, 100);

        let result = manager.handle_headers_pipeline(peer(), &branch, &requests).await;

        assert_eq!(fork_detected(&result), expected, "{result:?}");
        assert_eq!(manager.tip().await.unwrap().height(), 4);
    }

    /// evolite (SPV item 1, AC-T1.5): a branch held while a ChainLock arrives above its
    /// fork point is dropped, even once it has more work: the locked block is final.
    #[tokio::test]
    async fn a_held_fork_below_a_chain_lock_since_is_dropped() {
        let ours = mined_chain(BlockHash::dummy(0), 5, EASY, 0);
        let (mut manager, _storage) = fork_manager(FORK_BASE, &ours).await;
        let (requests, _rx) = create_test_request_sender();
        let branch = mined_chain(*ours[2].hash(), 3, EASY, 100);
        let result = manager.handle_headers_pipeline(peer(), &branch[..1], &requests).await;
        assert_eq!(fork_detected(&result), None, "{result:?}");
        assert!(manager.fork.is_some());

        store_chain_lock(&manager, FORK_BASE + 3).await;
        let result = manager.handle_headers_pipeline(peer(), &branch[1..], &requests).await;

        assert_eq!(fork_detected(&result), None, "{result:?}");
        assert!(manager.fork.is_none(), "dropped");
        assert_eq!(manager.tip().await.unwrap().height(), FORK_BASE + 4);
    }

    /// evolite (SPV item 1): a branch longer than [`MAX_FORK_HEADERS`] without more
    /// work than ours is not held: dropped, its peer disconnected (not banned, its
    /// headers are valid). In one message or grown past it.
    #[test_case(&[MAX_FORK_HEADERS + 1]; "in one message")]
    #[test_case(&[MAX_FORK_HEADERS, 1]; "grown past it")]
    #[tokio::test]
    async fn a_fork_too_long_without_more_work_is_dropped_and_its_peer_disconnected(
        messages: &[usize],
    ) {
        let ours = mined_chain(BlockHash::dummy(0), MAX_FORK_HEADERS + 5, EASY, 0);
        let (mut manager, _storage) = fork_manager(FORK_BASE, &ours).await;
        let (requests, mut rx) = create_test_request_sender();
        let branch = mined_chain(*ours[2].hash(), messages.iter().sum(), EASY, 1_000_000);

        let mut sent_so_far = 0;
        for &count in messages {
            let part = &branch[sent_so_far..sent_so_far + count];
            sent_so_far += count;
            let result = manager.handle_headers_pipeline(peer(), part, &requests).await;
            assert_eq!(fork_detected(&result), None, "{result:?}");
            assert!(result.is_ok(), "{result:?}");
        }

        assert!(manager.fork.is_none(), "not held");
        let sent = sent(&mut rx);
        assert_eq!(sent.disconnected, vec![peer()]);
        assert!(sent.banned.is_empty(), "{sent:?}");
        assert_eq!(manager.tip().await.unwrap().height(), FORK_BASE + MAX_FORK_HEADERS as u32 + 4);
    }

    /// The peer headers come from in these tests.
    fn peer() -> SocketAddr {
        "1.2.3.4:9999".parse().unwrap()
    }

    /// Create a manager in synced state with an initialized pipeline.
    async fn create_synced_manager() -> TestBlockHeadersManager {
        let mut manager = create_test_manager().await;
        let tip = manager.tip().await.unwrap();
        manager.pipeline.init(tip.height(), *tip.hash(), tip.height());
        manager.progress.set_state(SyncState::Synced);
        manager
    }

    /// Headers that finished downloading must reach storage even if no further
    /// `Headers` message ever arrives.
    ///
    /// Promotion used to hang off `handle_headers_pipeline` alone, so the last
    /// segment of a sync had nothing left to trigger it — the pipeline sat on
    /// fully downloaded, fully validated headers forever. Seen in the field on
    /// a testnet restore: 1,046,289 headers buffered, stored tip frozen at
    /// 1,474,000, peers healthy, chain locks still arriving.
    ///
    /// Asserts the completion too, not just the store: a regression that keeps
    /// the last segment but leaves the manager in `Syncing` is the same stall
    /// wearing a different hat.
    #[tokio::test]
    async fn test_tick_promotes_buffered_headers_with_no_further_messages() {
        let mut manager = create_test_manager().await;
        let tip = manager.tip().await.unwrap();
        let start_height = tip.height();

        manager.pipeline.init(start_height, *tip.hash(), start_height + 2);
        manager.progress.set_state(SyncState::Syncing);

        let (sender, _rx) = create_test_request_sender();
        // Issue the request the segment expects, so the headers below are a
        // legitimate answer to it rather than unsolicited.
        manager.pipeline.send_pending(&sender).unwrap();

        // The headers land in the pipeline without the manager's own
        // message-driven promotion running — the state the field stall was in.
        let headers: Vec<HashedBlockHeader> =
            Header::dummy_chain(2, *tip.hash()).iter().map(HashedBlockHeader::from).collect();
        manager.pipeline.receive_headers(&headers).unwrap();
        assert!(
            manager.pipeline.total_buffered() > 0,
            "precondition: headers are downloaded but not yet promoted"
        );
        assert_eq!(
            manager.tip().await.unwrap().height(),
            start_height,
            "precondition: storage has not advanced"
        );

        // No further `Headers` will arrive. The periodic tick is the only
        // thing left that can finish the job.
        let events = manager.tick(&sender).await.unwrap();

        assert!(
            manager.tip().await.unwrap().height() > start_height,
            "tick must promote buffered headers into storage"
        );
        assert!(
            events.iter().any(|e| matches!(e, SyncEvent::BlockHeadersStored { .. })),
            "the promotion must be reported, not done silently"
        );

        // Completing the pipeline must also be reachable from a tick: an empty
        // response closes the tip segment, and the next tick has to announce
        // the sync rather than leaving the manager in `Syncing` forever.
        manager.pipeline.send_pending(&sender).unwrap();
        manager.pipeline.receive_headers(&[]).unwrap();
        assert!(manager.pipeline.is_complete(), "precondition: nothing left to download");

        let events = manager.tick(&sender).await.unwrap();

        assert_eq!(
            manager.state(),
            SyncState::Synced,
            "a tick over a complete pipeline must finish the sync"
        );
        assert!(
            events.iter().any(|e| matches!(e, SyncEvent::BlockHeaderSyncComplete { .. })),
            "completion must be announced, or downstream managers never start"
        );
    }

    #[tokio::test]
    async fn test_block_headers_manager_new() {
        let manager = create_test_manager().await;
        assert_eq!(manager.identifier(), ManagerIdentifier::BlockHeader);
        assert_eq!(manager.state(), SyncState::WaitingForConnections);
        assert_eq!(manager.wanted_message_types(), vec![MessageType::Headers, MessageType::Inv]);
    }

    #[tokio::test]
    async fn test_headers_manager_progress() {
        let mut manager = create_test_manager().await;
        manager.progress.update_tip_height(100);
        manager.progress.update_target_height(200);
        manager.progress.add_processed(50);

        let progress = manager.progress();
        if let SyncManagerProgress::BlockHeaders(progress) = progress {
            assert_eq!(progress.state(), SyncState::WaitingForConnections);
            assert_eq!(progress.tip_height(), 100);
            assert_eq!(progress.target_height(), 200);
            assert_eq!(progress.processed(), 50);
            assert!(progress.last_activity().elapsed().as_secs() < 1);
        } else {
            panic!("Expected SyncManagerProgress::BlockHeaders");
        }
    }

    #[tokio::test]
    async fn test_headers_manager_has_pipeline() {
        let manager = create_test_manager().await;
        assert!(!manager.pipeline.is_initialized());
        assert_eq!(manager.pipeline.segment_count(), 0);
    }

    fn create_test_request_sender(
    ) -> (RequestSender, tokio::sync::mpsc::UnboundedReceiver<NetworkRequest>) {
        let (tx, rx) = unbounded_channel();
        (RequestSender::new(tx), rx)
    }

    #[tokio::test]
    async fn test_unsolicited_post_sync_header_batch_does_not_trigger_get_headers() {
        let mut manager = create_test_manager().await;
        let tip = manager.tip().await.unwrap();
        let tip_hash = *tip.hash();

        // Simulate completed sync: pipeline initialized with tip segment marked complete
        manager.pipeline.init(0, tip_hash, 0);
        manager.pipeline.mark_tip_complete();
        manager.progress.set_state(SyncState::Synced);

        let (sender, mut rx) = create_test_request_sender();

        let headers = Header::dummy_chain(2, tip_hash)
            .into_iter()
            .map(HashedBlockHeader::from)
            .collect::<Vec<_>>();

        let events = manager.handle_headers_pipeline(peer(), &headers, &sender).await.unwrap();

        // Header should have been stored
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            SyncEvent::BlockHeadersStored {
                tip_height: 2
            }
        ));

        // No GetHeaders request should have been sent
        assert!(rx.try_recv().is_err());

        // Tip segment marked complete again for the next unsolicited header
        assert!(manager.pipeline.is_tip_complete());
    }

    #[tokio::test]
    async fn test_response_cycle_refills_window_after_ordered_drain() {
        let mut manager = create_test_manager().await;
        let stored_tip = manager.tip().await.unwrap();
        let chain = Header::dummy_chain(ACTIVE_SEGMENT_WINDOW, *stored_tip.hash());

        let mut segments = Vec::new();
        for (id, header) in chain.iter().enumerate() {
            let start_hash = if id == 0 {
                *stored_tip.hash()
            } else {
                chain[id - 1].block_hash()
            };
            let target_hash = header.block_hash();
            let mut segment = SegmentState::new(
                id,
                id as u32,
                start_hash,
                Some(id as u32 + 1),
                Some(target_hash),
            );
            if id == 0 {
                segment.coordinator.mark_sent(&[start_hash]);
            } else {
                segment.current_tip_hash = target_hash;
                segment.current_height = id as u32 + 1;
                segment.complete = true;
                segment.buffered_headers.push((*header).into());
            }
            segments.push(segment);
        }
        let next_locator = chain.last().expect("non-empty chain").block_hash();
        segments.push(SegmentState::new(
            ACTIVE_SEGMENT_WINDOW,
            ACTIVE_SEGMENT_WINDOW as u32,
            next_locator,
            None,
            None,
        ));
        manager.pipeline.set_segments_for_test(segments);
        manager.progress.set_state(SyncState::Syncing);

        let (requests, mut rx) = create_test_request_sender();
        let events = manager.handle_headers_pipeline(peer(), &[chain[0].into()], &requests).await.unwrap();

        assert_eq!(events.len(), ACTIVE_SEGMENT_WINDOW);
        match rx.try_recv().expect("response cycle did not refill active window") {
            NetworkRequest::SendMessage(NetworkMessage::GetHeaders(request)) => {
                assert_eq!(request.locator_hashes[0], next_locator);
            }
            other => panic!("Expected GetHeaders, got {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_peer_tip_announcement_lifecycle() {
        let mut manager = create_synced_manager().await;
        let (requests, mut rx) = create_test_request_sender();

        let addr: SocketAddr = "1.2.3.4:9999".parse().unwrap();
        let connect = NetworkEvent::PeerConnected {
            address: addr,
        };

        // Connect sends a peer-targeted GetHeaders
        let events = manager.handle_network_event(&connect, &requests).await.unwrap();
        assert!(events.is_empty());
        assert!(manager.announced_peers.contains(&addr));
        match rx.try_recv().unwrap() {
            NetworkRequest::SendMessageToPeer(_, target_addr) => {
                assert_eq!(target_addr, addr);
            }
            other => panic!("Expected SendMessageToPeer, got {:?}", other),
        }

        // Same peer again sends nothing (already announced)
        manager.handle_network_event(&connect, &requests).await.unwrap();
        assert!(rx.try_recv().is_err());

        // Disconnect removes from announced set
        let disconnect = NetworkEvent::PeerDisconnected {
            address: addr,
        };
        manager.handle_network_event(&disconnect, &requests).await.unwrap();
        assert!(!manager.announced_peers.contains(&addr));

        // Reconnect sends GetHeaders again
        manager.handle_network_event(&connect, &requests).await.unwrap();
        assert!(manager.announced_peers.contains(&addr));
        assert!(rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn test_peer_tip_announcement_guards() {
        // Not synced: peer connect does nothing
        let mut manager = create_test_manager().await;
        let (requests, mut rx) = create_test_request_sender();
        let addr: SocketAddr = "1.2.3.4:9999".parse().unwrap();
        let connect = NetworkEvent::PeerConnected {
            address: addr,
        };

        manager.handle_network_event(&connect, &requests).await.unwrap();
        assert!(!manager.announced_peers.contains(&addr));
        assert!(rx.try_recv().is_err());

        // Active catch-up: peer connect skipped while pipeline has pending request
        let mut manager = create_synced_manager().await;
        manager.pipeline.reset_tip_segment();
        manager.pipeline.send_pending(&requests).unwrap();
        rx.try_recv().unwrap(); // drain the pipeline GetHeaders

        manager.handle_network_event(&connect, &requests).await.unwrap();
        assert!(!manager.announced_peers.contains(&addr));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_disconnect_preserves_pipeline_and_resumes_from_advanced_tip() {
        let mut manager = create_test_manager().await;
        let (requests, mut rx) = create_test_request_sender();

        // Use a target below the first testnet checkpoint (50000) so the
        // pipeline produces a single open-ended tip segment.
        let initial_event = NetworkEvent::PeersUpdated {
            connected_count: 1,
            best_height: Some(40_000),
            addresses: vec![],
        };
        manager.handle_network_event(&initial_event, &requests).await.unwrap();
        assert_eq!(manager.state(), SyncState::Syncing);
        assert!(manager.pipeline.is_initialized());
        assert_eq!(manager.pipeline.segment_count(), 1);

        let initial_locator = match rx.try_recv().expect("initial GetHeaders not sent") {
            NetworkRequest::SendMessage(NetworkMessage::GetHeaders(msg)) => msg.locator_hashes[0],
            other => panic!("Expected GetHeaders, got {:?}", other),
        };
        assert!(rx.try_recv().is_err());

        // Simulate a peer response. The single tip segment drains its buffer
        // through take_ready_to_store, advancing the storage tip and the
        // segment's current_tip_hash to advanced_hash.
        let header = Header::dummy_chain(1, initial_locator).remove(0);
        let advanced_hash = header.block_hash();
        manager.handle_headers_pipeline(peer(), &[header.into()], &requests).await.unwrap();

        // Drain the follow-up GetHeaders that send_pending issued.
        match rx.try_recv().expect("follow-up GetHeaders not sent") {
            NetworkRequest::SendMessage(NetworkMessage::GetHeaders(msg)) => {
                assert_eq!(msg.locator_hashes[0], advanced_hash);
            }
            other => panic!("Expected GetHeaders, got {:?}", other),
        }
        assert!(rx.try_recv().is_err());

        let disconnect_event = NetworkEvent::PeersUpdated {
            connected_count: 0,
            best_height: Some(40_000),
            addresses: vec![],
        };
        manager.handle_network_event(&disconnect_event, &requests).await.unwrap();
        assert_eq!(manager.state(), SyncState::WaitingForConnections);
        assert!(
            manager.pipeline.is_initialized(),
            "pipeline must survive disconnect so resume can reuse validated state"
        );
        assert_eq!(manager.pipeline.segment_count(), 1);

        // Reconnect: start_sync must skip pipeline.init and resume by sending
        // GetHeaders from each segment's preserved current_tip_hash.
        manager.handle_network_event(&initial_event, &requests).await.unwrap();
        assert_eq!(manager.state(), SyncState::Syncing);

        let resumed_locator = match rx.try_recv().expect("resumed GetHeaders not sent") {
            NetworkRequest::SendMessage(NetworkMessage::GetHeaders(msg)) => msg.locator_hashes[0],
            other => panic!("Expected GetHeaders, got {:?}", other),
        };
        assert_eq!(
            resumed_locator, advanced_hash,
            "GetHeaders on reconnect must use the preserved current_tip_hash"
        );
        assert_ne!(resumed_locator, initial_locator);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_disconnect_after_sync_resumes_and_catches_up() {
        let mut manager = create_synced_manager().await;
        let tip = manager.tip().await.unwrap();
        let synced_hash = *tip.hash();
        manager.pipeline.mark_tip_complete();
        assert!(manager.pipeline.is_tip_complete());

        let (requests, mut rx) = create_test_request_sender();

        let disconnect_event = NetworkEvent::PeersUpdated {
            connected_count: 0,
            best_height: Some(tip.height()),
            addresses: vec![],
        };
        manager.handle_network_event(&disconnect_event, &requests).await.unwrap();
        assert_eq!(manager.state(), SyncState::WaitingForConnections);
        assert!(manager.pipeline.is_initialized());

        // Reconnect with a higher peer best_height (a new block was mined).
        let reconnect_event = NetworkEvent::PeersUpdated {
            connected_count: 1,
            best_height: Some(tip.height() + 1),
            addresses: vec![],
        };
        manager.handle_network_event(&reconnect_event, &requests).await.unwrap();
        assert_eq!(manager.state(), SyncState::Syncing);

        let resumed_locator = match rx.try_recv().expect("resumed GetHeaders not sent") {
            NetworkRequest::SendMessage(NetworkMessage::GetHeaders(msg)) => msg.locator_hashes[0],
            other => panic!("Expected GetHeaders, got {:?}", other),
        };
        assert_eq!(resumed_locator, synced_hash);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_empty_headers_after_tip_announcement_is_harmless() {
        let mut manager = create_synced_manager().await;
        manager.pipeline.mark_tip_complete();
        let (requests, mut rx) = create_test_request_sender();

        // Announce tip to a new peer
        let addr: SocketAddr = "1.2.3.4:9999".parse().unwrap();
        let connect = NetworkEvent::PeerConnected {
            address: addr,
        };
        manager.handle_network_event(&connect, &requests).await.unwrap();
        rx.try_recv().unwrap(); // drain the GetHeaders request

        // Peer responds with empty headers (same height as us)
        let events = manager.handle_headers_pipeline(peer(), &[], &requests).await.unwrap();

        // No events emitted, no requests sent, tip segment stays complete
        assert!(events.is_empty());
        assert!(rx.try_recv().is_err());
        assert!(manager.pipeline.is_tip_complete());
    }
}
