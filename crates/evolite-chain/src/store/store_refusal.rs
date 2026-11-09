//! The words for the person when the store refuses a folder (BUG-94).

use std::path::Path;

use platform_wallet_storage::InsecureAncestor;
use platform_wallet_storage::sqlite::WalletStorageError;

/// The words for the person when the store refuses a folder; `None` for any other failure.
pub fn refused(error: &WalletStorageError) -> Option<String> {
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
