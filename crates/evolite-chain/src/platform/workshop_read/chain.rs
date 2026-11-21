//! The Workshop's reads on the chain: each proved, none spends.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use dash_sdk::platform::{DocumentQuery, FetchMany};
use dpp::data_contract::DataContract;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::accessors::v1::DataContractV1Getters;
use dpp::document::{Document, DocumentV0Getters};
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use evolite_api::workshop::{
    Collection, Count, Instance, Instances, Limit as ApiLimit, Model, ModelCounts, Sale,
};
use sha2::{Digest, Sha256};

use super::profile::{self, Limit, Profile};
use super::{
    DOCUMENT_HISTORY, MAX_PAGES, PAGE, SalesCursor, count_mine, mine_query, new_sales, pages,
    price_of, sale_of, sales_query, serial_of,
};
use crate::chain::Chain;

/// How long a collection's contract is good for a read (AC-T-C4: its card, 10 minutes).
const CONTRACT_WITHIN: Duration = Duration::from_secs(600);

fn id_of(s: &str) -> Result<Identifier, String> {
    Identifier::from_string(s, Encoding::Base58).map_err(|e| e.to_string())
}

fn b58(id: Identifier) -> String {
    id.to_string(Encoding::Base58)
}

fn limit(l: Limit) -> ApiLimit {
    match l {
        Limit::NoSender => ApiLimit::NoSender,
        Limit::NoDelete => ApiLimit::NoDelete,
        Limit::NoBuyer => ApiLimit::NoBuyer,
        Limit::CountByPages => ApiLimit::CountByPages,
        Limit::SerialUnchecked => ApiLimit::SerialUnchecked,
    }
}

/// A model's picture as stored: `image`, or `part0…3` joined in their order.
fn stored_picture(doc: &Document) -> Vec<u8> {
    let props = doc.properties();
    let bytes = |k: &str| props.get(k).and_then(|v| v.as_bytes().cloned());
    match bytes(profile::IMAGE) {
        Some(b) => b,
        None => profile::PARTS
            .iter()
            .filter_map(|p| bytes(p))
            .flatten()
            .collect(),
    }
}

/// The key a model's picture is drawn by: the SHA-256 of its bytes as stored.
fn picture_key(stored: &[u8]) -> String {
    Sha256::digest(stored)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// BUG-269: a model's picture as the renderer takes it — its key, its stored bytes and whether
/// they are raw deflate. The «Workshop» deflates an SVG or a GLB when that is smaller (the
/// document does not say so): one whose bytes are not an SVG's or a GLB's by their first bytes
/// is packed. `None` without a picture.
pub fn model_picture(doc: &Document) -> Option<(String, Vec<u8>, bool)> {
    use crate::platform::nft_series::image::{ImageType, image_type};
    let stored = stored_picture(doc);
    if stored.is_empty() {
        return None;
    }
    let kind = doc
        .properties()
        .get(profile::IMAGE_TYPE)
        .and_then(|v| v.to_integer::<u8>().ok());
    let packable = [ImageType::Svg, ImageType::AnimatedSvg, ImageType::Glb]
        .iter()
        .any(|t| kind == Some(*t as u8));
    let deflated = packable && image_type(&stored).is_none();
    Some((picture_key(&stored), stored, deflated))
}

/// A model document as the screen reads it. The picture's key is the SHA-256 of its bytes as
/// stored (`image`, or `part0…3` joined).
pub fn model_of(doc: &Document) -> Model {
    let props = doc.properties();
    let text = |k: &str| {
        props
            .get(k)
            .and_then(|v| v.as_text())
            .map(crate::dashpay::shown)
            .unwrap_or_default()
    };
    Model {
        id: b58(doc.id()),
        name: text(profile::NAME),
        description: text(profile::DESCRIPTION),
        art: picture_key(&stored_picture(doc)),
        supply: props
            .get(profile::SUPPLY)
            .and_then(|v| v.to_integer::<u32>().ok())
            .unwrap_or(0),
    }
}

impl Chain {
    /// A collection's contract and profile, fresh within [`CONTRACT_WITHIN`].
    async fn collection_contract(
        &self,
        contract: &str,
    ) -> Result<(Arc<DataContract>, Profile), String> {
        let kept = self.fresh_cached(id_of(contract)?, CONTRACT_WITHIN).await?;
        let profile = profile::profile(&kept.contract)
            .map_err(|why| format!("not an NFT collection: {why:?}"))?;
        Ok((kept.contract, profile))
    }

    /// The models of a collection, at most [`MAX_PAGES`] pages of them.
    async fn models(
        &self,
        contract: Arc<DataContract>,
        profile: &Profile,
    ) -> Result<Vec<Model>, String> {
        let sdk = self.sdk().await?;
        let query = DocumentQuery::new(contract, &profile.model).map_err(|e| format!("{e:?}"))?;
        let page = pages(&sdk, query, None, usize::MAX, MAX_PAGES, |_| true).await?;
        // BUG-269: their pictures kept to draw, on any copy and after a restart.
        for doc in &page.docs {
            self.keep_workshop_picture(doc);
        }
        Ok(page.docs.iter().map(model_of).collect())
    }

    /// The ids of the collections `owner` (base58) made, as kept: no network.
    pub fn workshop_collection_ids(&self, owner: &str) -> BTreeSet<String> {
        let (Ok(owner), Some(cache)) = (id_of(owner), self.contracts()) else {
            return BTreeSet::new();
        };
        cache
            .nft_contracts()
            .into_iter()
            .filter(|(id, _)| {
                cache
                    .get(id)
                    .is_some_and(|k| k.contract.owner_id() == owner)
            })
            .map(|(id, _)| b58(id))
            .collect()
    }

    /// The collections `owner` (base58) made: the contracts kept that fit the profile and are
    /// its own, with their models (AC-T-C5; the one dev3's module registers is kept too).
    pub async fn workshop_collections(&self, owner: &str) -> Result<Vec<Collection>, String> {
        self.ready()?;
        let owner = id_of(owner)?;
        let cache = self.contracts().ok_or("the contracts cache is not open")?;
        let mut out = Vec::new();
        for (id, _) in cache.nft_contracts() {
            let Some(kept) = cache.get(&id) else { continue };
            if kept.contract.owner_id() != owner {
                continue;
            }
            let Ok(profile) = profile::profile(&kept.contract) else {
                continue;
            };
            let models = self.models(kept.contract.clone(), &profile).await?;
            out.push(Collection {
                contract: b58(id),
                description: kept.contract.description().cloned().unwrap_or_default(),
                open: profile.open,
                limits: profile.limits.iter().copied().map(limit).collect(),
                models,
                checked_at: kept.checked_at,
            });
        }
        Ok(out)
    }

    /// A model's numbers for `owner` (R-14): proved count by the countable index, else counted
    /// over the pages read; for sale over at most [`MAX_PAGES`] pages; sold from `sold`, the
    /// sales already read for this model.
    pub async fn workshop_counts(
        &self,
        contract: &str,
        model: &str,
        owner: &str,
        sold: Option<u32>,
    ) -> Result<ModelCounts, String> {
        self.ready()?;
        let sdk = self.sdk().await?;
        let (c, profile) = self.collection_contract(contract).await?;
        let query = mine_query(c.clone(), &profile, id_of(model)?, id_of(owner)?)?;
        let page = pages(&sdk, query, None, usize::MAX, MAX_PAGES, |_| true).await?;
        let complete = page.next.is_none();
        let documents = page.read as u64;
        let mine = if profile.limits.contains(&Limit::CountByPages) {
            Count::Counted {
                count: documents,
                documents,
                complete,
            }
        } else {
            Count::Proved(count_mine(&sdk, c, &profile, id_of(model)?, id_of(owner)?).await?)
        };
        let mut prices = std::collections::BTreeMap::<i64, u32>::new();
        for p in page.docs.iter().filter_map(price_of) {
            *prices.entry(p as i64).or_default() += 1;
        }
        let for_sale: u64 = prices.values().map(|n| *n as u64).sum();
        // W4: issued is the highest number the network holds (R-6); a failed read is «—».
        let issued = self
            .nft_max_serial(id_of(contract)?, id_of(model)?)
            .await
            .ok()
            .map(|max| max.unwrap_or(0));
        Ok(ModelCounts {
            contract: contract.into(),
            model: model.into(),
            mine: Some(mine),
            for_sale: Some(Count::Counted {
                count: for_sale,
                documents,
                complete,
            }),
            prices: prices.into_iter().collect(),
            sold,
            issued,
            checked_at: evolite_common::wall_secs() as i64,
        })
    }

    /// One's instances of a model (or those for sale), from `after`: at most [`MAX_PAGES`]
    /// pages, up to a screen's page ([`PAGE`]), sorted by number within it.
    pub async fn workshop_instances(
        &self,
        contract: &str,
        model: &str,
        owner: &str,
        for_sale: bool,
        after: Option<&str>,
    ) -> Result<Instances, String> {
        self.ready()?;
        let sdk = self.sdk().await?;
        let (c, profile) = self.collection_contract(contract).await?;
        let query = mine_query(c, &profile, id_of(model)?, id_of(owner)?)?;
        let after = after.map(id_of).transpose()?;
        let page = pages(&sdk, query, after, PAGE as usize, MAX_PAGES, |d| {
            !for_sale || price_of(d).is_some()
        })
        .await?;
        let mut instances: Vec<Instance> = page
            .docs
            .iter()
            .filter_map(|d| {
                Some(Instance {
                    id: b58(d.id()),
                    serial: serial_of(d)?,
                    price: price_of(d).map(|p| p as i64),
                })
            })
            .collect();
        instances.sort_by_key(|i| i.serial);
        Ok(Instances {
            contract: contract.into(),
            model: model.into(),
            for_sale,
            instances,
            next: page.next.map(b58),
        })
    }

    /// `seller`'s sales since `cursor` (AC-T-W4) of any contract that fits the NFT profile — its
    /// own collections and resales of others' (review r1 Should) — at most [`MAX_PAGES`] pages
    /// per call, each resolved to its model and number where the instance is still there, and
    /// the buyer's name; with the cursor after them. A contract not kept is read with a proof; one
    /// that cannot be read stops the cursor before its sale.
    pub async fn workshop_sales(
        &self,
        seller: &str,
        cursor: &SalesCursor,
    ) -> Result<(Vec<Sale>, SalesCursor), String> {
        self.ready()?;
        let sdk = self.sdk().await?;
        let history = self
            .fresh_cached(id_of(DOCUMENT_HISTORY)?, CONTRACT_WITHIN)
            .await?
            .contract;
        let seller = id_of(seller)?;
        let mut cursor = cursor.clone();
        let mut found = Vec::new();
        for _ in 0..MAX_PAGES {
            let since = (cursor.at_ms > 0).then_some(cursor.at_ms);
            let query = sales_query(history.clone(), seller, since)?;
            let docs: Vec<_> = Document::fetch_many(&sdk, query)
                .await
                .map_err(|e| format!("{e:?}"))?
                .into_values()
                .flatten()
                .filter_map(|d| sale_of(&d))
                .collect();
            let full = docs.len() >= PAGE as usize;
            let mut verdicts = BTreeMap::new();
            for contract in docs
                .iter()
                .map(|d| d.contract.clone())
                .collect::<BTreeSet<_>>()
            {
                let verdict = self.fits_profile(&contract).await;
                verdicts.insert(contract, verdict);
            }
            let (sales, next) = new_sales(docs, &cursor, |c| verdicts.get(c).copied().flatten());
            let moved = next != cursor;
            cursor = next;
            found.extend(sales);
            if !full || !moved {
                break;
            }
        }
        let mut out = Vec::with_capacity(found.len());
        for s in found {
            let (model_id, model, serial) = self.sold_instance(&s.contract, &s.instance).await;
            let buyer_name = match id_of(&s.buyer) {
                Ok(b) => self.name_of(b).await.ok().flatten(),
                Err(_) => None,
            };
            out.push(Sale {
                purchase: s.id,
                contract: s.contract,
                instance: s.instance,
                model_id,
                model,
                serial,
                buyer: s.buyer,
                buyer_name,
                price: s.price as i64,
                at: (s.at_ms / 1000) as i64,
            });
        }
        Ok((out, cursor))
    }

    /// Whether `contract` fits the NFT profile: from the cache, or read with a proof (C5 keeps
    /// it and its verdict); `None` when it cannot be read now.
    async fn fits_profile(&self, contract: &str) -> Option<bool> {
        let kept = self
            .fresh_cached(id_of(contract).ok()?, CONTRACT_WITHIN)
            .await
            .ok()?;
        Some(profile::profile(&kept.contract).is_ok())
    }

    /// The model's id and name and the number of a sold instance; all `None` when it is gone
    /// (deleted after the sale, the architect's point 4) or cannot be read.
    async fn sold_instance(
        &self,
        contract: &str,
        instance: &str,
    ) -> (Option<String>, Option<String>, Option<u32>) {
        let read = async {
            let sdk = self.sdk().await?;
            let (c, profile) = self.collection_contract(contract).await?;
            let doc = Document::fetch_many(
                &sdk,
                DocumentQuery::new(c.clone(), &profile.instance)
                    .map_err(|e| format!("{e:?}"))?
                    .with_document_id(&id_of(instance)?),
            )
            .await
            .map_err(|e| format!("{e:?}"))?
            .into_values()
            .flatten()
            .next();
            let Some(doc) = doc else {
                return Ok::<_, String>((None, None, None));
            };
            let serial = serial_of(&doc);
            let model_id = doc
                .properties()
                .get(&profile.model_field)
                .and_then(|v| v.to_identifier().ok());
            let model = match model_id {
                Some(m) => Document::fetch_many(
                    &sdk,
                    DocumentQuery::new(c, &profile.model)
                        .map_err(|e| format!("{e:?}"))?
                        .with_document_id(&m),
                )
                .await
                .map_err(|e| format!("{e:?}"))?
                .into_values()
                .flatten()
                .next()
                .map(|d| {
                    self.keep_workshop_picture(&d);
                    model_of(&d).name
                }),
                None => None,
            };
            Ok((model_id.map(b58), model, serial))
        };
        read.await.unwrap_or((None, None, None))
    }
}
