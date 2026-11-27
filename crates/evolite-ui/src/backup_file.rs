//! FEAT-0045: the system's «Save as» and «Open» for the backup file, on a thread of their own
//! so that the window keeps drawing; the core reads and writes the file. In tests no dialog
//! opens: the path a test put here is «chosen» ([`test_pick`]).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

/// What the dialog is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Save,
    Open,
    /// The folder of the automatic copies (FEAT-0045 step 2).
    Folder,
}

/// The default name of a backup (R-3): `evolite-{wallet}-{network}-{YYYY-MM-DD}.evbackup`,
/// the wallet's name kept to what a file name may hold on every system.
pub fn default_name(wallet: &str, network: &str, day: &str) -> String {
    let wallet: String = wallet
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let wallet = wallet.trim_matches(['.', ' ']);
    let wallet = if wallet.is_empty() { "wallet" } else { wallet };
    format!(
        "evolite-{wallet}-{network}-{day}.{}",
        evolite_api::backup::EXTENSION
    )
}

/// Opens the dialog for `pick` and answers once: the path chosen, or `None` (cancelled).
pub fn ask(pick: Pick, name: &str) -> Receiver<Option<PathBuf>> {
    let (tx, rx) = channel();
    #[cfg(test)]
    {
        let _ = (pick, name);
        let _ = tx.send(TEST_PICK.with(|p| p.borrow_mut().take()));
    }
    #[cfg(not(test))]
    {
        let parent = crate::qr_scan::Parent::now();
        let name = name.to_owned();
        let title = crate::i18n::t(match pick {
            Pick::Save => tr!("Save a backup file…"),
            Pick::Open => tr!("Restore from a file…"),
            Pick::Folder => tr!("Save a copy once a week"),
        })
        .trim_end_matches('…')
        .to_owned();
        std::thread::spawn(move || {
            let ext = evolite_api::backup::EXTENSION;
            let dialog = rfd::FileDialog::new()
                .set_title(&title)
                .add_filter("evolite backup", &[ext]);
            #[cfg(windows)]
            let dialog = dialog.set_parent(&parent);
            #[cfg(not(windows))]
            let _ = parent;
            let chosen = match pick {
                Pick::Save => {
                    let dialog = dialog.set_file_name(&name);
                    let dialog = match crate::csv_export::downloads() {
                        Some(dir) => dialog.set_directory(dir),
                        None => dialog,
                    };
                    dialog.save_file()
                }
                Pick::Open => dialog.pick_file(),
                // BUG-344: the folder's dialog over evolite too, not behind it.
                Pick::Folder => {
                    let folder = rfd::FileDialog::new().set_title(&title);
                    #[cfg(windows)]
                    let folder = folder.set_parent(&parent);
                    folder.pick_folder()
                }
            };
            let _ = tx.send(chosen);
        });
    }
    rx
}

/// The words of a template with `{}`, translated, and `what` put in.
fn put(template: &str, what: &str) -> String {
    crate::i18n::t(template).replacen("{}", what, 1)
}

/// What a refusal says.
fn refusal(e: &evolite_api::backup::BackupError) -> String {
    use evolite_api::backup::BackupError as E;
    match e {
        E::Locked => crate::i18n::t(tr!("The wallet is locked")).into_owned(),
        E::OtherWallet => crate::i18n::t(tr!("This file is from another wallet")).into_owned(),
        E::OtherNetwork(network) => put(tr!("This file is for {}"), network),
        E::Newer => {
            crate::i18n::t(tr!("The file was made by a newer evolite. Update the app")).into_owned()
        }
        E::Damaged => crate::i18n::t(tr!("The file is damaged or changed")).into_owned(),
        E::Io(_) => crate::i18n::t(tr!("The file could not be read or written")).into_owned(),
        E::Partial { done, .. } => put(
            tr!("Restored in part: {}. Try again — a repeat duplicates nothing"),
            &what_came(done),
        ),
    }
}

/// UC-1: «Saved: {name}», or why not.
pub fn said_saved(result: &Result<String, evolite_api::backup::BackupError>) -> (bool, String) {
    match result {
        Ok(name) => (true, put(tr!("Saved: {}"), name)),
        Err(e) => (false, refusal(e)),
    }
}

/// UC-2: «Restored from the file: 37 notes, 12 addresses…», only what is not none; or why not.
pub fn said_restored(
    result: &Result<evolite_api::backup::Restored, evolite_api::backup::BackupError>,
) -> (bool, String) {
    match result {
        Ok(done) => (
            true,
            put(tr!("Restored from the file: {}"), &what_came(done)),
        ),
        Err(e) => (false, refusal(e)),
    }
}

/// The counts of a restore, by kind, only those above none.
fn what_came(done: &evolite_api::backup::Restored) -> String {
    use crate::text::plural_in;
    let mut parts = Vec::new();
    let mut add = |n: usize, text: String| {
        if n > 0 {
            parts.push(crate::i18n::t(&text).into_owned());
        }
    };
    let notes = done.notes;
    add(
        notes,
        plural_in(notes as u32, tr!("{} note"), tr!("{} notes")),
    );
    // Review r1, Could 1: notes the file had changed later than here.
    let newer = done.notes_newer;
    add(
        newer,
        plural_in(
            newer as u32,
            tr!("{} note newer in the file"),
            tr!("{} notes newer in the file"),
        ),
    );
    let n = done.addresses;
    add(
        n,
        plural_in(n as u32, tr!("{} address"), tr!("{} addresses")),
    );
    let n = done.contact_names;
    add(
        n,
        plural_in(n as u32, tr!("{} contact name"), tr!("{} contact names")),
    );
    let n = done.hidden_requests;
    add(
        n,
        plural_in(
            n as u32,
            tr!("{} hidden request"),
            tr!("{} hidden requests"),
        ),
    );
    let n = done.apps;
    add(
        n,
        plural_in(n as u32, tr!("{} app sign-in"), tr!("{} app sign-ins")),
    );
    let n = done.tokens;
    add(n, plural_in(n as u32, tr!("{} token"), tr!("{} tokens")));
    let n = done.records;
    add(n, plural_in(n as u32, tr!("{} record"), tr!("{} records")));
    if parts.is_empty() {
        crate::i18n::t(tr!("nothing new")).into_owned()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
thread_local! {
    static TEST_PICK: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Tests: the next dialog «chooses» `path` (`None`: cancelled). Only the kittests use it.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub fn test_pick(path: Option<PathBuf>) {
    TEST_PICK.with(|p| *p.borrow_mut() = path);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R-3: the default name, the wallet's name kept to what a file name may hold.
    #[test]
    fn the_default_name_is_the_wallets_the_networks_and_the_days() {
        assert_eq!(
            default_name("Savings", "testnet", "2026-11-21"),
            "evolite-Savings-testnet-2026-11-21.evbackup"
        );
        assert_eq!(
            default_name(" a/b:c? ", "mainnet", "2026-11-21"),
            "evolite-a-b-c--mainnet-2026-11-21.evbackup"
        );
        assert_eq!(
            default_name("..", "testnet", "2026-11-21"),
            "evolite-wallet-testnet-2026-11-21.evbackup"
        );
    }
}
