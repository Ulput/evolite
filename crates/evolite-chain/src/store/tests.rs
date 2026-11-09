use super::*;
use platform_wallet::changeset::{
    CoreChangeSet, DashPayBackfillCoveredContact, PersistenceErrorKind, persists_backfill_coverage,
};
use std::sync::atomic::{AtomicBool, Ordering};

const ID: WalletId = [7; 32];
const OTHER: WalletId = [8; 32];

/// SQLite as far as the record cares: inline, atomic, and a commit that can fail.
#[derive(Default)]
struct Sqlite {
    fail: AtomicBool,
}

impl PlatformWalletPersistence for Sqlite {
    fn store_commits_inline(&self) -> bool {
        true
    }
    fn persistence_capabilities(&self) -> PersistenceCapabilities {
        PersistenceCapabilities::ATOMIC_CHANGESETS
            .union(PersistenceCapabilities::ACCOUNT_ADDRESS_POOLS)
    }
    fn store(&self, _: WalletId, _: PlatformWalletChangeSet) -> Result<(), PersistenceError> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(PersistenceError::backend_with_kind(
                PersistenceErrorKind::Fatal,
                "the commit failed",
            ));
        }
        Ok(())
    }
    fn flush(&self, _: WalletId) -> Result<(), PersistenceError> {
        Ok(())
    }
    fn load(&self) -> Result<ClientStartState, PersistenceError> {
        Ok(ClientStartState::default())
    }
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("evolite-store-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("platform-wallet.db")
}

fn open(db: &Path) -> Store<Sqlite> {
    Store::new(Sqlite::default(), db.to_owned())
}

fn contact(n: u8, covered_from: u32) -> DashPayBackfillCoveredContact {
    DashPayBackfillCoveredContact {
        owner: Identifier::from([1; 32]),
        contact: Identifier::from([n; 32]),
        account_index: 0,
        covered_from,
    }
}

/// A backfill round as platform-wallet stores it: the record and the cursor lowered to `to`.
fn rewind(
    to: u32,
    from: u32,
    covered: Vec<DashPayBackfillCoveredContact>,
) -> PlatformWalletChangeSet {
    PlatformWalletChangeSet {
        core: Some(CoreChangeSet {
            synced_height: Some(to),
            synced_height_is_rewind: true,
            ..CoreChangeSet::default()
        }),
        dashpay_backfill: Some(DashPayBackfillRecord::from_entries(to, from, covered)),
        ..PlatformWalletChangeSet::default()
    }
}

/// The scan's progress, as the filter batches report it.
fn scanned(to: u32) -> PlatformWalletChangeSet {
    PlatformWalletChangeSet {
        core: Some(CoreChangeSet {
            synced_height: Some(to),
            ..CoreChangeSet::default()
        }),
        ..PlatformWalletChangeSet::default()
    }
}

/// werner555's case: the store at 1 600 000, a contact paid at 1 582 131. The backfill rewinds,
/// and the scan climbs back to 1 600 000.
fn backfilled(db: &Path) -> Store<Sqlite> {
    let store = open(db);
    store.take_back(&ID, 1_600_000);
    store
        .store(
            ID,
            rewind(1_582_131, 1_600_000, vec![contact(2, 1_582_131)]),
        )
        .unwrap();
    store.store(ID, scanned(1_590_000)).unwrap();
    store.store(ID, scanned(1_600_000)).unwrap();
    store
}

fn saved(db: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(backfill_file(db)).unwrap()).unwrap()
}

fn write(db: &Path, value: &serde_json::Value) {
    std::fs::write(backfill_file(db), serde_json::to_vec(value).unwrap()).unwrap();
}

fn owner() -> Identifier {
    Identifier::from([1; 32])
}

/// Workaround (1): every method of the trait reaches SQLite — a method left to its default would
/// answer instead of the store — but the backfill one, and platform-wallet then keeps the record
/// durably.
#[test]
fn every_method_goes_to_the_sqlite_store_and_the_record_is_kept() {
    struct Marked;
    impl PlatformWalletPersistence for Marked {
        fn store_commits_inline(&self) -> bool {
            true
        }
        fn persistence_capabilities(&self) -> PersistenceCapabilities {
            PersistenceCapabilities::INVITATIONS
        }
        fn store_transient_is_reissuable(&self) -> bool {
            true
        }
        fn persists_durably(&self) -> bool {
            true
        }
        fn store(&self, _: WalletId, _: PlatformWalletChangeSet) -> Result<(), PersistenceError> {
            Err(PersistenceError::LockPoisoned)
        }
        fn flush(&self, _: WalletId) -> Result<(), PersistenceError> {
            Err(PersistenceError::LockPoisoned)
        }
        fn persist_tracked_masternodes(
            &self,
            _: Network,
            _: &[TrackedMasternode],
        ) -> Result<(), PersistenceError> {
            Err(PersistenceError::LockPoisoned)
        }
        fn load_tracked_masternodes(
            &self,
            _: Network,
        ) -> Result<Vec<TrackedMasternode>, PersistenceError> {
            Err(PersistenceError::LockPoisoned)
        }
        fn load(&self) -> Result<ClientStartState, PersistenceError> {
            Err(PersistenceError::LockPoisoned)
        }
        fn get_core_tx_record(
            &self,
            _: WalletId,
            _: &Txid,
        ) -> Result<Option<TransactionRecord>, PersistenceError> {
            Err(PersistenceError::LockPoisoned)
        }
        fn list_wallet_core_txids(
            &self,
            _: WalletId,
        ) -> Result<Option<Vec<ListedCoreTxid>>, PersistenceError> {
            Ok(Some(Vec::new()))
        }
        fn get_dpns_name_state(
            &self,
            _: WalletId,
            _: &Identifier,
            _: &str,
        ) -> Result<Option<DpnsNameStateEntry>, PersistenceError> {
            Err(PersistenceError::LockPoisoned)
        }
    }
    let db = folder("forward");
    let s = Store::new(Marked, db.clone());
    let poisoned =
        |r: Result<(), PersistenceError>| matches!(r, Err(PersistenceError::LockPoisoned));
    assert!(s.store_commits_inline());
    assert_eq!(
        s.persistence_capabilities(),
        PersistenceCapabilities::INVITATIONS
    );
    assert!(s.store_transient_is_reissuable());
    assert!(s.persists_durably());
    assert!(poisoned(s.store(ID, PlatformWalletChangeSet::default())));
    assert!(poisoned(s.flush(ID)));
    assert!(poisoned(
        s.persist_tracked_masternodes(Network::Testnet, &[])
    ));
    assert!(s.load_tracked_masternodes(Network::Testnet).is_err());
    assert!(s.load().is_err());
    assert!(s.get_core_tx_record(ID, &Txid::from([1; 32])).is_err());
    assert_eq!(s.list_wallet_core_txids(ID).unwrap(), Some(Vec::new()));
    assert!(s.get_dpns_name_state(ID, &owner(), "a").is_err());
    assert!(s.stores_dashpay_backfill());

    // The real store: platform-wallet keeps the record through it, not through SQLite alone.
    let sqlite = platform_wallet_storage::sqlite::SqlitePersister::open(
        platform_wallet_storage::sqlite::SqlitePersisterConfig::new(db.clone()),
    )
    .unwrap();
    assert!(!persists_backfill_coverage(&sqlite));
    let store = Store::new(sqlite, db.clone());
    assert!(persists_backfill_coverage(&store));
    drop(store);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A backfill scanned back up to where it started is taken back after a restart: the contact it
/// covers is not rewound to again (no pass over the filters at each start).
#[test]
fn a_finished_backfill_is_taken_back_after_a_restart() {
    let db = folder("finished");
    drop(backfilled(&db));
    let (record, verdict) = open(&db).take_back(&ID, 1_600_000);
    assert_eq!(verdict, Verdict::Kept);
    assert!(record.covers(&owner(), &Identifier::from([2; 32]), 0, 1_582_131));
    assert_eq!((record.floor, record.rewound_from), (1_582_131, 1_600_000));
    // The store went on: still vouched for.
    assert_eq!(open(&db).take_back(&ID, 1_600_500).1, Verdict::Kept);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// The hole SQLite leaves: it keeps its cursor as a max, so a backfill the app stopped in the
/// middle of still shows the store at 1 600 000. Its record must not be taken back — the blocks
/// above 1 590 000 were never tested with the contact watched.
#[test]
fn a_backfill_cut_short_is_not_taken_back() {
    let db = folder("cut-short");
    let store = open(&db);
    store.take_back(&ID, 1_600_000);
    store
        .store(
            ID,
            rewind(1_582_131, 1_600_000, vec![contact(2, 1_582_131)]),
        )
        .unwrap();
    store.store(ID, scanned(1_590_000)).unwrap();
    drop(store);
    let (record, verdict) = open(&db).take_back(&ID, 1_600_000);
    assert_eq!(verdict, Verdict::Unfinished);
    assert!(record.is_empty(), "the rewind again");
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A record written with the store further on than the store it lies beside now — a copy of the
/// folder, a store from a backup — vouches for blocks this store has not scanned.
#[test]
fn a_record_ahead_of_the_store_is_not_taken_back() {
    let db = folder("ahead");
    drop(backfilled(&db));
    let (record, verdict) = open(&db).take_back(&ID, 1_599_999);
    assert_eq!(verdict, Verdict::Ahead);
    assert!(record.is_empty());
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// An edited record — a contact's start moved down, a cursor moved, another wallet's record put
/// under this wallet, a broken file — is no record: the rewind as before.
#[test]
fn an_edited_or_broken_record_is_not_taken_back() {
    let db = folder("edited");
    drop(backfilled(&db));
    let good = saved(&db);
    let key = hex(&ID);
    let judged = |value: &serde_json::Value| {
        write(&db, value);
        open(&db).take_back(&ID, 1_600_000)
    };
    // A contact covered from block 0: every payment to it would be "already seen".
    let mut lower = good.clone();
    let covered = lower["wallets"][&key]["covered"]
        .as_str()
        .unwrap()
        .to_owned();
    lower["wallets"][&key]["covered"] = format!("{}00000000", &covered[..covered.len() - 8]).into();
    assert_eq!(
        judged(&lower),
        (DashPayBackfillRecord::default(), Verdict::Tampered)
    );
    let mut cursor = good.clone();
    cursor["wallets"][&key]["cursor"] = 1_000.into();
    assert_eq!(judged(&cursor).1, Verdict::Tampered);
    let mut floor = good.clone();
    floor["wallets"][&key]["floor"] = 1.into();
    assert_eq!(judged(&floor).1, Verdict::Tampered);
    // Another wallet's record moved over: its check is of the other id.
    let mut moved = good.clone();
    moved["wallets"][&key] = serde_json::to_value(Entry::new(
        &OTHER,
        1_600_000,
        &DashPayBackfillRecord::from_entries(1, 1_600_000, vec![contact(2, 1)]),
        true,
    ))
    .unwrap();
    assert_eq!(judged(&moved).1, Verdict::Tampered);
    // Another wallet's record under its own id is not this wallet's.
    let mut theirs = serde_json::json!({"wallets": {}});
    theirs["wallets"][hex(&OTHER)] = good["wallets"][&key].clone();
    assert_eq!(judged(&theirs).1, Verdict::None);
    // A cover list cut mid-entry, with its check made again: not a shorter list.
    let mut cut = good.clone();
    let mut short: Entry = serde_json::from_value(good["wallets"][&key].clone()).unwrap();
    short.covered.truncate(short.covered.len() - 2);
    short.check = short.digest(&ID);
    cut["wallets"][&key] = serde_json::to_value(short).unwrap();
    assert_eq!(judged(&cut).1, Verdict::Damaged);
    std::fs::write(backfill_file(&db), b"{not json").unwrap();
    assert_eq!(open(&db).take_back(&ID, 1_600_000).1, Verdict::Damaged);
    // The untouched one still is.
    assert_eq!(judged(&good).1, Verdict::Kept);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// The record is written after the SQLite commit only: a round SQLite refused leaves the record
/// as it was (a record ahead of its round would vouch for blocks never scanned).
#[test]
fn a_round_sqlite_refused_leaves_the_record_as_it_was() {
    let db = folder("refused");
    let store = backfilled(&db);
    let before = std::fs::read(backfill_file(&db)).unwrap();
    store.inner.fail.store(true, Ordering::Relaxed);
    assert!(
        store
            .store(
                ID,
                rewind(1_500_000, 1_600_000, vec![contact(3, 1_500_000)])
            )
            .is_err()
    );
    assert_eq!(std::fs::read(backfill_file(&db)).unwrap(), before);
    // An empty record SQLite refused does not drop it either.
    let empty = PlatformWalletChangeSet {
        dashpay_backfill: Some(DashPayBackfillRecord::default()),
        ..PlatformWalletChangeSet::default()
    };
    assert!(store.store(ID, empty.clone()).is_err());
    assert_eq!(std::fs::read(backfill_file(&db)).unwrap(), before);
    store.inner.fail.store(false, Ordering::Relaxed);
    store.store(ID, empty).unwrap();
    assert!(!backfill_file(&db).exists(), "an empty record is none");
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A stale record — the app stopped between SQLite's commit of a new backfill and the file —
/// covers what it covered: a contact added since is rewound to as before.
#[test]
fn a_stale_record_does_not_cover_a_contact_added_since() {
    let db = folder("stale");
    let store = backfilled(&db);
    let older = std::fs::read(backfill_file(&db)).unwrap();
    store
        .store(
            ID,
            rewind(
                1_595_000,
                1_600_000,
                vec![contact(2, 1_582_131), contact(3, 1_595_000)],
            ),
        )
        .unwrap();
    store.store(ID, scanned(1_600_000)).unwrap();
    drop(store);
    std::fs::write(backfill_file(&db), older).unwrap();
    let (record, verdict) = open(&db).take_back(&ID, 1_600_000);
    assert_eq!(verdict, Verdict::Kept);
    assert!(record.covers(&owner(), &Identifier::from([2; 32]), 0, 1_582_131));
    assert_eq!(
        record.covered_from(&owner(), &Identifier::from([3; 32]), 0),
        None
    );
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A second backfill (a new contact) after the first was complete: until it is scanned back up,
/// the file says unfinished and nothing of it is taken back.
#[test]
fn a_new_backfill_is_unfinished_until_scanned_back_up() {
    let db = folder("second");
    let store = backfilled(&db);
    store
        .store(
            ID,
            rewind(
                1_595_000,
                1_600_000,
                vec![contact(2, 1_582_131), contact(3, 1_595_000)],
            ),
        )
        .unwrap();
    assert_eq!(open(&db).take_back(&ID, 1_600_000).1, Verdict::Unfinished);
    store.store(ID, scanned(1_601_000)).unwrap();
    assert_eq!(open(&db).take_back(&ID, 1_601_000).1, Verdict::Kept);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// The journal says what the load did, by a name for each case.
#[test]
fn each_verdict_has_its_own_name_in_the_journal() {
    let all = [
        Verdict::Kept,
        Verdict::None,
        Verdict::Damaged,
        Verdict::Tampered,
        Verdict::Ahead,
        Verdict::Unfinished,
    ];
    let names: BTreeSet<_> = all.iter().map(|v| v.name()).collect();
    assert_eq!(names.len(), all.len());
    assert!(all[2..].iter().all(|v| v.name().starts_with("rejected_")));
}

/// The customer's question: a stale, edited, ahead or unfinished record is rejected — the rewind
/// follows — and the journal says which. Each case goes through the load as the app does it.
#[test]
fn the_journal_says_what_the_load_did_with_the_record() {
    let logs = std::env::temp_dir().join(format!("evolite-store-journal-{}", std::process::id()));
    crate::diag::init(&logs);
    let db = folder("journal");
    let cases: [(&str, Verdict, u32); 5] = [
        ("kept", Verdict::Kept, 1_600_000),
        ("ahead", Verdict::Ahead, 1_599_999),
        ("tampered", Verdict::Tampered, 1_600_000),
        ("damaged", Verdict::Damaged, 1_600_000),
        ("none", Verdict::None, 1_600_000),
    ];
    for (case, verdict, loaded) in cases {
        drop(backfilled(&db));
        match case {
            "tampered" => {
                let mut value = saved(&db);
                value["wallets"][hex(&ID)]["floor"] = 1.into();
                write(&db, &value);
            }
            "damaged" => std::fs::write(backfill_file(&db), b"{\"wallets\":").unwrap(),
            "none" => std::fs::remove_file(backfill_file(&db)).unwrap(),
            _ => {}
        }
        let (record, got) = open(&db).take_back(&ID, loaded);
        assert_eq!(got, verdict, "{case}");
        assert_eq!(
            record.is_empty(),
            verdict != Verdict::Kept,
            "{case}: the rewind follows"
        );
    }
    // Unfinished: the app stopped in the middle of the climb.
    let store = open(&db);
    store.take_back(&ID, 1_600_000);
    store
        .store(
            ID,
            rewind(1_590_000, 1_600_000, vec![contact(4, 1_590_000)]),
        )
        .unwrap();
    drop(store);
    assert_eq!(open(&db).take_back(&ID, 1_600_000).1, Verdict::Unfinished);
    // The writer is a thread: wait for the lines.
    let wanted = [
        "\"outcome\":\"kept\",\"contacts\":1",
        "\"outcome\":\"rejected_ahead\",\"contacts\":0",
        "\"outcome\":\"rejected_tampered\",\"contacts\":0",
        "\"outcome\":\"rejected_damaged\",\"contacts\":0",
        "\"outcome\":\"none\",\"contacts\":0",
        "\"outcome\":\"rejected_unfinished\",\"contacts\":0",
    ];
    let mut journal = String::new();
    for _ in 0..100 {
        journal = crate::diag::files()
            .iter()
            .filter_map(|f| std::fs::read_to_string(f).ok())
            .collect();
        if wanted.iter().all(|w| journal.contains(w)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    for w in wanted {
        assert!(
            journal.contains(&format!("\"k\":\"backfill\",{w}")),
            "{w}: {journal}"
        );
    }
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// Where a spender stands in the store.
#[derive(Clone, Copy)]
enum Mined {
    Mempool,
    /// In a block without a ChainLock yet: it may be reorganized out.
    Block,
    ChainLocked,
}

fn spender(txid: u8, spends: &[OutPoint], mined: Mined) -> TransactionRecord {
    use key_wallet::dashcore::{BlockHash, Transaction, TxIn};
    use key_wallet::transaction_checking::{BlockInfo, TransactionContext};
    let tx = Transaction {
        version: 2,
        lock_time: 0,
        input: spends
            .iter()
            .map(|o| TxIn {
                previous_output: *o,
                ..TxIn::default()
            })
            .collect(),
        output: Vec::new(),
        special_transaction_payload: None,
    };
    let block = BlockInfo::new(100, BlockHash::from([txid; 32]), 0);
    let context = match mined {
        Mined::Mempool => TransactionContext::Mempool,
        Mined::Block => TransactionContext::InBlock(block),
        Mined::ChainLocked => TransactionContext::InChainLockedBlock(block),
    };
    let mut record = TransactionRecord::new(
        tx,
        key_wallet::account::AccountType::Standard {
            index: 0,
            standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
        },
        context,
        key_wallet::transaction_checking::TransactionType::Standard,
        key_wallet::managed_account::transaction_record::TransactionDirection::Outgoing,
        Vec::new(),
        Vec::new(),
        0,
    );
    record.txid = Txid::from([txid; 32]);
    record
}

/// Workaround (2): only a coin whose spender the store holds in a ChainLocked block is claimed.
/// A send seen in the mempool alone may never be mined: its coin must be able to come back.
#[test]
fn only_coins_spent_in_a_block_are_claimed() {
    let a = OutPoint::new(Txid::from([1; 32]), 0);
    let b = OutPoint::new(Txid::from([2; 32]), 1);
    let c = OutPoint::new(Txid::from([3; 32]), 0);
    let spent = BTreeSet::from([a, b, c]);
    let records = [
        spender(9, &[a], Mined::ChainLocked),
        spender(10, &[b], Mined::Mempool),
    ];
    let claims = claims_of(&spent, &BTreeSet::new(), &records, 0);
    assert_eq!(claims.claimed, vec![(a, Some(Txid::from([9; 32])))]);
    assert_eq!(claims.not_in_block, 2);
}

/// BUG-77 workarounds r1 should 1: a spend in a block without a ChainLock is not claimed. If
/// that block is reorganized out while the store still holds its spender `InBlock`, a claim made
/// at every start would hide the coin until a resync; only a ChainLock makes the spend final.
#[test]
fn a_spend_in_a_block_without_a_chainlock_is_not_claimed() {
    let a = OutPoint::new(Txid::from([1; 32]), 0);
    let b = OutPoint::new(Txid::from([2; 32]), 1);
    let spent = BTreeSet::from([a, b]);
    let records = [
        spender(9, &[a], Mined::Block),
        spender(10, &[b], Mined::ChainLocked),
    ];
    let claims = claims_of(&spent, &BTreeSet::new(), &records, 0);
    assert_eq!(claims.claimed, vec![(b, Some(Txid::from([10; 32])))]);
    assert_eq!(
        claims.not_in_block, 1,
        "the unlocked block's coin is left to the pass"
    );
}

/// The store's outpoint (bincode, 34 bytes) is read as the store writes it; a row that does not
/// decode is left out, and a store that does not open claims nothing.
#[test]
fn spent_coins_are_read_from_the_store_as_it_writes_them() {
    let db = folder("rows");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE core_utxos (wallet_id BLOB, outpoint BLOB, spent INTEGER);")
        .unwrap();
    let spent = OutPoint::new(Txid::from([5; 32]), 300);
    let live = OutPoint::new(Txid::from([6; 32]), 0);
    let blob =
        |o: &OutPoint| bincode::serde::encode_to_vec(o, bincode::config::standard()).unwrap();
    assert_eq!(
        blob(&live).len(),
        34,
        "0x20, the txid, the index as a varint"
    );
    for (o, s, w) in [(&spent, 1, ID), (&live, 0, ID), (&spent, 1, OTHER)] {
        conn.execute(
            "INSERT INTO core_utxos VALUES (?1, ?2, ?3)",
            rusqlite::params![w.as_slice(), blob(o), s],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO core_utxos VALUES (?1, ?2, 1)",
        rusqlite::params![ID.as_slice(), vec![0x20u8, 1, 2]],
    )
    .unwrap();
    drop(conn);
    assert_eq!(spent_outpoints(&db, &ID).unwrap(), BTreeSet::from([spent]));
    assert_eq!(
        spent_claims(&db.with_file_name("none.db"), &ID, 0),
        Claims::default()
    );
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// The customer's question: a coin wrongly claimed as spent can only lower the balance. A claim
/// stops a coin from being credited again; it never adds one, a coin already held stays as it
/// is, and a claim on a coin that is not the wallet's changes nothing.
#[tokio::test]
async fn a_claimed_coin_can_only_lower_the_balance() {
    use key_wallet::dashcore::{BlockHash, Transaction, TxIn, TxOut};
    use key_wallet::transaction_checking::{
        BlockInfo, TransactionContext, WalletTransactionChecker,
    };
    use key_wallet::wallet::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let keys = crate::keys_of(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let to = ManagedWalletInfo::from_wallet(&keys, 0).monitored_addresses()[0].clone();
    let pay = |n: u8, value: u64| Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::from([n; 32]), 0),
            ..TxIn::default()
        }],
        output: vec![TxOut {
            value,
            script_pubkey: to.script_pubkey(),
        }],
        special_transaction_payload: None,
    };
    let held = pay(40, 70_000);
    let again = pay(41, 150_000);
    let block =
        |h: u32| TransactionContext::InBlock(BlockInfo::new(h, BlockHash::from([h as u8; 32]), 0));
    // The claims made once the first coin is held, before the second one comes.
    let run = async |claims: Vec<(OutPoint, Option<Txid>)>| {
        let mut keys = keys.clone();
        let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
        info.check_core_transaction(&held, block(10), &mut keys, true, true)
            .await;
        info.restore_spent_outpoints(&claims);
        info.check_core_transaction(&again, block(20), &mut keys, true, true)
            .await;
        info.update_balance();
        info.balance().total()
    };
    let none = run(Vec::new()).await;
    assert_eq!(none, 220_000);
    let wrong = OutPoint::new(again.txid(), 0);
    let mine = OutPoint::new(held.txid(), 0);
    let foreign = OutPoint::new(Txid::from([99; 32]), 3);
    assert_eq!(run(vec![(wrong, None)]).await, 70_000, "not credited");
    assert_eq!(
        run(vec![(mine, None)]).await,
        none,
        "a held coin stays as it is"
    );
    assert_eq!(run(vec![(foreign, None)]).await, none);
    for claims in [
        vec![(wrong, None), (mine, None), (foreign, None)],
        vec![(wrong, Some(Txid::from([5; 32])))],
    ] {
        assert!(run(claims).await <= none, "never higher");
    }
}

/// Workaround (2) as the app runs it: the load claims the store's spent coins whose spender it
/// holds in a block, before any block reaches the wallet. A rewound pass then brings back the
/// coin spent in a block without crediting it (no 6,80 on screen), while a coin whose spend was
/// seen in the mempool only is credited again: that send may never be mined.
#[tokio::test]
async fn the_load_claims_coins_spent_in_a_block_and_never_a_mempool_send() {
    use key_wallet::dashcore::{BlockHash, Transaction, TxIn, TxOut};
    use key_wallet::transaction_checking::{
        BlockInfo, TransactionContext, WalletTransactionChecker,
    };
    use key_wallet::wallet::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    use platform_wallet::changeset::ClientWalletStartState;

    struct Loads(Mutex<Option<ClientStartState>>);
    impl PlatformWalletPersistence for Loads {
        fn store(&self, _: WalletId, _: PlatformWalletChangeSet) -> Result<(), PersistenceError> {
            Ok(())
        }
        fn flush(&self, _: WalletId) -> Result<(), PersistenceError> {
            Ok(())
        }
        fn load(&self) -> Result<ClientStartState, PersistenceError> {
            Ok(self.0.lock().unwrap().take().unwrap_or_default())
        }
    }

    let keys = crate::keys_of(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let to = ManagedWalletInfo::from_wallet(&keys, 0).monitored_addresses()[0].clone();
    let pay = |n: u8, value: u64| Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::from([n; 32]), 0),
            ..TxIn::default()
        }],
        output: vec![TxOut {
            value,
            script_pubkey: to.script_pubkey(),
        }],
        special_transaction_payload: None,
    };
    let mined = pay(50, 150_000);
    let pending = pay(51, 70_000);
    let mined_coin = OutPoint::new(mined.txid(), 0);
    let pending_coin = OutPoint::new(pending.txid(), 0);

    // The store: both coins spent (`spent = 1`); one spender in a block, one in the mempool.
    let db = folder("load-claims");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE core_utxos (wallet_id BLOB, outpoint BLOB, spent INTEGER);
         CREATE TABLE core_transactions (wallet_id BLOB, record_blob BLOB);",
    )
    .unwrap();
    let config = bincode::config::standard();
    for coin in [mined_coin, pending_coin] {
        conn.execute(
            "INSERT INTO core_utxos VALUES (?1, ?2, 1)",
            rusqlite::params![
                ID.as_slice(),
                bincode::serde::encode_to_vec(coin, config).unwrap()
            ],
        )
        .unwrap();
    }
    for record in [
        spender(60, &[mined_coin], Mined::ChainLocked),
        spender(61, &[pending_coin], Mined::Mempool),
    ] {
        conn.execute(
            "INSERT INTO core_transactions VALUES (?1, ?2)",
            rusqlite::params![
                ID.as_slice(),
                bincode::serde::encode_to_vec(&record, config).unwrap()
            ],
        )
        .unwrap();
    }
    drop(conn);

    let mut start = ClientStartState::default();
    start.wallets.insert(
        ID,
        ClientWalletStartState {
            wallet: keys.clone(),
            wallet_info: ManagedWalletInfo::from_wallet(&keys, 0),
            identity_manager: Default::default(),
            unused_asset_locks: BTreeMap::new(),
            unconfirmed_outgoing_txs: Vec::new(),
            dashpay_backfill: DashPayBackfillRecord::default(),
        },
    );
    let store = Store::new(Loads(Mutex::new(Some(start))), db.clone());
    let mut loaded = store.load().unwrap();
    let wallet = loaded.wallets.remove(&ID).unwrap();
    let (mut keys, mut info) = (wallet.wallet, wallet.wallet_info);
    // The rewound pass: both fundings come again.
    let block =
        |h: u32| TransactionContext::InBlock(BlockInfo::new(h, BlockHash::from([h as u8; 32]), 0));
    info.check_core_transaction(&mined, block(10), &mut keys, true, true)
        .await;
    info.check_core_transaction(&pending, block(11), &mut keys, true, true)
        .await;
    info.update_balance();
    assert_eq!(
        info.balance().total(),
        70_000,
        "the coin spent in a block is not credited again; the mempool send's coin is"
    );
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// BUG-64's rescan lowers the cursor with no record: the record keeps vouching for what it
/// covered (those blocks were scanned with the contacts watched), the app stopped in the middle
/// of the rescan or not. The rescan's own state (`mined.json`, the owed rescan) is not here.
#[test]
fn a_rescan_without_a_record_keeps_the_record() {
    let db = folder("rescan");
    let store = backfilled(&db);
    let rescan = PlatformWalletChangeSet {
        core: Some(CoreChangeSet {
            synced_height: Some(1_550_000),
            synced_height_is_rewind: true,
            ..CoreChangeSet::default()
        }),
        ..PlatformWalletChangeSet::default()
    };
    store.store(ID, rescan).unwrap();
    store.store(ID, scanned(1_570_000)).unwrap();
    let (record, verdict) = open(&db).take_back(&ID, 1_600_000);
    assert_eq!(
        verdict,
        Verdict::Kept,
        "stopped in the middle of the rescan"
    );
    assert!(record.covers(&owner(), &Identifier::from([2; 32]), 0, 1_582_131));
    store.store(ID, scanned(1_600_000)).unwrap();
    assert_eq!(open(&db).take_back(&ID, 1_600_000).1, Verdict::Kept);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// BUG-99, as the store of pass 13 held it: a contact's backfill brought the funding of a coin
/// again, and the store wrote it back unspent. Its spender is in a block under the last ChainLock
/// (`InBlock` for good: a catch-up never tells the record of the lock), so the coin is gone: the
/// load neither counts it nor lets a later pass credit it.
#[tokio::test]
async fn a_coin_the_store_has_unspent_is_gone_when_a_final_block_spent_it() {
    use key_wallet::dashcore::{BlockHash, Transaction, TxIn, TxOut};
    use key_wallet::transaction_checking::{
        BlockInfo, TransactionContext, WalletTransactionChecker,
    };
    use key_wallet::wallet::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    use platform_wallet::changeset::ClientWalletStartState;

    struct Loads(Mutex<Option<ClientStartState>>);
    impl PlatformWalletPersistence for Loads {
        fn store(&self, _: WalletId, _: PlatformWalletChangeSet) -> Result<(), PersistenceError> {
            Ok(())
        }
        fn flush(&self, _: WalletId) -> Result<(), PersistenceError> {
            Ok(())
        }
        fn load(&self) -> Result<ClientStartState, PersistenceError> {
            Ok(self.0.lock().unwrap().take().unwrap_or_default())
        }
    }

    let mut keys = crate::keys_of(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
    let to = info.monitored_addresses()[0].clone();
    let pay = |n: u8, value: u64| Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::from([n; 32]), 0),
            ..TxIn::default()
        }],
        output: vec![TxOut {
            value,
            script_pubkey: to.script_pubkey(),
        }],
        special_transaction_payload: None,
    };
    let block =
        |h: u32| TransactionContext::InBlock(BlockInfo::new(h, BlockHash::from([h as u8; 32]), 0));
    // Three coins the store holds unspent: spent under the ChainLock, spent above it, not spent.
    let gone = pay(70, 18_899_511);
    let recent = pay(71, 70_000);
    let live = pay(72, 5_000);
    for (tx, h) in [(&gone, 10), (&recent, 11), (&live, 12)] {
        info.check_core_transaction(tx, block(h), &mut keys, true, true)
            .await;
    }
    info.update_balance();
    assert_eq!(info.balance().total(), 18_899_511 + 70_000 + 5_000);
    info.metadata.last_applied_chain_lock = Some(key_wallet::dashcore::ChainLock {
        block_height: 100,
        block_hash: BlockHash::from([1; 32]),
        signature: [0; 96].into(),
    });

    let (gone_coin, recent_coin, live_coin) = (
        OutPoint::new(gone.txid(), 0),
        OutPoint::new(recent.txid(), 0),
        OutPoint::new(live.txid(), 0),
    );
    let db = folder("spent-unspent");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE core_utxos (wallet_id BLOB, outpoint BLOB, spent INTEGER);
         CREATE TABLE core_transactions (wallet_id BLOB, record_blob BLOB);",
    )
    .unwrap();
    let config = bincode::config::standard();
    for coin in [gone_coin, recent_coin, live_coin] {
        conn.execute(
            "INSERT INTO core_utxos VALUES (?1, ?2, 0)",
            rusqlite::params![
                ID.as_slice(),
                bincode::serde::encode_to_vec(coin, config).unwrap()
            ],
        )
        .unwrap();
    }
    let mut above = spender(81, &[recent_coin], Mined::Block);
    above.context = block(101);
    for record in [spender(80, &[gone_coin], Mined::Block), above] {
        conn.execute(
            "INSERT INTO core_transactions VALUES (?1, ?2)",
            rusqlite::params![
                ID.as_slice(),
                bincode::serde::encode_to_vec(&record, config).unwrap()
            ],
        )
        .unwrap();
    }
    drop(conn);

    let mut start = ClientStartState::default();
    start.wallets.insert(
        ID,
        ClientWalletStartState {
            wallet: keys.clone(),
            wallet_info: info,
            identity_manager: Default::default(),
            unused_asset_locks: BTreeMap::new(),
            unconfirmed_outgoing_txs: Vec::new(),
            dashpay_backfill: DashPayBackfillRecord::default(),
        },
    );
    let store = Store::new(Loads(Mutex::new(Some(start))), db.clone());
    let mut loaded = store.load().unwrap();
    let wallet = loaded.wallets.remove(&ID).unwrap();
    let (mut keys, mut info) = (wallet.wallet, wallet.wallet_info);
    assert_eq!(
        info.balance().total(),
        70_000 + 5_000,
        "the coin a final block spent is not counted; the others are"
    );
    // The same funding delivered by a later pass does not bring it back.
    info.check_core_transaction(&gone, block(10), &mut keys, true, true)
        .await;
    info.update_balance();
    assert_eq!(info.balance().total(), 70_000 + 5_000);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}
