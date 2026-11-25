//! The series on a network that is not one (FEAT-0038 item 1, the architect's conditions).

use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicUsize;

use evolite_local::nft_series::Series;

use super::*;

/// What a write does on the fake network.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Does {
    Answers,
    /// Refused by Platform: nothing changes.
    Refused,
    /// Lands, its answer lost.
    LostLanded,
    /// Its answer lost, not landed (yet: `land_late` may land it).
    LostNotLanded,
    /// «Too many requests», once; then answers.
    SlowOnce,
}

struct Fake {
    me: Identifier,
    held: StdMutex<BTreeMap<u32, Instance>>,
    does: StdMutex<BTreeMap<u32, Does>>,
    credits: StdMutex<u64>,
    writes: StdMutex<BTreeMap<u32, u32>>,
    in_flight: AtomicUsize,
    most_in_flight: AtomicUsize,
    /// Locks the wallet at its first write.
    lock_at_first: Option<Arc<SeriesRun>>,
    /// What the series file says of the write's send time while it is out.
    peek: Option<(Arc<NftSeries>, u64)>,
    seen_sent_at: StdMutex<Option<Option<u64>>>,
}

fn id(n: u8) -> Identifier {
    Identifier::from([n; 32])
}

fn doc(serial: u32) -> Identifier {
    Identifier::from([serial as u8 + 100; 32])
}

impl Fake {
    fn new(free: &[u32]) -> Self {
        let me = id(1);
        let held = free
            .iter()
            .map(|&s| {
                (
                    s,
                    Instance {
                        id: doc(s),
                        serial: s,
                        owner: me,
                        price: None,
                    },
                )
            })
            .collect();
        Fake {
            me,
            held: StdMutex::new(held),
            does: StdMutex::default(),
            credits: StdMutex::new(u64::MAX),
            writes: StdMutex::default(),
            in_flight: AtomicUsize::new(0),
            most_in_flight: AtomicUsize::new(0),
            lock_at_first: None,
            peek: None,
            seen_sent_at: StdMutex::default(),
        }
    }

    fn does(self, serial: u32, does: Does) -> Self {
        self.does.lock().unwrap().insert(serial, does);
        self
    }

    fn apply(&self, write: &Write, serial: u32) {
        let mut held = self.held.lock().unwrap();
        match write {
            Write::Mint => {
                held.insert(
                    serial,
                    Instance {
                        id: doc(serial),
                        serial,
                        owner: self.me,
                        price: None,
                    },
                );
            }
            Write::Price(p) => {
                if let Some(i) = held.get_mut(&serial) {
                    i.price = Some(*p);
                }
            }
            Write::Transfer(to) => {
                if let Some(i) = held.get_mut(&serial) {
                    i.owner = *to;
                    i.price = None;
                }
            }
            Write::Delete => {
                held.remove(&serial);
            }
        }
    }

    fn writes_of(&self, serial: u32) -> u32 {
        self.writes
            .lock()
            .unwrap()
            .get(&serial)
            .copied()
            .unwrap_or(0)
    }

    fn total_writes(&self) -> u32 {
        self.writes.lock().unwrap().values().sum()
    }
}

impl SeriesNet for Fake {
    fn credits(&self) -> Fut<Result<u64, String>> {
        let c = *self.credits.lock().unwrap();
        Box::pin(async move { Ok(c) })
    }

    fn read(&self, serial: u32, _: Option<Identifier>) -> Fut<Result<Option<Instance>, String>> {
        let found = self.held.lock().unwrap().get(&serial).copied();
        Box::pin(async move { Ok(found) })
    }

    fn write(
        &self,
        write: Write,
        serial: u32,
        _: Option<Identifier>,
    ) -> Fut<Result<Option<Identifier>, String>> {
        let n = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_in_flight.fetch_max(n, Ordering::SeqCst);
        let first = {
            let mut w = self.writes.lock().unwrap();
            let all: u32 = w.values().sum();
            *w.entry(serial).or_insert(0) += 1;
            all == 0
        };
        if first && let Some(runs) = &self.lock_at_first {
            runs.lock();
        }
        if let Some((store, id)) = &self.peek {
            let at = store.get(*id).and_then(|s| {
                s.items
                    .iter()
                    .find(|i| i.serial == serial)
                    .map(|i| i.sent_at)
            });
            *self.seen_sent_at.lock().unwrap() = at;
        }
        let does = self
            .does
            .lock()
            .unwrap()
            .get(&serial)
            .copied()
            .unwrap_or(Does::Answers);
        let result = match does {
            Does::Answers => {
                self.apply(&write, serial);
                Ok((write == Write::Mint).then(|| doc(serial)))
            }
            Does::Refused => Err("state transition broadcast error: refused".to_owned()),
            Does::LostLanded => {
                self.apply(&write, serial);
                Err("no complete response within 20s".to_owned())
            }
            Does::LostNotLanded => Err("no complete response within 20s".to_owned()),
            Does::SlowOnce => {
                self.does.lock().unwrap().insert(serial, Does::Answers);
                Err("status: ResourceExhausted, 429 too many".to_owned())
            }
        };
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Box::pin(async move { result })
    }

    fn me(&self) -> Identifier {
        self.me
    }
}

fn store(name: &str) -> Arc<NftSeries> {
    let dir = std::env::temp_dir().join(format!("evolite-series-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Arc::new(NftSeries::new(dir.join("nft-series.json")))
}

fn series(store: &NftSeries, kind: Kind, serials: &[u32]) -> u64 {
    store
        .start(Series {
            id: 0,
            identity: "me".into(),
            contract: "c".into(),
            model: "m".into(),
            kind,
            items: serials
                .iter()
                .map(|&serial| Item {
                    serial,
                    doc: Some(doc(serial).to_string(Encoding::Base58)),
                    state: State::Waiting,
                    why: None,
                    sent_at: None,
                    no_since: None,
                })
                .collect(),
            created_at: 0,
            spent: 0,
            closed: false,
            model_name: None,
            cost_unknown: false,
        })
        .unwrap()
}

fn states(store: &NftSeries, id: u64) -> Vec<State> {
    store
        .get(id)
        .unwrap()
        .items
        .iter()
        .map(|i| i.state)
        .collect()
}

async fn go(fake: Fake, store: &Arc<NftSeries>, id: u64, now: u64) -> (Ended, Arc<Fake>) {
    let fake = Arc::new(fake);
    let ended = run(
        fake.clone(),
        store.clone(),
        Arc::new(SeriesRun::default()),
        id,
        None,
        1,
        move || now,
    )
    .await;
    (ended, fake)
}

/// Each instance written once, never more than four at a time (AC-T-M3).
#[tokio::test(flavor = "multi_thread")]
async fn a_series_writes_each_instance_once_four_at_most() {
    let serials: Vec<u32> = (1..=9).collect();
    let store = store("once");
    let id = series(&store, Kind::Price { price: 5 }, &serials);
    let (ended, fake) = go(Fake::new(&serials), &store, id, 0).await;
    assert_eq!(ended, Ended::Finished);
    assert!(states(&store, id).iter().all(|s| *s == State::Done));
    assert_eq!(fake.total_writes(), 9);
    assert!(fake.most_in_flight.load(Ordering::SeqCst) <= AT_ONCE);
}

/// A refusal in the middle stops nothing else (the architect: «a refusal does not bring down
/// the other nonces»).
#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_in_the_middle_leaves_the_others() {
    let serials: Vec<u32> = (1..=6).collect();
    let store = store("refusal");
    let id = series(&store, Kind::Delete, &serials);
    let (ended, fake) = go(Fake::new(&serials).does(3, Does::Refused), &store, id, 0).await;
    assert_eq!(ended, Ended::Finished);
    let s = states(&store, id);
    assert_eq!(s[2], State::Refused);
    assert_eq!(s.iter().filter(|x| **x == State::Done).count(), 5);
    assert_eq!(fake.total_writes(), 6);
}

/// The answer lost, the write landed: a read shows it — done, written once.
#[tokio::test(flavor = "multi_thread")]
async fn a_lost_answer_that_landed_is_done_once() {
    let store = store("lost");
    let id = series(&store, Kind::Price { price: 5 }, &[1]);
    let (ended, fake) = go(Fake::new(&[1]).does(1, Does::LostLanded), &store, id, 0).await;
    assert_eq!(ended, Ended::Finished);
    assert_eq!(states(&store, id), [State::Done]);
    assert_eq!(fake.writes_of(1), 1);
}

/// BUG-243: not seen, then seen — one write. «Continue» reads it first and sends nothing.
#[tokio::test(flavor = "multi_thread")]
async fn not_seen_then_seen_is_one_write() {
    let store = store("late");
    let id = series(&store, Kind::Price { price: 5 }, &[1]);
    let fake = Fake::new(&[1]).does(1, Does::LostNotLanded);
    let (ended, fake) = go(fake, &store, id, 1_000).await;
    assert_eq!(ended, Ended::Partial("unclear"));
    assert_eq!(states(&store, id), [State::Unclear]);
    // It lands late.
    fake.apply(&Write::Price(5), 1);
    let again = Fake::new(&[]);
    *again.held.lock().unwrap() = fake.held.lock().unwrap().clone();
    let (ended, again) = go(again, &store, id, 1_010).await;
    assert_eq!(ended, Ended::Finished);
    assert_eq!(states(&store, id), [State::Done]);
    assert_eq!(fake.writes_of(1) + again.writes_of(1), 1);
}

/// Not landed: sent again only past the quiet and after two reads [`NO_GAP`] apart.
#[tokio::test(flavor = "multi_thread")]
async fn not_landed_is_sent_again_only_after_the_quiet_and_two_reads() {
    let store = store("quiet");
    let id = series(&store, Kind::Price { price: 5 }, &[1]);
    let (_, _) = go(
        Fake::new(&[1]).does(1, Does::LostNotLanded),
        &store,
        id,
        1_000,
    )
    .await;
    // Within the quiet: left as it is.
    let (ended, f) = go(Fake::new(&[1]), &store, id, 1_000 + 60).await;
    assert_eq!((ended, f.total_writes()), (Ended::Partial("unclear"), 0));
    // Past it, the first «no»: still not sent.
    let (_, f) = go(Fake::new(&[1]), &store, id, 1_000 + QUIET.as_secs()).await;
    assert_eq!(f.total_writes(), 0);
    // The second «no», far enough after: sent again.
    let t = 1_000 + QUIET.as_secs() + NO_GAP.as_secs();
    let (ended, f) = go(Fake::new(&[1]), &store, id, t).await;
    assert_eq!(ended, Ended::Finished);
    assert_eq!(f.writes_of(1), 1);
}

/// SEC-6: a lock during the series: nothing new is sent; the ones sent finish.
#[tokio::test(flavor = "multi_thread")]
async fn a_lock_stops_the_series_before_its_next_write() {
    let serials: Vec<u32> = (1..=12).collect();
    let store = store("lock");
    let id = series(&store, Kind::Price { price: 5 }, &serials);
    let runs = Arc::new(SeriesRun::default());
    let mut fake = Fake::new(&serials);
    fake.lock_at_first = Some(runs.clone());
    let fake = Arc::new(fake);
    let ended = run(fake.clone(), store.clone(), runs, id, None, 1, || 0).await;
    assert_eq!(ended, Ended::Partial("lock"));
    let written = fake.total_writes() as usize;
    assert!((1..=AT_ONCE).contains(&written), "{written}");
    let s = states(&store, id);
    assert_eq!(
        s.iter().filter(|x| **x == State::Waiting).count(),
        12 - written
    );
}

/// R-19: too few credits for the next write — the series stops partial, nothing sent.
#[tokio::test(flavor = "multi_thread")]
async fn too_few_credits_stop_the_series_without_a_write() {
    let store = store("credits");
    let id = series(&store, Kind::Price { price: 5 }, &[1, 2]);
    let fake = Fake::new(&[1, 2]);
    *fake.credits.lock().unwrap() = 0;
    let (ended, fake) = go(fake, &store, id, 0).await;
    assert_eq!(ended, Ended::Partial("credits"));
    assert_eq!(fake.total_writes(), 0);
}

/// R-19: an instance on sale bought meanwhile — «not yours», no write.
#[tokio::test(flavor = "multi_thread")]
async fn an_instance_bought_meanwhile_is_not_written() {
    let store = store("bought");
    let id = series(&store, Kind::Unprice, &[1, 2]);
    let fake = Fake::new(&[1, 2]);
    fake.held.lock().unwrap().get_mut(&1).unwrap().owner = Identifier::from([9; 32]);
    let (ended, fake) = go(fake, &store, id, 0).await;
    assert_eq!(ended, Ended::Finished);
    assert_eq!(states(&store, id), [State::Refused, State::Done]);
    assert_eq!(fake.writes_of(1), 0);
}

/// AC-T-M3: «too often» pauses and sends the same write again; it is not a refusal.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn too_often_pauses_and_goes_on() {
    let store = store("slow");
    let id = series(&store, Kind::Price { price: 5 }, &[1, 2]);
    let (ended, fake) = go(Fake::new(&[1, 2]).does(1, Does::SlowOnce), &store, id, 0).await;
    assert_eq!(ended, Ended::Finished);
    assert_eq!(states(&store, id), [State::Done, State::Done]);
    assert_eq!(fake.writes_of(1), 2);
}

/// r1 Must: evolite closed while a write was out — `Sending` is on disk with its send time,
/// and «Continue» waits the quiet from it (BUG-243) instead of sending it again.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_cut_off_mid_send_waits_its_quiet() {
    let store = store("cut");
    let id = series(&store, Kind::Price { price: 5 }, &[1]);
    // As the series leaves it at the send, then closed.
    store
        .edit(id, 1, |i| {
            i.state = State::Sending;
            i.sent_at = Some(1_000);
        })
        .unwrap();
    // A minute later, not landed: not sent again.
    let (ended, f) = go(Fake::new(&[1]), &store, id, 1_060).await;
    assert_eq!((ended, f.total_writes()), (Ended::Partial("unclear"), 0));
    // Past the quiet and two «no» far enough apart: sent again, once.
    let (_, f) = go(Fake::new(&[1]), &store, id, 1_000 + QUIET.as_secs()).await;
    assert_eq!(f.total_writes(), 0);
    let t = 1_000 + QUIET.as_secs() + NO_GAP.as_secs();
    let (ended, f) = go(Fake::new(&[1]), &store, id, t).await;
    assert_eq!((ended, f.writes_of(1)), (Ended::Finished, 1));
}

/// r1 Must: an older file's `Sending` without a send time — its quiet runs from the first
/// «Continue» that sees it, not from 0.
#[tokio::test(flavor = "multi_thread")]
async fn an_entry_without_its_send_time_waits_from_continue() {
    let store = store("old");
    let id = series(&store, Kind::Price { price: 5 }, &[1]);
    store.set(id, 1, State::Sending, None, None).unwrap();
    // The first «Continue» long after: starts the quiet, sends nothing.
    let (ended, f) = go(Fake::new(&[1]), &store, id, 50_000).await;
    assert_eq!((ended, f.total_writes()), (Ended::Partial("unclear"), 0));
    assert_eq!(store.get(id).unwrap().items[0].sent_at, Some(50_000));
    // 30 s later: still within the quiet, nothing sent.
    let (_, f) = go(Fake::new(&[1]), &store, id, 50_000 + NO_GAP.as_secs()).await;
    assert_eq!(f.total_writes(), 0);
}

/// The series' writes are marked `Sending` with their send time before they go out.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_is_kept_with_its_send_time_before_it_goes() {
    let store = store("stamp");
    let id = series(&store, Kind::Price { price: 5 }, &[1]);
    let mut fake = Fake::new(&[1]);
    fake.peek = Some((store.clone(), id));
    let (_, fake) = go(fake, &store, id, 7_000).await;
    assert_eq!(
        *fake.seen_sent_at.lock().unwrap(),
        Some(Some(7_000)),
        "on disk while out"
    );
}
