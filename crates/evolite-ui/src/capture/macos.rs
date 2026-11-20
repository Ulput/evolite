//! SEC-10: macOS. While the phrase or a key is shown, the window's sharing type is `None`:
//! `screencapture`, ⌘⇧3/4/5 and programs that read windows through the older Core Graphics
//! calls get no picture of it. ScreenCaptureKit (macOS 14 and later: QuickTime's recording,
//! OBS, newer screen sharing) may still see it, and no public call refuses that, so a warning
//! comes before the phrase too ([`super::phrase_warning`]). Otherwise the sharing type is back
//! to `ReadOnly`, the system's own.
//!
//! Its own file so that `tests/macos_capture.rs` builds it on a window it makes.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_app_kit::{NSView, NSWindowSharingType};

/// Sets the sharing type of the window that holds `ns_view`, and reads it back: false when
/// the view has no window or the window kept another type.
///
/// # Safety
///
/// `ns_view` points to a live `NSView`, and this runs on the main thread.
pub unsafe fn set_sharing(ns_view: NonNull<c_void>, hidden: bool) -> bool {
    // SAFETY: by the caller.
    let view: &NSView = unsafe { ns_view.cast().as_ref() };
    let Some(window) = view.window() else {
        return false;
    };
    let wanted = if hidden {
        NSWindowSharingType::None
    } else {
        NSWindowSharingType::ReadOnly
    };
    window.setSharingType(wanted);
    window.sharingType() == wanted
}
