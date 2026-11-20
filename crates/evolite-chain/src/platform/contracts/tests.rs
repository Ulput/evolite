use super::*;
use crate::platform::tokens::info::test_contract;

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "evolite-contracts-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("contracts").join("testnet").join("contracts.db")
}

fn contract(n: u8, decimals: u8) -> Arc<DataContract> {
    Arc::new(test_contract(
        [n; 32],
        [9; 32],
        &[("coffee", decimals), ("tea", 0)],
    ))
}

/// A contract put in is read back, by this cache (proved in this run) and by another opened on
/// the same file later — the next start — as the same version, not proved.
#[test]
fn a_contract_kept_reads_back_and_is_not_proved_after_a_restart() {
    let path = folder("roundtrip");
    let cache = ContractCache::new(path.clone());
    let kept = cache.put(contract(1, 2), 1_000).unwrap();
    assert!(kept.proved);
    let id = Identifier::from([1; 32]);
    let here = cache.get(&id).expect("kept");
    assert!(here.proved, "proved in this run");
    assert_eq!(here.version, kept.version);
    let next = ContractCache::new(path.clone());
    let read = next.get(&id).expect("on the disk");
    assert!(!read.proved, "the disk is not a proof");
    assert_eq!(read.version, kept.version);
    assert_eq!(read.checked_at, 1_000);
    assert_eq!(next.counts().0, 1);
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap().parent().unwrap());
}

/// Review condition 1: bytes rewritten on the disk under the same version (the decimals) are
/// read — and say they are not proved, so a write takes the network's.
#[test]
fn bytes_changed_on_the_disk_are_not_proved() {
    let path = folder("tampered");
    let cache = ContractCache::new(path.clone());
    cache.put(contract(2, 2), 1_000).unwrap();
    cache.close();
    let forged = crate::platform::tokens::info::contract_bytes(&contract(2, 8)).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE contract SET bytes = ?1 WHERE id = ?2",
        params![forged, Identifier::from([2; 32]).as_slice()],
    )
    .unwrap();
    drop(conn);
    let next = ContractCache::new(path.clone());
    let read = next.get(&Identifier::from([2; 32])).unwrap();
    assert!(!read.proved);
    assert!(
        decide(Some(&read), 1_000, 600, true) == Next::Fetch,
        "a write fetches it whatever the version"
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap().parent().unwrap());
}

/// Review condition 1: a token row moved on the disk to another contract or position gives no
/// configuration: the contract's own token id at that position decides.
#[test]
fn a_token_row_changed_on_the_disk_gives_no_settings() {
    let path = folder("position");
    let cache = ContractCache::new(path.clone());
    let a = cache.put(contract(3, 2), 1_000).unwrap();
    cache.put(contract(4, 6), 1_000).unwrap();
    let token = a.contract.token_id(0).unwrap();
    assert!(cache.token_configuration(&token).is_some());
    assert_eq!(
        cache.token_configuration(&token),
        a.token_configuration(&token),
        "the kept contract's own"
    );
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE token SET contract_id = ?1 WHERE token_id = ?2",
        params![Identifier::from([4; 32]).as_slice(), token.as_slice()],
    )
    .unwrap();
    drop(conn);
    assert!(
        cache.token_configuration(&token).is_none(),
        "not the other contract's token"
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap().parent().unwrap());
}

/// A file that is not a database is set aside with what it held, and a new one is made.
#[test]
fn a_file_that_does_not_open_is_set_aside() {
    let path = folder("broken");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        b"not sqlite, and long enough to be read as a header ...........",
    )
    .unwrap();
    let cache = ContractCache::new(path.clone());
    cache.put(contract(5, 0), 1_000).unwrap();
    let aside: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("contracts.db.unreadable-"))
        .collect();
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert!(cache.get(&Identifier::from([5; 32])).is_some());
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap().parent().unwrap());
}

/// Review condition 2: a write under way (its transaction open) does not hold the provider's
/// read: the read answers while the transaction is still open, with the last committed row.
/// Not timed by the wall clock (batch 50: 500 ms ran out under two full checks at once): a
/// read that waited for the writer's lock would give up after [`BUSY`] with nothing, and one
/// on the writer's connection would see the uncommitted row.
#[test]
fn a_write_under_way_does_not_hold_the_provider() {
    let path = folder("busy");
    let cache = ContractCache::new(path.clone());
    cache.put(contract(6, 0), 1_000).unwrap();
    let id = Identifier::from([6; 32]);
    // Out of memory, so the read goes to the file.
    cache.decoded.lock().unwrap().map.clear();
    let mut writer = cache.writer.lock().unwrap();
    let conn = cache.connection(&mut writer).unwrap();
    let tx = conn.transaction().unwrap();
    tx.execute(
        "UPDATE contract SET checked_at = 5 WHERE id = ?1",
        params![id.as_slice()],
    )
    .unwrap();
    // The transaction is open on this thread until `drop(tx)` below.
    let read = cache
        .try_get(&id)
        .expect("answered while the write is under way");
    assert_eq!(
        read.checked_at, 1_000,
        "the last committed row, not the write's"
    );
    drop(tx);
    drop(writer);
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap().parent().unwrap());
}

/// Review condition 4: at most [`DECODED`] contracts are kept decoded.
#[test]
fn the_decoded_contracts_are_bounded() {
    let mut d = Decoded::default();
    let c = Cached {
        version: 1,
        contract: contract(7, 0),
        checked_at: 0,
        proved: true,
        version_checked: true,
    };
    for n in 0..(DECODED + 40) {
        let mut id = [0u8; 32];
        id[..8].copy_from_slice(&(n as u64).to_le_bytes());
        d.put(Identifier::from(id), c.clone());
    }
    assert_eq!(d.map.len(), DECODED);
    assert_eq!(d.order.len(), DECODED);
}

/// The file beside the network's SPV folder.
#[test]
fn the_file_is_per_network_beside_spv() {
    let spv = Path::new("data").join("spv").join("testnet");
    assert_eq!(
        path_beside_spv(&spv),
        Path::new("data")
            .join("contracts")
            .join("testnet")
            .join("contracts.db")
    );
}

/// What to do before using a contract (review condition 1, design «fresh»).
#[test]
fn the_rule_before_use() {
    let c = Cached {
        version: 3,
        contract: contract(8, 0),
        checked_at: 1_000,
        proved: true,
        version_checked: true,
    };
    assert!(decide(None, 1_000, 60, false) == Next::Fetch);
    assert!(decide(Some(&c), 1_030, 60, false) == Next::Use);
    assert!(decide(Some(&c), 1_061, 60, false) == Next::CheckVersion);
    assert!(
        decide(Some(&c), 1_000, 0, false) == Next::CheckVersion,
        "0: always checked"
    );
    let disk = Cached {
        proved: false,
        ..c.clone()
    };
    assert!(
        decide(Some(&disk), 1_000, 60, false) == Next::Use,
        "a read may use the disk"
    );
    assert!(
        decide(Some(&disk), 1_000, 60, true) == Next::Fetch,
        "a write may not"
    );
    assert!(
        decide(Some(&c), 1_000, 0, true) == Next::CheckVersion,
        "proved this run: the version"
    );
}

/// Review condition 10: the pass row and its page's contracts are one transaction — a page that
/// fails to be kept leaves the pass where it was, so a cut never moves it past a page not kept.
#[test]
fn a_page_and_its_pass_are_kept_together_or_not_at_all() {
    let path = folder("pass");
    let cache = ContractCache::new(path.clone());
    cache
        .put_with_pass(
            vec![contract(10, 0)],
            1_000,
            Some(("token_search", b"page-1")),
        )
        .unwrap();
    assert_eq!(cache.pass("token_search").as_deref(), Some(&b"page-1"[..]));
    // The next page's second contract cannot be written: neither it, nor the first, nor the
    // pass moves.
    {
        let mut writer = cache.writer.lock().unwrap();
        cache
            .connection(&mut writer)
            .unwrap()
            .execute_batch(&format!(
                "CREATE TRIGGER refuse BEFORE INSERT ON contract WHEN NEW.id = X'{}'
                 BEGIN SELECT RAISE(ABORT, 'cut'); END;",
                "0c".repeat(32)
            ))
            .unwrap();
    }
    assert!(
        cache
            .put_with_pass(
                vec![contract(11, 0), contract(12, 0)],
                1_001,
                Some(("token_search", b"page-2")),
            )
            .is_err()
    );
    assert_eq!(cache.pass("token_search").as_deref(), Some(&b"page-1"[..]));
    let next = ContractCache::new(path.clone());
    assert!(
        next.get(&Identifier::from([11; 32])).is_none(),
        "not half a page"
    );
    assert!(next.get(&Identifier::from([10; 32])).is_some());
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap().parent().unwrap());
}
