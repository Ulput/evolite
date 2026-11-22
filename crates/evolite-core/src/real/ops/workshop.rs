//! FEAT-0038 item 2, FEAT-0031 stage 1: the Workshop's reading for the shown identity. Every
//! answer is [`Event::Workshop`] of the ask; nothing here spends. Sales are kept per identity
//! with where their reading stands, so a sale is read once (AC-T-W4) and a model's «Продано» is
//! counted from them.

use std::collections::HashMap;
use std::sync::Mutex;

use evolite_api::workshop::{Sale, WorkshopAsk, WorkshopRead};
use evolite_chain::workshop_read::SalesCursor;
use evolite_local::journal;

use crate::real::ops::names::read_within;
use crate::real::*;

/// How long one Workshop read may take.
const WORKSHOP_WAIT: Duration = Duration::from_secs(60);

/// Each identity's sales read so far, and where their reading stands.
#[derive(Clone, Default)]
pub(in crate::real) struct WorkshopState(Arc<Mutex<HashMap<String, Sold>>>);

#[derive(Clone, Default)]
struct Sold {
    cursor: SalesCursor,
    sales: Vec<Sale>,
}

impl WorkshopState {
    /// How many of `model` in `contract` `identity` sold, of the sales read; `None` before any.
    fn sold(&self, identity: &str, contract: &str, model: &str) -> Option<u32> {
        let all = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let sold = all.get(identity)?;
        Some(
            sold.sales
                .iter()
                .filter(|s| s.contract == contract && s.model_id.as_deref() == Some(model))
                .count() as u32,
        )
    }

    fn cursor(&self, identity: &str) -> SalesCursor {
        let all = self.0.lock().unwrap_or_else(|e| e.into_inner());
        all.get(identity)
            .map(|s| s.cursor.clone())
            .unwrap_or_default()
    }

    /// Keeps `new` sales after the earlier ones; a sale already kept is not kept again.
    fn add(&self, identity: &str, new: &[Sale], cursor: SalesCursor) {
        let mut all = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let sold = all.entry(identity.to_owned()).or_default();
        for s in new {
            if !sold.sales.iter().any(|k| k.purchase == s.purchase) {
                sold.sales.push(s.clone());
            }
        }
        sold.cursor = cursor;
    }
}

impl Real {
    /// [`Command::ReadWorkshop`]: answered later with [`Event::Workshop`].
    pub(in crate::real) fn read_workshop(&mut self, ask: WorkshopAsk) -> (Option<Event>, bool) {
        let answer = |ask: WorkshopAsk, result| Some(Event::Workshop { ask, result });
        let Some(identity) = self.tokens.shown_identity() else {
            let refused = Err(CoreError::Failed("no identity".into()));
            return (answer(ask, refused), false);
        };
        let not_synced = Event::Workshop {
            ask: ask.clone(),
            result: Err(CoreError::NotSynced),
        };
        let chain = self.chain.clone();
        let (state, journal) = (self.workshop.clone(), self.journal.clone());
        let series = self.nft_series.clone();
        self.answer_when_platform_ready("task:read_workshop", not_synced, async move {
            let read = {
                let ask = ask.clone();
                async move { read(&chain, &state, &journal, &series, &identity, ask).await }
            };
            let result = read_within(WORKSHOP_WAIT, read)
                .await
                .map_err(CoreError::Failed);
            Event::Workshop { ask, result }
        });
        (None, false)
    }
}

async fn read(
    chain: &Arc<Chain>,
    state: &WorkshopState,
    journal: &journal::Journal,
    series: &nft_series::NftSeries,
    identity: &str,
    ask: WorkshopAsk,
) -> Result<WorkshopRead, String> {
    match ask {
        WorkshopAsk::Collections => chain
            .workshop_collections(identity)
            .await
            .map(WorkshopRead::Collections),
        WorkshopAsk::Counts { contract, model } => {
            let sold = state.sold(identity, &contract, &model);
            chain
                .workshop_counts(&contract, &model, identity, sold)
                .await
                .map(WorkshopRead::Counts)
        }
        WorkshopAsk::Instances {
            contract,
            model,
            for_sale,
            after,
        } => chain
            .workshop_instances(&contract, &model, identity, for_sale, after.as_deref())
            .await
            .map(WorkshopRead::Instances),
        WorkshopAsk::Sales => {
            let (new, cursor) = chain
                .workshop_sales(identity, &state.cursor(identity))
                .await?;
            // History's lines first: a sale whose line is not kept is read again next time.
            journal.merge_network(new.iter().map(|s| sale_entry(s, identity)).collect())?;
            state.add(identity, &new, cursor);
            Ok(WorkshopRead::Sales(new))
        }
        WorkshopAsk::Given { contract, model } => {
            let ours = given_here(series, identity, &contract);
            let read = chain
                .workshop_given(identity, &contract, &model, &ours.models)
                .await?;
            Ok(WorkshopRead::Given(given(&contract, &model, &read, &ours)))
        }
    }
}

/// W7 «Given» from this computer's series: each copy a gift series gave (instance → model),
/// and how many and the last gift of each model — the fallback for a collection without
/// transfer history (the architect, 2026-11-22).
#[derive(Default)]
struct GivenHere {
    models: std::collections::BTreeMap<String, String>,
    count: std::collections::BTreeMap<String, u64>,
    last: std::collections::BTreeMap<String, (Timestamp, String)>,
}

fn given_here(series: &nft_series::NftSeries, identity: &str, contract: &str) -> GivenHere {
    let mut here = GivenHere::default();
    for s in series.all() {
        let nft_series::Kind::Gift { to } = &s.kind else {
            continue;
        };
        if s.identity != identity || s.contract != contract {
            continue;
        }
        let done: Vec<_> = s
            .items
            .iter()
            .filter(|i| i.state == nft_series::State::Done)
            .collect();
        for item in &done {
            if let Some(doc) = &item.doc {
                here.models.insert(doc.clone(), s.model.clone());
            }
        }
        if !done.is_empty() {
            *here.count.entry(s.model.clone()).or_default() += done.len() as u64;
            let at = s.created_at as Timestamp;
            let newer = here.last.get(&s.model).is_none_or(|(t, _)| *t < at);
            if newer {
                here.last.insert(s.model.clone(), (at, to.clone()));
            }
        }
    }
    here
}

/// The tile's numbers: from DocumentHistory, or this computer's series when the collection
/// keeps no transfer history.
fn given(
    contract: &str,
    model: &str,
    read: &evolite_chain::workshop_read::GivenRead,
    here: &GivenHere,
) -> evolite_api::workshop::Given {
    use evolite_api::workshop::Count;
    if read.no_history {
        return evolite_api::workshop::Given {
            contract: contract.to_owned(),
            model: model.to_owned(),
            count: Count::Proved(here.count.get(model).copied().unwrap_or(0)),
            last: here.last.get(model).cloned(),
            this_computer_only: true,
        };
    }
    evolite_api::workshop::Given {
        contract: contract.to_owned(),
        model: model.to_owned(),
        count: if read.complete {
            Count::Proved(read.count)
        } else {
            Count::Counted {
                count: read.count,
                documents: read.read,
                complete: false,
            }
        },
        last: read.last.clone(),
        this_computer_only: false,
    }
}

/// A sale as History's line (R-15, AC-T-W4): keyed by its `purchase`, so it is one line however
/// often it is read; the price came in.
fn sale_entry(sale: &Sale, identity: &str) -> journal::Entry {
    let buyer = match &sale.buyer_name {
        Some(name) => format!("@{name}"),
        None => sale.buyer.clone(),
    };
    let kind = journal::Kind::NftSold {
        model: sale.model.clone(),
        serial: sale.serial,
        buyer,
    };
    let mut entry = journal::entry_of(kind, sale.at, -sale.price, Some(identity.to_owned()));
    entry.key = Some(format!("nftsold:{}", sale.purchase));
    entry.source = journal::Source::Network;
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sale(purchase: &str, model: &str) -> Sale {
        Sale {
            purchase: purchase.into(),
            contract: "c".into(),
            instance: "i".into(),
            model_id: Some(model.into()),
            model: Some("M".into()),
            serial: Some(1),
            buyer: "b".into(),
            buyer_name: None,
            price: 10,
            at: 1,
        }
    }

    /// AC-T-W4: a sale read twice counts once; «Продано» is per model.
    #[test]
    fn a_sale_counts_once_per_model() {
        let state = WorkshopState::default();
        assert_eq!(state.sold("me", "c", "m"), None, "not read yet: «—»");
        state.add(
            "me",
            &[sale("p1", "m"), sale("p2", "n")],
            SalesCursor::default(),
        );
        state.add("me", &[sale("p1", "m")], SalesCursor::default());
        assert_eq!(state.sold("me", "c", "m"), Some(1));
        assert_eq!(state.sold("me", "c", "n"), Some(1));
        assert_eq!(state.sold("me", "x", "m"), Some(0));
        assert_eq!(state.sold("other", "c", "m"), None);
    }

    /// R-15, AC-T-W4: a sale's History line is keyed by its `purchase` and brings its price in;
    /// the buyer is shown by name when known.
    #[test]
    fn a_sale_is_a_line_keyed_by_its_purchase() {
        let mut s = sale("p1", "m");
        s.buyer_name = Some("alice".into());
        let e = sale_entry(&s, "me");
        assert_eq!(e.key.as_deref(), Some("nftsold:p1"));
        assert_eq!(e.spent, -10, "the price came in");
        assert_eq!(e.identity.as_deref(), Some("me"));
        let a = journal::activity(&e);
        assert_eq!(a.amount, 10);
        assert_eq!(
            a.kind,
            evolite_api::ActivityKind::NftSold {
                model: Some("M".into()),
                serial: Some(1),
                buyer: "@alice".into(),
            }
        );
        s.buyer_name = None;
        assert!(matches!(
            journal::activity(&sale_entry(&s, "me")).kind,
            evolite_api::ActivityKind::NftSold { buyer, .. } if buyer == "b"
        ));
    }
}

#[cfg(test)]
mod given_tests {
    use super::*;

    /// W7 «Given» (the architect, 2026-11-22): this computer's gift series give the fallback —
    /// the copies given (done only) and the last gift — and each given copy's model, for one
    /// its recipient deleted; a collection without transfer history says «only from this
    /// computer».
    #[test]
    fn a_collection_without_transfer_history_counts_this_computers_gifts() {
        let dir = std::env::temp_dir().join(format!("evolite-given-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let series = nft_series::NftSeries::new(dir.join("nft-series.json"));
        let item = |serial, doc: &str, state| nft_series::Item {
            serial,
            doc: Some(doc.into()),
            state,
            why: None,
            sent_at: None,
            no_since: None,
        };
        let gift = |model: &str, to: &str, at, items| nft_series::Series {
            id: 0,
            identity: "me".into(),
            contract: "c".into(),
            model: model.into(),
            kind: nft_series::Kind::Gift { to: to.into() },
            items,
            created_at: at,
            spent: 0,
            closed: false,
        };
        use nft_series::State::{Done, Refused};
        series
            .start(gift(
                "m",
                "@bob",
                10,
                vec![item(1, "d1", Done), item(2, "d2", Refused)],
            ))
            .unwrap();
        series
            .start(gift("m", "@carol", 20, vec![item(3, "d3", Done)]))
            .unwrap();
        series
            .start(gift("other", "@dave", 30, vec![item(4, "d4", Done)]))
            .unwrap();
        let here = given_here(&series, "me", "c");
        assert_eq!(here.models.get("d1").map(String::as_str), Some("m"));
        assert!(!here.models.contains_key("d2"), "refused: not given");
        let none = evolite_chain::workshop_read::GivenRead {
            count: 0,
            read: 0,
            complete: true,
            last: None,
            no_history: true,
        };
        let shown = given("c", "m", &none, &here);
        assert_eq!(shown.count, evolite_api::workshop::Count::Proved(2));
        assert_eq!(shown.last, Some((20, "@carol".to_owned())));
        assert!(shown.this_computer_only);
        // With the history read, its numbers are the network's.
        let read = evolite_chain::workshop_read::GivenRead {
            count: 7,
            read: 500,
            complete: false,
            last: Some((40, "@erin".into())),
            no_history: false,
        };
        let shown = given("c", "m", &read, &here);
        assert_eq!(
            shown.count,
            evolite_api::workshop::Count::Counted {
                count: 7,
                documents: 500,
                complete: false
            }
        );
        assert!(!shown.this_computer_only);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
