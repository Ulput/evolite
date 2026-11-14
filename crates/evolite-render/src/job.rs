//! A Windows Job Object around the worker: a memory cap (over it the process is ended), no
//! child processes, no clipboard, no desktop or system settings, and the worker dies with the
//! job — that is, when the wallet closes or drops it.

use std::io;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject,
};

pub(crate) struct Job(HANDLE);

// The handle is owned by this value only and closed once.
unsafe impl Send for Job {}

impl Job {
    /// A new job capped at `memory` bytes, for a worker to be created in it
    /// (`PROC_THREAD_ATTRIBUTE_JOB_LIST`): never a moment when the worker exists outside it
    /// (BUG-168, the worker left suspended when the wallet ended before putting it in).
    pub(crate) fn new(memory: usize) -> io::Result<Self> {
        // SAFETY: plain Win32 calls on a handle this function creates and owns; the structures
        // are zero-initialised (valid for these PODs) and passed with their exact sizes.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Job(handle);

            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
            limits.BasicLimitInformation.ActiveProcessLimit = 1;
            limits.ProcessMemoryLimit = memory;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }

            let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                    | JOB_OBJECT_UILIMIT_EXITWINDOWS
                    | JOB_OBJECT_UILIMIT_GLOBALATOMS
                    | JOB_OBJECT_UILIMIT_HANDLES
                    | JOB_OBJECT_UILIMIT_READCLIPBOARD
                    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
            };
            if SetInformationJobObject(
                job.0,
                JobObjectBasicUIRestrictions,
                (&raw const ui).cast(),
                size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }

            Ok(job)
        }
    }
}

impl Job {
    /// The job's handle, for `PROC_THREAD_ATTRIBUTE_JOB_LIST`.
    pub(crate) fn handle(&self) -> HANDLE {
        self.0
    }

    /// Whether `process` is in this job (BUG-168).
    pub(crate) fn holds(&self, process: HANDLE) -> bool {
        let mut result = 0;
        // SAFETY: two open handles and an out pointer.
        let ok = unsafe {
            windows_sys::Win32::System::JobObjects::IsProcessInJob(process, self.0, &mut result)
        };
        ok != 0 && result != 0
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateJobObjectW and is closed only here. Closing the
        // last handle ends the worker (KILL_ON_JOB_CLOSE).
        unsafe {
            CloseHandle(self.0);
        }
    }
}
