mod manager;
mod pipeline;
mod progress;
mod segment_state;

#[cfg(test)]
pub(crate) const HEADERS_TIMEOUT_FOR_TESTS: crate::sync::download_coordinator::AdaptiveTimeout =
    segment_state::HEADERS_TIMEOUT;
mod sync_manager;

pub use manager::BlockHeadersManager;
pub(crate) use pipeline::HeadersPipeline;
pub use progress::BlockHeadersProgress;
