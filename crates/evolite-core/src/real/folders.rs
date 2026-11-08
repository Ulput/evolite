//! The wallet's folders on Unix (BUG-94). platform-wallet's store walks every folder above its
//! database up to `/` and refuses to open under one that others can write to; a umask of 0002,
//! the default of many Linux desktops, makes plain `create_dir_all` folders just that. On
//! Windows the store checks nothing and these are plain folders.

use std::path::Path;

use platform_wallet_storage::InsecureAncestor;
use platform_wallet_storage::sqlite::WalletStorageError;

/// `dir` and the folders missing above it, its owner's alone whatever the umask.
pub fn create(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// What a version before BUG-94 made, put right at start. Inside the data folder `wallets` and
/// below are evolite's alone: owner-only. The data folder may be one the person chose
/// (`--data-dir`), so it only loses the write access the store refuses. Nothing above it is
/// touched: [`refused`] tells the person. A symbolic link is left as it is, and a folder that
/// cannot be changed is the store's to refuse.
pub fn repair(data_dir: &Path) {
    set_mode(data_dir, without_others_writing);
    set_mode(&data_dir.join("wallets"), |_| 0o700);
    let network = super::wallets::network_dir(data_dir);
    set_mode(&network, |_| 0o700);
    for entry in std::fs::read_dir(&network).into_iter().flatten().flatten() {
        set_mode(&entry.path(), |_| 0o700);
    }
}

/// A folder's mode without the write access of group and others; a sticky folder is kept: the
/// store accepts it, and someone made it shared on purpose.
fn without_others_writing(mode: u32) -> u32 {
    if mode & 0o1000 != 0 {
        mode
    } else {
        mode & !0o022
    }
}

#[cfg(unix)]
fn set_mode(dir: &Path, new: impl Fn(u32) -> u32) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return;
    };
    let mode = meta.permissions().mode() & 0o7777;
    if meta.is_dir() && new(mode) != mode {
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(new(mode)));
    }
}

#[cfg(not(unix))]
fn set_mode(_dir: &Path, _new: impl Fn(u32) -> u32) {}

/// The words for the person when the store refuses a folder; `None` for any other failure.
pub(super) fn refused(error: &WalletStorageError) -> Option<String> {
    match error {
        WalletStorageError::InsecureParentDir { ancestor, reason } => {
            Some(refused_text(ancestor, reason))
        }
        _ => None,
    }
}

fn refused_text(folder: &Path, reason: &InsecureAncestor) -> String {
    let folder = folder.display();
    match reason {
        InsecureAncestor::WritableWithoutSticky { .. } => format!(
            tr!(
                "Sync cannot start: other users of this computer can write to the folder {0}. Run chmod go-w '{0}' in a terminal, and it starts by itself within a minute."
            ),
            folder
        ),
        InsecureAncestor::UntrustedOwner { .. } => format!(
            tr!(
                "Sync cannot start: the folder {} belongs to another user. Make it yours, or start evolite with another --data-dir."
            ),
            folder
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_folder_only_loses_the_write_access_of_others() {
        assert_eq!(without_others_writing(0o775), 0o755);
        assert_eq!(without_others_writing(0o770), 0o750);
        assert_eq!(without_others_writing(0o700), 0o700);
        assert_eq!(without_others_writing(0o1777), 0o1777, "sticky: on purpose");
    }

    #[test]
    fn a_refused_folder_is_said_with_what_to_do() {
        let folder = Path::new("/home/a/.local/share");
        let open = refused_text(
            folder,
            &InsecureAncestor::WritableWithoutSticky { mode: 0o775 },
        );
        assert!(
            open.contains("the folder /home/a/.local/share.")
                && open.contains("chmod go-w '/home/a/.local/share'"),
            "{open}"
        );
        let owned = refused_text(
            folder,
            &InsecureAncestor::UntrustedOwner {
                uid: 2,
                current_uid: 1,
            },
        );
        assert!(
            owned.contains("the folder /home/a/.local/share belongs"),
            "{owned}"
        );
    }

    #[test]
    fn only_the_stores_refusal_of_a_folder_is_put_in_words() {
        let other = WalletStorageError::Io(std::io::Error::other("disk"));
        assert_eq!(refused(&other), None);
        let folder = WalletStorageError::InsecureParentDir {
            ancestor: "/x".into(),
            reason: InsecureAncestor::WritableWithoutSticky { mode: 0o775 },
        };
        assert!(refused(&folder).is_some_and(|text| text.contains("chmod go-w '/x'")));
    }
}
