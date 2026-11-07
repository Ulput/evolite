//! The evolite wallet core. It runs in its own thread: the UI hands it [`Command`]s and takes
//! [`Event`]s back, so a frame never waits for the network and the UI never holds a key.
//!
//! Two cores sit behind the same [`Core`] handle: [`Core::real`] (dash-spv on testnet, the
//! wallet from the phrase, the encrypted vault) and `Core::mock` (the example wallet from Figma
//! with a staged first sync, for screenshots and the demo). The mock is built with the `demo`
//! feature, without `real` (then it is the only core) and for this crate's tests.

#![forbid(unsafe_code)]

/// Marks interface text (a reason the screens show): the literal as it is, in English, the key
/// of the catalogs in `evolite-ui/locales`, which `extract.pl` collects from these marks.
macro_rules! tr {
    ($text:literal) => {
        $text
    };
}

mod art;
mod art_cache;
pub mod attempts;
pub mod bip39;
pub mod diag;
mod dpns;
#[cfg(any(feature = "demo", not(feature = "real"), test))]
mod mock;
/// The password the demo core refuses (the UI tests' "Wrong password").
#[cfg(any(feature = "demo", not(feature = "real"), test))]
pub use mock::WRONG_PASSWORD as DEMO_WRONG_PASSWORD;
/// A spend without the password while "password for every send" is on (the screen did not
/// ask): the same refusal from the demo and the real core.
pub(crate) const PASSWORD_NEEDED: &str =
    tr!("This wallet asks for the password for every send — enter it and try again");
pub mod network;
#[cfg(feature = "real")]
mod phrase_check;
#[cfg(feature = "real")]
mod real;

use evolite_api::{Command, Event};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// How NFT pictures are drawn and kept.
#[derive(Clone, Debug, Default)]
pub struct ArtOptions {
    /// Where drawn pictures are kept between runs; `None`: drawn again every time.
    pub cache: Option<PathBuf>,
    /// The renderer process; `None`: `evolite-renderer` next to the app (Win32k off), or the
    /// app itself with `--render-worker`.
    pub worker: Option<evolite_render::Settings>,
    /// No pictures at all: no renderer process and nothing kept, `worker` and `cache` aside
    /// ([`Self::off`]).
    pub off: bool,
}

impl ArtOptions {
    /// Pictures are not drawn: no renderer process is started and nothing is written to the
    /// disk. For the UI tests: their executable is not the app (it would be started as the
    /// renderer), and they must not write to the user's folder.
    pub fn off() -> Self {
        Self {
            cache: None,
            worker: None,
            off: true,
        }
    }
}

/// The UI's end of the core: send commands, drain events.
pub struct Core {
    commands: Sender<Command>,
    events: Receiver<Event>,
    /// What the journal last saw of the sync and the vault.
    watch: std::sync::Mutex<diag::Watch>,
}

impl Core {
    /// Starts the mock core in a thread. `wake` is called after every event, so the UI can
    /// repaint (egui: `ctx.request_repaint()`).
    #[cfg(any(feature = "demo", not(feature = "real"), test))]
    pub fn mock(art: ArtOptions, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (commands, inbox) = channel();
        let (outbox, events) = channel();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        std::thread::Builder::new()
            .name("evolite-core".into())
            .spawn(move || mock::run(inbox, outbox, wake, art))
            .expect("start the core thread");
        Self::new(commands, events)
    }

    #[cfg(feature = "real")]
    /// [`Self::real`] on `network`, which stays the network of this process (testnet unless
    /// the app was started with `--network mainnet`). Returns `None` when another network was
    /// chosen earlier in this process.
    pub fn real_on(
        network: evolite_api::Network,
        data_dir: PathBuf,
        art: ArtOptions,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Option<Self> {
        network::choose(network).then(|| Self::real(data_dir, art, wake))
    }

    #[cfg(feature = "real")]
    /// Starts the real core: files under `data_dir` (SPV storage, the vault), on the network of
    /// this process ([`network::current`]: testnet unless chosen).
    pub fn real(
        data_dir: PathBuf,
        art: ArtOptions,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (commands, inbox) = channel();
        let (outbox, events) = channel();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        std::thread::Builder::new()
            .name("evolite-core".into())
            // Proof checks of Platform data recurse deeply; the runtime is built on this thread.
            .stack_size(16 << 20)
            .spawn(move || real::run(inbox, outbox, wake, data_dir, art))
            .expect("start the core thread");
        Self::new(commands, events)
    }

    pub fn send(&self, command: Command) {
        // `Touch` comes with every move of the mouse: not worth a line.
        if !matches!(command, Command::Touch) {
            diag::record(diag::command(&command));
        }
        // The core only stops with the app; a closed channel means it has crashed.
        let _ = self.commands.send(command);
    }

    /// Everything the core has said since the last call.
    pub fn events(&self) -> Vec<Event> {
        let events: Vec<Event> = self.events.try_iter().collect();
        let mut watch = self.watch.lock().unwrap_or_else(|e| e.into_inner());
        for event in &events {
            match event {
                Event::Snapshot(snapshot) => {
                    watch.snapshot(snapshot).into_iter().for_each(diag::record)
                }
                other => {
                    if let Some(record) = diag::answer(other) {
                        diag::record(record);
                    }
                }
            }
        }
        events
    }

    fn new(commands: Sender<Command>, events: Receiver<Event>) -> Self {
        Self {
            commands,
            events,
            watch: Default::default(),
        }
    }
}

/// A value from `.env.test.local` at the workspace root (never committed): the live tests'
/// phrases and the names of the test accounts stay out of the repository.
#[cfg(all(test, feature = "real"))]
pub(crate) fn test_env(key: &str) -> Option<zeroize::Zeroizing<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env.test.local");
    let text = zeroize::Zeroizing::new(std::fs::read_to_string(path).ok()?);
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| zeroize::Zeroizing::new(v.trim().trim_matches('"').to_owned()))
        .filter(|v| !v.is_empty())
}

/// A test account's DPNS name from `.env.test.local`, without the `@`.
#[cfg(all(test, feature = "real"))]
pub(crate) fn test_name(key: &str) -> String {
    test_env(key)
        .map(|v| v.trim_start_matches('@').to_owned())
        .unwrap_or_else(|| panic!("{key} in .env.test.local"))
}

#[cfg(all(test, feature = "real"))]
mod tests {
    use super::*;
    use evolite_api::{Password, Spend, SyncStage, VaultState};
    use std::time::{Duration, Instant};

    fn wait_for<T>(core: &Core, minutes: u64, mut found: impl FnMut(&Event) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(minutes * 60);
        loop {
            for event in core.events() {
                if let Some(t) = found(&event) {
                    return t;
                }
            }
            assert!(Instant::now() < deadline, "no answer in {minutes} min");
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// The folder of the wallet in a data folder: its own folder in `wallets/<network>/` since
    /// of its own (the first one there), or `wallets/<network>/` itself in a folder from before.
    fn wallet_folder(dir: &std::path::Path, network: &str) -> Option<std::path::PathBuf> {
        let base = dir.join("wallets").join(network);
        if base.join("wallet.json").exists() {
            return Some(base);
        }
        std::fs::read_dir(&base)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.join("wallet.json").exists())
    }

    /// The password of the warm copy of the test wallet.
    const WARM_PASSWORD: &str = "evolite live tests";

    /// A spend with the warm wallet's password: "password for every send" is on by default.
    fn spend(op: Spend) -> Command {
        Command::Spend {
            op,
            password: Some(Password::new(WARM_PASSWORD.into())),
        }
    }

    /// The test wallet, synced and open, in `%TEMP%\<name>`: a copy of `evolite-live-warm`.
    /// Restoring the phrase scans the whole chain (about 12 minutes); the warm copy is made
    /// once by such a restore, and a test on a copy catches up with the blocks since in
    /// seconds. Delete `%TEMP%\evolite-live-warm` to make it again from the phrase.
    fn warm_wallet(name: &str) -> Core {
        let warm = std::env::temp_dir().join("evolite-live-warm");
        if wallet_folder(&warm, "testnet").is_none() {
            make_warm(&warm);
        }
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&warm, &dir).expect("copy of the warm wallet");
        let core = Core::real(dir, ArtOptions::default(), || {});
        core.send(Command::Unlock {
            password: Password::new(WARM_PASSWORD.into()),
        });
        wait_for(&core, 5, |e| match e {
            Event::Done {
                what: "unlock",
                result,
            } => Some(result.clone()),
            _ => None,
        })
        .expect("the warm wallet opens");
        core
    }

    /// Restores the test phrase into `warm` and syncs it to the tip, then lets the core go.
    fn make_warm(warm: &std::path::Path) {
        let _ = std::fs::remove_dir_all(warm);
        let phrase =
            test_env("EVOLITE_TEST_PHRASE").expect("EVOLITE_TEST_PHRASE in .env.test.local");
        println!("making the warm wallet: a restore from the start of the chain, minutes");
        let core = Core::real(warm.to_path_buf(), ArtOptions::default(), || {});
        let words = phrase.split_whitespace().map(str::to_owned).collect();
        core.send(Command::UsePhrase {
            phrase: evolite_api::Phrase::new(words),
            dashpay: false,
        });
        core.send(Command::CreateWallet {
            password: Password::new(WARM_PASSWORD.into()),
        });
        wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                (s.sync.stage == SyncStage::Done && s.wallet.is_some()).then_some(())
            }
            _ => None,
        });
        // The core stops its client and lets go of the store once its channel closes.
        drop(core);
        std::thread::sleep(Duration::from_secs(10));
    }

    fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir(&entry.path(), &target)?;
            } else if entry.file_name() == "evolite.lock" {
                // The one-window lock of an app open on the folder: held, and not data.
            } else {
                match std::fs::copy(entry.path(), &target) {
                    // Part of it locked by an app open on the folder (SQLite's `-shm`, which a
                    // reader rebuilds from the log): left out.
                    Err(e) if e.raw_os_error() == Some(33) => {
                        println!("not copied (locked): {}", entry.path().display())
                    }
                    other => {
                        other?;
                    }
                }
            }
        }
        Ok(())
    }

    /// BUG-8 on testnet: "Add a wallet", then back to the wallet open before ("Cancel"), and
    /// its password opens it again — the chain of the wallet comes back. Spends nothing.
    /// `cargo test -p evolite-core live_cancel_adding -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and the test phrase in .env.test.local"]
    fn live_cancel_adding_a_wallet_opens_the_old_one() {
        let core = warm_wallet("evolite-live-cancel-adding");
        // Synced to the end, as in the report: the Platform and DashPay syncs hold the store too.
        wait_for(&core, 20, |e| match e {
            Event::Snapshot(s)
                if s.vault == VaultState::Unlocked
                    && s.wallet.is_some()
                    && s.sync.stage == SyncStage::Done =>
            {
                Some(())
            }
            _ => None,
        });
        // And a little longer, for the background syncs to run a pass.
        std::thread::sleep(Duration::from_secs(30));
        // Each change of (vault, adding) with its time: how long each switch of cores takes.
        let mut seen = None;
        let mut shown = |e: &Event, since: Instant| {
            if let Event::Snapshot(s) = e
                && seen != Some((s.vault, s.adding_wallet))
            {
                seen = Some((s.vault, s.adding_wallet));
                println!(
                    "{} ms: {:?}, adding {}, sync {:?} {:?}",
                    since.elapsed().as_millis(),
                    s.vault,
                    s.adding_wallet,
                    s.sync.stage,
                    s.sync.error
                );
            }
        };
        let added = Instant::now();
        core.send(Command::AddWallet);
        wait_for(&core, 5, |e| {
            shown(e, added);
            match e {
                Event::Snapshot(s) if s.adding_wallet && s.vault == VaultState::NoWallet => {
                    Some(())
                }
                _ => None,
            }
        });
        let started = Instant::now();
        core.send(Command::CancelAddWallet);
        wait_for(&core, 5, |e| {
            shown(e, started);
            match e {
                Event::Snapshot(s) if !s.adding_wallet && s.vault == VaultState::Locked => Some(()),
                _ => None,
            }
        });
        println!(
            "back at the old wallet in {} ms",
            started.elapsed().as_millis()
        );
        // At once, as a person types the password right away.
        core.send(Command::Unlock {
            password: Password::new(WARM_PASSWORD.into()),
        });
        let result = wait_for(&core, 5, |e| match e {
            Event::Done {
                what: "unlock",
                result,
            } => Some(result.clone()),
            Event::Snapshot(s) if s.sync.error.is_some() => {
                println!("sync error: {:?}", s.sync.error);
                None
            }
            _ => None,
        });
        println!(
            "unlock after {} ms: {result:?}",
            started.elapsed().as_millis()
        );
        assert_eq!(result, Ok(()), "the old wallet opens");
        // BUG-65: what the store has is shown at once, not after the adding chain's start
        // (≈50 s) and its stop.
        let unlocked = Instant::now();
        let history = wait_for(&core, 5, |e| match e {
            Event::Snapshot(s)
                if s.vault == VaultState::Unlocked
                    && s.wallet.as_ref().is_some_and(|w| !w.activity.is_empty()) =>
            {
                Some(s.sync.stage)
            }
            _ => None,
        });
        let after = unlocked.elapsed();
        println!(
            "history after the password in {} ms (sync {history:?})",
            after.as_millis()
        );
        assert!(
            after < Duration::from_secs(15),
            "the store is read at once: {after:?}"
        );
        let synced = wait_for(&core, 10, |e| match e {
            Event::Snapshot(s)
                if s.vault == VaultState::Unlocked && s.sync.stage == SyncStage::Done =>
            {
                Some(s.wallet.as_ref().map_or(0, |w| w.activity.len()))
            }
            _ => None,
        });
        assert!(synced > 0, "its history is back");
    }

    /// Deleting the wallet from this computer, on testnet with a new empty wallet: a wrong
    /// password deletes nothing, the right one deletes the vault at once and the core comes back
    /// with no wallet; a new wallet goes into a folder of its own, and "Forgot the password?"
    /// (no password) deletes it. Spends nothing.
    /// `cargo test -p evolite-core live_delete -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access"]
    fn live_delete_the_wallet_and_make_another() {
        let dir = std::env::temp_dir().join("evolite-live-delete");
        let _ = std::fs::remove_dir_all(dir.join("wallets"));
        let password = || Password::new("Correct-horse-7".into());
        let make = |core: &Core| {
            core.send(Command::NewPhrase { words: 12 });
            wait_for(core, 1, |e| matches!(e, Event::NewPhrase(_)).then_some(()));
            core.send(Command::CreateWallet {
                password: password(),
            });
            wait_for(core, 5, |e| match e {
                Event::Done {
                    what: "create",
                    result,
                } => Some(result.clone()),
                _ => None,
            })
        };
        let deleted = |core: &Core, password: Option<Password>| {
            core.send(Command::DeleteWallet {
                password,
                wallet: None,
            });
            wait_for(core, 2, |e| match e {
                Event::Done {
                    what: "delete",
                    result,
                } => Some(result.clone()),
                _ => None,
            })
        };

        let core = Core::real(dir.clone(), ArtOptions::default(), || {});
        make(&core).expect("wallet created");
        let vault = wallet_folder(&dir, "testnet")
            .expect("the wallet's folder")
            .join("wallet.json");
        assert_eq!(
            deleted(&core, Some(Password::new("wrong".into()))),
            Err(evolite_api::CoreError::WrongPassword)
        );
        assert!(vault.exists(), "a wrong password deletes nothing");
        deleted(&core, Some(password())).expect("deleted");
        wait_for(&core, 2, |e| match e {
            Event::Snapshot(s) if s.vault == VaultState::NoWallet && s.wallet.is_none() => Some(()),
            _ => None,
        });
        assert!(!vault.exists(), "the vault is gone at once");
        assert!(dir.join("spv").exists(), "headers and filters stay");
        // A new wallet in a folder of its own, not over the deleted one's store.
        make(&core).expect("a new wallet after the deletion");
        let again = wallet_folder(&dir, "testnet")
            .expect("the new wallet's folder")
            .join("wallet.json");
        assert_ne!(again, vault);
        deleted(&core, None).expect("deleted without the password (\"Forgot the password?\")");
        assert!(!again.exists());
    }

    /// On testnet: two wallets in one data folder. The warm test wallet (A, moved into a
    /// folder of its own), then a second one added beside it from a new phrase (B); the test
    /// phrase is refused as a second wallet. Switching locks one and opens the other at its
    /// password, each with its own balance and history (nothing of A's in B), and a new start
    /// opens the one open last. Spends nothing.
    /// `cargo test -p evolite-core live_two_wallets -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and the test phrase in .env.test.local"]
    fn live_two_wallets_switch() {
        let name = "evolite-live-wallets";
        let dir = std::env::temp_dir().join(name);
        let core = warm_wallet(name);
        let synced = |core: &Core, minutes| {
            wait_for(core, minutes, |e| match e {
                Event::Snapshot(s)
                    if s.vault == VaultState::Unlocked
                        && s.sync.stage == SyncStage::Done
                        && s.wallet.is_some() =>
                {
                    Some((**s).clone())
                }
                _ => None,
            })
        };
        let a = synced(&core, 10);
        assert_eq!(
            a.wallets.len(),
            1,
            "the warm wallet, moved: {:?}",
            a.wallets
        );
        let a_id = a.wallets[0].id.clone();
        assert!(a.wallets[0].open);
        let a_wallet = a.wallet.unwrap();
        let (a_total, a_ops) = (a_wallet.balances.total(), a_wallet.activity.len());
        println!("A: {a_ops} operations");
        assert!(a_ops > 0, "the test wallet has a history");

        core.send(Command::AddWallet);
        wait_for(&core, 2, |e| match e {
            Event::Snapshot(s) if s.adding_wallet && s.vault == VaultState::NoWallet => Some(()),
            _ => None,
        });
        // The test phrase is A: not a second wallet.
        let phrase = test_env("EVOLITE_TEST_PHRASE").expect("EVOLITE_TEST_PHRASE");
        core.send(Command::UsePhrase {
            phrase: evolite_api::Phrase::new(
                phrase.split_whitespace().map(str::to_owned).collect(),
            ),
            dashpay: false,
        });
        drop(phrase);
        let refused = wait_for(&core, 1, |e| match e {
            Event::PhraseAccepted(result) => Some(result.clone()),
            _ => None,
        });
        assert!(
            matches!(&refused, Err(evolite_api::CoreError::NotAvailable(t)) if t.contains("already on this computer")),
            "{refused:?}"
        );

        let b_password = || Password::new("Second-wallet-7".into());
        core.send(Command::NewPhrase { words: 12 });
        wait_for(&core, 1, |e| matches!(e, Event::NewPhrase(_)).then_some(()));
        core.send(Command::CreateWallet {
            password: b_password(),
        });
        let b = synced(&core, 10);
        assert_eq!(b.wallets.len(), 2);
        assert!(!b.adding_wallet);
        let b_id = b.wallets.iter().find(|w| w.open).unwrap().id.clone();
        assert_ne!(b_id, a_id);
        let b_wallet = b.wallet.unwrap();
        assert_eq!(b_wallet.balances.total(), 0, "nothing of A's in B");
        assert!(b_wallet.activity.is_empty(), "nothing of A's history in B");

        let unlock = |core: &Core, password: Password| {
            core.send(Command::Unlock { password });
            wait_for(core, 5, |e| match e {
                Event::Done {
                    what: "unlock",
                    result,
                } => Some(result.clone()),
                _ => None,
            })
        };
        let switch_to = |core: &Core, id: &str| {
            core.send(Command::SwitchWallet {
                id: id.into(),
                password: None,
            });
            wait_for(core, 2, |e| match e {
                Event::Snapshot(s)
                    if s.vault == VaultState::Locked
                        && s.wallet.is_none()
                        && s.wallets.iter().any(|w| w.open && w.id == id) =>
                {
                    Some(())
                }
                _ => None,
            })
        };
        switch_to(&core, &a_id);
        assert_eq!(
            unlock(&core, b_password()),
            Err(evolite_api::CoreError::WrongPassword),
            "B's password does not open A"
        );
        unlock(&core, Password::new(WARM_PASSWORD.into())).expect("A opens");
        let again = synced(&core, 5).wallet.unwrap();
        assert_eq!(again.balances.total(), a_total, "A's balance, as before");
        assert!(again.activity.len() >= a_ops, "A's history, as before");

        // Figma 29: B by its password, checked before A is locked — a wrong one changes nothing.
        let switch_with = |core: &Core, password: Password| {
            core.send(Command::SwitchWallet {
                id: b_id.clone(),
                password: Some(password),
            });
            wait_for(core, 5, |e| match e {
                Event::Done {
                    what: "switch",
                    result,
                } => Some(result.clone()),
                _ => None,
            })
        };
        assert_eq!(
            switch_with(&core, Password::new(WARM_PASSWORD.into())),
            Err(evolite_api::CoreError::WrongPassword),
            "A's password does not open B"
        );
        let still = synced(&core, 1);
        assert!(
            still.wallets.iter().any(|w| w.open && w.id == a_id),
            "A stays open"
        );
        switch_with(&core, b_password()).expect("B opens by its password");
        let b_again = synced(&core, 5);
        assert!(b_again.wallets.iter().any(|w| w.open && w.id == b_id));
        let b_again = b_again.wallet.unwrap();
        assert_eq!(b_again.balances.total(), 0);
        assert!(b_again.activity.is_empty());

        // A new start opens the wallet open last: B, locked.
        drop(core);
        std::thread::sleep(Duration::from_secs(10));
        let core = Core::real(dir.clone(), ArtOptions::default(), || {});
        let start = wait_for(&core, 2, |e| match e {
            Event::Snapshot(s) if !s.wallets.is_empty() => Some((**s).clone()),
            _ => None,
        });
        assert_eq!(start.vault, VaultState::Locked);
        assert!(start.wallets.iter().any(|w| w.open && w.id == b_id));
        println!("two wallets, switched both ways and restarted: ok");
    }

    /// The real core end to end on testnet: a new wallet, its first sync to a ChainLocked tip,
    /// lock and unlock with the password. Needs the network and minutes:
    /// `cargo test -p evolite-core live_real_core -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs testnet access and several minutes"]
    fn live_real_core_creates_syncs_and_unlocks() {
        let dir = std::env::temp_dir().join("evolite-live-core");
        let _ = std::fs::remove_dir_all(dir.join("wallets"));
        let core = Core::real(dir, ArtOptions::default(), || {});
        let password = || Password::new("correct horse battery staple".into());
        // Nobody touches the app here: the default auto-lock (15 min) would close the wallet
        // before a first sync that runs longer.
        core.send(Command::AutoLock { after_secs: None });
        let started = Instant::now();

        core.send(Command::NewPhrase { words: 12 });
        wait_for(&core, 1, |e| matches!(e, Event::NewPhrase(_)).then_some(()));
        core.send(Command::CreateWallet {
            password: password(),
        });
        wait_for(&core, 5, |e| match e {
            Event::Done {
                what: "create",
                result,
            } => Some(result.clone()),
            _ => None,
        })
        .expect("wallet created");

        let synced = wait_for(&core, 30, |e| match e {
            Event::Snapshot(s) if s.sync.stage == SyncStage::Done && s.sync.chainlocked => {
                Some(s.clone())
            }
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                None
            }
            _ => None,
        });
        let wallet = synced.wallet.expect("an open wallet");
        println!(
            "synced: block {}, address {}, first sync {} s",
            synced.sync.tip_height,
            wallet.core_address,
            started.elapsed().as_secs()
        );
        assert_eq!(synced.vault, VaultState::Unlocked);
        assert!(wallet.core_address.starts_with('y'), "a testnet address");
        assert_eq!(wallet.balances.core, 0);

        core.send(Command::Lock);
        wait_for(&core, 2, |e| match e {
            Event::Snapshot(s) if s.vault == VaultState::Locked => Some(()),
            _ => None,
        });
        core.send(Command::Unlock {
            password: Password::new("wrong".into()),
        });
        let wrong = wait_for(&core, 2, |e| match e {
            Event::Done {
                what: "unlock",
                result,
            } => Some(result.clone()),
            _ => None,
        });
        assert_eq!(wrong, Err(evolite_api::CoreError::WrongPassword));
        core.send(Command::Unlock {
            password: password(),
        });
        wait_for(&core, 5, |e| match e {
            Event::Done {
                what: "unlock",
                result,
            } => Some(result.clone()),
            _ => None,
        })
        .expect("unlocked");
    }

    /// Prints how the wallet's balance and history change for `secs` seconds of snapshots.
    fn watch(core: &Core, label: &str, secs: u64) -> (i64, usize) {
        let until = Instant::now() + Duration::from_secs(secs);
        let mut last = (-1, usize::MAX);
        while Instant::now() < until {
            for event in core.events() {
                if let Event::Snapshot(s) = event
                    && let Some(w) = &s.wallet
                {
                    let now = (w.balances.core, w.activity.len());
                    if now != last {
                        println!(
                            "{label}: {:?} filters {} / {}, balance {}, {} operations",
                            s.sync.stage, s.sync.filters_height, s.sync.tip_height, now.0, now.1
                        );
                        last = now;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        last
    }

    /// The history of a funded wallet is whole: right after the first sync, a while later, and
    /// after locking and unlocking on the same storage (what a restart of the app does).
    /// `cargo test -p evolite-core live_history -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_history_survives_unlock() {
        let phrase =
            test_env("EVOLITE_TEST_PHRASE").expect("EVOLITE_TEST_PHRASE in .env.test.local");
        let dir = std::env::temp_dir().join("evolite-live-send");
        let _ = std::fs::remove_dir_all(dir.join("wallets"));
        let core = Core::real(dir, ArtOptions::default(), || {});
        let password = || Password::new("live history test".into());
        let words = phrase.split_whitespace().map(str::to_owned).collect();
        core.send(Command::UsePhrase {
            phrase: evolite_api::Phrase::new(words),
            dashpay: false,
        });
        core.send(Command::CreateWallet {
            password: password(),
        });
        let first = watch(&core, "restored", 240);
        core.send(Command::Lock);
        watch(&core, "locked", 20);
        core.send(Command::Unlock {
            password: password(),
        });
        let again = watch(&core, "unlocked", 240);
        assert!(first.1 > 0, "no history after the first sync");
        assert_eq!(again, first, "balance or history changed after unlocking");
    }

    /// Stage 4 on testnet: a funded wallet pays 0.0001 DASH to its own next address and the
    /// network relays it. Only the fee is spent. Needs `EVOLITE_TEST_PHRASE` (and optionally
    /// `EVOLITE_TEST_BIRTH`) in `.env.test.local`; the phrase is never printed.
    /// `cargo test -p evolite-core live_real_send -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_real_send_is_relayed() {
        let core = warm_wallet("evolite-live-send");
        let synced = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) if s.sync.stage == SyncStage::Done => Some(s.clone()),
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                None
            }
            _ => None,
        });
        let wallet = synced.wallet.expect("an open wallet");
        println!(
            "balance {} duffs, {} operations, next address {}, tip {}, headers {}, filters {}",
            wallet.balances.core,
            wallet.activity.len(),
            wallet.core_address,
            synced.sync.tip_height,
            synced.sync.headers_height,
            synced.sync.filters_height
        );
        assert!(wallet.balances.core > 20_000, "the test wallet needs coins");

        core.send(spend(Spend::Send {
            from: evolite_api::Source::Core,
            to: wallet.core_address.clone(),
            amount: 10_000,
        }));
        let sent = wait_for(&core, 3, |e| match e {
            Event::Sent(r) => Some(r.clone()),
            _ => None,
        })
        .expect("payment signed and broadcast");
        println!(
            "sent {} fee {} relayed {}",
            sent.txid, sent.fee, sent.relayed
        );
        assert!(sent.relayed, "the network did not relay the payment");
        assert!(sent.fee > 0 && sent.fee <= wallet.fees.core_payment);
    }

    /// What a send did to the history (Overview's recent operations and the History are the same
    /// list), snapshot by snapshot for `secs`: when its line first showed among the top 5, and
    /// the new lines of the last snapshot. A Core payment's line also keeps the time it was
    /// first seen until a block takes it (BUG-12: an InstantSend payment was "now" again at
    /// every snapshot).
    fn watch_send(
        core: &Core,
        wallet: &evolite_api::Wallet,
        from: evolite_api::Source,
        to: String,
        amount: evolite_api::Duffs,
        secs: u64,
    ) -> (Duration, Vec<evolite_api::Activity>) {
        let before: Vec<u64> = wallet.activity.iter().map(|a| a.id).collect();
        let started = Instant::now();
        core.send(spend(Spend::Send { from, to, amount }));
        let mut first_shown = None;
        let mut sent = None;
        let mut last_new = Vec::new();
        let mut seen_at: std::collections::HashMap<String, i64> = Default::default();
        while started.elapsed() < Duration::from_secs(secs) {
            for event in core.events() {
                match event {
                    Event::Sent(r) => {
                        println!("{} ms: sent {r:?}", started.elapsed().as_millis());
                        sent = Some(r.expect("signed and taken"));
                    }
                    Event::Snapshot(s) => {
                        let Some(w) = &s.wallet else { continue };
                        let new: Vec<_> = w
                            .activity
                            .iter()
                            .filter(|a| !before.contains(&a.id))
                            .cloned()
                            .collect();
                        let brief: Vec<_> = new
                            .iter()
                            .map(|a| (a.id, a.proof, a.at, a.amount, a.block))
                            .collect();
                        let was: Vec<_> = last_new
                            .iter()
                            .map(|a: &evolite_api::Activity| {
                                (a.id, a.proof, a.at, a.amount, a.block)
                            })
                            .collect();
                        if brief != was {
                            println!("{} ms: new {brief:?}", started.elapsed().as_millis());
                        }
                        // This run's "being sent" line has the time of signing; the wallet's own
                        // line, the time it was seen.
                        for a in new
                            .iter()
                            .filter(|a| a.block.is_none() && a.proof != evolite_api::Proof::Sending)
                        {
                            if let Some(txid) = &a.txid {
                                let at = *seen_at.entry(txid.clone()).or_insert(a.at);
                                assert_eq!(a.at, at, "the time it was seen stays: {a:?}");
                            }
                        }
                        if first_shown.is_none()
                            && w.activity.iter().take(5).any(|a| !before.contains(&a.id))
                        {
                            first_shown = Some(started.elapsed());
                        }
                        last_new = new;
                    }
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        sent.expect("an answer to Send");
        let shown = first_shown.expect("the send never showed in the recent operations");
        println!("first shown after {shown:?}");
        (shown, last_new)
    }

    /// BUG-12 on testnet, Core: a payment to the wallet's own address is in the history at once
    /// ("Being sent" from the snapshot right after "Sign", then with its txid), once, and keeps
    /// its time. Only the fee is spent (~226 duffs).
    /// `cargo test -p evolite-core live_payment_shows -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_payment_shows_in_the_history_at_once() {
        let core = warm_wallet("evolite-live-send-history");
        let wallet = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) if s.sync.stage == SyncStage::Done => s.wallet.clone(),
            _ => None,
        });
        assert!(wallet.balances.core > 20_000, "the test wallet needs coins");
        let to = wallet.core_address.clone();
        let (shown, new) = watch_send(&core, &wallet, evolite_api::Source::Core, to, 10_000, 90);
        assert!(shown < Duration::from_secs(2), "shown after {shown:?}");
        assert_eq!(new.len(), 1, "one line: {new:?}");
        assert!(new[0].txid.is_some());
    }

    /// BUG-64 on testnet, reads only: once synced, the test wallet's history has no Core line
    /// without a block that is in fact long mined (the customer's: received 2026-10-26, block
    /// 1 579 644, shown as "Pending" today). Prints every line still without a block.
    /// `cargo test -p evolite-core live_nothing_old_waits -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and the test phrase in .env.test.local"]
    fn live_nothing_old_waits() {
        use evolite_api::Proof;
        const OLD: &str = "0666a44a69e6fff9f67384b2791e3509572a72e0035af84703aad2355d67f919";
        // `EVOLITE_B64_QA=1`: on a copy of the QA data folder instead (its werner555 wallet,
        // «Кошелёк 2», where the customer saw it), opened with the QA password from its file.
        let qa = std::env::var("EVOLITE_B64_QA").is_ok();
        let password = if qa {
            // A worktree has no `.test-identities`: then the main checkout's.
            let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.test-identities/qa-evolite-password.txt");
            let main = std::path::Path::new(r"E:\reps\evolite-rust\.test-identities")
                .join("qa-evolite-password.txt");
            std::fs::read_to_string(&here)
                .or_else(|_| std::fs::read_to_string(&main))
                .expect("the QA password file")
                .trim()
                .to_owned()
        } else {
            WARM_PASSWORD.to_owned()
        };
        let core = if qa {
            let dir = std::env::temp_dir().join("evolite-live-b64-qa");
            let _ = std::fs::remove_dir_all(&dir);
            copy_dir(std::path::Path::new(r"E:\reps\qa-data"), &dir).expect("a copy of qa-data");
            let core = Core::real(dir, ArtOptions::default(), || {});
            core.send(Command::Unlock {
                password: Password::new(password.clone()),
            });
            let wallets = wait_for(&core, 5, |e| match e {
                Event::Done {
                    what: "unlock",
                    result,
                } => Some(result.clone()),
                _ => None,
            });
            println!("QA copy unlocked: {wallets:?}");
            let target = wait_for(&core, 2, |e| match e {
                Event::Snapshot(s) => s
                    .wallets
                    .iter()
                    .find(|w| w.name == "Кошелёк 2")
                    .map(|w| (w.id.clone(), w.open)),
                _ => None,
            });
            if !target.1 {
                core.send(Command::SwitchWallet {
                    id: target.0,
                    password: Some(Password::new(password.clone())),
                });
            }
            core
        } else {
            warm_wallet("evolite-live-nothing-old-waits")
        };
        // Prints the stage, Core balance, the lines without a block and the old payment each
        // time they change, for `secs`; the last of them.
        let watch = |label: &str, secs: u64| {
            let started = Instant::now();
            let mut last = None;
            while started.elapsed() < Duration::from_secs(secs) {
                for event in core.events() {
                    let Event::Snapshot(s) = event else { continue };
                    let Some(w) = &s.wallet else { continue };
                    let waiting: Vec<_> = w
                        .activity
                        .iter()
                        .filter(|a| {
                            a.chain != evolite_api::Chain::Platform
                                && a.block.is_none()
                                && a.proof != Proof::Proved
                        })
                        .map(|a| {
                            let txid = a.txid.clone().unwrap_or_default();
                            (txid, format!("{:?}", a.kind), a.proof, a.at)
                        })
                        .collect();
                    let old = w
                        .activity
                        .iter()
                        .find(|a| a.txid.as_deref() == Some(OLD))
                        .map(|a| (a.proof, a.block, a.at));
                    let mined = w.activity.iter().filter(|a| a.block.is_some()).count();
                    let now = (s.sync.stage, w.balances.core, waiting, old);
                    if last.as_ref() != Some(&now) {
                        println!(
                            "{label} {} s: headers {} filters {} tip {} mined {mined}/{} {now:?}",
                            started.elapsed().as_secs(),
                            s.sync.headers_height,
                            s.sync.filters_height,
                            s.sync.tip_height,
                            w.activity.len()
                        );
                        last = Some(now);
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            last.expect("snapshots")
        };
        let (stage, _, waiting, old) = watch("open", if qa { 240 } else { 90 });
        // The QA copy may hold the customer's broken store: then it goes on to the restart.
        let first_ok = stage == SyncStage::Done
            && old.is_none_or(|(_, block, _)| block.is_some())
            && waiting.is_empty();
        println!("open: {stage:?} old {old:?} waiting {waiting:?}");
        assert!(qa || first_ok, "after opening");
        // The customer's way (BUG-64): "Add a wallet", back, the password again — a new chain
        // on the same store in the same process.
        core.send(Command::AddWallet);
        std::thread::sleep(Duration::from_secs(3));
        core.send(Command::CancelAddWallet);
        std::thread::sleep(Duration::from_secs(3));
        core.send(Command::Unlock {
            password: Password::new(password.clone()),
        });
        let (stage, _, waiting, old) = watch("again", 4 * 60);
        assert_eq!(stage, SyncStage::Done);
        assert!(
            old.is_none_or(|(_, block, _)| block.is_some()),
            "the old payment waits: {old:?}"
        );
        assert!(waiting.is_empty(), "lines without a block: {waiting:?}");
    }

    /// BUG-77 on a copy of the QA data folder (werner555, «Кошелёк 2»; nothing is spent): in
    /// every snapshot the Core balance is a checked one — the sum of the coins each once while
    /// the filters are at the tip, the last checked one with its time otherwise — and never
    /// the sum counting the 7 DIP-15 coins twice (3,91724464) or one a rewound pass brings
    /// spent coins back into (6,80…). Then «Add a wallet» → back → the password: the DashPay
    /// backfill rewinds the filters again, and the checked balance stays on screen.
    /// `cargo test -p evolite-core live_the_core_balance_shown -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and the QA data folder"]
    fn live_the_core_balance_shown_is_always_a_checked_one() {
        use evolite_api::CoreAsOf;
        const DOUBLE: i64 = 391_724_464;
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.test-identities/qa-evolite-password.txt");
        let main = std::path::Path::new(r"E:\reps\evolite-rust\.test-identities")
            .join("qa-evolite-password.txt");
        let password = std::fs::read_to_string(&here)
            .or_else(|_| std::fs::read_to_string(&main))
            .expect("the QA password file")
            .trim()
            .to_owned();
        let dir = std::env::temp_dir().join("evolite-live-b77-qa");
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(std::path::Path::new(r"E:\reps\qa-data"), &dir).expect("a copy of qa-data");
        let core = Core::real(dir, ArtOptions::default(), || {});
        core.send(Command::Unlock {
            password: Password::new(password.clone()),
        });
        let target = wait_for(&core, 5, |e| match e {
            Event::Snapshot(s) if s.vault == VaultState::Unlocked => s
                .wallets
                .iter()
                .find(|w| w.name == "Кошелёк 2")
                .map(|w| (w.id.clone(), w.open)),
            _ => None,
        });
        if !target.1 {
            core.send(Command::SwitchWallet {
                id: target.0,
                password: Some(Password::new(password.clone())),
            });
        }
        // Every change of (stage, balance, as of) for `secs`, checked; the balances checked now
        // and whether one was shown as of earlier.
        let watch = |label: &str, secs: u64| {
            let started = Instant::now();
            let mut last = None;
            let mut checked = std::collections::BTreeSet::new();
            let mut earlier = false;
            while started.elapsed() < Duration::from_secs(secs) {
                for event in core.events() {
                    let Event::Snapshot(s) = event else { continue };
                    if !s.wallets.iter().any(|w| w.open && w.name == "Кошелёк 2") {
                        continue;
                    }
                    let Some(w) = &s.wallet else { continue };
                    let b = &w.balances;
                    let now = (s.sync.stage, b.core, b.core_as_of);
                    if last.as_ref() != Some(&now) {
                        println!(
                            "{label} {} s: filters {} tip {} {now:?}",
                            started.elapsed().as_secs(),
                            s.sync.filters_height,
                            s.sync.tip_height,
                        );
                        last = Some(now);
                    }
                    assert_ne!(b.core, DOUBLE, "{label}: the DIP-15 coins counted twice");
                    assert!(
                        b.core < 450_000_000,
                        "{label}: spent coins back in the balance: {now:?}"
                    );
                    match b.core_as_of {
                        None => {
                            checked.insert(b.core);
                        }
                        Some(CoreAsOf::Checked(_)) => earlier = true,
                        Some(CoreAsOf::NotYet) => assert_eq!(b.core, 0, "{label}: no sum unknown"),
                    }
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            (last.expect("snapshots"), checked, earlier)
        };
        let (last, checked, _) = watch("open", 5 * 60);
        println!("open: last {last:?}, checked {checked:?}");
        assert_eq!(last.2, None, "checked at the end");
        // The customer's way: a new chain on the same store, the backfill rewinding again.
        core.send(Command::AddWallet);
        std::thread::sleep(Duration::from_secs(3));
        core.send(Command::CancelAddWallet);
        std::thread::sleep(Duration::from_secs(3));
        core.send(Command::Unlock {
            password: Password::new(password.clone()),
        });
        let (again, checked_again, earlier) = watch("again", 4 * 60);
        println!("again: last {again:?}, checked {checked_again:?}, as of earlier {earlier}");
        assert_eq!(again.2, None, "checked at the end");
        assert_eq!(again.1, last.1, "the same balance as after opening");
        assert!(
            earlier,
            "while syncing again the checked balance is shown with its time"
        );
    }

    /// BUG-64 on testnet: a payment to the wallet's own address is "Confirmed" by its InstantLock
    /// in seconds, and stays at least that after the app is opened again on the same folder —
    /// then "In a block" and "Final" as the chain goes on. Prints each change of its status.
    /// Only the fee is spent (~226 duffs).
    /// `cargo test -p evolite-core live_instant_payment_stays -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_instant_payment_stays_confirmed_after_a_restart() {
        use evolite_api::Proof;
        let name = "evolite-live-instant-restart";
        let core = warm_wallet(name);
        let wallet = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) if s.sync.stage == SyncStage::Done => s.wallet.clone(),
            _ => None,
        });
        assert!(wallet.balances.core > 20_000, "the test wallet needs coins");
        let to = wallet.core_address.clone();
        let (_, new) = watch_send(&core, &wallet, evolite_api::Source::Core, to, 10_000, 30);
        let txid = new
            .iter()
            .find_map(|a| a.txid.clone())
            .expect("the payment's line");
        // Its status as each snapshot shows it, printed when it changes, until `done` says so.
        let follow = |core: &Core, label: &str, secs: u64, done: &dyn Fn(Proof) -> bool| {
            let started = Instant::now();
            let mut last = None;
            while started.elapsed() < Duration::from_secs(secs) {
                for event in core.events() {
                    if let Event::Snapshot(s) = event {
                        let Some(a) = s.wallet.as_ref().and_then(|w| {
                            w.activity.iter().find(|a| a.txid.as_ref() == Some(&txid))
                        }) else {
                            continue;
                        };
                        let now = (
                            a.proof,
                            a.block,
                            s.sync.stage,
                            s.sync.tip_height,
                            s.sync.chainlocked,
                        );
                        if last != Some(now) {
                            println!("{label} {} s: {now:?}", started.elapsed().as_secs());
                            last = Some(now);
                        }
                    }
                }
                if last.is_some_and(|(p, ..)| done(p)) {
                    return last.map(|(p, ..)| p);
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            last.map(|(p, ..)| p)
        };
        let first = follow(&core, "run 1", 60, &|p| {
            p != Proof::Mempool && p != Proof::Sending
        });
        println!("run 1: {first:?}");
        drop(core);
        std::thread::sleep(Duration::from_secs(10));

        // The same folder opened again, as the app is the next day.
        let dir = std::env::temp_dir().join(name);
        let core = Core::real(dir, ArtOptions::default(), || {});
        core.send(Command::Unlock {
            password: Password::new(WARM_PASSWORD.into()),
        });
        let again = follow(&core, "run 2", 9 * 60, &|p| p == Proof::ChainLock);
        assert_eq!(first, Some(Proof::InstantLock), "confirmed in run 1");
        assert_eq!(again, Some(Proof::ChainLock), "final in run 2");
    }

    /// BUG-12 on testnet, credits: 0.0001 DASH of the identity's credits to the wallet's own
    /// Platform address show as "Being sent" at once, then as the one line Platform proved.
    /// Spends Platform's fee (the amount stays the wallet's).
    /// `cargo test -p evolite-core live_credits_show -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_credits_show_in_the_history_at_once() {
        let core = warm_wallet("evolite-live-credits-history");
        let wallet = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done
                    && w.identity.is_some()
                    && w.platform_address.starts_with("tdash1"))
                .then(|| w.clone())
            }
            _ => None,
        });
        assert!(
            wallet.balances.credits > 30_000,
            "the identity needs credits"
        );
        let to = wallet.platform_address.clone();
        let (shown, new) = watch_send(&core, &wallet, evolite_api::Source::Credits, to, 10_000, 60);
        assert!(shown < Duration::from_secs(2), "shown after {shown:?}");
        assert_eq!(new.len(), 1, "one line: {new:?}");
        assert_eq!(new[0].proof, evolite_api::Proof::Proved);
    }

    /// BUG-12 on testnet, the pool: 0.001 DASH from the pool to the wallet's own private address
    /// shows as "Being sent" at once (the proof takes tens of seconds before Platform is asked),
    /// then as the pool's one line. Spends the pool's fee (the amount stays in the pool).
    /// `cargo test -p evolite-core --features pool live_pool_send_shows -- --ignored --nocapture`
    #[cfg(feature = "pool")]
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_pool_send_shows_in_the_history_at_once() {
        let core = warm_wallet("evolite-live-pool-history");
        let wallet = wait_for(&core, 30, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done
                    && w.pool.address.is_some()
                    && w.pool.checked_at.is_some())
                .then(|| w.clone())
            }
            _ => None,
        });
        assert!(wallet.balances.shielded > 1_000_000, "the pool needs funds");
        let to = wallet.pool.address.clone().unwrap_or_default();
        let (shown, new) = watch_send(&core, &wallet, evolite_api::Source::Pool, to, 100_000, 240);
        assert!(shown < Duration::from_secs(2), "shown after {shown:?}");
        assert_eq!(new.len(), 1, "one line: {new:?}");
        let ours = u64::MAX - (1 << 33)..u64::MAX - (1 << 32);
        assert!(!ours.contains(&new[0].id), "the pool's own line: {new:?}");
    }

    /// Identities on testnet: the test phrase's identities are found after a restore, and a new
    /// one is made from Core funds (asset lock of 0.003 DASH plus the name's fee) with a fresh
    /// DPNS name that then resolves. Spends about 0.013 tDASH.
    /// `cargo test -p evolite-core live_identity -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_identity_found_and_created() {
        let core = warm_wallet("evolite-live-identity");
        let found = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done && w.identity.is_some()).then(|| w.clone())
            }
            _ => None,
        });
        let identity = found.identity.as_ref().unwrap();
        println!(
            "found {} ({}), names {:?}, {} keys, credits {} duffs, core {} duffs",
            identity.id,
            identity.key_source,
            identity.names,
            identity.keys.len(),
            found.balances.credits,
            found.balances.core
        );

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Digits other than 0 and 1 keep it out of the contested set.
        let label = format!("evolite-{}", stamp % 100_000_000 + 22_222_222);
        assert!(!dpns::is_contested(&label));
        let started = Instant::now();
        core.send(spend(Spend::CreateIdentity {
            name: label.clone(),
            top_up: 300_000,
        }));
        let created = wait_for(&core, 15, |e| match e {
            Event::Done {
                what: "identity",
                result,
            } => Some(result.clone()),
            _ => None,
        });
        println!(
            "create @{label}: {created:?} in {}s",
            started.elapsed().as_secs()
        );
        created.expect("identity and name created");

        core.send(Command::ResolveName {
            name: format!("@{label}"),
        });
        let resolved = wait_for(&core, 2, |e| match e {
            Event::NameResolved {
                result: Ok(name), ..
            } => Some(name.clone()),
            Event::NameResolved { result: Err(e), .. }
                if *e != evolite_api::CoreError::NotSynced =>
            {
                panic!("@{label}: {e:?}")
            }
            _ => None,
        });
        println!("@{label} -> {}", resolved.identity_id);
        assert_ne!(
            resolved.identity_id, identity.id,
            "a new identity, not the first one"
        );
    }

    /// Two identities of the test phrase become DashPay contacts and one pays the other, end to
    /// end on testnet: `EVOLITE_TEST_NAME` asked `EVOLITE_TEST_NAME_2` (by `live_dashpay`); working as
    /// the second the request is accepted; back as the first, a Core payment to the new
    /// contact is previewed and signed only to the address shown. A few thousandths of a
    /// DASH in credits and 0.0001 DASH on Core.
    /// `cargo test -p evolite-core live_contacts -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_contacts_between_own_identities() {
        use evolite_api::{ContactState, IdentitySearch};
        let core = warm_wallet("evolite-live-contacts");
        let wallet = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done
                    && w.identity_search == IdentitySearch::Done
                    && w.identities.len() >= 2)
                    .then(|| w.clone())
            }
            _ => None,
        });
        let id_of = |name: &str| {
            wallet
                .identities
                .iter()
                .find(|c| c.name.as_deref() == Some(name))
                .map(|c| c.id.clone())
                .unwrap_or_else(|| panic!("@{name} among {:?}", wallet.identities))
        };
        let (a, b) = (
            id_of(&test_name("EVOLITE_TEST_NAME")),
            id_of(&test_name("EVOLITE_TEST_NAME_2")),
        );
        println!("{} identities; A {a}, B {b}", wallet.identities.len());

        // As B: the request from A is there to answer (or was answered before).
        core.send(Command::ChooseIdentity { id: b.clone() });
        let from_a = wait_for(&core, 5, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (w.identity.as_ref()?.id == b)
                    .then(|| w.contacts.iter().find(|c| c.identity_id == a).cloned())
                    .flatten()
            }
            _ => None,
        });
        println!("as B, A is {:?}", from_a.state);
        if from_a.state == ContactState::Incoming {
            core.send(spend(Spend::AcceptContact {
                identity_id: a.clone(),
            }));
            let accepted = wait_for(&core, 5, |e| match e {
                Event::Done {
                    what: "contact",
                    result,
                } => Some(result.clone()),
                _ => None,
            });
            println!("accepted: {accepted:?}");
            accepted.expect("request accepted");
        }

        // As A: B is a contact now, and a payment to it goes to the address shown.
        core.send(Command::ChooseIdentity { id: a.clone() });
        let to_b = wait_for(&core, 5, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (w.identity.as_ref()?.id == a)
                    .then(|| {
                        w.contacts
                            .iter()
                            .find(|c| c.identity_id == b && c.state == ContactState::Established)
                            .cloned()
                    })
                    .flatten()
            }
            _ => None,
        });
        println!("as A, B is {:?}, {} payments", to_b.state, to_b.payments);
        // As the Send screen does: the name is checked first, then the payment previewed.
        let to = format!("@{}", test_name("EVOLITE_TEST_NAME_2"));
        core.send(Command::ResolveName { name: to.clone() });
        let resolved = wait_for(&core, 3, |e| match e {
            Event::NameResolved { name, result } if *name == to => Some(result.clone()),
            _ => None,
        })
        .expect("the contact's name resolves");
        println!(
            "resolved: {} is {:?}",
            resolved.identity_id, resolved.contact
        );
        assert_eq!(resolved.identity_id, b);
        core.send(Command::PreviewPayment { to: to.clone() });
        let address = wait_for(&core, 3, |e| match e {
            Event::PaymentPreview { to: asked, result } if *asked == to => Some(result.clone()),
            _ => None,
        })
        .expect("the address the payment goes to");
        println!("preview: {to} -> {address}");
        core.send(spend(Spend::Send {
            from: evolite_api::Source::Core,
            to,
            amount: 10_000,
        }));
        let sent = wait_for(&core, 3, |e| match e {
            Event::Sent(r) => Some(r.clone()),
            _ => None,
        })
        .expect("payment to the contact signed and broadcast");
        println!(
            "sent {} fee {} relayed {}",
            sent.txid, sent.fee, sent.relayed
        );
        assert!(sent.relayed, "the network did not relay the payment");
    }

    /// On testnet: a Core payment to a DashPay contact, then a restart of the app on the
    /// same folder; the next payment to that contact goes to the next address of its key, not
    /// the one just paid. Run it twice, as two processes are two runs of the app (the store
    /// opens once per process): the first pays and writes the contact and the address it paid
    /// to a file in the folder; the second opens the same folder, previews the next payment and
    /// compares. Needs an identity with an established, named contact. Spends 0.0001 DASH on
    /// Core (the second payment is only previewed).
    /// `cargo test -p evolite-core live_contact_address -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase with a DashPay contact"]
    fn live_contact_address_survives_restart() {
        use evolite_api::ContactState;
        let name = "evolite-live-contact-restart";
        let dir = std::env::temp_dir().join(name);
        let paid_file = dir.join("contact-restart.json");
        let contact_of = |s: &evolite_api::Snapshot| {
            let w = s.wallet.as_ref()?;
            (s.sync.stage == SyncStage::Done)
                .then(|| {
                    w.contacts
                        .iter()
                        .find(|c| c.state == ContactState::Established && c.name.is_some())
                        .map(|c| format!("@{}", c.name.as_deref().unwrap_or_default()))
                })
                .flatten()
        };
        let preview = |core: &Core, to: &str| {
            core.send(Command::ResolveName { name: to.into() });
            wait_for(core, 3, |e| match e {
                Event::NameResolved { name, result } if name == to => Some(result.clone()),
                _ => None,
            })
            .expect("the contact's name resolves");
            core.send(Command::PreviewPayment { to: to.into() });
            wait_for(core, 3, |e| match e {
                Event::PaymentPreview { to: asked, result } if asked == to => Some(result.clone()),
                _ => None,
            })
            .expect("the address the payment goes to")
        };

        let Ok(text) = std::fs::read(&paid_file) else {
            // The first run: a fresh copy of the warm wallet pays the contact.
            let core = warm_wallet(name);
            let to = wait_for(&core, 60, |e| match e {
                Event::Snapshot(s) => contact_of(s),
                _ => None,
            });
            let first = preview(&core, &to);
            println!("{to}: paying {first}");
            core.send(spend(Spend::Send {
                from: evolite_api::Source::Core,
                to: to.clone(),
                amount: 10_000,
            }));
            let sent = wait_for(&core, 3, |e| match e {
                Event::Sent(r) => Some(r.clone()),
                _ => None,
            })
            .expect("payment to the contact signed and broadcast");
            println!("sent {} relayed {}", sent.txid, sent.relayed);
            std::fs::write(&paid_file, serde_json::to_vec(&(to, first)).unwrap()).unwrap();
            println!("first run done; run the test again for the restart");
            // The core stops its client and lets go of the store once its channel closes.
            drop(core);
            std::thread::sleep(Duration::from_secs(10));
            return;
        };
        let (to, first): (String, String) = serde_json::from_slice(&text).unwrap();
        let _ = std::fs::remove_file(&paid_file);

        // The second run: the same folder, opened with the password.
        let core = Core::real(dir, ArtOptions::default(), || {});
        core.send(Command::Unlock {
            password: Password::new(WARM_PASSWORD.into()),
        });
        wait_for(&core, 5, |e| match e {
            Event::Done {
                what: "unlock",
                result,
            } => Some(result.clone()),
            _ => None,
        })
        .expect("the wallet opens again");
        wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) => contact_of(s).filter(|c| *c == to),
            _ => None,
        });
        let next = preview(&core, &to);
        println!("{to} after the restart: {next}");
        assert_ne!(
            next, first,
            "the address paid before the restart is not paid again"
        );
    }

    /// "Sync again" on testnet, over two runs as two starts of the app. The
    /// first: a copy of the synced test wallet notes its balance and history and asks for the
    /// resync. The second, on the same folder: the headers, filters and store are gone before
    /// anything opens; the wallet is scanned again from its birth and comes back with the same
    /// balance and history. Spends nothing; the second run takes minutes (a whole rescan).
    /// `cargo test -p evolite-core live_resync -- --ignored --nocapture`, twice.
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_resync_comes_back_with_the_same_wallet() {
        let name = "evolite-live-resync";
        let dir = std::env::temp_dir().join(name);
        let noted = dir.join("before-resync.json");
        let synced = |core: &Core, minutes| {
            wait_for(core, minutes, |e| match e {
                Event::Snapshot(s) if s.sync.stage == SyncStage::Done => s
                    .wallet
                    .as_ref()
                    .map(|w| (w.balances.core, w.activity.len())),
                Event::Snapshot(s) => {
                    assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                    None
                }
                _ => None,
            })
        };
        if !noted.exists() {
            let core = warm_wallet(name);
            let before = synced(&core, 60);
            println!("before: {} duffs, {} operations", before.0, before.1);
            core.send(Command::Resync);
            wait_for(&core, 2, |e| match e {
                Event::Done {
                    what: "resync",
                    result,
                } => Some(result.clone()),
                _ => None,
            })
            .expect("resync asked");
            std::fs::write(&noted, serde_json::to_vec(&before).unwrap()).unwrap();
            println!("first run done; run the test again for the next start");
            return;
        }
        let before: (i64, usize) = serde_json::from_slice(&std::fs::read(&noted).unwrap()).unwrap();
        std::fs::remove_file(&noted).unwrap();
        let started = Instant::now();
        let core = Core::real(dir.clone(), ArtOptions::default(), || {});
        core.send(Command::Unlock {
            password: Password::new(WARM_PASSWORD.into()),
        });
        wait_for(&core, 5, |e| match e {
            Event::Done {
                what: "unlock",
                result,
            } => Some(result.clone()),
            _ => None,
        })
        .expect("the wallet opens after the resync");
        let wallets = wallet_folder(&dir, "testnet").expect("the wallet's folder");
        assert!(!wallets.join("resync-on-start").exists(), "done once");
        let after = synced(&core, 60);
        println!(
            "after the resync: {} duffs, {} operations, in {} s",
            after.0,
            after.1,
            started.elapsed().as_secs()
        );
        assert_eq!(after, before, "the same wallet back from the chain");
    }

    /// BUG-57 on the warm copy (the test phrase restored, many identities made by asset
    /// locks): each lock line says whether it made the identity or topped it up, and credits
    /// are «not added» exactly when Identity offers to continue; after locking and unlocking
    /// (the chain started again) the lines say the same. Spends nothing.
    /// `cargo t live_asset_locks_say -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_asset_locks_say_what_they_did() {
        use evolite_api::{ActivityKind, Credits};
        type Line = (String, bool, bool, Credits);
        let lines = |core: &Core| -> (Vec<Line>, bool) {
            wait_for(core, 10, |e| match e {
                Event::Snapshot(s) => {
                    let w = s.wallet.as_ref()?;
                    (s.sync.stage == SyncStage::Done
                        && w.identity_search == evolite_api::IdentitySearch::Done
                        && !w.identities.is_empty())
                    .then_some(())
                }
                _ => None,
            });
            // The history and the locks' facts settle a little after the lookup.
            let settle = Instant::now() + Duration::from_secs(20);
            let mut last = None;
            while Instant::now() < settle {
                for event in core.events() {
                    if let Event::Snapshot(s) = event
                        && let Some(w) = s.wallet
                    {
                        last = Some(w);
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            let w = last.expect("a snapshot");
            let mut out: Vec<Line> = w
                .activity
                .iter()
                .filter_map(|a| match &a.kind {
                    ActivityKind::TopUp {
                        identity,
                        creates,
                        credits,
                    } => Some((
                        a.txid.clone().unwrap_or_default(),
                        !identity.is_empty(),
                        *creates,
                        *credits,
                    )),
                    _ => None,
                })
                .collect();
            out.sort_by(|a, b| a.0.cmp(&b.0));
            let waits = w.unfinished_top_up.is_some_and(|u| !u.awaiting_chain_lock)
                || w.unfinished_identity.is_some_and(|u| u.locked.is_some());
            (out, waits)
        };
        let count = |l: &[Line], f: &dyn Fn(&Line) -> bool| l.iter().filter(|x| f(x)).count();
        let summary = |l: &[Line]| {
            println!(
                "{} locks: {} creations ({} named, {} added), {} top-ups ({} named, {} added), {} not added, {} unknown",
                l.len(),
                count(l, &|x| x.2),
                count(l, &|x| x.2 && x.1),
                count(l, &|x| x.2 && x.3 == Credits::Added),
                count(l, &|x| !x.2),
                count(l, &|x| !x.2 && x.1),
                count(l, &|x| !x.2 && x.3 == Credits::Added),
                count(l, &|x| x.3 == Credits::NotAdded),
                count(l, &|x| x.3 == Credits::Unknown),
            );
        };
        let core = warm_wallet("evolite-live-locks");
        let (first, waits) = lines(&core);
        summary(&first);
        assert!(
            first.iter().any(|x| x.1 && x.2 && x.3 == Credits::Added),
            "an identity made by this phrase is its creation, named and added"
        );
        assert_eq!(
            first.iter().any(|x| x.3 == Credits::NotAdded),
            waits,
            "«not added» exactly when Identity offers to continue"
        );
        core.send(Command::Lock);
        std::thread::sleep(Duration::from_secs(3));
        core.send(Command::Unlock {
            password: Password::new(WARM_PASSWORD.into()),
        });
        let (again, _) = lines(&core);
        summary(&again);
        assert_eq!(again, first, "the same after the chain started again");
    }

    /// A top-up on testnet: 0.001 DASH from Core goes into the first identity's credits by an
    /// asset lock; the credits grow by about that, Core shrinks by it and the fee, and nothing
    /// is left unfinished. Spends about 0.001 tDASH.
    /// `cargo test -p evolite-core live_top_up -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_top_up_identity() {
        let core = warm_wallet("evolite-live-top-up");
        let ready = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done
                    && w.identity.is_some()
                    && w.identity_search == evolite_api::IdentitySearch::Done)
                    .then(|| w.clone())
            }
            _ => None,
        });
        assert_eq!(ready.unfinished_top_up, None, "a top-up left from before");
        // Right after the lookup the core reads the balances from Platform: the store of the
        // warm copy may be behind. The last snapshot of the next 20 s is the one to start from.
        let settle = Instant::now() + Duration::from_secs(20);
        let mut ready = ready;
        while Instant::now() < settle {
            for event in core.events() {
                if let Event::Snapshot(s) = event
                    && let Some(w) = s.wallet
                {
                    ready = w;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let amount = 100_000;
        println!(
            "before: credits {} duffs, core {} duffs",
            ready.balances.credits, ready.balances.core
        );
        let started = Instant::now();
        core.send(spend(Spend::TopUpIdentity { amount }));
        let done = wait_for(&core, 15, |e| match e {
            Event::Done {
                what: "top-up",
                result,
            } => Some(result.clone()),
            _ => None,
        });
        println!("top-up: {done:?} in {}s", started.elapsed().as_secs());
        done.expect("credits topped up");
        let after = wait_for(&core, 3, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (w.balances.credits > ready.balances.credits).then(|| w.clone())
            }
            _ => None,
        });
        println!(
            "after: credits {} duffs, core {} duffs",
            after.balances.credits, after.balances.core
        );
        // Platform keeps a fee for processing the top-up: 8–12 % of 0.001 DASH on testnet.
        let added = after.balances.credits - ready.balances.credits;
        println!("added {added} of {amount} duffs");
        assert!(added >= amount * 8 / 10 && added <= amount, "{added}");
        assert!(after.balances.core <= ready.balances.core - amount);
        assert_eq!(after.unfinished_top_up, None);
    }

    /// Credits on testnet: the first identity sends 0.0001 DASH of credits to the wallet's own
    /// Platform address and 0.0001 DASH to `EVOLITE_TEST_NAME_2` (an identity of the same phrase,
    /// made by `live_identity_found_and_created`), and the Platform address balance grows.
    /// `cargo test -p evolite-core live_credits -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_credits_move_to_an_address_and_a_name() {
        let core = warm_wallet("evolite-live-credits");
        let ready = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done
                    && w.identity.is_some()
                    && w.platform_address.starts_with("tdash1"))
                .then(|| w.clone())
            }
            _ => None,
        });
        // One sync pass of the Platform addresses (every 30 s) before reading their balance.
        let before = watch_platform(&core, 40).unwrap_or(ready.balances.platform_address);
        println!(
            "credits {} duffs, Platform address {} with {} duffs",
            ready.balances.credits, ready.platform_address, before
        );
        assert!(
            ready.balances.credits > 30_000,
            "the identity needs credits"
        );

        let second = format!("@{}", test_name("EVOLITE_TEST_NAME_2"));
        for to in [ready.platform_address.clone(), second] {
            core.send(spend(Spend::Send {
                from: evolite_api::Source::Credits,
                to: to.clone(),
                amount: 10_000,
            }));
            let sent = wait_for(&core, 3, |e| match e {
                Event::Sent(r) => Some(r.clone()),
                _ => None,
            });
            println!("credits to {to}: {sent:?}");
            sent.expect("credits sent");
        }
        let after = watch_platform(&core, 90).expect("Platform address synced");
        println!("Platform address now {after} duffs");
        assert!(after >= before + 10_000, "{before} -> {after}");
    }

    /// BUG-10 on testnet: the Collection of the identity the wallet shows comes out of
    /// "Loading" — the list, or a failure with its reason — within the time an ask may take.
    /// Reads only.
    /// `cargo test -p evolite-core live_collection_loads -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and the test phrase in .env.test.local"]
    fn live_collection_loads_or_says_why() {
        let core = warm_wallet("evolite-live-collection");
        let started = Instant::now();
        let (nfts, status) = wait_for(&core, 5, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (w.identity.is_some() && w.nfts_status != evolite_api::NftsStatus::Loading)
                    .then(|| (w.nfts.len(), w.nfts_status.clone()))
            }
            _ => None,
        });
        println!(
            "after {} s: {status:?}, {nfts} NFTs",
            started.elapsed().as_secs()
        );
        assert!(
            started.elapsed() < Duration::from_secs(4 * 60),
            "out of Loading only after {:?}",
            started.elapsed()
        );
    }

    /// DashPay on testnet: the first identity writes its profile (with a picture by https) and
    /// sends a contact request to `EVOLITE_TEST_NAME_2`, an identity of the same phrase made by
    /// `live_identity_found_and_created`; the profile, its picture and the request show in the
    /// snapshot. Two documents: a few thousandths of a DASH in credits.
    /// `cargo test -p evolite-core live_dashpay -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_dashpay_profile_and_request() {
        let core = warm_wallet("evolite-live-dashpay");
        let ready = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done && w.identity.is_some()).then(|| w.clone())
            }
            _ => None,
        });
        let me = ready.identity.as_ref().unwrap();
        println!(
            "identity {} profile {:?}, {} contacts",
            me.id,
            me.profile,
            ready.contacts.len()
        );

        let display = "evolite probe";
        core.send(spend(Spend::SaveProfile {
            display: display.into(),
            about: "Profile from an evolite live test".into(),
            avatar_url: Some("https://github.com/dashpay.png?size=96".into()),
        }));
        let saved = wait_for(&core, 5, |e| match e {
            Event::Done {
                what: "profile",
                result,
            } => Some(result.clone()),
            _ => None,
        });
        println!("profile: {saved:?}");
        saved.expect("profile written");
        let profile = wait_for(&core, 3, |e| match e {
            Event::Snapshot(s) => s
                .wallet
                .as_ref()?
                .identity
                .as_ref()?
                .profile
                .clone()
                .filter(|p| p.display.as_deref() == Some(display) && p.avatar.is_some()),
            _ => None,
        });
        println!("profile in the snapshot: {profile:?}");

        core.send(spend(Spend::AddContact {
            name: format!("@{}", test_name("EVOLITE_TEST_NAME_2")),
        }));
        let sent = wait_for(&core, 5, |e| match e {
            Event::Done {
                what: "contact",
                result,
            } => Some(result.clone()),
            _ => None,
        });
        println!("contact request: {sent:?}");
        sent.expect("request sent");
        let contact = wait_for(&core, 3, |e| match e {
            Event::Snapshot(s) => s
                .wallet
                .as_ref()?
                .contacts
                .iter()
                .find(|c| {
                    c.name.as_deref() == Some(test_name("EVOLITE_TEST_NAME_2").as_str())
                        && c.state != evolite_api::ContactState::Incoming
                })
                .cloned(),
            _ => None,
        });
        println!("contact in the snapshot: {contact:?}");
    }

    /// BUG-13 on testnet: a contact request between two named identities of the test phrase
    /// with none between them. As A it is sent, and B is "sent" in the snapshot at once; as B,
    /// shown straight after, A's request comes with no other action (each DashPay pass makes the
    /// snapshot again); back as A — "Act as" empties the snapshot's contacts — the same request
    /// again is refused and costs nothing. One request: about 0.001 DASH of A's credits.
    /// `cargo test -p evolite-core live_request_between_own -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local; spends credits"]
    fn live_request_between_own_identities_is_seen() {
        use evolite_api::{ContactState, CoreError, IdentitySearch};
        let core = warm_wallet("evolite-live-own-request");
        let wallet = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done
                    && w.identity_search == IdentitySearch::Done
                    && w.identities.len() >= 2)
                    .then(|| w.clone())
            }
            _ => None,
        });
        // A DashPay pass or two first: what Platform has, not only the store.
        let _ = watch_last(&core, 45);
        // Each named identity as worked as: its credits and everyone it has a request with.
        struct Seen {
            id: String,
            name: String,
            credits: i64,
            ties: Vec<String>,
        }
        let shown = |core: &Core, id: &str| {
            core.send(Command::ChooseIdentity { id: id.to_owned() });
            wait_for(core, 3, |e| match e {
                Event::Snapshot(s) => {
                    let w = s.wallet.as_ref()?;
                    (w.identity.as_ref()?.id == id).then(|| {
                        (
                            w.balances.credits,
                            w.contacts.iter().map(|c| c.identity_id.clone()).collect(),
                        )
                    })
                }
                _ => None,
            })
        };
        let mut seen = Vec::new();
        for choice in &wallet.identities {
            let Some(name) = choice.name.clone() else {
                continue;
            };
            let (credits, ties): (i64, Vec<String>) = shown(&core, &choice.id);
            println!("@{name}: {credits} credits, {} contacts", ties.len());
            seen.push(Seen {
                id: choice.id.clone(),
                name,
                credits,
                ties,
            });
        }
        let a = seen
            .iter()
            .max_by_key(|s| s.credits)
            .expect("a named identity");
        assert!(
            a.credits >= 120_000,
            "@{} has {} credits: a request costs about 102 000",
            a.name,
            a.credits
        );
        let b = seen
            .iter()
            .find(|s| s.id != a.id && !a.ties.contains(&s.id) && !s.ties.contains(&a.id))
            .expect("two named identities with no request between them");
        println!("A @{} ({} credits) -> B @{}", a.name, a.credits, b.name);

        // As A: the request, and B "sent" in the snapshot right after its answer.
        let (before, _) = shown(&core, &a.id);
        core.send(spend(Spend::AddContact {
            name: format!("@{}", b.name),
        }));
        let started = Instant::now();
        let sent = wait_for(&core, 5, |e| match e {
            Event::Done {
                what: "contact",
                result,
            } => Some(result.clone()),
            _ => None,
        });
        println!("request: {sent:?} after {} s", started.elapsed().as_secs());
        sent.expect("request sent");
        let after = wait_for(&core, 1, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (w.identity.as_ref()?.id == a.id
                    && w.contacts
                        .iter()
                        .any(|c| c.identity_id == b.id && c.state == ContactState::Outgoing))
                .then_some(w.balances.credits)
            }
            _ => None,
        });
        println!(
            "as A, B is sent after {} s; A's credits {before} -> {after} (spent {})",
            started.elapsed().as_secs(),
            before - after
        );

        // As B at once: A's request comes by itself, without another command.
        core.send(Command::ChooseIdentity { id: b.id.clone() });
        let switched = Instant::now();
        let mut first = None;
        wait_for(&core, 3, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                if w.identity.as_ref()?.id != b.id {
                    return None;
                }
                let incoming = w
                    .contacts
                    .iter()
                    .any(|c| c.identity_id == a.id && c.state == ContactState::Incoming);
                first.get_or_insert(incoming);
                incoming.then_some(())
            }
            _ => None,
        });
        println!(
            "as B, A's request after {} s (in B's first snapshot: {first:?})",
            switched.elapsed().as_secs()
        );

        // Back as A, the request again at once: refused by what platform-wallet has, free.
        core.send(Command::ChooseIdentity { id: a.id.clone() });
        core.send(spend(Spend::AddContact {
            name: format!("@{}", b.name),
        }));
        let again = wait_for(&core, 5, |e| match e {
            Event::Done {
                what: "contact",
                result,
            } => Some(result.clone()),
            _ => None,
        });
        println!("again: {again:?}");
        assert!(
            matches!(again, Err(CoreError::NotAvailable(_))),
            "{again:?}"
        );
        let last = watch_last(&core, 20)
            .and_then(|s| s.wallet)
            .filter(|w| w.identity.as_ref().is_some_and(|i| i.id == a.id))
            .map(|w| w.balances.credits);
        println!("A's credits at the end: {last:?}");
        assert_eq!(last, Some(after), "the refused repeat spent nothing");
    }

    /// What a run of the app saw once synced: written by the first run of
    /// `live_restart_keeps_history`, compared by the second.
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    struct RunStats {
        balance: i64,
        operations: usize,
        tip: u32,
        seconds_to_synced: u64,
    }

    /// A real wallet for screenshots of the app (`evolite --open <screen> --data-dir …`):
    /// restores a phrase into `EVOLITE_SHOT_DIR` under `EVOLITE_SHOT_PASSWORD`, as the
    /// "Restore" screen does, and waits for the sync and the identity search. The phrase is
    /// read from `EVOLITE_SHOT_PHRASE_FILE` — the first line of a file outside the repository, or
    /// `EVOLITE_TEST_PHRASE` of an `.env.test.local` — into memory only; it is never printed.
    /// `cargo t shots_restore -- --ignored --nocapture`
    #[test]
    #[ignore = "makes a wallet for screenshots: needs testnet and the EVOLITE_SHOT_* variables"]
    fn shots_restore_wallet() {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} is not set"));
        let dir = std::path::PathBuf::from(var("EVOLITE_SHOT_DIR"));
        let file = var("EVOLITE_SHOT_PHRASE_FILE");
        let text =
            zeroize::Zeroizing::new(std::fs::read_to_string(&file).expect("the phrase file"));
        let phrase = zeroize::Zeroizing::new(
            text.lines()
                .find_map(|l| l.trim().strip_prefix("EVOLITE_TEST_PHRASE="))
                .map(|v| v.trim().trim_matches('"').to_owned())
                .unwrap_or_else(|| text.lines().next().unwrap_or("").trim().to_owned()),
        );
        assert!(
            phrase.split_whitespace().count() >= 12,
            "no phrase in the file"
        );
        assert!(
            wallet_folder(&dir, "testnet").is_none(),
            "a wallet is there already"
        );
        let core = Core::real(dir, ArtOptions::default(), || {});
        let started = Instant::now();
        core.send(Command::UsePhrase {
            phrase: evolite_api::Phrase::new(
                phrase.split_whitespace().map(str::to_owned).collect(),
            ),
            dashpay: false,
        });
        drop(phrase);
        drop(text);
        core.send(Command::CreateWallet {
            password: Password::new(var("EVOLITE_SHOT_PASSWORD")),
        });
        let s = wait_for(&core, 60, |e| match e {
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                (s.sync.stage == SyncStage::Done && s.wallet.is_some()).then(|| (**s).clone())
            }
            _ => None,
        });
        // A minute more: identities, names, contacts and balances come after the sync.
        let s = watch_last(&core, 60).unwrap_or(s);
        let w = s.wallet.unwrap();
        println!(
            "restored in {} s: {} operations, identity {:?}, {} contacts",
            started.elapsed().as_secs(),
            w.activity.len(),
            w.identity.as_ref().and_then(|i| i.name()),
            w.contacts.len()
        );
        drop(core);
        std::thread::sleep(Duration::from_secs(10));
    }

    /// Settings on testnet with the warm test wallet: the folder's
    /// size; one receive address with "A new address…" off — the wallet's first — and the next
    /// unused one again with it on; the price of DASH from the rate service; then "Own node":
    /// a bad address refused, a testnet node from the DNS seed saved and forgotten again. The
    /// start with that node is `live_own_node_alone`. Spends
    /// nothing. `cargo test -p evolite-core live_settings -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_settings_reach_the_core() {
        let name = "evolite-live-settings";
        let core = warm_wallet(name);
        let synced = |s: &evolite_api::Snapshot| {
            s.sync.stage == SyncStage::Done && s.wallet.is_some() && s.sync.chainlocked
        };
        let s = wait_for(&core, 30, |e| match e {
            Event::Snapshot(s) if synced(s) => Some((**s).clone()),
            _ => None,
        });
        let disk = s.sync.disk_bytes.expect("the folder is counted");
        println!("data folder {} MB", disk / (1024 * 1024));
        assert!(disk > 10 << 20, "headers and filters take more than 10 MB");

        let fresh = s.wallet.as_ref().unwrap().core_address.clone();
        core.send(Command::ReceiveAddress { fresh: false });
        let first = wait_for(&core, 1, |e| match e {
            Event::Snapshot(s) => s
                .wallet
                .as_ref()
                .map(|w| w.core_address.clone())
                .filter(|a| *a != fresh && !a.is_empty()),
            _ => None,
        });
        println!("next unused {fresh}, the first {first}");
        assert!(first.starts_with('y'));
        // The first address is the same from another derivation of the same phrase.
        let phrase = test_env("EVOLITE_TEST_PHRASE").unwrap();
        let wallet = real::keys_of(&phrase).unwrap();
        let path: key_wallet::bip32::DerivationPath = "m/44'/1'/0'/0/0".parse().unwrap();
        let public = key_wallet::dashcore::PublicKey::new(wallet.derive_public_key(&path).unwrap());
        let expected = key_wallet::dashcore::Address::p2pkh(&public, key_wallet::Network::Testnet);
        assert_eq!(first, expected.to_string());
        core.send(Command::ReceiveAddress { fresh: true });
        wait_for(&core, 1, |e| match e {
            Event::Snapshot(s) => s
                .wallet
                .as_ref()
                .filter(|w| w.core_address == fresh)
                .map(|_| ()),
            _ => None,
        });

        core.send(Command::FiatRate {
            currency: Some("USD".into()),
        });
        let rate = wait_for(&core, 2, |e| match e {
            Event::Snapshot(s) => s.fiat.clone(),
            _ => None,
        });
        println!(
            "rate: 1 DASH ={}.{:08} {}",
            rate.price / 100_000_000,
            rate.price % 100_000_000,
            rate.currency
        );
        assert_eq!(rate.currency, "USD");
        assert!(rate.price > 100_000_000, "more than a dollar");
        core.send(Command::FiatRate { currency: None });

        let node_saved = |core: &Core| {
            wait_for(core, 1, |e| match e {
                Event::Done {
                    what: "core-node",
                    result,
                } => Some(result.clone()),
                _ => None,
            })
        };
        core.send(Command::CoreNode {
            address: Some("https://example.org".into()),
        });
        assert!(node_saved(&core).is_err(), "a link is not a node");
        use std::net::ToSocketAddrs;
        let node = ("testnet-seed.dashdot.io", 19999)
            .to_socket_addrs()
            .expect("the testnet DNS seed answers")
            .find(|a| a.is_ipv4())
            .expect("an IPv4 node");
        let node = node.to_string();
        core.send(Command::CoreNode {
            address: Some(node.clone()),
        });
        node_saved(&core).expect("the node is saved");
        let next = wait_for(&core, 1, |e| match e {
            Event::Snapshot(s) => s.sync.core_node_next.clone(),
            _ => None,
        });
        assert_eq!(next, node);
        core.send(Command::CoreNode { address: None });
        node_saved(&core).expect("forgotten");
        wait_for(&core, 1, |e| match e {
            Event::Snapshot(s) => s.sync.core_node_next.is_none().then_some(()),
            _ => None,
        });
    }

    /// "Own node" at the start, in its own process: a store stays held by the process
    /// that opened it, so a second core in the same test cannot stand for a restart. The
    /// warm wallet's copy with `core-node.txt` naming a testnet node from the DNS seed: the core
    /// connects to that node alone, syncs to a ChainLocked tip through it and takes nobody else
    /// for a minute more; "off" is kept for the next start.
    /// `cargo test -p evolite-core live_own_node -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_own_node_alone() {
        let synced = |s: &evolite_api::Snapshot| {
            s.sync.stage == SyncStage::Done && s.wallet.is_some() && s.sync.chainlocked
        };
        let node_saved = |core: &Core| {
            wait_for(core, 1, |e| match e {
                Event::Done {
                    what: "core-node",
                    result,
                } => Some(result.clone()),
                _ => None,
            })
        };
        use std::net::ToSocketAddrs;
        let node = ("testnet-seed.dashdot.io", 19999)
            .to_socket_addrs()
            .expect("the testnet DNS seed answers")
            .find(|a| a.is_ipv4())
            .expect("an IPv4 node")
            .to_string();
        let warm = std::env::temp_dir().join("evolite-live-warm");
        assert!(
            wallet_folder(&warm, "testnet").is_some(),
            "run live_settings first: it makes the warm wallet"
        );
        let dir = std::env::temp_dir().join("evolite-live-own-node");
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&warm, &dir).expect("copy of the warm wallet");
        std::fs::write(dir.join("core-node.txt"), format!("{node}\n")).unwrap();
        let core = Core::real(dir, ArtOptions::default(), || {});
        core.send(Command::Unlock {
            password: Password::new(WARM_PASSWORD.into()),
        });
        let started = Instant::now();
        let mut most_peers = 0;
        let mut said = Instant::now();
        let s = wait_for(&core, 15, |e| match e {
            Event::Snapshot(s) => {
                most_peers = most_peers.max(s.sync.core_peers);
                if said.elapsed() > Duration::from_secs(30) {
                    said = Instant::now();
                    println!(
                        "  {} s: {:?} {:.0} %, headers {}/{}, filters {}, peers {}, chainlocked {}, error {:?}",
                        started.elapsed().as_secs(),
                        s.sync.stage,
                        s.sync.progress.unwrap_or(0.0) * 100.0,
                        s.sync.headers_height,
                        s.sync.tip_height,
                        s.sync.filters_height,
                        s.sync.core_peers,
                        s.sync.chainlocked,
                        s.sync.error
                    );
                }
                assert_eq!(s.sync.core_node.as_deref(), Some(node.as_str()));
                assert!(
                    s.sync.core_node_next.is_none(),
                    "this run uses the saved one"
                );
                (synced(s) && s.sync.core_peers == 1).then(|| (**s).clone())
            }
            _ => None,
        });
        println!(
            "own node: synced through {node} alone in {} s, block {}, peers at most {most_peers}",
            started.elapsed().as_secs(),
            s.sync.tip_height
        );
        assert_eq!(most_peers, 1, "no other node");
        // A minute more on that node: still nobody else.
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until {
            for e in core.events() {
                if let Event::Snapshot(s) = e {
                    assert!(s.sync.core_peers <= 1, "{} peers", s.sync.core_peers);
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        core.send(Command::CoreNode { address: None });
        node_saved(&core).expect("back to DNS seeds");
        let next = wait_for(&core, 1, |e| match e {
            Event::Snapshot(s) => s.sync.core_node_next.clone(),
            _ => None,
        });
        assert_eq!(next, "", "DNS seeds from the next start");
    }

    /// How long the shielded pool takes to connect after the unlock — "Connecting the pool…"
    /// the while, then its address. Reads only (the viewing keys are kept in the store).
    /// Best measured in a release build:
    /// `cargo t --release live_pool_connect -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_pool_connect_time() {
        let core = warm_wallet("evolite-live-pool");
        let unlocked = Instant::now();
        let mut connecting_seen = false;
        let mut synced_at = None;
        let s = wait_for(&core, 20, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                connecting_seen |= w.pool.connecting;
                if synced_at.is_none() && s.sync.stage == SyncStage::Done {
                    synced_at = Some(unlocked.elapsed().as_secs());
                }
                w.pool.address.is_some().then(|| (**s).clone())
            }
            _ => None,
        });
        let w = s.wallet.unwrap();
        println!(
            "pool: synced in {:?} s, pool connected {} s after the unlock (release: {}), {} notes",
            synced_at,
            unlocked.elapsed().as_secs(),
            !cfg!(debug_assertions),
            w.pool.notes
        );
        assert!(connecting_seen, "\"Connecting the pool…\" was shown before");
        assert!(!w.pool.connecting);
    }

    /// Mainnet behind `--network mainnet`: the real core on mainnet with no wallet, only the
    /// start and the header sync — nothing is made, signed or sent. Its files go to
    /// `spv\mainnet`, never next to testnet's; headers come from mainnet nodes and are checked
    /// as on testnet. Must run alone (the network is chosen once per process):
    /// `cargo t live_mainnet -- --ignored --nocapture`
    #[test]
    #[ignore = "needs mainnet access; run alone, it chooses the network of the process"]
    fn live_mainnet_starts_and_syncs_headers() {
        let dir = std::env::temp_dir().join("evolite-live-mainnet");
        let core = Core::real_on(
            evolite_api::Network::Mainnet,
            dir.clone(),
            ArtOptions::default(),
            || {},
        )
        .expect("run this test alone: another test chose testnet for the process");
        let started = Instant::now();
        let mut first: Option<u32> = None;
        let mut said = Instant::now();
        let s = wait_for(&core, 15, |e| match e {
            Event::Snapshot(s) => {
                assert_eq!(s.network, evolite_api::Network::Mainnet);
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                assert_eq!(s.vault, VaultState::NoWallet, "no wallet is made");
                if s.sync.headers_height > 0 && first.is_none() {
                    first = Some(s.sync.headers_height);
                }
                if said.elapsed() > Duration::from_secs(30) {
                    said = Instant::now();
                    println!(
                        "  {} s: {:?}, headers {}/{}, peers {}",
                        started.elapsed().as_secs(),
                        s.sync.stage,
                        s.sync.headers_height,
                        s.sync.tip_height,
                        s.sync.core_peers
                    );
                }
                // Real mainnet headers: past block 2 300 000 (October 2025), and moving.
                let moved = first.is_some_and(|f| s.sync.headers_height > f + 1000);
                (s.sync.core_peers > 0 && s.sync.tip_height > 2_300_000 && moved)
                    .then(|| (**s).clone())
            }
            _ => None,
        });
        println!(
            "mainnet: {} peers, headers {} of {} after {} s",
            s.sync.core_peers,
            s.sync.headers_height,
            s.sync.tip_height,
            started.elapsed().as_secs()
        );
        assert!(
            dir.join("spv").join("mainnet").exists(),
            "mainnet's own folder"
        );
        assert!(
            !dir.join("spv").join("testnet").exists(),
            "nothing of testnet's"
        );
        assert!(wallet_folder(&dir, "mainnet").is_none());
    }

    /// A restart of the app: the wallet comes back from the store with its whole history at
    /// once, and syncs only what is new, not the chain from its start. Run it twice, as two
    /// processes are two runs of the app: the first restores the test phrase on a fresh folder
    /// and syncs; the second unlocks the same folder and compares.
    /// `cargo test -p evolite-core live_restart -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_restart_keeps_history() {
        let dir = std::env::temp_dir().join("evolite-live-restart");
        let stats_file = dir.join("restart-stats.json");
        let password = || Password::new("live restart test".into());
        let synced =
            |s: &evolite_api::Snapshot| s.sync.stage == SyncStage::Done && s.wallet.is_some();

        let Ok(text) = std::fs::read(&stats_file) else {
            // The first run: restore and sync from the start of the chain.
            let _ = std::fs::remove_dir_all(&dir);
            let phrase =
                test_env("EVOLITE_TEST_PHRASE").expect("EVOLITE_TEST_PHRASE in .env.test.local");
            let core = Core::real(dir.clone(), ArtOptions::default(), || {});
            let started = Instant::now();
            let words = phrase.split_whitespace().map(str::to_owned).collect();
            core.send(Command::UsePhrase {
                phrase: evolite_api::Phrase::new(words),
                dashpay: false,
            });
            core.send(Command::CreateWallet {
                password: password(),
            });
            let s = wait_for(&core, 60, |e| match e {
                Event::Snapshot(s) if synced(s) => Some((**s).clone()),
                _ => None,
            });
            // A minute more: payments seen while the filters were checked settle into history.
            let s = watch_last(&core, 60).filter(|s| synced(s)).unwrap_or(s);
            let w = s.wallet.unwrap();
            let stats = RunStats {
                balance: w.balances.core,
                operations: w.activity.len(),
                tip: s.sync.tip_height,
                seconds_to_synced: started.elapsed().as_secs(),
            };
            println!("first run: {stats:?}; run the test again for the restart");
            std::fs::write(&stats_file, serde_json::to_vec(&stats).unwrap()).unwrap();
            return;
        };
        let before: RunStats = serde_json::from_slice(&text).unwrap();
        let _ = std::fs::remove_file(&stats_file);

        // The second run: the same folder, opened with the password.
        let core = Core::real(dir.clone(), ArtOptions::default(), || {});
        let started = Instant::now();
        let locked = wait_for(&core, 2, |e| match e {
            Event::Snapshot(s) => Some(s.vault),
            _ => None,
        });
        assert_eq!(locked, VaultState::Locked, "the wallet is found on disk");
        core.send(Command::Unlock {
            password: password(),
        });
        // The history before any sync: from the store, at once.
        let first = wait_for(&core, 2, |e| match e {
            Event::Snapshot(s) => s
                .wallet
                .as_ref()
                .map(|w| (w.activity.len(), w.balances.core)),
            _ => None,
        });
        let first_secs = started.elapsed().as_secs();
        let s = wait_for(&core, 30, |e| match e {
            Event::Snapshot(s) if synced(s) => Some(s.clone()),
            _ => None,
        });
        let w = s.wallet.as_ref().unwrap();
        let after = RunStats {
            balance: w.balances.core,
            operations: w.activity.len(),
            tip: s.sync.tip_height,
            seconds_to_synced: started.elapsed().as_secs(),
        };
        println!("first run: {before:?}");
        println!(
            "restart: {} operations and {} duffs in the first snapshot after {first_secs}s; synced: {after:?}",
            first.0, first.1
        );
        assert_eq!(
            first.0, before.operations,
            "the history comes from the store at once"
        );
        assert_eq!(first.1, before.balance, "and so does the balance");
        assert!(after.operations >= before.operations);
        assert!(
            after.seconds_to_synced < before.seconds_to_synced,
            "a restart syncs only what is new"
        );
    }

    /// The last snapshot within `secs` seconds.
    fn watch_last(core: &Core, secs: u64) -> Option<evolite_api::Snapshot> {
        let until = Instant::now() + Duration::from_secs(secs);
        let mut last = None;
        while Instant::now() < until {
            for event in core.events() {
                if let Event::Snapshot(s) = event {
                    last = Some(*s);
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        last
    }

    /// The Platform address balance of the last snapshot within `secs` seconds.
    /// The shielded pool on testnet: the pool connects (its private address shows),
    /// 0.01 DASH of the first identity's credits go in, the pool's balance shows them after a
    /// scan, and 0.005 DASH come out to the wallet's own Platform address. Two Orchard proofs
    /// built here; about 0.01 DASH of fees.
    /// `cargo test -p evolite-core --features pool live_pool -- --ignored --nocapture`
    #[cfg(feature = "pool")]
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_pool_shield_and_unshield() {
        let core = warm_wallet("evolite-live-pool");
        let started = Instant::now();
        let ready = wait_for(&core, 30, |e| match e {
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                let w = s.wallet.as_ref()?;
                (w.identity.is_some() && w.pool.address.is_some()).then(|| w.clone())
            }
            _ => None,
        });
        println!(
            "pool connected after {} s: {} · {} duffs in it, credits {} duffs",
            started.elapsed().as_secs(),
            ready.pool.address.as_deref().unwrap_or_default(),
            ready.balances.shielded,
            ready.balances.credits
        );
        assert!(
            ready.balances.credits > 2_000_000,
            "the identity needs credits"
        );
        let before = ready.balances.shielded;

        let move_and_wait = |command: Command, what: &'static str| {
            let started = Instant::now();
            core.send(command);
            let result = wait_for(&core, 15, |e| match e {
                Event::Done { what: w, result } if *w == what => Some(result.clone()),
                _ => None,
            });
            println!("{what}: {result:?} in {} s", started.elapsed().as_secs());
            result.unwrap_or_else(|e| panic!("{what} failed: {e:?}"));
        };
        move_and_wait(
            spend(Spend::Shield {
                from: evolite_api::ShieldSource::Credits,
                amount: 1_000_000,
            }),
            "shield",
        );
        let shielded = wait_for(&core, 15, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (w.balances.shielded >= before + 1_000_000).then_some(w.balances.shielded)
            }
            _ => None,
        });
        println!("in the pool now: {shielded} duffs");

        move_and_wait(spend(Spend::Unshield { amount: 500_000 }), "unshield");
        let left = wait_for(&core, 15, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                // The change note: what is left of the note after the amount and the fee.
                (w.balances.shielded < shielded && w.balances.shielded > 0)
                    .then(|| (w.balances.shielded, w.pool.clone()))
            }
            _ => None,
        });
        println!(
            "after unshield: {} duffs, {} notes, {} operations",
            left.0,
            left.1.notes,
            left.1.operations.len()
        );
    }

    /// The shielded pool from a Platform address on testnet: 0.003 DASH of the wallet's
    /// Platform address credits go into the pool, signed with the address keys derived here.
    /// Needs credits on the Platform address (`live_pool_shield_and_unshield` leaves some).
    /// `cargo test -p evolite-core --features pool live_pool_from_address -- --ignored --nocapture`
    #[cfg(feature = "pool")]
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_pool_from_address() {
        let core = warm_wallet("evolite-live-pool-address");
        let ready = wait_for(&core, 30, |e| match e {
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                let w = s.wallet.as_ref()?;
                w.pool.address.as_ref()?;
                Some(w.clone())
            }
            _ => None,
        });
        // One pass of the Platform address sync (every 30 s) before reading their balance.
        let on_address = watch_platform(&core, 40).unwrap_or(ready.balances.platform_address);
        println!(
            "pool {} duffs, Platform address {} duffs",
            ready.balances.shielded, on_address
        );
        assert!(on_address > 400_000, "the Platform address needs credits");
        let before = ready.balances.shielded;
        let started = Instant::now();
        core.send(spend(Spend::Shield {
            from: evolite_api::ShieldSource::PlatformAddress,
            amount: 300_000,
        }));
        let result = wait_for(&core, 15, |e| match e {
            Event::Done {
                what: "shield",
                result,
            } => Some(result.clone()),
            _ => None,
        });
        println!(
            "shield from the address: {result:?} in {} s",
            started.elapsed().as_secs()
        );
        result.expect("shielded from the Platform address");
        let shielded = wait_for(&core, 15, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (w.balances.shielded >= before + 300_000).then_some(w.balances.shielded)
            }
            _ => None,
        });
        let left = watch_platform(&core, 40).unwrap_or_default();
        println!("in the pool now: {shielded} duffs; Platform address {left} duffs");
    }

    /// The shielded pool to Core on testnet: 0.003 DASH leave the pool for the wallet's
    /// own Core address; then, for up to 20 minutes, the payout is watched for (Platform pays
    /// withdrawals in batches, so it is printed, not required).
    /// `cargo test -p evolite-core --features pool live_pool_to_core -- --ignored --nocapture`
    #[cfg(feature = "pool")]
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_pool_to_core() {
        let core = warm_wallet("evolite-live-pool-core");
        let ready = wait_for(&core, 30, |e| match e {
            Event::Snapshot(s) => {
                assert!(s.sync.error.is_none(), "sync failed: {:?}", s.sync.error);
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done
                    && w.pool.address.is_some()
                    && w.balances.shielded > 0
                    && !w.core_address.is_empty())
                .then(|| w.clone())
            }
            _ => None,
        });
        println!(
            "pool {} duffs, Core {} duffs, to {}",
            ready.balances.shielded, ready.balances.core, ready.core_address
        );
        assert!(ready.balances.shielded > 500_000, "the pool needs funds");
        let started = Instant::now();
        core.send(spend(Spend::Send {
            from: evolite_api::Source::Pool,
            to: ready.core_address.clone(),
            amount: 300_000,
        }));
        let sent = wait_for(&core, 15, |e| match e {
            Event::Sent(r) => Some(r.clone()),
            _ => None,
        });
        println!(
            "pool to Core: {sent:?} in {} s",
            started.elapsed().as_secs()
        );
        sent.expect("withdrawn from the pool");
        let pool = wait_for(&core, 15, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (w.balances.shielded < ready.balances.shielded).then_some(w.balances.shielded)
            }
            _ => None,
        });
        println!("pool now {pool} duffs; watching Core for the payout");
        let until = Instant::now() + Duration::from_secs(20 * 60);
        let mut core_now = ready.balances.core;
        while Instant::now() < until && core_now <= ready.balances.core {
            for event in core.events() {
                if let Event::Snapshot(s) = event
                    && let Some(w) = &s.wallet
                {
                    core_now = w.balances.core;
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        println!(
            "Core {} -> {} duffs after {} s",
            ready.balances.core,
            core_now,
            started.elapsed().as_secs()
        );
    }

    /// The pool's fees on screen against what testnet charges: 0.003 DASH of credits go into the
    /// pool, then 0.001 DASH come out to the Platform address; the pool's change after each move
    /// is compared with the fee the snapshot shows (`Fees::shield`, `Fees::unshield`).
    /// `cargo test -p evolite-core --features pool live_pool_fees -- --ignored --nocapture`
    #[cfg(feature = "pool")]
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_pool_fees() {
        let core = warm_wallet("evolite-live-pool-fees");
        let ready = wait_for(&core, 30, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done
                    && w.identity.is_some()
                    && w.pool.checked_at.is_some())
                .then(|| w.clone())
            }
            _ => None,
        });
        let fees = ready.fees.clone();
        println!(
            "pool {} duffs; fees: shield {} unshield {} transfer {} withdrawal {}",
            ready.balances.shielded,
            fees.shield,
            fees.unshield,
            fees.pool_transfer,
            fees.pool_withdrawal
        );
        let step = |command: Command, what: &'static str| {
            core.send(command);
            let result = wait_for(&core, 15, |e| match e {
                Event::Done { what: w, result } if *w == what => Some(result.clone()),
                _ => None,
            });
            result.unwrap_or_else(|e| panic!("{what} failed: {e:?}"));
        };
        let pool_after = |before: i64| {
            wait_for(&core, 15, |e| match e {
                Event::Snapshot(s) => {
                    let w = s.wallet.as_ref()?;
                    (w.balances.shielded != before).then_some(w.balances.shielded)
                }
                _ => None,
            })
        };
        step(
            spend(Spend::Shield {
                from: evolite_api::ShieldSource::Credits,
                amount: 300_000,
            }),
            "shield",
        );
        let shielded = pool_after(ready.balances.shielded);
        println!(
            "pool after shield: {shielded} (+{})",
            shielded - ready.balances.shielded
        );
        step(spend(Spend::Unshield { amount: 100_000 }), "unshield");
        let left = pool_after(shielded);
        let charged = shielded - left - 100_000;
        println!(
            "pool after unshield: {left}; fee charged {charged}, shown {}",
            fees.unshield
        );
        assert_eq!(
            charged, fees.unshield,
            "the unshield fee on screen is what Platform takes"
        );
    }

    /// Core into the pool and the pool into the identity on testnet: 0.004 DASH of Core are
    /// locked and reach the pool less `Fees::shield_from_core`; then 0.001 DASH go from the pool
    /// into the identity's credits, the pool paying `Fees::pool_top_up` on top.
    /// `cargo test -p evolite-core --features pool live_pool_core_and_identity -- --ignored --nocapture`
    #[cfg(feature = "pool")]
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_pool_core_and_identity() {
        let core = warm_wallet("evolite-live-pool-in-out");
        let ready = wait_for(&core, 30, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done
                    && w.identity.is_some()
                    && w.pool.checked_at.is_some())
                .then(|| w.clone())
            }
            _ => None,
        });
        let fees = ready.fees.clone();
        println!(
            "pool {} Core {} credits {}; fees: from Core {} top-up {}; unfinished {:?}",
            ready.balances.shielded,
            ready.balances.core,
            ready.balances.credits,
            fees.shield_from_core,
            fees.pool_top_up,
            ready.pool.unfinished_shield
        );
        let step = |command: Command, what: &'static str| {
            let started = Instant::now();
            core.send(command);
            let result = wait_for(&core, 20, |e| match e {
                Event::Done { what: w, result } if *w == what => Some(result.clone()),
                _ => None,
            });
            println!("{what}: {result:?} in {} s", started.elapsed().as_secs());
            result.unwrap_or_else(|e| panic!("{what} failed: {e:?}"));
        };
        let pool_after = |before: i64| {
            wait_for(&core, 15, |e| match e {
                Event::Snapshot(s) => {
                    let w = s.wallet.as_ref()?;
                    (w.balances.shielded != before).then(|| w.clone())
                }
                _ => None,
            })
        };
        step(
            spend(Spend::Shield {
                from: evolite_api::ShieldSource::Core,
                amount: 400_000,
            }),
            "shield",
        );
        let after_shield = pool_after(ready.balances.shielded);
        let arrived = after_shield.balances.shielded - ready.balances.shielded;
        println!(
            "pool +{arrived} (shown {}), Core now {}",
            400_000 - fees.shield_from_core,
            after_shield.balances.core
        );
        assert_eq!(arrived, 400_000 - fees.shield_from_core);
        assert!(after_shield.pool.unfinished_shield.is_none());

        step(
            spend(Spend::TopUpFromPool { amount: 100_000 }),
            "pool-top-up",
        );
        let after_top_up = pool_after(after_shield.balances.shielded);
        let taken = after_shield.balances.shielded - after_top_up.balances.shielded;
        println!(
            "pool -{taken} (shown {}), credits {} -> {}",
            100_000 + fees.pool_top_up,
            ready.balances.credits,
            after_top_up.balances.credits
        );
        // Platform counts credits (1000 a duff): a fee of 114 359.x duffs shows rounded up, and
        // the pool's balance in duffs is rounded down — a duff either way.
        assert!((taken - (100_000 + fees.pool_top_up)).abs() <= 1, "{taken}");
        assert_eq!(
            after_top_up.balances.credits - ready.balances.credits,
            100_000
        );
    }

    /// The history after the pool's live test (`live_pool_core_and_identity` leaves its folder
    /// in `%TEMP%`): the Core asset lock made for the pool is "Shielded from Core", and the pool's
    /// own operations are lines of the history too. Spends nothing.
    /// `cargo test -p evolite-core --features pool live_history_of_the_pool -- --ignored --nocapture`
    #[cfg(feature = "pool")]
    #[test]
    #[ignore = "needs testnet access and the folder of live_pool_core_and_identity"]
    fn live_history_of_the_pool() {
        let from = std::env::temp_dir().join("evolite-live-pool-in-out");
        let dir = std::env::temp_dir().join("evolite-live-history");
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&from, &dir).expect("the folder of live_pool_core_and_identity");
        let core = Core::real(dir, ArtOptions::default(), || {});
        core.send(Command::Unlock {
            password: Password::new(WARM_PASSWORD.into()),
        });
        let wallet = wait_for(&core, 30, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                (s.sync.stage == SyncStage::Done
                    && w.pool.checked_at.is_some()
                    && !w.pool.operations.is_empty())
                .then(|| w.clone())
            }
            _ => None,
        });
        for act in wallet.activity.iter().take(12) {
            println!("{:?} {} fee {:?}", act.kind, act.amount, act.fee);
        }
        println!(
            "fees: credits {} withdrawal {} name {} contested {} identity {} key {}",
            wallet.fees.credit_transfer,
            wallet.fees.credit_withdrawal,
            wallet.fees.name_regular,
            wallet.fees.name_contested,
            wallet.fees.identity_create,
            wallet.fees.add_key
        );
        assert!(wallet.activity.iter().any(|a| a.kind
            == evolite_api::ActivityKind::Shielded {
                from: Some("Core".into())
            }));
        assert!(
            wallet
                .activity
                .iter()
                .any(|a| a.kind == evolite_api::ActivityKind::PoolSent),
            "the pool's operations are in the history"
        );
    }

    /// The pool's history over a restart: a top-up of the identity from the pool keeps its
    /// amount and fee after the core starts again (platform-wallet derives the pool's history
    /// again from the scan, which gives such a spend as 0). On a copy of the folder that
    /// `live_pool_core_and_identity` leaves: 0.003 DASH from Core into the pool first, then
    /// 0.001 DASH from the pool into the identity.
    /// `cargo test -p evolite-core --features pool live_pool_history_over_a_restart -- --ignored --nocapture`
    #[cfg(feature = "pool")]
    #[test]
    #[ignore = "needs testnet access and the folder of live_pool_core_and_identity"]
    fn live_pool_history_over_a_restart() {
        let from = std::env::temp_dir().join("evolite-live-pool-in-out");
        let dir = std::env::temp_dir().join("evolite-live-pool-restart");
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&from, &dir).expect("the folder of live_pool_core_and_identity");
        let open = || {
            let core = Core::real(dir.clone(), ArtOptions::default(), || {});
            core.send(Command::Unlock {
                password: Password::new(WARM_PASSWORD.into()),
            });
            let ready = wait_for(&core, 30, |e| match e {
                Event::Snapshot(s) => {
                    let w = s.wallet.as_ref()?;
                    // The pool's history is read before its time of check is known.
                    (!w.pool.operations.is_empty()).then(|| w.clone())
                }
                _ => None,
            });
            (core, ready)
        };
        let (core, ready) = open();
        let step = |command: Command, what: &'static str| {
            core.send(command);
            let result = wait_for(&core, 20, |e| match e {
                Event::Done { what: w, result } if *w == what => Some(result.clone()),
                _ => None,
            });
            println!("{what}: {result:?}");
            result.unwrap_or_else(|e| panic!("{what} failed: {e:?}"));
        };
        println!("pool {} duffs", ready.balances.shielded);
        step(
            spend(Spend::Shield {
                from: evolite_api::ShieldSource::Core,
                amount: 300_000,
            }),
            "shield",
        );
        step(
            spend(Spend::TopUpFromPool { amount: 100_000 }),
            "pool-top-up",
        );
        let top_up = |w: &evolite_api::Wallet| {
            w.pool
                .operations
                .iter()
                .filter(|a| a.kind == evolite_api::ActivityKind::PoolSent)
                .max_by_key(|a| a.at)
                .cloned()
        };
        let before = wait_for(&core, 10, |e| match e {
            Event::Snapshot(s) => {
                let w = s.wallet.as_ref()?;
                top_up(w).filter(|a| a.amount != 0)
            }
            _ => None,
        });
        println!("before the restart: {} fee {:?}", before.amount, before.fee);
        drop(core);
        std::thread::sleep(Duration::from_secs(10));
        let (_core, again) = open();
        let after = top_up(&again).expect("the top-up is still in the pool's history");
        println!("after the restart: {} fee {:?}", after.amount, after.fee);
        assert_eq!((after.amount, after.fee), (before.amount, before.fee));
    }

    /// "Import from DashPay" on testnet: a folder with no wallet, only the warm copy's
    /// headers (so the masternode lists come in a minute, as on a computer that ran evolite
    /// before); once they are in, the test phrase is typed and, before any wallet is made, the
    /// core reads Platform and answers what O5 shows — the identity, its credits and keys, its
    /// name and contacts. Makes no wallet, spends nothing.
    /// `cargo test -p evolite-core live_dashpay_import_scan -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and a funded test phrase in .env.test.local"]
    fn live_dashpay_import_scan() {
        let dir = std::env::temp_dir().join("evolite-live-dashpay-import");
        let _ = std::fs::remove_dir_all(&dir);
        let warm = std::env::temp_dir().join("evolite-live-warm").join("spv");
        copy_dir(&warm, &dir.join("spv")).expect("the warm copy's headers");
        let core = Core::real(dir, ArtOptions::default(), || {});
        wait_for(&core, 10, |e| match e {
            Event::Snapshot(s) => (!matches!(
                s.sync.stage,
                SyncStage::Connecting
                    | SyncStage::Headers
                    | SyncStage::Masternodes
                    | SyncStage::Quorums
            ))
            .then_some(()),
            _ => None,
        });
        let phrase =
            test_env("EVOLITE_TEST_PHRASE").expect("EVOLITE_TEST_PHRASE in .env.test.local");
        let started = Instant::now();
        core.send(Command::UsePhrase {
            phrase: evolite_api::Phrase::new(
                phrase.split_whitespace().map(str::to_owned).collect(),
            ),
            dashpay: true,
        });
        let scan = wait_for(&core, 10, |e| match e {
            Event::PhraseAccepted(result) => Some(result.clone()),
            _ => None,
        })
        .expect("the phrase is accepted")
        .expect("the phrase has a DashPay identity");
        println!(
            "after {} s: identity {:?}, {} duffs of credits, {} keys, name {:?}, {} contacts",
            started.elapsed().as_secs(),
            scan.identity_id,
            scan.identity_credits,
            scan.identity_keys,
            scan.name,
            scan.contacts
        );
        assert!(scan.identity_id.is_some() && !scan.core_known);
    }

    fn watch_platform(core: &Core, secs: u64) -> Option<i64> {
        let until = Instant::now() + Duration::from_secs(secs);
        let mut last = None;
        while Instant::now() < until {
            for event in core.events() {
                if let Event::Snapshot(s) = event
                    && let Some(w) = &s.wallet
                {
                    last = Some(w.balances.platform_address);
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        last
    }
}
