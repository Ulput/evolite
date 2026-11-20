//! A data contract the wallet registers itself (FEAT-0038 «Workshop» collections, FEAT-0019
//! T-4 tokens): what is asked for, what it costs, and why it may not go — checked before the
//! password with [`crate::Command::CheckContract`], written with
//! [`crate::Spend::RegisterContract`].

use crate::Duffs;

/// What is to be registered. Built by the core from templates, never from JSON the screen
/// writes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ContractDraft {
    /// A collection by the strict «NFT evolite v1» profile (FEAT-0031 P-1), `readonly`.
    NftCollection {
        /// 3–100 characters, if any.
        description: Option<String>,
        /// Search keywords, at most [`MAX_KEYWORDS`], 0.1 DASH each; made lowercase.
        keywords: Vec<String>,
    },
}

/// At most this many keywords: each costs 0.1 DASH (architect, 2026-11-20).
pub const MAX_KEYWORDS: usize = 5;

/// What a registration costs, part by part, in duffs (credits rounded up): the screen shows
/// the parts before the password, «+0.1 DASH» at each keyword.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ContractFee {
    /// The contract itself.
    pub base: Duffs,
    /// Its document types.
    pub document_types: Duffs,
    /// Their indexes.
    pub indexes: Duffs,
    /// Its tokens and their distributions.
    pub tokens: Duffs,
    /// Its keywords.
    pub keywords: Duffs,
    /// What one keyword adds.
    pub per_keyword: Duffs,
    /// Storage and processing, estimated with room to spare.
    pub storage: Duffs,
}

impl ContractFee {
    /// Platform's registration cost: exact.
    pub fn registration(&self) -> Duffs {
        [
            self.base,
            self.document_types,
            self.indexes,
            self.tokens,
            self.keywords,
        ]
        .into_iter()
        .fold(0, Duffs::saturating_add)
    }

    /// The most it takes: «≈ registration + storage, not more than this». The amount FEAT-0004's
    /// large-spend check is to count for [`crate::Spend::RegisterContract`].
    pub fn total(&self) -> Duffs {
        self.registration().saturating_add(self.storage)
    }
}

/// Why a contract is not registered, said before the password. Typed: the screen says it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ContractRefusal {
    /// No AUTHENTICATION · CRITICAL key of this wallet on the identity (FEAT-0038 AC-T-W6:
    /// «Add a key» with `Tokens`).
    NoKey,
    /// An earlier registration of this identity, sent without an answer and not seen yet, may
    /// still land (BUG-243): «The earlier registration may still go through — wait a couple of
    /// minutes».
    OnItsWay,
    /// The identity's credits are below the most it costs.
    NotEnoughCredits {
        have: Duffs,
        need: Duffs,
    },
    TooManyKeywords(usize),
    /// A keyword of this many characters, outside 3–50.
    KeywordLength {
        keyword: String,
        chars: usize,
    },
    /// A keyword with a space or a control character.
    KeywordCharacter {
        keyword: String,
    },
    /// The same keyword twice (lowercase).
    KeywordRepeated {
        keyword: String,
    },
    /// A description of this many characters, outside 3–100.
    DescriptionLength(usize),
    /// The network runs a protocol newer than this build of evolite knows: «Update evolite».
    NewerNetwork(u32),
    /// Platform's rules refuse the contract: their words, for «Details».
    Invalid(String),
}

/// The checks before the password: the fee when it could be counted, and the reason it cannot
/// go, if any.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ContractCheck {
    pub fee: Option<ContractFee>,
    pub refusal: Option<ContractRefusal>,
}
