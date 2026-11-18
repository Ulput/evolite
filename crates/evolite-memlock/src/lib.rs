//! SEC-5 (П6, the customer's yes of 2026-11-16; the architect's option B): the open wallet's
//! phrase and seed kept out of the page file and the hibernation file. [`Locked`] is a buffer of
//! `N` bytes in a page of its own — allocated page-aligned and a whole page long, so the locked
//! page holds secrets only and never moves — locked to RAM (`VirtualLock` on Windows, `mlock` on
//! Unix), zeroed, unlocked and freed when it goes.
//!
//! A lock the system refuses never stops the wallet: on Windows the minimum working set is raised
//! by a few pages once and the lock tried again; if that fails too, the buffer works unlocked. The
//! journal says once a run whether it held (`memlock {ok, error}`).
//!
//! This is the one crate of the core with `unsafe` code: every other one is
//! `#![forbid(unsafe_code)]`. Each block says why it is sound.

#![deny(unsafe_op_in_unsafe_fn)]

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::sync::atomic::{AtomicBool, Ordering};

/// `N` bytes in a page of their own, locked to RAM while they live (see the crate).
pub struct Locked<const N: usize> {
    ptr: std::ptr::NonNull<[u8; N]>,
    layout: Layout,
    locked: bool,
}

// SAFETY: `Locked` owns its allocation alone, as a `Box<[u8; N]>` would.
unsafe impl<const N: usize> Send for Locked<N> {}
// SAFETY: `&Locked` gives only `&[u8; N]`: shared reads.
unsafe impl<const N: usize> Sync for Locked<N> {}

impl<const N: usize> Locked<N> {
    /// `N` zero bytes, locked to RAM if the system lets it. `N` must fit in one page.
    pub fn new() -> Self {
        let page = page_size();
        assert!(N > 0 && N <= page, "a Locked buffer is one page at most");
        let layout = Layout::from_size_align(page, page).expect("a page is a valid layout");
        // SAFETY: `layout` has a non-zero size.
        let raw = unsafe { alloc_zeroed(layout) };
        let Some(ptr) = std::ptr::NonNull::new(raw.cast::<[u8; N]>()) else {
            std::alloc::handle_alloc_error(layout);
        };
        let locked = lock(raw, page);
        say_once(&locked);
        let locked = locked.is_ok();
        Self {
            ptr,
            layout,
            locked,
        }
    }

    /// Whether the system locked it to RAM.
    pub fn is_locked(&self) -> bool {
        self.locked
    }
}

impl<const N: usize> Default for Locked<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> AsRef<[u8; N]> for Locked<N> {
    fn as_ref(&self) -> &[u8; N] {
        // SAFETY: `ptr` is this buffer's own live, initialised (zeroed) allocation.
        unsafe { self.ptr.as_ref() }
    }
}

impl<const N: usize> AsMut<[u8; N]> for Locked<N> {
    fn as_mut(&mut self) -> &mut [u8; N] {
        // SAFETY: as in `as_ref`, and `&mut self` makes this the only reference.
        unsafe { self.ptr.as_mut() }
    }
}

impl<const N: usize> Drop for Locked<N> {
    fn drop(&mut self) {
        let raw = self.ptr.as_ptr().cast::<u8>();
        // Zeroed byte by byte with volatile writes: the compiler may not drop them as dead.
        for i in 0..self.layout.size() {
            // SAFETY: `i` is inside this allocation of `layout.size()` bytes.
            unsafe { raw.add(i).write_volatile(0) };
        }
        std::sync::atomic::compiler_fence(Ordering::SeqCst);
        #[cfg(test)]
        tests::after_zeroing(raw, self.layout.size());
        if self.locked {
            unlock(raw, self.layout.size());
        }
        // SAFETY: allocated in `new` with this same `layout`, freed once.
        unsafe { dealloc(raw, self.layout) };
    }
}

impl<const N: usize> std::fmt::Debug for Locked<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Locked(hidden)")
    }
}

/// The journal's line said this run.
static SAID: AtomicBool = AtomicBool::new(false);

/// `memlock {ok, error}` once a run: whether the first lock held, and if not why.
fn say_once(locked: &Result<(), &'static str>) {
    if SAID.swap(true, Ordering::Relaxed) {
        return;
    }
    evolite_common::diag::record(evolite_common::diag::Record::Memlock {
        ok: locked.is_ok(),
        error: locked.err().unwrap_or(""),
    });
}

#[cfg(test)]
static FAIL: AtomicBool = AtomicBool::new(false);

/// Whether a lock should fail now (tests force it).
fn forced_failure() -> bool {
    #[cfg(test)]
    {
        FAIL.load(Ordering::Relaxed)
    }
    #[cfg(not(test))]
    {
        false
    }
}

#[cfg(windows)]
fn page_size() -> usize {
    let mut info = std::mem::MaybeUninit::uninit();
    // SAFETY: `GetSystemInfo` fills the whole struct it is given.
    let info = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemInfo(info.as_mut_ptr());
        info.assume_init()
    };
    info.dwPageSize as usize
}

#[cfg(unix)]
fn page_size() -> usize {
    // SAFETY: `sysconf` only reads a system constant.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(size).unwrap_or(4096)
}

#[cfg(not(any(windows, unix)))]
fn page_size() -> usize {
    4096
}

/// Locks `len` bytes at `ptr` to RAM; why not, when not. The error is read right after the
/// `VirtualLock` that failed (review r1, Should 2): the working set's calls in between would
/// overwrite it.
#[cfg(windows)]
fn lock(ptr: *mut u8, len: usize) -> Result<(), &'static str> {
    use windows_sys::Win32::System::Memory::VirtualLock;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessWorkingSetSize, SetProcessWorkingSetSize,
    };
    if forced_failure() {
        return Err("other");
    }
    // SAFETY: `ptr` is a live allocation of `len` bytes owned by the caller.
    if unsafe { VirtualLock(ptr.cast(), len) } != 0 {
        return Ok(());
    }
    let first = last_error();
    // The working set's quota: a few pages more, once, and again (the architect).
    const MORE: usize = 64 * 1024;
    let (mut min, mut max) = (0usize, 0usize);
    // SAFETY: the current process's pseudo-handle; two out-pointers to live locals.
    let raised = unsafe {
        let me = GetCurrentProcess();
        GetProcessWorkingSetSize(me, &mut min, &mut max) != 0
            && SetProcessWorkingSetSize(me, min + MORE, max + MORE) != 0
    };
    if !raised {
        return Err(first);
    }
    // SAFETY: as above.
    if unsafe { VirtualLock(ptr.cast(), len) } != 0 {
        return Ok(());
    }
    Err(last_error())
}

#[cfg(windows)]
fn unlock(ptr: *mut u8, len: usize) {
    // SAFETY: the same live allocation that `lock` locked.
    unsafe {
        windows_sys::Win32::System::Memory::VirtualUnlock(ptr.cast(), len);
    }
}

#[cfg(windows)]
fn last_error() -> &'static str {
    // SAFETY: reads this thread's last error code.
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    // ERROR_WORKING_SET_QUOTA
    if code == 1453 { "quota" } else { "other" }
}

#[cfg(unix)]
fn lock(ptr: *mut u8, len: usize) -> Result<(), &'static str> {
    if forced_failure() {
        return Err("other");
    }
    // SAFETY: `ptr` is a live allocation of `len` bytes owned by the caller.
    if unsafe { libc::mlock(ptr.cast(), len) } == 0 {
        Ok(())
    } else {
        Err(last_error())
    }
}

#[cfg(unix)]
fn unlock(ptr: *mut u8, len: usize) {
    // SAFETY: the same live allocation that `lock` locked.
    unsafe {
        libc::munlock(ptr.cast(), len);
    }
}

#[cfg(unix)]
fn last_error() -> &'static str {
    match std::io::Error::last_os_error().raw_os_error() {
        // RLIMIT_MEMLOCK reached, or no right to lock.
        Some(libc::ENOMEM) | Some(libc::EPERM) => "quota",
        _ => "other",
    }
}

#[cfg(not(any(windows, unix)))]
fn lock(_: *mut u8, _: usize) -> Result<(), &'static str> {
    Err("other")
}

#[cfg(not(any(windows, unix)))]
fn unlock(_: *mut u8, _: usize) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        /// The bytes of the last buffer of this thread, read right after zeroing, before free.
        static SEEN: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    }

    /// The test-only hook `drop` calls after zeroing, before the page is freed.
    pub(super) fn after_zeroing(raw: *const u8, len: usize) {
        // SAFETY: called from `drop` on its own live allocation of `len` bytes.
        let bytes = unsafe { std::slice::from_raw_parts(raw, len) }.to_vec();
        SEEN.with(|s| *s.borrow_mut() = Some(bytes));
    }

    /// The tests that force a failure take turns with the others (one switch per process).
    fn turn() -> std::sync::MutexGuard<'static, ()> {
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        TURN.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A buffer of 256 bytes reads back what was written, and is locked to RAM here.
    #[test]
    fn a_buffer_reads_back_what_was_written() {
        let _turn = turn();
        let mut phrase = Locked::<256>::new();
        phrase.as_mut()[..5].copy_from_slice(b"words");
        assert_eq!(&phrase.as_ref()[..5], b"words");
        assert!(phrase.as_ref()[5..].iter().all(|b| *b == 0));
        assert!(phrase.is_locked(), "the system locked one page");
        assert_eq!(format!("{phrase:?}"), "Locked(hidden)");
    }

    /// After a drop, every byte of its page was zero before the page was freed.
    #[test]
    fn a_dropped_buffer_is_zeroed_before_it_is_freed() {
        let _turn = turn();
        let mut seed = Locked::<64>::new();
        seed.as_mut().fill(0xAB);
        drop(seed);
        let seen = SEEN.with(|s| s.borrow_mut().take()).expect("the hook ran");
        assert_eq!(seen.len(), page_size());
        assert!(seen.iter().all(|b| *b == 0));
    }

    /// A lock the system refuses leaves a working buffer, unlocked, and the journal's line
    /// (said once a run) says it did not hold.
    #[test]
    fn a_refused_lock_still_works_and_is_said() {
        let _turn = turn();
        FAIL.store(true, Ordering::Relaxed);
        SAID.store(false, Ordering::Relaxed);
        let mut seed = Locked::<64>::new();
        FAIL.store(false, Ordering::Relaxed);
        seed.as_mut()[0] = 9;
        assert_eq!(seed.as_ref()[0], 9);
        assert!(!seed.is_locked());
        assert!(SAID.load(Ordering::Relaxed), "the line was written");
    }
}
