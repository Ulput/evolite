//! A contract the wallet registers itself (FEAT-0038 module V1, FEAT-0019 T-4): its checks
//! before the password and its registration, by `evolite_chain::contract_v1`.

use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::accessors::v1::DataContractV1Getters;
use dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use evolite_api::contracts::{ContractCheck, ContractDraft, ContractFee, ContractRefusal};
use evolite_chain::contract_v1::fee::Fee;
use evolite_chain::contract_v1::register::RegisterError;
use evolite_chain::contract_v1::rules::{KeywordRefusal, Refusal};
use evolite_chain::contract_v1::{Draft, templates};

use crate::real::*;

/// The chain's draft for what the screen asked for.
pub(in crate::real) fn draft_of(draft: &ContractDraft) -> Draft {
    match draft {
        ContractDraft::NftCollection {
            description,
            keywords,
        } => {
            let mut d = templates::nft_collection(description.clone());
            d.keywords = keywords.clone();
            d
        }
    }
}

/// Credits as duffs, rounded up: a fee shown is never below what it is.
fn duffs(credits: u64) -> Duffs {
    Duffs::try_from(credits.div_ceil(1_000)).unwrap_or(Duffs::MAX)
}

pub(in crate::real) fn fee_of(fee: &Fee) -> ContractFee {
    ContractFee {
        base: duffs(fee.base),
        document_types: duffs(fee.document_types),
        indexes: duffs(fee.indexes),
        tokens: duffs(fee.tokens),
        keywords: duffs(fee.keywords),
        per_keyword: duffs(fee.per_keyword),
        storage: duffs(fee.storage),
    }
}

/// A refusal before the password as the screen says it; the others are errors.
pub(in crate::real) fn refusal_of(e: RegisterError) -> Result<ContractRefusal, CoreError> {
    Ok(match e {
        RegisterError::NoKey => ContractRefusal::NoKey,
        RegisterError::NewerNetwork(protocol) => ContractRefusal::NewerNetwork(protocol),
        RegisterError::NotEnough { have, need } => ContractRefusal::NotEnoughCredits {
            have: Duffs::try_from(have / 1_000).unwrap_or(Duffs::MAX),
            need: duffs(need),
        },
        RegisterError::Refused(r) => match r {
            Refusal::Keyword(KeywordRefusal::TooMany(n)) => ContractRefusal::TooManyKeywords(n),
            Refusal::Keyword(KeywordRefusal::Length { keyword, chars }) => {
                ContractRefusal::KeywordLength { keyword, chars }
            }
            Refusal::Keyword(KeywordRefusal::Character { keyword }) => {
                ContractRefusal::KeywordCharacter { keyword }
            }
            Refusal::Keyword(KeywordRefusal::Repeated { keyword }) => {
                ContractRefusal::KeywordRepeated { keyword }
            }
            Refusal::DescriptionLength(n) => ContractRefusal::DescriptionLength(n),
            Refusal::SystemFieldInIndex {
                document_type,
                field,
            } => ContractRefusal::Invalid(format!("{document_type}: {field} in an index")),
            Refusal::UnknownKey { document_type, key } => {
                ContractRefusal::Invalid(format!("{document_type}: no key {key}"))
            }
            Refusal::RepeatedIndexName {
                document_type,
                name,
            } => ContractRefusal::Invalid(format!("{document_type}: index {name} twice")),
            Refusal::IndexOnMissingField {
                document_type,
                field,
            } => ContractRefusal::Invalid(format!("{document_type}: no field {field} to index")),
            Refusal::Invalid(words) => ContractRefusal::Invalid(words),
        },
        // No proved answer yet: as any read before the sync.
        RegisterError::VersionUnknown => return Err(CoreError::NotSynced),
        RegisterError::Failed(e) => return Err(CoreError::Failed(e)),
        RegisterError::Unclear { .. } | RegisterError::NotFound { .. } => {
            return Err(CoreError::Unclear);
        }
    })
}

/// A registration's error as its `Done` says it. The screen checked first with
/// `CheckContract`, so a refusal here means something changed between the two.
fn registration_error(e: RegisterError) -> CoreError {
    match e {
        RegisterError::NotEnough { .. } => CoreError::NotEnough,
        RegisterError::VersionUnknown => CoreError::NotSynced,
        RegisterError::Unclear { .. } => CoreError::Unclear,
        RegisterError::Failed(e) => CoreError::Failed(e),
        other => CoreError::Failed(format!("{other:?}")),
    }
}

/// Where an earlier registration of the identity stands (BUG-243, r1 Must 1).
#[derive(Debug, PartialEq, Eq)]
pub(in crate::real) enum Prior<T> {
    /// None waits.
    None,
    /// One may still land: no new one meanwhile.
    Waits,
    /// It landed: found, and no longer waits.
    Found(T),
}

/// The earlier registration of `identity` kept as not known: within its quiet it waits; past
/// it, read again by `find` — found, it is done; not found twice [`sent::NO_GAP`] apart, it
/// did not land.
pub(in crate::real) async fn prior<T, F, Fut>(
    unclear: &sent::Unclear,
    identity: &str,
    find: F,
) -> Prior<T>
where
    F: FnOnce(u64) -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, String>>,
{
    let Some(p) = unclear.contract_of(identity) else {
        return Prior::None;
    };
    let sent::Watch::Contract { nonce, .. } = p.watch else {
        return Prior::None;
    };
    if p.running() || p.quiet_for().is_none_or(|q| q < sent::QUIET) {
        return Prior::Waits;
    }
    match find(nonce).await {
        Ok(Some(found)) => {
            unclear.take(p.ticket());
            Prior::Found(found)
        }
        Ok(None) if p.no_for().is_some_and(|d| d >= sent::NO_GAP) => {
            unclear.take(p.ticket());
            Prior::None
        }
        Ok(None) => {
            unclear.saw_no(p.ticket());
            Prior::Waits
        }
        Err(_) => Prior::Waits,
    }
}

/// A registration that did not end with its answer, as `Done` says it: refused before it went
/// out — a failure, a repeat may go; otherwise (not found right after, or the read after it
/// failed too) kept as not known (BUG-243, r1 Must 1, r2), so the next one waits ([`prior`]).
pub(in crate::real) fn kept_if_unseen(
    unclear: &sent::Unclear,
    identity: &str,
    e: RegisterError,
) -> CoreError {
    let (nonce, sent) = match e {
        RegisterError::NotFound { nonce, sent } | RegisterError::Unclear { nonce, sent, .. } => {
            (nonce, sent)
        }
        other => return registration_error(other),
    };
    if sent::refused_before(&sent) {
        return CoreError::Failed(sent);
    }
    unclear.not_seen(sent::Pending::new(
        identity.to_owned(),
        sent::Watch::Contract {
            owner: identity.to_owned(),
            nonce,
        },
        None,
        None,
        None,
    ))
}

fn identifier(id: &str) -> Result<Identifier, CoreError> {
    Identifier::from_string(id, Encoding::Base58)
        .map_err(|_| CoreError::NotAvailable(tr!("Make an identity first").into()))
}

impl Real {
    /// `Command::CheckContract`: answered later with `Event::ContractChecked`.
    pub(in crate::real) fn check_contract(
        &self,
        identity: String,
        draft: ContractDraft,
    ) -> Result<(), CoreError> {
        let (id, keys) = self.dashpay_ready()?;
        let owner = identifier(&identity)?;
        let chain = self.chain.clone();
        let (unclear, out) = (self.unclear.clone(), self.out.clone());
        self.answer_later("task:check_contract", async move {
            let result = async {
                let wallet = chain
                    .wallet(&id)
                    .await
                    .ok_or(CoreError::Failed(NOT_LOADED.into()))?;
                // r1 Must 1: an earlier registration that may still land is waited for.
                match prior(&unclear, &identity, |n| chain.registered_after(&owner, n)).await {
                    Prior::Waits => {
                        return Ok(ContractCheck {
                            fee: None,
                            refusal: Some(ContractRefusal::OnItsWay),
                        });
                    }
                    Prior::Found(c) => {
                        let _ = out.send(Event::ContractRegistered {
                            identity: identity.clone(),
                            id: c.id().to_string(Encoding::Base58),
                            charged: None,
                        });
                    }
                    Prior::None => {}
                }
                let checked = chain
                    .check_contract(&wallet, &keys, &owner, &draft_of(&draft))
                    .await;
                Ok(ContractCheck {
                    fee: checked.fee.as_ref().map(fee_of),
                    refusal: checked.refusal.map(refusal_of).transpose()?,
                })
            }
            .await;
            Event::ContractChecked {
                identity,
                draft,
                result,
            }
        });
        Ok(())
    }

    /// `Spend::RegisterContract`, past the password: one change of the identity at a time
    /// (FEAT-0038 AC-T-W5). `Event::ContractRegistered`, then `Done { what: Contract }`.
    pub(in crate::real) fn register_contract(
        &self,
        identity: String,
        draft: ContractDraft,
        fee_max: Duffs,
    ) -> Result<(), CoreError> {
        let Some(busy) = self.identity_busy.take() else {
            return Err(super::identity_keys::identity_busy());
        };
        let ready = self
            .dashpay_ready()
            .and_then(|ready| Ok((ready, identifier(&identity)?)));
        let ((id, keys), owner) = match ready {
            Ok(ready) => ready,
            Err(e) => {
                drop(busy);
                return Err(e);
            }
        };
        let chain = self.chain.clone();
        let out = self.out.clone();
        let unclear = self.unclear.clone();
        let refresh = self.refresher();
        self.answer_later("task:register_contract", async move {
            let started = std::time::Instant::now();
            let result = async {
                let wallet = chain
                    .wallet(&id)
                    .await
                    .ok_or(CoreError::Failed(NOT_LOADED.into()))?;
                // r1 Must 1: not twice. An earlier one that may still land: wait; found: it is
                // the registration (`recovered`), none is sent again.
                match prior(&unclear, &identity, |n| chain.registered_after(&owner, n)).await {
                    Prior::Waits => return Err(CoreError::Unclear),
                    Prior::Found(c) => {
                        let _ = out.send(Event::ContractRegistered {
                            identity: identity.clone(),
                            id: c.id().to_string(Encoding::Base58),
                            charged: None,
                        });
                        return Ok(());
                    }
                    Prior::None => {}
                }
                let chain_draft = draft_of(&draft);
                // FEAT-0004: what the person agreed to (and the tick counted) is the most it
                // takes; a fee grown past it since the check is refused before signing.
                let checked = chain
                    .check_contract(&wallet, &keys, &owner, &chain_draft)
                    .await;
                if let Some(fee) = checked.fee
                    && fee_of(&fee).total() > fee_max
                {
                    return Err(CoreError::Failed(format!(
                        "the fee grew: {} over {fee_max}",
                        fee_of(&fee).total()
                    )));
                }
                let done = match chain
                    .register_contract(&wallet, &keys, &owner, &chain_draft)
                    .await
                {
                    Ok(done) => done,
                    Err(e) => return Err(kept_if_unseen(&unclear, &identity, e)),
                };
                crate::diag::record(crate::diag::Record::ContractRegistered {
                    types: done.contract.document_types().len() as u32,
                    indexes: done
                        .contract
                        .document_types()
                        .values()
                        .map(|t| t.indexes().len() as u32)
                        .sum(),
                    keywords: done.contract.keywords().len() as u32,
                    bytes: done.bytes as u64,
                    estimate: done.fee.total().div_ceil(1_000),
                    charged: done.charged.map(|c| c.div_ceil(1_000)),
                    protocol: done.protocol,
                    ms: started.elapsed().as_millis() as u64,
                    recovered: done.recovered,
                });
                let _ = out.send(Event::ContractRegistered {
                    identity: identity.clone(),
                    id: done.contract.id().to_string(Encoding::Base58),
                    charged: done.charged.map(duffs),
                });
                Ok(())
            }
            .await;
            drop(busy);
            refresh();
            Event::Done {
                what: DoneWhat::Contract,
                result,
                ask: None,
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A registration sent without an answer and not seen, kept as not known (r1 Must 1),
    /// as if it ended `ago` (and a first «no» `no_ago` ago).
    fn waiting(ago: Duration, no_ago: Option<Duration>) -> sent::Unclear {
        let unclear = sent::Unclear::default();
        let _ = unclear.not_seen(sent::Pending::new(
            "me".into(),
            sent::Watch::Contract {
                owner: "me".into(),
                nonce: 7,
            },
            None,
            None,
            None,
        ));
        let mut p = unclear.contract_of("me").unwrap();
        unclear.take(p.ticket());
        p.aged(ago, no_ago);
        unclear.keep(p);
        unclear
    }

    fn found(n: u64) -> impl std::future::Future<Output = Result<Option<u64>, String>> {
        std::future::ready(Ok(Some(n)))
    }

    fn not_found(_: u64) -> impl std::future::Future<Output = Result<Option<u64>, String>> {
        std::future::ready(Ok(None))
    }

    /// r2: a send that may have gone out, whose read after failed too (the commonest case on a
    /// bad network) — kept as not known like «not found»: the next registration waits. One
    /// refused before it went out is a failure, and a repeat may go.
    #[tokio::test]
    async fn a_failed_read_after_a_send_keeps_it_waiting() {
        let unclear = sent::Unclear::default();
        let e = kept_if_unseen(
            &unclear,
            "me",
            RegisterError::Unclear {
                nonce: 7,
                sent: "no complete response within 20s".into(),
                read: "transport error".into(),
            },
        );
        assert_eq!(e, CoreError::Unclear);
        assert_eq!(prior(&unclear, "me", found).await, Prior::Waits);
        let refused = sent::Unclear::default();
        let e = kept_if_unseen(
            &refused,
            "me",
            RegisterError::NotFound {
                nonce: 7,
                sent: "state transition broadcast error: refused".into(),
            },
        );
        assert!(matches!(e, CoreError::Failed(_)));
        assert_eq!(prior(&refused, "me", found).await, Prior::None);
    }

    /// BUG-243 for a registration: within the quiet a repeat waits (and nothing is read); past
    /// it a read that finds it ends the wait as done — one write; not found, a first «no» still
    /// waits, a second [`sent::NO_GAP`] later lets a repeat go.
    #[tokio::test]
    async fn an_unseen_registration_is_waited_for_then_found_or_let_go() {
        assert_eq!(
            prior(&sent::Unclear::default(), "me", found).await,
            Prior::None
        );
        // Within the quiet: waits, not read.
        let unclear = waiting(Duration::from_secs(10), None);
        let read = std::cell::Cell::new(false);
        let r = prior(&unclear, "me", |n| {
            read.set(true);
            found(n)
        })
        .await;
        assert_eq!(r, Prior::Waits);
        assert!(!read.get());
        // Past it, found: the registration that landed (its nonce read), no longer waiting.
        let unclear = waiting(sent::QUIET, None);
        assert_eq!(prior(&unclear, "me", found).await, Prior::Found(7));
        assert!(unclear.contract_of("me").is_none());
        // Past it, not found: a first «no» waits...
        let unclear = waiting(sent::QUIET, None);
        assert_eq!(prior(&unclear, "me", not_found).await, Prior::Waits);
        assert!(unclear.contract_of("me").unwrap().no_for().is_some());
        // ... a second, far enough after, lets a repeat go.
        let unclear = waiting(sent::QUIET + sent::NO_GAP, Some(sent::NO_GAP));
        assert_eq!(prior(&unclear, "me", not_found).await, Prior::None);
        assert!(unclear.contract_of("me").is_none());
        // Another identity's waiting registration does not hold this one.
        assert_eq!(
            prior(&waiting(Duration::ZERO, None), "other", found).await,
            Prior::None
        );
    }

    /// A fee shown is never below what it is: credits are rounded up to duffs.
    #[test]
    fn the_fee_is_rounded_up_to_duffs() {
        let fee = fee_of(&Fee {
            base: 10_000_000_000,
            storage: 414_290_001,
            per_keyword: 10_000_000_000,
            ..Fee::default()
        });
        assert_eq!(fee.base, 10_000_000);
        assert_eq!(fee.storage, 414_291);
        assert_eq!(fee.per_keyword, 10_000_000);
        assert_eq!(fee.total(), 10_414_291);
    }

    /// Each refusal before the password is typed for the screen; the rest are errors.
    #[test]
    fn refusals_are_typed_and_failures_are_errors() {
        assert_eq!(refusal_of(RegisterError::NoKey), Ok(ContractRefusal::NoKey));
        assert_eq!(
            refusal_of(RegisterError::NewerNetwork(16)),
            Ok(ContractRefusal::NewerNetwork(16))
        );
        assert_eq!(
            refusal_of(RegisterError::NotEnough {
                have: 1_999,
                need: 2_001
            }),
            Ok(ContractRefusal::NotEnoughCredits { have: 1, need: 3 })
        );
        assert_eq!(
            refusal_of(RegisterError::Refused(Refusal::Keyword(
                KeywordRefusal::TooMany(6)
            ))),
            Ok(ContractRefusal::TooManyKeywords(6))
        );
        assert_eq!(
            refusal_of(RegisterError::Refused(Refusal::DescriptionLength(2))),
            Ok(ContractRefusal::DescriptionLength(2))
        );
        assert_eq!(
            refusal_of(RegisterError::VersionUnknown),
            Err(CoreError::NotSynced)
        );
        assert_eq!(
            refusal_of(RegisterError::Unclear {
                nonce: 7,
                sent: "lost".into(),
                read: "lost".into()
            }),
            Err(CoreError::Unclear)
        );
        assert_eq!(
            registration_error(RegisterError::NotEnough { have: 0, need: 1 }),
            CoreError::NotEnough
        );
    }

    /// The screen's draft is the profile's collection with its keywords.
    #[test]
    fn the_draft_is_the_profiles_collection() {
        let d = draft_of(&ContractDraft::NftCollection {
            description: Some("Mine".into()),
            keywords: vec!["NFT".into()],
        });
        assert!(d.readonly);
        assert_eq!(d.description.as_deref(), Some("Mine"));
        assert_eq!(d.keywords, ["NFT"]);
        assert!(d.documents.get("nft").is_some());
    }
}
