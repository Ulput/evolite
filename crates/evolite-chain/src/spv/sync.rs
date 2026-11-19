//! What the SPV sync reports, as the core keeps it. The client itself now lives inside
//! platform-wallet ([`crate::chain`]); the [`Reporter`] is its event handler and turns progress,
//! peers and errors into a [`SyncStatus`] the snapshot is made from.
//!
//! dash-spv syncs block headers and masternode lists, validates ChainLocks (BLS signatures
//! against the quorums of our own masternode list) and matches compact block filters against
//! the wallets platform-wallet holds. Validation is always `ValidationMode::Full` (the dash-spv
//! default) and must stay that way.

use std::collections::{HashMap, HashSet, VecDeque};
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
    /// must). A new block keeps it — the headers one ahead for a few seconds, then the filters
    /// syncing again for a moment (dash-spv's `handle_new_filter_headers`): no flicker. It goes
    /// when the headers are not done (all peers lost: dash-spv waits for connections again) and
    /// on a rewind; after either, the filters have to reach the headers' tip again.
    pub filters_at_tip: Option<u32>,
    /// BUG-162: how far the open wallet itself is synced (key-wallet's `synced_height`), noted
    /// by the core at each snapshot; none before the first one. dash-spv's filters run with no
    /// wallet at all (a first start, while the phrase is being written down) and reach the tip
    /// for nobody; then the new wallet's own pass begins at 0. Only this height says the
    /// wallet's coins are known.
    pub wallet_synced: Option<u32>,
    /// [`Self::filters_at_tip`] for the wallet: the headers' height its own sync reached, kept
    /// through a new block (the wallet a few seconds behind), gone when the headers are not
    /// done or the wallet's height falls below it (its pass restarted).
    pub wallet_at_tip: Option<u32>,
    /// A node answered this run's request for the masternode lists (QRInfo): dash-spv checks
    /// their quorums now. Its list height moves only at the very end, so this is all there is
    /// to tell the two apart (BUG-61).
    pub lists_answered: bool,
    /// Platform has answered for the address balances this run (BUG-91): before that a 0 there is
    /// not an amount.
    pub platform_address_read: bool,
    /// BUG-240 r2: when a pass of the pool's scan last succeeded for each wallet in this run
    /// (unix seconds). Before that its balance is not known: platform-wallet stamps a failed
    /// pass too, and a pool bound without notes on disk reads as a confident 0.
    #[serde(skip)]
    pub pool_passed: std::collections::BTreeMap<[u8; 32], u64>,
    /// BUG-240 r2 (3): the failed passes in a row of each wallet's pool since its last success,
    /// and the kind of the last failure.
    #[serde(skip)]
    pub pool_failed: std::collections::BTreeMap<[u8; 32], (u32, &'static str)>,
    /// BUG-240 r4 (R2): the passes in a row since its last success that failed on something
    /// that passes by itself — the connection, a node behind, SPV's quorums not ready — and
    /// the kind of the last. Counted apart, to a higher threshold.
    #[serde(skip)]
    pub pool_waited: std::collections::BTreeMap<[u8; 32], (u32, &'static str)>,
    /// Height of the best ChainLock whose BLS signature was verified (this run or, restored
    /// from storage, an earlier one). The counters below cover this run only.
    pub chainlock_height: Option<u32>,
    pub chainlocks_valid: u32,
    pub chainlocks_invalid: u32,
    /// Overall progress, 0–100.
    pub percent: f64,
    pub error: Option<String>,
    /// The store refused a folder: what the person has to do ([`crate::folders::refused`]).
    pub folder_refused: Option<String>,
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
        self.latch_wallet_at_tip();
    }

    /// BUG-162: the open wallet's own synced height, from the core (none: no wallet open).
    pub fn note_wallet_synced(&mut self, height: Option<u32>) {
        self.wallet_synced = height;
        self.latch_wallet_at_tip();
    }

    /// Moves [`Self::wallet_at_tip`] as [`Self::latch_filters_at_tip`] moves its own.
    pub fn latch_wallet_at_tip(&mut self) {
        match (self.headers, self.wallet_synced) {
            (Some(h), Some(w)) if h.done && w >= h.height => self.wallet_at_tip = Some(w),
            (Some(h), _) if !h.done => self.wallet_at_tip = None,
            (_, Some(w)) if self.wallet_at_tip.is_some_and(|at| w < at) => {
                self.wallet_at_tip = None;
            }
            (_, None) => self.wallet_at_tip = None,
            _ => {}
        }
    }

    /// BUG-162: a wallet joined the running client — what the client did before was not for
    /// it: the latches and the filters' start go, its own pass decides.
    pub fn wallet_attached(&mut self) {
        self.filters_at_tip = None;
        self.filters_from = None;
        self.wallet_synced = None;
        self.wallet_at_tip = None;
    }

    /// Moves [`Self::filters_at_tip`]: up when the filters are done to the headers' tip, away
    /// when the headers are not done or the filters go below it (a rewind: the headers may go
    /// on meanwhile, and the old height is then no longer the tip).
    pub fn latch_filters_at_tip(&mut self) {
        match (self.headers, self.filters) {
            (Some(h), Some(f)) if h.done && f.done && f.height >= h.height => {
                self.filters_at_tip = Some(f.height);
            }
            (Some(h), _) if !h.done => self.filters_at_tip = None,
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
        self.wallet_at_tip = None;
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
    /// The newest locks' own bytes, for the wallet to keep those of its transactions across a
    /// restart (BUG-173): at most [`VerifiedLocks::BYTES_CAP`].
    bytes: HashMap<Txid, Vec<u8>>,
    bytes_order: VecDeque<Txid>,
}

impl VerifiedLocks {
    pub const CAP: usize = 20_000;
    /// How many locks' bytes are kept: the wallet keeps its own at the next snapshot, seconds
    /// after the lock came.
    pub const BYTES_CAP: usize = 2_000;

    /// The bytes of `txid`'s verified InstantLock (`isdlock`), while they are kept here.
    pub fn bytes(&self, txid: &Txid) -> Option<Vec<u8>> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bytes
            .get(txid)
            .cloned()
    }

    /// Keeps the bytes of `txid`'s verified lock among the newest [`Self::BYTES_CAP`].
    pub(crate) fn keep_bytes(&self, txid: Txid, bytes: Vec<u8>) {
        let mut locks = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if locks.bytes.insert(txid, bytes).is_none() {
            locks.bytes_order.push_back(txid);
        }
        while locks.bytes_order.len() > Self::BYTES_CAP {
            if let Some(old) = locks.bytes_order.pop_front() {
                locks.bytes.remove(&old);
            }
        }
    }

    pub fn contains(&self, txid: &Txid) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set
            .contains(txid)
    }

    /// Notes `txid`; false when it was noted already.
    pub(crate) fn insert(&self, txid: Txid) -> bool {
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
    /// BUG-240 r4 (R3): told after a pass that failed on something passing: one early pass.
    pool_retry: Option<Arc<tokio::sync::Notify>>,
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
            pool_retry: None,
        }
    }

    /// BUG-240 r4 (R3): `retry` is told after each pool pass that failed on something passing.
    pub fn with_pool_retry(mut self, retry: Arc<tokio::sync::Notify>) -> Self {
        self.pool_retry = Some(retry);
        self
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
            // Its bytes, for the wallet to keep with its own transaction (BUG-173).
            self.instant_locks.keep_bytes(
                instant_lock.txid,
                key_wallet::dashcore::consensus::encode::serialize(instant_lock),
            );
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
        summary: &platform_wallet::PlatformAddressSyncSummary,
    ) {
        // Platform address balances: the snapshot is rebuilt from the wallet.
        let answered = address_pass_answered(summary);
        self.update(|s| s.platform_address_read |= answered);
    }

    /// BUG-240 r2: a pass of the pool's scan — its outcome in the journal; for each wallet it
    /// scanned, the time (only then is its balance known); for each it failed, one more failure
    /// in a row. Before, every outcome was dropped.
    #[cfg(feature = "pool")]
    fn on_shielded_sync_completed(
        &self,
        summary: &platform_wallet::manager::shielded_sync::ShieldedSyncPassSummary,
    ) {
        let (record, outcomes) = pool_pass(summary);
        note_pool_pass(record);
        if outcomes.is_empty() {
            return;
        }
        let at = summary.sync_unix_seconds;
        let mut passing = false;
        self.update(|s| {
            for (wallet, failed) in outcomes {
                match failed {
                    None => {
                        s.pool_passed.insert(wallet, at);
                        s.pool_failed.remove(&wallet);
                        s.pool_waited.remove(&wallet);
                    }
                    // BUG-240 r4 (R2): the connection, a node behind, SPV's quorums not ready
                    // pass by themselves: counted apart, to a higher threshold.
                    Some(kind) if passes_by_itself(kind) => {
                        passing = true;
                        let row = s.pool_waited.entry(wallet).or_insert((0, kind));
                        *row = (row.0 + 1, kind);
                    }
                    Some(kind) => {
                        let row = s.pool_failed.entry(wallet).or_insert((0, kind));
                        *row = (row.0 + 1, kind);
                    }
                }
            }
        });
        // R3: one early pass after it, not the regular minute.
        if passing && let Some(retry) = &self.pool_retry {
            retry.notify_one();
        }
    }
}

/// Each wallet a pass of the pool scanned (`None`) or failed (`Some(kind)`).
#[cfg(feature = "pool")]
type PassOutcomes = Vec<([u8; 32], Option<&'static str>)>;

/// BUG-240 r2: one pass of the pool's scan as the journal says it, and each wallet it scanned
/// (`None`) or failed (`Some(kind)`); a skipped one is neither.
#[cfg(feature = "pool")]
fn pool_pass(
    summary: &platform_wallet::manager::shielded_sync::ShieldedSyncPassSummary,
) -> (crate::diag::Record, PassOutcomes) {
    use platform_wallet::manager::shielded_sync::WalletShieldedOutcome;
    let (mut ok, mut skipped, mut failed, mut new_notes, mut scanned) = (0, 0, 0, 0usize, 0u64);
    let mut error = "";
    let mut outcomes = Vec::new();
    for (wallet, outcome) in &summary.wallet_results {
        match outcome {
            WalletShieldedOutcome::Ok(done) => {
                ok += 1;
                new_notes += done
                    .notes_result
                    .new_notes_per_account
                    .values()
                    .sum::<usize>();
                scanned += done.notes_result.total_scanned;
                outcomes.push((*wallet, None));
            }
            WalletShieldedOutcome::Skipped => skipped += 1,
            WalletShieldedOutcome::Err(text) => {
                failed += 1;
                error = pass_error(text);
                outcomes.push((*wallet, Some(error)));
            }
        }
    }
    let record = crate::diag::Record::PoolPass {
        ok,
        skipped,
        failed,
        new_notes: new_notes as u32,
        scanned,
        error,
        repeated: 0,
        stale_retries: crate::diag::take_stale_retries(),
    };
    (record, outcomes)
}

/// BUG-240 r4: a pass that waited on SPV's quorums — the one that signed Platform's answer not
/// held yet («Quorum not found», «quorum key unavailable») or not verified yet («quorum source
/// unavailable … not verified by SPV yet»), the masternode list not synced, or the SPV client
/// not started yet (QA batch 45, at the start). Before, its words («verif») made it `proof`.
#[cfg(feature = "pool")]
pub(crate) const QUORUM_WAIT: &str = "quorum_wait";

/// BUG-240 r4 (R2): a node's answer older than the SDK takes («received height is outdated …
/// try another server»), after its retries elsewhere (QA batch 45).
#[cfg(feature = "pool")]
pub(crate) const STALE_NODE: &str = "stale_node";

/// BUG-240 r4 (R2): the kinds of a failed pass that pass by themselves — the connection, a node
/// behind, SPV's quorums not ready — counted apart from the others, to a higher threshold.
#[cfg(feature = "pool")]
pub(crate) fn passes_by_itself(kind: &str) -> bool {
    kind == "network" || kind == QUORUM_WAIT || kind == STALE_NODE
}

/// The kind of a failed pass, never its words (they may name an address).
#[cfg(feature = "pool")]
pub(crate) fn pass_error(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") || lower.contains("deadline") {
        "timeout"
    } else if text == crate::chain::NOT_READY || lower.contains("not ready") {
        "not_ready"
    } else if lower.contains("height is outdated")
        || lower.contains("stalenode")
        || lower.contains("try another server")
    {
        STALE_NODE
    } else if lower.contains("quorum source unavailable")
        || lower.contains("not verified by spv")
        || lower.contains("quorum not found")
        || lower.contains("quorum key unavailable")
        || lower.contains("masternode list not yet synced")
        || lower.contains("spv client not started")
    {
        QUORUM_WAIT
    } else if lower.contains("proof") || lower.contains("verif") {
        "proof"
    } else if lower.contains("connect")
        || lower.contains("transport")
        || lower.contains("unavailable")
        || lower.contains("dapi")
    {
        "network"
    } else {
        "other"
    }
}

/// The passes written: one line a change, not one a minute — and, BUG-240 r3, no pass lost:
/// the same outcome again is counted, and written with its count when the outcome changes.
#[cfg(feature = "pool")]
#[derive(Default)]
struct PassLog {
    last: Option<crate::diag::Record>,
    repeats: u32,
}

#[cfg(feature = "pool")]
impl PassLog {
    /// The lines `record` makes: none for the same outcome again; else the previous one with
    /// how often it repeated (when it did), then this one.
    fn step(&mut self, record: crate::diag::Record) -> Vec<crate::diag::Record> {
        if self.last.as_ref() == Some(&record) {
            self.repeats = self.repeats.saturating_add(1);
            return Vec::new();
        }
        let mut lines = Vec::new();
        if let Some(crate::diag::Record::PoolPass {
            ok,
            skipped,
            failed,
            new_notes,
            scanned,
            error,
            stale_retries,
            ..
        }) = self.last.take()
            && self.repeats > 0
        {
            lines.push(crate::diag::Record::PoolPass {
                ok,
                skipped,
                failed,
                new_notes,
                scanned,
                error,
                repeated: self.repeats,
                stale_retries,
            });
        }
        self.repeats = 0;
        self.last = Some(record.clone());
        lines.push(record);
        lines
    }
}

#[cfg(feature = "pool")]
static POOL_PASSES: Mutex<Option<PassLog>> = Mutex::new(None);

#[cfg(feature = "pool")]
fn note_pool_pass(record: crate::diag::Record) {
    let mut log = POOL_PASSES.lock().unwrap_or_else(|e| e.into_inner());
    for line in log.get_or_insert_with(PassLog::default).step(record) {
        crate::diag::record(line);
    }
}

#[cfg(all(test, feature = "pool"))]
mod pool_pass_tests {
    use super::*;
    use platform_wallet::manager::shielded_sync::{ShieldedSyncPassSummary, WalletShieldedOutcome};

    /// BUG-240 r2: a failed pass is a line with its kind (never its words) and a failure of its
    /// wallet; a skip is neither; the reporter counts the failures in a row and a success
    /// clears them and stamps its time.
    #[test]
    fn a_failed_pass_is_said_by_its_kind_and_counted() {
        let mut summary = ShieldedSyncPassSummary::default();
        summary.wallet_results.insert(
            [1; 32],
            WalletShieldedOutcome::Err("request timed out after 10s at yAbc…".into()),
        );
        summary
            .wallet_results
            .insert([2; 32], WalletShieldedOutcome::Skipped);
        summary.sync_unix_seconds = 1_800;
        let (record, outcomes) = pool_pass(&summary);
        assert_eq!(outcomes, vec![([1; 32], Some("timeout"))]);
        assert_eq!(
            record,
            crate::diag::Record::PoolPass {
                ok: 0,
                skipped: 1,
                failed: 1,
                new_notes: 0,
                scanned: 0,
                error: "timeout",
                repeated: 0,
                stale_retries: None,
            }
        );
        assert_eq!(pass_error("proof verification failed"), "proof");
        assert_eq!(pass_error("something else"), "other");
        let status = Arc::new(Mutex::new(SyncStatus::default()));
        let reporter = Reporter::new(status.clone(), Arc::new(|_| {}));
        use platform_wallet::PlatformEventHandler;
        reporter.on_shielded_sync_completed(&summary);
        reporter.on_shielded_sync_completed(&summary);
        let s = status.lock().unwrap().clone();
        assert_eq!(s.pool_failed.get(&[1; 32]), Some(&(2, "timeout")));
        assert!(s.pool_passed.is_empty(), "a failed pass is no time");
    }

    fn pass(failed: u32, error: &'static str) -> crate::diag::Record {
        crate::diag::Record::PoolPass {
            ok: 1 - failed,
            skipped: 0,
            failed,
            new_notes: 0,
            scanned: 0,
            error,
            repeated: 0,
            stale_retries: None,
        }
    }

    /// BUG-240 r4: SPV's quorums not ready are a wait, not a proof failure (the SDK's own
    /// words, 575d961), and they are not counted towards «Could not read the pool»; a real
    /// proof failure still is.
    #[test]
    fn a_quorum_not_ready_is_a_wait_not_counted() {
        for text in [
            "Shielded sync failed: Shielded sync failed: Context provider error: quorum source              unavailable: quorum 6:ab12 at Core chain-locked height 1300000: quorum 6:ab12 is              not verified by SPV yet (Unknown)",
            "Shielded sync failed: Proof verification error: quorum key unavailable for quorum              6:ab12: Context provider error: Quorum not found: type 6 at or before height 1300000",
            "Shielded sync failed: Configuration error: masternode list not yet synced              (quorums unavailable)",
            "Shielded sync failed: Proof verification error: quorum key unavailable for quorum              6:ab12: Generic(\"SPV error: SPV Client not started\")",
        ] {
            assert_eq!(pass_error(text), QUORUM_WAIT, "{text}");
        }
        assert_eq!(
            pass_error("Proof verification error: grovedb: invalid proof"),
            "proof"
        );
        assert_eq!(
            pass_error(
                "Proof verification error: invalid signature: signature ab could not be verified"
            ),
            "proof"
        );
        let wait = |wallet: [u8; 32], text: &str| {
            let mut summary = ShieldedSyncPassSummary::default();
            summary
                .wallet_results
                .insert(wallet, WalletShieldedOutcome::Err(text.into()));
            summary
        };
        let status = Arc::new(Mutex::new(SyncStatus::default()));
        let reporter = Reporter::new(status.clone(), Arc::new(|_| {}));
        use platform_wallet::PlatformEventHandler;
        let proof = wait([1; 32], "Proof verification error: grovedb: invalid proof");
        reporter.on_shielded_sync_completed(&proof);
        for _ in 0..5 {
            reporter.on_shielded_sync_completed(&wait(
                [1; 32],
                "quorum 6:ab is not verified by SPV yet (Skipped)",
            ));
        }
        let s = status.lock().unwrap().clone();
        assert_eq!(
            s.pool_failed.get(&[1; 32]),
            Some(&(1, "proof")),
            "the waits add nothing to the failures and keep their kind"
        );
        assert_eq!(s.pool_waited.get(&[1; 32]), Some(&(5, QUORUM_WAIT)));
        reporter.on_shielded_sync_completed(&proof);
        let s = status.lock().unwrap().clone();
        assert_eq!(s.pool_failed.get(&[1; 32]), Some(&(2, "proof")));
    }

    /// BUG-240 r4 (R2, R3), QA batch 45: a broken connection and a node behind pass by
    /// themselves — counted apart — and each such pass asks for one early pass; a success
    /// clears both counts.
    #[tokio::test]
    async fn passing_failures_are_counted_apart_and_ask_an_early_pass() {
        let broken = "Shielded sync failed: Dapi client error: transport error: grpc error: code: \
             'Unknown error', message: \"h2 protocol error: error reading a body from connection\"";
        let behind = "Shielded sync failed: received height is outdated: expected 715912, \
             received 715909, tolerance 1; try another server";
        assert_eq!(pass_error(broken), "network");
        assert_eq!(pass_error(behind), STALE_NODE);
        assert!(passes_by_itself("network") && passes_by_itself(STALE_NODE));
        assert!(!passes_by_itself("proof") && !passes_by_itself("timeout"));
        let pass = |text: Option<&str>| {
            let mut summary = ShieldedSyncPassSummary::default();
            summary.wallet_results.insert(
                [1; 32],
                match text {
                    Some(text) => WalletShieldedOutcome::Err(text.into()),
                    None => WalletShieldedOutcome::Ok(Default::default()),
                },
            );
            summary.sync_unix_seconds = 1_900;
            summary
        };
        let status = Arc::new(Mutex::new(SyncStatus::default()));
        let retry = Arc::new(tokio::sync::Notify::new());
        let reporter =
            Reporter::new(status.clone(), Arc::new(|_| {})).with_pool_retry(retry.clone());
        use platform_wallet::PlatformEventHandler;
        reporter.on_shielded_sync_completed(&pass(Some(broken)));
        reporter.on_shielded_sync_completed(&pass(Some(behind)));
        let s = status.lock().unwrap().clone();
        assert_eq!(s.pool_waited.get(&[1; 32]), Some(&(2, STALE_NODE)));
        assert!(s.pool_failed.is_empty(), "not a failure of the pool");
        tokio::time::timeout(std::time::Duration::from_secs(1), retry.notified())
            .await
            .expect("an early pass asked");
        reporter.on_shielded_sync_completed(&pass(Some("Proof verification error: grovedb: bad")));
        reporter.on_shielded_sync_completed(&pass(None));
        let s = status.lock().unwrap().clone();
        assert!(
            s.pool_waited.is_empty() && s.pool_failed.is_empty(),
            "a success clears both"
        );
        assert_eq!(s.pool_passed.get(&[1; 32]), Some(&1_900));
    }

    /// BUG-240 r3 (QA batch 43): the same outcome again is not a new line, but it is not lost
    /// either — when the outcome changes, the previous one is written with how often it
    /// repeated; once is written once.
    #[test]
    fn repeated_passes_are_counted_not_dropped() {
        let mut log = PassLog::default();
        let net = pass(1, "network");
        assert_eq!(log.step(net.clone()), vec![net.clone()]);
        assert!(log.step(net.clone()).is_empty());
        assert!(log.step(net.clone()).is_empty());
        let ok = pass(0, "");
        let mut net_again = net.clone();
        if let crate::diag::Record::PoolPass { repeated, .. } = &mut net_again {
            *repeated = 2;
        }
        assert_eq!(log.step(ok.clone()), vec![net_again, ok.clone()]);
        // Once: no count line.
        let proof = pass(1, "proof");
        assert_eq!(log.step(proof.clone()), vec![proof.clone()]);
        assert_eq!(log.step(ok.clone()), vec![ok]);
    }
}

/// Whether Platform answered for the addresses in this pass (BUG-91): a pass that failed, or
/// was skipped and so has no results, says nothing about the balance there.
fn address_pass_answered(summary: &platform_wallet::PlatformAddressSyncSummary) -> bool {
    summary
        .wallet_results
        .values()
        .any(|outcome| outcome.is_ok())
}

#[cfg(test)]
mod address_pass_tests {
    use super::*;
    use platform_wallet::PlatformAddressSyncSummary;
    use platform_wallet::manager::platform_address_sync::WalletSyncOutcome;

    fn pass(outcomes: Vec<WalletSyncOutcome>) -> PlatformAddressSyncSummary {
        let mut summary = PlatformAddressSyncSummary::default();
        for (i, outcome) in outcomes.into_iter().enumerate() {
            summary.wallet_results.insert([i as u8; 32], outcome);
        }
        summary
    }

    #[test]
    fn only_an_answer_from_platform_makes_the_address_read() {
        assert!(!address_pass_answered(&pass(vec![])), "a skipped pass");
        assert!(
            !address_pass_answered(&pass(vec![WalletSyncOutcome::Err("no".into())])),
            "a failed pass"
        );
        assert!(address_pass_answered(&pass(vec![WalletSyncOutcome::Ok(
            Default::default()
        )])));
    }

    /// A new run knows nothing yet: the chain's status starts unread.
    #[test]
    fn a_new_run_has_not_read_the_address() {
        assert!(!SyncStatus::default().platform_address_read);
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
        let shown = crate::convert::sync_info(&status, true);
        assert_eq!(shown.stage, evolite_api::SyncStage::Filters);
        assert_eq!(shown.progress, None, "no count before the start is known");
        status.apply_progress(&report(birth - 1));
        assert_eq!(status.filters_from, Some(birth - 1));
        status.apply_progress(&report(birth + 93_247));
        assert_eq!(status.filters_from, Some(birth - 1));
        let part = crate::convert::sync_info(&status, true)
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
