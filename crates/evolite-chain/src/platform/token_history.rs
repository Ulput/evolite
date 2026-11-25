//! Token history (FEAT-0019 T-1, АРХ-8): the `transfer` documents of the TokenHistory system
//! contract for one identity and one token — to it (incoming) and from it (its own sends, which
//! a restore brings back, R-19) — proved, 100 a page, in the order of `$createdAt`. Read only
//! when asked (the token's layer in History, «Show earlier») or when a proved balance grew with
//! no own operation; never in the background over all tokens. How far each was read, and each
//! token's last proved balance, are kept in `token-history.json` ([`State`]); the documents
//! themselves become journal entries, keyed by their id, so a document is never a line twice.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{DocumentQuery, FetchMany};
use dpp::data_contract::DataContract;
use dpp::document::{Document, DocumentV0Getters};
use dpp::platform_value::Value;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use evolite_api::Timestamp;

use crate::chain::Chain;

/// Documents per page; Platform answers at most 100.
pub const PAGE: usize = 100;

/// Which way a transfer went, for the identity read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    /// To the identity: someone sent it the token (index `to`).
    To,
    /// From the identity: its own send (index `from`, by `$ownerId`).
    From,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Side::To => "to",
            Side::From => "from",
        }
    }
}

/// Which way a page goes from its cursor: newer documents than the newest read (a balance that
/// grew), or older than the oldest shown («Show earlier»; the first page with no cursor is the
/// newest one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    Newer,
    Older,
}

impl Direction {
    fn name(self) -> &'static str {
        match self {
            Direction::Newer => "newer",
            Direction::Older => "older",
        }
    }
}

/// A `transfer` document, as the wallet keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferDoc {
    /// The document's id (base58): the line's key, never two lines for one.
    pub id: String,
    /// Who sent it: the document's owner (base58).
    pub from: String,
    /// To whom (base58).
    pub to: String,
    /// In the token's smallest units.
    pub amount: u64,
    /// `$createdAt`, ms.
    pub at_ms: u64,
}

impl TransferDoc {
    /// Its time, in seconds.
    pub fn at(&self) -> Timestamp {
        (self.at_ms / 1000) as Timestamp
    }
}

/// How far one (identity, token, side) was read: the last `$createdAt` and the ids read at that
/// very millisecond (the next page asks from it on, `>=`, and drops them: two documents of one
/// millisecond are neither lost nor read twice).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub at_ms: u64,
    #[serde(default)]
    pub ids: BTreeSet<String>,
}

/// A token's last proved balance on an identity, and when it was read (UC-5в).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Balance {
    pub units: u64,
    pub at: Timestamp,
}

/// `token-history.json`: `{"version":1,"cursors":{…},"balances":{…}}`.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct State {
    #[serde(default = "version")]
    pub version: u32,
    /// `"<identity>:<token>:to|from:newer|older"` → how far it was read; none: not read yet.
    #[serde(default)]
    pub cursors: BTreeMap<String, Cursor>,
    /// `"<identity>:<token>"` → its last proved balance.
    #[serde(default)]
    pub balances: BTreeMap<String, Balance>,
}

fn version() -> u32 {
    1
}

/// Why the file was not taken: it is there and does not read. It is then left as it is — not
/// written over — and everything is read again from the network (as `lock-status.json`).
#[derive(Debug, PartialEq, Eq)]
pub struct Broken;

impl State {
    /// The file's state; an empty one when there is no file yet; [`Broken`] when it does not read.
    pub fn load(path: &Path) -> Result<Self, Broken> {
        match evolite_local::json_file::read(path) {
            Ok(bytes) => serde_json::from_slice::<State>(&bytes)
                .ok()
                .filter(|s| s.version == 1)
                .ok_or(Broken),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State {
                version: 1,
                ..State::default()
            }),
            Err(_) => Err(Broken),
        }
    }

    /// Written whole beside the file and renamed over it (`json_file`).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        evolite_local::json_file::save(path, self)
    }

    pub fn cursor(
        &self,
        identity: &str,
        token: &str,
        side: Side,
        dir: Direction,
    ) -> Option<&Cursor> {
        self.cursors.get(&cursor_key(identity, token, side, dir))
    }

    pub fn set_cursor(
        &mut self,
        identity: &str,
        token: &str,
        side: Side,
        dir: Direction,
        cursor: Cursor,
    ) {
        self.cursors
            .insert(cursor_key(identity, token, side, dir), cursor);
    }

    pub fn balance(&self, identity: &str, token: &str) -> Option<Balance> {
        self.balances.get(&balance_key(identity, token)).copied()
    }

    pub fn set_balance(&mut self, identity: &str, token: &str, balance: Balance) {
        self.balances.insert(balance_key(identity, token), balance);
    }
}

fn cursor_key(identity: &str, token: &str, side: Side, dir: Direction) -> String {
    format!("{identity}:{token}:{}:{}", side.name(), dir.name())
}

fn balance_key(identity: &str, token: &str) -> String {
    format!("{identity}:{token}")
}

/// A page as Platform answered it (newer: `$createdAt >= cursor`, ascending; older: `<=`,
/// descending) taken past the cursor: the documents not read yet, in `dir`'s order, and the
/// cursor after them. The ids read at the cursor's millisecond are dropped; the new cursor keeps
/// the ids of its own last millisecond.
pub fn past_cursor(
    page: Vec<TransferDoc>,
    cursor: Option<&Cursor>,
    dir: Direction,
) -> (Vec<TransferDoc>, Cursor) {
    let beyond = |d: &TransferDoc, c: &Cursor| match dir {
        Direction::Newer => d.at_ms > c.at_ms,
        Direction::Older => d.at_ms < c.at_ms,
    };
    let mut docs: Vec<TransferDoc> = page
        .into_iter()
        .filter(|d| match cursor {
            Some(c) => beyond(d, c) || (d.at_ms == c.at_ms && !c.ids.contains(&d.id)),
            None => true,
        })
        .collect();
    docs.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then_with(|| a.id.cmp(&b.id)));
    if dir == Direction::Older {
        docs.reverse();
    }
    let next = match docs.last() {
        // default-ok: an empty page: the cursor stays where it was
        None => cursor.cloned().unwrap_or_default(),
        Some(last) => {
            let at_ms = last.at_ms;
            let mut ids: BTreeSet<String> = docs
                .iter()
                .filter(|d| d.at_ms == at_ms)
                .map(|d| d.id.clone())
                .collect();
            // Still the cursor's millisecond: what was read at it before stays read.
            if let Some(c) = cursor.filter(|c| c.at_ms == at_ms) {
                ids.extend(c.ids.iter().cloned());
            }
            Cursor { at_ms, ids }
        }
    };
    (docs, next)
}

/// A page of `transfer` documents of `token_id`, `side` of `me`, from `at` in `dir`: TokenHistory
/// v2's `from` and `to` indexes (checked offline, `query_index_tests`).
pub(crate) fn transfer_query(
    contract: Arc<DataContract>,
    token_id: Identifier,
    me: Identifier,
    side: Side,
    dir: Direction,
    at: Option<u64>,
) -> Result<DocumentQuery, String> {
    let field = match side {
        Side::To => "toIdentityId",
        Side::From => "$ownerId",
    };
    let mut query = DocumentQuery::new(contract, "transfer")
        .map_err(|e| format!("{e:?}"))?
        .with_where(WhereClause {
            field: "tokenId".into(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(token_id.to_buffer()),
        })
        .with_where(WhereClause {
            field: field.into(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(me.to_buffer()),
        })
        .with_order_by(OrderClause {
            field: "$createdAt".into(),
            ascending: dir == Direction::Newer,
        })
        .with_limit(PAGE as u32);
    if let Some(at) = at {
        query = query.with_where(WhereClause {
            field: "$createdAt".into(),
            operator: match dir {
                Direction::Newer => WhereOperator::GreaterThanOrEquals,
                Direction::Older => WhereOperator::LessThanOrEquals,
            },
            value: Value::U64(at),
        });
    }
    Ok(query)
}

impl Chain {
    /// One page of `transfer` documents of `token` for `identity` (both base58), `side` of it,
    /// past `cursor` in `dir`: proved, at most [`PAGE`], with the cursor after them. Spends
    /// nothing.
    pub async fn token_transfers(
        &self,
        token: &str,
        identity: &str,
        side: Side,
        dir: Direction,
        cursor: Option<&Cursor>,
    ) -> Result<(Vec<TransferDoc>, Cursor), String> {
        self.ready()?;
        let sdk = self.sdk().await?;
        let contract = load_system_data_contract(SystemDataContract::TokenHistory, sdk.version())
            .map(Arc::new)
            .map_err(|e| format!("{e:?}"))?;
        let id = |s: &str| Identifier::from_string(s, Encoding::Base58).map_err(|e| e.to_string());
        let (token_id, me) = (id(token)?, id(identity)?);
        let query = transfer_query(contract, token_id, me, side, dir, cursor.map(|c| c.at_ms))?;
        let docs: Vec<TransferDoc> = Document::fetch_many(&sdk, query)
            .await
            .map_err(|e| format!("{e:?}"))?
            .into_values()
            .flatten()
            .filter_map(|d| transfer_doc(&d))
            .collect();
        Ok(past_cursor(docs, cursor, dir))
    }
}

/// A `transfer` document's fields; `None` for one without them.
fn transfer_doc(doc: &Document) -> Option<TransferDoc> {
    let props = doc.properties();
    let to = props.get("toIdentityId")?.to_identifier().ok()?;
    let amount = props.get("amount")?.to_integer::<u64>().ok()?;
    Some(TransferDoc {
        id: doc.id().to_string(Encoding::Base58),
        from: doc.owner_id().to_string(Encoding::Base58),
        to: to.to_string(Encoding::Base58),
        amount,
        at_ms: doc.created_at().unwrap_or(0), // default-ok: timestamp for ordering only
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(n: u64, at_ms: u64) -> TransferDoc {
        TransferDoc {
            id: format!("doc{n:03}"),
            from: "SENDER".into(),
            to: "ME".into(),
            amount: n,
            at_ms,
        }
    }

    /// Platform's side of a page: newer `$createdAt >= cursor` ascending, older `<=`
    /// descending, at most 100.
    fn platform_dir(
        all: &[TransferDoc],
        cursor: Option<&Cursor>,
        dir: Direction,
    ) -> Vec<TransferDoc> {
        let mut page: Vec<TransferDoc> = all
            .iter()
            .filter(|d| {
                cursor.is_none_or(|c| match dir {
                    Direction::Newer => d.at_ms >= c.at_ms,
                    Direction::Older => d.at_ms <= c.at_ms,
                })
            })
            .cloned()
            .collect();
        page.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then_with(|| a.id.cmp(&b.id)));
        if dir == Direction::Older {
            page.reverse();
        }
        page.truncate(PAGE);
        page
    }

    fn platform(all: &[TransferDoc], cursor: Option<&Cursor>) -> Vec<TransferDoc> {
        platform_dir(all, cursor, Direction::Newer)
    }

    /// «Show earlier»: the first page is the newest, each next one older, nothing lost or twice
    /// across a millisecond shared at a page's edge.
    #[test]
    fn earlier_pages_go_back_without_gaps() {
        let mut all: Vec<TransferDoc> = (0..230).map(|n| doc(n, 1_000 + n * 10)).collect();
        for n in 128..133 {
            all[n as usize].at_ms = 2_300;
        }
        let mut cursor: Option<Cursor> = None;
        let mut read: Vec<TransferDoc> = Vec::new();
        for _ in 0..10 {
            let page = platform_dir(&all, cursor.as_ref(), Direction::Older);
            let (docs, next) = past_cursor(page, cursor.as_ref(), Direction::Older);
            if docs.is_empty() {
                break;
            }
            read.extend(docs);
            cursor = Some(next);
        }
        assert_eq!(
            read.first().map(|d| d.amount),
            Some(229),
            "the newest first"
        );
        let mut ids: Vec<&str> = read.iter().map(|d| d.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!((ids.len(), read.len()), (all.len(), all.len()));
    }

    /// T-1.1 (AC-T10): page 2 goes on from page 1 with nothing lost and nothing twice — with
    /// several documents of one millisecond across the pages' edge.
    #[test]
    fn page_two_goes_on_from_page_one() {
        // 250 documents; five share the millisecond at the first page's edge, three the second's.
        let mut all: Vec<TransferDoc> = (0..250).map(|n| doc(n, 1_000 + n * 10)).collect();
        for n in 96..101 {
            all[n as usize].at_ms = 1_960;
        }
        for n in 198..201 {
            all[n as usize].at_ms = 2_980;
        }
        let mut cursor: Option<Cursor> = None;
        let mut read: Vec<TransferDoc> = Vec::new();
        for _ in 0..10 {
            let (docs, next) = past_cursor(
                platform(&all, cursor.as_ref()),
                cursor.as_ref(),
                Direction::Newer,
            );
            if docs.is_empty() {
                break;
            }
            read.extend(docs);
            cursor = Some(next);
        }
        let mut ids: Vec<&str> = read.iter().map(|d| d.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), read.len(), "nothing twice");
        assert_eq!(read.len(), all.len(), "nothing lost");
        // Read again from the end: nothing new.
        let (again, _) = past_cursor(
            platform(&all, cursor.as_ref()),
            cursor.as_ref(),
            Direction::Newer,
        );
        assert!(again.is_empty(), "{again:?}");
    }

    /// A new document after the last read comes alone.
    #[test]
    fn a_new_document_comes_alone() {
        let all: Vec<TransferDoc> = (0..3).map(|n| doc(n, 1_000 + n)).collect();
        let (first, cursor) = past_cursor(platform(&all, None), None, Direction::Newer);
        assert_eq!(first.len(), 3);
        let mut more = all.clone();
        more.push(doc(9, 1_002));
        let (new, _) = past_cursor(
            platform(&more, Some(&cursor)),
            Some(&cursor),
            Direction::Newer,
        );
        assert_eq!(
            new,
            vec![doc(9, 1_002)],
            "the same millisecond, not read yet"
        );
    }

    /// T-1.6: the file written and read back is the same; a broken file is [`Broken`] and stays
    /// as it was; no file is an empty state.
    #[test]
    fn the_state_file_reads_back_and_a_broken_one_is_left() {
        let dir =
            std::env::temp_dir().join(format!("evolite-token-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("token-history.json");
        let empty = State::load(&path).expect("no file: empty");
        assert!(empty.cursors.is_empty() && empty.balances.is_empty());
        let mut state = empty;
        state.set_cursor(
            "ME",
            "TOK",
            Side::To,
            Direction::Newer,
            Cursor {
                at_ms: 5,
                ids: ["a".to_owned()].into(),
            },
        );
        state.set_balance("ME", "TOK", Balance { units: 7, at: 9 });
        state.save(&path).unwrap();
        let back = State::load(&path).unwrap();
        assert_eq!(back, state, "after a restart, the same");
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(State::load(&path), Err(Broken));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{ not json",
            "left as it was"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
