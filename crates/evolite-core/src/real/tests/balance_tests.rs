//! The saved Core balance against the sync's own word on the coins (`core_balance` holds the
//! rest of its tests; this one needs the sync's state).

use std::path::PathBuf;

use evolite_api::CoreAsOf;
use evolite_local::core_balance::{CoreBalance, REFRESH};

fn folder(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("evolite-core-balance-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
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
