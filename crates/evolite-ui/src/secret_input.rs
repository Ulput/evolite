//! SEC-3 (phrase review В1): the words of the phrase and a password typed are not seen by
//! Windows' text services. While such a field has focus:
//!
//! - **no IME**: the field takes egui's IME request back (`PlatformOutput::ime`), so egui-winit
//!   turns the window's IME off (`ImmAssociateContextEx(hwnd, 0, IACE_CHILDREN)`, winit 0.30)
//!   and on again once the field lets the focus go — the same switch egui-winit already keeps,
//!   so nothing fights it. A Japanese or Chinese IME sees no composition of the words.
//! - **a password scope**: the window's input scope is `IS_PASSWORD` (msctf `SetInputScope`),
//!   so «text suggestions» for the hardware keyboard and the touch keyboard neither suggest nor
//!   learn the words into the profile's personalization data; back to `IS_DEFAULT` after.
//!
//! winit's `set_ime_purpose(Password)` is empty on Windows, hence this module. Elsewhere only
//! the IME part applies (winit honours `ime = None` on every platform).

use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(test))]
static WANTED: AtomicBool = AtomicBool::new(false);

// One request per test thread, as `capture::WANTED`.
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

/// Called by a phrase-word or password field right after it is drawn, when it has the focus:
/// no IME for it this frame, and the window's input scope a password's.
pub fn focused(ctx: &egui::Context) {
    ctx.output_mut(|o| o.ime = None);
    wanted(|w| w.store(true, Ordering::Relaxed));
}

/// Whether a secret field had the focus in the frame just drawn; cleared for the next one.
pub fn take() -> bool {
    wanted(|w| w.swap(false, Ordering::Relaxed))
}

/// Sets the input scope of `frame`'s window to a password's (`secret`) or back to the default;
/// false when the system has no such call or refused.
pub fn apply(frame: &eframe::Frame, secret: bool) -> bool {
    // A test's window has no handle: it records the scope asked for (`applied`) and is done.
    #[cfg(test)]
    {
        let _ = frame;
        APPLIED.with(|a| a.set(Some(secret)));
        true
    }
    #[cfg(all(windows, not(test)))]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = frame.window_handle() else {
            return false;
        };
        let RawWindowHandle::Win32(win) = handle.as_raw() else {
            return false;
        };
        // `InputScope` values (InputScope.h): IS_DEFAULT 0, IS_PASSWORD 31.
        let scope = if secret { 31 } else { 0 };
        set_input_scope(win.hwnd.get(), scope)
    }
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = (frame, secret);
        false
    }
}

// The last scope `apply` was asked for on this test thread: `Some(true)` a password's.
#[cfg(test)]
thread_local! {
    static APPLIED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// The input scope the app last asked for on this test thread (`Some(true)`: a password's).
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub fn applied() -> Option<bool> {
    APPLIED.with(|a| a.get())
}

/// msctf's `SetInputScope(HWND, InputScope) -> HRESULT`, looked up at run time: neither
/// windows-sys 0.61 nor windows 0.62 binds it, and msctf.dll is on every Windows since Vista.
#[cfg(all(windows, not(test)))]
fn set_input_scope(hwnd: isize, scope: i32) -> bool {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    type SetInputScope = unsafe extern "system" fn(isize, i32) -> i32;
    static CALL: OnceLock<Option<SetInputScope>> = OnceLock::new();
    let call = CALL.get_or_init(|| {
        let name: Vec<u16> = "msctf.dll\0".encode_utf16().collect();
        // SAFETY: a NUL-terminated module name and procedure name; the module stays loaded for
        // the life of the process (never freed), so the function pointer stays valid.
        unsafe {
            let module = LoadLibraryW(name.as_ptr());
            if module.is_null() {
                return None;
            }
            GetProcAddress(module, c"SetInputScope".as_ptr().cast())
                .map(|f| std::mem::transmute::<_, SetInputScope>(f))
        }
    });
    // SAFETY: our own top-level window's handle, valid while the frame is; the documented
    // signature of SetInputScope.
    call.is_some_and(|f| unsafe { f(hwnd, scope) } >= 0)
}

#[cfg(test)]
mod tests {
    /// A request lasts one frame: taken once, then gone.
    #[test]
    fn a_request_lasts_one_frame() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| super::focused(ui.ctx()));
        out.textures_delta.clear();
        assert!(super::take());
        assert!(!super::take());
    }

    /// The focused secret field takes egui's IME request back: egui-winit then turns the
    /// window's IME off.
    #[test]
    fn a_secret_field_asks_for_no_ime() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let ctx = ui.ctx();
            ctx.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Password,
                    rect: egui::Rect::NOTHING,
                    cursor_rect: egui::Rect::NOTHING,
                    should_interrupt_composition: false,
                })
            });
            super::focused(ctx);
        });
        out.textures_delta.clear();
        assert!(out.platform_output.ime.is_none());
        let _ = super::take();
    }
}
