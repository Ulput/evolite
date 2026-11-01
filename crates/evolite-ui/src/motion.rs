//! Whether the system asks for less motion. Windows: "Show animations in Windows"
//! (Accessibility → Visual effects). Elsewhere the system is not asked yet, and
//! animations play unless turned off in Settings.

/// `false` when the system has animations turned off.
#[cfg(windows)]
pub fn system_animations() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut on: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the pointer given.
    let read =
        unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut on).cast(), 0) };
    read == 0 || on != 0
}

#[cfg(not(windows))]
pub fn system_animations() -> bool {
    true
}
