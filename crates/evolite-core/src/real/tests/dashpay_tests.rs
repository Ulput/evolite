use super::*;

/// A wallet restored from its phrase has its established contacts in the store at once,
/// before platform-wallet's sweep has queued their keys: a try that built nothing
/// is not success, and the count is kept only once every contact has its account.
#[test]
fn a_restored_wallet_tries_its_contacts_accounts_until_all_are_built() {
    // Two established contacts read from the store; the queue was empty: none built.
    assert!(!contact_accounts_done(2, Some(0)));
    // The try failed (no network): not done either.
    assert!(!contact_accounts_done(2, None));
    // One of two: tried again.
    assert!(!contact_accounts_done(2, Some(1)));
    // Both: kept, no more tries until more contacts are established.
    assert!(contact_accounts_done(2, Some(2)));
}

#[test]
fn a_dashpay_scan_shows_what_platform_has() {
    // No identity on Platform for the phrase: no O5, the onboarding goes on.
    assert_eq!(dashpay_scan_of(chain::DashPayFound::default()), None);
    let scan = dashpay_scan_of(chain::DashPayFound {
        identity: Some(("HFZQD1".into(), 2_391_207_000, 6)),
        name: Some("ferrowyn58".into()),
        contacts: 2,
    })
    .expect("an identity");
    assert_eq!(scan.identity_credits, 2_391_207, "credits as duffs");
    assert_eq!(
        (scan.identity_keys, scan.name.as_deref(), scan.contacts),
        (6, Some("ferrowyn58"), 2)
    );
    assert!(
        !scan.core_known,
        "Core balances come with the wallet's sync"
    );
}

/// BUG-78: no Platform read leaves before the masternode lists are in — not even with
/// the lists under way — and it is not counted as made; once they are, it goes out.
#[tokio::test]
async fn no_platform_read_goes_out_before_the_lists() {
    let chain = Chain::default();
    let id = Identifier::from([7; 32]);
    assert_eq!(chain.ready(), Err(chain::NOT_READY.to_owned()));
    chain.set_status(|s| {
        s.masternodes = Some(sync::Stage {
            height: 10,
            target: 1_000,
            done: false,
        })
    });
    assert_eq!(chain.name_of(id).await, Err(chain::NOT_READY.into()));
    assert_eq!(
        chain.resolve_name("dash").await,
        Err(chain::NOT_READY.into())
    );
    assert_eq!(
        chain.profile_of(id).await.err(),
        Some(chain::NOT_READY.into())
    );
    assert_eq!(
        chain.jembe_gifts(id, &[]).await.err(),
        Some(chain::NOT_READY.into())
    );
    assert_eq!(count(&chain.proof_reads), 0, "nothing went out");
    lists_in(&chain);
    assert_eq!(chain.ready(), Ok(()));
    // Past the guard: here it stops at the missing SDK, in the app it asks the network.
    assert_eq!(
        chain.name_of(id).await,
        Err("the chain is not started".into())
    );
    assert_eq!(count(&chain.proof_reads), 2);
}

/// BUG-78 (a): contacts read from the store before the lists are in are not asked for
/// their names, nor marked asked; the first snapshot after the lists asks them.
#[tokio::test(flavor = "multi_thread")]
async fn contact_names_wait_for_the_lists_and_then_are_asked() {
    let mut core = core_with_vault("b78-names", "pw");
    let id = some_identity();
    core.ask_names(vec![id.clone()]);
    core.ask_names(vec![id.clone()]);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(core.names_asked.lock().unwrap().is_empty(), "none marked");
    assert_eq!(count(&core.chain.proof_reads), 0, "none asked");
    lists_in(&core.chain);
    core.ask_names(vec![id.clone()]);
    assert!(core.names_asked.lock().unwrap().contains(&id));
    assert_eq!(counted(&core.chain.proof_reads, 1).await, 1, "asked once");
    // Being asked: the next snapshot does not ask it again.
    core.ask_names(vec![id]);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(count(&core.chain.proof_reads), 1);
}

/// BUG-78 next to BUG-76: after "Add a wallet" → "Back" the wallet's core asks names
/// through the parked chain it took over, once that chain has its lists.
#[tokio::test(flavor = "multi_thread")]
async fn after_going_back_from_adding_names_wait_for_the_parked_chains_lists() {
    let dir = fresh_dir("b78-back");
    let (chain, mut adding) = adding_from_open_store(&dir).await;
    let mut back = core_at(&dir, "bbbb0002");
    hand_on_chain(&mut adding, &mut back, false);
    assert!(Arc::ptr_eq(&back.chain, &chain));
    let id = some_identity();
    back.ask_names(vec![id.clone()]);
    assert!(back.names_asked.lock().unwrap().is_empty());
    lists_in(&chain);
    back.ask_names(vec![id]);
    assert_eq!(counted(&chain.proof_reads, 1).await, 1);
    back.chain.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-78: a read not made for want of the lists is "wait for the sync" to the UI.
#[test]
fn a_read_not_made_yet_is_wait_for_the_sync() {
    assert_eq!(read_failure(chain::NOT_READY.into()), CoreError::NotSynced);
    assert_eq!(
        read_failure("no answer".into()),
        CoreError::Failed("no answer".into())
    );
}

/// BUG-76 r1, should 3 (M4): a DashPay import during "Add a wallet" reads Platform through
/// the parked chain: the onboarding's own never starts, and through it the import would
/// wait out its 30 s and find nothing. Without an onboarding, through the core's own.
#[tokio::test(flavor = "multi_thread")]
async fn a_dashpay_import_while_adding_goes_through_the_parked_chain() {
    let dir = fresh_dir("b76-dashpay");
    seal_wallet(&dir, "bbbb0002");
    let mut from = core_at(&dir, "bbbb0002");
    assert!(
        Arc::ptr_eq(from.network_chain(), &from.chain),
        "no onboarding"
    );
    let chain = from.chain.clone();
    let mut adding = core_at(&dir, "cccc0003");
    hand_on_chain(&mut from, &mut adding, true);
    assert!(Arc::ptr_eq(adding.network_chain(), &chain));
    // Proofs can be checked; no store is open, so the scan ends at once, off the network.
    chain.set_status(|s| {
        s.masternodes = Some(sync::Stage {
            height: 1,
            target: 1,
            done: true,
        })
    });
    let other = "legal winner thank year wave sausage worth useful legal winner thank yellow";
    let phrase = Phrase::new(other.split_whitespace().map(str::to_owned).collect());
    let (event, _) = adding
        .handle(Command::UsePhrase {
            phrase,
            dashpay: true,
        })
        .await;
    assert!(event.is_none(), "answered later");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while count(&chain.dashpay_scans) == 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(count(&chain.dashpay_scans), 1, "through the parked chain");
    assert_eq!(count(&adding.chain.dashpay_scans), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn asks_a_contact_only_once() {
    assert!(repeat_request(Some(ContactState::Established), "bob").is_some());
    assert!(repeat_request(Some(ContactState::Outgoing), "bob").is_some());
    // Their request is answered with ours; a stranger gets one.
    assert_eq!(repeat_request(Some(ContactState::Incoming), "bob"), None);
    assert_eq!(repeat_request(None, "bob"), None);
}

#[tokio::test]
async fn a_contact_is_paid_only_to_the_address_shown() {
    let (out, events) = std::sync::mpsc::channel();
    let mut core = core_with_vault("preview", "pw");
    core.out = out;
    let meta = core.saved.clone().unwrap();
    core.unlocked = Some(Unlocked {
        id: [0; 32],
        meta,
        keys: None,
        seed: None,
    });
    // An address is its own preview.
    let address = "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ";
    assert_eq!(core.preview_payment(address.into()), Ok(()));
    match events.try_recv() {
        Ok(Event::PaymentPreview { to, result }) => {
            assert_eq!(to, address);
            assert_eq!(result.as_deref(), Ok(address));
        }
        other => panic!("{other:?}"),
    }
    // A name that is not a contact has no address from a key.
    assert!(matches!(
        core.preview_payment("@bob".into()),
        Err(CoreError::NotAvailable(_))
    ));
    // A contact whose address was not shown is not paid.
    core.contacts = vec![Contact {
        identity_id: "F5teLR3Vq4JjzQA7jgdGjZQvJbKpMubbu5dsC4MJd1Eh".into(),
        name: Some("bob".into()),
        profile: Profile::default(),
        state: ContactState::Established,
        since: 0,
        payments: 0,
        last_payment: None,
    }];
    assert!(matches!(
        core.send(0, Source::Core, "@bob", 1_000, (None, None, None))
            .await,
        Err(CoreError::NotAvailable(_))
    ));
    // A contact without a name is paid by its identity id, not taken for an address.
    core.contacts[0].name = None;
    assert!(matches!(
        core.send(
            0,
            Source::Core,
            "F5teLR3Vq4JjzQA7jgdGjZQvJbKpMubbu5dsC4MJd1Eh",
            1_000,
            (None, None, None),
        )
        .await,
        Err(CoreError::NotAvailable(_))
    ));
}

#[tokio::test]
async fn names_wait_for_the_masternode_lists() {
    let dir = std::env::temp_dir().join(format!("evolite-test-names-{}", std::process::id()));
    let (out, answers) = std::sync::mpsc::channel();
    let mut core = Real::new(dir, TEST_WALLET.into(), out, Arc::new(|| {}));
    // No proof can be checked yet: "not synced" now, the answer later from the task.
    let (event, _) = core
        .handle(Command::CheckName {
            label: "@Evolite2".into(),
        })
        .await;
    assert!(event.is_none());
    match answers.try_recv() {
        Ok(Event::NameChecked { label, result }) => {
            assert_eq!(label, "@Evolite2", "the label comes back as asked");
            assert_eq!(result, Err(CoreError::NotSynced));
        }
        other => panic!("{other:?}"),
    }
    // A contested name (letters only, under 20) is said at once, with no wait.
    let (event, _) = core
        .handle(Command::CheckName {
            label: "@Evolite".into(),
        })
        .await;
    assert!(
        matches!(
            &event,
            Some(Event::NameChecked {
                label,
                result: Ok(NameCheck::Contested),
            }) if label == "@Evolite"
        ),
        "{event:?}"
    );
    // Not a name at all: refused at once, nothing waits.
    let (event, _) = core
        .handle(Command::ResolveName {
            name: "@a_b".into(),
        })
        .await;
    assert!(matches!(
        event,
        Some(Event::NameResolved {
            result: Err(CoreError::BadName),
            ..
        })
    ));
    assert!(answers.try_recv().is_err());

    // A name that can be asked, before the lists: "not synced" now, and nothing else until
    // Platform is ready: the question waits.
    let (event, _) = core
        .handle(Command::ResolveName {
            name: "@Evolite2".into(),
        })
        .await;
    assert!(event.is_none());
    assert!(matches!(
        answers.try_recv(),
        Ok(Event::NameResolved {
            result: Err(CoreError::NotSynced),
            ..
        })
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(answers.try_recv().is_err(), "the real answer has to wait");
}
