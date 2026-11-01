//! What the worker does to itself once the fonts are found and before the first untrusted
//! byte. On Windows the parent has already put it in an AppContainer ([`crate::sandbox`]);
//! on Linux and macOS a process can only close the doors from the inside:
//!
//! - **Linux**: Landlock — of the file system only the font folders stay, read-only; TCP
//!   neither out nor in (kernel 6.7+). Then seccomp — no sockets, no new programs, no
//!   debugging or reading other processes, no namespaces, mounts or kernel modules, and no
//!   memory made executable (what shellcode needs). Landlock works on kernels from 5.13 and
//!   is applied as far as the kernel goes; seccomp is required: without it the worker refuses
//!   to start.
//! - **macOS**: a Seatbelt profile — everything denied but reading the font folders.

use std::path::PathBuf;

/// Closes what can be closed. `Err`: the worker must not serve.
pub(crate) fn apply(font_dirs: &[PathBuf]) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        linux::landlock(font_dirs)?;
        linux::seccomp()?;
    }
    #[cfg(target_os = "macos")]
    macos::seatbelt(font_dirs)?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = font_dirs;
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use landlock::{
        ABI, Access, AccessFs, AccessNet, Ruleset, RulesetAttr, RulesetCreatedAttr,
        path_beneath_rules,
    };
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule,
    };

    pub(super) fn landlock(font_dirs: &[PathBuf]) -> Result<(), String> {
        let fs = ABI::V5;
        Ruleset::default()
            .handle_access(AccessFs::from_all(fs))
            .and_then(|r| r.handle_access(AccessNet::from_all(ABI::V4)))
            .and_then(|r| r.create())
            .and_then(|r| r.add_rules(path_beneath_rules(font_dirs, AccessFs::from_read(fs))))
            .and_then(|r| r.restrict_self())
            .map(|_| ())
            .map_err(|e| format!("landlock: {e}"))
    }

    /// Calls that answer `EPERM` from now on.
    fn denied() -> Vec<i64> {
        let mut calls = vec![
            libc::SYS_socket,
            libc::SYS_socketpair,
            libc::SYS_connect,
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_accept,
            libc::SYS_accept4,
            libc::SYS_execve,
            libc::SYS_execveat,
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_unshare,
            libc::SYS_setns,
            libc::SYS_mount,
            libc::SYS_umount2,
            libc::SYS_pivot_root,
            libc::SYS_chroot,
            libc::SYS_bpf,
            libc::SYS_perf_event_open,
            libc::SYS_userfaultfd,
            libc::SYS_keyctl,
            libc::SYS_add_key,
            libc::SYS_request_key,
            libc::SYS_io_uring_setup,
            libc::SYS_init_module,
            libc::SYS_finit_module,
            libc::SYS_delete_module,
            libc::SYS_kexec_load,
            libc::SYS_reboot,
            libc::SYS_swapon,
            libc::SYS_swapoff,
        ];
        #[cfg(target_arch = "x86_64")]
        calls.extend([libc::SYS_fork, libc::SYS_vfork]);
        calls
    }

    /// The argument of `mmap`/`mprotect` with the protection, and the rule that matches when it
    /// asks for executable memory.
    fn asks_for_exec(arg: u8) -> Result<SeccompRule, String> {
        let exec = libc::PROT_EXEC as u64;
        SeccompCondition::new(
            arg,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::MaskedEq(exec),
            exec,
        )
        .and_then(|c| SeccompRule::new(vec![c]))
        .map_err(|e| format!("seccomp rule: {e}"))
    }

    pub(super) fn seccomp() -> Result<(), String> {
        let mut rules: BTreeMap<i64, Vec<SeccompRule>> = denied()
            .into_iter()
            .map(|call| (call, Vec::new()))
            .collect();
        // mmap(addr, len, prot, …) and mprotect(addr, len, prot): `prot` is the third.
        rules.insert(libc::SYS_mmap, vec![asks_for_exec(2)?]);
        rules.insert(libc::SYS_mprotect, vec![asks_for_exec(2)?]);
        rules.insert(libc::SYS_pkey_mprotect, vec![asks_for_exec(2)?]);
        let arch = std::env::consts::ARCH
            .try_into()
            .map_err(|e| format!("seccomp arch: {e:?}"))?;
        let filter = SeccompFilter::new(
            rules,
            SeccompAction::Allow,
            SeccompAction::Errno(libc::EPERM as u32),
            arch,
        )
        .map_err(|e| format!("seccomp filter: {e}"))?;
        let program: BpfProgram = filter
            .try_into()
            .map_err(|e| format!("seccomp program: {e:?}"))?;
        // SAFETY: a plain prctl; seccomp requires no_new_privs for an unprivileged process.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            return Err("no_new_privs".into());
        }
        seccompiler::apply_filter_all_threads(&program).map_err(|e| format!("seccomp: {e}"))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CStr, CString, c_char, c_int};
    use std::path::PathBuf;

    unsafe extern "C" {
        fn sandbox_init(profile: *const c_char, flags: u64, error: *mut *mut c_char) -> c_int;
        fn sandbox_free_error(error: *mut c_char);
    }

    /// A path as an SBPL string literal.
    fn literal(path: &std::path::Path) -> String {
        let text = path.to_string_lossy();
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }

    pub(super) fn seatbelt(font_dirs: &[PathBuf]) -> Result<(), String> {
        let mut profile = String::from("(version 1)(deny default)(allow sysctl-read)");
        for dir in font_dirs {
            profile.push_str(&format!("(allow file-read* (subpath {}))", literal(dir)));
        }
        let profile = CString::new(profile).map_err(|_| "a font path with NUL".to_string())?;
        let mut error: *mut c_char = std::ptr::null_mut();
        // SAFETY: a NUL-terminated profile; the error string, if any, is freed as documented.
        unsafe {
            if sandbox_init(profile.as_ptr(), 0, &mut error) == 0 {
                return Ok(());
            }
            let message = if error.is_null() {
                "sandbox_init failed".to_string()
            } else {
                let m = CStr::from_ptr(error).to_string_lossy().into_owned();
                sandbox_free_error(error);
                m
            };
            Err(message)
        }
    }
}
