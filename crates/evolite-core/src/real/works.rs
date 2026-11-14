//! What the wallet catches up on Platform now (BUG-137): the core's background reads hold a
//! [`Working`] guard while they run, and the snapshot names them in `SyncInfo::platform`. The
//! guard takes its kind away on any way out (an answer, a `?`, a panic), so no flag is left
//! raised; its start and its end each make the snapshot again (`refresh`).

use std::sync::{Arc, Mutex};

pub(crate) use evolite_api::PlatformWork;

/// How many works of each kind run now.
pub(crate) struct Works {
    running: Mutex<[u32; PlatformWork::ALL.len()]>,
    refresh: Arc<dyn Fn() + Send + Sync>,
}

impl Works {
    /// `refresh`: makes the snapshot again soon (the core's `dirty` and `wake`).
    pub(crate) fn new(refresh: Arc<dyn Fn() + Send + Sync>) -> Arc<Works> {
        Arc::new(Works {
            running: Mutex::default(),
            refresh,
        })
    }

    /// A work of `kind` starts: named until the guard is dropped.
    pub(crate) fn start(self: &Arc<Self>, kind: PlatformWork) -> Working {
        self.count(kind, |n| n + 1);
        Working {
            works: self.clone(),
            kind,
        }
    }

    /// What runs now, in the kinds' order, each once.
    pub(crate) fn now(&self) -> Vec<PlatformWork> {
        let running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        PlatformWork::ALL
            .into_iter()
            .zip(running.iter())
            .filter(|(_, n)| **n > 0)
            .map(|(kind, _)| kind)
            .collect()
    }

    fn count(&self, kind: PlatformWork, change: impl FnOnce(u32) -> u32) {
        {
            let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
            let n = &mut running[kind as usize];
            *n = change(*n);
        }
        (self.refresh)();
    }
}

/// A work under way; dropped, it ends.
#[must_use = "the work ends when the guard is dropped"]
pub(crate) struct Working {
    works: Arc<Works>,
    kind: PlatformWork,
}

impl Drop for Working {
    fn drop(&mut self) {
        self.works.count(self.kind, |n| n.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn counting() -> (Arc<Works>, Arc<AtomicU32>) {
        let refreshed = Arc::new(AtomicU32::new(0));
        let r = refreshed.clone();
        let works = Works::new(Arc::new(move || {
            r.fetch_add(1, Ordering::Relaxed);
        }));
        (works, refreshed)
    }

    /// AC-T1, AC-T3: a guard names its kind while it lives and takes it away when it ends; its
    /// start and its end each make the snapshot again.
    #[test]
    fn a_guard_names_its_work_while_it_lives() {
        let (works, refreshed) = counting();
        assert!(works.now().is_empty());
        let guard = works.start(PlatformWork::Tokens);
        assert_eq!(works.now(), [PlatformWork::Tokens]);
        assert_eq!(
            refreshed.load(Ordering::Relaxed),
            1,
            "the start wakes the snapshot"
        );
        drop(guard);
        assert!(works.now().is_empty());
        assert_eq!(refreshed.load(Ordering::Relaxed), 2, "the end too");
    }

    /// AC-T1: a `?` and a panic take the work away as well.
    #[test]
    fn an_error_or_a_panic_takes_the_work_away() {
        let (works, _) = counting();
        let failing = |works: &Arc<Works>| -> Result<(), String> {
            let _working = works.start(PlatformWork::Identities);
            Err("no nodes".to_owned())?;
            Ok(())
        };
        assert!(failing(&works).is_err());
        assert!(works.now().is_empty(), "after a `?`");
        let w = works.clone();
        let panicked = std::thread::spawn(move || {
            let _working = w.start(PlatformWork::Pool);
            panic!("the read panicked");
        })
        .join();
        assert!(panicked.is_err());
        assert!(works.now().is_empty(), "after a panic");
    }

    /// AC-T1, AC-T2: two works of one kind end with the second; the list is in the kinds' order,
    /// each once.
    #[test]
    fn two_of_a_kind_end_with_the_second_and_the_list_is_in_order() {
        let (works, _) = counting();
        let tokens = works.start(PlatformWork::Tokens);
        let first = works.start(PlatformWork::Credits);
        let second = works.start(PlatformWork::Credits);
        let identities = works.start(PlatformWork::Identities);
        assert_eq!(
            works.now(),
            [
                PlatformWork::Identities,
                PlatformWork::Credits,
                PlatformWork::Tokens
            ]
        );
        drop(first);
        assert!(
            works.now().contains(&PlatformWork::Credits),
            "the second still reads"
        );
        drop(second);
        assert!(!works.now().contains(&PlatformWork::Credits));
        drop((tokens, identities));
        assert!(works.now().is_empty());
    }
}
