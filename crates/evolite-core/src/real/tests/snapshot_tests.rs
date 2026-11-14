use super::*;

/// BUG-64 review r2 should 1: a rescan is finished only once the wallet's filters are back
/// where they stood when it was asked — not while they climb, not for another wallet; a
/// second one asked meanwhile adds what it is for; a wallet switch drops what was owed.
#[test]
fn a_rescan_is_noted_only_when_it_has_run_to_its_end() {
    let (a, b): (WalletId, WalletId) = ([0xa; 32], [0xb; 32]);
    let mut owed = None;
    let s = |t: &str| vec![t.to_owned()];
    OwedRescan::add(&mut owed, a, 1_600_000, s("stale"), Vec::new());
    // Rewound to the birth and climbing.
    assert_eq!(OwedRescan::finished(&mut owed, &a, 1_400_000), None);
    assert_eq!(
        OwedRescan::finished(&mut owed, &b, 1_700_000),
        None,
        "another wallet"
    );
    OwedRescan::add(&mut owed, a, 1_600_002, Vec::new(), s("lost"));
    assert_eq!(OwedRescan::finished(&mut owed, &a, 1_600_001), None);
    let done = OwedRescan::finished(&mut owed, &a, 1_600_002).expect("back at the tip");
    assert_eq!((done.stale, done.lost), (s("stale"), s("lost")));
    assert!(owed.is_none(), "noted once");
    // Asked for a, then for b: a's is dropped (asked again in a later run).
    OwedRescan::add(&mut owed, a, 10, s("x"), Vec::new());
    OwedRescan::add(&mut owed, b, 20, s("y"), Vec::new());
    let done = OwedRescan::finished(&mut owed, &b, 20).expect("b's");
    assert_eq!(done.stale, s("y"));
}

/// BUG-150: a rescan for the contacts' receiving accounts is under way until the filters are back
/// where it was asked from — not asked again meanwhile — and then hands its accounts over to be
/// noted done (`contact-rescans.json`), once.
#[test]
fn a_rescan_for_contact_accounts_is_noted_only_when_it_has_run_to_its_end() {
    let a: WalletId = [0xa; 32];
    let mut owed = None;
    let k = vec!["11-22-0".to_owned()];
    assert!(OwedRescan::contacts_under_way(&owed, &a).is_empty());
    OwedRescan::add_contacts(&mut owed, a, 1_600_000, k.clone(), 0);
    assert_eq!(
        OwedRescan::contacts_under_way(&owed, &a),
        k.as_slice(),
        "under way"
    );
    assert_eq!(
        OwedRescan::finished(&mut owed, &a, 1_400_000),
        None,
        "climbing"
    );
    assert_eq!(OwedRescan::contacts_under_way(&owed, &a), k.as_slice());
    let done = OwedRescan::finished(&mut owed, &a, 1_600_000).expect("back at the tip");
    assert_eq!(done.contacts, k);
    assert!(
        OwedRescan::contacts_under_way(&owed, &a).is_empty(),
        "noted once"
    );
}

/// BUG-13: each DashPay pass makes the snapshot again, once.
#[test]
fn a_new_dashpay_pass_is_noticed_once() {
    let mut last = None;
    assert!(!new_pass(&mut last, None), "no pass yet");
    assert!(new_pass(&mut last, Some(100)));
    assert!(!new_pass(&mut last, Some(100)), "the same pass");
    assert!(new_pass(&mut last, Some(115)));
    assert!(
        !new_pass(&mut last, None),
        "a chain gone keeps what was seen"
    );
    assert_eq!(last, Some(115));
}

#[test]
fn an_answer_without_a_new_snapshot_still_wakes_the_interface() {
    let (out, events) = std::sync::mpsc::channel();
    let woken = std::sync::atomic::AtomicUsize::new(0);
    let wake = || {
        woken.fetch_add(1, Ordering::Relaxed);
    };
    answer(
        &out,
        &wake,
        Some(Event::Done {
            what: DoneWhat::Unlock,
            result: Err(CoreError::WrongPassword),
        }),
    );
    assert!(matches!(
        events.try_recv(),
        Ok(Event::Done {
            what: DoneWhat::Unlock,
            result: Err(CoreError::WrongPassword)
        })
    ));
    assert_eq!(woken.load(Ordering::Relaxed), 1);
    // A command answered from inside `handle`, or by a snapshot later: woken all the same.
    answer(&out, &wake, None);
    assert_eq!(woken.load(Ordering::Relaxed), 2);
}

/// BUG-25: the open wallet goes into platform-wallet as soon as the store is open, so what
/// the store has (identities, balance, history) is shown while the SPV client still starts;
/// the phrase is kept until the client runs, and a payment still waits for the sync.
#[tokio::test(flavor = "multi_thread")]
async fn the_store_is_read_before_the_chain_runs() {
    let mut core = core_with_vault("attach-store", "pw");
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    let id = core.unlocked.as_ref().map(|o| o.id).unwrap();
    core.chain
        .open_store_only(&core.paths.chain)
        .await
        .expect("the store opens");
    assert!(core.chain.store_open() && !core.chain.running());
    assert_eq!(core.attach_wallet().await, Ok(()));
    assert!(core.chain.wallet(&id).await.is_some(), "before the client");
    assert!(core.attach.is_some(), "settled once the client runs");
    assert_eq!(
        loaded(&core.chain, &id).await.err(),
        Some(CoreError::NotSynced)
    );
    // Again while the client starts: nothing new to open, nothing to redraw.
    core.dirty.store(false, Ordering::Relaxed);
    assert_eq!(core.attach_wallet().await, Ok(()));
    assert!(!core.dirty.load(Ordering::Relaxed));
    // The client did not start (r1): the store goes with its wallets, and the next try
    // opens it again with the wallet put in anew.
    let first = core.chain.manager().await.expect("the manager");
    core.chain.let_store_go().await;
    assert!(!core.chain.store_open() && core.chain.wallet(&id).await.is_none());
    assert!(
        first.wallet_ids().await.is_empty(),
        "torn down, not only dropped"
    );
    drop(first);
    core.chain
        .open_store_only(&core.paths.chain)
        .await
        .expect("the store opens again");
    assert_eq!(core.attach_wallet().await, Ok(()));
    assert!(core.chain.wallet(&id).await.is_some());
    core.chain.stop().await;
    assert!(!core.chain.store_open());
}

/// FEAT-0009 review r1 must 1: the snapshot's lines are settled with what is the wallet's
/// own ([`crate::layers::settle_wallet`], tested there with every kind it changes). A
/// withdrawal to one of the wallet's Core addresses is a move between own balances, which
/// only the wallet knows.
#[tokio::test(flavor = "multi_thread")]
async fn the_snapshot_settles_its_lines_with_the_wallets_own_addresses() {
    let mut core = core_with_vault("settle-own", "pw");
    assert_eq!(core.unlock(&pw("pw")).await, Ok(()));
    let open = core.unlocked.as_ref().unwrap();
    let (id, meta) = (open.id, open.meta.clone());
    core.chain
        .open_store_only(&core.paths.chain)
        .await
        .expect("the store opens");
    assert_eq!(core.attach_wallet().await, Ok(()));
    let wallet = core.chain.wallet(&id).await.expect("the wallet is in");
    let mine = wallet
        .state()
        .await
        .core_wallet
        .monitored_addresses()
        .first()
        .map(ToString::to_string)
        .expect("an address of the wallet");
    drop(wallet);
    for to in [mine.clone(), "yQeRmBYHdNaSt7QnKpGWLzRaZDZpSkdeKC".into()] {
        core.journal.add(journal::Entry {
            at: 1_793_000_000,
            what: journal::What::Withdrawal { to },
            spent: 100_000,
            moved: 99_000,
            ..Default::default()
        });
    }
    let shown = core.wallet(id, meta).await;
    let flow_to = |to: &str| {
        shown
            .activity
            .iter()
            .find(|a| matches!(&a.kind, ActivityKind::Withdrawal { to: t, .. } if t == to))
            .map(|a| a.flow)
    };
    assert_eq!(flow_to(&mine), Some(Flow::Between), "to the wallet's own");
    assert_eq!(
        flow_to("yQeRmBYHdNaSt7QnKpGWLzRaZDZpSkdeKC"),
        Some(Flow::Out)
    );
    core.chain.stop().await;
}

/// CS-1a r2 must 1: «Try again» reaches the NFT ask through the core. After a failed
/// ask `RefreshNfts` makes the next snapshot ask at once, not after `REFRESH`, and asks
/// for a new snapshot to do it.
#[tokio::test(flavor = "multi_thread")]
async fn refresh_nfts_asks_at_once_after_a_failed_ask() {
    let mut core = core_at(&fresh_dir("refresh-nfts"), TEST_WALLET);
    let owner = "owner";
    assert!(core.nfts.due(owner), "the first ask");
    core.nfts.found(owner, "@o", Err("timeout".into()));
    assert!(
        !core.nfts.due(owner),
        "on its own, the next ask waits for REFRESH"
    );
    let (event, snapshot) = core.handle(Command::RefreshNfts).await;
    assert!(
        event.is_none() && snapshot,
        "no answer of its own: a new snapshot"
    );
    assert!(core.nfts.due(owner), "asked again at once");
}

/// BUG-103: the pool's lines are in the first snapshot after a start, from the file, before
/// the pool connects (masternode lists, seconds to a minute later).
#[tokio::test]
async fn the_pools_lines_do_not_wait_for_the_pool() {
    let mut core = opened("pool-lines-at-start", "pw");
    let line = Activity {
        id: 7,
        kind: ActivityKind::PoolSent,
        chain: evolite_api::Chain::Platform,
        at: 1_000,
        amount: -214_360,
        proof: Proof::Proved,
        txid: None,
        fee: Some(114_360),
        block: None,
        address: None,
        layers: evolite_api::layer::POOL,
        flow: evolite_api::Flow::Out,
        key: format!("pool:{:016x}", 7),
        detail: None,
        note: None,
        rate: None,
    };
    pool_history::PoolHistory::new(core.paths.pool_history.clone()).merge(vec![line]);
    core.pool_history = pool_history::PoolHistory::new(core.paths.pool_history.clone());
    let shown = core.snapshot().await.wallet.expect("open").activity;
    assert_eq!(shown.iter().map(|a| a.id).collect::<Vec<_>>(), vec![7]);
}

/// FEAT-0005b, FR-3: a lock's line is built with `Credits::Unknown`; the amount a top-up added
/// follows the status [`say_the_lock`] has just set, and is not claimed for a lock Platform did
/// not take. The card opens the identity the line names.
#[test]
fn a_lock_takes_its_amount_from_the_status_it_has_just_got() {
    let unknown = || Activity {
        id: 1,
        kind: ActivityKind::TopUp {
            identity: String::new(),
            creates: false,
            credits: Credits::Unknown,
        },
        chain: evolite_api::Chain::CoreToPlatform,
        at: 1,
        amount: -500,
        proof: Proof::ChainLock,
        txid: Some("aa".into()),
        fee: None,
        block: None,
        address: None,
        layers: 0,
        flow: evolite_api::Flow::Between,
        key: "core:aa".into(),
        note: None,
        rate: None,
        detail: Some(Box::default()),
    };
    let top_ups: HashMap<String, Duffs> = [("aa".to_string(), 42_298)].into();
    let mine = [IdentityChoice {
        id: "HFZ".into(),
        name: Some("werner".into()),
        slot: 0,
    }];
    let card = |act: &Activity| act.detail.as_deref().cloned().unwrap_or_default();

    let mut added = unknown();
    say_the_lock(
        &mut added,
        ("@werner".into(), false, Credits::Added),
        &top_ups,
        &mine,
    );
    assert_eq!(card(&added).credits_added, Some(42_298));
    assert_eq!(card(&added).identity.map(|i| i.id).as_deref(), Some("HFZ"));

    for credits in [Credits::NotAdded, Credits::Unknown] {
        let mut line = unknown();
        say_the_lock(
            &mut line,
            ("@werner".into(), false, credits),
            &top_ups,
            &mine,
        );
        assert_eq!(card(&line).credits_added, None, "{credits:?}");
    }
}

/// BUG-137 AC-T2, AC-T3: the snapshot names the Platform work under way, in the kinds' order,
/// and nothing once it ended; its start and its end each make a snapshot without waiting for
/// the tick.
#[tokio::test]
async fn the_snapshot_names_the_platform_work_under_way() {
    let dir = fresh_dir("platform-work");
    let mut core = core_at(&dir, TEST_WALLET);
    assert!(core.snapshot().await.sync.platform.is_empty());
    core.dirty.store(false, Ordering::Relaxed);
    let tokens = core.work(PlatformWork::Tokens);
    assert!(
        core.dirty.load(Ordering::Relaxed),
        "the start makes a snapshot"
    );
    let identities = core.work(PlatformWork::Identities);
    assert_eq!(
        core.snapshot().await.sync.platform,
        [PlatformWork::Identities, PlatformWork::Tokens]
    );
    core.dirty.store(false, Ordering::Relaxed);
    drop(tokens);
    assert!(core.dirty.load(Ordering::Relaxed), "the end too");
    assert_eq!(
        core.snapshot().await.sync.platform,
        [PlatformWork::Identities]
    );
    drop(identities);
    assert!(core.snapshot().await.sync.platform.is_empty());
}
