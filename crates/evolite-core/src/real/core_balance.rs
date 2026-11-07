//! The last checked Core balance and when it was checked, kept in `core-balance.json` beside the
//! store (BUG-77, R-34; the designer's improvement 1 «Ещё не знаем», approved 2026-11-06).
//!
//! While the filters are not at the tip, the wallet does not know its coins: after a start the
//! store's coins are there, and a pass over the filters from lower down (DashPay's backfill
//! rewinds at each start, BUG-64's rescans) brings old payments back before the blocks that
//! spent them — werner555 showed 6,80 DASH for a while instead of 3,87. Such an amount is never
//! shown: the snapshot gives the last checked one with its time, or none at all before the
//! first check (a new or restored wallet).

use evolite_api::{CoreAsOf, Duffs, Timestamp};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// A checked balance written again, though unchanged, after this long: «as of» then says when
/// it was last checked, not when it last changed — at most this much earlier.
const REFRESH: Timestamp = 5 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Checked {
    duffs: Duffs,
    at: Timestamp,
}

pub struct CoreBalance {
    path: PathBuf,
    /// What the file holds: read at the first use; `None` inside: no balance checked yet.
    saved: Mutex<Option<Option<Checked>>>,
}

impl CoreBalance {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            saved: Mutex::new(None),
        }
    }

    fn load(&self) -> Option<Checked> {
        super::json_file::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    }

    /// The Core balance the snapshot shows and how sure it is. `counted`: the wallet's balance
    /// now ([`super::convert::core_balance`]); `known`: whether its coins are known
    /// ([`super::convert::coins_known`]). Known: that balance, checked now, written down when it
    /// changed (or [`REFRESH`] after the last write). Not known: the last checked one and its
    /// time, or 0 and [`CoreAsOf::NotYet`] — never `counted`.
    pub fn shown(&self, counted: Duffs, known: bool, now: Timestamp) -> (Duffs, Option<CoreAsOf>) {
        let mut guard = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        let saved = guard.get_or_insert_with(|| self.load());
        if known {
            let stale = saved.is_none_or(|s| s.duffs != counted || now - s.at >= REFRESH);
            if stale {
                let checked = Checked {
                    duffs: counted,
                    at: now,
                };
                // A failed write is in the journal (json_file); the amount is shown all the same.
                let _ = super::json_file::save(&self.path, &checked);
                *saved = Some(checked);
            }
            return (counted, None);
        }
        match saved {
            Some(s) => (s.duffs, Some(CoreAsOf::Checked(s.at))),
            None => (0, Some(CoreAsOf::NotYet)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evolite-core-balance-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Only a checked balance is written; after a restart, before the filters are done again,
    /// it is what is shown, with its time. Before any check: nothing known, never 0 as a sum.
    #[test]
    fn only_a_checked_balance_is_kept_and_shown_after_a_restart() {
        let dir = folder("restart");
        let path = dir.join("core-balance.json");
        let first = CoreBalance::new(path.clone());
        // A new or restored wallet: the store's coins while syncing are no balance.
        assert_eq!(
            first.shown(391_724_464, false, 1_000),
            (0, Some(CoreAsOf::NotYet))
        );
        assert!(!path.exists(), "nothing checked, nothing written");
        assert_eq!(first.shown(386_574_464, true, 2_000), (386_574_464, None));
        assert!(path.exists());

        // The next start: the store's sum during the sync is not shown; the checked one is.
        let again = CoreBalance::new(path.clone());
        assert_eq!(
            again.shown(391_724_464, false, 9_000),
            (386_574_464, Some(CoreAsOf::Checked(2_000)))
        );
        // Checked again, the same: «as of» moves on only after REFRESH (no write each frame).
        assert_eq!(again.shown(386_574_464, true, 2_100), (386_574_464, None));
        assert_eq!(
            CoreBalance::new(path.clone()).shown(0, false, 9_000),
            (386_574_464, Some(CoreAsOf::Checked(2_000)))
        );
        assert_eq!(
            again.shown(386_574_464, true, 2_000 + REFRESH),
            (386_574_464, None)
        );
        assert_eq!(
            CoreBalance::new(path.clone()).shown(0, false, 9_000),
            (386_574_464, Some(CoreAsOf::Checked(2_000 + REFRESH)))
        );
        // A change is written at once.
        assert_eq!(again.shown(380_000_000, true, 2_400), (380_000_000, None));
        assert_eq!(
            CoreBalance::new(path).shown(0, false, 9_000),
            (380_000_000, Some(CoreAsOf::Checked(2_400)))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A pass over the filters from lower down (the DashPay backfill rewind at each start,
    /// BUG-64's rescans) brings back coins spent long ago until it reaches the blocks spending
    /// them: werner555 showed 6,80 DASH in the middle of it. The checked balance stays on screen
    /// through the whole pass, and the counted one comes back once the filters are at the tip.
    #[test]
    fn a_rewound_pass_that_brings_spent_coins_back_shows_the_checked_balance() {
        let dir = folder("rewind");
        let balance = CoreBalance::new(dir.join("core-balance.json"));
        assert_eq!(balance.shown(386_574_464, true, 100), (386_574_464, None));
        // The rewind: filters no longer done, spent coins come back as unspent.
        for (n, counted) in [386_574_464, 480_328_822, 680_301_341, 679_990_100]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                balance.shown(counted, false, 200 + n as Timestamp),
                (386_574_464, Some(CoreAsOf::Checked(100))),
                "in the middle of the pass: {counted}"
            );
        }
        // At the tip again.
        assert_eq!(balance.shown(386_574_464, true, 300), (386_574_464, None));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// BUG-77 r1 must: after a start the headers reach the tip at N while dash-spv already says
    /// the filters are done — at N − k, the last run's height. The balance then is the last
    /// run's: shown as of when it was checked, and the file keeps that time (it is not written
    /// again as if checked now, which made the later «as of» a false one).
    #[test]
    fn filters_done_below_the_tip_neither_show_nor_write_a_checked_balance() {
        use super::super::convert::coins_known;
        use super::super::sync::{Stage, SyncStatus};
        let dir = folder("below-tip");
        let path = dir.join("core-balance.json");
        CoreBalance::new(path.clone()).shown(386_574_464, true, 1_000);
        let before = std::fs::read(&path).unwrap();
        let stage = |height| Stage {
            height,
            target: height,
            done: true,
        };
        let mut s = SyncStatus {
            headers: Some(stage(1_586_823)),
            masternodes: Some(Stage {
                done: false,
                ..stage(1_586_823)
            }),
            filters: Some(stage(1_586_762)),
            ..Default::default()
        };
        s.latch_filters_at_tip();
        let restarted = CoreBalance::new(path.clone());
        let later = 1_000 + 2 * REFRESH;
        assert_eq!(
            restarted.shown(386_574_464, coins_known(&s, true), later),
            (386_574_464, Some(CoreAsOf::Checked(1_000)))
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "the time it was checked"
        );
        // The filters at the headers' tip: checked now.
        s.filters = Some(stage(1_586_823));
        s.latch_filters_at_tip();
        assert_eq!(
            restarted.shown(386_574_464, coins_known(&s, true), later),
            (386_574_464, None)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A damaged file is as none: «not known yet», not a made-up amount.
    #[test]
    fn a_damaged_file_is_no_balance() {
        let dir = folder("damaged");
        let path = dir.join("core-balance.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"{\"duffs\": ").unwrap();
        assert_eq!(
            CoreBalance::new(path).shown(5, false, 10),
            (0, Some(CoreAsOf::NotYet))
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
