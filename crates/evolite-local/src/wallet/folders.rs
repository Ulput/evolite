//! The wallet's folders on Unix (BUG-94). platform-wallet's store walks every folder above its
//! database up to `/` and refuses to open under one that others can write to; a umask of 0002,
//! the default of many Linux desktops, makes plain `create_dir_all` folders just that. On
//! Windows the store checks nothing and these are plain folders.

use std::path::Path;

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
    let network = crate::wallets::network_dir(data_dir);
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
}
