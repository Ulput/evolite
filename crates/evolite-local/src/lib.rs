//! The files of a wallet's folder: the vault, the list of wallets, what the wallet did, the
//! person's notes and the small JSON files beside the store. Nothing here talks to the chain.

#![forbid(unsafe_code)]

#[macro_use]
extern crate evolite_common;

pub mod address_book;
pub mod attempts;
pub mod connected_apps;
pub mod contact_addresses;
pub mod core_balance;
pub mod folders;
pub mod hidden_requests;
pub mod journal;
pub mod json_file;
pub mod mempool;
pub mod node;
pub mod notes;
pub mod pending;
pub mod pool_history;
pub mod rates;
pub mod security;
pub mod vault;
pub mod wallets;
