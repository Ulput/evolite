//! Keeping the recovery phrase out of screenshots and screen recordings. While a screen with the
//! phrase on it is drawn, the window asks Windows to leave it out of captures
//! (`WDA_EXCLUDEFROMCAPTURE`, Windows 10 2004 and later): screenshot tools, recorders, screen
//! sharing and remote-desktop viewers get a blank window instead. The app's own `--screenshot`
//! reads egui's frame, not the screen, and is not affected. Elsewhere there is no such switch.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set by a screen with the phrase each frame it is drawn, read once per frame by the app.
#[cfg(not(test))]
static WANTED: AtomicBool = AtomicBool::new(false);

// One request per test thread, as the interface language (`i18n::CURRENT`): an app test that
// draws a phrase screen, or takes the request at the end of its frame, on its own thread never
// sets or clears it for a test running beside it. The app draws on the thread that runs it, as
// egui_kittest does.
#[cfg(test)]
thread_local! {
    static WANTED: AtomicBool = const { AtomicBool::new(false) };
}

fn wanted<T>(f: impl FnOnce(&AtomicBool) -> T) -> T {
    #[cfg(not(test))]
    return f(&WANTED);
    #[cfg(test)]
    return WANTED.with(f);
}

/// This frame shows the phrase (or words of it): keep the window out of captures.
pub fn keep_out() {
    wanted(|w| w.store(true, Ordering::Relaxed));
}

/// Whether the frame just drawn asked for it; the request is cleared for the next one.
pub fn take() -> bool {
    wanted(|w| w.swap(false, Ordering::Relaxed))
}

/// Keeps the window of `frame` out of captures, or lets it back in; false when the system
/// refused or cannot do it.
pub fn apply(frame: &eframe::Frame, hidden: bool) -> bool {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
        };
        let Ok(handle) = frame.window_handle() else {
            return false;
        };
        let RawWindowHandle::Win32(win) = handle.as_raw() else {
            return false;
        };
        let affinity = if hidden {
            WDA_EXCLUDEFROMCAPTURE
        } else {
            WDA_NONE
        };
        // SAFETY: the window handle of our own top-level window, valid while `frame` is.
        unsafe { SetWindowDisplayAffinity(win.hwnd.get() as _, affinity) != 0 }
    }
    #[cfg(not(windows))]
    {
        let _ = (frame, hidden);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_lasts_one_frame() {
        assert!(!take());
        keep_out();
        assert!(take());
        assert!(
            !take(),
            "the next frame asks again or the window comes back"
        );
    }

    /// A phrase screen drawn on another thread (an app test beside this one) neither asks
    /// for this thread's frame nor takes its request away.
    #[test]
    fn a_request_belongs_to_the_thread_that_draws() {
        keep_out();
        std::thread::spawn(|| {
            assert!(!take(), "another thread's request");
            keep_out();
        })
        .join()
        .unwrap();
        assert!(take(), "still this thread's request");
        assert!(!take());
    }
}
