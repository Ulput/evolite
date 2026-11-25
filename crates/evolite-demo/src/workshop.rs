//! The Workshop's example reading (FEAT-0038 item 2): one collection with one model, a few
//! instances (some for sale) and one sale. Fixed; nothing is asked of a network.

use evolite_api::workshop::{
    Collection, Count, Given, Instance, Instances, Model, ModelCounts, Sale, WorkshopAsk,
    WorkshopRead,
};

pub const CONTRACT: &str = "FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P";
pub const MODEL: &str = "7Yowk46VwwHqmD5yZyyygggh937aP6h2UW7aQWBdWpM5";
/// BUG-266: a collection of this name is checked «not synced» the first time, as when the sync
/// ends after the form was opened; the next check answers.
pub const SYNC_PENDING: &str = "Sync pending";
pub(crate) const AT: i64 = 1_763_600_000;
/// BUG-312: the example's numbers add up — 14 issued: 12 held (1–12), no. 13 given to @alice,
/// no. 14 sold to @alice.
const ISSUED: u32 = 14;
/// The copy sold, its purchase and its price (1 DASH; the History line says the same).
pub(crate) const SOLD_SERIAL: u32 = 14;
pub(crate) const PURCHASE: &str = "demo-purchase-1";
pub(crate) const PRICE: evolite_api::Duffs = 100_000_000;
/// What the gift of no. 13 took: one transfer, as the demo's plan of a gift estimates it.
pub(crate) const GIFT_FEE: evolite_api::Duffs = 145_000;

/// The example's instances: numbers 1–12, every third for sale at 1 DASH.
fn instances() -> Vec<Instance> {
    (1..=12)
        .map(|n| Instance {
            id: format!("demo-instance-{n}"),
            serial: n,
            price: (n % 3 == 0).then_some(PRICE),
        })
        .collect()
}

/// BUG-313: the example collection's instances as the Collection shows them — the same list
/// the Workshop counts ([`instances`]): «you have 12, 4 for sale» on both screens.
pub fn own_nfts(owner: &str, owner_id: &str) -> Vec<evolite_api::Nft> {
    instances()
        .into_iter()
        .map(|i| evolite_api::Nft {
            id: i.id,
            name: "Winter tram".into(),
            description: "A tram in the snow, night".into(),
            // The model's picture (BUG-312), as the Workshop draws it.
            art: crate::demo::art("winter-tram"),
            art_verified: true,
            origin: evolite_api::NftOrigin::Release,
            issuer: owner.into(),
            contract: CONTRACT.into(),
            edition: (Some(i.serial), 100),
            owner: owner.into(),
            owner_id: owner_id.into(),
            price: i.price,
            // The wallet's own collection: its pictures drawn without asking.
            chosen: true,
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

/// The example's reading; `sales_read`: `Sales` was asked (BUG-314) — before it «Sold» is not
/// known, as the real core's count of sales is the sales it read.
pub fn read(ask: &WorkshopAsk, made: &[(String, String)], sales_read: bool) -> WorkshopRead {
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
                    // BUG-312: the one model of the demo without a picture had none.
                    art: crate::demo::art("winter-tram"),
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
                prices: vec![(PRICE, 4)],
                sold: sales_read.then_some(1),
                // BUG-276: the one sale of the demo's History, at its price.
                sold_for: sales_read.then_some(PRICE),
                issued: Some(ISSUED),
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
        // W7 «Given»: one gift to @alice; the example's «take off sale» (a transfer to oneself)
        // and a holder's resale are not gifts (the real core's filter: `workshop_read::gift_of`).
        WorkshopAsk::Given { contract, model } => WorkshopRead::Given(Given {
            contract: contract.clone(),
            model: model.clone(),
            count: Count::Proved(1),
            last: Some((AT, "@alice".into())),
            this_computer_only: false,
        }),
        WorkshopAsk::Sales => WorkshopRead::Sales(vec![Sale {
            purchase: PURCHASE.into(),
            contract: CONTRACT.into(),
            instance: format!("demo-instance-{SOLD_SERIAL}"),
            model_id: Some(MODEL.into()),
            model: Some("Winter tram".into()),
            serial: Some(SOLD_SERIAL),
            buyer: "demo-buyer".into(),
            buyer_name: Some("alice".into()),
            price: PRICE,
            at: AT,
        }]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-313 (two sources of one number): the demo wallet's Collection holds as many of the
    /// example model's NFTs, and as many for sale, as the Workshop counts.
    #[test]
    fn the_collection_and_the_workshop_count_the_same_nfts() {
        let wallet = crate::demo::demo_wallet(0);
        let mine: Vec<_> = wallet
            .nfts
            .iter()
            .filter(|n| n.contract == CONTRACT)
            .collect();
        let WorkshopRead::Counts(counts) = read(
            &WorkshopAsk::Counts {
                contract: CONTRACT.into(),
                model: MODEL.into(),
            },
            &[],
        ) else {
            panic!("counts");
        };
        assert_eq!(counts.mine, Some(Count::Proved(mine.len() as u64)));
        let for_sale = mine.iter().filter(|n| n.price.is_some()).count() as u64;
        assert!(
            matches!(counts.for_sale, Some(Count::Counted { count, .. }) if count == for_sale),
            "{:?} for sale, the Collection {for_sale}",
            counts.for_sale
        );
    }

    /// BUG-312: every model of the demo has its picture, in the demo's gallery, and its numbers
    /// add up — issued = held + given + sold; the sale is in History, so «Recent» has it.
    #[test]
    fn every_demo_model_has_a_picture_and_numbers_that_add_up() {
        let gallery = crate::demo::demo_art();
        let WorkshopRead::Collections(all) = read(&WorkshopAsk::Collections, &[], true) else {
            panic!("collections")
        };
        let models: Vec<_> = all
            .iter()
            .flat_map(|c| c.models.iter().map(move |m| (c, m)))
            .collect();
        assert!(!models.is_empty());
        for (c, m) in models {
            assert!(!m.art.is_empty(), "{}: no picture", m.name);
            assert!(
                gallery.contains_key(&m.art),
                "{}: not in the gallery",
                m.name
            );
            let ask = |ask| read(&ask, &[], true);
            let WorkshopRead::Counts(n) = ask(WorkshopAsk::Counts {
                contract: c.contract.clone(),
                model: m.id.clone(),
            }) else {
                panic!("counts")
            };
            let WorkshopRead::Given(g) = ask(WorkshopAsk::Given {
                contract: c.contract.clone(),
                model: m.id.clone(),
            }) else {
                panic!("given")
            };
            let held = match n.mine {
                Some(Count::Proved(k)) => k,
                other => panic!("{other:?}"),
            };
            let given = match g.count {
                Count::Proved(k) => k,
                other => panic!("{other:?}"),
            };
            let sold = u64::from(n.sold.unwrap());
            assert_eq!(
                u64::from(n.issued.unwrap()),
                held + given + sold,
                "{}: issued = held + given + sold",
                m.name
            );
            let WorkshopRead::Sales(sales) = ask(WorkshopAsk::Sales) else {
                panic!("sales")
            };
            let these: Vec<_> = sales
                .iter()
                .filter(|s| s.model_id.as_deref() == Some(&m.id))
                .collect();
            assert_eq!(
                these.len() as u32,
                n.sold.unwrap(),
                "{}: the sales read",
                m.name
            );
            assert_eq!(
                these.iter().map(|s| s.price).sum::<evolite_api::Duffs>(),
                n.sold_for.unwrap(),
                "{}: «Sold» sums the sales",
                m.name
            );
            let WorkshopRead::Instances(held_ones) = ask(WorkshopAsk::Instances {
                contract: c.contract.clone(),
                model: m.id.clone(),
                for_sale: false,
                after: None,
            }) else {
                panic!("instances")
            };
            for s in &these {
                assert!(
                    held_ones
                        .instances
                        .iter()
                        .all(|i| Some(i.serial) != s.serial),
                    "{}: no. {:?} sold and still held",
                    m.name,
                    s.serial
                );
            }
            // BUG-307: the gift's series line, one for the gift of no. 13 (R-15).
            let history = crate::demo::demo_wallet(crate::demo::now()).activity;
            let gifts: Vec<_> = history
                .iter()
                .filter(|a| {
                    matches!(&a.kind, evolite_api::ActivityKind::NftSeries {
                    act: evolite_api::series::SeriesAct::Gift { .. }, model: Some(x), done, .. }
                    if *x == m.name && u64::from(*done) == given)
                })
                .collect();
            assert_eq!(gifts.len(), 1, "{}: the gift's one History line", m.name);
            assert!(gifts[0].amount < 0, "{}: what the gift took", m.name);
            // The sale's line in History, the one «Recent» shows (written when the sales are read).
            let wallet = evolite_api::Wallet {
                activity: vec![crate::demo::sale_line()],
                ..crate::demo::demo_wallet(crate::demo::now())
            };
            for s in &these {
                assert!(
                    wallet.activity.iter().any(|a| a.key == format!("nftsold:{}", s.purchase)
                        && matches!(&a.kind, evolite_api::ActivityKind::NftSold { model: Some(x), serial, .. }
                            if *x == m.name && *serial == s.serial)
                        && a.amount == s.price),
                    "{}: the sale's History line",
                    m.name
                );
            }
        }
    }
}

/// BUG-314: «Sold» is not known before the sales are asked — the demo hid that nobody asked.
#[test]
fn sold_is_not_known_before_the_sales_are_read() {
    let counts = |sales_read| {
        let WorkshopRead::Counts(c) = read(
            &WorkshopAsk::Counts {
                contract: CONTRACT.into(),
                model: MODEL.into(),
            },
            &[],
            sales_read,
        ) else {
            panic!("counts")
        };
        (c.sold, c.sold_for)
    };
    assert_eq!(counts(false), (None, None), "«—», not 1");
    assert_eq!(counts(true), (Some(1), Some(PRICE)));
}
