//! What the real core does, by subject: each file holds the `impl Real` block of one group of
//! commands, or the small state those commands keep.

pub(super) mod address_fund;
pub(super) mod addresses;
pub(super) mod app_connect2;
pub(super) mod app_jobs;
pub(super) mod avatars;
pub(super) mod core;
pub(super) mod dashpay;
pub(super) mod identity;
pub(super) mod identity_keys;
pub(super) mod kept_locks;
pub(super) mod keyed;
mod local_files;
pub(super) mod lock_coins;
mod misc;
pub(super) mod mixed;
mod name_contests;
pub(super) mod name_search;
mod names;
pub(super) mod notes;
pub(super) mod outgoing;
pub(super) mod platform_sending;
pub(super) mod pool;
pub(in crate::real) mod resend;
pub(super) mod token_actions;
pub(super) mod token_history;
pub(super) mod token_manage;
pub(super) mod token_search;
pub(super) mod tokens;
pub(super) mod wallet;
