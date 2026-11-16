//! TECH-busy-guard: «one change of the identity at a time» as a guard, not a hand-kept flag.
//!
//! [`BusyFlag::take`] raises the flag and hands back a [`BusyGuard`]; the flag goes down when
//! the guard is dropped — the task's end, its panic, or the task dropped from outside. Before
//! this the flag was lowered by hand in a dozen places, and a task that panicked left the
//! identity busy until a restart, its screen never answered.
//!
//! The flag is one `Arc` for the core and its parked chain (BUG-210): «Add a wallet» → «Back»
//! keeps the same flag while a task started before it still runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether a change of the identity is under way; shared, cheap to clone.
#[derive(Clone, Default)]
pub(in crate::real) struct BusyFlag(Arc<AtomicBool>);

impl BusyFlag {
    /// Takes the identity, or `None` while it is taken.
    pub(in crate::real) fn take(&self) -> Option<BusyGuard> {
        (!self.0.swap(true, Ordering::SeqCst)).then(|| BusyGuard(self.0.clone()))
    }

    pub(in crate::real) fn is_busy(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The identity taken; free again when this is dropped, however the task ends.
pub(in crate::real) struct BusyGuard(Arc<AtomicBool>);

impl BusyGuard {
    /// The flag this guard holds, to watch it go down (the creation's steps).
    pub(in crate::real) fn flag(&self) -> BusyFlag {
        BusyFlag(self.0.clone())
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One at a time; the second take waits for the first guard to go.
    #[test]
    fn one_change_at_a_time() {
        let flag = BusyFlag::default();
        let first = flag.take().expect("free");
        assert!(flag.is_busy());
        assert!(flag.take().is_none(), "taken");
        drop(first);
        assert!(!flag.is_busy());
        assert!(flag.take().is_some());
    }

    /// AC-T1: a task that panics, and one dropped from outside, both let the identity go.
    #[tokio::test]
    async fn a_task_that_panics_or_is_dropped_lets_the_identity_go() {
        let flag = BusyFlag::default();
        let guard = flag.take().unwrap();
        let panicked = tokio::spawn(async move {
            let _guard = guard;
            panic!("the task fails");
        })
        .await;
        assert!(panicked.is_err_and(|e| e.is_panic()));
        assert!(!flag.is_busy(), "free after a panic");

        let guard = flag.take().unwrap();
        let running = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;
        assert!(flag.is_busy());
        running.abort();
        assert!(running.await.is_err_and(|e| e.is_cancelled()));
        assert!(!flag.is_busy(), "free after the task is dropped");
    }

    /// AC-T3: the identity's flag is raised and lowered only through [`BusyFlag::take`] and
    /// [`BusyGuard`]: no store or swap on it by hand anywhere in the core's code or tests.
    #[test]
    fn the_identity_flag_is_only_lowered_by_its_guard() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut dirs = vec![root];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    // Written in two parts, so this test does not find itself.
                    let by_hand = [".store(", ".swap("].map(|m| ["identity_busy", m].concat());
                    if by_hand.iter().any(|n| text.contains(n.as_str())) {
                        found.push(path.display().to_string());
                    }
                }
            }
        }
        assert!(found.is_empty(), "lowered by hand: {found:?}");
    }
}
