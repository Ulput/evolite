//! Generic download coordinator for pipelined downloads.
//!
//! Provides a single abstraction for managing concurrent downloads with:
//! - Pending queue management
//! - In-flight tracking with timestamps
//! - Timeout detection and retry logic
//! - Configurable concurrency limits

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// evolite (TECH-spv-peers): how long a request may go unanswered, from how long answers
/// actually take: `factor` × the median of the recent reply times, kept within `min..=max`;
/// `initial` until `min_samples` replies are known. A fixed 30 s held a lost header request
/// (and everything stored after it) for 30 s; most answers take a few seconds.
///
/// r1 (Must 1): `max` is not a wall. After `escalate_after` timeouts in a row with no answer
/// between them, the ceiling doubles, up to `ceiling` — a network where every peer is slower
/// than `max` (a slow link, megabytes of QRInfo) still gets its answers, as it did with the
/// fixed 30 s / 60 s. It comes down a step with each answer that came within `max`; an answer
/// slower than `max` keeps it where it is, so a network that always takes 20 s does not swing
/// between «three timeouts, one answer» for ever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdaptiveTimeout {
    pub(crate) factor: u32,
    pub(crate) min: Duration,
    pub(crate) max: Duration,
    pub(crate) initial: Duration,
    pub(crate) min_samples: usize,
    pub(crate) escalate_after: u32,
    pub(crate) ceiling: Duration,
}

impl AdaptiveTimeout {
    /// The timeout for `times` now.
    pub(crate) fn timeout(&self, times: &ReplyTimes) -> Duration {
        let max = self.current_max(times);
        match times.median(self.min_samples) {
            None => self.initial.max(max.min(self.initial.saturating_mul(1 << times.level.min(8)))),
            Some(median) => median.saturating_mul(self.factor).clamp(self.min, max),
        }
    }

    /// `max`, doubled once per escalation level, at most `ceiling`.
    fn current_max(&self, times: &ReplyTimes) -> Duration {
        self.max.saturating_mul(1 << times.level.min(8)).min(self.ceiling).max(self.max)
    }

    /// Fold in an answer that took `elapsed`.
    pub(crate) fn answered(&self, times: &mut ReplyTimes, elapsed: Duration) {
        times.record(elapsed);
        times.streak = 0;
        if elapsed <= self.max {
            times.level = times.level.saturating_sub(1);
        }
    }

    /// Fold in a request that went unanswered for `waited`.
    pub(crate) fn timed_out(&self, times: &mut ReplyTimes, waited: Duration) {
        times.record(waited);
        times.streak += 1;
        if times.streak >= self.escalate_after && self.current_max(times) < self.ceiling {
            times.level += 1;
            times.streak = 0;
        }
    }
}

/// The recent reply times of one kind of request, shared by everything that sends it (all the
/// header segments, say), so the timeout learns from every answer.
#[derive(Debug, Default)]
pub(crate) struct ReplyTimes {
    samples: VecDeque<Duration>,
    /// Timeouts in a row since the last answer or escalation.
    streak: u32,
    /// How many times the ceiling is doubled now.
    level: u32,
}

/// Reply times of one kind of request, shared.
pub(crate) type SharedReplyTimes = Arc<Mutex<ReplyTimes>>;

impl ReplyTimes {
    /// How many replies the median is taken over: the last minutes of a sync, not its whole
    /// history.
    const KEEP: usize = 64;

    /// Fold in one reply time. A request that timed out is recorded with the time it waited,
    /// so a slow network raises the timeout rather than only the answers that came in time.
    pub(crate) fn record(&mut self, elapsed: Duration) {
        if self.samples.len() == Self::KEEP {
            self.samples.pop_front();
        }
        self.samples.push_back(elapsed);
    }

    /// The median reply time, once at least `min_samples` are known.
    pub(crate) fn median(&self, min_samples: usize) -> Option<Duration> {
        if self.samples.is_empty() || self.samples.len() < min_samples {
            return None;
        }
        let mut sorted: Vec<Duration> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        Some(sorted[sorted.len() / 2])
    }

    pub(crate) fn shared() -> SharedReplyTimes {
        Arc::new(Mutex::new(Self::default()))
    }
}

/// Configuration for download coordination.
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// Maximum concurrent in-flight requests.
    max_concurrent: usize,
    /// Timeout duration for requests.
    timeout: Duration,
    /// evolite (TECH-spv-peers): the timeout from the reply times instead of `timeout`.
    adaptive: Option<(AdaptiveTimeout, SharedReplyTimes)>,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 10,
            timeout: Duration::from_secs(30),
            adaptive: None,
        }
    }
}

impl DownloadConfig {
    /// Create config with custom max concurrent.
    pub(crate) fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent = max;
        self
    }

    /// Create config with custom timeout.
    pub(crate) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// evolite (TECH-spv-peers): time requests by `rule` over `times`, shared with the other
    /// senders of the same kind; every answer and every timeout is recorded there.
    pub(crate) fn with_adaptive_timeout(
        mut self,
        rule: AdaptiveTimeout,
        times: SharedReplyTimes,
    ) -> Self {
        self.adaptive = Some((rule, times));
        self
    }
}

/// Generic download coordinator.
///
/// Handles the common mechanics of pipelined downloads:
/// - Queue management (pending items)
/// - In-flight tracking with timestamps
/// - Timeout detection and retry
/// - Concurrency limits
///
/// Generic over the key type `K` which identifies download items.
/// Use `u32` for height-based downloads, `BlockHash` for hash-based.
#[derive(Debug)]
pub(crate) struct DownloadCoordinator<K: Hash + Eq + Clone> {
    /// Items waiting to be requested.
    pending: VecDeque<K>,
    /// Items currently in-flight (key -> sent time).
    in_flight: HashMap<K, Instant>,
    /// Retry counts per key.
    retry_counts: HashMap<K, u32>,
    /// Configuration.
    config: DownloadConfig,
    /// Last time progress was made.
    last_progress: Instant,
}

impl<K: Hash + Eq + Clone> Default for DownloadCoordinator<K> {
    fn default() -> Self {
        Self::new(DownloadConfig::default())
    }
}

impl<K: Hash + Eq + Clone> DownloadCoordinator<K> {
    /// Create a new coordinator with the given configuration.
    pub(crate) fn new(config: DownloadConfig) -> Self {
        Self {
            pending: VecDeque::new(),
            in_flight: HashMap::new(),
            retry_counts: HashMap::new(),
            config,
            last_progress: Instant::now(),
        }
    }

    /// Clear all state.
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.in_flight.clear();
        self.retry_counts.clear();
        self.last_progress = Instant::now();
    }

    /// Move all in-flight items back to the front of the pending queue.
    ///
    /// Used on peer disconnect: the requests went to a now-dead peer, but the
    /// items themselves are still wanted. Retry counts are preserved so a peer
    /// that consistently fails to deliver an item still trips the normal retry
    /// budget. Without this hook, items would only be retried once their
    /// timeout elapsed.
    pub(crate) fn requeue_in_flight(&mut self) {
        let items: Vec<K> = self.in_flight.drain().map(|(k, _)| k).collect();
        if items.is_empty() {
            return;
        }
        for item in items.into_iter().rev() {
            self.pending.push_front(item);
        }
    }

    /// Queue items for download.
    pub(crate) fn enqueue(&mut self, items: impl IntoIterator<Item = K>) {
        for item in items {
            self.pending.push_back(item);
        }
    }

    /// Queue an item for retry (goes to front of queue).
    pub(crate) fn enqueue_retry(&mut self, item: K) {
        let count = self.retry_counts.entry(item.clone()).or_insert(0);
        *count += 1;
        tracing::warn!("Retrying item (attempt {})", count);
        self.pending.push_front(item);
    }

    /// Get the number of items available to send (respecting concurrency limit).
    pub(crate) fn available_to_send(&self) -> usize {
        self.config.max_concurrent.saturating_sub(self.in_flight.len()).min(self.pending.len())
    }

    /// Take items from the pending queue (up to count).
    ///
    /// Items are removed from pending but NOT yet marked as in-flight.
    /// Call `mark_sent` after successfully sending the request.
    pub(crate) fn take_pending(&mut self, count: usize) -> Vec<K> {
        let actual = count.min(self.pending.len());
        let mut items = Vec::with_capacity(actual);
        for _ in 0..actual {
            if let Some(item) = self.pending.pop_front() {
                items.push(item);
            }
        }
        items
    }

    /// Mark items as sent (now in-flight).
    pub(crate) fn mark_sent(&mut self, items: &[K]) {
        let now = Instant::now();
        for item in items {
            self.in_flight.insert(item.clone(), now);
        }
    }

    /// The timeout requests have now: the adaptive one when configured.
    pub(crate) fn timeout(&self) -> Duration {
        match &self.config.adaptive {
            Some((rule, times)) => rule.timeout(&times.lock().unwrap_or_else(|e| e.into_inner())),
            None => self.config.timeout,
        }
    }

    /// evolite (TECH-spv-peers): an answer for the adaptive timeout, if there is one.
    fn record_reply(&self, elapsed: Duration) {
        if let Some((rule, times)) = &self.config.adaptive {
            rule.answered(&mut times.lock().unwrap_or_else(|e| e.into_inner()), elapsed);
        }
    }

    /// evolite (TECH-spv-peers r1): a timeout for the adaptive timeout, if there is one.
    fn record_timeout(&self, waited: Duration) {
        if let Some((rule, times)) = &self.config.adaptive {
            rule.timed_out(&mut times.lock().unwrap_or_else(|e| e.into_inner()), waited);
        }
    }

    /// Handle a received item.
    ///
    /// Returns true if the item was being tracked, false if unexpected.
    pub(crate) fn receive(&mut self, key: &K) -> bool {
        if let Some(sent) = self.in_flight.get(key) {
            self.record_reply(sent.elapsed());
        }
        if self.in_flight.remove(key).is_some() {
            self.retry_counts.remove(key);
            self.last_progress = Instant::now();
            true
        } else {
            false
        }
    }

    /// Drop a key from the pending queue without touching in-flight state.
    ///
    /// Used when a pending item is satisfied through a side channel: a late
    /// response from a disconnected peer can complete a batch that
    /// `requeue_in_flight` just moved from in-flight back to pending. Without
    /// this hook, the key would stay in `pending` with no tracker, and the
    /// next `take_pending` would resurrect a finished batch.
    pub(crate) fn cancel_pending(&mut self, key: &K) {
        self.pending.retain(|k| k != key);
        self.retry_counts.remove(key);
    }

    /// Check if an item is currently in-flight.
    pub(crate) fn is_in_flight(&self, key: &K) -> bool {
        self.in_flight.contains_key(key)
    }

    /// Check for timed-out items.
    ///
    /// Returns items that have timed out. They are removed from in-flight tracking.
    /// Caller should call `enqueue_retry` for items that should be retried.
    pub(crate) fn check_timeouts(&mut self) -> Vec<K> {
        let now = Instant::now();
        let timeout = self.timeout();
        let timed_out: Vec<K> = self
            .in_flight
            .iter()
            .filter(|(_, sent_time)| now.duration_since(**sent_time) > timeout)
            .map(|(key, _)| key.clone())
            .collect();

        for key in &timed_out {
            if let Some(sent) = self.in_flight.remove(key) {
                self.record_timeout(now.duration_since(sent));
            }
        }

        if !timed_out.is_empty() {
            tracing::debug!("{} items timed out after {:?}", timed_out.len(), timeout);
        }

        timed_out
    }

    /// Check for timed-out items and re-enqueue them for retry.
    ///
    /// Combines `check_timeouts()` and `enqueue_retry()` in one call.
    /// Returns all timed-out items that were re-queued.
    pub(crate) fn check_and_retry_timeouts(&mut self) -> Vec<K> {
        let timed_out = self.check_timeouts();
        for item in &timed_out {
            self.enqueue_retry(item.clone());
        }
        timed_out
    }

    /// Check if the coordinator has no work (empty pending and in-flight).
    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.in_flight.is_empty()
    }

    /// Get the number of pending items.
    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Get the number of in-flight items.
    pub(crate) fn active_count(&self) -> usize {
        self.in_flight.len()
    }

    /// Get the total remaining items (pending + in-flight).
    pub(crate) fn remaining(&self) -> usize {
        self.pending.len() + self.in_flight.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_coordinator() {
        let coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        assert!(coord.is_empty());
        assert_eq!(coord.pending_count(), 0);
        assert_eq!(coord.active_count(), 0);
    }

    #[test]
    fn test_enqueue() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue([1, 2, 3, 4, 5]);

        assert_eq!(coord.pending_count(), 5);
    }

    #[test]
    fn test_enqueue_retry_goes_to_front() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue([1, 2]);
        coord.enqueue_retry(99);

        let items = coord.take_pending(3);
        assert_eq!(items, vec![99, 1, 2]);
    }

    #[test]
    fn test_take_pending() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue([1, 2, 3, 4, 5]);

        let items = coord.take_pending(3);
        assert_eq!(items, vec![1, 2, 3]);
        assert_eq!(coord.pending_count(), 2);
    }

    /// evolite (TECH-spv-peers): 3× the median reply, within the bounds; the initial timeout
    /// until enough replies are known; a request that timed out counts with its wait, so a slow
    /// network raises the timeout.
    #[test]
    fn the_adaptive_timeout_follows_the_replies() {
        let rule = AdaptiveTimeout {
            factor: 3,
            min: Duration::from_secs(6),
            max: Duration::from_secs(15),
            initial: Duration::from_secs(15),
            min_samples: 5,
            escalate_after: 3,
            ceiling: Duration::from_secs(15),
        };
        let mut times = ReplyTimes::default();
        assert_eq!(rule.timeout(&times), Duration::from_secs(15));
        for s in [1, 1, 2, 2] {
            times.record(Duration::from_secs(s));
        }
        assert_eq!(rule.timeout(&times), Duration::from_secs(15), "four are too few");
        times.record(Duration::from_secs(1));
        assert_eq!(rule.timeout(&times), Duration::from_secs(6), "3 × 1 s, at least 6");
        for _ in 0..10 {
            times.record(Duration::from_secs(4));
        }
        assert_eq!(rule.timeout(&times), Duration::from_secs(12));
        for _ in 0..70 {
            times.record(Duration::from_secs(30));
        }
        assert_eq!(rule.timeout(&times), Duration::from_secs(15), "at most 15");
        assert_eq!(times.samples.len(), ReplyTimes::KEEP, "only the recent ones");
    }

    /// evolite (TECH-spv-peers r1, Must 1): when every peer answers slower than `max`, three
    /// timeouts in a row raise the ceiling until the answers come — and it stays raised
    /// while they keep coming slower than `max`. With peers that all answer in 20 s the
    /// header timeout settles above 20 s and every request after the first few is answered;
    /// with fast answers again it comes back down.
    #[test]
    fn a_network_slower_than_the_ceiling_still_gets_its_answers() {
        let rule = crate::sync::block_headers::HEADERS_TIMEOUT_FOR_TESTS;
        let mut times = ReplyTimes::default();
        let mut answered = Vec::new();
        for _ in 0..40 {
            let timeout = rule.timeout(&times);
            let reply = Duration::from_secs(20);
            if reply <= timeout {
                rule.answered(&mut times, reply);
                answered.push(true);
            } else {
                rule.timed_out(&mut times, timeout);
                answered.push(false);
            }
        }
        assert!(answered[..6].contains(&true), "answers start within a few requests");
        assert!(answered[10..].iter().all(|a| *a), "then every request is answered");
        assert!(rule.timeout(&times) > Duration::from_secs(20));
        assert!(rule.timeout(&times) <= Duration::from_secs(60), "at most the ceiling");

        // The network is fast again: the ceiling comes back down step by step.
        for _ in 0..80 {
            rule.answered(&mut times, Duration::from_secs(1));
        }
        assert_eq!(rule.timeout(&times), Duration::from_secs(6));
    }

    /// evolite (TECH-spv-peers): an answer and a timeout are both recorded where the
    /// coordinator shares its reply times.
    #[test]
    fn answers_and_timeouts_feed_the_shared_times() {
        let times = ReplyTimes::shared();
        let rule = AdaptiveTimeout {
            factor: 3,
            min: Duration::from_millis(1),
            max: Duration::from_millis(5),
            initial: Duration::from_millis(5),
            min_samples: 1,
            escalate_after: 3,
            ceiling: Duration::from_millis(5),
        };
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::new(
            DownloadConfig::default().with_adaptive_timeout(rule, times.clone()),
        );
        coord.mark_sent(&[1, 2]);
        assert!(coord.receive(&1));
        assert_eq!(times.lock().unwrap().samples.len(), 1);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(coord.check_timeouts(), vec![2]);
        assert_eq!(times.lock().unwrap().samples.len(), 2, "the timeout counts too");
    }

    #[test]
    fn test_mark_sent() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue([1, 2, 3]);

        let items = coord.take_pending(2);
        coord.mark_sent(&items);

        assert_eq!(coord.pending_count(), 1);
        assert_eq!(coord.active_count(), 2);
        assert!(coord.is_in_flight(&1));
        assert!(coord.is_in_flight(&2));
        assert!(!coord.is_in_flight(&3));
    }

    #[test]
    fn test_receive() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.mark_sent(&[1]);
        coord.mark_sent(&[2]);

        assert!(coord.receive(&1));
        assert_eq!(coord.active_count(), 1);

        assert!(!coord.receive(&99)); // Not tracked
        assert_eq!(coord.active_count(), 1);
    }

    #[test]
    fn test_available_to_send() {
        let mut coord: DownloadCoordinator<u32> =
            DownloadCoordinator::new(DownloadConfig::default().with_max_concurrent(3));

        coord.enqueue([1, 2, 3, 4, 5]);
        assert_eq!(coord.available_to_send(), 3);

        coord.mark_sent(&[1]);
        coord.mark_sent(&[2]);
        assert_eq!(coord.available_to_send(), 1);

        coord.mark_sent(&[3]);
        assert_eq!(coord.available_to_send(), 0);
    }

    #[test]
    fn test_check_timeouts() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::new(
            DownloadConfig::default().with_timeout(Duration::from_millis(10)),
        );

        coord.mark_sent(&[1]);
        coord.mark_sent(&[2]);

        // Immediately, nothing timed out
        let timed_out = coord.check_timeouts();
        assert!(timed_out.is_empty());

        // Wait for timeout
        std::thread::sleep(Duration::from_millis(20));

        let timed_out = coord.check_timeouts();
        assert_eq!(timed_out.len(), 2);
        assert!(coord.in_flight.is_empty());
    }

    #[test]
    fn test_requeue_in_flight_moves_items_to_pending_front() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue([10, 11]);
        coord.mark_sent(&[1, 2, 3]);

        coord.requeue_in_flight();

        assert_eq!(coord.active_count(), 0);
        // Requeued items go to the front, original pending follows.
        let items = coord.take_pending(5);
        assert_eq!(items.len(), 5);
        assert_eq!(&items[3..], &[10, 11]);
        let mut requeued = items[..3].to_vec();
        requeued.sort();
        assert_eq!(requeued, vec![1, 2, 3]);
    }

    #[test]
    fn test_requeue_in_flight_preserves_retry_counts() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue_retry(7);
        let items = coord.take_pending(1);
        coord.mark_sent(&items);
        assert_eq!(coord.retry_counts.get(&7), Some(&1));

        coord.requeue_in_flight();

        assert_eq!(coord.retry_counts.get(&7), Some(&1));
        assert!(!coord.is_in_flight(&7));
        assert_eq!(coord.pending_count(), 1);

        // A peer disconnect is not an attempt against the item's budget, so
        // repeated requeues must not spend it.
        let items = coord.take_pending(1);
        coord.mark_sent(&items);
        coord.requeue_in_flight();
        assert_eq!(coord.retry_counts.get(&7), Some(&1));

        // A genuine timeout still counts, and delivery still clears the budget.
        let items = coord.take_pending(1);
        coord.mark_sent(&items);
        coord.enqueue_retry(7);
        assert_eq!(coord.retry_counts.get(&7), Some(&2));
        assert!(coord.receive(&7));
        assert_eq!(coord.retry_counts.get(&7), None);
    }

    #[test]
    fn test_requeue_in_flight_no_op_when_empty() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue([1, 2]);

        coord.requeue_in_flight();

        assert_eq!(coord.pending_count(), 2);
        assert_eq!(coord.active_count(), 0);
    }

    #[test]
    fn test_cancel_pending_removes_from_pending_only() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue([1, 2, 3]);
        coord.mark_sent(&[10]);
        coord.enqueue_retry(2);
        assert_eq!(coord.retry_counts.get(&2), Some(&1));

        coord.cancel_pending(&2);

        assert_eq!(coord.pending_count(), 2);
        assert!(coord.is_in_flight(&10));
        assert_eq!(coord.retry_counts.get(&2), None);

        let items = coord.take_pending(2);
        assert_eq!(items, vec![1, 3]);
    }

    #[test]
    fn test_cancel_pending_unknown_key_is_noop() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue([1, 2]);
        coord.mark_sent(&[5]);

        coord.cancel_pending(&99);

        assert_eq!(coord.pending_count(), 2);
        assert_eq!(coord.active_count(), 1);
    }

    #[test]
    fn test_clear() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue([1, 2, 3]);
        coord.mark_sent(&[4]);
        coord.enqueue_retry(5);

        coord.clear();

        assert!(coord.is_empty());
        assert_eq!(coord.pending_count(), 0);
        assert_eq!(coord.active_count(), 0);
    }

    #[test]
    fn test_remaining() {
        let mut coord: DownloadCoordinator<u32> = DownloadCoordinator::default();
        coord.enqueue([1, 2, 3]);
        coord.mark_sent(&[4]);
        coord.mark_sent(&[5]);

        assert_eq!(coord.remaining(), 5);
    }

    #[test]
    fn test_config_builders() {
        let config =
            DownloadConfig::default().with_max_concurrent(20).with_timeout(Duration::from_secs(60));

        assert_eq!(config.max_concurrent, 20);
        assert_eq!(config.timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_with_string_keys() {
        let mut coord: DownloadCoordinator<String> = DownloadCoordinator::default();
        coord.enqueue(["block_a".to_string(), "block_b".to_string()]);

        let items = coord.take_pending(1);
        coord.mark_sent(&items);

        assert!(coord.receive(&"block_a".to_string()));
        assert!(!coord.receive(&"block_c".to_string()));
    }
}
