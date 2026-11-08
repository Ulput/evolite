//! BUG-94, Unix only: the store's check of the folders above it. Their names begin with
//! `private_folders`: CI runs them again under `umask 0002`, the default of many Linux desktops.

use std::os::unix::fs::PermissionsExt;

use super::*;

fn mode_of(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

fn set(path: &std::path::Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// The folders from the data folder down to the wallet's.
fn owned(dir: &std::path::Path, id: &str) -> [PathBuf; 3] {
    [
        dir.join("wallets"),
        wallets::network_dir(dir),
        wallets::wallet_dir(dir, id),
    ]
}

/// As the app makes them: the folder locked first, then the wallet written.
#[tokio::test(flavor = "multi_thread")]
async fn private_folders_of_a_new_wallet_let_its_store_open() {
    let dir = fresh_dir("private-new");
    let _lock = lock_data_dir(&dir).unwrap();
    seal_wallet(&dir, "aaaa0001");
    for folder in owned(&dir, "aaaa0001") {
        assert_eq!(mode_of(&folder), 0o700, "{}", folder.display());
    }
    let core = core_at(&dir, "aaaa0001");
    core.chain
        .open_store_only(&core.paths.chain)
        .await
        .expect("the store opens");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A folder of a version before: put right at the next start, whatever the umask.
#[tokio::test(flavor = "multi_thread")]
async fn private_folders_made_before_are_put_right_at_start() {
    let dir = fresh_dir("private-old");
    seal_wallet(&dir, "aaaa0001");
    set(&dir, 0o775);
    for folder in owned(&dir, "aaaa0001") {
        set(&folder, 0o775);
    }
    let _lock = lock_data_dir(&dir).unwrap();
    assert_eq!(mode_of(&dir), 0o755, "only the write access goes");
    for folder in owned(&dir, "aaaa0001") {
        assert_eq!(mode_of(&folder), 0o700, "{}", folder.display());
    }
    let core = core_at(&dir, "aaaa0001");
    core.chain
        .open_store_only(&core.paths.chain)
        .await
        .expect("the store opens");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A folder above the data folder is not evolite's: left as it is, the person told in words,
/// and the next try opens once they have changed it.
#[tokio::test(flavor = "multi_thread")]
async fn private_folders_above_the_data_folder_are_only_named() {
    let above = fresh_dir("private-above");
    let dir = above.join("data");
    seal_wallet(&dir, "aaaa0001");
    set(&above, 0o775);
    let _lock = lock_data_dir(&dir).unwrap();
    assert_eq!(mode_of(&above), 0o775, "not evolite's");
    let core = core_at(&dir, "aaaa0001");
    assert!(core.chain.open_store_only(&core.paths.chain).await.is_err());
    let said = core.chain.status().folder_refused.expect("said in words");
    assert!(
        said.contains(&format!("chmod go-w '{}'", above.display())),
        "{said}"
    );

    set(&above, 0o700);
    core.chain
        .open_store_only(&core.paths.chain)
        .await
        .expect("the store opens");
    assert_eq!(core.chain.status().folder_refused, None);
    let _ = std::fs::remove_dir_all(&above);
}

/// What is not evolite's stays as it is, and every wallet's folder is put right.
#[test]
fn private_folders_repair_only_what_is_evolites() {
    let dir = fresh_dir("private-repair");
    let outside = fresh_dir("private-outside");
    std::fs::create_dir_all(&outside).unwrap();
    set(&outside, 0o775);
    seal_wallet(&dir, "aaaa0001");
    seal_wallet(&dir, "bbbb0002");
    for id in ["aaaa0001", "bbbb0002"] {
        set(&wallets::wallet_dir(&dir, id), 0o775);
    }
    std::os::unix::fs::symlink(&outside, wallets::network_dir(&dir).join("linked")).unwrap();
    set(&dir, 0o1775);
    let _lock = lock_data_dir(&dir).unwrap();
    for id in ["aaaa0001", "bbbb0002"] {
        assert_eq!(mode_of(&wallets::wallet_dir(&dir, id)), 0o700, "{id}");
    }
    assert_eq!(mode_of(&outside), 0o775, "behind a link: not evolite's");
    assert_eq!(mode_of(&dir), 0o1775, "sticky: shared on purpose");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&outside);
}
