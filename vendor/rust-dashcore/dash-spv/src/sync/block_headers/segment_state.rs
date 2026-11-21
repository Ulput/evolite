use crate::error::{SyncError, SyncResult};
use crate::network::RequestSender;
use crate::sync::download_coordinator::{
    AdaptiveTimeout, DownloadConfig, DownloadCoordinator, ReplyTimes, SharedReplyTimes,
};
use crate::types::HashedBlockHeader;
use dashcore::BlockHash;
use std::time::Duration;

/// evolite (TECH-spv-peers): a header request is asked again after 3× the median reply of all
/// the segments, within 6..=15 s (15 s until five replies are known), not a fixed 30 s; three
/// timeouts in a row double the 15 s, up to 60 s (r1: a network slower than 15 s everywhere). On
/// mainnet a reply of 8 000 headers took 5.8 s at the median, and a lost one held the in-order
/// write for 30 s each time. The floor stays above the network's `REQUEST_OWED_TIMEOUT`, so
/// the retry goes to another peer than the one that owes it.
pub(crate) const HEADERS_TIMEOUT: AdaptiveTimeout = AdaptiveTimeout {
    factor: 3,
    min: Duration::from_secs(6),
    max: Duration::from_secs(15),
    initial: Duration::from_secs(15),
    min_samples: 5,
    escalate_after: 3,
    ceiling: Duration::from_secs(60),
};

fn headers_coordinator(times: SharedReplyTimes) -> DownloadCoordinator<BlockHash> {
    DownloadCoordinator::new(
        DownloadConfig::default()
            .with_max_concurrent(1) // Only 1 request at a time (sequential getheaders)
            .with_adaptive_timeout(HEADERS_TIMEOUT, times),
    )
}

/// State for a single download segment between two checkpoints.
#[derive(Debug)]
pub(super) struct SegmentState {
    /// Unique segment identifier (index in segments array).
    pub(super) segment_id: usize,
    /// Starting height of this segment.
    pub(super) start_height: u32,
    /// Target height (None for tip segment).
    pub(super) target_height: Option<u32>,
    /// Target hash (next checkpoint hash for validation).
    target_hash: Option<BlockHash>,
    /// Current tip hash for GetHeaders locator.
    pub(super) current_tip_hash: BlockHash,
    /// Current height reached in this segment.
    pub(super) current_height: u32,
    /// Download coordinator for tracking in-flight requests.
    pub(super) coordinator: DownloadCoordinator<BlockHash>,
    /// Buffered headers waiting to be stored.
    pub(super) buffered_headers: Vec<HashedBlockHeader>,
    /// Whether this segment has completed downloading.
    pub(super) complete: bool,
    /// evolite (SPV item 1б): stored hashes below `current_tip_hash` the next getheaders
    /// carries after it, once (see `HeadersPipeline::reset_tip_segment_with_locator`).
    pub(super) locator_tail: Vec<BlockHash>,
}

impl SegmentState {
    /// Create a new segment state.
    pub(super) fn new(
        segment_id: usize,
        start_height: u32,
        start_hash: BlockHash,
        target_height: Option<u32>,
        target_hash: Option<BlockHash>,
    ) -> Self {
        Self {
            segment_id,
            start_height,
            target_height,
            target_hash,
            current_tip_hash: start_hash,
            current_height: start_height,
            coordinator: headers_coordinator(ReplyTimes::shared()),
            buffered_headers: Vec::new(),
            complete: false,
            locator_tail: Vec::new(),
        }
    }

    /// evolite (TECH-spv-peers): time this segment's requests with `times`, shared with the
    /// other segments. For a segment that has sent nothing yet.
    pub(super) fn with_reply_times(mut self, times: SharedReplyTimes) -> Self {
        debug_assert_eq!(self.coordinator.active_count(), 0);
        self.coordinator = headers_coordinator(times);
        self
    }

    /// Check if the segment can send more requests.
    /// Only one getheaders request can be in-flight at a time (sequential protocol).
    pub(super) fn can_send(&self) -> bool {
        !self.complete && !self.coordinator.is_in_flight(&self.current_tip_hash)
    }

    /// Send a GetHeaders request for this segment.
    pub(super) fn send_request(&mut self, requests: &RequestSender) -> SyncResult<()> {
        if self.locator_tail.is_empty() {
            requests.request_block_headers(self.current_tip_hash)?;
        } else {
            // evolite (SPV item 1б): the tip, then hashes back, as Dash Core's locator.
            let mut locator = vec![self.current_tip_hash];
            locator.append(&mut self.locator_tail);
            requests.request_block_headers_with_locator(locator, None)?;
        }
        self.coordinator.mark_sent(&[self.current_tip_hash]);
        tracing::debug!(
            "Segment {}: sent GetHeaders from height {} hash {}",
            self.segment_id,
            self.current_height,
            self.current_tip_hash
        );
        Ok(())
    }

    /// Try to match incoming headers to this segment.
    /// Returns true if the headers belong to this segment.
    pub(super) fn matches(&self, prev_blockhash: &BlockHash) -> bool {
        // Match if prev_blockhash equals our current tip hash
        &self.current_tip_hash == prev_blockhash
    }

    /// Process received headers for this segment.
    /// Returns the number of headers processed, or an error if checkpoint validation fails.
    pub(super) fn receive_headers(&mut self, headers: &[HashedBlockHeader]) -> SyncResult<usize> {
        if headers.is_empty() {
            // Empty response means we've reached the peer's tip for this segment
            self.complete = true;
            // Clear in-flight tracking for the current tip hash
            self.coordinator.receive(&self.current_tip_hash);
            tracing::info!(
                "Segment {}: complete (empty response at height {})",
                self.segment_id,
                self.current_height
            );
            return Ok(0);
        }

        // Reject headers on a segment that already reached its checkpoint
        if self.complete {
            return Err(SyncError::InvalidState(format!(
                "Segment {}: received {} headers on completed segment (height {})",
                self.segment_id,
                headers.len(),
                self.current_height
            )));
        }

        // Mark the request as received, reject if we never requested this hash
        let prev_hash = headers[0].header().prev_blockhash;
        if !self.coordinator.receive(&prev_hash) {
            return Err(SyncError::InvalidState(format!(
                "Segment {}: received unrequested headers (prev_hash {})",
                self.segment_id, prev_hash
            )));
        }

        // Process headers
        let mut processed = 0;
        for hashed in headers {
            let hash = *hashed.hash();
            let height = self.current_height + processed as u32 + 1;

            // Check if we've reached the target (next checkpoint)
            if let (Some(target_height), Some(target_hash)) = (self.target_height, self.target_hash)
            {
                if height == target_height {
                    if hash == target_hash {
                        tracing::info!(
                            "Segment {}: reached target checkpoint at height {}",
                            self.segment_id,
                            target_height
                        );
                        self.buffered_headers.push(hashed.clone());
                        processed += 1;
                        self.complete = true;
                        break;
                    } else {
                        tracing::error!(
                            "Segment {}: checkpoint mismatch at height {}! expected {}, got {}",
                            self.segment_id,
                            target_height,
                            target_hash,
                            hash
                        );
                        return Err(SyncError::Validation(format!(
                            "Block at height {} does not match checkpoint: expected {}, got {}",
                            target_height, target_hash, hash
                        )));
                    }
                }
            }

            self.buffered_headers.push(hashed.clone());
            processed += 1;
        }

        // Update current tip for next request
        if processed > 0 {
            self.current_tip_hash = *headers[processed - 1].hash();
            self.current_height += processed as u32;
        }

        tracing::debug!(
            "Segment {}: received {} headers, now at height {}, buffered {}",
            self.segment_id,
            processed,
            self.current_height,
            self.buffered_headers.len()
        );

        Ok(processed)
    }

    /// Take buffered headers from this segment.
    pub(super) fn take_buffered(&mut self) -> Vec<HashedBlockHeader> {
        std::mem::take(&mut self.buffered_headers)
    }

    /// Check for timed out requests and handle retries.
    pub(super) fn handle_timeouts(&mut self) {
        let timed_out = self.coordinator.check_timeouts();
        for hash in timed_out {
            tracing::warn!(
                "Segment {}: request timed out for hash {}, will retry",
                self.segment_id,
                hash
            );
            // Re-enqueue for retry
            self.coordinator.enqueue_retry(hash);
        }
    }

    /// Drop only per-peer in-flight bookkeeping.
    ///
    /// Buffered headers and the validated `current_tip_hash` / `current_height`
    /// are preserved so a reconnect can resume from where the last peer left off
    /// without re-fetching headers we already have.
    pub(super) fn clear_in_flight(&mut self) {
        self.coordinator.clear();
    }
}

#[cfg(test)]
mod tests {
    use crate::error::SyncError;
    use crate::sync::block_headers::segment_state::SegmentState;

    /// evolite (TECH-spv-peers): a header request is asked again sooner than 30 s, and never
    /// before the network has stopped routing to the peer that owes it — the retry goes to
    /// another peer.
    #[test]
    fn a_header_retry_waits_less_than_before_but_past_the_owing_peer() {
        use super::HEADERS_TIMEOUT;
        use crate::network::manager::REQUEST_OWED_TIMEOUT;
        assert!(HEADERS_TIMEOUT.min > REQUEST_OWED_TIMEOUT);
        assert!(HEADERS_TIMEOUT.max < std::time::Duration::from_secs(30));
        assert!(HEADERS_TIMEOUT.initial <= HEADERS_TIMEOUT.max);
    }

    /// evolite (TECH-spv-peers): the segments of one pipeline time their requests by the
    /// answers all of them got.
    #[test]
    fn segments_share_their_reply_times() {
        use crate::sync::download_coordinator::ReplyTimes;
        use dashcore::BlockHash;
        use std::time::Duration;
        let times = ReplyTimes::shared();
        let a = SegmentState::new(0, 0, BlockHash::dummy(1), Some(10), None)
            .with_reply_times(times.clone());
        let b = SegmentState::new(1, 10, BlockHash::dummy(2), Some(20), None)
            .with_reply_times(times.clone());
        assert_eq!(a.coordinator.timeout(), Duration::from_secs(15), "none known yet");
        for _ in 0..5 {
            times.lock().unwrap().record(Duration::from_secs(3));
        }
        assert_eq!(a.coordinator.timeout(), Duration::from_secs(9));
        assert_eq!(b.coordinator.timeout(), Duration::from_secs(9), "the same for both");
    }
    use crate::types::HashedBlockHeader;
    use dashcore::{BlockHash, Header};

    #[test]
    fn test_segment_state_new() {
        let hash = BlockHash::dummy(0);
        let segment = SegmentState::new(0, 0, hash, Some(500_000), None);

        assert_eq!(segment.segment_id, 0);
        assert_eq!(segment.start_height, 0);
        assert_eq!(segment.current_height, 0);
        assert!(!segment.complete);
        assert!(segment.buffered_headers.is_empty());
    }

    #[test]
    fn test_segment_can_send() {
        let hash = BlockHash::dummy(0);
        let segment = SegmentState::new(0, 0, hash, Some(1000), None);

        assert!(segment.can_send());
    }

    #[test]
    fn test_segment_matches() {
        let hash = BlockHash::dummy(0);
        let segment = SegmentState::new(0, 0, hash, Some(1000), None);

        assert!(segment.matches(&hash));
        assert!(!segment.matches(&BlockHash::dummy(1)));
    }

    #[test]
    fn test_segment_receive_empty() {
        let hash = BlockHash::dummy(1);
        let mut segment = SegmentState::new(0, 0, hash, Some(1000), None);

        let processed = segment.receive_headers(&[]).unwrap();

        assert_eq!(processed, 0);
        assert!(segment.complete);
    }

    #[test]
    fn test_segment_receive_headers() {
        let hash = BlockHash::dummy(1);
        let mut segment = SegmentState::new(0, 0, hash, None, None);
        segment.coordinator.mark_sent(&[hash]);

        // Create dummy headers that chain from all-zeros
        let headers: Vec<Header> = (1..=10).map(Header::dummy).collect();

        // Manually fix the prev_blockhash of first header
        let mut first = headers[0];
        first.prev_blockhash = hash;

        let processed = segment.receive_headers(&[first.into()]).unwrap();

        assert_eq!(processed, 1);
        assert_eq!(segment.buffered_headers.len(), 1);
        assert_eq!(segment.current_height, 1);
        assert!(!segment.complete);
    }

    #[test]
    fn test_segment_checkpoint_mismatch_returns_error() {
        let start_hash = BlockHash::dummy(0);
        // Segment with checkpoint at height 1 expecting a specific hash
        let expected_checkpoint_hash = BlockHash::dummy(99);
        let mut segment =
            SegmentState::new(0, 0, start_hash, Some(1), Some(expected_checkpoint_hash));
        segment.coordinator.mark_sent(&[start_hash]);

        // Create a header that will be at height 1 but with a different hash
        let mut header = Header::dummy(1);
        header.prev_blockhash = start_hash;

        // The header's hash won't match the expected checkpoint hash
        let hashed = HashedBlockHeader::from(header);
        let actual_hash = hashed.hash();
        assert_ne!(*actual_hash, expected_checkpoint_hash);

        // Receiving this header should fail with a validation error
        let result = segment.receive_headers(&[header.into()]);
        assert!(result.is_err());

        let err = result.unwrap_err();
        match err {
            SyncError::Validation(msg) => {
                assert!(msg.contains("does not match checkpoint"));
                assert!(msg.contains("height 1"));
            }
            _ => panic!("Expected SyncError::Validation, got {:?}", err),
        }

        // Segment should not be complete and no headers should be buffered
        assert!(!segment.complete);
        assert!(segment.buffered_headers.is_empty());
    }

    #[test]
    fn test_segment_checkpoint_match_completes_segment() {
        let start_hash = BlockHash::dummy(0);
        // Create a header first to get its hash for the checkpoint
        let mut header = Header::dummy(1);
        header.prev_blockhash = start_hash;
        let hashed = HashedBlockHeader::from(header);
        let header_hash = *hashed.hash();

        // Create segment with checkpoint matching the header's hash
        let mut segment = SegmentState::new(0, 0, start_hash, Some(1), Some(header_hash));
        segment.coordinator.mark_sent(&[start_hash]);

        // Receiving this header should succeed and complete the segment
        let result = segment.receive_headers(&[header.into()]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);

        // Segment should be complete with the header buffered
        assert!(segment.complete);
        assert_eq!(segment.buffered_headers.len(), 1);
    }

    #[test]
    fn test_unrequested_headers_returns_error() {
        let start_hash = BlockHash::dummy(0);
        let mut segment = SegmentState::new(0, 0, start_hash, None, None);

        let mut header = Header::dummy(1);
        header.prev_blockhash = start_hash;

        let result = segment.receive_headers(&[header.into()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            SyncError::InvalidState(msg) => {
                assert!(msg.contains("unrequested headers"));
            }
            other => panic!("Expected SyncError::InvalidState, got {:?}", other),
        }
        assert!(segment.buffered_headers.is_empty());
    }

    #[test]
    fn test_completed_segment_rejects_new_headers() {
        let start_hash = BlockHash::dummy(0);
        let mut segment = SegmentState::new(0, 0, start_hash, Some(100), None);

        // Mark segment as complete (simulating checkpoint reached)
        segment.complete = true;
        segment.current_height = 100;

        // Create a header that would match
        let mut header = Header::dummy(1);
        header.prev_blockhash = start_hash;

        // Completed segment should return an invalid state error
        let result = segment.receive_headers(&[header.into()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            SyncError::InvalidState(msg) => {
                assert!(msg.contains("completed segment"));
            }
            other => panic!("Expected SyncError::InvalidState, got {:?}", other),
        }
        assert!(segment.buffered_headers.is_empty());
    }

    #[test]
    fn test_clear_in_flight_preserves_chain_state() {
        let start_hash = BlockHash::dummy(0);
        let mut segment = SegmentState::new(0, 0, start_hash, None, None);
        segment.coordinator.mark_sent(&[start_hash]);

        let mut header = Header::dummy(1);
        header.prev_blockhash = start_hash;
        segment.receive_headers(&[header.into()]).unwrap();

        let preserved_tip_hash = segment.current_tip_hash;
        let preserved_height = segment.current_height;
        let preserved_buffered = segment.buffered_headers.len();
        assert_ne!(preserved_tip_hash, start_hash);
        assert_eq!(preserved_height, 1);
        assert_eq!(preserved_buffered, 1);

        // Simulate a fresh in-flight request, then clear it.
        segment.coordinator.mark_sent(&[preserved_tip_hash]);
        assert!(segment.coordinator.is_in_flight(&preserved_tip_hash));

        segment.clear_in_flight();

        assert!(!segment.coordinator.is_in_flight(&preserved_tip_hash));
        assert_eq!(segment.coordinator.active_count(), 0);
        assert_eq!(segment.coordinator.pending_count(), 0);

        assert_eq!(segment.current_tip_hash, preserved_tip_hash);
        assert_eq!(segment.current_height, preserved_height);
        assert_eq!(segment.buffered_headers.len(), preserved_buffered);
        assert!(!segment.complete);

        // After clearing, can_send should be true again so a fresh GetHeaders
        // can resume from the preserved tip hash without re-fetching what we have.
        assert!(segment.can_send());
    }
}
