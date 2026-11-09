//! What the worker does to itself once the fonts are found and before the first untrusted
//! byte. On Windows the parent has already put it in an AppContainer ([`crate::sandbox`]);
//! on Linux and macOS a process can only close the doors from the inside:
//!
//! - **Linux**: Landlock — of the file system only the font folders stay, read-only; TCP
//!   neither out nor in (kernel 6.7+). Then seccomp — no sockets, no new programs, no new
//!   processes (threads stay: they are how pictures are drawn), no signals to anyone but the
//!   worker itself (BUG-96), no
//!   debugging or reading other processes, no namespaces, mounts or kernel modules, no changing
//!   of a file's length or attributes by its path (Landlock leaves those: `truncate` before
//!   ABI 3, `chmod`, `chown`, `setxattr` and `utimensat` at every ABI), and no
//!   memory made executable (what shellcode needs). Landlock works on kernels from 5.13 and
//!   is applied as far as the kernel goes, but a kernel without it is refused, as one without
//!   seccomp: seccomp alone leaves the user's files open (BUG-95).
//! - **macOS**: the memory cap, then a Seatbelt profile — everything denied but reading the
//!   font folders. The cap is set here and not between fork and exec, as on Linux: XNU refuses
//!   a limit below what the process has mapped, and the dyld shared region alone is 6 GB. So
//!   it is counted above what is mapped now (BUG-95).

use std::path::PathBuf;

use crate::frames::SandboxRefusal;

/// Why the worker did not close itself: the kind for the wallet, the words for a test's log.
#[derive(Debug)]
pub struct Refusal {
    pub kind: SandboxRefusal,
    pub detail: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.detail)
    }
}

/// `result`, an error of it being a refusal of this `kind`.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn refuse(kind: SandboxRefusal, result: Result<(), String>) -> Result<(), Refusal> {
    result.map_err(|detail| Refusal { kind, detail })
}

/// Where the wallet tells a macOS worker how much memory it may take, in bytes.
#[cfg(target_os = "macos")]
pub(crate) const MEMORY_ENV: &str = "EVOLITE_RENDER_MEMORY";

/// The address-space cap for a process with `mapped` bytes mapped now, allowed `memory` more
/// (as the wallet wrote it). `Err`: no usable size, and no cap means no serving.
#[cfg(any(target_os = "macos", test))]
fn cap_above(mapped: u64, memory: Option<&str>) -> Result<u64, String> {
    let memory = memory
        .and_then(|text| text.parse::<u64>().ok())
        .filter(|bytes| *bytes > 0)
        .ok_or("memory cap: no size given")?;
    mapped
        .checked_add(memory)
        .ok_or_else(|| format!("memory cap: {memory} above {mapped} bytes does not fit"))
}

/// The memory cap, where the worker sets it itself. `Err`: the worker must not serve.
pub(crate) fn cap_memory() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    macos::cap_memory()?;
    Ok(())
}

/// Closes what can be closed. `Err`: the worker must not serve.
pub(crate) fn apply(font_dirs: &[PathBuf]) -> Result<(), Refusal> {
    #[cfg(target_os = "linux")]
    {
        refuse(SandboxRefusal::NoFileIsolation, linux::landlock(font_dirs))?;
        refuse(SandboxRefusal::NoSandbox, linux::seccomp())?;
    }
    #[cfg(target_os = "macos")]
    {
        // Measured after the fonts, so what is mapped for them does not eat into the cap.
        refuse(SandboxRefusal::NoMemoryCap, cap_memory())?;
        refuse(SandboxRefusal::NoSandbox, macos::seatbelt(font_dirs))?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = font_dirs;
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use landlock::{
        ABI, Access, AccessFs, AccessNet, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
        path_beneath_rules,
    };
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule, sock_filter,
    };

    pub(super) fn landlock(font_dirs: &[PathBuf]) -> Result<(), String> {
        let fs = ABI::V5;
        let status = Ruleset::default()
            .handle_access(AccessFs::from_all(fs))
            .and_then(|r| r.handle_access(AccessNet::from_all(ABI::V4)))
            .and_then(|r| r.create())
            .and_then(|r| r.add_rules(path_beneath_rules(font_dirs, AccessFs::from_read(fs))))
            .and_then(|r| r.restrict_self())
            .map_err(|e| format!("landlock: {e}"))?;
        // Best effort means an older kernel may close less; closing nothing leaves the user's
        // files open to a compromised decoder.
        if matches!(status.ruleset, RulesetStatus::NotEnforced) {
            return Err("landlock: not enforced on this system".into());
        }
        Ok(())
    }

    /// `fchmodat2`, the same number on every architecture; `libc` does not name it on all.
    const FCHMODAT2: i64 = 452;
    /// The same on every architecture too, and `libc` names them on none of ours.
    const SETXATTRAT: i64 = 463;
    const REMOVEXATTRAT: i64 = 466;
    const FILE_SETATTR: i64 = 469;
    /// `fcntl`'s other way to name a file's owner; `libc` does not name it for glibc.
    const F_SETOWN_EX: u64 = 15;

    /// Set in a call's number when it is made through the x32 ABI. seccompiler's check of the
    /// architecture does not tell x32 from x86-64, and no number in the main filter matches.
    const X32_BIT: u32 = 0x4000_0000;

    /// Calls that answer `EPERM` from now on, whatever their arguments.
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
            // No signal to another process: the wallet and the user's other programs. A pidfd
            // is a way to signal, and to take the descriptors of, whoever it names. `tkill`
            // takes any thread on the system; glibc uses `tgkill`, which has its own rule.
            libc::SYS_kill,
            libc::SYS_tkill,
            libc::SYS_rt_sigqueueinfo,
            libc::SYS_rt_tgsigqueueinfo,
            libc::SYS_pidfd_send_signal,
            libc::SYS_pidfd_open,
            libc::SYS_pidfd_getfd,
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
            // What Landlock does not stop, and a decoder has no use for.
            libc::SYS_truncate,
            libc::SYS_fchmod,
            libc::SYS_fchmodat,
            FCHMODAT2,
            libc::SYS_fchown,
            libc::SYS_fchownat,
            libc::SYS_utimensat,
            libc::SYS_setxattr,
            libc::SYS_lsetxattr,
            libc::SYS_fsetxattr,
            libc::SYS_removexattr,
            libc::SYS_lremovexattr,
            libc::SYS_fremovexattr,
            SETXATTRAT,
            REMOVEXATTRAT,
            FILE_SETATTR,
        ];
        #[cfg(target_arch = "x86_64")]
        calls.extend([
            libc::SYS_fork,
            libc::SYS_vfork,
            libc::SYS_chmod,
            libc::SYS_chown,
            libc::SYS_lchown,
            libc::SYS_utime,
            libc::SYS_utimes,
            libc::SYS_futimesat,
        ]);
        calls
    }

    /// The rule that matches when the low half of argument `arg` compares to `value` by `op`.
    /// The low half is all that counts here: pids are `pid_t` and the `fcntl` command is
    /// `unsigned int`, `clone` keeps only the low 32 bits of its flags, `PROT_EXEC` is a low bit.
    fn when(arg: u8, op: SeccompCmpOp, value: u64) -> Result<SeccompRule, String> {
        SeccompCondition::new(arg, SeccompCmpArgLen::Dword, op, value)
            .and_then(|c| SeccompRule::new(vec![c]))
            .map_err(|e| format!("seccomp rule: {e}"))
    }

    /// The argument of `mmap`/`mprotect` with the protection, and the rule that matches when it
    /// asks for executable memory.
    fn asks_for_exec(arg: u8) -> Result<SeccompRule, String> {
        let exec = libc::PROT_EXEC as u64;
        when(arg, SeccompCmpOp::MaskedEq(exec), exec)
    }

    /// When `clone` is refused: for anything but a thread of this process. The animation and
    /// model renderers start threads after the filter is in place, so `clone` itself must stay.
    /// `CLONE_THREAD` is what makes a thread, and the kernel takes it only together with
    /// `CLONE_SIGHAND` and `CLONE_VM`; glibc and musl pass it for every thread. A rule says what
    /// is refused and rules are alternatives, so a missing flag and each new namespace are
    /// rules of their own. The flags are the first argument on all three architectures
    /// seccompiler builds for (not on s390x, which it refuses).
    fn not_a_thread() -> Result<Vec<SeccompRule>, String> {
        let thread = libc::CLONE_THREAD as u64;
        let mut rules = vec![when(0, SeccompCmpOp::MaskedEq(thread), 0)?];
        for namespace in [
            libc::CLONE_NEWNS,
            libc::CLONE_NEWCGROUP,
            libc::CLONE_NEWUTS,
            libc::CLONE_NEWIPC,
            libc::CLONE_NEWUSER,
            libc::CLONE_NEWPID,
            libc::CLONE_NEWNET,
        ] {
            let flag = namespace as u64;
            rules.push(when(0, SeccompCmpOp::MaskedEq(flag), flag)?);
        }
        Ok(rules)
    }

    /// The calls that must look as if this kernel did not have them: `ENOSYS`, where the main
    /// filter answers `EPERM`, which is why this is a filter of its own: one filter has one
    /// answer. Of several the kernel takes the strongest, and of equals the newest filter's, so
    /// `clone3` must stay out of the main filter: its `EPERM` would win and end all threads.
    ///
    /// - `clone3`: its flags are in a struct seccomp cannot read, so it cannot be told from a
    ///   fork. glibc 2.34+ tries it first for every thread and goes to `clone` only on `ENOSYS`;
    ///   `EPERM` here would be no threads and no animated pictures.
    /// - Anything through the x32 ABI, which no number in the main filter matches. Our own
    ///   binary is 64-bit and never makes such a call.
    fn absent_calls() -> BpfProgram {
        let step = |code, jt, jf, k| sock_filter { code, jt, jf, k };
        // BPF_LD | BPF_W | BPF_ABS, from offset 0: the number in `seccomp_data`.
        const LOAD_NUMBER: u16 = 0x20;
        // BPF_JMP | BPF_K with BPF_JEQ and BPF_JGE; BPF_RET | BPF_K.
        const JUMP_EQUAL: u16 = 0x15;
        const JUMP_AT_LEAST: u16 = 0x35;
        const RETURN: u16 = 0x06;
        vec![
            step(LOAD_NUMBER, 0, 0, 0),
            step(JUMP_EQUAL, 2, 0, libc::SYS_clone3 as u32),
            step(JUMP_AT_LEAST, 1, 0, X32_BIT),
            step(RETURN, 0, 0, libc::SECCOMP_RET_ALLOW),
            step(RETURN, 0, 0, libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32),
        ]
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
        rules.insert(libc::SYS_clone, not_a_thread()?);
        // tgkill(tgid, tid, signal) reaches only threads of `tgid`. Our own stays: `abort()` is
        // how glibc ends the process on an allocation over the memory cap or a double panic.
        let own = u64::from(std::process::id());
        rules.insert(libc::SYS_tgkill, vec![when(0, SeccompCmpOp::Ne, own)?]);
        // A file owner gets SIGIO, by default fatal, when its file is ready: a signal by way
        // of a pipe. Nothing here sets an owner.
        rules.insert(
            libc::SYS_fcntl,
            vec![
                when(1, SeccompCmpOp::Eq, libc::F_SETOWN as u64)?,
                when(1, SeccompCmpOp::Eq, F_SETOWN_EX)?,
            ],
        );
        // prlimit64(pid, …): libc asks for its own limits with pid 0. On someone else, a CPU or
        // file-size limit is a fatal signal later.
        rules.insert(libc::SYS_prlimit64, vec![when(0, SeccompCmpOp::Ne, 0)?]);
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
        for program in [absent_calls(), program] {
            seccompiler::apply_filter_all_threads(&program).map_err(|e| format!("seccomp: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CStr, CString, c_char, c_int};
    use std::path::PathBuf;

    #[allow(non_upper_case_globals, reason = "the system's own name")]
    unsafe extern "C" {
        fn sandbox_init(profile: *const c_char, flags: u64, error: *mut *mut c_char) -> c_int;
        fn sandbox_free_error(error: *mut c_char);
        static mach_task_self_: u32;
        fn task_info(task: u32, flavor: c_int, info: *mut c_int, count: *mut u32) -> c_int;
    }

    /// Bytes mapped by this process: `virtual_size`, the first field of
    /// `mach_task_basic_info`, the size XNU holds the limit against.
    fn mapped() -> Result<u64, String> {
        const MACH_TASK_BASIC_INFO: c_int = 20;
        const COUNT: u32 = 12;
        let mut info = [0u32; COUNT as usize];
        let mut count = COUNT;
        // SAFETY: this task's own port and a buffer of the 12 words the flavor fills.
        let status = unsafe {
            task_info(
                mach_task_self_,
                MACH_TASK_BASIC_INFO,
                info.as_mut_ptr().cast(),
                &mut count,
            )
        };
        if status != 0 {
            return Err(format!("memory cap: task_info answered {status}"));
        }
        Ok(u64::from(info[0]) | (u64::from(info[1]) << 32))
    }

    /// Above what is mapped now, by what the wallet allows. A limit lowered this way cannot be
    /// raised again: both the soft and the hard one are set.
    pub(super) fn cap_memory() -> Result<(), String> {
        let mapped = mapped()?;
        let memory = std::env::var(super::MEMORY_ENV).ok();
        let cap = super::cap_above(mapped, memory.as_deref())?;
        let limit = libc::rlimit {
            rlim_cur: cap,
            rlim_max: cap,
        };
        // SAFETY: a plain system call with a limit that lives through it.
        if unsafe { libc::setrlimit(libc::RLIMIT_AS, &limit) } != 0 {
            let error = std::io::Error::last_os_error();
            return Err(format!(
                "memory cap: setrlimit({cap}) with {mapped} bytes mapped: {error}"
            ));
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_is_counted_above_what_is_mapped() {
        assert_eq!(cap_above(6_000, Some("512")), Ok(6_512));
    }

    #[test]
    fn no_cap_without_a_size_that_fits() {
        for bad in [None, Some(""), Some("0"), Some("-1"), Some("lots")] {
            assert!(cap_above(1, bad).is_err(), "{bad:?}");
        }
        assert!(cap_above(u64::MAX, Some("1")).is_err());
    }
}
