//! FEAT-0031 stage 1, AC-T-C1…C3: does a contract hold NFTs as evolite shows them? A pure
//! function over the `DataContract`, no network. The «Изображение» (model) type is the one an
//! instance type's `refersTo` `permanentDocument` points at; the instance type is the one with
//! that link. What is required refuses the contract with a reason (C1); what is only wanted
//! lets it in with the limits the screen names (C2).

use std::collections::BTreeSet;

use dpp::data_contract::DataContract;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::config::v0::DataContractConfigGettersV0;
use dpp::data_contract::config::v2::DataContractConfigGettersV2;
use dpp::data_contract::document_type::DocumentTypeRef;
use dpp::data_contract::document_type::IndexCountability;
use dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dpp::data_contract::document_type::restricted_creation::CreationRestrictionMode;
use dpp::document::transfer::Transferable;
use dpp::nft::TradeMode;
use dpp::platform_value::Value;

/// The model type's fields (the schema of FEAT-0038, `collection-v1.json`).
pub const NAME: &str = "name";
pub const DESCRIPTION: &str = "description";
pub const IMAGE_TYPE: &str = "imageType";
pub const SUPPLY: &str = "supply";
pub const IMAGE: &str = "image";
pub const PARTS: [&str; 4] = ["part0", "part1", "part2", "part3"];
/// The instance type's fields.
pub const SERIAL: &str = "serial";
/// The most bytes of one picture field (AC-T-C1).
pub const PICTURE_BYTES: u64 = 5120;

/// A contract that fits: its two types, open or closed, and what it lacks (C2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// The model («Изображение») type.
    pub model: String,
    /// The instance type.
    pub instance: String,
    /// The instance's field that points at its model.
    pub model_field: String,
    /// Anyone may make instances (`creationRestrictionMode` 0); closed: the owner only.
    pub open: bool,
    pub limits: BTreeSet<Limit>,
}

/// What a fitting contract lacks, said on its screen (AC-T-C2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Limit {
    /// No `keepsTransferHistory`: the sender is unknown, no «Вернуть».
    NoSender,
    /// Instances cannot be deleted: «Скрыть» instead of «Удалить навсегда».
    NoDelete,
    /// No `keepsPurchaseHistory`: «Продано» without the buyer.
    NoBuyer,
    /// No countable `[model, $ownerId]`: counted page by page, marked so (AC-T-M5).
    CountByPages,
    /// No unique `(model, serial)`: evolite checks numbers itself.
    SerialUnchecked,
}

/// Why a contract does not fit (AC-T-C1), each in words on the screen (AC-A9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unfit {
    NotReadonly,
    Moderated,
    /// No type points at another with a `permanentDocument` link.
    NoModelLink,
    NotTransferable,
    Mutable,
    NoOwnerIndex,
    ModelDeletable,
    /// Creation closed to everyone (`creationRestrictionMode` 2).
    NoCreation,
    /// A field missing or of the wrong type or bounds.
    Field(&'static str),
}

/// The profile of `contract`, or every reason it does not fit.
pub fn profile(contract: &DataContract) -> Result<Profile, Vec<Unfit>> {
    let mut unfit = Vec::new();
    let config = contract.config();
    if !config.readonly() {
        unfit.push(Unfit::NotReadonly);
    }
    if config.moderation().is_some() {
        unfit.push(Unfit::Moderated);
    }
    let types = contract.document_types();
    let Some((instance, model_field, model)) = types.iter().find_map(|(name, t)| {
        model_link(t.as_ref()).map(|(field, model)| (name.clone(), field, model))
    }) else {
        unfit.push(Unfit::NoModelLink);
        return Err(unfit);
    };
    let Some(model_type) = types.get(&model) else {
        unfit.push(Unfit::NoModelLink);
        return Err(unfit);
    };
    let it = types[&instance].as_ref();
    let mt = model_type.as_ref();
    if it.documents_transferable() != Transferable::Always {
        unfit.push(Unfit::NotTransferable);
    }
    if it.documents_mutable() {
        unfit.push(Unfit::Mutable);
    }
    if !it
        .indexes()
        .values()
        .any(|i| i.properties.first().is_some_and(|p| p.name == "$ownerId"))
    {
        unfit.push(Unfit::NoOwnerIndex);
    }
    if mt.documents_can_be_deleted() {
        unfit.push(Unfit::ModelDeletable);
    }
    let open = match it.creation_restriction_mode() {
        CreationRestrictionMode::NoRestrictions => true,
        CreationRestrictionMode::OwnerOnly => false,
        _ => {
            unfit.push(Unfit::NoCreation);
            false
        }
    };
    fields(mt, it, &mut unfit);
    if !unfit.is_empty() {
        return Err(unfit);
    }

    let mut limits = BTreeSet::new();
    if !it.documents_keep_transfer_history() {
        limits.insert(Limit::NoSender);
    }
    if !it.documents_can_be_deleted() {
        limits.insert(Limit::NoDelete);
    }
    if !it.documents_keep_purchase_history() || it.trade_mode() != TradeMode::DirectPurchase {
        limits.insert(Limit::NoBuyer);
    }
    let fields_are = |i: &dpp::data_contract::document_type::Index, names: [&str; 2]| {
        i.properties.len() == 2 && i.properties.iter().zip(names).all(|(p, n)| p.name == n)
    };
    if !it.indexes().values().any(|i| {
        fields_are(i, [model_field.as_str(), "$ownerId"])
            && i.countable != IndexCountability::NotCountable
    }) {
        limits.insert(Limit::CountByPages);
    }
    if !it
        .indexes()
        .values()
        .any(|i| i.unique && fields_are(i, [model_field.as_str(), SERIAL]))
    {
        limits.insert(Limit::SerialUnchecked);
    }
    Ok(Profile {
        model,
        instance,
        model_field,
        open,
        limits,
    })
}

/// The field of `t` that refers to a `permanentDocument` of another type, and that type.
fn model_link(t: DocumentTypeRef) -> Option<(String, String)> {
    let Some(Value::Map(props)) = t.schema().get_optional_value("properties").ok().flatten() else {
        return None;
    };
    props.iter().find_map(|(name, prop)| {
        let refers = prop.get_optional_value("refersTo").ok().flatten()?;
        let kind = refers.get_optional_str("type").ok().flatten()?;
        let target = refers.get_optional_str("documentType").ok().flatten()?;
        if kind != "permanentDocument" || target == t.name() {
            return None;
        }
        Some((name.as_text()?.to_owned(), target.to_owned()))
    })
}

/// The fields the screens read, of the types and bounds they read them in (AC-T-C1).
fn fields(model: DocumentTypeRef, instance: DocumentTypeRef, unfit: &mut Vec<Unfit>) {
    let schema = |t: DocumentTypeRef, name: &str| {
        t.schema()
            .get_optional_value("properties")
            .ok()
            .flatten()
            .and_then(|p| p.get_optional_value(name).ok().flatten().cloned())
    };
    let kind = |v: &Value| v.get_optional_str("type").ok().flatten().map(str::to_owned);
    let max = |v: &Value, key: &str| v.get_optional_integer::<u64>(key).ok().flatten();
    let mut is = |t, name: &'static str, want: &str| match schema(t, name) {
        Some(v) if kind(&v).as_deref() == Some(want) => Some(v),
        _ => {
            unfit.push(Unfit::Field(name));
            None
        }
    };
    is(model, NAME, "string");
    is(model, DESCRIPTION, "string");
    is(model, IMAGE_TYPE, "integer");
    is(model, SUPPLY, "integer");
    is(instance, SERIAL, "integer");
    let picture = |v: &Value| {
        kind(v).as_deref() == Some("array")
            && v.get_optional_bool("byteArray").ok().flatten() == Some(true)
            && max(v, "maxItems").is_some_and(|m| m <= PICTURE_BYTES)
    };
    let whole = schema(model, IMAGE);
    let parts: Vec<_> = PARTS.iter().map(|p| schema(model, p)).collect();
    let whole_ok = whole.as_ref().is_some_and(picture);
    let parts_ok = parts.iter().all(|p| p.as_ref().is_some_and(picture));
    let any_bad =
        whole.as_ref().is_some_and(|v| !picture(v)) || parts.iter().flatten().any(|v| !picture(v));
    if any_bad || !(whole_ok || parts_ok) {
        unfit.push(Unfit::Field(IMAGE));
    }
}

#[cfg(test)]
#[path = "profile_tests.rs"]
pub(crate) mod tests;
