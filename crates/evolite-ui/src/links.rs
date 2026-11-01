//! Opening a link with the system's handler: the browser for a web page, the mail program for
//! `mailto:`. egui's own `open_url` needs eframe's `links` feature and another crate; this is
//! the one call the app makes, through what is already here.

/// Opens `url` with the system's handler; false when nothing opened it.
pub fn open(url: &str) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let (verb, target) = (wide("open"), wide(url));
        // SAFETY: NUL-terminated wide strings that outlive the call; no window handle.
        let code = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        // Above 32: started.
        code > 32
    }
    #[cfg(not(windows))]
    {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(program).arg(url).spawn().is_ok()
    }
}
