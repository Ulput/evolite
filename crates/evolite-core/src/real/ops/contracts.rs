//! A contract the wallet registers itself (FEAT-0038 module V1, FEAT-0019 T-4): its checks
//! before the password and its registration, by `evolite_chain::contract_v1`.

use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::accessors::v1::DataContractV1Getters;
use dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use evolite_api::contracts::{
    ABOUT_NOT_WRITTEN, ABOUT_UNCLEAR, ContractCheck, ContractDraft, ContractFee, ContractRefusal,
};
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
            ..
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
        about: 0,
    }
}

/// The fee shown for `draft`: the registration's, and the «About the collection» document's
/// estimate when one is to be written after it (W2).
fn fee_for(fee: &Fee, draft: &ContractDraft) -> ContractFee {
    let mut shown = fee_of(fee);
    if main_of(draft).is_some() {
        shown.about = duffs(evolite_chain::nft_series::plan::per_write::MINT_FIRST);
    }
    shown
}

/// «Belongs to a contract» of `draft`, if any.
fn main_of(draft: &ContractDraft) -> Option<&str> {
    match draft {
        ContractDraft::NftCollection { main_contract, .. } => main_contract.as_deref(),
    }
}

/// «Belongs to a contract» checked with a proof (W2): its description when it is there, the
/// refusal when it is not an id or not on Platform; a failed read is an error, never «not
/// there».
async fn main_contract(
    chain: &Chain,
    main: &str,
) -> Result<Result<(Identifier, Option<String>), ContractRefusal>, CoreError> {
    let Ok(id) = Identifier::from_string(main.trim(), Encoding::Base58) else {
        return Ok(Err(ContractRefusal::MainContractId));
    };
    match chain.fresh_proved(id).await {
        Ok(found) => Ok(Ok((id, found.contract.description().cloned()))),
        Err(e) if e == evolite_chain::contracts::fresh::NOT_ON_PLATFORM => {
            Ok(Err(ContractRefusal::MainContractMissing))
        }
        Err(e) => Err(CoreError::Failed(e)),
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

/// The key of the History line of `identity`'s registration with `nonce` sent without an
/// answer (R-15, BUG-243): «in transit» until it is found or known not to have landed.
fn sent_key(identity: &str, nonce: u64) -> String {
    format!("contract-sent:{identity}:{nonce}")
}

/// The collection's name for its History line: its description.
fn name_of(draft: &ContractDraft) -> String {
    match draft {
        // default-ok: an optional description: none is empty
        ContractDraft::NftCollection { description, .. } => description.clone().unwrap_or_default(),
    }
}

/// The History line of a registration (R-15): «Collection “{name}” created», what it took
/// (`charged`, else the estimate), keyed by its contract.
fn created_line(
    draft: &ContractDraft,
    identity: &str,
    contract: &str,
    charged: Option<Duffs>,
    estimate: Duffs,
) -> journal::Entry {
    let kind = journal::Kind::CollectionCreated {
        name: name_of(draft),
        contract: Some(contract.to_owned()),
        sending: false,
    };
    let mut entry = journal::entry_of(
        kind,
        now(),
        charged.unwrap_or(estimate),
        Some(identity.into()),
    );
    entry.key = Some(format!("contract:{contract}"));
    entry.cost_known = charged.is_some();
    entry
}

/// [`prior`], with the earlier registration's «in transit» line settled (R-15): found — the
/// line names its contract and is no longer in transit; known not to have landed — the line
/// goes (nothing was paid).
async fn earlier<F, Fut>(
    unclear: &sent::Unclear,
    journal: &journal::Journal,
    identity: &str,
    find: F,
) -> Prior<Arc<dpp::data_contract::DataContract>>
where
    F: FnOnce(u64) -> Fut,
    Fut:
        std::future::Future<Output = Result<Option<Arc<dpp::data_contract::DataContract>>, String>>,
{
    // W0–W4 r2 Must 1: what was not known lives in memory; after a restart the History line
    // «in transit» brings it back, with its time — a repeat waits as it did before.
    if unclear.contract_of(identity).is_none()
        && let Some((nonce, at)) = in_transit(journal, identity)
    {
        // default-ok: clock behind the line's time: sent just now
        let ago = Duration::from_secs(u64::try_from(now() - at).unwrap_or(0));
        let watch = sent::Watch::Contract {
            owner: identity.to_owned(),
            nonce,
        };
        let pending = sent::Pending::new(identity.to_owned(), watch, None, None, None);
        unclear.restore(pending, ago);
    }
    let waiting = unclear.contract_of(identity).and_then(|p| match p.watch {
        sent::Watch::Contract { nonce, .. } => Some(nonce),
        _ => None,
    });
    let answer = prior(unclear, identity, find).await;
    if let Some(nonce) = waiting {
        let key = sent_key(identity, nonce);
        match &answer {
            Prior::Found(c) => {
                let id = c.id().to_string(Encoding::Base58);
                journal.settle(&key, |e| {
                    if let Some(journal::Kind::CollectionCreated {
                        contract, sending, ..
                    }) = &mut e.kind
                    {
                        *contract = Some(id.clone());
                        *sending = false;
                    }
                    e.key = Some(format!("contract:{id}"));
                    true
                });
            }
            Prior::None => {
                journal.settle(&key, |_| false);
            }
            Prior::Waits => {}
        }
    }
    answer
}

/// The latest registration of `identity` still «in transit» in History: its nonce (from the
/// line's key) and when it was sent.
fn in_transit(journal: &journal::Journal, identity: &str) -> Option<(u64, Timestamp)> {
    let prefix = format!("contract-sent:{identity}:");
    journal
        .entries()
        .into_iter()
        .filter(|e| {
            matches!(
                e.kind,
                Some(journal::Kind::CollectionCreated { sending: true, .. })
            )
        })
        .filter_map(|e| {
            let nonce = e.key.as_deref()?.strip_prefix(&prefix)?.parse().ok()?;
            Some((nonce, e.at))
        })
        .max_by_key(|(nonce, _)| *nonce)
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
        let journal = self.journal.clone();
        self.answer_later("task:check_contract", async move {
            let result = async {
                let wallet = chain
                    .wallet(&id)
                    .await
                    .ok_or(CoreError::Failed(NOT_LOADED.into()))?;
                // r1 Must 1: an earlier registration that may still land is waited for.
                let find = |n| chain.registered_after(&owner, n);
                match earlier(&unclear, &journal, &identity, find).await {
                    Prior::Waits => {
                        return Ok(ContractCheck {
                            fee: None,
                            refusal: Some(ContractRefusal::OnItsWay),
                            main_description: None,
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
                let mut refusal = checked.refusal.map(refusal_of).transpose()?;
                let mut main_description = None;
                if let Some(main) = main_of(&draft) {
                    match main_contract(&chain, main).await? {
                        Ok((_, description)) => main_description = description,
                        Err(why) => refusal = refusal.or(Some(why)),
                    }
                }
                Ok(ContractCheck {
                    fee: checked.fee.as_ref().map(|f| fee_for(f, &draft)),
                    refusal,
                    main_description,
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
        let journal = self.journal.clone();
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
                let find = |n| chain.registered_after(&owner, n);
                match earlier(&unclear, &journal, &identity, find).await {
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
                // W2: the contract it belongs to, checked again before anything is paid.
                let main = match main_of(&draft) {
                    Some(main) => match main_contract(&chain, main).await? {
                        Ok((id, _)) => Some(id),
                        // r1 Could 3: said in words, not as the enum's name.
                        Err(ContractRefusal::MainContractId) => {
                            return Err(CoreError::NotAvailable(
                                tr!("This is not a contract id.").into(),
                            ));
                        }
                        Err(_) => {
                            return Err(CoreError::NotAvailable(
                                tr!("There is no contract with this id on Dash Platform.").into(),
                            ));
                        }
                    },
                    None => None,
                };
                // FEAT-0004: what the person agreed to (and the tick counted) is the most it
                // takes; a fee grown past it since the check is refused before signing.
                let checked = chain
                    .check_contract(&wallet, &keys, &owner, &chain_draft)
                    .await;
                // What the History line says until the registration's charge is read.
                let estimate = checked
                    .fee
                    .as_ref()
                    .map_or(fee_max, |f| fee_for(f, &draft).total());
                if let Some(fee) = checked.fee
                    && fee_for(&fee, &draft).total() > fee_max
                {
                    return Err(CoreError::Failed(format!(
                        "the fee grew: {} over {fee_max}",
                        fee_for(&fee, &draft).total()
                    )));
                }
                let done = match chain
                    .register_contract(&wallet, &keys, &owner, &chain_draft)
                    .await
                {
                    Ok(done) => done,
                    Err(e) => {
                        // R-15, BUG-243: a registration that may have gone out has its line
                        // «in transit», at its estimate, until it is found or known not to have
                        // landed.
                        let nonce = match &e {
                            RegisterError::NotFound { nonce, sent }
                            | RegisterError::Unclear { nonce, sent, .. }
                                if !sent::refused_before(sent) =>
                            {
                                Some(*nonce)
                            }
                            _ => None,
                        };
                        let error = kept_if_unseen(&unclear, &identity, e);
                        if let Some(nonce) = nonce {
                            let kind = journal::Kind::CollectionCreated {
                                name: name_of(&draft),
                                contract: None,
                                sending: true,
                            };
                            let mut line =
                                journal::entry_of(kind, now(), estimate, Some(identity.clone()));
                            line.key = Some(sent_key(&identity, nonce));
                            line.cost_known = false;
                            journal.add(line);
                        }
                        return Err(error);
                    }
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
                // The registration kept the contract in the cache and the provider: «My
                // collections» list it, and «About» is written without reading it again.
                let contract = done.contract.id();
                let id = contract.to_string(Encoding::Base58);
                // R-15 (r1 Must 2): its History line, with what it took.
                journal.add(created_line(
                    &draft,
                    &identity,
                    &id,
                    done.charged.map(duffs),
                    estimate,
                ));
                let _ = out.send(Event::ContractRegistered {
                    identity: identity.clone(),
                    id,
                    charged: done.charged.map(duffs),
                });
                // W2: «About the collection», once the collection is there. Not written, the
                // collection stays made: its `Done` says so, after the registration above. A
                // write that may have gone out is «not known» (r1 Should 3, BUG-243), never
                // «not written»; it is not repeated.
                if let Some(main) = main {
                    chain
                        .nft_collection_info(&wallet, &keys, &owner, contract, main)
                        .await
                        .map_err(|e| {
                            let head = if sent::refused_before(&e) {
                                ABOUT_NOT_WRITTEN
                            } else {
                                ABOUT_UNCLEAR
                            };
                            CoreError::Failed(format!("{head}: {e}"))
                        })?;
                }
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
            main_contract: None,
        });
        assert!(d.readonly);
        assert_eq!(d.description.as_deref(), Some("Mine"));
        assert_eq!(d.keywords, ["NFT"]);
        assert!(d.documents.get("nft").is_some());
    }

    /// W2: «Belongs to a contract» adds its document's estimate to the total the tick counts;
    /// without it, nothing is added and the registration is as before.
    #[test]
    fn the_about_document_is_in_the_fee_only_when_asked() {
        let fee = Fee {
            base: 10_000_000_000,
            ..Fee::default()
        };
        let draft = |main: Option<&str>| ContractDraft::NftCollection {
            description: Some("Mine".into()),
            keywords: vec![],
            main_contract: main.map(str::to_owned),
        };
        let without = fee_for(&fee, &draft(None));
        assert_eq!(without.about, 0);
        let with = fee_for(&fee, &draft(Some("c")));
        assert_eq!(
            with.about,
            evolite_chain::nft_series::plan::per_write::MINT_FIRST.div_ceil(1_000) as Duffs
        );
        assert_eq!(with.total(), without.total() + with.about);
        assert_eq!(with.registration(), without.registration());
    }

    fn journal_in(name: &str) -> journal::Journal {
        let dir =
            std::env::temp_dir().join(format!("evolite-contracts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        journal::Journal::new(dir.join("platform-history.json"))
    }

    fn sent_line() -> journal::Entry {
        let kind = journal::Kind::CollectionCreated {
            name: "Pins".into(),
            contract: None,
            sending: true,
        };
        let mut line = journal::entry_of(kind, 1, 19_000_000, Some("me".into()));
        line.key = Some(sent_key("me", 7));
        line.cost_known = false;
        line
    }

    /// R-15 (W0–W4 r1 Must 2), BUG-243: a registration sent without an answer has its History
    /// line «in transit»; found after its quiet, the line names the contract and is in transit no
    /// more; known not to have landed (two «no» apart), the line goes — nothing was paid.
    #[tokio::test]
    async fn a_registration_in_transit_is_settled_in_history() {
        let owner = Identifier::new([3; 32]);
        let built = evolite_chain::contract_v1::build(
            &templates::nft_collection(Some("Pins".into())),
            owner,
            dpp::version::PlatformVersion::latest(),
        )
        .unwrap();
        let contract = Arc::new(built.contract);
        let id = contract.id().to_string(Encoding::Base58);

        let journal = journal_in("found");
        journal.add(sent_line());
        let unclear = waiting(sent::QUIET + Duration::from_secs(1), None);
        let found = {
            let c = contract.clone();
            move |_| std::future::ready(Ok(Some(c)))
        };
        assert!(matches!(
            earlier(&unclear, &journal, "me", found).await,
            Prior::Found(_)
        ));
        let lines = journal.entries();
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0].key.as_deref(),
            Some(format!("contract:{id}").as_str())
        );
        assert_eq!(
            lines[0].kind,
            Some(journal::Kind::CollectionCreated {
                name: "Pins".into(),
                contract: Some(id.clone()),
                sending: false,
            })
        );

        let journal = journal_in("gone");
        journal.add(sent_line());
        let unclear = waiting(
            sent::QUIET + Duration::from_secs(1),
            Some(sent::NO_GAP + Duration::from_secs(1)),
        );
        let none = |_| std::future::ready(Ok(None));
        assert_eq!(earlier(&unclear, &journal, "me", none).await, Prior::None);
        assert!(journal.entries().is_empty(), "not landed: no line");
    }

    /// W0–W4 r2 Must 1: after a restart nothing of a registration in transit is in memory; its
    /// History line brings it back — within its quiet a repeat waits (the check says «on its
    /// way»), past it «found» settles the line.
    #[tokio::test]
    async fn a_registration_in_transit_survives_a_restart() {
        let owner = Identifier::new([4; 32]);
        let built = evolite_chain::contract_v1::build(
            &templates::nft_collection(Some("Pins".into())),
            owner,
            dpp::version::PlatformVersion::latest(),
        )
        .unwrap();
        let contract = Arc::new(built.contract);
        let found = {
            let c = contract.clone();
            move |_| std::future::ready(Ok(Some(c)))
        };
        // Sent ten seconds ago, then the app restarted: a new, empty `Unclear`.
        let journal = journal_in("restart-quiet");
        let mut line = sent_line();
        line.at = now() - 10;
        journal.add(line);
        let unclear = sent::Unclear::default();
        assert_eq!(
            earlier(&unclear, &journal, "me", found.clone()).await,
            Prior::Waits
        );
        assert!(unclear.contract_of("me").is_some(), "brought back");
        assert!(
            matches!(
                journal.entries()[0].kind,
                Some(journal::Kind::CollectionCreated { sending: true, .. })
            ),
            "still in transit"
        );
        // Sent five minutes ago: past its quiet, the registration is found and its line settled.
        let journal = journal_in("restart-found");
        let mut line = sent_line();
        line.at = now() - 300;
        journal.add(line);
        let unclear = sent::Unclear::default();
        assert!(matches!(
            earlier(&unclear, &journal, "me", found).await,
            Prior::Found(_)
        ));
        assert!(matches!(
            journal.entries()[0].kind,
            Some(journal::Kind::CollectionCreated { sending: false, .. })
        ));
    }

    /// R-15: the line of a registration that answered — its name, what it took (else the
    /// estimate, its cost then not known), keyed by its contract.
    #[test]
    fn a_registration_has_its_line() {
        let draft = ContractDraft::NftCollection {
            description: Some("Pins".into()),
            keywords: vec![],
            main_contract: None,
        };
        let line = created_line(&draft, "me", "C1", Some(12), 19);
        assert_eq!((line.spent, line.cost_known), (12, true));
        assert_eq!(line.key.as_deref(), Some("contract:C1"));
        let line = created_line(&draft, "me", "C1", None, 19);
        assert_eq!((line.spent, line.cost_known), (19, false));
    }
}
