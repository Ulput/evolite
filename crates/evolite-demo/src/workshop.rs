//! The Workshop's example reading (FEAT-0038 item 2): one collection with one model, a few
//! instances (some for sale) and one sale. Fixed; nothing is asked of a network.

use evolite_api::workshop::{
    Collection, Count, Instance, Instances, Model, ModelCounts, Sale, WorkshopAsk, WorkshopRead,
};

pub const CONTRACT: &str = "FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P";
pub const MODEL: &str = "7Yowk46VwwHqmD5yZyyygggh937aP6h2UW7aQWBdWpM5";
const AT: i64 = 1_763_600_000;

/// The example's instances: numbers 1–12, every third for sale at 0.001 DASH in credits.
fn instances() -> Vec<Instance> {
    (1..=12)
        .map(|n| Instance {
            id: format!("demo-instance-{n}"),
            serial: n,
            price: (n % 3 == 0).then_some(100_000_000),
        })
        .collect()
}

/// A model's picture's room in its write, in the demo (the real core measures it).
pub const ROOM: usize = 19_000;

/// PNG, JPEG, GIF or WebP by the first bytes: what «Make a smaller copy» takes (R-21).
pub fn is_raster(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG")
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
}

/// The id of the demo's `n`-th collection «registered» in this run (W2): base58, 44 letters.
pub fn made_id(n: usize) -> String {
    format!("DemoNewCo{n:0>35}").replace('0', "1")
}

/// «Belongs to a contract» in the demo (W2): an id of base58 letters, 32–44 of them, is the
/// example game; anything else is not an id.
pub fn main_contract(id: &str) -> Result<String, evolite_api::contracts::ContractRefusal> {
    let base58 = |c: char| c.is_ascii_alphanumeric() && !"0OIl".contains(c);
    let id = id.trim();
    if (32..=44).contains(&id.len()) && id.chars().all(base58) {
        Ok("City game: items and rewards".into())
    } else {
        Err(evolite_api::contracts::ContractRefusal::MainContractId)
    }
}

pub fn read(ask: &WorkshopAsk, made: &[(String, String)]) -> WorkshopRead {
    match ask {
        WorkshopAsk::Collections => WorkshopRead::Collections(
            std::iter::once(Collection {
                contract: CONTRACT.into(),
                description: "Seasons of the city".into(),
                open: false,
                limits: Vec::new(),
                models: vec![Model {
                    id: MODEL.into(),
                    name: "Winter tram".into(),
                    description: "A tram in the snow, night".into(),
                    art: String::new(),
                    supply: 100,
                }],
                checked_at: AT,
            })
            .chain(made.iter().map(|(id, name)| Collection {
                contract: id.clone(),
                description: name.clone(),
                open: false,
                limits: Vec::new(),
                models: Vec::new(),
                checked_at: AT,
            }))
            .collect(),
        ),
        WorkshopAsk::Counts { contract, model } => {
            let all = instances();
            WorkshopRead::Counts(ModelCounts {
                contract: contract.clone(),
                model: model.clone(),
                mine: Some(Count::Proved(all.len() as u64)),
                for_sale: Some(Count::Counted {
                    count: all.iter().filter(|i| i.price.is_some()).count() as u64,
                    documents: all.len() as u64,
                    complete: true,
                }),
                prices: vec![(100_000_000, 4)],
                sold: Some(1),
                issued: Some(13),
                checked_at: AT,
            })
        }
        WorkshopAsk::Instances {
            contract,
            model,
            for_sale,
            ..
        } => WorkshopRead::Instances(Instances {
            contract: contract.clone(),
            model: model.clone(),
            for_sale: *for_sale,
            instances: instances()
                .into_iter()
                .filter(|i| !for_sale || i.price.is_some())
                .collect(),
            next: None,
        }),
        WorkshopAsk::Sales => WorkshopRead::Sales(vec![Sale {
            purchase: "demo-purchase-1".into(),
            contract: CONTRACT.into(),
            instance: "demo-instance-13".into(),
            model_id: Some(MODEL.into()),
            model: Some("Winter tram".into()),
            serial: Some(13),
            buyer: "demo-buyer".into(),
            buyer_name: Some("alice".into()),
            price: 100_000_000,
            at: AT,
        }]),
    }
}
