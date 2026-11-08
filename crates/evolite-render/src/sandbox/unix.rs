//! The worker on Linux and macOS: a child process with its limits set between fork and exec.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

/// The started worker. Dropping it does not stop it: call [`Sandboxed::kill`].
pub struct Sandboxed {
    pub stdin: File,
    pub stdout: File,
    child: Mutex<Child>,
}

impl Sandboxed {
    pub fn kill(&self) {
        let _ = self.child.lock().unwrap_or_else(|e| e.into_inner()).kill();
    }

    /// Waits until the worker has exited.
    pub fn wait(&self) -> bool {
        self.child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .wait()
            .is_ok()
    }

    /// The exit code, once the worker has exited (a signal reads as 128 + its number).
    pub fn exit_code(&self) -> Option<u32> {
        use std::os::unix::process::ExitStatusExt;
        let status = self
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_wait()
            .ok()??;
        status
            .code()
            .map(|c| c as u32)
            .or(status.signal().map(|s| 128 + s as u32))
    }
}

/// Between fork and exec: only async-signal-safe calls here (plain system calls). The memory
/// cap on macOS is the worker's own (`lockdown.rs`): XNU refuses it here.
fn confine(parent: libc::pid_t, memory: usize) -> io::Result<()> {
    let last = io::Error::last_os_error;
    // SAFETY: setrlimit/prctl/getppid/unshare/_exit are system calls, safe after fork.
    unsafe {
        #[cfg(not(target_os = "macos"))]
        {
            let cap = libc::rlimit {
                rlim_cur: memory as libc::rlim_t,
                rlim_max: memory as libc::rlim_t,
            };
            if libc::setrlimit(libc::RLIMIT_AS, &cap) != 0 {
                return Err(last());
            }
        }
        #[cfg(target_os = "macos")]
        let _ = memory;
        let none = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &none) != 0 {
            return Err(last());
        }
        #[cfg(target_os = "linux")]
        {
            // Dies with the wallet; and if the wallet died before this line, now.
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(last());
            }
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(last());
            }
            // A network of its own with nothing in it, where unprivileged user namespaces are
            // allowed; elsewhere seccomp and Landlock still close the sockets.
            let _ = libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = parent;
    }
    Ok(())
}

/// Starts `program args` with `memory` as its address-space cap.
/// `_win32k_off`: Windows only; nothing to turn off here.
pub fn spawn(
    program: &Path,
    args: &[impl AsRef<OsStr>],
    memory: usize,
    _win32k_off: bool,
) -> io::Result<Sandboxed> {
    let parent = std::process::id() as libc::pid_t;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "macos")]
    command.env(crate::lockdown::MEMORY_ENV, memory.to_string());
    // SAFETY: `confine` makes only system calls, which are async-signal-safe.
    unsafe {
        command.pre_exec(move || confine(parent, memory));
    }
    let mut child = command.spawn()?;
    let stdin = child.stdin.take().ok_or(io::ErrorKind::BrokenPipe)?;
    let stdout = child.stdout.take().ok_or(io::ErrorKind::BrokenPipe)?;
    Ok(Sandboxed {
        stdin: File::from(OwnedFd::from(stdin)),
        stdout: File::from(OwnedFd::from(stdout)),
        child: Mutex::new(child),
    })
}
