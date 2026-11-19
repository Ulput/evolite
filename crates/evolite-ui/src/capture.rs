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

/// SEC-7: the desktop session on Linux, where no capture can be refused: the warning before
/// the phrase says what can see it.
// Told only on Linux (and in tests): elsewhere no warning is asked for.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    X11,
    Wayland,
    /// Not told: the X11 text, the stricter.
    Unknown,
}

impl Session {
    /// For the journal (R-4).
    pub fn name(self) -> &'static str {
        match self {
            Session::X11 => "x11",
            Session::Wayland => "wayland",
            Session::Unknown => "unknown",
        }
    }
}

/// R-1: Wayland when `XDG_SESSION_TYPE` says so or `WAYLAND_DISPLAY` is set; X11 when it says
/// x11 or only `DISPLAY` is set; else unknown. A window that is an X11 client (`x11_window`,
/// once known) is X11 whatever the session: under XWayland the other X programs of the
/// session see it.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub fn session_of(
    xdg_session_type: Option<&str>,
    wayland_display: bool,
    display: bool,
    x11_window: Option<bool>,
) -> Session {
    if x11_window == Some(true) {
        return Session::X11;
    }
    match xdg_session_type.map(str::to_ascii_lowercase).as_deref() {
        Some("wayland") => Session::Wayland,
        Some("x11") => Session::X11,
        _ if wayland_display => Session::Wayland,
        _ if display => Session::X11,
        _ => Session::Unknown,
    }
}

/// Whether the window is an X11 client: 0 not known yet, 1 yes, 2 no (Wayland).
#[cfg(target_os = "linux")]
static X11_WINDOW: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Notes which protocol the window speaks, once it has one (from the app's first frame).
pub fn note_window(frame: &eframe::Frame) {
    #[cfg(target_os = "linux")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if X11_WINDOW.load(Ordering::Relaxed) != 0 {
            return;
        }
        if let Ok(handle) = frame.window_handle() {
            let x11 = matches!(
                handle.as_raw(),
                RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)
            );
            X11_WINDOW.store(if x11 { 1 } else { 2 }, Ordering::Relaxed);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = frame;
}

/// The session the phrase would be shown in when a warning must come first: on Linux only
/// (Windows keeps the phrase out of captures; macOS has its own, SEC-10). In every network: a
/// phrase is not tied to one (analyst-155).
pub fn phrase_warning() -> Option<Session> {
    #[cfg(test)]
    return TEST_SESSION.with(std::cell::Cell::get);
    #[cfg(all(not(test), target_os = "linux"))]
    {
        let var = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        let x11 = match X11_WINDOW.load(Ordering::Relaxed) {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        };
        return Some(session_of(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            var("WAYLAND_DISPLAY"),
            var("DISPLAY"),
            x11,
        ));
    }
    #[cfg(all(not(test), not(target_os = "linux")))]
    None
}

// Tests stand in for the platform on their own thread: `None` is Windows, `Some` a Linux
// session (AC-A1, AC-A4).
#[cfg(test)]
thread_local! {
    static TEST_SESSION: std::cell::Cell<Option<Session>> = const { std::cell::Cell::new(None) };
}

/// For a test: the platform this thread's frames are drawn on. Only the app's kittests use it,
/// so it is built with them (as their helpers).
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub fn set_test_session(session: Option<Session>) {
    TEST_SESSION.with(|s| s.set(session));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R-1, AC-A1: the session from the environment and the window.
    #[test]
    fn the_session_is_told_by_the_environment_and_the_window() {
        assert_eq!(
            session_of(Some("wayland"), true, true, None),
            Session::Wayland
        );
        assert_eq!(
            session_of(Some("Wayland"), false, false, None),
            Session::Wayland
        );
        assert_eq!(session_of(None, true, false, None), Session::Wayland);
        assert_eq!(session_of(Some("x11"), true, true, None), Session::X11);
        assert_eq!(session_of(None, false, true, None), Session::X11);
        assert_eq!(
            session_of(Some("tty"), false, false, None),
            Session::Unknown
        );
        // XWayland: an X11 window in a Wayland session.
        assert_eq!(
            session_of(Some("wayland"), true, true, Some(true)),
            Session::X11
        );
        assert_eq!(
            session_of(Some("wayland"), true, true, Some(false)),
            Session::Wayland
        );
    }

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
