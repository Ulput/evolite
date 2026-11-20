//! Drafts evolite makes itself.

use serde_json::Value;

use super::Draft;

/// The document types of a collection by the strict «NFT evolite v1» profile (FEAT-0031 P-1,
/// FEAT-0038 AC-T-W1): «image» (permanent, its picture or parts), «nft» (transferable and for
/// sale, deletable, transfer and purchase history, an index from `$ownerId`, a countable
/// `[imageId, $ownerId]`, a unique `(imageId, serial)`, `serial ≤ supply`) and
/// «collectionInfo» (one at most). Registered on testnet by the FEAT-0038 probe, 2026-11-19.
const COLLECTION: &str = include_str!("collection-v1.json");

/// A new collection: the profile's types, `readonly`, with `description` if given.
pub fn nft_collection(description: Option<String>) -> Draft {
    Draft {
        documents: serde_json::from_str::<Value>(COLLECTION).expect("the collection schema"),
        tokens: None,
        keywords: Vec::new(),
        description,
        readonly: true,
    }
}
