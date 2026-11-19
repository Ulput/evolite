use super::*;
use crate::platform::tokens::info::test_contract;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Platform as a test has it: counted requests, a pause, a failure on demand, a gate to hold a
/// request open.
#[derive(Default)]
struct Fake {
    versions: std::sync::Mutex<HashMap<Identifier, u32>>,
    bodies: HashMap<Identifier, Arc<DataContract>>,
    version_calls: AtomicUsize,
    version_ids: std::sync::Mutex<Vec<usize>>,
    fetch_calls: AtomicUsize,
    pause: Duration,
    versions_fail: bool,
    gate: Option<Arc<tokio::sync::Notify>>,
}

impl Source for Fake {
    async fn latest_versions(
        &self,
        ids: &[Identifier],
    ) -> Result<HashMap<Identifier, u32>, String> {
        self.version_calls.fetch_add(1, Ordering::SeqCst);
        self.version_ids.lock().unwrap().push(ids.len());
        if let Some(gate) = &self.gate {
            gate.notified().await;
        }
        tokio::time::sleep(self.pause).await;
        if self.versions_fail {
            return Err("no answer".into());
        }
        let known = self.versions.lock().unwrap().clone();
        Ok(ids
            .iter()
            .filter_map(|id| Some((*id, *known.get(id)?)))
            .collect())
    }

    async fn fetch(&self, ids: &[Identifier]) -> Result<Vec<Arc<DataContract>>, String> {
        self.fetch_calls.fetch_add(1, Ordering::SeqCst);
        Ok(ids
            .iter()
            .filter_map(|id| self.bodies.get(id).cloned())
            .collect())
    }
}

fn folder(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "evolite-fresh-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("contracts.db")
}

fn contract(n: u8) -> Arc<DataContract> {
    Arc::new(test_contract([n; 32], [9; 32], &[("coffee", 2)]))
}

fn id(n: u8) -> Identifier {
    Identifier::from([n; 32])
}

/// A cache with contracts `ns` kept, last checked at 0 (stale for any window).
fn kept(tag: &str, ns: &[u8]) -> ContractCache {
    let cache = ContractCache::new(folder(tag));
    for n in ns {
        cache.put(contract(*n), 0).unwrap();
    }
    cache
}

fn fake(ns: &[u8]) -> Fake {
    let fake = Fake::default();
    for n in ns {
        let c = contract(*n);
        fake.versions.lock().unwrap().insert(c.id(), c.version());
    }
    Fake {
        bodies: ns.iter().map(|n| (id(*n), contract(*n))).collect(),
        ..fake
    }
}

/// Condition 7: two calls for one contract at once ask Platform once — the second takes what
/// the first checked.
#[tokio::test(flavor = "multi_thread")]
async fn one_flight_per_contract() {
    let cache = kept("flight", &[1]);
    let source = Fake {
        pause: Duration::from_millis(150),
        ..fake(&[1])
    };
    let ids = [id(1)];
    let (a, b) = tokio::join!(
        fresh_many(&cache, &source, &ids, 60, false, 1_000),
        fresh_many(&cache, &source, &ids, 60, false, 1_000),
    );
    assert!(a[0].is_ok() && b[0].is_ok());
    assert_eq!(source.version_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.fetch_calls.load(Ordering::SeqCst), 0);
}

/// Condition 7: the stale ones' versions in one request; only a grown one is read, in one more.
#[tokio::test(flavor = "multi_thread")]
async fn many_contracts_in_one_request() {
    let cache = kept("many", &[2, 3, 4]);
    let source = fake(&[2, 3, 4]);
    source.versions.lock().unwrap().insert(id(4), 99);
    let answers = fresh_many(&cache, &source, &[id(2), id(3), id(4)], 60, false, 1_000).await;
    assert!(answers.iter().all(|a| a.is_ok()));
    assert_eq!(source.version_calls.load(Ordering::SeqCst), 1);
    assert_eq!(*source.version_ids.lock().unwrap(), [3]);
    assert_eq!(
        source.fetch_calls.load(Ordering::SeqCst),
        1,
        "only the grown one"
    );
}

/// Condition 8: a check that fails gives a read the contract kept, not checked; a write an error.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_check_is_a_read_not_checked_and_no_write() {
    let cache = kept("failed", &[5]);
    // Proved in this run (put): a write would only need the version.
    let source = Fake {
        versions_fail: true,
        ..fake(&[5])
    };
    let read = fresh_many(&cache, &source, &[id(5)], 60, false, 1_000).await;
    let read = read[0].as_ref().expect("the one kept");
    assert!(!read.version_checked);
    assert!(read.proved, "two flags, not one");
    let write = fresh_many(&cache, &source, &[id(5)], 0, true, 1_000).await;
    assert!(write[0].is_err(), "no write on a version not checked");
}

/// Condition 6: a check waiting on the network holds no lock the provider needs: `try_get` and
/// `get` answer at once.
#[tokio::test(flavor = "multi_thread")]
async fn a_check_waiting_on_the_network_does_not_hold_the_provider() {
    let cache = Arc::new(kept("waiting", &[6]));
    let gate = Arc::new(tokio::sync::Notify::new());
    let source = Arc::new(Fake {
        gate: Some(gate.clone()),
        ..fake(&[6])
    });
    let task = {
        let (cache, source) = (cache.clone(), source.clone());
        tokio::spawn(async move { fresh_many(&cache, &*source, &[id(6)], 60, false, 1_000).await })
    };
    while source.version_calls.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let started = std::time::Instant::now();
    assert!(cache.try_get(&id(6)).is_some());
    assert!(cache.get(&id(6)).is_some());
    assert!(started.elapsed() < Duration::from_millis(200));
    gate.notify_one();
    assert!(task.await.unwrap()[0].is_ok());
}

/// Condition 1: a body read from the disk (the next start) is fetched for a write, whatever its
/// version says.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_reads_the_body_again_after_a_restart() {
    let path = folder("restart");
    ContractCache::new(path.clone())
        .put(contract(7), 1_000)
        .unwrap();
    let next = ContractCache::new(path);
    let source = fake(&[7]);
    let read = fresh_many(&next, &source, &[id(7)], 60, false, 1_010).await;
    assert!(
        read[0].as_ref().is_ok_and(|c| !c.proved),
        "a read takes the disk"
    );
    assert_eq!(source.fetch_calls.load(Ordering::SeqCst), 0);
    let write = fresh_many(&next, &source, &[id(7)], 0, true, 1_010).await;
    assert!(write[0].as_ref().is_ok_and(|c| c.proved));
    assert_eq!(
        source.fetch_calls.load(Ordering::SeqCst),
        1,
        "the body, proved"
    );
}

/// Conditions 1 and 9: a write takes a token's contract proved, and refuses a token row that
/// points at another position than the contract's own.
#[tokio::test(flavor = "multi_thread")]
async fn a_token_for_a_write_is_checked_against_its_contract() {
    let path = folder("token");
    let cache = ContractCache::new(path.clone());
    let c = contract(8);
    cache.put(c.clone(), 1_000).unwrap();
    let token = c.token_id(0).unwrap();
    let source = fake(&[8]);
    let (got, position) = token_for_write(&cache, &source, &token, 1_000)
        .await
        .unwrap();
    assert_eq!((got.contract.id(), position), (id(8), 0));
    // The row moved to another position on the disk.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE token SET position = 3 WHERE token_id = ?1",
            rusqlite::params![token.as_slice()],
        )
        .unwrap();
    assert!(
        token_for_write(&cache, &source, &token, 1_000)
            .await
            .is_err()
    );
}
