//! A new data folder that only its owner opens. Under `%APPDATA%` the profile's own access list
//! already does that, but `--data-dir` may point anywhere, and a folder on another drive takes
//! that drive's list — often "Authenticated Users: modify": anyone with an account on the
//! computer could copy the vault and guess its password offline.
//!
//! On Windows the folder gets its own protected list, inherited by everything inside: full access
//! for its owner, SYSTEM and Administrators, nobody else. On Unix the folder and any missing
//! above it are 0700. Only a folder evolite creates: an existing one keeps the access its owner
//! gave it (the core takes write access from others at start, BUG-94: the store refuses it).

use std::path::Path;

/// The access list a new data folder gets on Windows (SDDL): protected from the parent's, and
/// inherited by files and folders inside — the owner (Owner Rights), SYSTEM, Administrators.
#[cfg(windows)]
const OWNER_ONLY: &str = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

/// Creates `dir` (and its parents) if it is not there; a folder created here is made private.
pub fn create(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    create_above(dir)?;
    owners_alone().create(dir)?;
    restrict(dir)
}

/// The folders missing above `path`, not `path` itself: on Unix they are the owner's alone
/// whatever the umask, since the wallet's database refuses a folder above it that others can
/// write to (`~/.local` of a new user, made by the installer, BUG-94).
pub fn create_above(path: &Path) -> std::io::Result<()> {
    match path.parent() {
        Some(parent) => owners_alone().create(parent),
        None => Ok(()),
    }
}

fn owners_alone() -> std::fs::DirBuilder {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
}

#[cfg(unix)]
fn restrict(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(windows)]
fn restrict(dir: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        SetFileSecurityW,
    };
    let sddl: Vec<u16> = OWNER_ONLY.encode_utf16().chain([0]).collect();
    let path: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: a NUL-terminated SDDL string; on success the descriptor is ours to free with
    // LocalFree, which happens below on every path.
    let made = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if made == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a NUL-terminated path and the descriptor made above, alive for the call.
    let set = unsafe {
        SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    let error = std::io::Error::last_os_error();
    // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe { LocalFree(descriptor) };
    if set == 0 { Err(error) } else { Ok(()) }
}

#[cfg(not(any(unix, windows)))]
fn restrict(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evolite-private-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// The folder's access list as SDDL.
    #[cfg(windows)]
    fn access_of(path: &Path) -> String {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, GetFileSecurityW};
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut needed = 0u32;
        // SAFETY: asks for the size only.
        unsafe {
            GetFileSecurityW(
                wide.as_ptr(),
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                0,
                &mut needed,
            )
        };
        let mut buffer = vec![0u8; needed as usize];
        let mut text: windows_sys::core::PWSTR = std::ptr::null_mut();
        // SAFETY: a buffer of the size asked for; the string is freed with LocalFree.
        unsafe {
            assert_ne!(
                GetFileSecurityW(
                    wide.as_ptr(),
                    DACL_SECURITY_INFORMATION,
                    buffer.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                ),
                0
            );
            assert_ne!(
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    buffer.as_mut_ptr().cast(),
                    SDDL_REVISION_1,
                    DACL_SECURITY_INFORMATION,
                    &mut text,
                    std::ptr::null_mut(),
                ),
                0
            );
            let len = (0..).take_while(|&i| *text.add(i) != 0).count();
            let sddl = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            LocalFree(text.cast());
            sddl
        }
    }

    #[test]
    fn a_new_data_folder_is_its_owners_alone() {
        let dir = folder("new");
        let inside = dir.join("evolite");
        create(&inside).unwrap();
        assert!(inside.is_dir());
        #[cfg(windows)]
        {
            let sddl = access_of(&inside);
            assert!(sddl.starts_with("D:P"), "not protected: {sddl}");
            // Owner Rights, SYSTEM, Administrators — and no Users, Authenticated Users or Everyone.
            for who in ["OW", "SY", "BA"] {
                assert!(
                    sddl.contains(&format!(";;;{who})")),
                    "{who} missing: {sddl}"
                );
            }
            for who in ["BU", "AU", "WD"] {
                assert!(
                    !sddl.contains(&format!(";;;{who})")),
                    "{who} has access: {sddl}"
                );
            }
            // A file inside inherits it, and its owner can still write and read it.
            std::fs::write(inside.join("wallet.json"), b"{}").unwrap();
            assert_eq!(std::fs::read(inside.join("wallet.json")).unwrap(), b"{}");
            let file = access_of(&inside.join("wallet.json"));
            assert!(
                !file.contains(";;;AU)") && !file.contains(";;;BU)"),
                "{file}"
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&inside).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn private_folders_missing_above_are_made_and_not_the_last() {
        let dir = folder("above");
        let last = dir.join("share").join("applications");
        create_above(&last).unwrap();
        assert!(dir.join("share").is_dir() && !last.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for made in [dir.clone(), dir.join("share")] {
                let mode = std::fs::metadata(&made).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o700, "{}", made.display());
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_existing_folder_keeps_its_access() {
        let dir = folder("old");
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(windows)]
        let before = access_of(&dir);
        create(&dir).unwrap();
        #[cfg(windows)]
        assert_eq!(access_of(&dir), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
