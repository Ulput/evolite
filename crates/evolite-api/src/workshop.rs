//! FEAT-0038 item 2 and FEAT-0031 stage 1: what the Workshop reads. Asked with
//! [`crate::Command::ReadWorkshop`], answered with [`crate::Event::Workshop`]. Only reads,
//! proved; nothing here spends. Collections and models pass as base58 ids (the series side,
//! `crate::series`, takes them so).

use crate::{Duffs, Timestamp};

/// What the screen asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkshopAsk {
    /// The shown identity's own collections: the contracts it owns that fit the NFT profile,
    /// with their models.
    Collections,
    /// A model's numbers (R-14, AC-A14): how many one holds, for sale, sold. Also «Обновить».
    Counts { contract: String, model: String },
    /// One's instances of a model, page by page in `$id` order (the architect's Q1), or only
    /// those for sale: at most 5 network pages per ask (AC-T-M5); `after`: «Показать ещё».
    Instances {
        contract: String,
        model: String,
        for_sale: bool,
        after: Option<String>,
    },
    /// What the identity sold since the last read (AC-T-W4); the core keeps where it stands.
    Sales,
    /// W7 «Given»: how many copies of a model the identity gave away, and the last gift
    /// (the architect, 2026-11-22: DocumentHistory's transfers from it, not to itself).
    Given { contract: String, model: String },
}

/// The answer to one [`WorkshopAsk`].
#[derive(Clone, Debug, PartialEq)]
pub enum WorkshopRead {
    Collections(Vec<Collection>),
    Counts(ModelCounts),
    Instances(Instances),
    Sales(Vec<Sale>),
    Given(Given),
}

/// W7 «Given N · last: {when}, @name».
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Given {
    pub contract: String,
    pub model: String,
    /// `Proved` when every transfer was read; `Counted { complete: false }` past the pages read
    /// («at least N»).
    pub count: Count,
    /// The last gift: when, and to whom (an @name, else a short id).
    pub last: Option<(Timestamp, String)>,
    /// Counted from this computer's series only: the collection keeps no transfer history.
    pub this_computer_only: bool,
}

/// BUG-271 r1 (TEAM.md §8d-2): where the search for the shown identity's own collections on
/// the network stands — said on the Workshop while it is not done.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OwnSearch {
    /// Done for this run (or nothing to search): the list is the whole of them.
    #[default]
    Done,
    /// Not done yet: more may come.
    Searching,
    /// The last pass failed; it is tried again after a pause, or on «Retry».
    Failed,
}

/// One's own collection.
#[derive(Clone, Debug, PartialEq)]
pub struct Collection {
    pub contract: String,
    /// The contract's description; empty when it has none.
    pub description: String,
    /// Anyone may make its instances; else the owner only.
    pub open: bool,
    /// What it lacks, named on its screen (AC-T-C2).
    pub limits: Vec<Limit>,
    pub models: Vec<Model>,
    pub checked_at: Timestamp,
}

/// A fitting contract's lack (AC-T-C2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Limit {
    /// «Отправитель неизвестен», no «Вернуть».
    NoSender,
    /// «Скрыть» instead of «Удалить навсегда».
    NoDelete,
    /// «Продано» without the buyer.
    NoBuyer,
    /// Counted page by page, marked so.
    CountByPages,
    /// Numbers checked by evolite, not by the network.
    SerialUnchecked,
}

/// A model («Изображение»).
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub description: String,
    /// The picture by its SHA-256 (hex), drawn on request as an NFT's ([`crate::Nft::art`]).
    pub art: String,
    /// The edition's size; 0: none set.
    pub supply: u32,
}

/// A model's numbers (R-14). `None`: not read (yet, or the read failed): «—».
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCounts {
    pub contract: String,
    pub model: String,
    pub mine: Option<Count>,
    /// For sale among one's own, from at most 5 pages; `complete` when they were all read.
    pub for_sale: Option<Count>,
    /// R-14 «ЦЕНЫ»: each price for sale and how many at it, cheapest first, from the same pages
    /// as `for_sale`.
    pub prices: Vec<(Duffs, u32)>,
    pub sold: Option<u32>,
    /// BUG-276: what those `sold` sales brought, from the same sales (their prices); `None`
    /// with `sold`.
    pub sold_for: Option<Duffs>,
    /// The highest number issued, by anyone's copy (W4 «выпущено {m} из {n}», FEAT-0038 R-6);
    /// `Some(0)`: none yet; `None`: not read.
    pub issued: Option<u32>,
    /// «проверено {время}».
    pub checked_at: Timestamp,
}

/// A number and how it was got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    /// From the countable index, proved.
    Proved(u64),
    /// Counted over `documents` documents read; `complete`: every one of them (else
    /// «не менее»).
    Counted {
        count: u64,
        documents: u64,
        complete: bool,
    },
}

/// A page of one's instances of a model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instances {
    pub contract: String,
    pub model: String,
    pub for_sale: bool,
    /// Sorted by number within the page (the architect's Q1).
    pub instances: Vec<Instance>,
    /// Where «Показать ещё» goes on; `None`: all read.
    pub next: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    pub id: String,
    pub serial: u32,
    /// Its price for sale; `None`: not for sale.
    pub price: Option<Duffs>,
}

/// A sale, from DocumentHistory's `purchase`: a History line «Продан «{model}» № {n} ·
/// @{name}», keyed by `purchase` (AC-T-W4); the amount is `price`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sale {
    pub purchase: String,
    pub contract: String,
    pub instance: String,
    /// The model's id and name; `None` when the instance is gone (deleted after the sale) and
    /// its model with it.
    pub model_id: Option<String>,
    pub model: Option<String>,
    /// `None` when the instance is gone: the line has no number.
    pub serial: Option<u32>,
    pub buyer: String,
    /// The buyer's name, when it has one.
    pub buyer_name: Option<String>,
    pub price: Duffs,
    pub at: Timestamp,
}
