//! Starting the worker with as little as the system allows. Both sides hand back the same
//! [`Sandboxed`]: the two pipes and a way to stop the process.
//!
//! - Windows (`sandbox/windows.rs`): an AppContainer with no capabilities, a Job Object, mitigations,
//!   all set by the parent before the worker runs a single instruction.
//! - Linux and macOS (`sandbox/unix.rs`): limits set between fork and exec (memory on Linux, core
//!   dumps, death with the wallet, no new privileges, on Linux an empty network of its own); the
//!   rest the worker closes itself before reading anything (`lockdown.rs`), on macOS its memory
//!   cap too.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::{Sandboxed, spawn};
#[cfg(windows)]
pub use windows::{Sandboxed, spawn};
