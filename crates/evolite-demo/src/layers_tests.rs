//! A layers test that needs the demo's example wallet, so it stays beside the demo core.

use evolite_api::layer::{CORE, IDENTITY, PLATFORM_ADDRESS, POOL};
use evolite_api::{Activity, ActivityKind, Chain, Credits, Flow, IdentityChoice, Proof};
use evolite_common::layers::*;

fn line(kind: ActivityKind, chain: Chain) -> Activity {
    let (layers, flow) = of(&kind, chain);
    Activity {
        id: 1,
        kind,
        chain,
        at: 0,
        amount: 0,
        proof: Proof::Proved,
        txid: None,
        fee: None,
        block: None,
        address: None,
        layers,
        flow,
        key: String::new(),
        detail: None,
        note: None,
        rate: None,
    }
}

fn lock(identity: &str, creates: bool) -> ActivityKind {
    ActivityKind::TopUp {
        identity: identity.into(),
        creates,
        credits: Credits::Added,
    }
}

fn shield(from: &str) -> ActivityKind {
    ActivityKind::Shielded {
        from: Some(from.into()),
    }
}

/// Review r1 must 1: the snapshot's pass over its history and its pool. Lines are made
/// before the wallet knows what they are for (`Real::wallet`): a lock's identity comes from
/// `lock_line`, a lock made for the pool turns into a shield, and the pool's own lines are
/// settled too.
#[test]
fn the_snapshot_settles_its_history_and_its_pool_with_what_is_its_own() {
    use ActivityKind::*;
    const ID: &str = "4EfA9Jrvv3nnCFdSf7fad59851iiTRZ6Wcu6YVJ4iSeF";
    let mut wallet = crate::example_wallet(0);
    wallet.identities = vec![IdentityChoice {
        id: ID.into(),
        name: Some("kestrel7".into()),
        slot: 0,
    }];
    wallet.platform_address = "tdash1mine".into();
    // Made as a lock to the Platform address; its identity is filled in afterwards.
    let mut top_up = line(lock("", false), Chain::CoreToPlatform);
    top_up.kind = lock("@kestrel7", false);
    // Made as a lock; then known to be the pool's.
    let mut from_core = line(lock("", false), Chain::CoreToPlatform);
    from_core.kind = shield(FROM_CORE);
    let withdrawal = |to: &str| {
        line(
            Withdrawal {
                to: to.into(),
                status: None,
            },
            Chain::Platform,
        )
    };
    let credits = |to: &str| line(CreditTransfer { to: to.into() }, Chain::Platform);
    wallet.activity = vec![
        top_up,
        from_core,
        withdrawal("yMine"),
        withdrawal("yOther"),
        credits("@kestrel7"),
        credits(ID),
        credits("tdash1mine"),
        credits("@bob"),
    ];
    // A shield the pool's scan found, its source learnt later.
    let mut scanned = line(Shielded { from: None }, Chain::Platform);
    scanned.kind = shield(FROM_CREDITS);
    let mut to_mine = credits("@kestrel7");
    to_mine.flow = Flow::Out;
    wallet.pool.operations = vec![scanned, to_mine];

    settle_wallet(&mut wallet, &["yMine".to_string()]);

    let got: Vec<_> = wallet.activity.iter().map(|a| (a.layers, a.flow)).collect();
    assert_eq!(
        got,
        [
            (CORE | IDENTITY, Flow::Between),
            (CORE | POOL, Flow::Between),
            (IDENTITY, Flow::Between),
            (IDENTITY, Flow::Out),
            (IDENTITY, Flow::Between),
            (IDENTITY, Flow::Between),
            (IDENTITY | PLATFORM_ADDRESS, Flow::Between),
            (IDENTITY, Flow::Out),
        ]
    );
    let pool: Vec<_> = wallet
        .pool
        .operations
        .iter()
        .map(|a| (a.layers, a.flow))
        .collect();
    assert_eq!(
        pool,
        [(IDENTITY | POOL, Flow::Between), (IDENTITY, Flow::Between)]
    );
}
