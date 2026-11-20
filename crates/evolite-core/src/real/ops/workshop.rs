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
        self.answer_when_platform_ready("task:read_workshop", not_synced, async move {
            let read = {
                let ask = ask.clone();
                async move { read(&chain, &state, &journal, &identity, ask).await }
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
