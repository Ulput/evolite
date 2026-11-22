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

/// BUG-221 (live, qa-f27): a contact's account added while the wallet's filters are below where
/// its rescan starts — the add rewinds them to the birth by itself — is owed until they reach
/// the headers' tip, not the wallet's height at the ask: that one was «reached» on the next
/// snapshot, with the scan still ahead and the balance said final.
#[test]
fn a_contact_rescan_asked_with_the_filters_below_its_start_waits_for_the_tip() {
    let a: WalletId = [0xa; 32];
    let tip = 1_593_887;
    let status = sync::SyncStatus {
        phase: sync::Phase::Synced,
        headers: Some(sync::Stage {
            height: tip,
            target: tip,
            done: true,
        }),
        ..Default::default()
    };
    // The account's add left the wallet at the birth (1_585_000), below the start (1_585_217).
    let rewound = 1_584_999;
    let up_to = rescan_end(rewound, &status);
    assert_eq!(up_to, tip, "the tip, not the wallet's height");
    let mut owed = None;
    let k = vec!["11-22-0".to_owned()];
    OwedRescan::add_contacts(&mut owed, a, up_to, k.clone(), 0);
    assert_eq!(
        OwedRescan::finished(&mut owed, &a, rewound),
        None,
        "not begun"
    );
    assert_eq!(
        OwedRescan::finished(&mut owed, &a, 1_590_762),
        None,
        "past the start, short of the tip"
    );
    assert_eq!(OwedRescan::contacts_under_way(&owed, &a), k.as_slice());
    let done = OwedRescan::finished(&mut owed, &a, tip).expect("at the tip");
    assert_eq!(done.contacts, k);
    // A wallet already higher than the headers (they lag a block): its own height.
    assert_eq!(rescan_end(tip + 1, &status), tip + 1);
    // No headers yet: the wallet's height (no rescan is asked before the sync is done).
    assert_eq!(rescan_end(rewound, &sync::SyncStatus::default()), rewound);
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
            ask: None,
        }),
    );
    assert!(matches!(
        events.try_recv(),
        Ok(Event::Done {
            what: DoneWhat::Unlock,
            result: Err(CoreError::WrongPassword),
            ..
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

/// CS-1a r2 must 1: «Try again» reaches the NFT ask through the core and asks for a new
/// snapshot to do it. FEAT-0054 (TEAM.md §8d-4): no more often than every 30 s — an ask that
/// failed just now is not repeated at once (the chain's `try_again_asks_at_once_and_only_once`
/// has the one 30 s old asked at once).
#[tokio::test(flavor = "multi_thread")]
async fn refresh_nfts_asks_again_no_sooner_than_30_s_after_a_failed_ask() {
    let mut core = core_at(&fresh_dir("refresh-nfts"), TEST_WALLET);
    let owner = "owner";
    assert!(core.nfts.due(owner), "the first ask");
    core.nfts.found(owner, Err("timeout".into()));
    assert!(
        !core.nfts.due(owner),
        "on its own, the next ask waits for REFRESH"
    );
    let (event, snapshot) = core.handle(Command::RefreshNfts).await;
    assert!(
        event.is_none() && snapshot,
        "no answer of its own: a new snapshot"
    );
    assert!(
        !core.nfts.due(owner),
        "the failed ask is not 30 s old: not again yet"
    );
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
        tax_transfer: false,
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
        tax_transfer: false,
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

/// BUG-232 review M1: the rescan owed to addresses taken in past the gap waits while the filters
/// are not synced, is taken at the first synced snapshot, and only once; two noted keep the
/// lower height; another wallet's is not taken for this one.
#[test]
fn a_rescan_past_the_gap_waits_for_the_sync_and_is_asked_once() {
    let wallet: WalletId = [1; 32];
    let other: WalletId = [2; 32];
    let mut pending = None;
    note_past_the_gap(&mut pending, wallet, 1_592_250);
    note_past_the_gap(&mut pending, wallet, 1_590_000);
    assert_eq!(
        take_past_the_gap(&mut pending, &wallet, false),
        None,
        "not synced"
    );
    assert_eq!(
        take_past_the_gap(&mut pending, &other, true),
        None,
        "another wallet"
    );
    assert_eq!(
        take_past_the_gap(&mut pending, &wallet, true),
        Some(1_590_000)
    );
    assert_eq!(take_past_the_gap(&mut pending, &wallet, true), None, "once");
}

/// BUG-232 review r2 S1: when one wallet's rescan for coins is done, only its own outputs are
/// settled — those that got a coin and those still without one; another wallet's stay owed.
#[test]
fn a_rescan_for_coins_settles_only_its_own_wallets_outputs() {
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{OutPoint, Txid};
    let (a, b): (WalletId, WalletId) = ([1; 32], [2; 32]);
    let out = |n: u8| OutPoint::new(Txid::from_byte_array([n; 32]), 1);
    let mut pending = vec![(a, out(1)), (b, out(2)), (b, out(3))];
    let (none, got) = settle_gap_pending(&mut pending, &b, &[out(3)]);
    assert_eq!((none, got), (vec![out(3)], vec![out(2)]));
    assert_eq!(pending, vec![(a, out(1))], "the other wallet's stays owed");
    let (none, got) = settle_gap_pending(&mut pending, &b, &[]);
    assert!(none.is_empty() && got.is_empty(), "nothing of b's left");
    assert_eq!(
        settle_gap_pending(&mut pending, &a, &[]),
        (vec![], vec![out(1)])
    );
    assert!(pending.is_empty());
}

/// BUG-269: a Workshop model's picture, read from Platform on a copy that did not make it (no
/// local picture, no NFT of the wallet), is there to draw by the model's key — the core said
/// «No picture» before.
#[tokio::test]
async fn a_workshop_model_read_back_has_its_picture_to_draw() {
    use dpp::document::{Document, DocumentV0};
    use dpp::platform_value::Value;
    let core = opened("workshop-picture", "pw");
    let jpeg: Vec<u8> = [0xFF, 0xD8, 0xFF, 0xE0]
        .into_iter()
        .chain((0..9_000u32).map(|i| (i % 251) as u8))
        .collect();
    let mut properties: std::collections::BTreeMap<String, Value> =
        [("imageType".to_owned(), Value::U8(3))].into();
    for (i, part) in jpeg.chunks(5_120).enumerate() {
        properties.insert(format!("part{i}"), Value::Bytes(part.to_vec()));
    }
    let doc = Document::V0(DocumentV0 {
        properties,
        ..Default::default()
    });
    let art = evolite_chain::workshop_read::model_of(&doc).art;
    assert!(core.stored_art(&art).is_none(), "not read yet");
    core.chain.keep_workshop_picture(&doc);
    let stored = core.stored_art(&art).expect("its picture");
    assert_eq!(&*stored.bytes, jpeg.as_slice());
    assert_eq!(stored.packing, evolite_render::Packing::Plain);
}

/// FEAT-0049 r1 Must: a withdrawal from the pool to an address of this wallet is «to your Core
/// balance», not «sent privately»; to someone else's it stays a payment out. The Core payment it
/// became says «from the pool» only when its address matches; another payout stays as it was.
#[test]
fn a_withdrawal_to_own_core_is_its_own_line_and_its_arrival_is_matched() {
    let line = |id: u64, kind: ActivityKind, chain, address: &str| {
        let (layers, flow) = crate::layers::of(&kind, chain);
        Activity {
            id,
            kind,
            chain,
            at: 1_000,
            amount: -100,
            proof: Proof::Proved,
            txid: None,
            fee: None,
            block: None,
            address: Some(address.into()),
            layers,
            flow,
            key: String::new(),
            detail: None,
            note: None,
            rate: None,
            tax_transfer: false,
        }
    };
    let platform = evolite_api::Chain::Platform;
    let mut ops = vec![
        line(1, ActivityKind::PoolSent, platform, "yChange"),
        line(2, ActivityKind::PoolSent, platform, "yTheirs"),
    ];
    own_pool_withdrawals(&mut ops, &["yReceive".into(), "yChange".into()]);
    assert_eq!(ops[0].kind, ActivityKind::PoolToCore { arrived: false });
    assert_eq!(
        ops[0].layers,
        evolite_api::layer::POOL | evolite_api::layer::CORE
    );
    assert_eq!(
        ops[1].kind,
        ActivityKind::PoolSent,
        "someone else's address"
    );
    let payout = |id, address: &str| {
        line(
            id,
            ActivityKind::Received {
                from: Some(tr!("withdrawal from Platform").into()),
            },
            evolite_api::Chain::Core,
            address,
        )
    };
    let mut history = ops;
    // r2: on its way, its address is held reserved; someone else's never.
    assert_eq!(
        withdrawals_on_their_way(&history),
        vec!["yChange".to_string()]
    );
    history.push(payout(3, "yChange"));
    history.push(payout(4, "yReceive"));
    match_pool_arrivals(&mut history);
    assert_eq!(history[0].kind, ActivityKind::PoolToCore { arrived: true });
    assert_eq!(
        history[2].kind,
        ActivityKind::Received {
            from: Some(tr!("withdrawal from the pool").into())
        }
    );
    assert_eq!(
        history[3].kind,
        ActivityKind::Received {
            from: Some(tr!("withdrawal from Platform").into())
        },
        "a credits withdrawal, not matched"
    );
    assert!(
        withdrawals_on_their_way(&history).is_empty(),
        "arrived: nothing to hold"
    );
}

/// FEAT-0054 (TEAM.md §8d-1, read back): a collection added by hand is in the wallet's folder,
/// so a new run (another core on the same folder) has it; «Remove» takes it out of the file. A
/// text that is not a contract id is refused before the network.
#[tokio::test(flavor = "multi_thread")]
async fn a_collection_added_by_id_is_read_back_by_a_new_run() {
    let dir = fresh_dir("added-collections");
    let id = "8rd3Ux7pQfAbCdEfGhJkLmNoPqRsTuVwXyZkV2mJ38R".to_string();
    {
        let core = core_at(&dir, TEST_WALLET);
        // Kept as the core keeps a contract it found of the standard (the check needs the
        // network: its function is `Chain::is_standard_collection`).
        core.added_collections
            .update(|all| {
                all.push(id.clone());
                ((), true)
            })
            .unwrap();
    }
    let mut again = core_at(&dir, TEST_WALLET);
    assert_eq!(again.added_collections.read(Clone::clone), vec![id.clone()]);
    let (event, _) = again
        .handle(Command::AddCollection {
            id: "not an id".into(),
        })
        .await;
    assert!(
        matches!(
            event,
            Some(Event::Done {
                what: DoneWhat::Collection,
                result: Err(_),
                ..
            })
        ),
        "{event:?}"
    );
    let (event, _) = again
        .handle(Command::RemoveCollection { id: id.clone() })
        .await;
    assert!(matches!(
        event,
        Some(Event::Done {
            what: DoneWhat::Collection,
            result: Ok(()),
            ..
        })
    ));
    let third = core_at(&dir, TEST_WALLET);
    assert!(third.added_collections.read(Clone::clone).is_empty());
}
