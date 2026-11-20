//! What the network refuses that DPP's validation of a contract does not catch offline, checked
//! before the password: a refused registration costs the owner (0.00001092 DASH in the probe).

use serde_json::Value;

/// Why a draft is not sent. Typed, so the screen says it in the person's language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Keyword(KeywordRefusal),
    /// The description has this many characters, outside 3–100.
    DescriptionLength(usize),
    /// An index of `document_type` holds the system field `field`, which the network does not
    /// index (`$price`: «'$price' property is not defined», testnet protocol 14, 2026-11-19).
    SystemFieldInIndex {
        document_type: String,
        field: String,
    },
    /// A key a document type's schema may not hold.
    UnknownKey {
        document_type: String,
        key: String,
    },
    /// Two indexes of a type with one name.
    RepeatedIndexName {
        document_type: String,
        name: String,
    },
    /// An index on a field the type does not have.
    IndexOnMissingField {
        document_type: String,
        field: String,
    },
    /// DPP refused the contract: its words, for the journal and a detail line.
    Invalid(String),
}

/// Why the keywords are refused (FEAT-0019 АРХ-13, Platform's rules).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeywordRefusal {
    /// More than [`MAX_KEYWORDS`].
    TooMany(usize),
    /// A keyword of this many characters, outside 3–50.
    Length { keyword: String, chars: usize },
    /// A keyword with a space or a control character.
    Character { keyword: String },
    /// The same keyword twice (after lowercasing).
    Repeated { keyword: String },
}

/// At most this many keywords: each costs 0.1 DASH (architect, 2026-11-20; Platform allows
/// 50, which would be 5 DASH). More only by the customer's decision.
pub const MAX_KEYWORDS: usize = 5;
const KEYWORD_CHARS: std::ops::RangeInclusive<usize> = 3..=50;
const DESCRIPTION_CHARS: std::ops::RangeInclusive<usize> = 3..=100;

/// The system fields the network indexes: the owner, and the times and heights a document
/// keeps.
const INDEXED_SYSTEM_FIELDS: &[&str] = &[
    "$ownerId",
    "$createdAt",
    "$updatedAt",
    "$transferredAt",
    "$createdAtBlockHeight",
    "$updatedAtBlockHeight",
    "$transferredAtBlockHeight",
    "$createdAtCoreBlockHeight",
    "$updatedAtCoreBlockHeight",
    "$transferredAtCoreBlockHeight",
];

impl From<KeywordRefusal> for Refusal {
    fn from(k: KeywordRefusal) -> Self {
        Refusal::Keyword(k)
    }
}

/// The keywords as they are sent: lowercase (evolite's search matches them lowercase), each
/// checked.
pub fn keywords(words: &[String]) -> Result<Vec<String>, KeywordRefusal> {
    if words.len() > MAX_KEYWORDS {
        return Err(KeywordRefusal::TooMany(words.len()));
    }
    let mut kept: Vec<String> = Vec::with_capacity(words.len());
    for word in words {
        let keyword = word.to_lowercase();
        if keyword.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(KeywordRefusal::Character { keyword });
        }
        let chars = keyword.chars().count();
        if !KEYWORD_CHARS.contains(&chars) {
            return Err(KeywordRefusal::Length { keyword, chars });
        }
        if kept.contains(&keyword) {
            return Err(KeywordRefusal::Repeated { keyword });
        }
        kept.push(keyword);
    }
    Ok(kept)
}

/// The description as it is sent: none, or 3–100 characters. An empty one is none.
pub fn description(text: Option<&str>) -> Result<Option<String>, Refusal> {
    match text.filter(|t| !t.is_empty()) {
        None => Ok(None),
        Some(t) => {
            let chars = t.chars().count();
            if DESCRIPTION_CHARS.contains(&chars) {
                Ok(Some(t.to_owned()))
            } else {
                Err(Refusal::DescriptionLength(chars))
            }
        }
    }
}

/// The keys a document type's schema may hold: DPP's meta-schema v3 (`document-meta.json`,
/// `additionalProperties: false`). The release has no DPP schema check (its `validation`
/// feature is for tests only, the customer 2026-11-20), so a key past these is refused here.
const SCHEMA_KEYS: &[&str] = &[
    "$comment",
    "$defs",
    "$schema",
    "actionFees",
    "additionalProperties",
    "canBeDeleted",
    "creationRestrictionMode",
    "creatorRefersTo",
    "deleteConstraints",
    "dependentRequired",
    "description",
    "documentsAverageable",
    "documentsCountable",
    "documentsKeepHistory",
    "documentsMutable",
    "documentsSummable",
    "entryPayload",
    "immutable",
    "indexOnly",
    "indices",
    "keepsPricingHistory",
    "keepsPurchaseHistory",
    "keepsTransferHistory",
    "maxProperties",
    "minProperties",
    "moderatorAbilities",
    "ownerRefersTo",
    "properties",
    "propertyConstraints",
    "rangeAverageable",
    "rangeCountable",
    "rangeSummable",
    "required",
    "requiresIdentityDecryptionBoundedKey",
    "requiresIdentityEncryptionBoundedKey",
    "retractedWhen",
    "signatureSecurityLevelRequirement",
    "tokenCost",
    "tradeMode",
    "transferable",
    "transient",
    "ttl",
    "type",
];

/// The shape of `documents` as the network checks it and the release does not (DPP's schema
/// check is in tests only): each type's keys known, each index's name once, each indexed field
/// a property of the type or a system field the network indexes. What the offline test showed
/// DPP refusing (`dpp_refuses_offline_what_the_network_refuses`), refused here before DPP.
pub fn schema_shape(documents: &Value) -> Result<(), Refusal> {
    let Some(types) = documents.as_object() else {
        return Ok(());
    };
    for (document_type, schema) in types {
        let dt = || document_type.clone();
        let Some(schema) = schema.as_object() else {
            continue;
        };
        if let Some(key) = schema.keys().find(|k| !SCHEMA_KEYS.contains(&k.as_str())) {
            return Err(Refusal::UnknownKey {
                document_type: dt(),
                key: key.clone(),
            });
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        let mut names = std::collections::HashSet::new();
        let indices = schema.get("indices").and_then(Value::as_array);
        for index in indices.into_iter().flatten() {
            if let Some(name) = index.get("name").and_then(Value::as_str)
                && !names.insert(name)
            {
                return Err(Refusal::RepeatedIndexName {
                    document_type: dt(),
                    name: name.to_owned(),
                });
            }
            let fields = index.get("properties").and_then(Value::as_array);
            for field in fields
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .flat_map(|f| f.keys())
            {
                if field.starts_with('$') {
                    if !INDEXED_SYSTEM_FIELDS.contains(&field.as_str()) {
                        return Err(Refusal::SystemFieldInIndex {
                            document_type: dt(),
                            field: field.clone(),
                        });
                    }
                    continue;
                }
                // A nested property is named by its dotted path: its first step is a property.
                let first = field.split('.').next().unwrap_or(field);
                if !properties.is_some_and(|p| p.contains_key(first)) {
                    return Err(Refusal::IndexOnMissingField {
                        document_type: dt(),
                        field: field.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}
