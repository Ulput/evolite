//! What the SPV sync reports, as the core keeps it. The client itself now lives inside
//! platform-wallet ([`super::chain`]); the [`Reporter`] is its event handler and turns progress,
//! peers and errors into a [`SyncStatus`] the snapshot is made from.
//!
//! dash-spv syncs block headers and masternode lists, validates ChainLocks (BLS signatures
//! against the quorums of our own masternode list) and matches compact block filters against
//! the wallets platform-wallet holds. Validation is always `ValidationMode::Full` (the dash-spv
//! default) and must stay that way.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use dash_spv::EventHandler;
use dash_spv::network::NetworkEvent;
use dash_spv::sync::{ProgressPercentage, SyncEvent, SyncProgress, SyncState};
use key_wallet::dashcore::Txid;
use platform_wallet::events::PlatformEventHandler;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    #[default]
    Stopped,
    Starting,
    Connecting,
    Syncing,
    Synced,
    Error,
}

/// Where one sync manager is: its height and the height it is heading for.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    pub height: u32,
    pub target: u32,
    pub done: bool,
}

/// What dash-spv reports, as the core keeps it; turned into `evolite_api::SyncInfo` for the UI.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub network: &'static str,
    pub phase: Phase,
    pub peers: usize,
    pub headers: Option<Stage>,
    pub masternodes: Option<Stage>,
    /// Compact block filters, only with a wallet. Its height is where every batch is fully
    /// done (scanned, and the blocks it matched downloaded and applied).
    pub filters: Option<Stage>,
    /// How far filters are downloaded and stored. It runs ahead of the filters' height: a batch
    /// with a match waits for its blocks, and dash-spv commits batches only in order, so the
    /// committed height stands still for minutes while filters keep coming, then jumps.
    pub filters_stored: u32,
    /// How far the filter headers are, which the filters follow.
    pub filter_headers: u32,
    /// Headers downloaded past `headers.height`: dash-spv fetches segments side by side and
    /// moves its tip only when they join up, so the tip alone stands and jumps.
    pub headers_buffered: u32,
    /// Where this run began, for the progress of each stage: the headers' tip at the first
    /// report, the lowest height the filters were done to (lower again on a rescan). Without
    /// them a warm folder, all of it near the tip, said «100 %» from the start (BUG-60).
    pub headers_from: Option<u32>,
    pub filters_from: Option<u32>,
    /// The height the filters were done to when, this run, they were at the headers' tip;
    /// gone again when they go below it (a rewind). dash-spv says the filters are done against
    /// the filter headers it has: right after a start that is the height of the last run, with
    /// the headers already at the network's tip — the coins then are the last run's (BUG-77 r1
    /// must). A new block (headers one ahead for a few seconds) keeps it: no flicker.
    pub filters_at_tip: Option<u32>,
    /// A node answered this run's request for the masternode lists (QRInfo): dash-spv checks
    /// their quorums now. Its list height moves only at the very end, so this is all there is
    /// to tell the two apart (BUG-61).
    pub lists_answered: bool,
    /// Height of the best ChainLock whose BLS signature was verified (this run or, restored
    /// from storage, an earlier one). The counters below cover this run only.
    pub chainlock_height: Option<u32>,
    pub chainlocks_valid: u32,
    pub chainlocks_invalid: u32,
    /// Overall progress, 0–100.
    pub percent: f64,
    pub error: Option<String>,
    /// "Own node" this run connects to alone; `None`: nodes from DNS seeds.
    pub core_node: Option<String>,
}

impl SyncStatus {
    /// Takes over everything `dash-spv` reports in its aggregate progress.
    pub fn apply_progress(&mut self, progress: &SyncProgress) {
        self.phase = match progress.state() {
            SyncState::WaitForEvents | SyncState::WaitingForConnections => Phase::Connecting,
            SyncState::Syncing => Phase::Syncing,
            SyncState::Synced => Phase::Synced,
            SyncState::Error => Phase::Error,
        };
        if let Ok(h) = progress.headers() {
            self.headers = Some(Stage {
                height: h.tip_height(),
                target: h.target_height().max(h.tip_height()),
                done: h.state() == SyncState::Synced,
            });
            self.headers_buffered = h.buffered();
            self.headers_from.get_or_insert(h.tip_height());
        }
        if let Ok(m) = progress.masternodes() {
            self.masternodes = Some(Stage {
                height: m.current_height(),
                target: m.target_height().max(m.current_height()),
                done: m.state() == SyncState::Synced,
            });
            // The counters start at nought with each client: a diff applied or a cycle of
            // quorums verified means the QRInfo answer came.
            self.lists_answered = m.diffs_processed() > 0 || m.validated_cycles() > 0;
        }
        if let Ok(f) = progress.filter_headers() {
            self.filter_headers = f.current_height();
        }
        if let Ok(f) = progress.filters() {
            let target = self.headers.map_or(0, |h| h.target);
            self.filters = Some(Stage {
                height: f.committed_height(),
                target: target.max(f.committed_height()),
                done: f.state() == SyncState::Synced,
            });
            self.filters_stored = f.stored_height().max(f.committed_height());
            // A new or restored wallet's first report says 0 (dash-spv begins at the wallet's
            // synced height, 0 for it, and only then at the birth − 1): no start yet, or the
            // filters counted from the genesis said ~90 % at once (BUG-60 r1, should 1).
            if f.committed_height() > 0 {
                let from = self.filters_from.get_or_insert(f.committed_height());
                *from = (*from).min(f.committed_height());
            }
        }
        if let Ok(c) = progress.chainlocks() {
            let height = c.best_validated_height();
            self.chainlock_height = (height > 0).then_some(height);
            self.chainlocks_valid = c.valid();
            self.chainlocks_invalid = c.invalid();
        }
        self.percent = (progress.percentage() * 100.0).clamp(0.0, 100.0);
        self.latch_filters_at_tip();
    }

    /// Moves [`Self::filters_at_tip`]: up when the filters are done to the headers' tip, away
    /// when they go below it (a rewind).
    pub fn latch_filters_at_tip(&mut self) {
        match (self.headers, self.filters) {
            (Some(h), Some(f)) if h.done && f.done && f.height >= h.height => {
                self.filters_at_tip = Some(f.height);
            }
            (_, Some(f)) if self.filters_at_tip.is_some_and(|at| f.height < at) => {
                self.filters_at_tip = None;
            }
            (_, None) => self.filters_at_tip = None,
            _ => {}
        }
    }

    /// A new client is coming: its stages begin where it finds them, not where the last one
    /// left them.
    pub fn begin_run(&mut self) {
        self.filters_at_tip = None;
        self.headers_from = None;
        self.filters_from = None;
        self.lists_answered = false;
    }
}

/// Transactions whose InstantLock dash-spv verified (its BLS signature against the cycle's
/// quorum), by txid. dash-spv marks a transaction InstantSend on any lock a peer sends, verified
/// or not (R-1); evolite trusts only these. The newest [`VerifiedLocks::CAP`] are kept: locks of
/// the whole network's mempool pass through here, and a transaction in a block no longer needs
/// its lock. Until the app closes.
#[derive(Clone, Default)]
pub struct VerifiedLocks(Arc<Mutex<VerifiedSet>>);

#[derive(Default)]
struct VerifiedSet {
    set: HashSet<Txid>,
    order: VecDeque<Txid>,
}

impl VerifiedLocks {
    pub const CAP: usize = 20_000;

    pub fn contains(&self, txid: &Txid) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set
            .contains(txid)
    }

    /// Notes `txid`; false when it was noted already.
    fn insert(&self, txid: Txid) -> bool {
        let mut locks = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !locks.set.insert(txid) {
            return false;
        }
        locks.order.push_back(txid);
        while locks.order.len() > Self::CAP {
            if let Some(old) = locks.order.pop_front() {
                locks.set.remove(&old);
            }
        }
        true
    }
}

/// Keeps the latest status and tells the core about every change.
#[derive(Clone)]
pub struct Reporter {
    status: Arc<Mutex<SyncStatus>>,
    publish: Arc<dyn Fn(&SyncStatus) + Send + Sync>,
    instant_locks: VerifiedLocks,
}

impl Reporter {
    pub fn new(
        status: Arc<Mutex<SyncStatus>>,
        publish: Arc<dyn Fn(&SyncStatus) + Send + Sync>,
    ) -> Self {
        Self {
            status,
            publish,
            instant_locks: VerifiedLocks::default(),
        }
    }

    /// Verified InstantLocks go to `locks` (shared with the chain, which outlives a client).
    pub fn with_instant_locks(mut self, locks: VerifiedLocks) -> Self {
        self.instant_locks = locks;
        self
    }

    pub fn update(&self, change: impl FnOnce(&mut SyncStatus)) {
        let snapshot = {
            let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
            change(&mut status);
            status.clone()
        };
        (self.publish)(&snapshot);
    }
}

impl EventHandler for Reporter {
    fn on_sync_event(&self, event: &SyncEvent) {
        // Sent on the first check and again when a pending lock is verified later
        // (`validate_pending`); an unverified one is not trusted (R-1, BUG-26).
        if let SyncEvent::InstantLockReceived {
            instant_lock,
            validated: true,
        } = event
            && self.instant_locks.insert(instant_lock.txid)
        {
            // The history shows it as InstantSend from now on.
            self.update(|_| {});
        }
    }

    fn on_progress(&self, progress: &SyncProgress) {
        self.update(|s| s.apply_progress(progress));
    }

    fn on_network_event(&self, event: &NetworkEvent) {
        if let NetworkEvent::PeersUpdated {
            connected_count, ..
        } = event
        {
            let peers = *connected_count;
            self.update(|s| s.peers = peers);
        }
    }

    fn on_wallet_event(&self, _event: &platform_wallet::events::WalletEvent) {
        // New transactions and balance changes: the snapshot is rebuilt from the wallet.
        self.update(|_| {});
    }

    fn on_error(&self, error: &str) {
        let error = error.to_owned();
        self.update(|s| {
            s.phase = Phase::Error;
            s.error = Some(error);
        });
    }
}

impl PlatformEventHandler for Reporter {
    fn on_platform_address_sync_completed(
        &self,
        _summary: &platform_wallet::PlatformAddressSyncSummary,
    ) {
        // Platform address balances: the snapshot is rebuilt from the wallet.
        self.update(|_| {});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_spv::sync::{BlockHeadersProgress, ChainLockProgress, MasternodesProgress};

    #[test]
    fn maps_progress_to_status() {
        let mut headers = BlockHeadersProgress::default();
        headers.update_target_height(1_300_000);
        headers.update_tip_height(650_000);
        headers.set_state(SyncState::Syncing);
        let mut masternodes = MasternodesProgress::default();
        masternodes.update_target_height(1_299_000);
        masternodes.update_current_height(1_200_000);
        masternodes.set_state(SyncState::Syncing);
        let mut chainlocks = ChainLockProgress::default();
        chainlocks.update_best_validated_height(1_299_990);
        chainlocks.add_valid(3);

        let mut progress = SyncProgress::default();
        progress.update_headers(headers);
        progress.update_masternodes(masternodes);
        progress.update_chainlocks(chainlocks);

        let mut status = SyncStatus::default();
        status.apply_progress(&progress);

        assert_eq!(status.phase, Phase::Syncing);
        assert_eq!(
            status.headers,
            Some(Stage {
                height: 650_000,
                target: 1_300_000,
                done: false
            })
        );
        assert_eq!(status.masternodes.map(|m| m.height), Some(1_200_000));
        assert_eq!(status.chainlock_height, Some(1_299_990));
        assert_eq!(status.chainlocks_valid, 3);
        assert!(status.percent > 0.0 && status.percent < 100.0);
    }

    /// BUG-60/61: where this run began each stage, and whether a node answered for the lists.
    #[test]
    fn remembers_where_the_run_began() {
        use dash_spv::sync::FiltersProgress;
        let report = |tip: u32, buffered: u32, committed: u32, diffs: u32| {
            let mut headers = BlockHeadersProgress::default();
            headers.update_target_height(1_586_494);
            headers.update_tip_height(tip);
            headers.update_buffered(buffered);
            let mut masternodes = MasternodesProgress::default();
            masternodes.update_current_height(1_585_800);
            masternodes.update_target_height(1_586_494);
            masternodes.add_diffs_processed(diffs);
            let mut filters = FiltersProgress::default();
            filters.update_committed_height(committed);
            filters.update_stored_height(committed);
            let mut progress = SyncProgress::default();
            progress.update_headers(headers);
            progress.update_masternodes(masternodes);
            progress.update_filters(filters);
            progress
        };
        let mut status = SyncStatus::default();
        status.apply_progress(&report(1_585_857, 0, 1_585_832, 0));
        assert!(!status.lists_answered);
        status.apply_progress(&report(1_586_000, 200, 1_586_100, 1));
        assert_eq!(
            status.headers_from,
            Some(1_585_857),
            "the first report's tip"
        );
        assert_eq!(status.headers_buffered, 200);
        assert_eq!(status.filters_from, Some(1_585_832));
        assert!(status.lists_answered, "a diff came after the QRInfo answer");
        // A rescan takes the filters back: they count from there.
        status.apply_progress(&report(1_586_000, 0, 1_500_000, 1));
        assert_eq!(status.filters_from, Some(1_500_000));
        // The next client begins afresh.
        status.begin_run();
        status.apply_progress(&report(1_586_494, 0, 1_586_494, 0));
        assert_eq!(status.headers_from, Some(1_586_494));
        assert_eq!(status.filters_from, Some(1_586_494));
        assert!(!status.lists_answered);
    }

    /// BUG-60 r1 (should 1): a new or restored wallet's filters begin at its synced height, 0,
    /// and only then at the birth − 1 — the start is the birth, not the genesis.
    #[test]
    fn a_restored_wallet_counts_its_filters_from_the_birth() {
        use dash_spv::sync::FiltersProgress;
        let report = |committed: u32| {
            let mut headers = BlockHeadersProgress::default();
            headers.update_target_height(1_586_494);
            headers.update_tip_height(1_586_494);
            headers.set_state(SyncState::Synced);
            let mut filters = FiltersProgress::default();
            filters.update_committed_height(committed);
            filters.update_stored_height(committed);
            let mut progress = SyncProgress::default();
            progress.update_headers(headers);
            progress.update_filters(filters);
            progress
        };
        let birth = 1_400_000;
        let mut status = SyncStatus::default();
        status.apply_progress(&report(0));
        assert_eq!(status.filters_from, None, "0 is no start");
        let shown = super::super::convert::sync_info(&status, true);
        assert_eq!(shown.stage, evolite_api::SyncStage::Filters);
        assert_eq!(shown.progress, None, "no count before the start is known");
        status.apply_progress(&report(birth - 1));
        assert_eq!(status.filters_from, Some(birth - 1));
        status.apply_progress(&report(birth + 93_247));
        assert_eq!(status.filters_from, Some(birth - 1));
        let part = super::super::convert::sync_info(&status, true)
            .progress
            .expect("counted");
        // A half of the way from the birth, not 94 % of the height.
        assert!((part - 0.5).abs() < 0.01, "{part}");
    }

    #[test]
    fn no_chainlock_until_one_is_validated() {
        let mut progress = SyncProgress::default();
        progress.update_chainlocks(ChainLockProgress::default());
        let mut status = SyncStatus::default();
        status.apply_progress(&progress);
        assert_eq!(status.chainlock_height, None);
    }

    #[test]
    fn serialises_in_camel_case() {
        let json = serde_json::to_value(SyncStatus::default()).unwrap();
        assert_eq!(json["phase"], "stopped");
        assert!(json.get("chainlockHeight").is_some());
    }

    /// R-1 / BUG-26: only an InstantLock whose signature dash-spv verified counts.
    #[test]
    fn only_a_verified_instant_lock_counts() {
        use key_wallet::dashcore::InstantLock;
        use key_wallet::dashcore::hashes::Hash;
        let published = Arc::new(Mutex::new(0));
        let count = published.clone();
        let locks = VerifiedLocks::default();
        let reporter = Reporter::new(
            Arc::new(Mutex::new(SyncStatus::default())),
            Arc::new(move |_: &SyncStatus| *count.lock().unwrap() += 1),
        )
        .with_instant_locks(locks.clone());
        let lock = InstantLock {
            txid: Txid::from_byte_array([7; 32]),
            ..Default::default()
        };
        reporter.on_sync_event(&SyncEvent::InstantLockReceived {
            instant_lock: lock.clone(),
            validated: false,
        });
        assert!(
            !locks.contains(&lock.txid),
            "a lock not verified is not trusted"
        );
        assert_eq!(*published.lock().unwrap(), 0);
        reporter.on_sync_event(&SyncEvent::InstantLockReceived {
            instant_lock: lock.clone(),
            validated: true,
        });
        assert!(locks.contains(&lock.txid));
        assert_eq!(*published.lock().unwrap(), 1, "the snapshot is made again");
        // The same lock again (verified once more from the pending queue): nothing new.
        reporter.on_sync_event(&SyncEvent::InstantLockReceived {
            instant_lock: lock.clone(),
            validated: true,
        });
        assert_eq!(*published.lock().unwrap(), 1);
    }

    /// The network's mempool passes through: only the newest locks are kept.
    #[test]
    fn verified_locks_keep_the_newest() {
        use key_wallet::dashcore::hashes::Hash;
        let locks = VerifiedLocks::default();
        let txid = |n: usize| {
            let mut bytes = [0u8; 32];
            bytes[..8].copy_from_slice(&(n as u64).to_le_bytes());
            Txid::from_byte_array(bytes)
        };
        for n in 0..=VerifiedLocks::CAP {
            assert!(locks.insert(txid(n)));
        }
        assert!(!locks.contains(&txid(0)), "the oldest went");
        assert!(locks.contains(&txid(1)) && locks.contains(&txid(VerifiedLocks::CAP)));
    }

    #[test]
    fn reporter_publishes_every_change() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let status = Arc::new(Mutex::new(SyncStatus::default()));
        let reporter = Reporter::new(
            status.clone(),
            Arc::new(move |s: &SyncStatus| {
                sink.lock().unwrap().push(s.peers);
            }),
        );
        reporter.on_network_event(&NetworkEvent::PeersUpdated {
            connected_count: 5,
            addresses: vec![],
            best_height: None,
        });
        reporter.on_error("boom");
        assert_eq!(*seen.lock().unwrap(), vec![5, 5]);
        let status = status.lock().unwrap();
        assert_eq!(status.phase, Phase::Error);
        assert_eq!(status.error.as_deref(), Some("boom"));
    }
}
