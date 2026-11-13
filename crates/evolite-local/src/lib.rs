//! The files of a wallet's folder: the vault, the list of wallets, what the wallet did, the
//! person's notes and the small JSON files beside the store. Nothing here talks to the chain.

#![forbid(unsafe_code)]

#[macro_use]
extern crate evolite_common;

mod contacts;
mod history;
pub mod node;
pub mod rates;
mod secrets;
mod tokens;
mod wallet;

pub use contacts::{address_book, connected_apps, contact_addresses, hidden_requests};
pub use history::{core_balance, journal, mempool, pending, pool_history};
pub use secrets::{attempts, notes, security, vault};
pub use tokens::wallet_tokens;
pub use wallet::{folders, json_file, wallets};
