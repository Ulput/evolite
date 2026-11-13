//! The journal's price record (TECH-price-check): a spend is noted when it is asked, with the
//! price of the snapshot the person saw, and written when its answer comes, with what the wallet's
//! history says it cost at that moment.

use super::*;
use crate::diag::Record;
use crate::price::{self, Seen};
use journal::{Entry, What};
use std::collections::BTreeMap;

/// More than this waiting means answers were lost: the oldest goes.
const MOST_WAITING: usize = 32;

/// Which answer ends a spend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Sent(u64),
    Done(DoneWhat),
}

/// Which lines of the wallet's history a spend writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Credits,
    ContactRequest,
    Profile,
    Name,
    Shield,
}

impl Line {
    fn of(op: &Spend) -> Option<Self> {
        match op {
            Spend::Send {
                from: Source::Credits,
                ..
            } => Some(Line::Credits),
            Spend::AddContact { .. } | Spend::AcceptContact { .. } => Some(Line::ContactRequest),
            Spend::SaveProfile { .. } => Some(Line::Profile),
            Spend::AddName { .. } => Some(Line::Name),
            Spend::Shield {
                from: ShieldSource::Credits | ShieldSource::PlatformAddress,
                ..
            } => Some(Line::Shield),
            _ => None,
        }
    }

    fn is(self, what: &What) -> bool {
        matches!(
            (self, what),
            (
                Line::Credits,
                What::Credits { .. } | What::Withdrawal { .. }
            ) | (Line::ContactRequest, What::ContactRequest { .. })
                | (Line::Profile, What::Profile)
                | (Line::Name, What::Name { .. })
                | (Line::Shield, What::Shield { .. })
        )
    }
}

#[derive(Debug)]
struct Waiting {
    answer: Answer,
    started: Timestamp,
    declared: price::Declared,
    line: Option<Line>,
}

#[derive(Default)]
struct Watch {
    seen: Option<Seen>,
    /// The cost each app's request showed, until it is answered.
    app_costs: BTreeMap<u64, Duffs>,
    waiting: Vec<Waiting>,
}

/// Shared by the core's loop and the tasks that answer spends.
#[derive(Clone)]
pub(super) struct PriceWatch {
    watch: Arc<std::sync::Mutex<Watch>>,
    journal: Arc<journal::Journal>,
}

/// What the newest of this spend's lines since it `started` cost above what it moved; `None`
/// when there is none or the wallet could not tell (`spent` 0, or all of it moved).
fn cost_in_history(entries: &[Entry], line: Line, started: Timestamp) -> Option<Duffs> {
    let newest = entries
        .iter()
        .filter(|e| e.source == journal::Source::Here && e.at >= started && line.is(&e.what))
        .max_by_key(|e| e.at)?;
    let fee = newest.spent.saturating_sub(newest.moved);
    (newest.spent > 0 && fee > 0).then_some(fee)
}

impl PriceWatch {
    pub(super) fn new(journal: Arc<journal::Journal>) -> Self {
        Self {
            watch: Default::default(),
            journal,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Watch> {
        self.watch.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The snapshot going to the window: what the next spend is judged by.
    pub(super) fn saw(&self, wallet: Option<&Wallet>) {
        self.lock().seen = wallet.map(Seen::of);
    }

    /// A spend past the password gate. Nothing is noted without a snapshot: no form was seen.
    pub(super) fn begin(&self, op: &Spend, started: Timestamp) {
        let mut watch = self.lock();
        let Some(seen) = &watch.seen else {
            return;
        };
        let app_cost = match op {
            Spend::ApproveApp { id, .. } => watch.app_costs.get(id).copied().unwrap_or(0),
            _ => 0,
        };
        let declared = price::declared(op, seen, app_cost);
        let answer = match op {
            Spend::Send { id, .. } | Spend::SendMixed { id, .. } => Answer::Sent(*id),
            other => Answer::Done(other.what()),
        };
        if watch.waiting.len() >= MOST_WAITING {
            watch.waiting.remove(0);
        }
        watch.waiting.push(Waiting {
            answer,
            started,
            declared,
            line: Line::of(op),
        });
    }

    /// An answer going to the window: the record of the spend it ends, if any.
    fn ended(&self, event: &Event) -> Option<Record> {
        let (answer, failure, fee) = match event {
            Event::AppRequest(Ok(request)) => {
                let mut watch = self.lock();
                let cost = price::app_cost(&request.kind);
                watch.app_costs.insert(request.id, cost);
                return None;
            }
            Event::Sent { id, result } => (
                Answer::Sent(*id),
                result.as_ref().err(),
                result.as_ref().ok().map_or(0, |paid| paid.fee),
            ),
            Event::Done { what, result } => (Answer::Done(*what), result.as_ref().err(), 0),
            _ => return None,
        };
        let spend = {
            let mut watch = self.lock();
            let at = watch.waiting.iter().position(|w| w.answer == answer)?;
            watch.waiting.remove(at)
        };
        let actual = match (failure, fee > 0, spend.line) {
            (Some(_), ..) => None,
            // A payment's own answer knows its fee.
            (None, true, _) => Some(fee),
            (None, false, Some(line)) => {
                cost_in_history(&self.journal.entries(), line, spend.started)
            }
            (None, false, None) => None,
        };
        Some(Record::Price {
            what: spend.declared.what,
            declared: spend.declared.declared,
            actual,
            enough: spend.declared.enough,
            reason: failure.map_or("ok", price::reason),
        })
    }

    /// Writes the record of the spend `event` ends.
    pub(super) fn finish(&self, event: &Event) {
        if let Some(record) = self.ended(event) {
            crate::diag::record(record);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::price::tests::{every_spend, seen};

    fn watch() -> (PriceWatch, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "evolite-prices-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let journal = Arc::new(journal::Journal::new(dir.join("history.json")));
        (PriceWatch::new(journal), dir)
    }

    fn seen_by(watch: &PriceWatch, balance: Duffs) {
        watch.lock().seen = Some(seen(balance));
    }

    /// The answer each spend gets when it fails.
    fn refused(op: Spend) -> Event {
        op.refused(CoreError::NotEnough)
    }

    /// AC-T1: every kind of spend leaves a record, whichever way it is answered.
    #[test]
    fn every_spend_leaves_a_price_record() {
        for op in every_spend() {
            let (watch, dir) = watch();
            seen_by(&watch, 10_000);
            watch.begin(&op, 1);
            let record = watch.ended(&refused(op.clone()));
            let Some(Record::Price { what, reason, .. }) = record else {
                panic!("no record for {op:?}: {record:?}");
            };
            assert_eq!(reason, "not_enough", "{what}");
            assert!(watch.lock().waiting.is_empty(), "{what}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_line_in_the_history_is_the_actual_price() {
        let (watch, dir) = watch();
        seen_by(&watch, 10_000);
        let op = Spend::Send {
            coins: None,
            id: 9,
            from: Source::Credits,
            to: "@bob".into(),
            amount: 100,
            note: None,
            rate: None,
        };
        watch.begin(&op, 1_000);
        watch.journal.add(Entry {
            at: 1_000,
            what: What::Credits { to: "@bob".into() },
            spent: 2_999,
            moved: 100,
            ..Default::default()
        });
        let record = watch.ended(&Event::Sent {
            id: 9,
            result: Ok(SentPayment {
                txid: String::new(),
                amount: 100,
                fee: 0,
                relayed: true,
            }),
        });
        assert_eq!(
            record,
            Some(Record::Price {
                what: "send_credits",
                declared: 12,
                actual: Some(2_899),
                enough: true,
                reason: "ok",
            })
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An older line of the same kind is another spend's; `spent` unknown is no price at all.
    #[test]
    fn only_a_known_cost_since_the_spend_counts() {
        let line = Line::Name;
        let name = |at, spent, source| Entry {
            at,
            what: What::Name { label: "x".into() },
            spent,
            source,
            ..Default::default()
        };
        let here = journal::Source::Here;
        assert_eq!(cost_in_history(&[name(5, 90, here)], line, 10), None);
        assert_eq!(cost_in_history(&[name(11, 0, here)], line, 10), None);
        assert_eq!(
            cost_in_history(&[name(11, 90, journal::Source::Network)], line, 10),
            None
        );
        assert_eq!(
            cost_in_history(&[name(11, 90, here), name(12, 70, here)], line, 10),
            Some(70)
        );
        let other = Entry {
            at: 11,
            what: What::Profile,
            spent: 90,
            ..Default::default()
        };
        assert_eq!(cost_in_history(&[other], line, 10), None);
    }

    #[test]
    fn a_payments_own_fee_is_its_actual_price() {
        let (watch, dir) = watch();
        seen_by(&watch, 10_000);
        let op = Spend::Send {
            coins: None,
            id: 3,
            from: Source::Core,
            to: "yAddress".into(),
            amount: 100,
            note: None,
            rate: None,
        };
        watch.begin(&op, 1);
        let record = watch.ended(&Event::Sent {
            id: 3,
            result: Ok(SentPayment {
                txid: "tx".into(),
                amount: 100,
                fee: 226,
                relayed: true,
            }),
        });
        assert!(
            matches!(
                record,
                Some(Record::Price {
                    actual: Some(226),
                    what: "send_core",
                    ..
                })
            ),
            "{record:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_app_is_judged_by_the_cost_its_request_showed() {
        let (watch, dir) = watch();
        seen_by(&watch, 10_000);
        watch.ended(&Event::AppRequest(Ok(evolite_api::AppRequest {
            id: 7,
            app: "yappr".into(),
            kind: evolite_api::AppRequestKind::Connect2 {
                app_id: String::new(),
                label: String::new(),
                owner: String::new(),
                owner_name: None,
                identity: String::new(),
                sessions: Vec::new(),
                encryption: Vec::new(),
                limits: evolite_api::SessionLimits::wallet(),
                breaks_old_clients: false,
                renews: None,
                created_at: 0,
                callback_host: None,
                cost: 77,
            },
        })));
        watch.begin(&Spend::ApproveApp { id: 7, grant: None }, 1);
        let record =
            watch.ended(&Spend::ApproveApp { id: 7, grant: None }.refused(CoreError::BadLink));
        assert!(
            matches!(record, Some(Record::Price { declared: 77, .. })),
            "{record:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Answers that end no spend of ours, and spends with no form seen, write nothing.
    #[test]
    fn other_answers_write_no_record() {
        let (watch, dir) = watch();
        // Hiding a request is answered as `Contact`, and nothing is waiting.
        seen_by(&watch, 10_000);
        let hide = Event::Done {
            what: DoneWhat::Contact,
            result: Ok(()),
        };
        assert_eq!(watch.ended(&hide), None);
        // No snapshot yet.
        watch.lock().seen = None;
        watch.begin(&Spend::Unshield { amount: 1 }, 1);
        assert!(watch.lock().waiting.is_empty());
        // The waiting list does not grow without end.
        seen_by(&watch, 10_000);
        for _ in 0..MOST_WAITING + 5 {
            watch.begin(&Spend::Unshield { amount: 1 }, 1);
        }
        assert_eq!(watch.lock().waiting.len(), MOST_WAITING);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Review r1, Should 1: with several spends waiting, an answer ends its own and no other.
    #[test]
    fn an_answer_ends_its_own_spend_and_no_other() {
        let (watch, dir) = watch();
        seen_by(&watch, 1_000_000_000);
        let name = Spend::AddName { name: "a".into() };
        let unshield = Spend::Unshield { amount: 1 };
        let send = |id| Spend::Send {
            coins: None,
            id,
            from: Source::Core,
            to: "yAddress".into(),
            amount: 1,
            note: None,
            rate: None,
        };
        watch.begin(&name, 1);
        watch.begin(&unshield, 1);
        watch.begin(&send(1), 1);
        watch.begin(&send(2), 1);
        let waiting = |watch: &PriceWatch| -> Vec<Answer> {
            watch.lock().waiting.iter().map(|w| w.answer).collect()
        };
        let what_of = |record: Option<Record>| match record {
            Some(Record::Price { what, .. }) => what,
            other => panic!("not a price record: {other:?}"),
        };

        // The second waiting: the first stays.
        let record = watch.ended(&unshield.clone().refused(CoreError::NotEnough));
        assert_eq!(what_of(record), "unshield");
        assert_eq!(
            waiting(&watch),
            [
                Answer::Done(DoneWhat::Name),
                Answer::Sent(1),
                Answer::Sent(2)
            ]
        );

        // A payment by its own number, not the first payment waiting.
        let record = watch.ended(&send(2).refused(CoreError::NotEnough));
        assert_eq!(what_of(record), "send_core");
        assert_eq!(
            waiting(&watch),
            [Answer::Done(DoneWhat::Name), Answer::Sent(1)]
        );

        // Answers to nothing waiting end nothing.
        assert_eq!(watch.ended(&send(9).refused(CoreError::NotEnough)), None);
        assert_eq!(
            watch.ended(&unshield.clone().refused(CoreError::NotEnough)),
            None
        );
        assert_eq!(waiting(&watch).len(), 2);

        // Two of one kind: the older first.
        watch.begin(&unshield, 5);
        watch.begin(&unshield, 6);
        watch.ended(&unshield.clone().refused(CoreError::NotEnough));
        assert_eq!(watch.lock().waiting.last().map(|w| w.started), Some(6));
        let _ = std::fs::remove_dir_all(dir);
    }
}
