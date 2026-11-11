//! What the real core does, by subject: each file holds the `impl Real` block of one group of
//! commands, or the small state those commands keep.

pub(super) mod addresses;
pub(super) mod app_jobs;
pub(super) mod avatars;
pub(super) mod core;
pub(super) mod dashpay;
pub(super) mod identity;
mod misc;
mod names;
pub(super) mod notes;
pub(super) mod outgoing;
pub(super) mod platform_sending;
pub(super) mod pool;
pub(super) mod wallet;
