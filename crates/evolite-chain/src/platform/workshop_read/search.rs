//! FEAT-0054 part 2 (the architect's «Поправка», 2026-11-22): the collections an identity got NFTs
//! of without making them — from DocumentHistory, which the evolite standard's `nft` keeps
//! (`keepsTransferHistory`, `keepsPurchaseHistory`): a `transfer` to it (index `to`:
//! `toIdentityId`, `$createdAt`) and a `purchase` by it (index `buyer`: `$ownerId`,
//! `$createdAt`). Each answers a contract id (`dataContractId`); which of them are collections of
//! the standard is [`super::profile::profile`]'s, later. Read from a cursor kept by the wallet,
//! so the next pass reads only what is new; a fresh copy reads from the start.

use std::collections::BTreeSet;
use std::sync::Arc;

use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::DocumentQuery;
use dpp::data_contract::DataContract;
use dpp::document::{Document, DocumentV0Getters};
use dpp::platform_value::Value;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;

use super::{PAGE, equal};

/// Where an identity's search stands, kept by the wallet: the newest `$createdAt` read of each
/// kind (ms) and the contracts found so far.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Found {
    #[serde(default)]
    pub received_ms: u64,
    #[serde(default)]
    pub bought_ms: u64,
    /// Contracts an NFT was transferred to the identity from.
    #[serde(default)]
    pub received: BTreeSet<String>,
    /// Contracts the identity bought an NFT of: a source it chose (its pictures drawn at once).
    #[serde(default)]
    pub bought: BTreeSet<String>,
}

/// Which history a query reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `transfer` to the identity.
    Received,
    /// `purchase` by the identity.
    Bought,
}

/// The documents of `kind` for `identity`, oldest first, from `since` (ms, inclusive) on.
pub fn query(
    history: Arc<DataContract>,
    kind: Kind,
    identity: Identifier,
    since: u64,
) -> Result<DocumentQuery, String> {
    let (doc_type, field) = match kind {
        Kind::Received => ("transfer", "toIdentityId"),
        Kind::Bought => ("purchase", "$ownerId"),
    };
    let mut query = DocumentQuery::new(history, doc_type)
        .map_err(|e| format!("{e:?}"))?
        .with_where(equal(field, identity))
        .with_order_by(OrderClause {
            field: "$createdAt".into(),
            ascending: true,
        })
        .with_limit(PAGE);
    if since > 0 {
        query = query.with_where(WhereClause {
            field: "$createdAt".into(),
            operator: WhereOperator::GreaterThanOrEquals,
            value: Value::U64(since),
        });
    }
    Ok(query)
}

/// A history document's contract (base58) and time (ms), when it has them. The whole history
/// of a contract's documents: which type is the instance is the profile's, when it is read.
pub fn contract_of(doc: &Document) -> Option<(String, u64)> {
    let contract = doc
        .properties()
        .get("dataContractId")?
        .to_identifier()
        .ok()?
        .to_string(Encoding::Base58);
    // A history document always has `$createdAt`; one without is only taken as the oldest —
    // the cursor never moves back for it, and its contract is still a candidate.
    // default-ok: no `$createdAt` is the oldest, see above
    Some((contract, doc.created_at().unwrap_or(0)))
}

impl Found {
    /// `docs` of `kind` read: their contracts added, the cursor moved to the newest time. The
    /// time itself is read again next time (`>=`): documents of the same millisecond may come
    /// later, and a contract already kept is only kept again.
    pub fn add(&mut self, kind: Kind, docs: impl IntoIterator<Item = (String, u64)>) {
        let (set, at) = match kind {
            Kind::Received => (&mut self.received, &mut self.received_ms),
            Kind::Bought => (&mut self.bought, &mut self.bought_ms),
        };
        for (contract, ms) in docs {
            set.insert(contract);
            *at = (*at).max(ms);
        }
    }

    /// Where the next read of `kind` starts.
    pub fn since(&self, kind: Kind) -> u64 {
        match kind {
            Kind::Received => self.received_ms,
            Kind::Bought => self.bought_ms,
        }
    }

    /// Every contract found, bought first.
    pub fn contracts(&self) -> Vec<String> {
        let mut all: Vec<String> = self.bought.iter().cloned().collect();
        all.extend(
            self.received
                .iter()
                .filter(|c| !self.bought.contains(*c))
                .cloned(),
        );
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cursor moves to the newest time read, a contract is kept once, the kinds apart.
    #[test]
    fn what_is_found_is_added_and_the_cursor_moves() {
        let mut found = Found::default();
        found.add(
            Kind::Received,
            [("A".into(), 10), ("B".into(), 30), ("A".into(), 20)],
        );
        found.add(Kind::Bought, [("C".into(), 5)]);
        assert_eq!(found.since(Kind::Received), 30);
        assert_eq!(found.since(Kind::Bought), 5);
        assert_eq!(found.received, ["A".to_string(), "B".to_string()].into());
        // Read again from 30: the same document again changes nothing.
        found.add(Kind::Received, [("B".into(), 30)]);
        assert_eq!(found.received.len(), 2);
        assert_eq!(found.since(Kind::Received), 30);
        // Bought first, then the others, none twice.
        found.add(Kind::Bought, [("A".into(), 7)]);
        assert_eq!(found.contracts(), vec!["A", "C", "B"]);
        // Kept as JSON and read back: the same.
        let json = serde_json::to_string(&found).unwrap();
        assert_eq!(serde_json::from_str::<Found>(&json).unwrap(), found);
    }
}
