use super::*;

fn seed(n: u8) -> Vec<u8> {
    (0..64).map(|i| i ^ n).collect()
}

fn note(text: &str, updated: i64) -> Note {
    Note {
        text: text.into(),
        rate: None,
        transfer: false,
        updated,
        deleted: false,
    }
}

fn gone(updated: i64) -> Note {
    Note {
        text: String::new(),
        rate: None,
        transfer: false,
        updated,
        deleted: true,
    }
}

fn entry(address: &str, label: &str) -> Entry {
    Entry {
        address: address.into(),
        label: label.into(),
    }
}

fn app(contract: &str, identity: &str, last_used: i64) -> App {
    App {
        contract: contract.into(),
        name: "@shop".into(),
        identity: identity.into(),
        generation: 1,
        keys: Some((3, 4)),
        last_used,
        session: None,
    }
}

fn full() -> Contents {
    Contents {
        notes: [("core:aa".to_owned(), note("Rent for May", 100))].into(),
        address_book: vec![entry("yExchange", "Exchange")],
        contact_names: [(
            "Id1".to_owned(),
            Kept {
                name: Some("alice".into()),
                at: 5,
            },
        )]
        .into(),
        hidden_requests: ["Spam1".to_owned()].into(),
        connected_apps: vec![app("C1", "Id1", 7)],
        settings: Some(serde_json::json!({"fee_rate": 2000})),
        nft: Some(serde_json::json!({})),
        ..Contents::default()
    }
}

/// AC-T-B1: what is sealed opens with the same seed on the same network, every section as it
/// was.
#[test]
fn a_file_opens_with_its_own_wallet() {
    let key = BackupKey::of(&seed(1));
    let bytes = seal(&key, "testnet", &full()).unwrap();
    let back = open(&key, "testnet", &bytes).expect("opens");
    assert_eq!(back.notes["core:aa"].text, "Rent for May");
    assert_eq!(back.address_book, full().address_book);
    assert_eq!(back.contact_names, full().contact_names);
    assert_eq!(back.hidden_requests, full().hidden_requests);
    assert_eq!(back.connected_apps, full().connected_apps);
    assert_eq!(back.settings, full().settings);
    let header = header(&bytes).expect("the open part");
    assert_eq!(
        header,
        Header {
            version: VERSION,
            network: "testnet".into(),
            wallet: key.fingerprint(),
        }
    );
    assert_eq!(key.fingerprint().len(), 16, "8 bytes");
}

/// AC-T-B1, AC-A3, AC-A7: another wallet — another phrase, or the same phrase with another
/// BIP-39 password, which is another seed — is told by the open part, before decrypting;
/// another network by name. Mutant: the fingerprint not checked → «damaged».
#[test]
fn another_wallet_or_network_is_refused_by_name() {
    let bytes = seal(&BackupKey::of(&seed(1)), "testnet", &full()).unwrap();
    assert_eq!(
        open(&BackupKey::of(&seed(2)), "testnet", &bytes).err(),
        Some(OpenError::OtherWallet)
    );
    assert_eq!(
        open(&BackupKey::of(&seed(1)), "mainnet", &bytes).err(),
        Some(OpenError::OtherNetwork("testnet".into()))
    );
}

/// AC-T-B1, AC-A3: a file cut short, a byte changed (in the secret part or in the open one),
/// or no backup at all — «damaged».
#[test]
fn a_damaged_file_is_refused() {
    let key = BackupKey::of(&seed(1));
    let bytes = seal(&key, "testnet", &full()).unwrap();
    let damaged = |bytes: &[u8]| open(&key, "testnet", bytes).err();
    assert_eq!(
        damaged(&bytes[..bytes.len() - 10]),
        Some(OpenError::Damaged)
    );
    // A hex digit of the ciphertext changed.
    let text = String::from_utf8(bytes.clone()).unwrap();
    let at = text.find("\"ciphertext\":\"").unwrap() + 20;
    let mut changed = text.clone().into_bytes();
    changed[at] = if changed[at] == b'0' { b'1' } else { b'0' };
    assert_eq!(damaged(&changed), Some(OpenError::Damaged));
    // The open part's nonce changed: the tag does not hold either.
    let at = text.find("\"nonce\":\"").unwrap() + 9;
    let mut changed = text.into_bytes();
    changed[at] = if changed[at] == b'0' { b'1' } else { b'0' };
    assert_eq!(damaged(&changed), Some(OpenError::Damaged));
    assert_eq!(
        damaged(b"{\"format\":\"evolite/notes/v1\"}"),
        Some(OpenError::Damaged)
    );
    assert_eq!(damaged(b"not json"), Some(OpenError::Damaged));
}

/// AC-A10 (R-11): a file of a newer version is refused as such, before anything else.
#[test]
fn a_newer_file_is_refused() {
    let key = BackupKey::of(&seed(1));
    let bytes = seal(&key, "testnet", &full()).unwrap();
    let mut e: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    e["version"] = (VERSION + 1).into();
    let bytes = serde_json::to_vec(&e).unwrap();
    assert_eq!(open(&key, "testnet", &bytes).err(), Some(OpenError::Newer));
}

/// AC-A2 (the byte guard): the file holds neither the seed nor its key — raw or in hex — and
/// says nothing of the wallet but its fingerprint.
#[test]
fn the_file_has_no_seed_and_no_key() {
    let seed = seed(9);
    let key = BackupKey::of(&seed);
    let bytes = seal(&key, "testnet", &full()).unwrap();
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(!has(&seed[..16]), "the seed's bytes");
    assert!(!has(hex(&seed[..16]).as_bytes()), "the seed in hex");
    assert!(!has(&key.key[..16]), "the key's bytes");
    assert!(!has(hex(&key.key[..16]).as_bytes()), "the key in hex");
    // Nor any of the contents in the clear.
    assert!(!has(b"Rent for May") && !has(b"yExchange") && !has(b"alice"));
}

/// AC-T-B2: the sections a file has are [`SECTIONS`], no more — no vault, no unfinished
/// operations, no network's history.
#[test]
fn the_file_has_its_sections_and_no_others() {
    let value = serde_json::to_value(full()).unwrap();
    let names: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(names, SECTIONS);
}

/// AC-T-B3, AC-A8: notes — the later record wins, a tie keeps this side's, a later deletion
/// here is not undone by an older note there, a later deletion there removes it here; the
/// same file again changes nothing (AC-A4).
#[test]
fn notes_merge_by_time_and_a_deletion_stays() {
    let mut here: BTreeMap<String, Note> = [
        ("core:new-here".to_owned(), note("mine, later", 200)),
        ("core:old-here".to_owned(), note("mine, earlier", 100)),
        ("core:deleted-here".to_owned(), gone(300)),
        ("core:tie".to_owned(), note("mine", 50)),
        ("core:deleted-there".to_owned(), note("will go", 10)),
    ]
    .into();
    let from: BTreeMap<String, Note> = [
        ("core:new-here".to_owned(), note("theirs, earlier", 150)),
        ("core:old-here".to_owned(), note("theirs, later", 150)),
        ("core:deleted-here".to_owned(), note("came back?", 250)),
        ("core:tie".to_owned(), note("theirs", 50)),
        ("core:deleted-there".to_owned(), gone(20)),
        ("core:only-there".to_owned(), note("added", 1)),
    ]
    .into();
    let merged = crate::notes::merge(&mut here, &from);
    let text = |k: &str| (here[k].text.clone(), here[k].deleted);
    assert_eq!(text("core:new-here"), ("mine, later".into(), false));
    assert_eq!(text("core:old-here"), ("theirs, later".into(), false));
    assert_eq!(
        text("core:deleted-here"),
        (String::new(), true),
        "no resurrection"
    );
    assert_eq!(text("core:tie"), ("mine".into(), false));
    assert_eq!(text("core:deleted-there"), (String::new(), true));
    assert_eq!(text("core:only-there"), ("added".into(), false));
    assert_eq!(
        merged,
        crate::notes::Merged {
            added: 1,
            newer: 2,
            kept: 3
        }
    );
    // The same again: nothing more taken.
    let again = crate::notes::merge(&mut here, &from);
    assert_eq!((again.added, again.newer), (0, 0));
}

/// AC-A8: the address book by address, this side's name kept; hidden requests a union; apps by
/// app and identity, the later sign-in winning; names by the later read. A repeat adds nothing.
#[test]
fn the_other_sections_merge_without_losses() {
    let mut book = vec![entry("yA", "Mine")];
    let from = [entry("yA", "Theirs"), entry("yB", "Shop")];
    assert_eq!(merge_book(&mut book, &from), 1);
    assert_eq!(book, vec![entry("yA", "Mine"), entry("yB", "Shop")]);
    assert_eq!(merge_book(&mut book, &from), 0);

    let mut hidden: BTreeSet<String> = ["S1".to_owned()].into();
    let from: BTreeSet<String> = ["S1".to_owned(), "S2".to_owned()].into();
    assert_eq!(merge_hidden(&mut hidden, &from), 1);
    assert_eq!(merge_hidden(&mut hidden, &from), 0);

    let mut apps = vec![app("C1", "I1", 10), app("C2", "I1", 50)];
    let from = [app("C1", "I1", 20), app("C2", "I1", 40), app("C3", "I2", 1)];
    assert_eq!(merge_apps(&mut apps, &from), 2);
    assert_eq!(
        apps.iter().map(|a| a.last_used).collect::<Vec<_>>(),
        [20, 50, 1]
    );
    assert_eq!(merge_apps(&mut apps, &from), 0);

    let kept = |name: &str, at| Kept {
        name: Some(name.into()),
        at,
    };
    let mut names: BTreeMap<String, Kept> = [
        ("I1".to_owned(), kept("old", 1)),
        ("I2".to_owned(), kept("here", 9)),
    ]
    .into();
    let from: BTreeMap<String, Kept> = [
        ("I1".to_owned(), kept("new", 5)),
        ("I2".to_owned(), kept("there", 3)),
    ]
    .into();
    assert_eq!(merge_names(&mut names, &from), 1);
    assert_eq!(names["I1"].name.as_deref(), Some("new"));
    assert_eq!(names["I2"].name.as_deref(), Some("here"));
    assert_eq!(merge_names(&mut names, &from), 0);
}

/// AC-T-B2′ (the architect, 2026-11-21): a journal line still on its way — a withdrawal queued,
/// pooled or broadcast, a contact request waiting for its answer — is not taken from a backup; a
/// finished one is, once, and one known here (by its line id) stays as it is.
#[test]
fn an_unfinished_journal_line_is_not_taken() {
    use crate::journal::{Entry, Journal, Reply, WithdrawalStatus, finished};
    let line = |id: u64, status, reply| Entry {
        at: 100 + id as i64,
        line: Some(id),
        status,
        reply,
        ..Entry::default()
    };
    let on_its_way = [
        line(1, Some(WithdrawalStatus::Queued), None),
        line(2, Some(WithdrawalStatus::Pooled), None),
        line(3, Some(WithdrawalStatus::Broadcasted), None),
        line(4, None, Some(Reply::Waiting)),
    ];
    assert!(on_its_way.iter().all(|l| !finished(l)));
    let done = [
        line(5, Some(WithdrawalStatus::Complete), None),
        line(6, None, Some(Reply::Accepted)),
        line(7, None, None),
    ];
    assert!(done.iter().all(finished));
    let dir = std::env::temp_dir().join(format!("evolite-backup-journal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let journal = Journal::new(dir.join("platform-history.json"));
    let from: Vec<Entry> = on_its_way.iter().chain(&done).cloned().collect();
    assert_eq!(journal.merge_backup(&from).unwrap(), 3);
    assert_eq!(
        journal.merge_backup(&from).unwrap(),
        0,
        "a repeat adds nothing"
    );
    let mut ids: Vec<u64> = journal.entries().iter().filter_map(|l| l.line).collect();
    ids.sort();
    assert_eq!(ids, [5, 6, 7]);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The analyst and the architect (2026-11-21): a name claim in the vote whose link was «not
/// written» comes from a backup as one to check — its retry not offered until the network is
/// read: (1) no link there → the retry is offered; (2) the link there → written, nothing
/// offered. A claim known here stays as it is.
#[test]
fn a_restored_unwritten_link_is_checked_before_it_is_offered() {
    use crate::name_requests::{NameRequest, NameRequests};
    let dir = std::env::temp_dir().join(format!("evolite-backup-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let requests = NameRequests::new(dir.join("name-requests.json"));
    let claim = |label: &str| NameRequest {
        identity: "Id1".into(),
        label: label.into(),
        sent_at: 1_000,
        join_until: 1_000 + 45 * 60,
        ends_about: 1_000 + 90 * 60,
        tally: Default::default(),
        fund: 10_000_000,
        result: None,
        hidden: false,
        contenders: Vec::new(),
        winner: None,
        checked_at: None,
        notice_closed: false,
        result_notice_closed: false,
        link: Some("https://example.org/me".into()),
        link_written: false,
        restored: false,
        link_unchecked: false,
    };
    assert_eq!(
        requests
            .merge_backup(&[claim("sam"), claim("max")])
            .unwrap(),
        2
    );
    let all = requests.load();
    assert!(all.iter().all(|r| r.link_unchecked), "to check");
    assert!(
        all.iter().all(|r| r.contest(2_000).link_written),
        "no retry offered before the check"
    );
    // (1) Not there: the retry is offered, still on a press only.
    let mut sam = all.iter().find(|r| r.label == "sam").unwrap().clone();
    sam.link_seen(None);
    assert!(!sam.link_unchecked && !sam.contest(2_000).link_written);
    // (2) There: written, nothing offered.
    let mut max = all.iter().find(|r| r.label == "max").unwrap().clone();
    max.link_seen(Some("https://example.org/me"));
    assert!(max.link_written && max.contest(2_000).link_written);
    assert_eq!(requests.merge_backup(&[claim("sam")]).unwrap(), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// AC-A11 (R-12): the sixth automatic copy lets the oldest of this wallet's go; a copy of
/// another wallet with an automatic copy's name, a manual copy and a file that is no backup stay;
/// two copies of one day get names of their own; a copy is due weekly.
#[test]
fn automatic_copies_keep_five_of_their_own_and_touch_nothing_else() {
    use super::auto;
    let dir = std::env::temp_dir().join(format!("evolite-auto-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mine = BackupKey::of(&seed(1));
    let theirs = BackupKey::of(&seed(2));
    // Not ours, by the fingerprint, with an automatic copy's name of this wallet's.
    let foreign = auto::name("Main", "testnet", "2026-01-01", 1);
    std::fs::write(
        dir.join(&foreign),
        seal(&theirs, "testnet", &full()).unwrap(),
    )
    .unwrap();
    // A manual copy and a stranger.
    let manual = "evolite-Main-testnet-2026-01-02.evbackup";
    std::fs::write(dir.join(manual), seal(&mine, "testnet", &full()).unwrap()).unwrap();
    std::fs::write(
        dir.join("evolite-x-testnet-2026-01-03-auto.evbackup"),
        b"not a backup",
    )
    .unwrap();
    let mut written = Vec::new();
    for day in 1..=6 {
        let day = format!("2026-02-0{day}");
        written.push(
            auto::write(&dir, &mine, "testnet", ("Main", &day), &full())
                .unwrap()
                .name,
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let own = auto::own_copies(&dir, &mine, "testnet");
    assert_eq!(own.len(), auto::KEEP, "five of its own");
    assert!(!dir.join(&written[0]).exists(), "the oldest went");
    assert!(dir.join(&written[5]).exists());
    assert!(dir.join(&foreign).exists(), "another wallet's stays");
    assert!(dir.join(manual).exists(), "a manual copy stays");
    assert!(
        dir.join("evolite-x-testnet-2026-01-03-auto.evbackup")
            .exists()
    );
    // The same day again: a name of its own, nothing written over.
    let again = auto::write(&dir, &mine, "testnet", ("Main", "2026-02-06"), &full())
        .unwrap()
        .name;
    assert_eq!(again, auto::name("Main", "testnet", "2026-02-06", 2));
    // Due: none yet, or a week since the last.
    assert!(auto::due(None, 100));
    assert!(!auto::due(Some(100), 100 + auto::EVERY - 1));
    assert!(auto::due(Some(100), 100 + auto::EVERY));
    // A folder that is not there: not found, nothing written.
    let gone = dir.join("removed");
    let err = auto::write(&gone, &mine, "testnet", ("Main", "2026-02-07"), &full()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Review r1, step 2 Should: copies are ordered by the day and number in their name, not by the
/// file's time (a copy moved or synced by a cloud has a new one).
#[test]
fn copies_are_ordered_by_their_name() {
    use super::auto::{name, order_of};
    assert_eq!(
        order_of(&name("My-Main", "testnet", "2026-02-06", 1)),
        Some(("2026-02-06".into(), 1))
    );
    assert_eq!(
        order_of(&name("My-Main", "testnet", "2026-02-06", 12)),
        Some(("2026-02-06".into(), 12))
    );
    assert_eq!(
        order_of("evolite-Main-testnet-2026-02-06.evbackup"),
        None,
        "manual"
    );
    assert_eq!(order_of("evolite-Main-testnet-x-auto.evbackup"), None);
}
