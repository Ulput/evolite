//! The Workshop's reading side (FEAT-0038 item 2, FEAT-0031 stage 1): which contracts hold NFTs
//! as evolite shows them ([`profile`]), how many of a model one has (the countable
//! `[model, $ownerId]`, AC-T-M5, R-14), one's own instances page by page and those of them for
//! sale (a local `$price` filter: Platform cannot index `$price`), and what one sold, from
//! DocumentHistory's `purchase` by `sellerId` (AC-T-W4). Everything here only reads, proved.
//!
//! Boundary with the series core (dev3, `platform/nft_series/`, agreed 2026-11-20): models and
//! collections pass as base58 ids; after a series this side only reads again.

mod chain;
pub mod profile;

pub use chain::model_of;

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;

use dapi_grpc::platform::v0::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::Sdk;
use dash_sdk::drive::query::{OrderClause, SelectProjection, WhereClause, WhereOperator};
use dash_sdk::platform::{DocumentQuery, Fetch, FetchMany};
use dpp::data_contract::DataContract;
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
