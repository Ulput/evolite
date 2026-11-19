use super::*;
use platform_wallet::changeset::changeset::UtxoCreditVerdict;
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

fn funding_coin(n: u8) -> (OutPoint, key_wallet::Utxo) {
    use key_wallet::dashcore::TxOut;
    let keys = crate::keys_of(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let info = key_wallet::wallet::ManagedWalletInfo::from_wallet(&keys, 0);
    let to = info.monitored_addresses()[0].clone();
    let outpoint = OutPoint::new(Txid::from([n; 32]), 0);
    let output = TxOut {
        value: 18_899_511,
        script_pubkey: to.script_pubkey(),
    };
    (
        outpoint,
        key_wallet::Utxo::new(outpoint, output, to, 10, false),
    )
}

/// The store as the app has it: [`Store`] over the real SQLite one, on a wallet it knows.
fn real_store(
    tag: &str,
) -> (
    PathBuf,
    Store<platform_wallet_storage::sqlite::SqlitePersister>,
) {
    let db = folder(tag);
    let sqlite = platform_wallet_storage::sqlite::SqlitePersister::open(
        platform_wallet_storage::sqlite::SqlitePersisterConfig::new(db.clone()),
    )
    .unwrap();
    let store = Store::new(sqlite, db.clone());
    store
        .store(
            ID,
            PlatformWalletChangeSet {
                wallet_metadata: Some(platform_wallet::changeset::WalletMetadataEntry {
                    network: Network::Testnet,
                    wallet_group_id: [0; 32],
                    birth_height: 0,
                }),
                ..PlatformWalletChangeSet::default()
            },
        )
        .unwrap();
    (db, store)
}

fn core_round(core: CoreChangeSet) -> PlatformWalletChangeSet {
    PlatformWalletChangeSet {
        core: Some(core),
        ..PlatformWalletChangeSet::default()
    }
}

/// A funding round: the outputs as new, and what the wallet said of those it did not credit.
fn funded(
    coins: &[&key_wallet::Utxo],
    verdicts: &[(OutPoint, UtxoCreditVerdict)],
) -> CoreChangeSet {
    CoreChangeSet {
        new_utxos: coins.iter().map(|c| (*c).clone()).collect(),
        utxo_credit_verdicts: verdicts.iter().copied().collect(),
        ..CoreChangeSet::default()
    }
}

fn spends(coin: &key_wallet::Utxo) -> CoreChangeSet {
    CoreChangeSet {
        spent_utxos: vec![coin.clone()],
        ..CoreChangeSet::default()
    }
}

fn is_spent(db: &Path, coin: OutPoint) -> Option<bool> {
    let spent = outpoints(db, &ID, true).unwrap().contains(&coin);
    let unspent = outpoints(db, &ID, false).unwrap().contains(&coin);
    assert!(!(spent && unspent), "one row a coin");
    (spent || unspent).then_some(spent)
}

/// BUG-99b: a pass brings a coin's funding again after the coin was spent. The wallet did not
/// credit it and says so, whichever the reason; the row stays spent. In the same round a coin
/// the wallet did credit is written as ever.
#[test]
fn a_coin_the_wallet_did_not_credit_is_not_written_back_unspent() {
    let (db, store) = real_store("verdict");
    let verdicts = [
        UtxoCreditVerdict::Uncredited,
        UtxoCreditVerdict::ObservedSpent { height: 90 },
        UtxoCreditVerdict::Doomed,
    ];
    let (fresh, fresh_utxo) = funding_coin(40);
    for (n, verdict) in (30u8..).zip(verdicts) {
        let (coin, utxo) = funding_coin(n);
        store.store(ID, core_round(funded(&[&utxo], &[]))).unwrap();
        store.store(ID, core_round(spends(&utxo))).unwrap();
        assert_eq!(is_spent(&db, coin), Some(true));
        store
            .store(
                ID,
                core_round(funded(&[&utxo, &fresh_utxo], &[(coin, verdict)])),
            )
            .unwrap();
        assert_eq!(is_spent(&db, coin), Some(true), "{verdict:?}");
    }
    assert_eq!(is_spent(&db, fresh), Some(false));
    drop(store);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A coin the wallet never held and did not credit gets no unspent row, and its spend still
/// leaves the row that says so.
#[test]
fn a_coin_never_credited_has_no_row_until_its_spend_comes() {
    let (db, store) = real_store("verdict-new");
    let (coin, utxo) = funding_coin(31);
    store
        .store(
            ID,
            core_round(funded(&[&utxo], &[(coin, UtxoCreditVerdict::Uncredited)])),
        )
        .unwrap();
    assert_eq!(is_spent(&db, coin), None);
    store.store(ID, core_round(spends(&utxo))).unwrap();
    assert_eq!(is_spent(&db, coin), Some(true));
    drop(store);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// Why the guard is the wallet's verdict and not "spent stays spent": a funding the wallet does
/// credit is written unspent over a spent row.
#[test]
fn a_coin_the_wallet_credited_is_written_unspent_over_a_spent_row() {
    let (db, store) = real_store("no-verdict");
    let (coin, utxo) = funding_coin(32);
    store.store(ID, core_round(funded(&[&utxo], &[]))).unwrap();
    store.store(ID, core_round(spends(&utxo))).unwrap();
    assert_eq!(is_spent(&db, coin), Some(true));
    store.store(ID, core_round(funded(&[&utxo], &[]))).unwrap();
    assert_eq!(is_spent(&db, coin), Some(false));
    drop(store);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// platform-wallet folds rounds before the store sees them, and the newer one speaks for the
/// coin: credited after a verdict is credited; a verdict after a credit is a verdict.
#[test]
fn merged_rounds_answer_as_the_newer_one_does() {
    use platform_wallet::changeset::Merge;
    let (db, store) = real_store("merged");
    let (credited, credited_utxo) = funding_coin(33);
    let (held, held_utxo) = funding_coin(34);
    for utxo in [&credited_utxo, &held_utxo] {
        store.store(ID, core_round(funded(&[utxo], &[]))).unwrap();
        store.store(ID, core_round(spends(utxo))).unwrap();
    }

    let mut round = funded(
        &[&credited_utxo],
        &[(credited, UtxoCreditVerdict::Uncredited)],
    );
    round.merge(funded(&[&credited_utxo], &[]));
    let mut other = funded(&[&held_utxo], &[]);
    other.merge(funded(
        &[&held_utxo],
        &[(held, UtxoCreditVerdict::Uncredited)],
    ));
    round.merge(other);
    store.store(ID, core_round(round)).unwrap();

    assert_eq!(is_spent(&db, credited), Some(false), "credited last");
    assert_eq!(is_spent(&db, held), Some(true), "not credited last");
    drop(store);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// The upstream reinstatement a blanket guard would break, through the real sweep: a swept
/// funding comes back ChainLocked and its coin lands unspent, and so does a coin a swept spend
/// held. With a verdict the same held coin stays spent.
#[test]
fn a_swept_funding_that_comes_back_lands_unspent() {
    use key_wallet::dashcore::TxOut;
    use platform_wallet::changeset::changeset::SweepBatch;
    let (db, store) = real_store("sweep");
    let (coin, utxo) = funding_coin(35);
    let mut funding = spender(35, &[], Mined::Block);
    funding.transaction.output = vec![TxOut {
        value: utxo.value(),
        script_pubkey: utxo.txout.script_pubkey.clone(),
    }];
    let mut first = funded(&[&utxo], &[]);
    first.records = vec![funding.clone()];
    store.store(ID, core_round(first)).unwrap();
    assert_eq!(is_spent(&db, coin), Some(false));

    let sweep = |txid: Txid| SweepBatch {
        txids: vec![txid],
        superseded_by: Txid::from([99; 32]),
        winner_mined_height: Some(11),
        released_outpoints: Vec::new(),
    };
    let swept = CoreChangeSet {
        sweeps: vec![sweep(funding.txid)],
        ..CoreChangeSet::default()
    };
    store.store(ID, core_round(swept)).unwrap();
    assert_eq!(is_spent(&db, coin), None, "gone with its funding");

    let mut back = funded(&[&utxo], &[]);
    back.records = vec![funding];
    store.store(ID, core_round(back)).unwrap();
    assert_eq!(is_spent(&db, coin), Some(false), "reinstated");

    // A spend of it swept, the coin not released: held spent, the row has no live spender.
    let loser = spender(36, &[coin], Mined::Mempool);
    let mut spent = spends(&utxo);
    spent.records = vec![loser.clone()];
    store.store(ID, core_round(spent)).unwrap();
    let held = CoreChangeSet {
        sweeps: vec![sweep(loser.txid)],
        ..CoreChangeSet::default()
    };
    store.store(ID, core_round(held)).unwrap();
    assert_eq!(is_spent(&db, coin), Some(true), "held by the sweep");
    store
        .store(
            ID,
            core_round(funded(&[&utxo], &[(coin, UtxoCreditVerdict::Uncredited)])),
        )
        .unwrap();
    assert_eq!(is_spent(&db, coin), Some(true), "not credited: stays");
    store.store(ID, core_round(funded(&[&utxo], &[]))).unwrap();
    assert_eq!(is_spent(&db, coin), Some(false), "credited: comes back");
    drop(store);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// The branch BUG-99 added and only tested with `spent = 0`: a row already spent, spender in a
/// block at or under the last ChainLock, is claimed; one block above it is not.
#[test]
fn a_spent_row_with_its_spender_under_the_chain_lock_is_claimed() {
    let a = OutPoint::new(Txid::from([1; 32]), 0);
    let spent = BTreeSet::from([a]);
    let records = [spender(9, &[a], Mined::Block)];
    let claims = claims_of(&spent, &BTreeSet::new(), &records, 100);
    assert_eq!(claims.claimed, vec![(a, Some(Txid::from([9; 32])))]);
    assert_eq!(claims.not_in_block, 0);
    let claims = claims_of(&spent, &BTreeSet::new(), &records, 99);
    assert!(claims.claimed.is_empty());
    assert_eq!(claims.not_in_block, 1);
}

/// BUG-194 AC-T3а: the pools platform-wallet leaves out are written through our store — change
/// past the first gap window (index 35 here), with its used flag — and an older snapshot does
/// not take a used flag back.
#[test]
fn pools_past_the_first_window_are_written_with_their_used_flags() {
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let (db, store) = real_store("pools");
    let keys = crate::keys_of(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
    let xpub = keys.accounts.standard_bip44_accounts[&0].account_xpub;
    let before = crate::convert::pool_entries(&info);
    let change = {
        let account = info
            .accounts_mut()
            .standard_bip44_accounts
            .get_mut(&0)
            .unwrap();
        let made = account
            .next_change_addresses(Some(&xpub), 40, true)
            .unwrap();
        account
            .keys_mut()
            .managed_account_type_mut()
            .mark_address_used(&made[35]);
        made[35].clone()
    };
    let write = |pools| {
        store.store(
            ID,
            PlatformWalletChangeSet {
                account_address_pools: pools,
                ..Default::default()
            },
        )
    };
    write(crate::convert::pool_entries(&info)).unwrap();
    // An older snapshot (index 35 not used yet, or not there) comes after: the flag stays.
    write(before).unwrap();
    // AC-T3б: a payment with its change there, as one account's view wrote it (the change
    // `Sent`), reads back after a restart with the change ours.
    {
        use key_wallet::dashcore::{Transaction, TxIn, TxOut};
        use key_wallet::managed_account::transaction_record::{
            InputDetail, OutputDetail, OutputRole, TransactionDirection, TransactionRecord,
        };
        use key_wallet::transaction_checking::TransactionContext;
        use key_wallet::transaction_checking::transaction_router::TransactionType;
        let payee = crate::convert::pool_entries(&info)[0].addresses[0]
            .address
            .clone();
        let tx = Transaction {
            version: 2,
            lock_time: 0,
            input: vec![TxIn::default()],
            output: [(&payee, 10_000), (&change, 5_000)]
                .map(|(a, value)| TxOut {
                    value,
                    script_pubkey: a.script_pubkey(),
                })
                .to_vec(),
            special_transaction_payload: None,
        };
        let record = TransactionRecord::new(
            tx,
            key_wallet::account::AccountType::Standard {
                index: 0,
                standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
            },
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Outgoing,
            vec![InputDetail {
                index: 0,
                value: 15_226,
                address: payee.clone(),
            }],
            vec![
                OutputDetail {
                    index: 0,
                    role: OutputRole::Sent,
                    address: Some(payee.clone()),
                    value: 10_000,
                },
                OutputDetail {
                    index: 1,
                    role: OutputRole::Sent,
                    address: Some(change.clone()),
                    value: 5_000,
                },
            ],
            -15_226,
        );
        store
            .store(
                ID,
                PlatformWalletChangeSet {
                    core: Some(CoreChangeSet {
                        records: vec![record],
                        ..CoreChangeSet::default()
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
    }
    drop(store);
    let read = crate::chain::stored_history(&db, &ID).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(
        read[0].output_details[1].role,
        key_wallet::managed_account::transaction_record::OutputRole::Change,
        "the change at index 35 is ours after a restart"
    );
    let conn = rusqlite::Connection::open(&db).unwrap();
    let (index, used): (i64, bool) = conn
        .query_row(
            "SELECT address_index, used FROM core_address_pool WHERE account_type = \
             'standard_bip44' AND pool_type = 1 AND script = ?1",
            rusqlite::params![change.script_pubkey().as_bytes()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((index, used), (35, true));
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// BUG-221 (the architect's p. 2): a payment between two identities of one wallet, recorded
/// by its BIP44 input before the receiving DashPay account was registered; the account comes
/// later (the DashPay sync), and the block is processed again (the contacts' rescan,
/// BUG-150). The coin on the new account must be credited, chain-locked block or not.
#[tokio::test]
async fn a_coin_to_an_account_registered_after_its_tx_is_credited_on_the_rescan() {
    use key_wallet::dashcore::{BlockHash, Transaction, TxIn, TxOut};
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::transaction_checking::{
        BlockInfo, TransactionContext, WalletTransactionChecker,
    };
    use key_wallet::wallet::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let keys = crate::keys_of(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let contact = key_wallet::Account {
        parent_wallet_id: None,
        account_type: key_wallet::account::AccountType::DashpayReceivingFunds {
            index: 0,
            user_identity_id: [1; 32],
            friend_identity_id: [2; 32],
        },
        network: key_wallet::Network::Testnet,
        account_xpub: keys.accounts.coinjoin_accounts[&0].account_xpub,
        is_watch_only: false,
    };
    let dashpay = key_wallet::managed_account::ManagedCoreFundsAccount::from_account(&contact);
    let theirs = dashpay.managed_account_type().all_addresses()[0].clone();
    for chainlocked in [false, true] {
        let mut keys = keys.clone();
        let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
        let own = info.monitored_addresses();
        let block = |h: u32| {
            let at = BlockInfo::new(h, BlockHash::from([h as u8; 32]), 0);
            if chainlocked {
                TransactionContext::InChainLockedBlock(at)
            } else {
                TransactionContext::InBlock(at)
            }
        };
        let funding = Transaction {
            version: 2,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: OutPoint::new(Txid::from([50; 32]), 0),
                ..TxIn::default()
            }],
            output: vec![TxOut {
                value: 1_000_000,
                script_pubkey: own[0].script_pubkey(),
            }],
            special_transaction_payload: None,
        };
        // tx1: from our BIP44 coin, 10 000 to the other identity's DashPay address, change back.
        let between = Transaction {
            version: 2,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: OutPoint::new(funding.txid(), 0),
                ..TxIn::default()
            }],
            output: vec![
                TxOut {
                    value: 10_000,
                    script_pubkey: theirs.script_pubkey(),
                },
                TxOut {
                    value: 980_000,
                    script_pubkey: own[1].script_pubkey(),
                },
            ],
            special_transaction_payload: None,
        };
        info.check_core_transaction(&funding, block(10), &mut keys, true, true)
            .await;
        info.check_core_transaction(&between, block(20), &mut keys, true, true)
            .await;
        info.accounts_mut()
            .insert_funds_bearing_account(dashpay.clone())
            .unwrap();
        // The rescan: the block of tx1 again.
        info.check_core_transaction(&between, block(20), &mut keys, true, true)
            .await;
        info.update_balance();
        let on_dashpay: u64 = info
            .accounts()
            .dashpay_receival_accounts
            .values()
            .flat_map(|a| a.utxos.values())
            .map(|u| u.value())
            .sum();
        assert_eq!(
            (on_dashpay, info.balance().total()),
            (10_000, 990_000),
            "chainlocked {chainlocked}"
        );
    }
}

/// BUG-232 review r1 M1, qa-f29's tx1 end to end: our coin spent to another wallet with the
/// change 3 past the change pool, both chain-locked, so the change was read as paid out. The
/// change is taken in, the rescan reads tx1's block again, and key-wallet credits its coin
/// (vendor `evolite-finalized-outputs.patch`); [`crate::convert::coins_of_sent_outputs`] finds it,
/// and the store keeps it (`Chain::save_coins`'s changeset), so a restart has it.
#[tokio::test]
async fn a_change_taken_in_past_the_gap_gets_its_coin_on_the_rescan_and_the_store_keeps_it() {
    use key_wallet::dashcore::{BlockHash, Transaction, TxIn, TxOut};
    use key_wallet::managed_account::address_pool::{AddressPoolType, KeySource};
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::transaction_checking::{
        BlockInfo, TransactionContext, WalletTransactionChecker,
    };
    use key_wallet::wallet::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    const CHANGE: u64 = 980_000;
    let mut keys = crate::keys_of(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
    let other = ManagedWalletInfo::from_wallet(
        &crate::keys_of(
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
        )
        .unwrap(),
        0,
    )
    .monitored_addresses()[0]
        .clone();
    let own = info.monitored_addresses()[0].clone();
    let change = {
        let mut pool = info.accounts.standard_bip44_accounts[&0]
            .keys()
            .managed_account_type()
            .address_pools()
            .into_iter()
            .find(|p| p.pool_type == AddressPoolType::Internal)
            .unwrap()
            .clone();
        let index = pool.highest_generated.unwrap() + 3;
        pool.gap_limit = index + 1;
        pool.maintain_gap_limit(&KeySource::Public(
            keys.accounts.standard_bip44_accounts[&0].account_xpub,
        ))
        .unwrap();
        pool.address_at_index(index).unwrap()
    };
    let chain_locked = |h: u32| {
        TransactionContext::InChainLockedBlock(BlockInfo::new(h, BlockHash::from([h as u8; 32]), 0))
    };
    let pay = |from: OutPoint, to: Vec<(&key_wallet::dashcore::Address, u64)>| Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: from,
            ..TxIn::default()
        }],
        output: to
            .into_iter()
            .map(|(a, value)| TxOut {
                value,
                script_pubkey: a.script_pubkey(),
            })
            .collect(),
        special_transaction_payload: None,
    };
    let funding = pay(
        OutPoint::new(Txid::from([50; 32]), 0),
        vec![(&own, 1_000_000)],
    );
    let tx1 = pay(
        OutPoint::new(funding.txid(), 0),
        vec![(&other, 10_000), (&change, CHANGE)],
    );
    info.check_core_transaction(&funding, chain_locked(10), &mut keys, true, true)
        .await;
    info.check_core_transaction(&tx1, chain_locked(20), &mut keys, true, true)
        .await;
    assert_eq!(info.balance().total(), 0, "the change read as paid out");
    assert_eq!(
        crate::convert::adopt_addresses_past_the_gap(
            &mut info,
            &keys,
            std::slice::from_ref(&change)
        ),
        vec![change.clone()]
    );
    let coin = OutPoint::new(tx1.txid(), 1);
    // Before its rescan (and after a close then, the pool as the store has it, the address
    // used): its rescan is owed, from tx1's block.
    assert_eq!(
        crate::convert::uncredited_outputs(&info, &[]),
        vec![(coin, 20)]
    );
    assert!(crate::convert::coins_of_sent_outputs(&info, &[]).is_empty());
    // The rescan: tx1's block again.
    info.check_core_transaction(&tx1, chain_locked(20), &mut keys, true, true)
        .await;
    assert_eq!(info.balance().total(), CHANGE, "the change's coin");
    assert!(
        crate::convert::uncredited_outputs(&info, &[]).is_empty(),
        "nothing owed"
    );
    let coins = crate::convert::coins_of_sent_outputs(&info, &[]);
    assert_eq!(
        coins.iter().map(|c| c.outpoint).collect::<Vec<_>>(),
        vec![coin]
    );
    let (db, store) = real_store("past-the-gap");
    store
        .store(
            ID,
            core_round(CoreChangeSet {
                new_utxos: coins,
                ..CoreChangeSet::default()
            }),
        )
        .unwrap();
    assert_eq!(is_spent(&db, coin), Some(false), "kept, unspent");
    drop(store);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// The twin: a spend of that change was read (it pays our receiving address), so no rescan is
/// owed for it and no coin is made.
#[tokio::test]
async fn a_change_spent_by_a_record_we_have_is_owed_nothing() {
    use key_wallet::dashcore::{BlockHash, Transaction, TxIn, TxOut};
    use key_wallet::managed_account::address_pool::{AddressPoolType, KeySource};
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::transaction_checking::{
        BlockInfo, TransactionContext, WalletTransactionChecker,
    };
    use key_wallet::wallet::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let mut keys = crate::keys_of(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
    let other = ManagedWalletInfo::from_wallet(
        &crate::keys_of(
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
        )
        .unwrap(),
        0,
    )
    .monitored_addresses()[0]
        .clone();
    let own = info.monitored_addresses()[0].clone();
    let change = {
        let mut pool = info.accounts.standard_bip44_accounts[&0]
            .keys()
            .managed_account_type()
            .address_pools()
            .into_iter()
            .find(|p| p.pool_type == AddressPoolType::Internal)
            .unwrap()
            .clone();
        let index = pool.highest_generated.unwrap() + 3;
        pool.gap_limit = index + 1;
        pool.maintain_gap_limit(&KeySource::Public(
            keys.accounts.standard_bip44_accounts[&0].account_xpub,
        ))
        .unwrap();
        pool.address_at_index(index).unwrap()
    };
    let chain_locked = |h: u32| {
        TransactionContext::InChainLockedBlock(BlockInfo::new(h, BlockHash::from([h as u8; 32]), 0))
    };
    let pay = |from: OutPoint, to: Vec<(&key_wallet::dashcore::Address, u64)>| Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: from,
            ..TxIn::default()
        }],
        output: to
            .into_iter()
            .map(|(a, value)| TxOut {
                value,
                script_pubkey: a.script_pubkey(),
            })
            .collect(),
        special_transaction_payload: None,
    };
    let funding = pay(
        OutPoint::new(Txid::from([50; 32]), 0),
        vec![(&own, 1_000_000)],
    );
    let tx1 = pay(
        OutPoint::new(funding.txid(), 0),
        vec![(&other, 10_000), (&change, 980_000)],
    );
    let tx2 = pay(OutPoint::new(tx1.txid(), 1), vec![(&own, 979_700)]);
    for (tx, h) in [(&funding, 10), (&tx1, 20), (&tx2, 30)] {
        info.check_core_transaction(tx, chain_locked(h), &mut keys, true, true)
            .await;
    }
    crate::convert::adopt_addresses_past_the_gap(&mut info, &keys, std::slice::from_ref(&change));
    assert!(crate::convert::uncredited_outputs(&info, &[]).is_empty());
    info.check_core_transaction(&tx1, chain_locked(20), &mut keys, true, true)
        .await;
    assert_eq!(info.balance().total(), 979_700);
    assert!(crate::convert::coins_of_sent_outputs(&info, &[]).is_empty());
}

/// BUG-237: the row the store keeps for a transaction, read back from the SQLite store.
fn stored_row(
    store: &Store<platform_wallet_storage::sqlite::SqlitePersister>,
    txid: Txid,
) -> TransactionRecord {
    store
        .get_core_tx_record(ID, &txid)
        .unwrap()
        .expect("a row for the transaction")
}

fn records_round(records: Vec<TransactionRecord>) -> PlatformWalletChangeSet {
    core_round(CoreChangeSet {
        records,
        ..CoreChangeSet::default()
    })
}

/// BUG-237: the spending account's slice, then a rescan's DashPay slice in a later round — the
/// row keeps the change (it was replaced by the DashPay slice alone: «Sent −0,1452581»).
#[test]
fn a_later_slice_does_not_write_over_the_stored_one() {
    use crate::convert::merge_slices_tests::{qa_f31, shape};
    let (_db, store) = real_store("slices-later");
    let (bip44, dashpay) = qa_f31();
    store.store(ID, records_round(vec![bip44.clone()])).unwrap();
    store
        .store(ID, records_round(vec![dashpay.clone()]))
        .unwrap();
    let row = stored_row(&store, bip44.txid);
    assert_eq!(
        shape(&row),
        shape(&crate::convert::merge_slices(&dashpay, &bip44))
    );
    assert_eq!(row.output_details.len(), 2, "the change is kept");
    // The other way round too: the DashPay slice first, the spending account's after.
    let (_db, store) = real_store("slices-later-2");
    store
        .store(ID, records_round(vec![dashpay.clone()]))
        .unwrap();
    store.store(ID, records_round(vec![bip44.clone()])).unwrap();
    assert_eq!(stored_row(&store, bip44.txid).output_details.len(), 2);
}

/// BUG-237 (the architect's condition 1): two slices of one transaction in ONE round give one
/// row with both — the second does not write over the first in the same write.
#[test]
fn two_slices_in_one_round_are_one_row() {
    use crate::convert::merge_slices_tests::qa_f31;
    let (_db, store) = real_store("slices-one-round");
    let (bip44, dashpay) = qa_f31();
    store
        .store(ID, records_round(vec![bip44.clone(), dashpay.clone()]))
        .unwrap();
    let row = stored_row(&store, bip44.txid);
    assert_eq!(row.output_details.len(), 2);
    assert_eq!(row.input_details.len(), 1);
    let (_db, store) = real_store("slices-one-round-2");
    store
        .store(ID, records_round(vec![dashpay, bip44.clone()]))
        .unwrap();
    assert_eq!(stored_row(&store, bip44.txid).output_details.len(), 2);
}

/// BUG-237: after a restart the merged row reads back whole — History's record of it is the
/// same, and the store's coins (where the balances come from) are not touched by the merge.
#[test]
fn a_merged_row_reads_back_the_same_after_a_restart() {
    use crate::convert::merge_slices_tests::{qa_f31, shape};
    let (db, store) = real_store("slices-restart");
    let (bip44, dashpay) = qa_f31();
    let (coin, utxo) = funding_coin(50);
    store.store(ID, core_round(funded(&[&utxo], &[]))).unwrap();
    store.store(ID, records_round(vec![bip44.clone()])).unwrap();
    store.store(ID, records_round(vec![dashpay])).unwrap();
    let before = shape(&stored_row(&store, bip44.txid));
    let coins_before = (is_spent(&db, coin), outpoints(&db, &ID, false).unwrap());
    drop(store);
    let sqlite = platform_wallet_storage::sqlite::SqlitePersister::open(
        platform_wallet_storage::sqlite::SqlitePersisterConfig::new(db.clone()),
    )
    .unwrap();
    // Opened again as at a restart. (The full `load` needs a wallet with its accounts in the
    // store, which these fixtures do not make: the row, History's read and the coins are what
    // it would take.)
    let store = Store::new(sqlite, db.clone());
    assert_eq!(shape(&stored_row(&store, bip44.txid)), before);
    let history = crate::chain::stored_history(&db, &ID).unwrap();
    let read = history
        .iter()
        .find(|r| r.txid == bip44.txid)
        .expect("History reads the row");
    assert_eq!(read.output_details.len(), 2);
    assert_eq!(
        (is_spent(&db, coin), outpoints(&db, &ID, false).unwrap()),
        coins_before
    );
}

/// BUG-242 (platform #5716): an asset lock's transaction as the wallet records it when it is sent,
/// and as it comes back after a resume — its record re-emitted with its block and no output
/// details (the funding transaction re-inserted without the account's classification). `vout 0`
/// is the credit burn, `vout 1` our change.
fn asset_lock_record(
    context: key_wallet::transaction_checking::TransactionContext,
    change: &key_wallet::Utxo,
    with_details: bool,
) -> TransactionRecord {
    use key_wallet::dashcore::{ScriptBuf, Transaction, TxIn, TxOut};
    use key_wallet::managed_account::transaction_record::{
        OutputDetail, OutputRole, TransactionDirection,
    };
    let tx = Transaction {
        version: 3,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::from([41; 32]), 0),
            ..TxIn::default()
        }],
        output: vec![
            TxOut {
                value: 400_000,
                script_pubkey: ScriptBuf::new_op_return(&[0u8; 0]),
            },
            change.txout.clone(),
        ],
        special_transaction_payload: None,
    };
    let details = with_details
        .then(|| OutputDetail {
            index: 1,
            role: OutputRole::Change,
            address: Some(change.address.clone()),
            value: change.value(),
        })
        .into_iter()
        .collect();
    TransactionRecord::new(
        tx,
        key_wallet::account::AccountType::Standard {
            index: 0,
            standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
        },
        context,
        key_wallet::transaction_checking::TransactionType::AssetLock,
        TransactionDirection::Internal,
        Vec::new(),
        details,
        -400_000,
    )
}

/// The height the store keeps for `txid`'s transaction: what a coin of it loads confirmed by
/// (platform-wallet-storage `load_state`: `is_confirmed = height.is_some()`).
fn stored_height(db: &Path, txid: Txid) -> Option<u32> {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.query_row(
        "SELECT height FROM core_transactions WHERE wallet_id = ?1 AND txid = ?2",
        rusqlite::params![ID.as_slice(), AsRef::<[u8]>::as_ref(&txid)],
        |row| row.get::<_, Option<i64>>(0),
    )
    .unwrap()
    .map(|h| h as u32)
}

/// BUG-242 test 1: the change of an asset lock sent before a close is in the store unconfirmed;
/// after the resume its record comes back in a block with no output details (#5716's case). Our
/// SQLite store takes a coin's confirmation from its transaction's height, not from the record's
/// details: the change loads confirmed at that height, and is spent as final.
#[test]
fn a_resumed_asset_locks_change_is_confirmed_by_its_record_in_a_block() {
    use key_wallet::dashcore::BlockHash;
    use key_wallet::transaction_checking::{BlockInfo, TransactionContext};
    const H: u32 = 1_234;
    let (db, store) = real_store("resumed-lock");
    // The wallet's BIP44 account and its addresses, so that a restart rebuilds it with its coins.
    {
        use key_wallet::account::{AccountType, StandardAccountType};
        let keys = crate::keys_of(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let info = key_wallet::wallet::ManagedWalletInfo::from_wallet(&keys, 0);
        store
            .store(
                ID,
                PlatformWalletChangeSet {
                    account_registrations: vec![
                        platform_wallet::changeset::AccountRegistrationEntry {
                            account_type: AccountType::Standard {
                                index: 0,
                                standard_account_type: StandardAccountType::BIP44Account,
                            },
                            account_xpub: keys.accounts.standard_bip44_accounts[&0].account_xpub,
                        },
                    ],
                    account_address_pools: crate::convert::pool_entries(&info),
                    ..PlatformWalletChangeSet::default()
                },
            )
            .unwrap();
    }
    let (_, funding) = funding_coin(41);
    let sent = asset_lock_record(TransactionContext::Mempool, &funding, true);
    let txid = sent.txid;
    let change = key_wallet::Utxo::new(
        OutPoint::new(txid, 1),
        sent.transaction.output[1].clone(),
        funding.address.clone(),
        0,
        false,
    );
    assert!(!change.is_confirmed);
    // Sent: the record and its change, unconfirmed — then the close.
    let mut first = funded(&[&change], &[]);
    first.records = vec![sent];
    store.store(ID, core_round(first)).unwrap();
    assert_eq!(stored_height(&db, txid), None, "unconfirmed");
    assert_eq!(is_spent(&db, change.outpoint), Some(false));

    // The resume: its block comes, the record re-emitted as it stands — no output details.
    let block = BlockInfo::new(H, BlockHash::from([12; 32]), 0);
    let back = asset_lock_record(TransactionContext::InBlock(block), &change, false);
    let round = CoreChangeSet {
        records: vec![back],
        ..CoreChangeSet::default()
    };
    store.store(ID, core_round(round)).unwrap();
    drop(store);

    assert_eq!(stored_height(&db, txid), Some(H), "confirmed at its block");
    assert_eq!(is_spent(&db, change.outpoint), Some(false), "still ours");
    // Read back as the app does at its next start.
    let sqlite = platform_wallet_storage::sqlite::SqlitePersister::open(
        platform_wallet_storage::sqlite::SqlitePersisterConfig::new(db.clone()),
    )
    .unwrap();
    let loaded = sqlite.load().unwrap();
    let coin = loaded.wallets.get(&ID).and_then(|w| {
        w.wallet_info
            .accounts
            .all_funding_accounts()
            .into_iter()
            .find_map(|a| a.utxos.get(&change.outpoint).cloned())
    });
    let coin = coin.expect("the change after a restart");
    assert!(coin.is_confirmed && coin.height == H, "{coin:?}");
    assert!(
        crate::convert::coin_is_final(&coin, |_| false),
        "spent as final"
    );
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// BUG-242 test 2 (the architect's): the app closed before the round with the change reached the
/// store — #5716's window between the send and the InstantLock. The store then has no record of
/// the asset lock and no coin. Our store loads no records into the wallet, so after the resume the
/// transaction comes as new: key-wallet classifies it, credits the change, and the record it emits
/// lists the change (what `derive_new_utxos` makes the coin from), which the store keeps confirmed.
#[tokio::test]
async fn an_asset_locks_change_never_stored_is_credited_when_its_block_comes() {
    use key_wallet::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
    use key_wallet::dashcore::blockdata::transaction::special_transaction::asset_lock::AssetLockPayload;
    use key_wallet::dashcore::{BlockHash, ScriptBuf, Transaction, TxIn, TxOut};
    use key_wallet::managed_account::address_pool::AddressPoolType;
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::managed_account::transaction_record::OutputRole;
    use key_wallet::transaction_checking::{
        BlockInfo, TransactionContext, WalletTransactionChecker,
    };
    use key_wallet::wallet::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    const H: u32 = 2_000;
    const CHANGE: u64 = 580_000;
    let mut keys = crate::keys_of(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
    let own = info.monitored_addresses()[0].clone();
    let change = info.accounts.standard_bip44_accounts[&0]
        .keys()
        .managed_account_type()
        .address_pools()
        .into_iter()
        .find(|p| p.pool_type == AddressPoolType::Internal)
        .unwrap()
        .address_at_index(0)
        .unwrap();
    let block =
        |h: u32| TransactionContext::InBlock(BlockInfo::new(h, BlockHash::from([h as u8; 32]), 0));
    let funding = Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::from([60; 32]), 0),
            ..TxIn::default()
        }],
        output: vec![TxOut {
            value: 1_000_000,
            script_pubkey: own.script_pubkey(),
        }],
        special_transaction_payload: None,
    };
    info.check_core_transaction(&funding, block(10), &mut keys, true, true)
        .await;
    // The asset lock: the credit burned, the change ours. Sent, then the close before its round
    // reached the store; after the resume its block comes.
    let lock = Transaction {
        version: 3,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: OutPoint::new(funding.txid(), 0),
            ..TxIn::default()
        }],
        output: vec![
            TxOut {
                value: 400_000,
                script_pubkey: ScriptBuf::new_op_return(&[0u8; 0]),
            },
            TxOut {
                value: CHANGE,
                script_pubkey: change.script_pubkey(),
            },
        ],
        special_transaction_payload: Some(TransactionPayload::AssetLockPayloadType(
            AssetLockPayload {
                version: 1,
                credit_outputs: vec![TxOut {
                    value: 400_000,
                    script_pubkey: own.script_pubkey(),
                }],
            },
        )),
    };
    info.check_core_transaction(&lock, block(H), &mut keys, true, true)
        .await;
    let coin = OutPoint::new(lock.txid(), 1);
    let account = &info.accounts.standard_bip44_accounts[&0];
    let utxo = account
        .utxos
        .get(&coin)
        .cloned()
        .expect("the change credited");
    assert!(utxo.is_confirmed && utxo.height == H, "{utxo:?}");
    let record = account
        .transactions()
        .get(&lock.txid())
        .cloned()
        .expect("the lock's record");
    assert!(
        record
            .output_details
            .iter()
            .any(|d| d.index == 1 && matches!(d.role, OutputRole::Change | OutputRole::Received)),
        "the record lists the change: {:?}",
        record.output_details
    );

    // Stored as platform-wallet emits it, and read back at the next start: confirmed.
    let (db, store) = real_store("lock-never-stored");
    let mut round = funded(&[&utxo], &[]);
    round.records = vec![record];
    store.store(ID, core_round(round)).unwrap();
    drop(store);
    assert_eq!(stored_height(&db, lock.txid()), Some(H));
    assert_eq!(is_spent(&db, coin), Some(false));
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}
