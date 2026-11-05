//! Opening a link with the system's handler: the browser for a web page, the mail program for
//! `mailto:`. egui's own `open_url` needs eframe's `links` feature and another crate; this is
//! the one call the app makes, through what is already here.

/// Opens `url` with the system's handler; false when nothing opened it. In tests nothing opens
/// (no browser on the desktop QA may be working at): the link is kept for [`opened`].
pub fn open(url: &str) -> bool {
    #[cfg(test)]
    {
        OPENED.with(|o| o.borrow_mut().push(url.to_owned()));
        true
    }
    #[cfg(all(windows, not(test)))]
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
    #[cfg(all(not(windows), not(test)))]
    {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(program).arg(url).spawn().is_ok()
    }
}

#[cfg(test)]
thread_local! {
    static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The links a test opened on its thread, in order.
#[cfg(test)]
#[allow(dead_code)] // for the tests that click a link
pub fn opened() -> Vec<String> {
    OPENED.with(|o| o.borrow().clone())
}
