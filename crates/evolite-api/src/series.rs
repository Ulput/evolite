//! «Workshop» series (FEAT-0038): one action over many instances of a model, one write per
//! instance and one password for the series (R-7). The serials are evolite's to choose (R-10).
//! Collections, models and their counts are `workshop`'s (FEAT-0038 reading core); a series
//! names them by id only.

use crate::Duffs;

/// A price in credits, as Platform keeps `$price` (1 duff = 1 000 credits).
pub type Credits = u64;

/// What a series does.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SeriesKind {
    /// Issue `count` more instances, on the serials after the highest ever issued (R-6).
    Mint { count: u32 },
    /// Put `count` free instances (held, no price) on sale at `price`, lowest serials first.
    Price { count: u32, price: Credits },
    /// «Change the price» of the group at `from`: the same instances at `price` (one price
    /// write over the other, architect 2026-11-20).
    Reprice { from: Credits, price: Credits },
    /// Give `count` free instances to `to` (an @name or an identity id), lowest serials first.
    Gift { count: u32, to: String },
    /// Take `count` instances off sale (at `price`, or at any price): a transfer to oneself,
    /// which drops `$price` (architect 2026-11-19), highest serials first.
    Unprice { count: u32, price: Option<Credits> },
    /// Delete `count` free instances for good (R-20), lowest serials first.
    Delete { count: u32 },
}

/// The model a series is about: its collection (contract) and its «image» document, by id.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SeriesTarget {
    /// The identity that writes, base58.
    pub identity: String,
    /// The collection's contract, base58.
    pub contract: String,
    /// The model («image» document), base58.
    pub model: String,
}

/// Why a series cannot go, said before the password.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SeriesRefusal {
    /// No AUTHENTICATION · CRITICAL key of this wallet (R-1, R-18).
    NoKey,
    /// The identity's credits are below the estimate (R-7).
    NotEnoughCredits { have: Duffs, need: Duffs },
    /// Fewer free instances than asked (or none): «You have no free instances» (R-10).
    NotEnoughInstances { have: u32 },
    /// Not enough could be found within the pages read: «try fewer» (architect).
    TooFewFound { found: u32 },
    /// Past the supply: this many can still be issued (R-6).
    PastSupply { left: u32 },
    /// A price of 0 (R-9): «Take off sale» does that.
    ZeroPrice,
    /// A gift to oneself (R-11).
    ToSelf,
    /// The recipient was not found (R-11).
    NoRecipient,
    /// Another series of this identity is running (R-8).
    SeriesRunning,
    /// No instance at that price.
    NoneAtPrice,
    /// A new model's name: 1–64 characters.
    NameLength(usize),
    /// A new model's description: up to 280 characters.
    DescriptionLength(usize),
    /// A link in the name or the description (R-4, [`has_link`]).
    Link,
    /// A supply of 0, or more issued now than it.
    Supply,
    /// The picture is not one of the profile's types (by its first bytes).
    ImageType,
    /// An SVG that could run something or reach a server: what (script, event handler, link,
    /// foreignObject).
    ImageUnsafe(String),
    /// Larger than the room in its write ([`PictureCheck::room`]), packed: its stored size in
    /// bytes. A raster may be made smaller ([`PictureCheck::can_fit`], R-21).
    ImageTooBig(usize),
    /// The file's structure could not be followed to take its metadata out (R-22): «Could not
    /// check the picture…». Never written as it is.
    ImageUnreadable,
}

/// What «Check» found of a new model's picture (R-21, R-22), with or without a refusal: what is
/// shown under it in W5.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PictureCheck {
    /// The picture as it will be written — its metadata taken out — to draw with
    /// [`crate::Command::DrawArt`] (the core keeps it while W5 is open).
    pub art: String,
    /// Its bytes after the metadata went: what is weighed.
    pub size: usize,
    /// The room it has in its write (R-21: measured for this name and description).
    pub room: usize,
    /// Something was taken out (R-22: «Details stored in the file … were removed»).
    pub removed: bool,
    /// A 3D model: its details stay inside it (R-22, its own line).
    pub glb: bool,
    /// Larger than the room and a raster: «Make a smaller copy» (R-21).
    pub can_fit: bool,
}

/// «Check» of a series (R-7, R-10): what it will do, before the password.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SeriesPlan {
    /// The serials it takes, in order.
    pub serials: Vec<u32>,
    /// One write's estimate, and the whole (one write per instance; for a delete, the most it
    /// may cost, with no promise of a refund: R-12).
    pub per_write: Duffs,
    pub total: Duffs,
    /// The identity's credits now.
    pub credits: Duffs,
    pub refusal: Option<SeriesRefusal>,
    /// A new model's picture, written before the instances.
    pub model: Option<ModelWrite>,
    /// A new model's picture as checked, also when it is refused (R-21, R-22).
    pub picture: Option<PictureCheck>,
}

/// What `Event::PictureFitted` fails with when no copy fits even at the smallest (R-21-failure),
/// as against a picture that could not be read.
pub const DOES_NOT_FIT: &str = "does not fit";

/// What `Event::PictureFitted` fails with when the renderer's time ran out before any copy
/// fitted: the picture may fit — another try, on a less busy machine, may find the copy.
pub const OUT_OF_TIME: &str = "out of time";

/// «Make a smaller copy» done (R-21): the copy, made by the renderer, never decoded here.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FittedPicture {
    /// The copy's bytes: what the model will carry if «Use» is pressed.
    pub image: Vec<u8>,
    /// To draw it ([`crate::Command::DrawArt`]).
    pub art: String,
    /// A PNG with a palette (it had transparency); else a JPEG.
    pub png: bool,
    pub width: u32,
    pub height: u32,
    /// The original file's own size.
    pub source_width: u32,
    pub source_height: u32,
}

/// The «image» write of a new model, in its plan (W5: the scale «5 / 19 KB»).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ModelWrite {
    /// The picture as stored: packed when that is smaller.
    pub stored: usize,
    /// One: the «image» field; more: parts.
    pub parts: u8,
    pub fee: Duffs,
}

/// How one write of a series ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemState {
    Waiting,
    /// Sent, its answer not here yet.
    Sending,
    Done,
    /// Not written: no longer ours, not enough credits, refused by the network.
    Refused,
    /// Sent, not seen: it may still land (BUG-243); «Continue» reads it again first.
    Unclear,
}

/// A series as the screens show it (W12, W12b, W13).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SeriesState {
    pub id: u64,
    pub target: SeriesTarget,
    pub kind: SeriesKind,
    pub total: u32,
    pub done: u32,
    pub refused: u32,
    pub unclear: u32,
    /// Running now; otherwise stopped (by the person, a lock, an error) with what is left.
    pub running: bool,
    /// «Stop after the current ones» was asked.
    pub stopping: bool,
    /// One write's estimate: «Continue» costs this for each write left.
    pub per_write: Duffs,
}

/// Whether `text` holds a link by the «Collection»'s rule (FEAT-0031, R-4 of FEAT-0038): `://`,
/// `www.`, or a word like `name.domain` — Latin letters, a dot, two or more Latin letters
/// («v1.2» is not one). The «Workshop» refuses such a text before the password, the
/// «Collection» does not show what holds one: one rule for both.
pub fn has_link(text: &str) -> bool {
    let lower = text.to_lowercase();
    if lower.contains("://") || lower.contains("www.") {
        return true;
    }
    let b = lower.as_bytes();
    for (i, _) in lower.match_indices('.') {
        let before = b[..i]
            .iter()
            .rev()
            .take_while(|c| c.is_ascii_alphabetic())
            .count();
        let after = b[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_alphabetic())
            .count();
        // A word joined to the dot on both sides: «evolite.org», not «v1.2» nor «end. Next».
        let word_before = before > 0 && b.get(i - before - 1).is_none_or(|c| !c.is_ascii_digit());
        if word_before && after >= 2 {
            return true;
        }
    }
    false
}

/// A model to create with its picture (FEAT-0038 UC-5).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NewModel {
    /// Up to 64 characters, no link.
    pub name: String,
    /// Up to 280 characters, no link.
    pub description: String,
    pub supply: u32,
    /// The picture's file as it is: packed by the core.
    pub image: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::has_link;

    #[test]
    fn a_link_is_told_by_the_collection_s_rule() {
        for text in [
            "see https://x.test",
            "www.example",
            "go to evolite.org now",
            "Mail: a.b@mail.ru",
        ] {
            assert!(has_link(text), "{text}");
        }
        for text in [
            "v1.2",
            "Model 1.5",
            "The end. Next one",
            "Fire & ice",
            "Дракон.ру",
        ] {
            assert!(!has_link(text), "{text}");
        }
    }
}
