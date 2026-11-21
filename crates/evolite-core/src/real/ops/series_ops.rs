//! The «Workshop» series' commands (FEAT-0038 item 1): «Check» before the password, the series
//! itself, «Continue», «Stop after the current ones», the plate's «×»; and the chain as a
//! series' network. The series runs in [`super::series`].

use evolite_api::series::{
    ItemState, ModelWrite, NewModel, PictureCheck, SeriesKind, SeriesPlan, SeriesRefusal,
    SeriesState, SeriesTarget, has_link,
};
use evolite_chain::nft_series::image::{self, ImageRefusal, ImageType, Packed};
use evolite_chain::nft_series::plan::{self, Instance, MAX_PAGES, Pick, per_write};
use evolite_chain::nft_series::strip::strip_metadata;
use evolite_local::nft_series::{Item, Kind, Series, State};

use super::names::recipient_id;
use super::series::{Ended, Fut, SeriesNet, Write, run};
use crate::real::*;

/// A page of `byImageSerial` as the network gives it.
const PAGE: u32 = 100;

/// The chain as the network of one series.
pub(in crate::real) struct ChainNet {
    chain: Arc<Chain>,
    wallet: Arc<platform_wallet::PlatformWallet>,
    keys: Arc<key_wallet::Wallet>,
    owner: Identifier,
    contract: Identifier,
    model: Identifier,
}

impl SeriesNet for ChainNet {
    fn credits(&self) -> Fut<Result<u64, String>> {
        let (chain, wallet, owner) = (self.chain.clone(), self.wallet.clone(), self.owner);
        Box::pin(async move {
            chain
                .identity_credits(&wallet, &owner, true)
                .await
                .ok_or_else(|| "the credits were not read".to_owned())
        })
    }

    fn read(&self, serial: u32, doc: Option<Identifier>) -> Fut<Result<Option<Instance>, String>> {
        let (chain, contract, model) = (self.chain.clone(), self.contract, self.model);
        Box::pin(async move { chain.nft_instance(contract, model, doc, serial).await })
    }

    fn write(
        &self,
        write: Write,
        serial: u32,
        doc: Option<Identifier>,
    ) -> Fut<Result<Option<Identifier>, String>> {
        let (chain, wallet, keys) = (self.chain.clone(), self.wallet.clone(), self.keys.clone());
        let (owner, contract, model) = (self.owner, self.contract, self.model);
        Box::pin(async move {
            let doc_of = || doc.ok_or_else(|| "no document".to_owned());
            match write {
                Write::Mint => chain
                    .nft_mint(&wallet, &keys, &owner, contract, model, serial)
                    .await
                    .map(Some),
                Write::Price(price) => chain
                    .nft_price(&wallet, &keys, &owner, contract, doc_of()?, price)
                    .await
                    .map(|_| None),
                Write::Transfer(to) => chain
                    .nft_transfer(&wallet, &keys, &owner, contract, doc_of()?, to)
                    .await
                    .map(|_| None),
                Write::Delete => chain
                    .nft_delete(&wallet, &keys, &owner, contract, doc_of()?)
                    .await
                    .map(|_| None),
            }
        })
    }

    fn me(&self) -> Identifier {
        self.owner
    }
}

/// Credits as duffs, rounded up: an estimate shown is never below what it is.
fn duffs(credits: u64) -> Duffs {
    Duffs::try_from(credits.div_ceil(1_000)).unwrap_or(Duffs::MAX)
}

fn ids(t: &SeriesTarget) -> Result<(Identifier, Identifier, Identifier), CoreError> {
    let id =
        |s: &str| Identifier::from_string(s, Encoding::Base58).map_err(|_| CoreError::BadAddress);
    Ok((id(&t.identity)?, id(&t.contract)?, id(&t.model)?))
}

/// One write's estimate for `kind`, in credits.
fn per_write_of(kind: &SeriesKind) -> u64 {
    match kind {
        SeriesKind::Mint { .. } => per_write::MINT,
        SeriesKind::Price { .. } | SeriesKind::Reprice { .. } => per_write::PRICE,
        SeriesKind::Gift { .. } | SeriesKind::Unprice { .. } => per_write::TRANSFER,
        SeriesKind::Delete { .. } => per_write::DELETE,
    }
}

/// What a plan found: the plan the screen sees, the instances it takes (serial, document) and
/// a gift's recipient.
struct Planned {
    plan: SeriesPlan,
    items: Vec<(u32, Option<Identifier>)>,
    to: Option<Identifier>,
}

fn refused(refusal: SeriesRefusal) -> Planned {
    Planned {
        plan: SeriesPlan {
            refusal: Some(refusal),
            ..SeriesPlan::default()
        },
        items: Vec::new(),
        to: None,
    }
}

/// «Check» of a series, read fresh (R-7, R-10): the refusals before the password, then the
/// serials and the fee.
async fn plan_of(
    chain: &Arc<Chain>,
    wallet: &Arc<platform_wallet::PlatformWallet>,
    keys: &key_wallet::Wallet,
    issued: Option<u32>,
    busy: bool,
    target: &SeriesTarget,
    kind: &SeriesKind,
) -> Result<Planned, CoreError> {
    let (owner, contract, model) = ids(target)?;
    if busy {
        return Ok(refused(SeriesRefusal::SeriesRunning));
    }
    if !Chain::nft_keys_ready(wallet, keys, &owner).await {
        return Ok(refused(SeriesRefusal::NoKey));
    }
    if matches!(
        kind,
        SeriesKind::Price { price: 0, .. } | SeriesKind::Reprice { price: 0, .. }
    ) {
        return Ok(refused(SeriesRefusal::ZeroPrice));
    }
    let mut to = None;
    if let SeriesKind::Gift { to: who, .. } = kind {
        let Ok(found) = recipient_id(chain, who).await else {
            return Ok(refused(SeriesRefusal::NoRecipient));
        };
        let found =
            Identifier::from_string(&found, Encoding::Base58).map_err(|_| CoreError::BadAddress)?;
        if found == owner {
            return Ok(refused(SeriesRefusal::ToSelf));
        }
        to = Some(found);
    }
    let failed = |e: String| CoreError::Failed(e);
    let items: Vec<(u32, Option<Identifier>)> = match kind {
        SeriesKind::Mint { count } => {
            let supply = chain
                .nft_model_supply(contract, model)
                .await
                .map_err(failed)?
                .ok_or(CoreError::Failed("no such model".into()))?;
            let top = chain
                .nft_max_serial(contract, model)
                .await
                .map_err(failed)?;
            match plan::next_serials(issued, top, *count, supply) {
                Ok(serials) => serials.into_iter().map(|s| (s, None)).collect(),
                Err(left) => return Ok(refused(SeriesRefusal::PastSupply { left })),
            }
        }
        _ => {
            let (pick, want) = match kind {
                SeriesKind::Price { count, .. }
                | SeriesKind::Gift { count, .. }
                | SeriesKind::Delete { count } => (Pick::Free, *count as usize),
                SeriesKind::Unprice { count, price } => (Pick::OnSale(*price), *count as usize),
                SeriesKind::Reprice { from, .. } => (Pick::OnSale(Some(*from)), usize::MAX),
                SeriesKind::Mint { .. } => unreachable!(),
            };
            let mut taken: Vec<Instance> = Vec::new();
            let mut after = None;
            let mut ended = false;
            for _ in 0..MAX_PAGES {
                let page = chain
                    .nft_serial_page(contract, model, after, pick.descending(), PAGE)
                    .await
                    .map_err(failed)?;
                let enough = plan::take_from_page(&page, pick, &owner, want, &mut taken);
                ended = (page.len() as u32) < PAGE;
                after = page.last().map(|i| i.serial);
                if enough || ended {
                    break;
                }
            }
            if taken.is_empty() && matches!(kind, SeriesKind::Reprice { .. }) {
                return Ok(refused(SeriesRefusal::NoneAtPrice));
            }
            if want != usize::MAX && taken.len() < want {
                let found = taken.len() as u32;
                return Ok(refused(if ended {
                    SeriesRefusal::NotEnoughInstances { have: found }
                } else {
                    SeriesRefusal::TooFewFound { found }
                }));
            }
            taken.into_iter().map(|i| (i.serial, Some(i.id))).collect()
        }
    };
    let one = per_write_of(kind);
    let credits = chain
        .identity_credits(wallet, &owner, true)
        .await
        .ok_or(CoreError::Failed("the credits were not read".into()))?;
    let total = one.saturating_mul(items.len() as u64);
    let refusal = (credits < total).then(|| SeriesRefusal::NotEnoughCredits {
        have: Duffs::try_from(credits / 1_000).unwrap_or(Duffs::MAX),
        need: duffs(total),
    });
    Ok(Planned {
        plan: SeriesPlan {
            serials: items.iter().map(|(s, _)| *s).collect(),
            per_write: duffs(one),
            total: duffs(total),
            credits: Duffs::try_from(credits / 1_000).unwrap_or(Duffs::MAX),
            refusal,
            model: None,
            picture: None,
        },
        items,
        to,
    })
}

/// The kind kept on disk.
fn kind_of(kind: &SeriesKind) -> Kind {
    match kind {
        SeriesKind::Mint { .. } => Kind::Mint,
        SeriesKind::Price { price, .. } => Kind::Price { price: *price },
        SeriesKind::Reprice { from, price } => Kind::Reprice {
            from: *from,
            price: *price,
        },
        SeriesKind::Gift { to, .. } => Kind::Gift { to: to.clone() },
        SeriesKind::Unprice { .. } => Kind::Unprice,
        SeriesKind::Delete { .. } => Kind::Delete,
    }
}

/// The kind as the screens know it, from the disk's.
fn kind_back(kind: &Kind, count: u32) -> SeriesKind {
    match kind {
        Kind::Mint => SeriesKind::Mint { count },
        Kind::Price { price } => SeriesKind::Price {
            count,
            price: *price,
        },
        Kind::Reprice { from, price } => SeriesKind::Reprice {
            from: *from,
            price: *price,
        },
        Kind::Gift { to } => SeriesKind::Gift {
            count,
            to: to.clone(),
        },
        Kind::Unprice => SeriesKind::Unprice { count, price: None },
        Kind::Delete => SeriesKind::Delete { count },
    }
}

/// What a series run starts from.
pub(in crate::real) enum Start {
    /// A new series on a model.
    New(SeriesTarget, SeriesKind),
    /// «Continue» series `id`.
    Continue(u64),
    /// A new model and its first instances (UC-5).
    Model {
        identity: String,
        contract: String,
        model: NewModel,
        mint: u32,
    },
}

/// A new model's picture as it will be written (R-21, R-22): its metadata taken out (refused
/// when the file's structure cannot be followed: never written as it is), packed, held to
/// `room`. The check is there whenever the picture could be read, refused or not; `stripped`
/// is what is weighed, drawn and written.
struct Picture {
    check: Option<PictureCheck>,
    stripped: Vec<u8>,
    packed: Result<Packed, SeriesRefusal>,
}

fn check_picture(bytes: &[u8], room: usize) -> Picture {
    let refused = |r| Picture {
        check: None,
        stripped: Vec::new(),
        packed: Err(r),
    };
    let Some(kind) = image::image_type(bytes) else {
        return refused(SeriesRefusal::ImageType);
    };
    let Ok(stripped) = strip_metadata(bytes) else {
        return refused(SeriesRefusal::ImageUnreadable);
    };
    let packed = image::pack(&stripped.bytes).map_err(|e| match e {
        ImageRefusal::Unknown | ImageRefusal::RoundTrip => SeriesRefusal::ImageType,
        ImageRefusal::Unsafe(why) => SeriesRefusal::ImageUnsafe(why.into()),
        ImageRefusal::TooBig(bytes) => SeriesRefusal::ImageTooBig(bytes),
        ImageRefusal::TooLargeUnpacked => SeriesRefusal::ImageTooBig(stripped.bytes.len()),
    });
    // The room of this write, after the metadata went (R-22): a file that fits only once
    // stripped fits.
    let packed = packed.and_then(|p| {
        if p.stored() > room {
            Err(SeriesRefusal::ImageTooBig(p.stored()))
        } else {
            Ok(p)
        }
    });
    let size = match &packed {
        Ok(p) => p.stored(),
        Err(SeriesRefusal::ImageTooBig(n)) => *n,
        Err(_) => stripped.bytes.len(),
    };
    let raster = matches!(
        kind,
        ImageType::Jpeg | ImageType::Png | ImageType::Gif | ImageType::Webp
    );
    let check = PictureCheck {
        art: String::new(),
        size,
        room,
        removed: stripped.removed,
        glb: kind == ImageType::Glb,
        can_fit: raster && matches!(packed, Err(SeriesRefusal::ImageTooBig(_))),
    };
    Picture {
        check: Some(check),
        stripped: stripped.bytes,
        packed,
    }
}

/// What a new model is refused for without the network (UC-5, R-4, R-5, R-6): its texts and its
/// supply; the picture is [`check_picture`]'s.
fn check_texts(model: &NewModel, mint: u32) -> Result<(), SeriesRefusal> {
    let name = model.name.chars().count();
    if !(1..=64).contains(&name) {
        return Err(SeriesRefusal::NameLength(name));
    }
    let description = model.description.chars().count();
    if description > 280 {
        return Err(SeriesRefusal::DescriptionLength(description));
    }
    if has_link(&model.name) || has_link(&model.description) {
        return Err(SeriesRefusal::Link);
    }
    if model.supply == 0 || mint > model.supply {
        return Err(SeriesRefusal::Supply);
    }
    Ok(())
}

/// The texts, then the picture in the full room: the checks that need no network.
#[cfg(test)]
fn check_new_model(model: &NewModel, mint: u32) -> Result<Packed, SeriesRefusal> {
    check_texts(model, mint)?;
    check_picture(&model.image, image::PARTS_TOTAL).packed
}

/// «Check» of a new model (UC-5): the refusals before the password, its picture packed, its
/// fee with `mint` instances.
#[allow(clippy::too_many_arguments)]
async fn plan_model_of(
    chain: &Arc<Chain>,
    wallet: &Arc<platform_wallet::PlatformWallet>,
    keys: &key_wallet::Wallet,
    artist: Option<&crate::art::Artist>,
    busy: bool,
    identity: &str,
    model: &NewModel,
    mint: u32,
) -> Result<(SeriesPlan, Option<Packed>), CoreError> {
    let owner =
        Identifier::from_string(identity, Encoding::Base58).map_err(|_| CoreError::BadAddress)?;
    let no = |r, picture| {
        Ok((
            SeriesPlan {
                refusal: Some(r),
                picture,
                ..SeriesPlan::default()
            },
            None,
        ))
    };
    if busy {
        return no(SeriesRefusal::SeriesRunning, None);
    }
    if let Err(r) = check_texts(model, mint) {
        return no(r, None);
    }
    // R-21: the room of this very write, measured for its name and description.
    let kind = image::image_type(&model.image).unwrap_or(ImageType::Png);
    let room = chain
        .nft_picture_room(&model.name, &model.description, model.supply, kind)
        .await
        .map_err(CoreError::Failed)?;
    let picture = check_picture(&model.image, room);
    // What W5 shows is what is written: the stripped picture, kept to draw.
    let check = picture.check.map(|mut check| {
        if let Some(artist) = artist {
            check.art = artist.keep(picture.stripped.clone().into());
        }
        check
    });
    let packed = match picture.packed {
        Ok(p) => p,
        Err(r) => return no(r, check),
    };
    if !Chain::nft_keys_ready(wallet, keys, &owner).await {
        return no(SeriesRefusal::NoKey, check);
    }
    let image_fee = per_write::image(packed.stored());
    let total = image_fee.saturating_add(per_write::MINT.saturating_mul(mint as u64));
    let credits = chain
        .identity_credits(wallet, &owner, true)
        .await
        .ok_or(CoreError::Failed("the credits were not read".into()))?;
    let refusal = (credits < total).then(|| SeriesRefusal::NotEnoughCredits {
        have: Duffs::try_from(credits / 1_000).unwrap_or(Duffs::MAX),
        need: duffs(total),
    });
    Ok((
        SeriesPlan {
            serials: (1..=mint).collect(),
            per_write: duffs(per_write::MINT),
            total: duffs(total),
            credits: Duffs::try_from(credits / 1_000).unwrap_or(Duffs::MAX),
            refusal,
            model: Some(ModelWrite {
                stored: packed.stored(),
                parts: packed.parts.len() as u8,
                fee: duffs(image_fee),
            }),
            picture: check,
        },
        Some(packed),
    ))
}

/// FEAT-0004: what the person agreed to (and the tick counted) is the most a series takes; an
/// estimate grown past it since the check is refused before the first write.
fn grown(total: Duffs, fee_max: Duffs) -> Result<(), CoreError> {
    if total > fee_max {
        return Err(CoreError::Failed(format!(
            "the fee grew: {total} over {fee_max}"
        )));
    }
    Ok(())
}

/// One write's estimate for a series kept on disk, in credits.
fn per_write_kind(kind: &Kind) -> u64 {
    match kind {
        Kind::Mint => per_write::MINT,
        Kind::Price { .. } | Kind::Reprice { .. } => per_write::PRICE,
        Kind::Gift { .. } | Kind::Unprice => per_write::TRANSFER,
        Kind::Delete => per_write::DELETE,
    }
}

/// How a series run's end answers its `Done`.
fn done_of(ended: &Ended) -> Result<(), CoreError> {
    match ended {
        Ended::Finished | Ended::Partial("stopped") => Ok(()),
        Ended::Partial("credits") => Err(CoreError::NotEnough),
        Ended::Partial("unclear") => Err(CoreError::Unclear),
        Ended::Partial("lock") => Err(CoreError::NotAvailable(tr!("The wallet is locked").into())),
        Ended::Partial(why) => Err(CoreError::Failed((*why).into())),
    }
}

impl Real {
    /// The series not closed with «×», as the plates show them: running, stopped with writes
    /// left, or finished («done {n} of {n}», W12) — the new model's id is read from there.
    pub(in crate::real) fn series_states(&self) -> Vec<SeriesState> {
        self.nft_series
            .all()
            .into_iter()
            .filter(|s| !s.closed)
            .map(|s| {
                let count = |state| s.count(state) as u32;
                SeriesState {
                    id: s.id,
                    target: SeriesTarget {
                        identity: s.identity.clone(),
                        contract: s.contract.clone(),
                        model: s.model.clone(),
                    },
                    kind: kind_back(&s.kind, s.items.len() as u32),
                    total: s.items.len() as u32,
                    done: count(State::Done),
                    refused: count(State::Refused),
                    unclear: count(State::Unclear) + count(State::Sending),
                    running: self.series_run.is_running(s.id),
                    stopping: false,
                    per_write: duffs(per_write_kind(&s.kind)),
                }
            })
            .collect()
    }

    /// `Command::PlanSeries`: answered later with `Event::SeriesPlanned`.
    pub(in crate::real) fn plan_series(
        &self,
        target: SeriesTarget,
        kind: SeriesKind,
    ) -> Result<(), CoreError> {
        let (id, keys) = self.dashpay_ready()?;
        let chain = self.chain.clone();
        let issued = self.nft_series.issued(&target.model);
        let busy = self.identity_busy.is_busy();
        self.answer_later("task:plan_series", async move {
            let result = async {
                let wallet = chain
                    .wallet(&id)
                    .await
                    .ok_or(CoreError::Failed(NOT_LOADED.into()))?;
                plan_of(&chain, &wallet, &keys, issued, busy, &target, &kind)
                    .await
                    .map(|p| p.plan)
            }
            .await;
            Event::SeriesPlanned {
                target,
                kind,
                result,
            }
        });
        Ok(())
    }

    /// `Command::PlanModel`: answered later with `Event::ModelPlanned`.
    pub(in crate::real) fn plan_model(
        &self,
        identity: String,
        model: NewModel,
        mint: u32,
    ) -> Result<(), CoreError> {
        let (id, keys) = self.dashpay_ready()?;
        let chain = self.chain.clone();
        let busy = self.identity_busy.is_busy();
        let artist = self.artist.clone();
        self.answer_later("task:plan_model", async move {
            let result = async {
                let wallet = chain
                    .wallet(&id)
                    .await
                    .ok_or(CoreError::Failed(NOT_LOADED.into()))?;
                plan_model_of(
                    &chain,
                    &wallet,
                    &keys,
                    artist.as_ref(),
                    busy,
                    &identity,
                    &model,
                    mint,
                )
                .await
                .map(|(plan, _)| plan)
            }
            .await;
            Event::ModelPlanned { result }
        });
        Ok(())
    }

    /// `Spend::NftBatch`, `ContinueSeries` or `CreateModel`, past the password: the series in
    /// the background, one change of the identity at a time (R-8, AC-T-W5).
    pub(in crate::real) fn run_series(
        &self,
        start: Start,
        fee_max: Duffs,
    ) -> Result<(), CoreError> {
        let Some(busy) = self.identity_busy.take() else {
            return Err(super::identity_keys::identity_busy());
        };
        let (wallet_id, keys) = match self.dashpay_ready() {
            Ok(ready) => ready,
            Err(e) => {
                drop(busy);
                return Err(e);
            }
        };
        let chain = self.chain.clone();
        let store = self.nft_series.clone();
        let runs = self.series_run.clone();
        let refresh = self.refresher();
        self.answer_later("task:nft_series", async move {
            let started = std::time::Instant::now();
            let result = async {
                let wallet = chain
                    .wallet(&wallet_id)
                    .await
                    .ok_or(CoreError::Failed(NOT_LOADED.into()))?;
                // A new series is planned again fresh: what was checked may have changed.
                let (series_id, target, to) = match start {
                    Start::Model {
                        identity,
                        contract,
                        model,
                        mint,
                    } => {
                        let (plan, packed) = plan_model_of(
                            &chain, &wallet, &keys, None, false, &identity, &model, mint,
                        )
                        .await?;
                        if let Some(refusal) = plan.refusal {
                            return Err(CoreError::Failed(format!("{refusal:?}")));
                        }
                        grown(plan.total, fee_max)?;
                        let packed = packed.expect("a plan without a refusal has the picture");
                        let owner = Identifier::from_string(&identity, Encoding::Base58)
                            .map_err(|_| CoreError::BadAddress)?;
                        let contract_id = Identifier::from_string(&contract, Encoding::Base58)
                            .map_err(|_| CoreError::BadAddress)?;
                        // The model first: its instances refer to it. A lost answer here leaves
                        // no series; the model, if it landed, shows in the collection, where
                        // «Issue more» issues its instances.
                        let model_id = chain
                            .nft_create_model(
                                &wallet,
                                &keys,
                                &owner,
                                contract_id,
                                &model.name,
                                &model.description,
                                model.supply,
                                &packed,
                            )
                            .await
                            .map_err(|e| {
                                if crate::real::sent::refused_before(&e) {
                                    CoreError::Failed(e)
                                } else {
                                    CoreError::Unclear
                                }
                            })?;
                        let target = SeriesTarget {
                            identity,
                            contract,
                            model: model_id.to_string(Encoding::Base58),
                        };
                        if mint == 0 {
                            return Ok(());
                        }
                        let series = Series {
                            id: 0,
                            identity: target.identity.clone(),
                            contract: target.contract.clone(),
                            model: target.model.clone(),
                            kind: Kind::Mint,
                            items: (1..=mint)
                                .map(|serial| Item {
                                    serial,
                                    doc: None,
                                    state: State::Waiting,
                                    why: None,
                                    sent_at: None,
                                    no_since: None,
                                })
                                .collect(),
                            created_at: evolite_common::wall_secs(),
                            spent: 0,
                            closed: false,
                        };
                        let id = store.start(series).map_err(CoreError::Failed)?;
                        (id, target, None)
                    }
                    Start::New(target, kind) => {
                        let issued = store.issued(&target.model);
                        let planned =
                            plan_of(&chain, &wallet, &keys, issued, false, &target, &kind).await?;
                        if let Some(refusal) = planned.plan.refusal {
                            return Err(CoreError::Failed(format!("{refusal:?}")));
                        }
                        grown(planned.plan.total, fee_max)?;
                        let series = Series {
                            id: 0,
                            identity: target.identity.clone(),
                            contract: target.contract.clone(),
                            model: target.model.clone(),
                            kind: kind_of(&kind),
                            items: planned
                                .items
                                .iter()
                                .map(|(serial, doc)| Item {
                                    serial: *serial,
                                    doc: doc.map(|d| d.to_string(Encoding::Base58)),
                                    state: State::Waiting,
                                    why: None,
                                    sent_at: None,
                                    no_since: None,
                                })
                                .collect(),
                            created_at: evolite_common::wall_secs(),
                            spent: 0,
                            closed: false,
                        };
                        let id = store.start(series).map_err(CoreError::Failed)?;
                        (id, target, planned.to)
                    }
                    Start::Continue(id) => {
                        let s = store
                            .get(id)
                            .ok_or(CoreError::Failed("no such series".into()))?;
                        let left = s
                            .items
                            .iter()
                            .filter(|i| i.state != State::Done && i.state != State::Refused)
                            .count() as u64;
                        grown(duffs(per_write_kind(&s.kind).saturating_mul(left)), fee_max)?;
                        let target = SeriesTarget {
                            identity: s.identity.clone(),
                            contract: s.contract.clone(),
                            model: s.model.clone(),
                        };
                        let to = match &s.kind {
                            Kind::Gift { to } => Some(
                                Identifier::from_string(
                                    &recipient_id(&chain, to).await?,
                                    Encoding::Base58,
                                )
                                .map_err(|_| CoreError::BadAddress)?,
                            ),
                            _ => None,
                        };
                        (id, target, to)
                    }
                };
                let (owner, contract, model) = ids(&target)?;
                let kind = store.get(series_id).map(|s| s.kind).expect("the series");
                let one = per_write_kind(&kind);
                let before = chain.identity_credits(&wallet, &owner, true).await;
                let net = Arc::new(ChainNet {
                    chain: chain.clone(),
                    wallet: wallet.clone(),
                    keys: Arc::new(keys.clone()),
                    owner,
                    contract,
                    model,
                });
                let ended = run(
                    net,
                    store.clone(),
                    runs,
                    series_id,
                    to,
                    one,
                    evolite_common::wall_secs,
                )
                .await;
                let after = chain.identity_credits(&wallet, &owner, true).await;
                if let (Some(b), Some(a)) = (before, after) {
                    let _ = store.spent(series_id, b.saturating_sub(a).div_ceil(1_000));
                }
                if let Some(s) = store.get(series_id) {
                    crate::diag::record(crate::diag::Record::SeriesEnded {
                        total: s.items.len() as u32,
                        done: s.count(State::Done) as u32,
                        refused: s.count(State::Refused) as u32,
                        unclear: (s.count(State::Unclear) + s.count(State::Sending)) as u32,
                        ms: started.elapsed().as_millis() as u64,
                    });
                }
                done_of(&ended)
            }
            .await;
            drop(busy);
            refresh();
            Event::Done {
                what: DoneWhat::Series,
                result,
                ask: None,
            }
        });
        Ok(())
    }

    /// «Stop after the current ones».
    pub(in crate::real) fn stop_series(&self, id: u64) {
        self.series_run.stop(id);
    }

    /// The plate's «×»: not while the series runs (series r1, Could) — its plate is how it is
    /// stopped.
    pub(in crate::real) fn close_series(&self, id: u64) -> Result<(), CoreError> {
        if self.series_run.is_running(id) {
            return Err(super::identity_keys::identity_busy());
        }
        self.nft_series.close(id).map_err(CoreError::Failed)
    }
}

/// The screens' state of a write, from the disk's.
#[allow(dead_code)]
pub(in crate::real) fn item_state(state: State) -> ItemState {
    match state {
        State::Waiting => ItemState::Waiting,
        State::Sending => ItemState::Sending,
        State::Done => ItemState::Done,
        State::Refused => ItemState::Refused,
        State::Unclear => ItemState::Unclear,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, description: &str, supply: u32, image: &[u8]) -> NewModel {
        NewModel {
            name: name.into(),
            description: description.into(),
            supply,
            image: image.to_vec(),
        }
    }

    const SVG: &[u8] =
        br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="1" height="1"/></svg>"#;

    /// UC-5, R-4, R-5, R-6: the texts, the supply and the picture, before the password.
    #[test]
    fn a_new_model_is_checked_before_the_password() {
        assert!(check_new_model(&model("Dragon", "A red one", 10, SVG), 3).is_ok());
        let cases = [
            (model("", "x", 10, SVG), 0, SeriesRefusal::NameLength(0)),
            (
                model(&"я".repeat(65), "x", 10, SVG),
                0,
                SeriesRefusal::NameLength(65),
            ),
            (
                model("Dragon", &"я".repeat(281), 10, SVG),
                0,
                SeriesRefusal::DescriptionLength(281),
            ),
            (
                model("Dragon", "see evolite.org", 10, SVG),
                0,
                SeriesRefusal::Link,
            ),
            (model("Dragon", "x", 0, SVG), 0, SeriesRefusal::Supply),
            (model("Dragon", "x", 3, SVG), 4, SeriesRefusal::Supply),
            (
                model("Dragon", "x", 3, b"not a picture"),
                0,
                SeriesRefusal::ImageType,
            ),
            (
                model("Dragon", "x", 3, br#"<svg><script>x()</script></svg>"#),
                0,
                SeriesRefusal::ImageUnsafe("script".into()),
            ),
        ];
        for (m, mint, refusal) in cases {
            assert_eq!(check_new_model(&m, mint).err(), Some(refusal), "{}", m.name);
        }
        // 280 characters of description and «v1.2» in the name are fine.
        assert!(check_new_model(&model("Dragon v1.2", &"я".repeat(280), 3, SVG), 3).is_ok());
    }

    /// A JPEG shaped as the encoders make one: SOI, JFIF, a table, a frame, a scan, EOI.
    fn jpeg(extra: &[u8], scan: usize) -> Vec<u8> {
        let seg = |code: u8, body: &[u8]| {
            let mut s = vec![0xFF, code];
            s.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
            s.extend_from_slice(body);
            s
        };
        let mut f = vec![0xFF, 0xD8];
        f.extend(seg(0xE0, b"JFIF      "));
        f.extend_from_slice(extra);
        f.extend(seg(0xDB, &[0; 65]));
        f.extend(seg(0xC0, &[8, 0, 16, 0, 16, 1, 1, 0x11, 0]));
        f.extend(seg(0xDA, &[1, 1, 0, 0, 0x3F, 0]));
        f.extend((0..scan).map(|i| (i % 200) as u8 + 1));
        f.extend_from_slice(&[0xFF, 0xD9]);
        f
    }

    /// R-22, AC-A22: a JPEG with a geotag is weighed and written without it, «removed» said; one
    /// that fits only once stripped fits, with no «Make a smaller copy» (R-21).
    #[test]
    fn a_picture_is_weighed_after_its_metadata_went() {
        let exif = {
            let body = b"Exif  MM *GPS 55.7558 37.6173";
            let mut s = vec![0xFF, 0xE1];
            s.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
            s.extend_from_slice(body);
            s
        };
        let file = jpeg(&exif, 1_000);
        let room = file.len() - exif.len() + 1;
        assert!(file.len() > room, "larger than the room as it is");
        let p = check_picture(&file, room);
        let packed = p.packed.unwrap();
        let check = p.check.unwrap();
        assert!(check.removed && !check.can_fit && !check.glb);
        assert_eq!(check.size, packed.stored());
        assert!(!p.stripped.windows(4).any(|w| w == b"Exif"));
        assert!(!p.stripped.windows(3).any(|w| w == b"GPS"));
        // Nothing to take out: nothing said.
        let clean = jpeg(&[], 1_000);
        assert!(!check_picture(&clean, 19_000).check.unwrap().removed);
    }

    /// R-21: a raster over the room may be made smaller; an SVG over it may not; a file whose
    /// structure cannot be followed is refused (R-22), never written as it is.
    #[test]
    fn what_is_over_the_room_and_what_cannot_be_read() {
        let big = jpeg(&[], 4_000);
        let p = check_picture(&big, 2_000);
        assert_eq!(p.packed, Err(SeriesRefusal::ImageTooBig(big.len())));
        assert!(p.check.unwrap().can_fit);
        let mut seed = 0x2545_F491u32;
        let words: String = (0..3_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                char::from(b'a' + (seed >> 24) as u8 % 26)
            })
            .collect();
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><text>{words}</text></svg>"#);
        let p = check_picture(svg.as_bytes(), 200);
        assert!(matches!(p.packed, Err(SeriesRefusal::ImageTooBig(_))));
        assert!(!p.check.unwrap().can_fit, "SVG is never made smaller");
        let mut cut = jpeg(&[], 1_000);
        cut.truncate(cut.len() - 300);
        let p = check_picture(&cut, 19_000);
        assert_eq!(p.packed, Err(SeriesRefusal::ImageUnreadable));
        assert!(p.check.is_none());
    }
}
