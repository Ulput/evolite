use super::*;

/// "Add a wallet" → "Create": the new wallet is born at the tip the wallet before saw, and
/// is made while this core's chain still starts; platform-wallet gets it later.
#[tokio::test]
async fn a_wallet_is_made_while_the_chain_starts() {
    let dir = fresh_dir("create-starting");
    let mut core = core_at(&dir, "cccc0003");
    core.pending = Some((Zeroizing::new(PHRASE.into()), false));
    core.birth_hint = Some(777);
    core.starting = Some(tokio::spawn(std::future::pending::<bool>()));
    let made =
        tokio::time::timeout(Duration::from_secs(20), core.create(&pw("Second-wallet-7"))).await;
    assert_eq!(
        made.ok(),
        Some(Ok(())),
        "made without waiting for the start"
    );
    assert_eq!(core.saved.as_ref().map(|m| m.birth_height), Some(777));
    assert!(core.unlocked.is_some() && core.attach.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Making a wallet waits for the chain's start and takes its answer once: the loop then
/// never polls the finished start again (that panicked the core thread).
#[tokio::test]
async fn a_settled_start_is_taken_once() {
    let mut core = core_with_vault("start-settled", "pw");
    core.starting = Some(tokio::spawn(async { false }));
    core.start_settled(Duration::from_secs(5)).await;
    assert!(core.starting.is_none() && core.leftovers && !core.start_tried);
    assert!(!core.start_ended().await);
    core.start_settled(Duration::from_secs(5)).await;
    // A start that does not end in time is left to the loop.
    core.starting = Some(tokio::spawn(std::future::pending::<bool>()));
    core.start_settled(Duration::from_millis(10)).await;
    assert!(core.starting.is_some());
    assert!(!core.start_ended().await);
}

/// BUG-65: "Add a wallet" → "Back": the wallet gone back to opens its own store while the
/// adding chain's start (its SPV client, ≈50 s) still runs, so after the password its
/// identities, balance and history are there at once — not only once that start and its
/// stop have ended. The SPV client still waits for them (the SPV files are shared).
#[tokio::test(flavor = "multi_thread")]
async fn going_back_from_adding_reads_the_store_at_once() {
    let dir = fresh_dir("back-from-adding");
    seal_wallet(&dir, "bbbb0002");
    // The adding core: a new folder with no vault, its chain's start still under way.
    let adding = core_at(&dir, "cccc0003");
    let (release, held) = tokio::sync::oneshot::channel::<()>();
    let adding_start = tokio::spawn(async move {
        let _ = held.await;
        false
    });
    // The chain before the adding one has stopped: its start said so.
    let (said, before) = watch::channel(false);
    let _ = said.send(true);
    adding.chain.retire();

    let mut back = core_at(&dir, "bbbb0002");
    back.start_chain(
        Some(adding.chain.clone()),
        Some(adding_start),
        Some(before),
        true,
    );
    let opened = async {
        while !back.chain.store_open() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    assert!(
        tokio::time::timeout(Duration::from_secs(20), opened)
            .await
            .is_ok(),
        "the store opens while the adding chain still starts"
    );
    assert!(!back.chain.running(), "the SPV client waits");
    assert!(!back.starting.as_ref().unwrap().is_finished());
    // The password now: the wallet is read from the store before the client runs.
    assert_eq!(back.unlock(&pw("pw")).await, Ok(()));
    let id = back.unlocked.as_ref().map(|o| o.id).unwrap();
    assert!(back.chain.wallet(&id).await.is_some(), "at once");
    // The next core's store may open once this one's start has stopped the adding chain.
    let mut next = back.stopped.clone().unwrap();
    assert!(!*next.borrow());
    // No SPV client in a test: this chain is replaced before its turn comes.
    back.chain.retire();
    let _ = release.send(());
    let started = back.starting.take().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(20), started)
            .await
            .is_ok()
    );
    assert!(next.wait_for(|s| *s).await.is_ok());
    assert!(!back.chain.running());
    back.chain.stop().await;
    assert!(!back.chain.store_open());
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-65, the other way: without word that the chains before have let their stores go —
/// the core before had the same wallet, say — the store waits for the start before.
#[tokio::test(flavor = "multi_thread")]
async fn a_store_held_before_is_not_opened_ahead() {
    let dir = fresh_dir("back-same-wallet");
    seal_wallet(&dir, "bbbb0002");
    let before = core_at(&dir, "bbbb0002");
    before.chain.retire();
    let (release, held) = tokio::sync::oneshot::channel::<()>();
    let start_before = tokio::spawn(async move {
        let _ = held.await;
        false
    });
    let mut back = core_at(&dir, "bbbb0002");
    back.start_chain(Some(before.chain.clone()), Some(start_before), None, false);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !back.chain.store_open(),
        "not while the one before may hold it"
    );
    back.chain.retire();
    let _ = release.send(());
    let started = back.starting.take().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(20), started)
            .await
            .is_ok()
    );
    assert!(!back.chain.store_open());
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-65 r1: the loop lets the next core open its store ahead only when the core before
/// had another wallet — the same one again (a restart after a delete that failed) would
/// open its store a second time.
#[test]
fn only_another_wallet_opens_its_store_ahead() {
    let said = || {
        let (stopped, before) = watch::channel(true);
        (stopped, Some(before))
    };
    let (_a, before) = said();
    assert!(open_ahead_after("aaaa0001", "bbbb0002", before).is_some());
    let (_b, before) = said();
    assert!(open_ahead_after("bbbb0002", "bbbb0002", before).is_none());
    assert!(open_ahead_after("aaaa0001", "bbbb0002", None).is_none());
}

/// BUG-65 r1: the wallet whose store was opened ahead is deleted before its SPV client's
/// turn, and its core replaced: the start ends at once — no 10 s of trying to remove a
/// folder its own store holds, which held up the next wallet's start — starting nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_wallet_deleted_while_its_store_waits_holds_up_nothing() {
    let dir = fresh_dir("deleted-in-the-window");
    let (_adding, mut back, release) = opened_ahead(&dir).await;
    // Deleted: the vault goes at once (F4); the loop replaces the core.
    std::fs::remove_file(&back.paths.vault).expect("the vault");
    back.chain.retire();
    let _ = release.send(());
    let started = back.starting.take().unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(5), started).await;
    assert_eq!(ended.ok().map(|r| r.ok()), Some(Some(false)), "at once");
    assert!(!back.chain.running());
    back.chain.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same before the core is replaced: a deleted wallet's store opened ahead is not
/// cleared under itself, and no SPV client starts for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_wallets_open_store_is_not_cleared_under_it() {
    let dir = fresh_dir("deleted-not-retired");
    let (_adding, mut back, release) = opened_ahead(&dir).await;
    std::fs::remove_file(&back.paths.vault).expect("the vault");
    let _ = release.send(());
    let started = back.starting.take().unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(5), started).await;
    assert_eq!(ended.ok().map(|r| r.ok()), Some(Some(false)), "at once");
    assert!(!back.chain.running(), "no client for a deleted wallet");
    back.chain.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-76: "Add a wallet" → "Back" keeps the wallet's chain — not stopped, not started
/// again, its store open throughout — and how far it was rescanned (BUG-64). The
/// onboarding meanwhile starts no chain of its own, not even when a retry would be due.
#[tokio::test(flavor = "multi_thread")]
async fn going_back_from_adding_keeps_the_chain_running() {
    let dir = fresh_dir("b76-back");
    let (chain, mut adding) = adding_from_open_store(&dir).await;
    assert!(adding.parked.is_some());
    assert!(!Arc::ptr_eq(&adding.chain, &chain));
    assert!(adding.starting.is_none(), "the onboarding starts nothing");
    // A retry is due by the clock: still nothing (were it made, it would only count).
    adding.chain.retire();
    adding.last_start = tokio::time::Instant::now() - RETRY_START * 2;
    adding.retry_start();
    assert_eq!(count(&adding.chain.starts), 0, "no start while adding");
    assert!(chain.store_open() && !chain.retired());

    let mut back = core_at(&dir, "bbbb0002");
    hand_on_chain(&mut adding, &mut back, false);
    assert!(Arc::ptr_eq(&back.chain, &chain), "the same chain");
    assert_eq!((count(&chain.starts), count(&chain.stops)), (0, 0));
    assert!(chain.store_open() && !chain.retired());
    assert_eq!(back.rescanned_from, Some(1_234));
    let owed = back
        .rescan_owed
        .as_ref()
        .map(|o| (o.up_to, o.stale.clone()));
    assert_eq!(
        owed,
        Some((1_600_000, vec!["t".to_owned()])),
        "still owed after Back"
    );
    assert!(adding.rescan_owed.is_none());
    assert!(back.parked.is_none() && back.starting.is_none());
    // The password: the wallet is in that store at once.
    assert_eq!(back.unlock(&pw("pw")).await, Ok(()));
    let id = back.unlocked.as_ref().map(|o| o.id).unwrap();
    assert!(back.chain.wallet(&id).await.is_some());
    back.chain.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-76: "Add a wallet" → "Create": the parked chain stops (its start under way ends
/// first) before the new wallet's chain starts, and the next core is told only then.
#[tokio::test(flavor = "multi_thread")]
async fn a_wallet_made_while_adding_stops_the_parked_chain_first() {
    let dir = fresh_dir("b76-create");
    let (chain, mut adding) = adding_from_open_store(&dir).await;
    let (release, start) = held_start();
    adding.parked.as_mut().unwrap().starting = Some(start);
    adding.pending = Some((Zeroizing::new(PHRASE.into()), false));
    adding.birth_hint = Some(777);
    assert_eq!(adding.create(&pw("Second-wallet-7")).await, Ok(()));
    // No SPV client in a test: the new chain is retired before its turn comes.
    adding.chain.retire();
    assert!(adding.parked.is_none());
    assert!(chain.retired(), "a start of it waiting in line is not made");
    assert_eq!(count(&adding.chain.starts), 1, "its own start is asked");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(chain.store_open(), "the start before is waited for");
    let mut next = adding.stopped.clone().unwrap();
    assert!(!*next.borrow());
    let _ = release.send(());
    let started = adding.starting.take().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(20), started)
            .await
            .is_ok()
    );
    assert_eq!(count(&chain.stops), 1, "the parked chain is stopped");
    assert!(!chain.store_open());
    assert!(next.wait_for(|s| *s).await.is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-76: from the onboarding to a third wallet (not "Back"): the parked chain stops
/// before that wallet's start, as after a switch.
#[tokio::test(flavor = "multi_thread")]
async fn another_wallet_after_adding_stops_the_parked_chain() {
    let dir = fresh_dir("b76-third");
    let (chain, mut adding) = adding_from_open_store(&dir).await;
    let (release, start) = held_start();
    adding.parked.as_mut().unwrap().starting = Some(start);
    seal_wallet(&dir, "aaaa0001");
    let mut third = core_at(&dir, "aaaa0001");
    hand_on_chain(&mut adding, &mut third, false);
    // No SPV client in a test: the new chain is retired before its turn comes.
    third.chain.retire();
    let _ = release.send(());
    assert!(!Arc::ptr_eq(&third.chain, &chain));
    let started = third.starting.take().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(20), started)
            .await
            .is_ok()
    );
    assert_eq!(count(&chain.stops), 1);
    assert!(!chain.store_open() && chain.retired());
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-76 r1, should 1: a parked chain that does not run ("Add a wallet" while offline)
/// is started again during the onboarding — that chain, not the onboarding's own — but
/// not before [`RETRY_START`], and not over a start of it still under way.
#[tokio::test(flavor = "multi_thread")]
async fn a_parked_chain_that_did_not_start_is_started_again() {
    let dir = fresh_dir("b76-retry-parked");
    let (chain, mut adding) = adding_from_open_store(&dir).await;
    // No SPV client in a test: retired, a start that is made only counts.
    chain.retire();
    let due = tokio::time::Instant::now() - RETRY_START * 2;
    adding.parked.as_mut().unwrap().last_start = tokio::time::Instant::now();
    adding.retry_start();
    assert_eq!(count(&chain.starts), 0, "not due yet");
    let (release, start) = held_start();
    {
        let parked = adding.parked.as_mut().unwrap();
        parked.starting = Some(start);
        parked.last_start = due;
    }
    adding.retry_start();
    assert_eq!(count(&chain.starts), 0, "a start under way is not doubled");
    let _ = release.send(());
    let finished = || {
        adding
            .parked
            .as_ref()
            .and_then(|p| p.starting.as_ref())
            .is_some_and(|t| t.is_finished())
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !finished() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    adding.retry_start();
    assert_eq!(count(&chain.starts), 1, "the parked chain is started again");
    assert_eq!(
        count(&adding.chain.starts),
        0,
        "the onboarding's own is not"
    );
    let again = adding.parked.as_mut().unwrap().starting.take().unwrap();
    assert_eq!(again.await.ok(), Some(false), "retired: nothing is made");
    adding.retry_start();
    assert_eq!(
        count(&chain.starts),
        1,
        "just tried: not again before RETRY_START"
    );
    chain.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-76 r1, should 2 (M3): "Add a wallet" → "Create" births the new wallet at the tip of
/// the parked chain's headers, not at the lower hint the wallet before left.
#[tokio::test(flavor = "multi_thread")]
async fn a_wallet_made_while_adding_is_born_at_the_parked_chains_tip() {
    let dir = fresh_dir("b76-birth");
    let (chain, mut adding) = adding_from_open_store(&dir).await;
    chain.set_status(|s| {
        s.headers = Some(sync::Stage {
            height: 900,
            target: 900,
            done: true,
        })
    });
    let (release, start) = held_start();
    adding.parked.as_mut().unwrap().starting = Some(start);
    adding.pending = Some((Zeroizing::new(PHRASE.into()), false));
    adding.birth_hint = Some(777);
    assert_eq!(adding.create(&pw("Second-wallet-7")).await, Ok(()));
    // No SPV client in a test: the new chain is retired before its turn comes.
    adding.chain.retire();
    assert_eq!(adding.saved.as_ref().map(|m| m.birth_height), Some(900));
    let _ = release.send(());
    let started = adding.starting.take().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(20), started)
            .await
            .is_ok()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-76: the app closes during the onboarding: the parked chain stops with the core's
/// own, its store let go.
#[tokio::test(flavor = "multi_thread")]
async fn quitting_while_adding_stops_both_chains() {
    let dir = fresh_dir("b76-quit");
    let (chain, mut adding) = adding_from_open_store(&dir).await;
    let own = adding.chain.clone();
    adding.shut_down().await;
    assert_eq!((count(&own.stops), count(&chain.stops)), (1, 1));
    assert!(!chain.store_open() && adding.parked.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-76, the paths that park nothing keep their order: a switch stops the chain before
/// the next wallet's start (BUG-21/22/24), and opens that store ahead only for another
/// wallet (BUG-65).
#[tokio::test(flavor = "multi_thread")]
async fn a_switch_still_stops_the_chain_before() {
    let dir = fresh_dir("b76-switch");
    seal_wallet(&dir, "aaaa0001");
    seal_wallet(&dir, "bbbb0002");
    let mut from = core_at(&dir, "bbbb0002");
    from.chain
        .open_store_only(&from.paths.chain)
        .await
        .expect("the store opens");
    let chain = from.chain.clone();
    let (release, start) = held_start();
    from.starting = Some(start);
    let mut to = core_at(&dir, "aaaa0001");
    hand_on_chain(&mut from, &mut to, false);
    to.chain.retire();
    let _ = release.send(());
    assert!(to.parked.is_none() && chain.retired());
    let started = to.starting.take().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(20), started)
            .await
            .is_ok()
    );
    assert_eq!(count(&chain.stops), 1);
    assert!(!chain.store_open());
    assert_eq!(count(&to.chain.starts), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
