//! SEC-4 (phrase review Ф7): a crash of the wallet leaves no memory dump behind. With a wallet
//! open, the process holds the phrase and keys; Windows Error Reporting would write a dump of
//! it (and, with the person's consent, send it to Microsoft). The panic itself is written to
//! our journal through `scrub` already.
//!
//! - This executable is excluded from Windows Error Reporting for the user
//!   (`WerAddExcludedApplication`, no admin needed); on every start, as the executable may
//!   have moved, and it costs one registry write.
//! - No «evolite has stopped working» box (`SEM_NOGPFAULTERRORBOX`), as the render worker
//!   (`evolite_render::worker::quiet_crashes`).

/// Keeps a crash of this process out of Windows Error Reporting's dumps and dialog; false when
/// the exclusion was refused (the box is gone either way). Elsewhere nothing to do.
pub fn keep_out() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::Diagnostics::Debug::{SEM_NOGPFAULTERRORBOX, SetErrorMode};
        use windows_sys::Win32::System::ErrorReporting::WerAddExcludedApplication;
        // SAFETY: a plain call that adds a flag to this process's error mode.
        unsafe {
            let mode = SetErrorMode(0);
            SetErrorMode(mode | SEM_NOGPFAULTERRORBOX);
        }
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let path: Vec<u16> = exe.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: a NUL-terminated path; FALSE: for this user only.
        unsafe { WerAddExcludedApplication(path.as_ptr(), 0) >= 0 }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Takes `exe` off Windows Error Reporting's exclusions again (this user's); true when it is
/// off or was never on. The tests' own binaries use it; the uninstaller has its own copy
/// (`setup::system`), being another program.
#[cfg(all(test, windows))]
fn let_in(exe: &std::path::Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::ErrorReporting::WerRemoveExcludedApplication;
    let path: Vec<u16> = exe.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: a NUL-terminated path; FALSE: for this user only.
    unsafe { WerRemoveExcludedApplication(path.as_ptr(), 0) >= 0 }
}

#[cfg(all(test, windows))]
mod tests {
    /// The exclusion is accepted for this user without admin rights, and no fault box is shown.
    #[test]
    fn a_crash_is_kept_out_of_windows_error_reporting() {
        use windows_sys::Win32::System::Diagnostics::Debug::{SEM_NOGPFAULTERRORBOX, SetErrorMode};
        assert!(super::keep_out(), "WerAddExcludedApplication refused");
        // SAFETY: reads the mode back (SetErrorMode returns the previous one) and restores it.
        let mode = unsafe {
            let mode = SetErrorMode(0);
            SetErrorMode(mode);
            mode
        };
        assert_ne!(mode & SEM_NOGPFAULTERRORBOX, 0);
        // WER keeps the exclusion by file name: each test binary (`evolite-<hash>.exe`) would
        // leave one more value in the user's registry (review S1).
        assert!(super::let_in(
            &std::env::current_exe().expect("this test's exe")
        ));
    }
}
