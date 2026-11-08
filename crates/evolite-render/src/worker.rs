//! The worker's side: read a picture, answer with frames or a refusal, repeat until stdin
//! closes. It holds no keys and talks to nobody but its parent.

use std::io::{self, BufReader, BufWriter, Read, Write};

use crate::frames::{Limits, RenderError};
use crate::{render_packed, wire};

/// The argument that makes the app's own executable start as a worker.
pub const FLAG: &str = "--render-worker";

/// Serves requests until the input ends. A decoder that panics gets the request refused,
/// not the worker killed: a panic unwinds cleanly and leaves no shared state behind.
pub fn serve(input: impl Read, output: impl Write, limits: &Limits) -> io::Result<()> {
    let mut input = BufReader::new(input);
    let mut output = BufWriter::new(output);
    // Fonts are read now, before the first untrusted byte.
    crate::svg::warm_up();
    while let Some(wire::Request {
        id,
        side,
        flags,
        bytes,
    }) = wire::read_request(&mut input, limits)?
    {
        let mut limits = if side > 0 {
            limits.for_side(side)
        } else {
            *limits
        };
        limits.first_frame_only = flags & wire::FIRST_FRAME != 0;
        limits.raw_deflate = flags & wire::RAW_DEFLATE != 0;
        let answer = std::panic::catch_unwind(|| render_packed(&bytes, &limits))
            .unwrap_or(Err(RenderError::Invalid("decoder panic")));
        wire::write_answer(&mut output, id, &answer)?;
    }
    Ok(())
}

/// Finds the fonts, then closes everything the worker does not need (`lockdown.rs`).
/// `Err`: the doors could not be closed, and nothing untrusted may be read.
pub fn lock_down() -> Result<(), crate::lockdown::Refusal> {
    let font_dirs = crate::svg::warm_up();
    crate::lockdown::apply(&font_dirs)
}

/// The memory cap on a system where the worker sets it itself (macOS); elsewhere the wallet has
/// set it already. For a test mode that does not go through [`lock_down`].
pub fn cap_memory() -> Result<(), String> {
    crate::lockdown::cap_memory()
}

/// A worker that crashes ends at once. Over the Job Object's memory cap an allocation fails and
/// the process aborts; Windows Error Reporting then held the dead process for about 2.5 s
/// (measured), half of a picture's deadline, so the wallet saw a timeout instead of a crash.
/// No dialog, no report: nobody is there to read either.
pub fn quiet_crashes() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetErrorMode,
        };
        // SAFETY: a plain call that sets this process's error mode.
        unsafe {
            SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
        }
    }
}

/// The worker process: stdin and stdout are the pipe to the app. Never returns, so a caller
/// in `main` cannot fall through into the rest of the app.
pub fn run() -> ! {
    quiet_crashes();
    // Nobody reads a panic message here; the answer carries what matters.
    std::panic::set_hook(Box::new(|_| {}));
    if let Err(refusal) = lock_down() {
        std::process::exit(refusal.kind.exit_code());
    }
    let code = match serve(io::stdin().lock(), io::stdout().lock(), &Limits::default()) {
        Ok(()) => 0,
        Err(_) => 1,
    };
    std::process::exit(code)
}
