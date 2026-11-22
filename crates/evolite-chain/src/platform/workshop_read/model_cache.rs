//! FEAT-0054 r2: the models of each collection, read once a run and kept for the Collection —
//! every identity's list asks for them each REFRESH, and on mainnet a model is its picture
//! (kilobytes each). One read per collection at a time: two identities with a collection in
//! common wait for one read. Read again when [`MAX_AGE`] old, or when an instance names a model
//! not kept — not sooner than [`UNKNOWN_AFTER`] after the last read.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Models are written once by the issuer; new ones come: read again after this.
pub const MAX_AGE: Duration = Duration::from_secs(600);
/// An instance of a model not kept reads the models again, but not more often than this.
pub const UNKNOWN_AFTER: Duration = Duration::from_secs(60);

struct Entry<T> {
    at: Instant,
    value: T,
}

/// What is kept by collection id.
pub struct ModelCache<T> {
    kept: Mutex<HashMap<String, Entry<T>>>,
    /// One read per collection at a time.
    reading: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl<T> Default for ModelCache<T> {
    fn default() -> Self {
        Self {
            kept: Mutex::new(HashMap::new()),
            reading: Mutex::new(HashMap::new()),
        }
    }
}

impl<T: Clone> ModelCache<T> {
    /// The models of `collection`: kept ones while they are younger than [`MAX_AGE`] and
    /// `enough` says they have what is needed (else read again once [`UNKNOWN_AFTER`] has
    /// passed), otherwise `read`, kept.
    pub async fn get<F, Fut>(
        &self,
        collection: &str,
        enough: impl Fn(&T) -> bool,
        read: F,
    ) -> Result<T, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, String>>,
    {
        let lock = self
            .reading
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(collection.to_owned())
            .or_default()
            .clone();
        let _one = lock.lock().await;
        if let Some(kept) = self.kept(collection, &enough) {
            return Ok(kept);
        }
        let value = read().await?;
        self.kept.lock().unwrap_or_else(|e| e.into_inner()).insert(
            collection.to_owned(),
            Entry {
                at: Instant::now(),
                value: value.clone(),
            },
        );
        Ok(value)
    }

    fn kept(&self, collection: &str, enough: &impl Fn(&T) -> bool) -> Option<T> {
        let kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        let entry = kept.get(collection)?;
        let age = entry.at.elapsed();
        let good = age < MAX_AGE && (enough(&entry.value) || age < UNKNOWN_AFTER);
        good.then(|| entry.value.clone())
    }

    /// Forgets `collection`'s, so the next [`Self::get`] reads (tests: an age passed).
    #[cfg(test)]
    fn age(&self, collection: &str, by: Duration) {
        if let Some(e) = self.kept.lock().unwrap().get_mut(collection) {
            e.at = Instant::now().checked_sub(by).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn read(
        count: &AtomicUsize,
        models: Vec<&'static str>,
    ) -> Result<Vec<&'static str>, String> {
        count.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(models)
    }

    /// FEAT-0054 r2: two identities with a collection in common, asked at once: its models are
    /// read once; the next REFRESH reads none; an instance of a new model reads them again,
    /// but not before UNKNOWN_AFTER; MAX_AGE reads them again anyway.
    #[tokio::test]
    async fn the_models_of_a_collection_are_read_once_for_every_identity() {
        let cache = ModelCache::default();
        let count = AtomicUsize::new(0);
        let has = |needed: &'static str| move |v: &Vec<&str>| v.contains(&needed);
        let (a, b) = tokio::join!(
            cache.get("coll", has("m1"), || read(&count, vec!["m1"])),
            cache.get("coll", has("m1"), || read(&count, vec!["m1"])),
        );
        assert_eq!((a.unwrap(), b.unwrap()), (vec!["m1"], vec!["m1"]));
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "one read for both identities"
        );
        // The next REFRESH: nothing read.
        cache
            .get("coll", has("m1"), || read(&count, vec!["m1"]))
            .await
            .unwrap();
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "no read on the next refresh"
        );
        // An instance of a model not kept, just after the read: not read yet…
        let got = cache
            .get("coll", has("m2"), || read(&count, vec!["m1", "m2"]))
            .await
            .unwrap();
        assert_eq!((got, count.load(Ordering::SeqCst)), (vec!["m1"], 1));
        // …once UNKNOWN_AFTER has passed, read again.
        cache.age("coll", UNKNOWN_AFTER);
        let got = cache
            .get("coll", has("m2"), || read(&count, vec!["m1", "m2"]))
            .await
            .unwrap();
        assert_eq!((got, count.load(Ordering::SeqCst)), (vec!["m1", "m2"], 2));
        // MAX_AGE: read again even with everything kept.
        cache.age("coll", MAX_AGE);
        cache
            .get("coll", has("m1"), || read(&count, vec!["m1", "m2"]))
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        // Another collection is its own.
        cache
            .get("other", has("x"), || read(&count, vec!["x"]))
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }
}
