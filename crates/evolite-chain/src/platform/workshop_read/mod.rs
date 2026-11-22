//! The Workshop's reading side (FEAT-0038 item 2, FEAT-0031 stage 1): which contracts hold NFTs
//! as evolite shows them ([`profile`]), how many of a model one has (the countable
//! `[model, $ownerId]`, AC-T-M5, R-14), one's own instances page by page and those of them for
//! sale (a local `$price` filter: Platform cannot index `$price`), and what one sold, from
//! DocumentHistory's `purchase` by `sellerId` (AC-T-W4). Everything here only reads, proved.
//!
//! Boundary with the series core (dev3, `platform/nft_series/`, agreed 2026-11-20): models and
//! collections pass as base58 ids; after a series this side only reads again.

mod chain;
pub mod model_cache;
pub mod profile;

pub use chain::{model_of, model_picture};

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;

use dapi_grpc::platform::v0::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::Sdk;
use dash_sdk::drive::query::{OrderClause, SelectProjection, WhereClause, WhereOperator};
use dash_sdk::platform::{DocumentQuery, Fetch, FetchMany};
use dpp::data_contract::DataContract;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::document::{Document, DocumentV0Getters};
use dpp::platform_value::Value;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use drive_proof_verifier::DocumentCount;

pub use profile::{Limit, Profile, Unfit, profile};

/// Documents per request: Platform's most.
pub const PAGE: u32 = 100;
/// Pages read per action before «Показать ещё» (AC-T-M5 after the probe).
pub const MAX_PAGES: usize = 5;
/// DocumentHistory (system contract 8): its id is known without its schema built in; the
/// schema comes from the network through the contracts cache.
pub const DOCUMENT_HISTORY: &str = "6voHRaoiPcfmMhbqCA9dixH98xcgPQ9UEcuaXjpVu3LD";

/// What [`pages`] read: the documents kept, where to go on from (`None`: the end), and how
/// many documents it read to find them.
#[derive(Debug, Default)]
pub struct Page {
    pub docs: Vec<Document>,
    pub next: Option<Identifier>,
    pub read: usize,
}

/// Reads `query` page by page in its own order (each after the last document read), keeping the
/// documents `keep` says, until it has `want` of them or has read `max_pages` pages. Proved,
/// as `Document::fetch_many`; a page shorter than [`PAGE`] is the last. Shared with the series
/// core («свободные экземпляры»).
pub async fn pages(
    sdk: &Sdk,
    query: DocumentQuery,
    start_after: Option<Identifier>,
    want: usize,
    max_pages: usize,
    keep: impl FnMut(&Document) -> bool,
) -> Result<Page, String> {
    pages_with(start_after, want, max_pages, keep, |after| {
        let mut q = query.clone().with_limit(PAGE);
        if let Some(after) = after {
            q.start = Some(Start::StartAfter(after.to_buffer().to_vec()));
        }
        async move {
            Document::fetch_many(sdk, q)
                .await
                .map(|m| m.into_values().flatten().collect())
                .map_err(|e| format!("{e:?}"))
        }
    })
    .await
}

/// [`pages`] over any source of pages: `fetch(after)` gives the documents after `after`, at
/// most [`PAGE`] of them.
pub async fn pages_with<F, Fut>(
    start_after: Option<Identifier>,
    want: usize,
    max_pages: usize,
    mut keep: impl FnMut(&Document) -> bool,
    mut fetch: F,
) -> Result<Page, String>
where
    F: FnMut(Option<Identifier>) -> Fut,
    Fut: Future<Output = Result<Vec<Document>, String>>,
{
    let mut page = Page {
        next: start_after,
        ..Page::default()
    };
    for _ in 0..max_pages {
        // In the query's order (the proved answer is an ordered map): the last one read is
        // where the next page starts, whatever the query orders by.
        let docs = fetch(page.next).await?;
        page.read += docs.len();
        let last = docs.last().map(|d| d.id());
        let full = docs.len() >= PAGE as usize;
        for d in docs {
            if keep(&d) {
                page.docs.push(d);
            }
        }
        page.next = if full { last } else { None };
        if page.next.is_none() || page.docs.len() >= want {
            break;
        }
    }
    Ok(page)
}

/// `field == id` for a query.
fn equal(field: &str, id: Identifier) -> WhereClause {
    WhereClause {
        field: field.into(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(id.to_buffer()),
    }
}

/// The instances of `model` that `owner` holds, by the `[model, $ownerId]` index.
pub fn mine_query(
    contract: Arc<DataContract>,
    profile: &Profile,
    model: Identifier,
    owner: Identifier,
) -> Result<DocumentQuery, String> {
    Ok(DocumentQuery::new(contract, &profile.instance)
        .map_err(|e| format!("{e:?}"))?
        .with_where(equal(&profile.model_field, model))
        .with_where(equal("$ownerId", owner)))
}

/// An instance's price for sale: `$price` above 0 (0 is not for sale, the architect's
/// decision on R-14).
pub fn price_of(doc: &Document) -> Option<u64> {
    doc.properties()
        .get("$price")
        .and_then(|v| v.to_integer::<u64>().ok())
        .filter(|p| *p > 0)
}

/// An instance's number.
pub fn serial_of(doc: &Document) -> Option<u32> {
    doc.properties()
        .get(profile::SERIAL)
        .and_then(|v| v.to_integer::<u32>().ok())
}

/// How many of `model` `owner` holds: proved, by the countable index (AC-T-M5); a contract
/// without one is counted from its pages by the caller ([`Limit::CountByPages`]).
pub async fn count_mine(
    sdk: &Sdk,
    contract: Arc<DataContract>,
    profile: &Profile,
    model: Identifier,
    owner: Identifier,
) -> Result<u64, String> {
    let query =
        mine_query(contract, profile, model, owner)?.with_select(SelectProjection::count_star());
    DocumentCount::fetch(sdk, query)
        .await
        .map(|c| c.map_or(0, |c| c.0))
        .map_err(|e| format!("{e:?}"))
}

/// A sale from DocumentHistory's `purchase`: the instance, its contract, the buyer
/// (`$ownerId`) and the price.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaleDoc {
    /// The `purchase` document's id: the key of the History line (AC-T-W4).
    pub id: String,
    pub contract: String,
    pub instance: String,
    pub buyer: String,
    pub price: u64,
    pub at_ms: u64,
}

/// A `purchase` document's fields; `None` for one without them.
pub fn sale_of(doc: &Document) -> Option<SaleDoc> {
    let props = doc.properties();
    let id = |k: &str| {
        props
            .get(k)
            .and_then(|v| v.to_identifier().ok())
            .map(|i| i.to_string(Encoding::Base58))
    };
    Some(SaleDoc {
        id: doc.id().to_string(Encoding::Base58),
        contract: id("dataContractId")?,
        instance: id("documentId")?,
        buyer: doc.owner_id().to_string(Encoding::Base58),
        price: props.get("price")?.to_integer::<u64>().ok()?,
        at_ms: doc.created_at().unwrap_or(0),
    })
}

/// BUG-271: the ids a contract of `owner` may have, for its identity nonces `from..=` the one
/// now (`nonce`, its value only: the high bits mark missed revisions) — by DPP's own
/// derivation, `generate_data_contract_id_v0`, never a hash of our own (byte order).
pub fn own_contract_ids(owner: Identifier, from: u64, nonce: u64) -> Vec<Identifier> {
    let to = nonce & dpp::identity::identity_nonce::IDENTITY_NONCE_VALUE_FILTER;
    (from.max(1)..=to)
        .map(|n| DataContract::generate_data_contract_id_v0(owner, n))
        .collect()
}

/// BUG-271: `owner`'s contracts for its nonces `from..=nonce`, `fetch`ed 100 ids at a time
/// (proved: absent ones are not there), each kept in `cache` — so a fresh data folder lists the
/// identity's own collections. How many of the NFT standard were new there, and the nonce
/// (its value) read to.
pub async fn scan_own<F, Fut>(
    cache: &crate::platform::contracts::ContractCache,
    owner: Identifier,
    from: u64,
    nonce: u64,
    mut fetch: F,
) -> Result<(usize, u64), String>
where
    F: FnMut(Vec<Identifier>) -> Fut,
    Fut: Future<Output = Result<Vec<Arc<DataContract>>, String>>,
{
    let ids = own_contract_ids(owner, from, nonce);
    let mut new = 0;
    for chunk in ids.chunks(PAGE as usize) {
        for contract in fetch(chunk.to_vec()).await? {
            // Only the identity's own: an id from its nonce is, but a node's answer is checked.
            if contract.owner_id() != owner {
                continue;
            }
            if profile::profile(&contract).is_ok() && cache.get(&contract.id()).is_none() {
                new += 1;
            }
            cache.put(contract, evolite_common::wall_secs() as i64)?;
        }
    }
    Ok((
        new,
        nonce & dpp::identity::identity_nonce::IDENTITY_NONCE_VALUE_FILTER,
    ))
}

/// W7 «Given» as read: how many, over how many transfers read, whether all were, and the
/// last gift's time (unix seconds) and recipient (`@name`, else its id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GivenRead {
    pub count: u64,
    pub read: u64,
    pub complete: bool,
    pub last: Option<(i64, String)>,
    /// The collection keeps no transfer history (`Limit::NoSender`): nothing was read.
    pub no_history: bool,
}

/// W7 «Given»: one gift — a `transfer` of one's own instance of the collection to someone else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GiftDoc {
    pub instance: Identifier,
    pub to: Identifier,
    pub at_ms: u64,
}

/// The `transfer` documents `owner` made, oldest first: DocumentHistory's
/// `from ($ownerId, $createdAt)` index. Not `byContract`: it has every holder's resales too.
pub fn transfers_query(
    history: Arc<DataContract>,
    owner: Identifier,
) -> Result<DocumentQuery, String> {
    Ok(DocumentQuery::new(history, "transfer")
        .map_err(|e| format!("{e:?}"))?
        .with_where(equal("$ownerId", owner))
        .with_order_by(OrderClause {
            field: "$createdAt".into(),
            ascending: true,
        })
        .with_limit(PAGE))
}

/// A `transfer` as a gift of `contract`'s instance type `instance_type` by `owner`, or `None`:
/// another collection, another type, one made by someone else (a holder's resale), or one to
/// `owner` itself — «Take off sale» is a transfer to oneself (FEAT-0038), never a gift.
pub fn gift_of(
    doc: &Document,
    contract: Identifier,
    instance_type: &str,
    owner: Identifier,
) -> Option<GiftDoc> {
    if doc.owner_id() != owner {
        return None;
    }
    let props = doc.properties();
    let id = |k: &str| props.get(k).and_then(|v| v.to_identifier().ok());
    if id("dataContractId")? != contract {
        return None;
    }
    if props.get("documentTypeName")?.as_text()? != instance_type {
        return None;
    }
    let to = id("toIdentityId")?;
    if to == owner {
        return None;
    }
    Some(GiftDoc {
        instance: id("documentId")?,
        to,
        at_ms: doc.created_at().unwrap_or(0),
    })
}

/// The instances of `ids` (at most [`PAGE`]), by their own ids: which model each is of.
pub fn instances_by_id(
    contract: Arc<DataContract>,
    profile: &Profile,
    ids: &[Identifier],
) -> Result<DocumentQuery, String> {
    Ok(DocumentQuery::new(contract, &profile.instance)
        .map_err(|e| format!("{e:?}"))?
        .with_where(WhereClause {
            field: "$id".into(),
            operator: WhereOperator::In,
            value: Value::Array(
                ids.iter()
                    .map(|i| Value::Identifier(i.to_buffer()))
                    .collect(),
            ),
        })
        .with_limit(PAGE))
}

/// Where the reading of one's sales stands: the newest sale's time and the ids read at it
/// (several may share a millisecond), so reading again adds only what is new.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SalesCursor {
    pub at_ms: u64,
    pub ids: BTreeSet<String>,
}

/// The `purchase` documents of `seller`, oldest first, from `since` on: DocumentHistory's
/// `seller (sellerId, $createdAt)` index.
pub fn sales_query(
    history: Arc<DataContract>,
    seller: Identifier,
    since: Option<u64>,
) -> Result<DocumentQuery, String> {
    let mut query = DocumentQuery::new(history, "purchase")
        .map_err(|e| format!("{e:?}"))?
        .with_where(equal("sellerId", seller))
        .with_order_by(OrderClause {
            field: "$createdAt".into(),
            ascending: true,
        })
        .with_limit(PAGE);
    if let Some(since) = since {
        query = query.with_where(WhereClause {
            field: "$createdAt".into(),
            operator: WhereOperator::GreaterThanOrEquals,
            value: Value::U64(since),
        });
    }
    Ok(query)
}

/// The sales in `docs` not read before `cursor`, oldest first, kept where `fits(contract)` says
/// the contract fits the NFT profile (review r1 Should: a resale of another's collection is a
/// sale too; the index has no contract field), and the cursor after them. `fits` is `None` for
/// a contract not read: the cursor stops before that sale, so it is read again next time
/// instead of being passed over.
pub fn new_sales(
    docs: Vec<SaleDoc>,
    cursor: &SalesCursor,
    mut fits: impl FnMut(&str) -> Option<bool>,
) -> (Vec<SaleDoc>, SalesCursor) {
    let mut sales: Vec<SaleDoc> = docs
        .into_iter()
        .filter(|s| {
            s.at_ms > cursor.at_ms || (s.at_ms == cursor.at_ms && !cursor.ids.contains(&s.id))
        })
        .collect();
    sales.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then_with(|| a.id.cmp(&b.id)));
    let mut next = cursor.clone();
    let mut kept = Vec::new();
    for s in sales {
        match fits(&s.contract) {
            // Not read: neither kept nor passed.
            None => break,
            Some(fit) => {
                if next.at_ms != s.at_ms {
                    next = SalesCursor {
                        at_ms: s.at_ms,
                        ids: BTreeSet::new(),
                    };
                }
                next.ids.insert(s.id.clone());
                if fit {
                    kept.push(s);
                }
            }
        }
    }
    (kept, next)
}

#[cfg(test)]
mod tests;
