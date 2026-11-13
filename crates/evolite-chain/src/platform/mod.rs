//! What is read from Platform and sent to it: identities, DashPay, NFTs, its history and the
//! shielded pool.

pub mod dashpay;
pub mod identity;
pub mod lock_status;
pub mod nfts;
pub mod platform_history;
pub mod pool;
// The real core asks the network with these from FEAT-0019 part 3; this goes then.
#[allow(dead_code)]
pub mod tokens;
