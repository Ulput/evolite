//! Which network a run opens (FEAT-0037, the customer's М3): `--network`, else the last one
//! chosen (`last-network` in the app's common folder), else testnet. The network is fixed for the
//! process (`evolite_core::network`, a `OnceLock`): a switch starts evolite again on the other one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use evolite_api::Network;

/// The app's common folder, beside the networks' folders (`evolite`, `evolite-mainnet`) in
/// `base`: what is the app's, not a network's — the last network and the app's settings. With
/// `--data-dir` (QA: one folder holds both networks, `wallets/<network>`), that folder itself.
pub fn app_dir(data_dir: Option<&str>, base: &Path) -> PathBuf {
    match data_dir {
        Some(dir) => PathBuf::from(dir),
        None => base.join("evolite-app"),
    }
}

/// The file of the last network chosen, in the common folder `dir`.
pub fn last_path(dir: &Path) -> PathBuf {
    dir.join("last-network")
}

/// The last network chosen; `None` with no file, or one that says nothing known.
pub fn read_last(dir: &Path) -> Option<Network> {
    let text = std::fs::read_to_string(last_path(dir)).ok()?;
    network_named(text.trim())
}

/// Keeps `network` as the last chosen, for the next run.
pub fn write_last(dir: &Path, network: Network) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        last_path(dir),
        format!("{}\n", evolite_core::network::name_of(network)),
    )
}

/// The network a run opens: `--network`'s word when it names one, else the last chosen, else
/// testnet.
pub fn start_network(flag: Option<&str>, last: Option<Network>) -> Network {
    flag.and_then(network_named)
        .or(last)
        .unwrap_or(Network::Testnet)
}

fn network_named(word: &str) -> Option<Network> {
    match word {
        "testnet" => Some(Network::Testnet),
        "mainnet" => Some(Network::Mainnet),
        _ => None,
    }
}

/// Marks a run started by a switch of network: a folder held by another window is said as the
/// network being open there (AC-A6), not as a second copy.
pub const SWITCHED: &str = "--switched";
/// The old window's place and size, `x,y,w,h` in points, for the run a switch starts (AC-T6ж).
pub const WINDOW_AT: &str = "--window-at";

/// The command line of a run started again on `network`: this one's, without its `--network`,
/// place and mark, with `--network <network>`, the mark and the window's place (`x, y, w, h`)
/// at the end.
pub fn restart_args(
    args: &[OsString],
    network: Network,
    window: Option<[f32; 4]>,
) -> Vec<OsString> {
    let mut again = Vec::with_capacity(args.len() + 5);
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        if arg == "--network" || arg == WINDOW_AT {
            skip = true;
            continue;
        }
        if arg == SWITCHED {
            continue;
        }
        again.push(arg.clone());
    }
    again.push("--network".into());
    again.push(evolite_core::network::name_of(network).into());
    again.push(SWITCHED.into());
    if let Some([x, y, w, h]) = window {
        again.push(WINDOW_AT.into());
        again.push(format!("{},{},{},{}", x.round(), y.round(), w.round(), h.round()).into());
    }
    again
}

/// `x,y,w,h` of [`WINDOW_AT`]; `None` unless four numbers with a size.
pub fn window_at(text: &str) -> Option<[f32; 4]> {
    let parts: Vec<f32> = text
        .split(',')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    match parts[..] {
        [x, y, w, h] if w > 0.0 && h > 0.0 => Some([x, y, w, h]),
        _ => None,
    }
}

/// Another evolite holds data folder `dir` (its `evolite.lock`, as `lock_data_dir` takes it):
/// that network is open in another window (AC-T6е). Only looks: no folder is made, and the lock,
/// when free, is let go at once.
pub fn held(dir: &Path) -> bool {
    let Ok(file) = std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join("evolite.lock"))
    else {
        // No lock file: no evolite has opened it yet.
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// The data folder `dir` is the other network's own (`evolite` or `evolite-mainnet` in `base`):
/// opened as `network` it would mix the two (AC-T7). A folder of one's own choosing
/// (`--data-dir`) keeps each network's files apart inside, and is never refused here.
pub fn wrong_folder(dir: &Path, network: Network, base: &Path) -> bool {
    let theirs = match network {
        Network::Testnet => base.join("evolite-mainnet"),
        Network::Mainnet => base.join("evolite"),
    };
    dir == theirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::Network::{Mainnet, Testnet};

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("evolite-network-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// AC-T6а: `--network` first, then the last choice, then testnet.
    #[test]
    fn the_network_is_the_flag_then_the_last_choice_then_testnet() {
        assert_eq!(start_network(Some("mainnet"), Some(Testnet)), Mainnet);
        assert_eq!(start_network(Some("testnet"), Some(Mainnet)), Testnet);
        assert_eq!(start_network(None, Some(Mainnet)), Mainnet);
        assert_eq!(start_network(None, None), Testnet);
        assert_eq!(
            start_network(Some("main"), Some(Mainnet)),
            Mainnet,
            "an unknown word is no choice: the last one stands"
        );
    }

    /// AC-T6а: the choice is written and read back; a missing or damaged file is no choice.
    #[test]
    fn the_last_choice_is_kept_and_a_damaged_file_is_none() {
        let d = dir("last");
        assert_eq!(read_last(&d), None, "no file yet");
        write_last(&d, Mainnet).unwrap();
        assert_eq!(read_last(&d), Some(Mainnet));
        write_last(&d, Testnet).unwrap();
        assert_eq!(read_last(&d), Some(Testnet));
        std::fs::write(last_path(&d), "mainnet\n").unwrap();
        assert_eq!(read_last(&d), Some(Mainnet), "a line end is fine");
        std::fs::write(last_path(&d), "\u{0}garbage").unwrap();
        assert_eq!(read_last(&d), None, "damaged: testnet at start");
    }

    /// AC-T6б, AC-T6ж: a switch starts evolite again with `--network <the other>`, at the old
    /// window's place and size, marked as a switch; the old network, place and mark dropped, the
    /// rest of the command kept.
    #[test]
    fn a_switch_starts_again_on_the_other_network() {
        let args: Vec<std::ffi::OsString> = [
            "--network",
            "testnet",
            "--data-dir",
            "D:/qa",
            "--window-at",
            "1,2,3,4",
            "--switched",
            "--theme",
            "light",
        ]
        .iter()
        .map(Into::into)
        .collect();
        let again = restart_args(&args, Mainnet, Some([10.0, 20.0, 1280.0, 800.0]));
        let again: Vec<&str> = again.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            again,
            [
                "--data-dir",
                "D:/qa",
                "--theme",
                "light",
                "--network",
                "mainnet",
                "--switched",
                "--window-at",
                "10,20,1280,800"
            ]
        );
    }

    /// AC-T6ж: the new run reads the old window's place and size; anything else is none.
    #[test]
    fn the_window_place_is_read_back() {
        assert_eq!(
            window_at("10,20,1280,800"),
            Some([10.0, 20.0, 1280.0, 800.0])
        );
        assert_eq!(
            window_at("-1920,0,1024,768"),
            Some([-1920.0, 0.0, 1024.0, 768.0])
        );
        for bad in ["", "1,2,3", "a,b,c,d", "1,2,0,800", "1,2,3,4,5"] {
            assert_eq!(window_at(bad), None, "«{bad}»");
        }
    }

    /// AC-T7: a network's own folder opened as the other network's is refused; a folder of one's
    /// own choosing (`--data-dir`) holds both networks apart and is not.
    #[test]
    fn a_networks_folder_is_not_opened_as_the_others() {
        let base = std::path::Path::new("/home/a/.local/share");
        let (testnet, mainnet) = (base.join("evolite"), base.join("evolite-mainnet"));
        assert!(!wrong_folder(&testnet, Testnet, base));
        assert!(!wrong_folder(&mainnet, Mainnet, base));
        assert!(wrong_folder(&mainnet, Testnet, base));
        assert!(wrong_folder(&testnet, Mainnet, base));
        let qa = std::path::Path::new("D:/qa");
        assert!(!wrong_folder(qa, Testnet, base) && !wrong_folder(qa, Mainnet, base));
    }

    /// The app's common folder: `evolite-app` beside the networks' folders; with `--data-dir`,
    /// that folder itself (the architect, 2026-11-14).
    #[test]
    fn the_common_folder_is_evolite_app_or_the_data_dir() {
        let base = std::path::Path::new("/home/a/.local/share");
        assert_eq!(app_dir(None, base), base.join("evolite-app"));
        assert_eq!(
            app_dir(Some("D:/qa"), base),
            std::path::PathBuf::from("D:/qa")
        );
    }

    /// AC-T6е: a network whose folder another evolite holds is seen as held, and nothing in it
    /// is touched by looking; once let go, it is free.
    #[test]
    fn a_folder_another_window_holds_is_seen() {
        let d = dir("held");
        std::fs::create_dir_all(&d).unwrap();
        assert!(!held(&d), "free");
        let other = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(d.join("evolite.lock"))
            .unwrap();
        other.try_lock().unwrap();
        assert!(held(&d), "another window has it");
        drop(other);
        assert!(!held(&d), "let go");
        assert!(!held(&dir("never-made")), "no folder yet: free");
        assert!(!dir("never-made").exists(), "looking makes no folder");
    }
}
