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

pub fn read(ask: &WorkshopAsk) -> WorkshopRead {
    match ask {
        WorkshopAsk::Collections => WorkshopRead::Collections(vec![Collection {
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
        }]),
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
