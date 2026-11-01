//! What the SPV sync reports, as the core keeps it. The client itself now lives inside
//! platform-wallet ([`super::chain`]); the [`Reporter`] is its event handler and turns progress,
//! peers and errors into a [`SyncStatus`] the snapshot is made from.
//!
//! dash-spv syncs block headers and masternode lists, validates ChainLocks (BLS signatures
//! against the quorums of our own masternode list) and matches compact block filters against
//! the wallets platform-wallet holds. Validation is always `ValidationMode::Full` (the dash-spv
//! default) and must stay that way.

use std::sync::{Arc, Mutex};

use dash_spv::EventHandler;
use dash_spv::network::NetworkEvent;
use dash_spv::sync::{ProgressPercentage, SyncProgress, SyncState};
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
        }
        if let Ok(m) = progress.masternodes() {
            self.masternodes = Some(Stage {
                height: m.current_height(),
                target: m.target_height().max(m.current_height()),
                done: m.state() == SyncState::Synced,
            });
        }
        if let Ok(f) = progress.filters() {
            let target = self.headers.map_or(0, |h| h.target);
            self.filters = Some(Stage {
                height: f.committed_height(),
                target: target.max(f.committed_height()),
                done: f.state() == SyncState::Synced,
            });
            self.filters_stored = f.stored_height().max(f.committed_height());
        }
        if let Ok(c) = progress.chainlocks() {
            let height = c.best_validated_height();
            self.chainlock_height = (height > 0).then_some(height);
            self.chainlocks_valid = c.valid();
            self.chainlocks_invalid = c.invalid();
        }
        self.percent = (progress.percentage() * 100.0).clamp(0.0, 100.0);
    }
}

/// Keeps the latest status and tells the core about every change.
#[derive(Clone)]
pub struct Reporter {
    status: Arc<Mutex<SyncStatus>>,
    publish: Arc<dyn Fn(&SyncStatus) + Send + Sync>,
}

impl Reporter {
    pub fn new(
        status: Arc<Mutex<SyncStatus>>,
        publish: Arc<dyn Fn(&SyncStatus) + Send + Sync>,
    ) -> Self {
        Self { status, publish }
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
