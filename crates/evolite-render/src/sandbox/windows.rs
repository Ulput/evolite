//! Starting the worker with as little as Windows allows.
//!
//! - **AppContainer** with no capabilities: no network (not even loopback), no user files or
//!   registry, low integrity. The only way out is the two pipes it inherits.
//! - **Created suspended**, put into the [`Job`] (memory cap, no children, no clipboard) and
//!   only then resumed, so not one instruction runs outside it.
//! - **Only the pipes are inherited** (an explicit handle list), whatever else the wallet has
//!   open.
//! - **Mitigations from the first instruction**: no dynamic code, no legacy DLL injection
//!   points, no images from the network or of low integrity, strict handle checks, terminate
//!   on heap corruption, ASLR. Win32k is not locked down: the app's executable imports
//!   user32, which needs it to load.
//!
//! The AppContainer must be able to read the executable: on the first start the container is
//! granted read and execute on it (an ACE for the container's SID only).

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GENERIC_EXECUTE, GENERIC_READ, HANDLE, HANDLE_FLAG_INHERIT,
    LocalFree, STILL_ACTIVE, SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT,
    SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP,
    TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::job::Job;

/// The container's name; its SID is derived from it, the same on every start.
const CONTAINER: &str = "evolite.renderer";

/// `PROCESS_CREATION_MITIGATION_POLICY_*` from winnt.h (not in windows-sys).
const MITIGATIONS: u64 = (1 << 12) // heap terminate
    | (1 << 16) // bottom-up ASLR
    | (1 << 20) // high-entropy ASLR
    | (1 << 24) // strict handle checks
    | (1 << 32) // extension points disabled
    | (1 << 36) // dynamic code prohibited
    | (1 << 48) // non-system fonts disabled (GDI; ours are parsed in Rust)
    | (1 << 52) // no images from remote locations
    | (1 << 56) // no images of low integrity
    | (1 << 60); // prefer System32 for DLLs
/// `PROCESS_CREATION_MITIGATION_POLICY_WIN32K_SYSTEM_CALL_DISABLE_ALWAYS_ON`: no window or
/// graphics system calls. Only for a program that links no window code (`evolite-renderer`):
/// one that loads user32 fails to start.
const WIN32K_OFF: u64 = 1 << 28;

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// One argument quoted the way the C runtime splits a command line.
fn quote(arg: &OsStr, out: &mut Vec<u16>) {
    let arg: Vec<u16> = arg.encode_wide().collect();
    let plain = !arg.is_empty()
        && !arg
            .iter()
            .any(|&c| c == b' ' as u16 || c == b'\t' as u16 || c == b'"' as u16);
    if plain {
        out.extend(arg);
        return;
    }
    out.push(b'"' as u16);
    let mut backslashes = 0;
    for c in arg {
        if c == b'\\' as u16 {
            backslashes += 1;
        } else {
            if c == b'"' as u16 {
                // Backslashes before a quote are doubled, and the quote escaped.
                out.extend(std::iter::repeat_n(b'\\' as u16, backslashes + 1));
            }
            backslashes = 0;
        }
        out.push(c);
    }
    // Backslashes before the closing quote are doubled too.
    out.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
    out.push(b'"' as u16);
}

fn command_line(program: &Path, args: &[impl AsRef<OsStr>]) -> Vec<u16> {
    let mut line = Vec::new();
    quote(program.as_os_str(), &mut line);
    for arg in args {
        line.push(b' ' as u16);
        quote(arg.as_ref(), &mut line);
    }
    line.push(0);
    line
}

/// The container's SID. Kept for the life of the process (and never freed), so every start
/// shares one: creating the profile from several threads at once on the first start on a
/// machine fails for all but one of them.
struct Sid(PSID);

// SAFETY: the SID is written once, before it is shared, and only read afterwards.
unsafe impl Send for Sid {}
unsafe impl Sync for Sid {}

/// The container's SID, creating its profile on the first start on this machine.
fn container() -> io::Result<&'static Sid> {
    static SID: OnceLock<Result<Sid, i32>> = OnceLock::new();
    let sid = SID.get_or_init(|| {
        // CI on a fresh windows-latest runner, BUG-168's tests: two processes creating the
        // profile at once both failed (0x8007000A ERROR_BAD_ENVIRONMENT, 0x80070002 not
        // found), the profile half made by the other. The lock above is per process; the
        // creation is held under a mutex of the session too, and a failure is tried again
        // for a while, while another process may still be finishing the profile.
        let _creating = SessionLock::take("Local\\evolite.renderer.profile");
        let mut last = 0;
        for attempt in 0..PROFILE_TRIES {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            match create_or_derive() {
                Ok(sid) => return Ok(Sid(sid)),
                Err(hr) => last = hr,
            }
        }
        Err(last)
    });
    sid.as_ref()
        .map_err(|hr| io::Error::from_raw_os_error(hr & 0xffff))
}

/// How many times [`container`] tries to create or find the profile: 20 × 250 ms.
const PROFILE_TRIES: u32 = 20;

/// The container's profile created, or found when it exists: its SID, or the HRESULT.
fn create_or_derive() -> Result<PSID, i32> {
    let name = wide(OsStr::new(CONTAINER));
    let display = wide(OsStr::new("evolite renderer"));
    let description = wide(OsStr::new("Decodes untrusted pictures for evolite"));
    let mut sid: PSID = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated strings, no capabilities, an out pointer for the SID.
    let hr = unsafe {
        CreateAppContainerProfile(
            name.as_ptr(),
            display.as_ptr(),
            description.as_ptr(),
            std::ptr::null(),
            0,
            &mut sid,
        )
    };
    if hr == (0x8007_0000u32 | ERROR_ALREADY_EXISTS) as i32 {
        // SAFETY: as above.
        let hr = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
        if hr != 0 {
            return Err(hr);
        }
    } else if hr != 0 {
        return Err(hr);
    }
    Ok(sid)
}

/// A named mutex of this session, held until dropped. Without one (it cannot be made, or the
/// wait fails) the caller goes on unlocked: the tries in [`container`] still cover a race.
struct SessionLock(HANDLE);

impl SessionLock {
    fn take(name: &str) -> Option<Self> {
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let name = wide(OsStr::new(name));
        // SAFETY: a NUL-terminated name; the handle is owned by the value returned and closed
        // on drop, or here when the wait fails.
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if handle.is_null() {
                return None;
            }
            // WAIT_ABANDONED (0x80): a process that held it ended; the mutex is ours now.
            match WaitForSingleObject(handle, 30_000) {
                WAIT_OBJECT_0 | 0x80 => Some(Self(handle)),
                _ => {
                    CloseHandle(handle);
                    None
                }
            }
        }
    }
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        // SAFETY: a mutex this value took and owns.
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

/// Lets the container read and run `program`: without it the container cannot even load the
/// executable. Once per path in a wallet process, under a lock; a failure (a read-only
/// install, where the installer must grant it) is left to the start itself to report.
fn grant_execute(program: &Path, sid: &Sid) {
    static GRANTED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let mut granted = GRANTED.lock().unwrap_or_else(|e| e.into_inner());
    if granted.iter().any(|p| p == program) {
        return;
    }
    granted.push(program.to_owned());
    let path = wide(program.as_os_str());
    // SAFETY: Win32 security calls on a NUL-terminated path; the descriptor and the new ACL
    // they allocate are released with LocalFree as documented.
    unsafe {
        let mut old: *mut ACL = std::ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        if GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut old,
            std::ptr::null_mut(),
            &mut descriptor,
        ) != 0
        {
            return;
        }
        let access = EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_READ | GENERIC_EXECUTE,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: sid.0.cast(),
            },
        };
        let mut new: *mut ACL = std::ptr::null_mut();
        if SetEntriesInAclW(1, &access, old, &mut new) == 0 {
            SetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new,
                std::ptr::null(),
            );
            LocalFree(new.cast());
        }
        LocalFree(descriptor);
    }
}

/// A handle closed when dropped (pipe ends the child keeps).
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: an open handle owned by this value.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// An inheritable pipe: `(read end, write end)`.
fn pipe() -> io::Result<(Handle, Handle)> {
    let inherit = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: out pointers for two handles, valid attributes.
    if unsafe { CreatePipe(&mut read, &mut write, &inherit, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((Handle(read), Handle(write)))
}

/// The wallet's end of a pipe must not leak into the child.
fn keep_here(handle: Handle) -> io::Result<File> {
    // SAFETY: an open handle we own; ownership passes to the File.
    unsafe {
        if SetHandleInformation(handle.0, HANDLE_FLAG_INHERIT, 0) == 0 {
            return Err(io::Error::last_os_error());
        }
        let raw = handle.0;
        std::mem::forget(handle);
        Ok(File::from_raw_handle(raw))
    }
}

/// The started worker. Dropping it ends the worker (through the job).
pub struct Sandboxed {
    pub stdin: File,
    pub stdout: File,
    process: OwnedHandle,
    _job: Job,
}

impl Sandboxed {
    pub fn kill(&self) {
        // SAFETY: our process handle, open until self drops.
        unsafe {
            TerminateProcess(self.process.as_raw_handle() as HANDLE, 1);
        }
    }

    /// Waits until the worker has exited.
    pub fn wait(&self) -> bool {
        // SAFETY: as above.
        unsafe {
            WaitForSingleObject(self.process.as_raw_handle() as HANDLE, INFINITE) == WAIT_OBJECT_0
        }
    }

    /// The exit code, once the worker has exited.
    pub fn exit_code(&self) -> Option<u32> {
        let mut code = 0u32;
        // SAFETY: as above, with an out pointer for the code.
        let ok = unsafe { GetExitCodeProcess(self.process.as_raw_handle() as HANDLE, &mut code) };
        (ok != 0 && code != STILL_ACTIVE as u32).then_some(code)
    }

    /// The worker's process id (BUG-168: a test follows it after its parent is gone).
    pub fn id(&self) -> u32 {
        // SAFETY: as above.
        unsafe {
            windows_sys::Win32::System::Threading::GetProcessId(
                self.process.as_raw_handle() as HANDLE
            )
        }
    }

    /// Whether the worker is in its own job, the one that ends it when the wallet's handle
    /// closes (BUG-168), not only in one it inherited.
    pub fn in_its_job(&self) -> bool {
        self._job.holds(self.process.as_raw_handle() as HANDLE)
    }
}

/// Starts `program args` in the sandbox, with `memory` as the job's cap; `win32k_off` also
/// takes away the window and graphics system calls.
pub fn spawn(
    program: &Path,
    args: &[impl AsRef<OsStr>],
    memory: usize,
    win32k_off: bool,
) -> io::Result<Sandboxed> {
    let sid = container()?;
    grant_execute(program, sid);

    let (child_in, wallet_out) = pipe()?;
    let (wallet_in, child_out) = pipe()?;
    let stdin = keep_here(wallet_out)?;
    let stdout = keep_here(wallet_in)?;

    let inherited: [HANDLE; 2] = [child_in.0, child_out.0];
    let mut capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        Capabilities: std::ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let mitigations: u64 = MITIGATIONS | if win32k_off { WIN32K_OFF } else { 0 };
    // The job first, and the worker created in it (BUG-168): with the job put on after
    // creation, a wallet that ended in between left the worker suspended for good — three
    // times on 2026-11-14, each holding the executable. Its handle closes with the wallet,
    // however the wallet ends, and the job's kill-on-close ends the worker.
    let job = Job::new(memory)?;
    let jobs: [HANDLE; 1] = [job.handle()];

    // SAFETY: the attribute list is sized by the first call and lives in `list` until
    // DeleteProcThreadAttributeList; every attribute points at a local that outlives
    // CreateProcessW; all handles are owned here and closed on every path.
    unsafe {
        let mut size = 0usize;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 4, 0, &mut size);
        let mut list = vec![0usize; size.div_ceil(size_of::<usize>())];
        let attributes: LPPROC_THREAD_ATTRIBUTE_LIST = list.as_mut_ptr().cast();
        if InitializeProcThreadAttributeList(attributes, 4, 0, &mut size) == 0 {
            return Err(io::Error::last_os_error());
        }
        struct List(LPPROC_THREAD_ATTRIBUTE_LIST);
        impl Drop for List {
            fn drop(&mut self) {
                // SAFETY: initialised above, deleted once.
                unsafe { DeleteProcThreadAttributeList(self.0) }
            }
        }
        let _list = List(attributes);
        for (attribute, value, len) in [
            (
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                inherited.as_ptr().cast(),
                size_of_val(&inherited),
            ),
            (
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                (&raw mut capabilities).cast(),
                size_of::<SECURITY_CAPABILITIES>(),
            ),
            (
                PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY as usize,
                (&raw const mitigations).cast(),
                size_of::<u64>(),
            ),
            (
                windows_sys::Win32::System::Threading::PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                jobs.as_ptr().cast(),
                size_of_val(&jobs),
            ),
        ] {
            if UpdateProcThreadAttribute(
                attributes,
                0,
                attribute,
                value,
                len,
                std::ptr::null_mut(),
                std::ptr::null(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }

        let mut startup: STARTUPINFOEXW = std::mem::zeroed();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = child_in.0;
        startup.StartupInfo.hStdOutput = child_out.0;
        startup.StartupInfo.hStdError = std::ptr::null_mut();
        startup.lpAttributeList = attributes;

        let application = wide(program.as_os_str());
        let mut line = command_line(program, args);
        let mut info: PROCESS_INFORMATION = std::mem::zeroed();
        if CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
            std::ptr::null(),
            std::ptr::null(),
            &startup.StartupInfo,
            &mut info,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let process = OwnedHandle::from_raw_handle(info.hProcess);
        let thread = Handle(info.hThread);
        // BUG-168's test: the wallet ends right here, the worker created and not yet running,
        // as `--screenshot` or a test that ends while the art pool starts one. Debug builds only.
        #[cfg(debug_assertions)]
        if std::env::var_os("EVOLITE_RENDER_EXIT_AFTER_CREATE").is_some() {
            use std::io::Write as _;
            println!("worker {} created", info.dwProcessId);
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        // The child holds its own copies of its pipe ends now.
        drop(child_in);
        drop(child_out);

        if ResumeThread(thread.0) == u32::MAX {
            let e = io::Error::last_os_error();
            TerminateProcess(info.hProcess, 1);
            return Err(e);
        }
        Ok(Sandboxed {
            stdin,
            stdout,
            process,
            _job: job,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(args: &[&str]) -> String {
        String::from_utf16(&command_line(Path::new(r"C:\a b\evolite.exe"), args)).unwrap()
    }

    #[test]
    fn quotes_like_the_c_runtime() {
        assert_eq!(
            line(&["--render-worker"]),
            "\"C:\\a b\\evolite.exe\" --render-worker\0"
        );
        assert_eq!(line(&["a\"b"]), "\"C:\\a b\\evolite.exe\" \"a\\\"b\"\0");
        assert_eq!(
            line(&[r"c:\x y\"]),
            "\"C:\\a b\\evolite.exe\" \"c:\\x y\\\\\"\0"
        );
        assert_eq!(line(&[""]), "\"C:\\a b\\evolite.exe\" \"\"\0");
    }
}

/// Deletes the container's profile, so the next start is a first start on this machine:
/// `cargo test -p evolite-render forget_the_container -- --ignored`, then the other tests.
#[cfg(test)]
#[test]
#[ignore = "changes this machine's AppContainer profiles"]
fn forget_the_container() {
    let name = wide(OsStr::new(CONTAINER));
    // SAFETY: a NUL-terminated name.
    let hr = unsafe {
        windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile(name.as_ptr())
    };
    assert!(hr == 0 || hr == (0x8007_0000u32 | 2) as i32, "{hr:#x}");
}
