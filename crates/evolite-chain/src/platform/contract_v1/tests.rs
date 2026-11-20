use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::accessors::v1::DataContractV1Getters;
use dpp::data_contract::config::v0::DataContractConfigGettersV0;
use dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use dpp::version::PlatformVersion;
use serde_json::json;

use super::fee::{self, Fee};
use super::rules::{KeywordRefusal, Refusal};
use super::*;

/// Testnet's protocol when the probe ran (2026-11-19): 14, read after a proved answer (the
/// SDK's seed before one is 13).
fn testnet() -> &'static PlatformVersion {
    PlatformVersion::get(14).unwrap()
}

fn owner() -> Identifier {
    Identifier::from_string(
        "5Y98scgD8Bgp3hp1Doq667wYCBchPxVGFzrCWLje7tXc",
        Encoding::Base58,
    )
    .unwrap()
}

fn words(w: &[&str]) -> Vec<String> {
    w.iter().map(|s| s.to_string()).collect()
}

/// AC-T-W1: the collection by the profile builds at testnet's protocol, `readonly`, with the
/// profile's flags and indexes, and its fee is what testnet took.
#[test]
fn the_collection_template_builds_at_the_networks_protocol() {
    // With the probe's description, as registered.
    let draft = templates::nft_collection(Some("evolite probe collection".into()));
    let built = build(&draft, owner(), testnet()).unwrap();
    let c = &built.contract;
    assert!(c.config().readonly());
    assert_eq!(c.document_types().len(), 3);
    let nft = c.document_type_for_name("nft").unwrap();
    assert!(nft.documents_transferable().is_transferable());
    assert!(nft.documents_can_be_deleted());
    assert!(!nft.documents_mutable());
    assert!(nft.documents_keep_transfer_history());
    assert!(nft.documents_keep_purchase_history());
    assert_eq!(nft.indexes().len(), 3);
    assert!(nft.indexes().values().any(|i| i.unique));
    let image = c.document_type_for_name("image").unwrap();
    assert!(!image.documents_can_be_deleted());
    assert_eq!(built.config_json, r#"{"readonly":true}"#);
    // The probe registered 2 349 bytes; the schema since takes 280 characters of description
    // and no required mainContract (architect, 2026-11-20).
    assert_eq!(built.bytes, 2_323);
}

/// AC-T-W2: the fee before the password. The registration exactly as Platform counts it; the
/// whole estimate never below what testnet took for the probe's collection.
#[test]
fn the_fee_is_exact_and_never_below_what_was_taken() {
    let draft = templates::nft_collection(Some("evolite probe collection".into()));
    let built = build(&draft, owner(), testnet()).unwrap();
    let fee = fee::estimate(&built, testnet()).unwrap();
    assert_eq!(
        fee.registration(),
        19_000_000_000,
        "0.1 + 3 × 0.02 + 3 × 0.01"
    );
    assert_eq!(
        (
            fee.base,
            fee.document_types,
            fee.indexes,
            fee.tokens,
            fee.keywords
        ),
        (10_000_000_000, 6_000_000_000, 3_000_000_000, 0, 0)
    );
    let taken: u64 = 19_163_897_000;
    assert!(fee.total() >= taken, "{fee:?} below {taken}");
    assert!(
        fee.total() - taken < 500_000_000,
        "within 0.005 of it: {fee:?}"
    );
}

/// FEAT-0019's token probe: 0.40 registration, 401 bytes, 0.00186311 on top.
#[test]
fn the_storage_covers_the_token_probe() {
    let fee = Fee {
        base: 10_000_000_000,
        tokens: 30_000_000_000,
        storage: fee::storage(401),
        ..Fee::default()
    };
    assert!(fee.total() >= 40_186_311_000, "{fee:?}");
}

/// The probe: testnet refuses `$price` in an index though DPP passes it offline.
#[test]
fn price_in_an_index_is_refused_before_signing() {
    let mut draft = templates::nft_collection(None);
    draft.documents["nft"]["indices"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "byPrice", "properties": [ { "imageId": "asc" }, { "$price": "asc" } ] }));
    assert_eq!(
        build(&draft, owner(), testnet()).err(),
        Some(Refusal::SystemFieldInIndex {
            document_type: "nft".into(),
            field: "$price".into()
        })
    );
}

/// FEAT-0019 АРХ-13: keywords lowercase, 3–50 characters, no spaces or control characters, no
/// repeats; at most 5 (architect, 2026-11-20).
#[test]
fn keywords_follow_platforms_rules() {
    assert_eq!(
        rules::keywords(&words(&["NFT", "Evolite", "abc"])),
        Ok(words(&["nft", "evolite", "abc"]))
    );
    assert_eq!(
        rules::keywords(&words(&["ab"])),
        Err(KeywordRefusal::Length {
            keyword: "ab".into(),
            chars: 2
        })
    );
    let fifty = "x".repeat(50);
    assert!(rules::keywords(std::slice::from_ref(&fifty)).is_ok());
    assert!(matches!(
        rules::keywords(&[format!("{fifty}x")]),
        Err(KeywordRefusal::Length { chars: 51, .. })
    ));
    assert_eq!(
        rules::keywords(&words(&["two words"])),
        Err(KeywordRefusal::Character {
            keyword: "two words".into()
        })
    );
    assert_eq!(
        rules::keywords(&words(&["tab\there"])),
        Err(KeywordRefusal::Character {
            keyword: "tab\there".into()
        })
    );
    assert_eq!(
        rules::keywords(&words(&["Token", "token"])),
        Err(KeywordRefusal::Repeated {
            keyword: "token".into()
        })
    );
    // At most 5: each costs 0.1 DASH.
    let many: Vec<String> = (0..6).map(|i| format!("word{i}")).collect();
    assert_eq!(rules::keywords(&many), Err(KeywordRefusal::TooMany(6)));
    assert!(rules::keywords(&many[..5]).is_ok());
}

#[test]
fn the_description_is_three_to_a_hundred_characters() {
    assert_eq!(rules::description(None), Ok(None));
    assert_eq!(rules::description(Some("")), Ok(None));
    assert_eq!(
        rules::description(Some("ab")),
        Err(Refusal::DescriptionLength(2))
    );
    assert_eq!(rules::description(Some("abc")), Ok(Some("abc".into())));
    // Characters, not bytes.
    let hundred = "я".repeat(100);
    assert_eq!(
        rules::description(Some(&hundred)),
        Ok(Some(hundred.clone()))
    );
    assert_eq!(
        rules::description(Some(&format!("{hundred}я"))),
        Err(Refusal::DescriptionLength(101))
    );
}

/// The keywords and description go into the contract (and its fee: 0.1 a keyword) as checked.
#[test]
fn keywords_and_description_are_in_the_contract_and_its_fee() {
    let mut draft = templates::nft_collection(Some("Probe collection".into()));
    draft.keywords = words(&["NFT", "Probe"]);
    let built = build(&draft, owner(), testnet()).unwrap();
    assert_eq!(built.keywords_json.as_deref(), Some(r#"["nft","probe"]"#));
    assert_eq!(built.description.as_deref(), Some("Probe collection"));
    assert_eq!(built.contract.keywords(), &words(&["nft", "probe"]));
    let fee = fee::estimate(&built, testnet()).unwrap();
    assert_eq!(fee.registration(), 19_000_000_000 + 2 * 10_000_000_000);
    assert_eq!(fee.keywords, 2 * 10_000_000_000);
}

/// An edit of a draft's document types, for a case of a test.
type Cut = Box<dyn Fn(&mut serde_json::Value)>;

/// What the network refuses that the offline test showed DPP refusing (2026-11-20), each
/// with the refusal [`rules`] gives.
fn network_refuses() -> Vec<(&'static str, Cut, Refusal)> {
    let nft = || "nft".to_owned();
    vec![
        (
            "$price in an index",
            Box::new(|d: &mut serde_json::Value| {
                d["nft"]["indices"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({ "name": "byPrice", "properties": [ { "$price": "asc" } ] }))
            }),
            Refusal::SystemFieldInIndex {
                document_type: nft(),
                field: "$price".into(),
            },
        ),
        (
            "an index on a field the type lacks",
            Box::new(|d: &mut serde_json::Value| {
                d["nft"]["indices"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({ "name": "byNothing", "properties": [ { "nosuch": "asc" } ] }))
            }),
            Refusal::IndexOnMissingField {
                document_type: nft(),
                field: "nosuch".into(),
            },
        ),
        (
            "a repeated index name",
            Box::new(|d: &mut serde_json::Value| {
                d["nft"]["indices"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({ "name": "byOwner", "properties": [ { "serial": "asc" } ] }))
            }),
            Refusal::RepeatedIndexName {
                document_type: nft(),
                name: "byOwner".into(),
            },
        ),
        (
            "an unknown key",
            Box::new(|d: &mut serde_json::Value| d["nft"]["nosuchflag"] = json!(true)),
            Refusal::UnknownKey {
                document_type: nft(),
                key: "nosuchflag".into(),
            },
        ),
    ]
}

/// The release's defence (dpp's `validation` is for tests only): each refusal comes from
/// [`rules`] itself, not from DPP — in tests the feature is on and DPP would refuse too.
#[test]
fn the_rules_refuse_what_the_network_refuses() {
    for (label, cut, refusal) in network_refuses() {
        let mut draft = templates::nft_collection(None);
        cut(&mut draft.documents);
        assert_eq!(
            rules::schema_shape(&draft.documents),
            Err(refusal.clone()),
            "{label}"
        );
        assert_eq!(
            build(&draft, owner(), testnet()).err(),
            Some(refusal),
            "{label}"
        );
    }
    assert_eq!(
        rules::schema_shape(&templates::nft_collection(None).documents),
        Ok(())
    );
}

/// Under dev-dependencies (the `validation` feature): DPP itself refuses each of them offline
/// at the network's version, past the rules — the list the rules keep is the network's.
#[test]
fn dpp_refuses_offline_what_the_network_refuses() {
    for (label, cut, _) in network_refuses() {
        let mut draft = templates::nft_collection(None);
        cut(&mut draft.documents);
        let refused = super::assemble(&draft, Vec::new(), None, owner(), testnet());
        assert!(
            matches!(refused, Err(Refusal::Invalid(_))),
            "{label}: {:?}",
            refused.map(|b| b.bytes)
        );
    }
    // A rule on a field the image lacks: DPP's own, in every build.
    let mut draft = templates::nft_collection(None);
    draft.documents["nft"]["propertyConstraints"]["serialWithinSupply"] =
        json!({ "lessThanOrEqual": [ "serial", { "referenced": ["imageId", "nosuch"] } ] });
    match build(&draft, owner(), testnet()) {
        Err(Refusal::Invalid(e)) => assert!(e.contains("serialWithinSupply"), "{e}"),
        other => panic!("{:?}", other.map(|b| b.bytes)),
    }
}

/// The network's version, not the newest: 14 is 14; an unknown one is refused. Protocol 13's
/// tables refuse the profile (no `refersTo`, no `propertyConstraints` before 14), which is why
/// the version must be the one a proved answer named, not the SDK's seed.
#[test]
fn the_version_is_the_networks() {
    assert_eq!(version_for(14).unwrap().protocol_version, 14);
    let at_13 = build(
        &templates::nft_collection(None),
        owner(),
        version_for(13).unwrap(),
    );
    assert!(matches!(at_13, Err(Refusal::Invalid(e)) if e.contains("refersTo")));
    assert!(version_for(9_999).is_err());
}

/// Before protocol 9 a registration cost nothing; from it on, the parts add up to Platform's
/// `registration_cost`, at testnet's protocol and the newest.
#[test]
fn the_parts_add_up_to_platforms_cost_at_each_version() {
    let mut draft = templates::nft_collection(Some("Probe collection".into()));
    draft.keywords = words(&["nft"]);
    for pv in [testnet(), PlatformVersion::latest()] {
        let built = build(&draft, owner(), pv).unwrap();
        let fee = fee::estimate(&built, pv).unwrap();
        assert_eq!(
            fee.registration(),
            built.contract.registration_cost(pv).unwrap()
        );
        assert_eq!(fee.per_keyword, 10_000_000_000, "0.1 a keyword");
        assert_eq!(fee.keywords, 10_000_000_000);
    }
    let free = (1..=14)
        .filter_map(|p| PlatformVersion::get(p).ok())
        .find(|pv| pv.dpp.contract_versions.methods.registration_cost == 0)
        .expect("a protocol before registration costs");
    let built = build(&draft, owner(), testnet()).unwrap();
    let fee = fee::estimate(&built, free).unwrap();
    assert_eq!(fee.registration(), 0, "protocol {}", free.protocol_version);
    assert_eq!(fee.total(), fee.storage);
}

/// The architect's schema decisions (2026-11-20), checked by DPP's document validation at the
/// verified version: an image's description of 280 characters, and a collectionInfo without
/// `mainContract` (optional: a reader without it takes the collection as one contract).
#[test]
fn the_schema_takes_a_long_description_and_no_main_contract() {
    use dpp::data_contract::document_type::methods::DocumentTypeV0Methods;
    use dpp::data_contract::validate_document::DataContractDocumentValidationMethodsV0;
    use dpp::platform_value::{Value, platform_value};
    let pv = testnet();
    let built = build(&templates::nft_collection(None), owner(), pv).unwrap();
    let c = &built.contract;
    let valid = |name: &str, data: Value| {
        let doc = c
            .document_type_for_name(name)
            .unwrap()
            .create_document_from_data(data, owner(), 1, 1, [7; 32], pv)
            .unwrap();
        c.validate_document(name, &doc, pv).unwrap()
    };
    let image = |description: String| {
        platform_value!({
            "name": "Model",
            "description": description,
            "imageType": 0u8,
            "supply": 10u32,
            "image": Value::Bytes(b"<svg/>".to_vec()),
        })
    };
    let r = valid("image", image("я".repeat(280)));
    assert!(r.is_valid(), "280 characters: {:?}", r.errors);
    let r = valid("image", image("я".repeat(281)));
    assert!(!r.is_valid(), "281 characters are refused");
    let r = valid("collectionInfo", platform_value!({}));
    assert!(r.is_valid(), "no mainContract: {:?}", r.errors);
}

/// FEAT-0038 V1 r1 Must 2: the template, built and fully checked at the network's version, is
/// kept by the contracts cache and read back from its file (a cache opened anew) whole — the
/// read path does not check it against the newest version's schema, so a contract lawful at
/// the network's is never refused there.
#[test]
fn the_template_comes_back_from_the_cache_file_whole() {
    use crate::platform::contracts::ContractCache;
    let built = build(&templates::nft_collection(None), owner(), testnet()).unwrap();
    let dir = std::env::temp_dir().join(format!("evolite-v1-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("contracts").join("testnet").join("contracts.db");
    let contract = std::sync::Arc::new(built.contract.clone());
    ContractCache::new(path.clone())
        .put(contract.clone(), 1_000)
        .unwrap();
    let again = ContractCache::new(path);
    let back = again.get(&contract.id()).expect("read from the file");
    assert_eq!(back.contract.document_types().len(), 3);
    assert_eq!(back.contract.version(), contract.version());
    assert_eq!(
        crate::platform::tokens::info::contract_bytes(&back.contract).unwrap(),
        crate::platform::tokens::info::contract_bytes(&contract).unwrap()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
