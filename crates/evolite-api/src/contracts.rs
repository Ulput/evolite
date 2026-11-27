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
        /// «Belongs to a contract» (W2, UC-4): the contract (base58) the collection is part of,
        /// written after the registration as its «About the collection» document
        /// (`collectionInfo.mainContract`); `None`: no such document.
        main_contract: Option<String>,
    },
}

/// How the `Done` of [`crate::Spend::RegisterContract`] begins (`CoreError::Failed`) when the
/// collection was registered — its `Event::ContractRegistered` came — and its «About the
/// collection» document was not written (W2).
pub const ABOUT_NOT_WRITTEN: &str = "about not written";

/// As [`ABOUT_NOT_WRITTEN`], when the «About the collection» write went out and its answer was
/// lost (BUG-243): it may have been written — the collection's page reads it from the network.
pub const ABOUT_UNCLEAR: &str = "about unclear";

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
    /// The «About the collection» document written after it, estimated; 0: none.
    pub about: Duffs,
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
        self.registration()
            .saturating_add(self.storage)
            .saturating_add(self.about)
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
    /// «Belongs to a contract» is not a contract id.
    MainContractId,
    /// No contract with that id on Platform (read with a proof).
    MainContractMissing,
    /// BUG-342: the contract `id` could not be read — Platform answered and the answer was not
    /// taken (`undecoded`: perhaps newer than this evolite), or the read failed; `details`: what
    /// Platform answered, for «Details». The fee is still counted, never «the network refused».
    MainContractUnread {
        id: String,
        undecoded: bool,
        details: String,
    },
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
    /// «Belongs to a contract»: that contract's description, read with a proof, shown under the
    /// field (W2); `None` when none was asked, or it has none.
    pub main_description: Option<String>,
}
