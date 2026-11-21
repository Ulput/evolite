//! The chain's side of «Workshop» series (FEAT-0038 item 1): instances of a model read with
//! proof by the unique `byImageSerial` index, and the four writes a series makes — an
//! instance, a price, a transfer (a gift, or to oneself: off sale), a delete — signed with the
//! identity's HIGH key of this phrase. The series runs in the core.

use super::*;
use crate::platform::nft_series::plan::Instance;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{DocumentQuery, FetchMany};
use dpp::data_contract::DataContract;
use dpp::document::{Document, DocumentV0Getters};
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::signer::Signer;
use dpp::identity::{KeyType, Purpose, SecurityLevel};
use dpp::platform_value::Value;

/// The instances' document type in a collection by the profile.
const NFT: &str = "nft";

/// A page of `byImageSerial`: as many as Platform gives in one proved answer.
const PAGE: u32 = 100;

fn instance(doc: &Document) -> Option<Instance> {
    let serial = doc
        .properties()
        .get("serial")
        .and_then(|v| v.as_integer::<u32>())?;
    Some(Instance {
        id: doc.id(),
        serial,
        owner: doc.owner_id(),
        price: doc
            .properties()
            .get("$price")
            .and_then(|v| v.as_integer::<u64>()),
    })
}

fn by_model(model: &Identifier) -> WhereClause {
    WhereClause {
        field: "imageId".into(),
        operator: WhereOperator::Equal,
        value: Value::Identifier(model.to_buffer()),
    }
}

impl Chain {
    /// The collection's contract, proved in this run: a write is checked against it.
    async fn nft_contract(&self, contract: Identifier) -> Result<Arc<DataContract>, String> {
        self.fresh_proved(contract).await.map(|c| c.contract)
    }

    /// The highest serial of `model` the network holds, proved: one request by `byImageSerial`
    /// descending (FEAT-0038 R-6). `None`: none.
    pub async fn nft_max_serial(
        &self,
        contract: Identifier,
        model: Identifier,
    ) -> Result<Option<u32>, String> {
        Ok(self
            .nft_serial_page(contract, model, None, true, 1)
            .await?
            .first()
            .map(|i| i.serial))
    }

    /// A page of `model`'s instances by serial, proved: ascending after `after`, or descending
    /// before it.
    pub async fn nft_serial_page(
        &self,
        contract: Identifier,
        model: Identifier,
        after: Option<u32>,
        descending: bool,
        limit: u32,
    ) -> Result<Vec<Instance>, String> {
        self.ready()?;
        let sdk = self.sdk().await?;
        let contract = self.nft_contract(contract).await?;
        let mut query = DocumentQuery::new(contract, NFT)
            .map_err(|e| e.to_string())?
            .with_where(by_model(&model));
        if let Some(after) = after {
            query = query.with_where(WhereClause {
                field: "serial".into(),
                operator: if descending {
                    WhereOperator::LessThan
                } else {
                    WhereOperator::GreaterThan
                },
                value: Value::U32(after),
            });
        }
        let query = query
            .with_order_by(OrderClause {
                field: "serial".into(),
                ascending: !descending,
            })
            .with_limit(limit.min(PAGE));
        let found = Document::fetch_many(&sdk, query)
            .await
            .map_err(|e| e.to_string())?;
        let mut page: Vec<Instance> = found.values().flatten().filter_map(instance).collect();
        // The proof's order is the query's; kept so whatever the map gives.
        page.sort_by_key(|i| i.serial);
        if descending {
            page.reverse();
        }
        Ok(page)
    }

    /// One instance read again, proved: by its document, or (a mint) by its serial. `None`:
    /// the network proves it is not there.
    pub async fn nft_instance(
        &self,
        contract: Identifier,
        model: Identifier,
        doc: Option<Identifier>,
        serial: u32,
    ) -> Result<Option<Instance>, String> {
        self.ready()?;
        let sdk = self.sdk().await?;
        let c = self.nft_contract(contract).await?;
        let query = DocumentQuery::new(c, NFT).map_err(|e| e.to_string())?;
        let query = match doc {
            Some(doc) => query.with_document_id(&doc),
            None => query.with_where(by_model(&model)).with_where(WhereClause {
                field: "serial".into(),
                operator: WhereOperator::Equal,
                value: Value::U32(serial),
            }),
        }
        .with_limit(1);
        let found = Document::fetch_many(&sdk, query)
            .await
            .map_err(|e| e.to_string())?;
        Ok(found.values().flatten().find_map(instance))
    }

    /// The supply of model `model` («image» document), read with proof; `None`: no such model.
    pub async fn nft_model_supply(
        &self,
        contract: Identifier,
        model: Identifier,
    ) -> Result<Option<u32>, String> {
        self.ready()?;
        let sdk = self.sdk().await?;
        let c = self.nft_contract(contract).await?;
        let query = DocumentQuery::new(c, "image")
            .map_err(|e| e.to_string())?
            .with_document_id(&model)
            .with_limit(1);
        let found = Document::fetch_many(&sdk, query)
            .await
            .map_err(|e| e.to_string())?;
        Ok(found.values().flatten().find_map(|d| {
            d.properties()
                .get("supply")
                .and_then(|v| v.as_integer::<u32>())
        }))
    }

    /// Whether this phrase signs for `owner` in the «Workshop»: an AUTHENTICATION · CRITICAL
    /// key (FEAT-0038 R-1, R-18) and a HIGH one for the instances' writes, both of this wallet.
    pub async fn nft_keys_ready(
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
    ) -> bool {
        let Ok((identity, signer)) = Self::signing(wallet, keys, owner).await else {
            return false;
        };
        let has = |level: SecurityLevel| {
            identity.public_keys().values().any(|k| {
                k.purpose() == Purpose::AUTHENTICATION
                    && k.security_level() == level
                    && k.key_type() == KeyType::ECDSA_SECP256K1
                    && k.disabled_at().is_none()
                    && Signer::<IdentityPublicKey>::can_sign_with(&signer, k)
            })
        };
        has(SecurityLevel::CRITICAL) && has(SecurityLevel::HIGH)
    }

    /// The signer of `owner` and its HIGH key of this phrase (the instances' writes: their type
    /// asks HIGH, platform-wallet's document writes take one).
    async fn nft_signer(
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
    ) -> Result<(crate::key_signer::KeySigner, u32), String> {
        let (identity, signer) = Self::signing(wallet, keys, owner).await?;
        let key = identity
            .public_keys()
            .values()
            .find(|k| {
                k.purpose() == Purpose::AUTHENTICATION
                    && k.security_level() == SecurityLevel::HIGH
                    && k.key_type() == KeyType::ECDSA_SECP256K1
                    && k.disabled_at().is_none()
                    && Signer::<IdentityPublicKey>::can_sign_with(&signer, k)
            })
            .ok_or_else(|| "no HIGH key of this phrase".to_owned())?
            .id();
        Ok((signer, key))
    }

    /// Writes a new model («image» document) with its picture `packed`; its document id.
    #[allow(clippy::too_many_arguments)]
    pub async fn nft_create_model(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
        contract: Identifier,
        name: &str,
        description: &str,
        supply: u32,
        packed: &crate::platform::nft_series::image::Packed,
    ) -> Result<Identifier, String> {
        self.nft_contract(contract).await?;
        let (signer, _) = Self::nft_signer(wallet, keys, owner).await?;
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let mut doc = serde_json::json!({
            "name": name,
            "description": description,
            "imageType": packed.image_type as u8,
            "supply": supply,
        });
        if let [one] = packed.parts.as_slice() {
            doc["image"] = serde_json::Value::String(hex(one));
        } else {
            for (i, part) in packed.parts.iter().enumerate() {
                doc[format!("part{i}")] = serde_json::Value::String(hex(part));
            }
        }
        wallet
            .identity()
            .create_document_with_signer(owner, &contract, "image", &doc.to_string(), None, &signer)
            .await
            .map(|d| d.id())
            .map_err(|e| e.to_string())
    }

    /// The room a model's picture has in its write (FEAT-0038 R-21): measured at the network's
    /// version for this name, description and supply. Before a proved answer the version is
    /// not known: an error, never a guess.
    pub async fn nft_picture_room(
        &self,
        name: &str,
        description: &str,
        supply: u32,
        kind: crate::platform::nft_series::image::ImageType,
    ) -> Result<usize, String> {
        let sdk = self.sdk().await?;
        let protocol = self
            .known_protocol(&sdk)
            .await
            .ok_or("the network's version is not known yet")?;
        let pv = crate::contract_v1::version_for(protocol).map_err(|r| format!("{r:?}"))?;
        crate::platform::nft_series::room::picture_room(name, description, supply, kind, pv).await
    }

    /// Writes the collection's «About the collection» document (`collectionInfo`, W2): the
    /// contract `main` it belongs to. Right after the collection's own registration, which kept
    /// the contract in the cache and the provider: it is not read again (a node a block behind
    /// would say «no such contract» — r1 Should 1). Never repeated by the core.
    pub async fn nft_collection_info(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
        contract: Identifier,
        main: Identifier,
    ) -> Result<Identifier, String> {
        let (signer, _) = Self::nft_signer(wallet, keys, owner).await?;
        let json =
            serde_json::json!({ "mainContract": main.to_string(Encoding::Base58) }).to_string();
        wallet
            .identity()
            .create_document_with_signer(owner, &contract, "collectionInfo", &json, None, &signer)
            .await
            .map(|d| d.id())
            .map_err(|e| e.to_string())
    }

    /// Writes instance `serial` of `model`; its document id.
    pub async fn nft_mint(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
        contract: Identifier,
        model: Identifier,
        serial: u32,
    ) -> Result<Identifier, String> {
        self.nft_contract(contract).await?;
        let (signer, _) = Self::nft_signer(wallet, keys, owner).await?;
        let json = serde_json::json!({
            "imageId": model.to_string(Encoding::Base58),
            "serial": serial,
        })
        .to_string();
        wallet
            .identity()
            .create_document_with_signer(owner, &contract, NFT, &json, None, &signer)
            .await
            .map(|d| d.id())
            .map_err(|e| e.to_string())
    }

    /// Puts instance `doc` on sale at `price` credits (over a price it has, too).
    pub async fn nft_price(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
        contract: Identifier,
        doc: Identifier,
        price: u64,
    ) -> Result<(), String> {
        if price == 0 {
            // FEAT-0038 R-9 and the probe: a price of 0 stays as `$price: 0`, and a purchase
            // at it fails in Drive. Never sent.
            return Err("a price of 0 is not a price".into());
        }
        self.nft_contract(contract).await?;
        let (signer, key) = Self::nft_signer(wallet, keys, owner).await?;
        wallet
            .identity()
            .set_document_price_with_signer(owner, &contract, NFT, &doc, price, key, &signer)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Transfers instance `doc` to `to`: a gift, or to `owner` itself — off sale (the transfer
    /// drops `$price`: testnet, 2026-11-20).
    #[allow(clippy::too_many_arguments)]
    pub async fn nft_transfer(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
        contract: Identifier,
        doc: Identifier,
        to: Identifier,
    ) -> Result<(), String> {
        self.nft_contract(contract).await?;
        let (signer, key) = Self::nft_signer(wallet, keys, owner).await?;
        wallet
            .identity()
            .transfer_document_with_signer(owner, &contract, NFT, &doc, &to, key, &signer)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Deletes instance `doc` for good (R-20).
    pub async fn nft_delete(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        owner: &Identifier,
        contract: Identifier,
        doc: Identifier,
    ) -> Result<(), String> {
        self.nft_contract(contract).await?;
        let (signer, key) = Self::nft_signer(wallet, keys, owner).await?;
        wallet
            .identity()
            .delete_document_with_signer(owner, &contract, NFT, &doc, key, &signer)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
