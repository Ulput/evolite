//! SEC-6 (the phrase review, П3): the background tasks that hold a copy of the wallet's keys or
//! seed end with the lock. Before, a lock let them run to their end — seconds, or minutes on a
//! slow network — with the keys in memory after «Lock». A paid operation is not cut here: cut
//! after its broadcast it would leave no History line (the BUG-205 class); its copy goes with it
//! when it ends.

use std::sync::{Arc, Mutex};
use tokio::task::AbortHandle;

/// The running background tasks with keys, by what they are.
#[derive(Clone, Default)]
pub(in crate::real) struct KeyedTasks(Arc<Mutex<Vec<(&'static str, AbortHandle)>>>);

impl KeyedTasks {
    /// A task that holds the keys or the seed, `what` it is; the ended ones are let go.
    pub(in crate::real) fn add(&self, what: &'static str, task: AbortHandle) {
        let mut tasks = self.0.lock().unwrap_or_else(|e| e.into_inner());
        tasks.retain(|(_, t)| !t.is_finished());
        tasks.push((what, task));
    }

    /// The lock: every one still running is cut, and with it its copy of the keys. What was cut,
    /// for what its owner has to set back (a mark of «connecting», a «busy»).
    pub(in crate::real) fn cut(&self) -> Vec<&'static str> {
        let tasks = std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()));
        tasks
            .into_iter()
            .filter(|(_, t)| !t.is_finished())
            .map(|(what, t)| {
                t.abort();
                what
            })
            .collect()
    }
}

/// What the tasks are.
pub(in crate::real) const IDENTITIES: &str = "identities";
pub(in crate::real) const POOL: &str = "pool";
pub(in crate::real) const CONTACT_COINS: &str = "contact_coins";
pub(in crate::real) const CONTACT_ACCOUNTS: &str = "contact_accounts";

#[cfg(test)]
mod tests {
    use super::*;

    /// SEC-6: a cut task lets go of what it holds (here a stand-in for the keys) at once, and is
    /// named; one that ended is not.
    #[tokio::test]
    async fn the_lock_cuts_a_task_and_its_copy_of_the_keys() {
        let keys = Arc::new([7u8; 64]);
        let tasks = KeyedTasks::default();
        let held = keys.clone();
        let task = tokio::spawn(async move {
            let _held = held;
            std::future::pending::<()>().await;
        });
        tasks.add(POOL, task.abort_handle());
        tasks.add(IDENTITIES, tokio::spawn(async {}).abort_handle());
        tokio::task::yield_now().await;
        assert_eq!(Arc::strong_count(&keys), 2, "the task holds its copy");
        assert_eq!(tasks.cut(), vec![POOL]);
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(Arc::strong_count(&keys), 1, "the copy went with it");
        assert!(tasks.cut().is_empty(), "nothing left after a cut");
    }
}
