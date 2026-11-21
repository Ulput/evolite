//! Contracts fresh enough for their use (design «fresh»; review conditions 1, 6–9): the network
//! behind [`Source`], so the rule is tested without it.
//!
//! - **No lock across the network** (condition 6): the cache's file and map are locked only to
//!   read or to write; while a version or a body is asked, only that id's flight is held.
//! - **One flight per id** (condition 7): calls for the same contract at the same time wait for
//!   the first, then take what it kept.
//! - **Many at once** (condition 7): the stale ones' versions in one request, the bodies to read in
//!   one more.
//! - **A failed check** (condition 8): a read gets the contract kept, marked not checked; a write
//!   (`proved`) gets the error, before anything is signed.

use std::collections::HashMap;
use std::sync::Arc;

use dpp::data_contract::DataContract;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::accessors::v1::DataContractV1Getters;
use dpp::prelude::Identifier;

use super::{Cached, ContractCache, Next, decide};

/// The answer for a contract Platform proved absent: its one wording, so a caller can tell it
/// from a failed read (the Workshop's «Belongs to a contract», W2).
pub const NOT_ON_PLATFORM: &str = "the contract is not on Platform";

/// Where versions and bodies come from: Platform, with proofs (the chain), or a test's stand-in.
pub trait Source {
    /// The latest versions of `ids`, proved; an id missing from the answer has no contract.
    fn latest_versions(
        &self,
        ids: &[Identifier],
    ) -> impl std::future::Future<Output = Result<HashMap<Identifier, u32>, String>> + Send;

    /// The bodies of `ids`, proved; an id missing has no contract.
    fn fetch(
        &self,
        ids: &[Identifier],
    ) -> impl std::future::Future<Output = Result<Vec<Arc<DataContract>>, String>> + Send;

    /// Keeps a proved contract: in `cache`, and wherever else it is to be known (the provider).
    fn keep(&self, cache: &ContractCache, contract: Arc<DataContract>) -> Result<Cached, String> {
        cache.put(contract, evolite_common::wall_secs() as i64)
    }
}

/// `ids` fresh within `within` seconds at `now`; `proved`: for a write or money (condition 1).
/// One answer per id, in the order asked.
pub async fn fresh_many<S: Source>(
    cache: &ContractCache,
    source: &S,
    ids: &[Identifier],
    within: i64,
    proved: bool,
    now: i64,
) -> Vec<Result<Cached, String>> {
    // This call's flights, held while the network is asked (never the file or the map).
    let mut order: Vec<Identifier> = ids.to_vec();
    order.sort();
    order.dedup();
    let flights: Vec<_> = order.iter().map(|id| cache.flight(id)).collect();
    let mut held = Vec::with_capacity(flights.len());
    for flight in &flights {
        held.push(flight.lock().await);
    }
    // Decided after the flights are held: one before may have kept what is needed.
    let mut answers: HashMap<Identifier, Result<Cached, String>> = HashMap::new();
    let mut check: Vec<(Identifier, Cached)> = Vec::new();
    let mut read: Vec<Identifier> = Vec::new();
    for id in &order {
        let kept = cache.get(id);
        match decide(kept.as_ref(), now, within, proved) {
            Next::Use => {
                let kept = kept.expect("decided on it");
                answers.insert(
                    *id,
                    Ok(Cached {
                        version_checked: true,
                        ..kept
                    }),
                );
            }
            Next::CheckVersion => check.push((*id, kept.expect("decided on it"))),
            Next::Fetch => read.push(*id),
        }
    }
    if !check.is_empty() {
        let asked: Vec<Identifier> = check.iter().map(|(id, _)| *id).collect();
        match source.latest_versions(&asked).await {
            Ok(versions) => {
                for (id, kept) in check {
                    if versions.get(&id) == Some(&kept.version) {
                        cache.checked(&id, now);
                        answers.insert(
                            id,
                            Ok(Cached {
                                checked_at: now,
                                version_checked: true,
                                ..kept
                            }),
                        );
                    } else {
                        read.push(id);
                    }
                }
            }
            // Condition 8: a read keeps going on the contract kept; a write stops here.
            Err(e) => {
                for (id, kept) in check {
                    let answer = if proved {
                        Err(format!("the contract's version could not be checked: {e}"))
                    } else {
                        Ok(Cached {
                            version_checked: false,
                            ..kept
                        })
                    };
                    answers.insert(id, answer);
                }
            }
        }
    }
    if !read.is_empty() {
        match source.fetch(&read).await {
            Ok(fetched) => {
                for contract in fetched {
                    let id = contract.id();
                    let kept = source.keep(cache, contract).map(|c| Cached {
                        version_checked: true,
                        ..c
                    });
                    answers.insert(id, kept);
                }
                for id in &read {
                    answers
                        .entry(*id)
                        .or_insert_with(|| Err(NOT_ON_PLATFORM.to_owned()));
                }
            }
            Err(e) => {
                for id in &read {
                    // A read may still show the one kept, not checked (condition 8).
                    let answer = match cache.get(id) {
                        Some(kept) if !proved => Ok(Cached {
                            version_checked: false,
                            ..kept
                        }),
                        _ => Err(e.clone()),
                    };
                    answers.insert(*id, answer);
                }
            }
        }
    }
    drop(held);
    ids.iter()
        .map(|id| {
            answers
                .get(id)
                .cloned()
                .unwrap_or_else(|| Err("not answered".to_owned()))
        })
        .collect()
}

/// The contract and position of `token_id` for a write (review conditions 1 and 9): the contract
/// proved in this run, and its own token id at that position must be `token_id` — a token row
/// changed on the disk cannot point a write at another token.
pub async fn token_for_write<S: Source>(
    cache: &ContractCache,
    source: &S,
    token_id: &Identifier,
    now: i64,
) -> Result<(Cached, u16), String> {
    let (contract_id, position) = cache
        .token(token_id)
        .ok_or("the token is not among the contracts kept")?;
    let contract = fresh_many(cache, source, &[contract_id], 0, true, now)
        .await
        .pop()
        .unwrap_or_else(|| Err("not answered".to_owned()))?;
    if contract.contract.token_id(position).as_ref() != Some(token_id) {
        return Err("the token is not at that position of its contract".into());
    }
    Ok((contract, position))
}

#[cfg(test)]
#[path = "fresh_tests.rs"]
mod tests;
