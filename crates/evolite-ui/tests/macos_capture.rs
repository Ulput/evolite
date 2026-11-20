//! SEC-10: the sharing type of the window as `capture::apply` sets it on macOS: `None` while
//! the phrase or a key is shown, `ReadOnly` (the system's own) after. AppKit makes windows on
//! the main thread only, so this test has its own `main` (`harness = false`). Elsewhere it does
//! nothing. Nothing is drawn: the window is never shown.

#[cfg(target_os = "macos")]
#[path = "../src/capture/macos.rs"]
mod macos;

fn main() {
    #[cfg(target_os = "macos")]
    {
        the_window_is_left_out_of_captures_while_asked();
        println!("macos_capture: ok");
    }
}

#[cfg(target_os = "macos")]
fn the_window_is_left_out_of_captures_while_asked() {
    use std::ptr::NonNull;

    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSView, NSWindow, NSWindowSharingType, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let mtm = MainThreadMarker::new().expect("the test's own main thread");
    let _app = NSApplication::sharedApplication(mtm);
    // SAFETY: a new window, made on the main thread.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(320.0, 240.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: the window is ours to drop: `close` must not release it as well.
    unsafe { window.setReleasedWhenClosed(false) };
    // The view the app gets from eframe's window handle (`AppKit { ns_view }`).
    let view = window.contentView().expect("a content view");
    let ns_view = NonNull::from(&*view).cast();
    assert_eq!(
        window.sharingType(),
        NSWindowSharingType::ReadOnly,
        "a new window can be captured"
    );

    // SAFETY: a live view, on the main thread.
    assert!(
        unsafe { macos::set_sharing(ns_view, true) },
        "the phrase shown"
    );
    assert_eq!(window.sharingType(), NSWindowSharingType::None);
    // SAFETY: as above.
    assert!(
        unsafe { macos::set_sharing(ns_view, false) },
        "the phrase gone"
    );
    assert_eq!(window.sharingType(), NSWindowSharingType::ReadOnly);

    // A view in no window: nothing to set, and it says so.
    let lone = NSView::new(mtm);
    // SAFETY: a live view, on the main thread.
    assert!(!unsafe { macos::set_sharing(NonNull::from(&*lone).cast(), true) });

    window.close();
}
