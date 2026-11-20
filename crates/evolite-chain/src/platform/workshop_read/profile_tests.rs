//! AC-T-C1…C3 over the Workshop's own schema and one change to it per case.

use super::*;
use dpp::version::PlatformVersion;
use serde_json::{Value as Json, json};

/// FEAT-0038's collection schema, as `contract_v1/collection-v1.json` on feature/contract-v1
/// (9507bbd0). Read from there once that branch is in dev.
const SCHEMA: &str = include_str!("collection-v1.json");

pub(crate) fn contract(documents: &Json, readonly: bool) -> Result<DataContract, String> {
    let pv = PlatformVersion::latest();
    let mut config = serde_json::Map::new();
    if readonly {
        config.insert("readonly".into(), json!(true));
    }
    config.insert(
        "$formatVersion".into(),
        json!(
            pv.dpp
                .contract_versions
                .config
                .default_current_version
                .to_string()
        ),
    );
    let format = json!({
        "$formatVersion": "1",
        "id": "FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P",
        "ownerId": "3EHobu1ki7vurdBaKH6JqEMPSbSM84Xf2crrL1HuHYzS",
        "version": 1,
        "documentSchemas": documents,
        "config": Json::Object(config),
    });
    let format = serde_json::from_str(&format.to_string()).map_err(|e| e.to_string())?;
    let mut operations = vec![];
    DataContract::try_from_platform_versioned(format, true, &mut operations, pv)
        .map_err(|e| e.to_string())
}

pub(crate) fn schema() -> Json {
    serde_json::from_str(SCHEMA).unwrap()
}

/// `change` applied to the Workshop's schema, read as a readonly contract.
fn changed(change: impl FnOnce(&mut Json)) -> Result<Profile, Vec<Unfit>> {
    let mut s = schema();
    change(&mut s);
    profile(&contract(&s, true).expect("DPP takes the changed schema"))
}

/// AC-T-C3 (with FEAT-0038 AC-T-W1): what the Workshop makes fits with no limits, closed.
#[test]
fn the_workshop_schema_fits_without_limits() {
    let p = profile(&contract(&schema(), true).unwrap()).unwrap();
    assert_eq!(p.model, "image");
    assert_eq!(p.instance, "nft");
    assert_eq!(p.model_field, "imageId");
    assert!(!p.open, "creationRestrictionMode 1: the owner only");
    assert!(p.limits.is_empty(), "{:?}", p.limits);
}

/// AC-T-C1: each required point refuses on its own, with its own reason.
#[test]
fn each_required_point_refuses_with_its_reason() {
    let refused = |r: Result<Profile, Vec<Unfit>>| r.expect_err("refused");
    assert_eq!(
        profile(&contract(&schema(), false).unwrap()).unwrap_err(),
        vec![Unfit::NotReadonly]
    );
    assert_eq!(
        refused(changed(|s| {
            s["nft"]["transferable"] = json!(0);
            s["nft"]["tradeMode"] = json!(0);
            s["nft"]["keepsPurchaseHistory"] = json!(false);
        })),
        vec![Unfit::NotTransferable]
    );
    assert_eq!(
        refused(changed(|s| s["image"]["canBeDeleted"] = json!(true))),
        vec![Unfit::ModelDeletable]
    );
    assert_eq!(
        refused(changed(|s| {
            s["nft"]["indices"] = json!([s["nft"]["indices"][2].clone()]);
        })),
        vec![Unfit::NoOwnerIndex]
    );
    assert_eq!(
        refused(changed(
            |s| s["image"]["properties"]["name"]["type"] = json!("integer")
        )),
        vec![Unfit::Field(NAME)]
    );
    assert_eq!(
        refused(changed(|s| {
            s["nft"]["properties"]["serial"] =
                json!({"type": "string", "maxLength": 9, "position": 1});
            s["nft"]
                .as_object_mut()
                .unwrap()
                .remove("propertyConstraints");
        })),
        vec![Unfit::Field(SERIAL)]
    );
    assert_eq!(
        refused(changed(|s| {
            s["nft"]["properties"]["imageId"]
                .as_object_mut()
                .unwrap()
                .remove("refersTo");
            s["nft"]
                .as_object_mut()
                .unwrap()
                .remove("propertyConstraints");
        })),
        vec![Unfit::NoModelLink]
    );
}

/// AC-T-C2: each wanted point lets the contract in, with its limit named.
#[test]
fn each_wanted_point_is_a_named_limit() {
    let limits = |r: Result<Profile, Vec<Unfit>>| r.expect("fits").limits;
    let one = |l| BTreeSet::from([l]);
    assert_eq!(
        limits(changed(|s| s["nft"]["keepsTransferHistory"] = json!(false))),
        one(Limit::NoSender)
    );
    assert_eq!(
        limits(changed(|s| s["nft"]["canBeDeleted"] = json!(false))),
        one(Limit::NoDelete)
    );
    assert_eq!(
        limits(changed(|s| s["nft"]["keepsPurchaseHistory"] = json!(false))),
        one(Limit::NoBuyer)
    );
    assert_eq!(
        limits(changed(|s| {
            s["nft"]["indices"][1]
                .as_object_mut()
                .unwrap()
                .remove("countable");
        })),
        one(Limit::CountByPages)
    );
    assert_eq!(
        limits(changed(|s| s["nft"]["indices"][2]["unique"] = json!(false))),
        one(Limit::SerialUnchecked)
    );
    // Open creation is not a limit: the screen says «открытая».
    let p = changed(|s| {
        s["nft"]["creationRestrictionMode"] = json!(0);
        s["image"]["creationRestrictionMode"] = json!(0);
    })
    .unwrap();
    assert!(p.open && p.limits.is_empty());
}
