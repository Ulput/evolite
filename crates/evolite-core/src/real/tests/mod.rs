//! Tests of the real core, by area; the helpers they share are here.

use super::*;
use crate::network::{hrp_of, platform_address_on};

mod balance_tests;
mod dashpay_tests;
#[cfg(unix)]
mod folder_tests;
mod identity_tests;
mod notes_tests;
mod payment_tests;
mod snapshot_tests;
mod start_tests;
mod wallet_tests;

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// The text of every file under `dir`, as anyone with the disk could read it.
fn every_file_text(dir: &std::path::Path) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            text += &every_file_text(&path);
        } else if let Ok(bytes) = std::fs::read(&path) {
            text += &String::from_utf8_lossy(&bytes);
        }
    }
    text
}

fn has_notes(core: &Real) -> bool {
    core.notes.lock().unwrap().is_some()
}

/// The wallet folder the tests use.
const TEST_WALLET: &str = "0000beef";

/// A core over a vault sealed with `password` in a fresh temp dir; nothing is started.
fn core_with_vault(name: &str, password: &str) -> Real {
    let dir = std::env::temp_dir().join(format!("evolite-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::new(dir.clone(), TEST_WALLET);
    let meta = VaultMeta {
        network: network().into(),
        wallet_id: "ab".repeat(32),
        birth_height: 1_000,
        word_count: 12,
        created_at: 1_793_000_000,
    };
    let cheap = KdfParams {
        memory_kib: 256,
        iterations: 1,
        parallelism: 1,
    };
    let file = vault::seal(PHRASE, password, meta, cheap).unwrap();
    vault::save(&paths.vault, &file).unwrap();
    let (out, _) = std::sync::mpsc::channel();
    Real::new(dir, TEST_WALLET.into(), out, Arc::new(|| {}))
}

fn pw(text: &str) -> Password {
    Password::new(text.into())
}

/// Seals PHRASE (with its real `WalletId`) as wallet `id` of the data folder `dir`.
fn seal_wallet(dir: &std::path::Path, id: &str) {
    let meta = VaultMeta {
        network: network().into(),
        wallet_id: hex(&chain::wallet_id_of(PHRASE).unwrap()),
        birth_height: 1_000,
        word_count: 12,
        created_at: 1_793_000_000,
    };
    let cheap = KdfParams {
        memory_kib: 256,
        iterations: 1,
        parallelism: 1,
    };
    let file = vault::seal(PHRASE, "pw", meta, cheap).unwrap();
    vault::save(&wallets::wallet_dir(dir, id).join("wallet.json"), &file).unwrap();
}

fn core_at(dir: &std::path::Path, id: &str) -> Real {
    let (out, _) = std::sync::mpsc::channel();
    Real::new(dir.to_owned(), id.into(), out, Arc::new(|| {}))
}

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("evolite-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A start whose store was opened ahead, after an adding start that ends when `release`
/// is sent; the store is open when this returns.
async fn opened_ahead(dir: &std::path::Path) -> (Real, Real, tokio::sync::oneshot::Sender<()>) {
    seal_wallet(dir, "bbbb0002");
    let adding = core_at(dir, "cccc0003");
    adding.chain.retire();
    let (release, held) = tokio::sync::oneshot::channel::<()>();
    let adding_start = tokio::spawn(async move {
        let _ = held.await;
        false
    });
    let (said, before) = watch::channel(false);
    let _ = said.send(true);
    let mut back = core_at(dir, "bbbb0002");
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
            .is_ok()
    );
    (adding, back, release)
}

/// The core of wallet "bbbb0002" with its store open, as after its unlock (no SPV client
/// in a test), and the onboarding that "Add a wallet" makes from it: its chain parked.
async fn adding_from_open_store(dir: &std::path::Path) -> (Arc<Chain>, Real) {
    seal_wallet(dir, "bbbb0002");
    let mut from = core_at(dir, "bbbb0002");
    from.chain
        .open_store_only(&from.paths.chain)
        .await
        .expect("the store opens");
    from.rescanned_from = Some(1_234);
    // A rescan asked before "Add a wallet" and not finished yet (BUG-82 review r1 should 1).
    OwedRescan::add(
        &mut from.rescan_owed,
        [0xa; 32],
        1_600_000,
        vec!["t".into()],
        Vec::new(),
    );
    let chain = from.chain.clone();
    let mut adding = core_at(dir, "cccc0003");
    hand_on_chain(&mut from, &mut adding, true);
    (chain, adding)
}

fn count(n: &std::sync::atomic::AtomicUsize) -> usize {
    n.load(Ordering::Relaxed)
}

/// The masternode lists in, as dash-spv reports them once they are.
fn lists_in(chain: &Chain) {
    chain.set_status(|s| {
        s.masternodes = Some(sync::Stage {
            height: 1_000,
            target: 1_000,
            done: true,
        })
    });
}

/// Waits up to five seconds for `n` to reach `want` (a spawned ask).
async fn counted(n: &std::sync::atomic::AtomicUsize, want: usize) -> usize {
    for _ in 0..50 {
        if count(n) >= want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    count(n)
}

fn some_identity() -> String {
    Identifier::from([7; 32]).to_string(Encoding::Base58)
}

/// A chain's start under way, ending when the sender is used (or dropped).
fn held_start() -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<bool>,
) {
    let (release, held) = tokio::sync::oneshot::channel::<()>();
    let start = tokio::spawn(async move {
        let _ = held.await;
        true
    });
    (release, start)
}

fn entry(core: &mut Real, id: &str) -> WalletEntry {
    core.wallet_entries()
        .into_iter()
        .find(|w| w.id == id)
        .expect("the wallet is listed")
}

fn answer_of(event: Option<Event>) -> (DoneWhat, Result<(), CoreError>) {
    match event {
        Some(Event::Done { what, result }) => (what, result),
        other => panic!("{other:?}"),
    }
}

/// The core of `core_with_vault` with its wallet open (no keys: nothing can be signed).
fn opened(name: &str, password: &str) -> Real {
    let mut core = core_with_vault(name, password);
    let meta = core.saved.clone().unwrap();
    core.unlocked = Some(Unlocked {
        id: [0; 32],
        meta,
        keys: None,
        seed: None,
    });
    core
}

/// Events compared by what they say (`Event` has no `PartialEq`).
fn debug(event: &Option<Event>) -> String {
    format!("{event:?}")
}

/// Every spend there is (R-27 AC-T1).
fn every_spend() -> Vec<Spend> {
    vec![
        Spend::Send {
            coins: None,
            id: 0,
            from: Source::Core,
            to: "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ".into(),
            amount: 10_000,
            note: None,
            rate: None,
        },
        Spend::TopUpIdentity { amount: 10_000 },
        Spend::ResumeTopUp,
        Spend::CreateIdentity {
            name: Some("satoshi-test".into()),
            top_up: 10_000,
        },
        Spend::CreateIdentity {
            name: None,
            top_up: 10_000,
        },
        Spend::ResumeIdentity,
        Spend::AddName {
            name: "satoshi-test".into(),
        },
        Spend::Shield {
            from: ShieldSource::Core,
            amount: 10_000,
        },
        Spend::Unshield { amount: 10_000 },
        Spend::TopUpFromPool { amount: 10_000 },
        Spend::AddContact {
            name: "alice".into(),
        },
        Spend::AcceptContact {
            identity_id: "F5teLR3Vq4JjzQA7jgdGjZQvJbKpMubbu5dsC4MJd1Eh".into(),
        },
        Spend::SaveProfile {
            display: "A".into(),
            about: String::new(),
            avatar_url: None,
        },
        Spend::ApproveApp { id: 7, grant: None },
        Spend::GiftNft {
            id: "nft".into(),
            to: "@alice".into(),
        },
        // FEAT-0023: an IdentityUpdate the identity pays for, so behind the password (BUG-42).
        Spend::DisconnectApp { id: "app".into() },
    ]
}
