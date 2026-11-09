use super::*;

/// R-5: a coin that is neither in a block nor InstantSend-locked is not spent: its parent may
/// never be mined, and the payment with it. Only such inputs can get an InstantSend lock.
#[test]
fn a_payment_spends_only_final_coins() {
    use key_wallet::dashcore::address::Payload;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{Network, OutPoint, PubkeyHash, TxOut, Txid};
    let address = Address::new(
        Network::Testnet,
        Payload::PubkeyHash(PubkeyHash::from_byte_array([2; 20])),
    );
    let build = |confirmed: bool, instant: bool, trusted: bool| {
        let mut coin = key_wallet::Utxo::new(
            OutPoint::new(Txid::from_byte_array([1; 32]), 0),
            TxOut {
                value: 5_000_000,
                script_pubkey: address.script_pubkey(),
            },
            address.clone(),
            100,
            false,
        );
        coin.is_confirmed = confirmed;
        coin.is_instantlocked = instant;
        coin.is_trusted = trusted;
        payment_builder(&address, 1_000_000)
            .set_current_height(200)
            .set_change_address(address.clone())
            .add_inputs([coin])
            .build_unsigned_reserved()
            .is_ok()
    };
    assert!(build(true, false, false), "in a block");
    assert!(build(false, true, false), "InstantSend-locked");
    assert!(!build(false, false, false), "someone's, unconfirmed");
    assert!(!build(false, false, true), "own change, not locked yet");
}
use std::time::{Duration, Instant};

/// BUG-57 r1: a lock is «added» after a restart only by a consumed row of this wallet in
/// this folder's store; an unconsumed row, another wallet's row, or no row says nothing.
#[test]
fn consumed_locks_are_this_wallets_consumed_rows() {
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{OutPoint, Txid, consensus};
    // The store's own encoding (r2): bincode, the length 32, the txid, the vout as a varint.
    let stored = |outpoint: &OutPoint| {
        let mut blob = vec![0x20];
        blob.extend_from_slice(outpoint.txid.as_byte_array());
        match outpoint.vout {
            v @ 0..=250 => blob.push(v as u8),
            v => {
                blob.push(0xfb);
                blob.extend_from_slice(&(v as u16).to_le_bytes());
            }
        }
        blob
    };
    let dir = std::env::temp_dir().join(format!("evolite-consumed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = dir.join("platform-wallet.db");
    let conn = rusqlite::Connection::open(&store).unwrap();
    conn.execute_batch(
        "CREATE TABLE asset_locks (wallet_id BLOB NOT NULL, outpoint BLOB NOT NULL, \
         status TEXT NOT NULL, PRIMARY KEY (wallet_id, outpoint))",
    )
    .unwrap();
    let ours: WalletId = [7; 32];
    let theirs: WalletId = [8; 32];
    // Bytes that differ, so a txid read in the wrong order would not match.
    let lock = |byte: u8, vout: u32| {
        let bytes: [u8; 32] = std::array::from_fn(|i| byte.wrapping_mul(31).wrapping_add(i as u8));
        OutPoint::new(Txid::from_byte_array(bytes), vout)
    };
    let insert = |wallet: WalletId, blob: Vec<u8>, status: &str| {
        conn.execute(
            "INSERT INTO asset_locks VALUES (?1, ?2, ?3)",
            rusqlite::params![wallet.as_slice(), blob, status],
        )
        .unwrap();
    };
    insert(ours, stored(&lock(1, 0)), "consumed");
    insert(ours, stored(&lock(4, 300)), "consumed");
    insert(ours, stored(&lock(2, 0)), "chain_locked");
    insert(theirs, stored(&lock(3, 0)), "consumed");
    // Another encoding (36-byte consensus, what r1 read): not the store's, left out.
    insert(ours, consensus::serialize(&lock(5, 0)), "consumed");
    drop(conn);
    let found = consumed_locks(&store, &ours).unwrap();
    assert_eq!(
        found,
        HashSet::from([lock(1, 0).txid.to_string(), lock(4, 300).txid.to_string()])
    );
    assert!(
        consumed_locks(&dir.join("none.db"), &ours).is_err(),
        "no store, no answer"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-57 r2 on a real store (a COPY: it is only read): every wallet's consumed locks.
/// `EVOLITE_STORE=<copy>\platform-wallet.db EVOLITE_CONSUMED=<txid prefix>
/// cargo t consumed_locks_of_a_real_store -- --ignored --nocapture`
#[test]
#[ignore = "reads a copy of a store named in EVOLITE_STORE"]
fn consumed_locks_of_a_real_store() {
    let store = std::path::PathBuf::from(std::env::var("EVOLITE_STORE").expect("EVOLITE_STORE"));
    let wallets: Vec<Vec<u8>> = {
        let conn = rusqlite::Connection::open_with_flags(
            &store,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut stmt = conn
            .prepare("SELECT DISTINCT wallet_id FROM asset_locks")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect()
    };
    let mut all = HashSet::new();
    for wallet in wallets {
        let id: WalletId = wallet.try_into().expect("a 32-byte wallet id");
        let found = consumed_locks(&store, &id).unwrap();
        println!("wallet {}…: consumed {found:?}", crate::hex(&id[..4]));
        all.extend(found);
    }
    if let Ok(prefix) = std::env::var("EVOLITE_CONSUMED") {
        assert!(
            all.iter().any(|t| t.starts_with(&prefix)),
            "{prefix} not in {all:?}"
        );
    }
}

/// BUG-60, the chain started again (a failed start, a switch, a lock): the stages count
/// from where the new client begins, not from where the last one left them.
#[tokio::test(flavor = "multi_thread")]
async fn a_start_again_counts_from_where_it_begins() {
    let dir = std::env::temp_dir().join(format!("evolite-again-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // A folder where the store should be: the store does not open, the start fails.
    std::fs::create_dir_all(dir.join("platform-wallet.db")).unwrap();
    let paths = ChainPaths {
        spv: dir.join("spv"),
        store: dir.join("platform-wallet.db"),
        core_node: dir.join("core-node.txt"),
    };
    let chain = Chain::default();
    {
        let mut s = chain.status.lock().unwrap();
        s.headers_from = Some(1_585_857);
        s.filters_from = Some(1_585_832);
        s.lists_answered = true;
    }
    assert!(
        chain
            .start(&paths, Arc::new(|_: &SyncStatus| {}))
            .await
            .is_err()
    );
    let s = chain.status();
    assert_eq!(s.phase, Phase::Error);
    assert_eq!((s.headers_from, s.filters_from), (None, None));
    assert!(!s.lists_answered);
    assert!(!chain.store_open() && !chain.running());
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-64, reads only: what a copy of a store holds for each Core transaction (its
/// context and amount). `EVOLITE_B64_STORE=<platform-wallet.db>`, `EVOLITE_B64_WALLET=<hex id>`.
#[test]
#[ignore = "reads a store copy named by EVOLITE_B64_STORE"]
fn store_contexts() {
    let store = std::env::var("EVOLITE_B64_STORE").expect("EVOLITE_B64_STORE");
    let wallet = std::env::var("EVOLITE_B64_WALLET").expect("EVOLITE_B64_WALLET");
    let mut id = [0u8; 32];
    for (i, b) in id.iter_mut().enumerate() {
        *b = u8::from_str_radix(&wallet[2 * i..2 * i + 2], 16).unwrap();
    }
    for r in stored_history(std::path::Path::new(&store), &id).unwrap() {
        let context = match &r.context {
            key_wallet::transaction_checking::TransactionContext::InstantSend(_) => {
                "InstantSend".to_owned()
            }
            c => format!("{c:?}"),
        };
        println!(
            "{} {context} {:?} {:?} net {} once {} ins {} outs {}",
            r.txid,
            r.transaction_type,
            r.direction,
            r.net_amount,
            crate::convert::net_once(&r),
            r.input_details.len(),
            r.output_details.len()
        );
        // BUG-86: one transaction's details, `EVOLITE_B64_TX=<txid prefix>`.
        if std::env::var("EVOLITE_B64_TX").is_ok_and(|t| r.txid.to_string().starts_with(&t)) {
            println!(
                "  {:?} fee {:?} tx ins {} outs {}",
                r.account_type,
                r.fee,
                r.transaction.input.len(),
                r.transaction.output.len()
            );
            for d in &r.input_details {
                println!("  in {} {} {}", d.index, d.value, d.address);
            }
            for d in &r.output_details {
                println!(
                    "  out {} {:?} {} {:?}",
                    d.index,
                    d.role,
                    d.value,
                    d.address.as_ref().map(|a| a.to_string())
                );
            }
            for (i, o) in r.transaction.output.iter().enumerate() {
                println!("  txout {i} {}", o.value);
            }
        }
    }
    // BUG-86 (§4a item 4): the History lines from the rows as written and as read now —
    // the same lines, but for the ones the fix is for.
    let conn = open_read_only(std::path::Path::new(&store)).unwrap();
    let lines = |mut records: Vec<_>| {
        crate::convert::attribute_late_inputs(&mut records);
        records
            .iter()
            .map(|r| {
                let a = crate::convert::activity(r, 0, true);
                (
                    r.txid.to_string(),
                    format!("{:?} {} {:?}", a.kind, a.amount, a.fee),
                )
            })
            .collect::<Vec<_>>()
    };
    let before = lines(stored_records(&conn, &id).unwrap());
    let after = lines(stored_history(std::path::Path::new(&store), &id).unwrap());
    assert_eq!(before.len(), after.len());
    let mut changed = 0;
    for ((txid, was), (_, now)) in before.iter().zip(&after) {
        if was != now {
            changed += 1;
            println!("CHANGED {txid}: {was} -> {now}");
        }
    }
    println!("lines {} changed {changed}", after.len());
}

/// BUG-86: a row kept as one account's slice — the output to an address of account 0
/// `Sent` — is read back with that output ours, by the store's own address pools; an
/// output to a contact's address (`dashpay_external`, where we pay them) stays paid out. A
/// store without the pools table gives the rows as written.
#[test]
fn a_stored_slice_gets_the_wallets_own_outputs_back() {
    use key_wallet::dashcore::address::Payload;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{PubkeyHash, Transaction, TxIn, TxOut};
    use key_wallet::managed_account::transaction_record::{
        InputDetail, OutputDetail, OutputRole, TransactionDirection, TransactionRecord,
    };
    use key_wallet::transaction_checking::TransactionContext;
    use key_wallet::transaction_checking::transaction_router::TransactionType;
    let address = |n: u8| {
        key_wallet::dashcore::Address::new(
            key_wallet::Network::Testnet,
            Payload::PubkeyHash(PubkeyHash::from_byte_array([n; 20])),
        )
    };
    let (mine, contact, change) = (address(7), address(9), address(8));
    let tx = Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn::default()],
        output: [(&mine, 600), (&contact, 300), (&change, 50)]
            .map(|(a, value)| TxOut {
                value,
                script_pubkey: a.script_pubkey(),
            })
            .to_vec(),
        special_transaction_payload: None,
    };
    let out = |index: u32, at: &key_wallet::dashcore::Address, value| OutputDetail {
        index,
        role: OutputRole::Sent,
        address: Some(at.clone()),
        value,
    };
    let slice = TransactionRecord::new(
        tx,
        key_wallet::account::AccountType::Standard {
            index: 1,
            standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
        },
        TransactionContext::Mempool,
        TransactionType::Standard,
        TransactionDirection::Outgoing,
        vec![InputDetail {
            index: 0,
            value: 1_000,
            address: address(5),
        }],
        vec![
            out(0, &mine, 600),
            out(1, &contact, 300),
            out(2, &change, 50),
        ],
        -1_000,
    );
    let dir = std::env::temp_dir().join(format!("evolite-b86-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("platform-wallet.db");
    let wallet: WalletId = [0x86; 32];
    let config = bincode::config::standard();
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE core_transactions (wallet_id BLOB, txid BLOB, record_blob BLOB);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO core_transactions VALUES (?1, ?2, ?3)",
            rusqlite::params![
                wallet.as_slice(),
                slice.txid.to_byte_array().as_slice(),
                bincode::serde::encode_to_vec(&slice, config).unwrap()
            ],
        )
        .unwrap();
    }
    let roles = |records: Vec<TransactionRecord>| {
        assert_eq!(records.len(), 1);
        let r = &records[0];
        let roles: Vec<_> = r.output_details.iter().map(|d| d.role).collect();
        (roles, r.direction, r.net_amount)
    };
    // No pools table (an older store): as written.
    assert_eq!(
        roles(stored_history(&db, &wallet).unwrap()),
        (
            vec![OutputRole::Sent; 3],
            TransactionDirection::Outgoing,
            -1_000
        )
    );
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE core_address_pool (wallet_id BLOB, account_type TEXT, \
             pool_type INTEGER, script BLOB);",
        )
        .unwrap();
        let other: WalletId = [0x87; 32];
        for (id, kind, pool, at) in [
            (&wallet, "standard_bip44", 0, &mine),
            (&wallet, "standard_bip44", 1, &change),
            (&wallet, "dashpay_external", 2, &contact),
            (&other, "standard_bip44", 0, &contact),
        ] {
            conn.execute(
                "INSERT INTO core_address_pool VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id.as_slice(), kind, pool, at.script_pubkey().as_bytes()],
            )
            .unwrap();
        }
    }
    assert_eq!(
        roles(stored_history(&db, &wallet).unwrap()),
        (
            vec![OutputRole::Received, OutputRole::Sent, OutputRole::Change],
            TransactionDirection::Outgoing,
            -350
        ),
        "ours received and change, the contact's paid"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// BUG-13 r1 should 1: "Act as" moves on between the check of a request and its signing —
/// the shown identity changes, the one checked does not.
#[test]
fn the_identity_checked_is_the_one_that_signs() {
    let wallet: WalletId = [0x13; 32];
    let (a, b) = (Identifier::from([1u8; 32]), Identifier::from([2u8; 32]));
    let mine = || [(0, a, "a"), (1, b, "b")].into_iter();
    choose_identity(wallet, 0);
    let (_, checked) = shown_of(wallet, mine().map(|(s, id, _)| (s, id))).unwrap();
    assert_eq!(checked, a);
    choose_identity(wallet, 1);
    assert_eq!(
        shown_of(wallet, mine().map(|(s, _, n)| (s, n))),
        Some((1, "b")),
        "shown now"
    );
    assert_eq!(
        with_id(mine(), &checked),
        Some((0, "a")),
        "signs as checked"
    );
    let other = Identifier::from([3u8; 32]);
    assert_eq!(with_id(mine(), &other), None, "gone from the wallet");
}

/// BUG-7: which tracked locks hold "The top-up stopped", and which only await a ChainLock.
#[test]
fn an_unfinished_top_up_is_a_lock_platform_has_not_taken() {
    use AssetLockFundingType::{IdentityRegistration, IdentityTopUp};
    use AssetLockStatus::*;
    for status in [Built, Broadcast, InstantSendLocked, ChainLocked] {
        assert_eq!(top_up_state(IdentityTopUp, &status, None), Some(false));
    }
    for status in [Consumed, DrainCompleted, RecoveredFromChain] {
        assert_eq!(
            top_up_state(IdentityTopUp, &status, None),
            None,
            "{status:?}"
        );
    }
    assert_eq!(top_up_state(IdentityRegistration, &Built, None), None);
    // "Already taken" without a ChainLock yet: shown as awaiting, not as to be continued.
    assert_eq!(
        top_up_state(
            IdentityTopUp,
            &InstantSendLocked,
            Some(TopUpNote::AwaitingChainLock)
        ),
        Some(true)
    );
    assert_eq!(
        top_up_state(IdentityTopUp, &InstantSendLocked, Some(TopUpNote::Credited)),
        None
    );
    // Its ChainLock proof is kept: nothing left, whatever was noted.
    assert_eq!(
        top_up_state(
            IdentityTopUp,
            &RecoveredFromChain,
            Some(TopUpNote::AwaitingChainLock)
        ),
        None
    );
}

/// BUG-7 (seen on testnet): "already taken" for a ChainLocked lock arrives as platform-wallet's
/// failure to record it; only that failure counts as such.
#[test]
fn a_taken_lock_with_a_chain_lock_is_recognised() {
    use platform_wallet::PlatformWalletError as E;
    // As platform-wallet 575d961 words it with evolite's SQLite store (testnet, BUG-7).
    let refused = E::Persistence(
        "asset-lock reconciliation requires persistence capabilities [\"wallet_restore\"] \
         (missing mask 0x80)"
            .into(),
    );
    assert!(is_taken_with_chain_lock(&refused));
    assert!(!is_cached_duplicate(&refused));
    assert!(!is_taken_with_chain_lock(&E::Persistence(
        "disk full".into()
    )));
}

/// BUG-106: "already in the cache" names no lock, so the one to send again is the one this
/// attempt made. A stopped top-up of the same identity (or another's) that sorts first in the
/// wallet's list is not touched, and when the attempt's own lock cannot be named, nothing is sent.
#[test]
fn a_repeat_sends_only_the_lock_this_attempt_made() {
    use AssetLockFundingType::{IdentityRegistration, IdentityTopUp};
    use AssetLockStatus::{Broadcast, Consumed, InstantSendLocked, RecoveredFromChain};
    use key_wallet::dashcore::{OutPoint, Txid};
    let lock = |n: u8| OutPoint::new(Txid::from([n; 32]), 0);
    let (stopped, other, new) = (lock(1), lock(2), lock(9));
    let known = HashSet::from([stopped, other]);
    let before = [
        (stopped, IdentityTopUp, InstantSendLocked, 3),
        (other, IdentityTopUp, InstantSendLocked, 0),
    ];
    let with = |more: &[(OutPoint, AssetLockFundingType, AssetLockStatus, u32)]| {
        let mut all = before.to_vec();
        all.extend_from_slice(more);
        all
    };
    let made = || (new, IdentityTopUp, InstantSendLocked, 3);
    assert_eq!(
        lock_of_this_attempt(&known, &with(&[made()]), Some(3)),
        Some(new),
        "the new one, not the stopped one of the same slot"
    );
    let cases = [
        (with(&[]), Some(3), "nothing new appeared"),
        (
            with(&[made(), (lock(10), IdentityTopUp, Broadcast, 3)]),
            Some(3),
            "two new: no guessing",
        ),
        (
            with(&[(new, IdentityTopUp, InstantSendLocked, 4)]),
            Some(3),
            "another identity's slot",
        ),
        (
            with(&[(new, IdentityRegistration, InstantSendLocked, 3)]),
            Some(3),
            "not a top-up",
        ),
        (
            with(&[(new, IdentityTopUp, RecoveredFromChain, 3)]),
            Some(3),
            "found on chain, not made here",
        ),
        (
            with(&[(new, IdentityTopUp, Consumed, 3)]),
            Some(3),
            "already consumed",
        ),
        (with(&[made()]), None, "the identity's slot is unknown"),
    ];
    for (tracked, slot, why) in cases {
        assert_eq!(lock_of_this_attempt(&known, &tracked, slot), None, "{why}");
    }
    // A foreign lock of the slot next to the new one is not "two new": it was there before.
    assert_eq!(
        lock_of_this_attempt(
            &HashSet::from([stopped]),
            &[made(), (stopped, IdentityTopUp, Broadcast, 3)],
            Some(3)
        ),
        Some(new)
    );
}

/// BUG-7 (seen on testnet): the same lock sent again is "already in the cache" — that alone
/// is sent again as a new transition; Platform's own verdicts are not.
#[test]
fn only_a_cached_duplicate_is_sent_again() {
    use platform_wallet::PlatformWalletError as E;
    let cached = E::Sdk(dash_sdk::Error::AlreadyExists(
        "tx already exists in cache".into(),
    ));
    assert!(is_cached_duplicate(&cached));
    assert!(!is_cached_duplicate(&E::Sdk(dash_sdk::Error::Generic(
        "x".into()
    ))));
    assert!(!is_cached_duplicate(&E::AssetLockAlreadyConsumed(
        Default::default()
    )));
}

/// BUG-7: "the lock is already taken" answers by the identity's credits and the lock.
#[test]
fn an_already_taken_lock_is_judged_by_the_credits() {
    use AssetLockStatus::*;
    assert_eq!(
        already_taken(Some(100), Some(50_000_100), Some(InstantSendLocked)),
        AlreadyTaken::Credited(50_000_100)
    );
    assert_eq!(
        already_taken(None, Some(7), None),
        AlreadyTaken::Credited(7)
    );
    // Credited before (an earlier sync saw it), no ChainLock yet: wait, do not fail.
    assert_eq!(
        already_taken(Some(100), Some(100), Some(InstantSendLocked)),
        AlreadyTaken::AwaitChainLock
    );
    assert_eq!(
        already_taken(Some(100), None, Some(InstantSendLocked)),
        AlreadyTaken::AwaitChainLock
    );
    // platform-wallet proved it by ChainLock: settled.
    assert_eq!(
        already_taken(Some(100), Some(100), Some(RecoveredFromChain)),
        AlreadyTaken::Settled
    );
    assert_eq!(
        already_taken(Some(100), Some(100), Some(Consumed)),
        AlreadyTaken::Settled
    );
}

/// BUG-9: the names of identities the store already had, and of new ones whose read
/// failed, are read again; a new identity platform-wallet just named is not.
#[test]
fn a_discovery_reads_the_names_platform_wallet_skipped() {
    let (old, fresh, nameless) = (
        Identifier::from([1u8; 32]),
        Identifier::from([2u8; 32]),
        Identifier::from([3u8; 32]),
    );
    let known = [(old, true), (fresh, true), (nameless, false)];
    let none = HashSet::new();
    assert_eq!(
        names_to_read(&known, &[fresh, nameless], &none),
        vec![old, nameless]
    );
    assert!(names_to_read(&[], &[], &none).is_empty());
    // On the next pass of the same search, a name read already is not read again.
    let read = HashSet::from([old]);
    assert_eq!(names_to_read(&known, &[], &read), vec![fresh, nameless]);
}

/// BUG-9 r1: a scan with unanswered slots, or a name that did not load, still brings what it
/// found, but is not complete: the search goes on. Unanswered slots are named first.
#[test]
fn a_discovery_is_complete_only_with_every_slot_and_name() {
    let done = discovery_answer(3, false, false);
    assert_eq!((done.found, done.complete, done.missing), (3, true, None));
    let slots = discovery_answer(2, true, false);
    assert!(!slots.complete && slots.found == 2 && slots.missing.is_some());
    let names = discovery_answer(0, false, true);
    assert!(!names.complete && names.missing.is_some());
    assert_ne!(slots.missing, names.missing);
    // BUG-9 r2 should-1: a name that did not load leaves the slots complete (a slot can be
    // chosen, a lock used); an unanswered slot does not.
    assert!(done.slots_complete && names.slots_complete);
    assert!(!slots.slots_complete && !discovery_answer(1, true, true).slots_complete);
    assert_eq!(discovery_answer(1, true, true).missing, slots.missing);
}

#[test]
fn the_creation_steps_follow_the_lock() {
    assert_eq!(creation_step_of(None), 1);
    assert_eq!(creation_step_of(Some(&AssetLockStatus::Built)), 1);
    assert_eq!(creation_step_of(Some(&AssetLockStatus::Broadcast)), 2);
    assert_eq!(
        creation_step_of(Some(&AssetLockStatus::InstantSendLocked)),
        3
    );
    assert_eq!(creation_step_of(Some(&AssetLockStatus::ChainLocked)), 3);
    assert_eq!(creation_step_of(Some(&AssetLockStatus::Consumed)), 4);
}

/// The chosen identity is shown while the wallet has it; otherwise the lowest slot.
#[test]
fn the_chosen_identity_wins_while_it_exists() {
    let wallet = [7u8; 32];
    assert_eq!(shown_slot(wallet, [2, 0, 1].into_iter()), Some(0));
    choose_identity(wallet, 1);
    assert_eq!(shown_slot(wallet, [2, 0, 1].into_iter()), Some(1));
    // Another wallet keeps its own choice.
    assert_eq!(shown_slot([8u8; 32], [2, 0, 1].into_iter()), Some(0));
    // A choice the wallet no longer has falls back to the lowest.
    assert_eq!(shown_slot(wallet, [0, 2].into_iter()), Some(0));
    assert_eq!(shown_slot(wallet, std::iter::empty()), None);
}

/// FEAT-0014 AC-T1: the new identity goes one past the highest slot; a lock still waiting
/// for its identity holds its slot, and once it is finished the next one is past it.
#[test]
fn a_new_identity_takes_the_slot_past_the_highest() {
    assert_eq!(next_slot([], None), 0);
    assert_eq!(next_slot([0, 1, 2], None), 3);
    assert_eq!(next_slot([2, 0, 1], None), 3, "in any order");
    // Creation cut short in slot 3: the slot is held.
    assert_eq!(next_slot([0, 1, 2], Some(3)), 4);
    // "Continue" made it: slot 3 is an identity now, the lock is gone.
    assert_eq!(next_slot([0, 1, 2, 3], None), 4);
    // A gap left by an identity made elsewhere is not filled: nothing is reused.
    assert_eq!(next_slot([0, 4], None), 5);
}

/// What a write costs is read from the identity that signed it: the chosen one, not the
/// lowest slot (the history said 0 spent after choosing another identity).
#[test]
fn credits_are_read_from_the_identity_that_signs() {
    let wallet = [9u8; 32];
    let mine = || [(0, "first"), (1, "second"), (2, "third")].into_iter();
    assert_eq!(shown_of(wallet, mine()), Some((0, "first")));
    choose_identity(wallet, 1);
    assert_eq!(shown_of(wallet, mine()), Some((1, "second")));
    assert_eq!(shown_of(wallet, std::iter::empty::<(u32, &str)>()), None);
}

/// A Platform read proved with quorum keys from our own SPV masternode list: no trusted
/// server in the loop. Testnet, read-only, minutes on a first run.
/// `cargo test -p evolite-chain live_spv_proved_read -- --ignored --nocapture`
#[test]
#[ignore = "needs testnet access and minutes"]
fn live_spv_proved_read() {
    // GroveDB proof checks recurse deeply: big stacks, as the core's own thread has.
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 << 20)
                .build()
                .unwrap()
                .block_on(proved_read())
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Jembe's gifts of a holder on testnet (`EVOLITE_TEST_NAME` in `.env.test.local`, who got
/// copies at the release): read with proofs, every picture checked against its `imageHash`.
/// Read-only, no phrase.
/// `cargo test -p evolite-chain live_jembe_gifts -- --ignored --nocapture`
#[test]
#[ignore = "needs testnet access and minutes"]
fn live_jembe_gifts() {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 << 20)
                .build()
                .unwrap()
                .block_on(jembe_gifts_of_a_holder())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn jembe_gifts_of_a_holder() {
    let dir = std::env::temp_dir().join("evolite-live-provider");
    let paths = ChainPaths {
        spv: dir.join("spv"),
        store: dir.join("platform-wallet.db"),
        core_node: dir.join("core-node.txt"),
    };
    let chain = Chain::default();
    chain
        .start(&paths, Arc::new(|_: &SyncStatus| {}))
        .await
        .expect("chain starts");
    let started = Instant::now();
    while !chain.platform_ready() {
        assert!(
            started.elapsed() < Duration::from_secs(30 * 60),
            "not ready"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let name = crate::test_name("EVOLITE_TEST_NAME");
    let holder = chain
        .resolve_name(&name)
        .await
        .expect("proved read")
        .expect("the holder's name is registered");
    let id = Identifier::from_string(&holder, Encoding::Base58).unwrap();
    let (gifts, models) = chain.jembe_gifts(id, &[]).await.expect("gifts read");
    println!(
        "{name} = {holder}: {} gifts, {} models",
        gifts.len(),
        models.len()
    );
    assert!(!gifts.is_empty(), "{name} got gifts at the release");
    for model in &models {
        let (art, stored, verified) = nfts::picture(model);
        println!(
            "  {} (format {}): {} B packed, {}, hash {}",
            model.name,
            model.image_format,
            stored.bytes.len(),
            if verified { "verified" } else { "NOT matching" },
            &art[..12]
        );
        assert!(verified, "{} matches its imageHash", model.name);
    }
    for gift in &gifts {
        println!(
            "  gift {} № {} of {}, transferred at {:?}, price {:?}",
            &gift.id[..8],
            gift.serial,
            gift.model_id.chars().take(8).collect::<String>(),
            gift.transferred_at,
            gift.price
        );
        assert!(models.iter().any(|m| m.id == gift.model_id));
        // Only the issuer creates gifts: one the holder has was handed over.
        assert!(!gift.held_by_issuer);
    }
    // Asked again with the models known: none read twice.
    let known: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
    let (again, none) = chain.jembe_gifts(id, &known).await.expect("gifts read");
    assert_eq!((again.len(), none.len()), (gifts.len(), 0));
}

async fn proved_read() {
    let dir = std::env::temp_dir().join("evolite-live-provider");
    let paths = ChainPaths {
        spv: dir.join("spv"),
        store: dir.join("platform-wallet.db"),
        core_node: dir.join("core-node.txt"),
    };
    let chain = Chain::default();
    chain
        .start(&paths, Arc::new(|_: &SyncStatus| {}))
        .await
        .expect("chain starts");
    let started = Instant::now();
    loop {
        let s = chain.status();
        if s.masternodes.is_some_and(|m| m.done) {
            println!(
                "masternode lists ready in {}s: {s:?}",
                started.elapsed().as_secs()
            );
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30 * 60),
            "not ready: {s:?}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    assert!(chain.platform_ready());
    let dash = chain.resolve_name("dash").await.expect("proved read");
    println!("dash.dash -> {dash:?}");
    assert!(dash.is_some(), "dash.dash is registered on testnet");
    assert_eq!(chain.name_available("dash").await, Ok(false));
    assert_eq!(
        chain.resolve_name("evolite-probe-surely-free-42").await,
        Ok(None)
    );
    assert_eq!(
        chain.name_available("evolite-probe-surely-free-42").await,
        Ok(true)
    );
    let sdk = chain.sdk().await.unwrap();
    println!("protocol version {}", sdk.protocol_version_number());
    chain.stop().await;
}

/// A value from `.env.test.local` at the workspace root (never committed, never printed).
fn test_env(key: &str) -> Option<zeroize::Zeroizing<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env.test.local");
    let text = zeroize::Zeroizing::new(std::fs::read_to_string(path).ok()?);
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| zeroize::Zeroizing::new(v.trim().trim_matches('"').to_owned()))
}

/// Starts the chain on `dir`, opens the test wallet, waits until coins and proofs are
/// known.
async fn open_test_wallet(
    dir: &std::path::Path,
) -> (Chain, Arc<PlatformWallet>, key_wallet::Wallet) {
    let phrase = test_env("EVOLITE_TEST_PHRASE").expect("EVOLITE_TEST_PHRASE in .env.test.local");
    let paths = ChainPaths {
        // Headers and masternode lists from the other live tests: not fetched again.
        spv: std::env::temp_dir()
            .join("evolite-live-send")
            .join("spv")
            .join("testnet"),
        store: dir
            .join("wallets")
            .join("testnet")
            .join("platform-wallet.db"),
        core_node: dir.join("core-node.txt"),
    };
    let chain = Chain::default();
    chain
        .start(&paths, Arc::new(|_: &SyncStatus| {}))
        .await
        .expect("chain starts");
    let (_, wallet) = chain.open_wallet(&phrase, 0).await.expect("wallet opens");
    let keys = crate::keys_of(&phrase).expect("keys");
    let started = Instant::now();
    loop {
        let s = chain.status();
        let filters = s.filters.is_some_and(|f| f.done);
        if filters && chain.platform_ready() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(40 * 60),
            "not ready: {s:?}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    (chain, wallet, keys)
}

/// An identity registration cut short after its money was locked is found again after a
/// restart (platform-wallet keeps the lock in its store) and finished with its name,
/// without locking more. Run it twice, as two processes are two runs of the app: the first
/// locks 0.013 tDASH of the test phrase and stops; the second finds the lock and finishes.
/// `cargo test -p evolite-chain live_resume_identity -- --ignored --nocapture`
#[test]
#[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
fn live_resume_identity_after_a_restart() {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 << 20)
                .build()
                .unwrap()
                .block_on(resume_after_restart())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn resume_after_restart() {
    let dir = std::env::temp_dir().join("evolite-live-resume");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let label = format!("evolite-{}", stamp % 100_000_000 + 22_222_222);

    let (chain, wallet, keys) = open_test_wallet(&dir).await;
    chain.discover_identities(&wallet, &keys).await.unwrap();
    let Some(unfinished) = chain.unfinished(&wallet).await else {
        // The first run: the money is locked, then the app "falls".
        let index = chain.next_identity_index(&wallet, &keys).await.unwrap();
        let (_, _, out_point) = wallet
            .asset_locks()
            .create_funded_asset_lock_proof(
                1_300_000,
                0,
                AssetLockFundingType::IdentityRegistration,
                index,
                &identity::AssetLockSigner(keys.clone()),
            )
            .await
            .expect("asset lock");
        println!("locked {out_point} for slot {index}; run the test again to finish it");
        chain.stop().await;
        return;
    };
    // The second run, on the same files.
    println!("found after a restart: {unfinished:?}");
    let index = unfinished.identity_index;
    let started = Instant::now();
    let id = chain
        .resume_identity(&wallet, &keys, index, Some(&label), 165_000)
        .await
        .expect("resumed");
    println!(
        "identity {id} with @{label} in {}s",
        started.elapsed().as_secs()
    );
    assert!(
        chain.unfinished(&wallet).await.is_none(),
        "nothing unfinished now"
    );
    assert!(
        chain.nameless(&wallet, index).await.is_none(),
        "the name is there"
    );
    assert_eq!(chain.resolve_name(&label).await, Ok(Some(id)));
    chain.stop().await;
}

/// Read only: each identity of the test phrase, its credits as the store of the warm wallet
/// keeps them and as Platform has them now (a proved balance query). Shows whether the
/// store lags behind, and what a top-up really added. Opens a copy of
/// `%TEMP%\evolite-live-warm` when there is one, so nothing is scanned again.
/// `cargo test -p evolite-chain live_identity_balances -- --ignored --nocapture`
#[test]
#[ignore = "needs testnet access and a test phrase in .env.test.local"]
fn live_identity_balances_against_platform() {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 << 20)
                .build()
                .unwrap()
                .block_on(balances_against_platform())
        })
        .unwrap()
        .join()
        .unwrap();
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

async fn balances_against_platform() {
    let dir = std::env::temp_dir().join("evolite-live-balances");
    let warm = std::env::temp_dir().join("evolite-live-warm");
    let _ = std::fs::remove_dir_all(&dir);
    if warm.join("wallets").exists() {
        copy_dir(&warm, &dir).expect("copy of the warm wallet");
    }
    let (chain, wallet, keys) = open_test_wallet(&dir).await;
    chain.discover_identities(&wallet, &keys).await.unwrap();
    let kept: Vec<(u32, Identifier, u64)> = {
        let state = wallet.state().await;
        state
            .identity_manager
            .managed_identities()
            .filter(|m| m.wallet_id == Some(wallet.wallet_id()))
            .filter_map(|m| Some((m.identity_index?, m.identity.id(), m.identity.balance())))
            .collect()
    };
    assert!(!kept.is_empty(), "the test phrase has identities");
    for (slot, id, store) in kept {
        let platform = wallet
            .identity()
            .refresh_identity_balance(&id)
            .await
            .expect("a proved balance");
        println!(
            "slot {slot} {}: store {store}, Platform {platform} credits ({:+})",
            id.to_string(Encoding::Base58),
            platform as i128 - store as i128
        );
    }
    chain.stop().await;
}

/// R-1 / BUG-26 on testnet: the wallet pays itself and receives the payment's InstantLock
/// from the network. dash-spv marks the transaction InstantSend; evolite counts it so only
/// once the lock's signature was verified. Prints when each happened. Only the fee is spent.
/// `cargo test -p evolite-chain live_instant_lock_verified -- --ignored --nocapture`
#[test]
#[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
fn live_instant_lock_verified_before_it_counts() {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 << 20)
                .build()
                .unwrap()
                .block_on(instant_lock_verified())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn instant_lock_verified() {
    use key_wallet::transaction_checking::TransactionContext;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let dir = std::env::temp_dir().join("evolite-live-instant-lock");
    let (chain, wallet, keys) = open_test_wallet(&dir).await;
    let to: Address = wallet
        .core()
        .next_receive_address_for_account(0)
        .await
        .expect("an address");
    let paid = chain.pay(&wallet, &keys, &to, 10_000).await.expect("paid");
    println!("paid {} relayed {}", paid.txid, paid.relayed);
    let txid: key_wallet::dashcore::Txid = paid.txid.parse().unwrap();
    let started = Instant::now();
    let (mut marked, mut verified) = (None, None);
    while started.elapsed() < Duration::from_secs(120) && verified.is_none() {
        let context = {
            let state = wallet.state().await;
            state
                .core_wallet
                .transaction_history()
                .iter()
                .find(|r| r.txid == txid)
                .map(|r| r.context.clone())
        };
        let is_marked = matches!(context, Some(TransactionContext::InstantSend(_)));
        let is_verified = chain.instant_lock_verified(&txid);
        if is_marked && marked.is_none() {
            marked = Some(started.elapsed());
            println!("{:?}: dash-spv marked it InstantSend", started.elapsed());
        }
        if is_verified {
            verified = Some(started.elapsed());
            println!("{:?}: its lock verified", started.elapsed());
        }
        // What the history shows at this moment.
        if let Some(context) = &context
            && is_marked
            && !is_verified
        {
            let shown = crate::convert::proof(context, is_verified);
            assert_eq!(
                shown,
                evolite_api::Proof::Mempool,
                "not InstantSend unverified"
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        verified.is_some(),
        "the network's InstantLock verified in 2 min"
    );
    chain.stop().await;
}

/// BUG-9 on testnet, read-only: one discovery brings every identity of the test phrase and
/// every name, on a fresh store and again on one that already has them (the second run of
/// the app). Proved by a direct read of each identity's names, which adds nothing.
/// `cargo test -p evolite-chain live_discovery_brings -- --ignored --nocapture`
#[test]
#[ignore = "needs testnet access and a test phrase in .env.test.local"]
fn live_discovery_brings_every_identity_and_name() {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 << 20)
                .build()
                .unwrap()
                .block_on(discovery_brings_everything())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn discovery_brings_everything() {
    let dir = std::env::temp_dir().join("evolite-live-discovery");
    let _ = std::fs::remove_dir_all(&dir);
    let (chain, wallet, keys) = open_test_wallet(&dir).await;
    for run in ["fresh store", "store with the identities"] {
        let started = Instant::now();
        let found = chain.discover_identities(&wallet, &keys).await;
        println!("{run}: {found:?} in {}s", started.elapsed().as_secs_f32());
        assert!(
            found.as_ref().is_ok_and(|d| d.complete),
            "{run}: every slot and name answered"
        );
        let kept: Vec<(Identifier, usize)> = {
            let state = wallet.state().await;
            assert!(
                !state
                    .identity_manager
                    .identity_scan_is_incomplete(&wallet.wallet_id())
            );
            of_wallet(&state.identity_manager, wallet.wallet_id())
                .map(|m| (m.identity.id(), m.dpns_names.len()))
                .collect()
        };
        assert!(!kept.is_empty(), "the test phrase has identities");
        for (id, names) in kept {
            let added = wallet.identity().sync_dpns_names(&id).await;
            println!(
                "  {}: {names} name(s), read again: {added:?}",
                id.to_string(Encoding::Base58)
            );
            assert_eq!(added.ok(), Some(0), "{run}: no name was missing");
        }
    }
    chain.stop().await;
}

/// A top-up cut short after its money was locked is found again after a restart and
/// finished from the same lock, without locking more. Run it twice: the first run locks
/// 0.001 tDASH for the first identity and stops; the second finds the lock and tops up.
/// `cargo test -p evolite-chain live_resume_top_up -- --ignored --nocapture`
#[test]
#[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
fn live_resume_top_up_after_a_restart() {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 << 20)
                .build()
                .unwrap()
                .block_on(resume_top_up_after_restart())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn resume_top_up_after_restart() {
    let dir = std::env::temp_dir().join("evolite-live-resume-top-up");
    let (chain, wallet, keys) = open_test_wallet(&dir).await;
    chain.discover_identities(&wallet, &keys).await.unwrap();
    let first = || async {
        let state = wallet.state().await;
        state
            .identity_manager
            .managed_identities()
            .filter(|m| m.wallet_id == Some(wallet.wallet_id()))
            .filter_map(|m| Some((m.identity_index?, m.identity.balance())))
            .min_by_key(|(index, _)| *index)
            .expect("the test phrase has an identity")
    };
    let Some(unfinished) = chain.unfinished_top_up(&wallet).await else {
        // The first run: the money is locked, then the app "falls".
        let (index, _) = first().await;
        let (_, _, out_point) = wallet
            .asset_locks()
            .create_funded_asset_lock_proof(
                100_000,
                0,
                AssetLockFundingType::IdentityTopUp,
                index,
                &identity::AssetLockSigner(keys.clone()),
            )
            .await
            .expect("asset lock");
        println!("locked {out_point} for identity slot {index}; run the test again");
        chain.stop().await;
        return;
    };
    // The second run, on the same files.
    println!("found after a restart: {unfinished:?}");
    let (_, before) = first().await;
    let started = Instant::now();
    let balance = chain.resume_top_up(&wallet, &keys).await.expect("resumed");
    println!(
        "credits {before} -> {balance} in {}s",
        started.elapsed().as_secs()
    );
    assert!(balance > before, "the credits grew");
    assert!(
        chain.unfinished_top_up(&wallet).await.is_none(),
        "nothing unfinished now"
    );
    chain.stop().await;
}

/// BUG-7 on testnet: Platform takes a top-up's lock but the wallet never hears of it (the
/// answer lost), as on the customer's wallet. "Continue" must not be a generic error: it
/// says the ChainLock is awaited, a new top-up is not held back, and once the ChainLock
/// comes "Check again" settles the lock. Spends 0.001 tDASH; up to ~15 min.
/// `cargo test -p evolite-chain live_top_up_taken -- --ignored --nocapture`
#[test]
#[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
fn live_top_up_taken_behind_the_wallets_back() {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 << 20)
                .build()
                .unwrap()
                .block_on(top_up_taken_behind())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn top_up_taken_behind() {
    use dash_sdk::platform::transition::top_up_identity::TopUpIdentity;
    let dir = std::env::temp_dir().join("evolite-live-top-up-taken");
    let (chain, wallet, keys) = open_test_wallet(&dir).await;
    chain.discover_identities(&wallet, &keys).await.unwrap();
    let signer = identity::AssetLockSigner(keys.clone());
    // What the store tracks, for the bug file: every top-up lock and the identities.
    for l in wallet.asset_locks().list_tracked_locks().await {
        println!(
            "tracked {:?} slot {} {} duffs {:?} {}",
            l.funding_type, l.identity_index, l.amount, l.status, l.out_point
        );
    }
    {
        let state = wallet.state().await;
        for m in state
            .identity_manager
            .managed_identities()
            .filter(|m| m.wallet_id == Some(wallet.wallet_id()))
        {
            println!(
                "identity slot {:?} credits {}",
                m.identity_index,
                m.identity.balance()
            );
        }
    }
    // Never resume a lock this test did not make: the test phrase is also a person's
    // wallet, whose stopped top-ups are theirs to finish (BUG-7, werner555). The test's
    // own lock is noted in its folder, so a run cut short can be picked up again.
    let mine = dir.join("test-lock.txt");
    let noted = std::fs::read_to_string(&mine).ok();
    let made = if let Some(stopped) = chain.unfinished_top_up(&wallet).await {
        assert_eq!(
            noted.as_deref().map(str::trim),
            Some(stopped.out_point.to_string().as_str()),
            "an unfinished top-up not made by this test is tracked: not touching it"
        );
        stopped.out_point
    } else {
        let (index, who) = {
            let state = wallet.state().await;
            state
                .identity_manager
                .managed_identities()
                .filter(|m| m.wallet_id == Some(wallet.wallet_id()))
                .filter_map(|m| Some((m.identity_index?, m.identity.clone())))
                .min_by_key(|(index, _)| *index)
                .expect("the test phrase has an identity")
        };
        let (proof, path, out_point) = wallet
            .asset_locks()
            .create_funded_asset_lock_proof(
                100_000,
                0,
                AssetLockFundingType::IdentityTopUp,
                index,
                &signer,
            )
            .await
            .expect("asset lock");
        std::fs::write(&mine, out_point.to_string()).expect("note the test's lock");
        // Platform takes the lock behind platform-wallet's back: its answer is "lost".
        let sdk = chain.sdk().await.expect("sdk");
        let credits = who
            .top_up_identity_with_signer(&sdk, proof, &path, &signer, None)
            .await
            .expect("Platform took the lock");
        println!("lock {out_point} taken by Platform behind the wallet: {credits} credits");
        // The credits are already in the store, as after a later sync: they do not grow.
        let _ = wallet.identity().refresh_identity(&who.id()).await;
        out_point
    };
    let stopped = chain
        .unfinished_top_up(&wallet)
        .await
        .expect("the wallet still sees the top-up as stopped");
    println!("stopped: {stopped:?}");
    assert_eq!(
        stopped.out_point, made,
        "only the test's own lock is resumed"
    );
    let started = Instant::now();
    let mut settled = false;
    while started.elapsed() < Duration::from_secs(15 * 60) {
        let answer = chain.resume_top_up(&wallet, &keys).await;
        let now = chain.unfinished_top_up(&wallet).await;
        println!(
            "after {}s: {answer:?}; unfinished {now:?}",
            started.elapsed().as_secs()
        );
        match answer {
            // Settled: credited now, or taken on an earlier attempt (no "Credits added").
            Ok(_) => {
                assert!(now.is_none(), "a settled lock is not shown");
                settled = true;
                break;
            }
            Err(text) if text == top_up_settled_text() => {
                assert!(now.is_none(), "a settled lock is not shown");
                settled = true;
                break;
            }
            Err(text) => {
                assert_eq!(text, awaiting_chain_lock_text(), "not a generic error");
                assert!(
                    now.is_some_and(|u| u.awaiting_chain_lock),
                    "shown as awaiting its ChainLock, not to be continued"
                );
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
    assert!(settled, "the lock settled once its block had a ChainLock");
    chain.stop().await;
}

/// A stopped chain lets go of its store, so a deleted wallet's folder can go at once and
/// a new wallet be made in the same window. Starts the SPV client, so it is run by
/// hand: `cargo test -p evolite-chain a_stopped_chain -- --ignored`.
#[test]
#[ignore = "starts the SPV client, which reaches testnet"]
fn a_stopped_chain_lets_go_of_the_store() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(8 << 20)
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = std::env::temp_dir().join(format!("evolite-b73-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = ChainPaths {
            spv: dir.join("spv").join("testnet"),
            store: dir.join("wallets").join("testnet").join("platform-wallet.db"),
            core_node: dir.join("core-node.txt"),
        };
        let chain = Chain::default();
        chain
            .start(&paths, Arc::new(|_: &SyncStatus| {}))
            .await
            .expect("chain starts");
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        drop(chain.open_wallet(phrase, 0).await.expect("wallet opens"));
        chain.stop().await;
        assert!(
            std::fs::remove_dir_all(dir.join("wallets")).is_ok(),
            "the store is still held after stop"
        );
        let _ = std::fs::remove_dir_all(&dir);
    });
}
