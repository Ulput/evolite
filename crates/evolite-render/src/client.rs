//! The wallet's side: one worker process at a time, a deadline on every picture, and a new
//! worker after any failure.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use crate::frames::{Limits, PackedArt, RenderError};
use crate::sandbox::{self, Sandboxed};
use crate::wire;

type Answer = io::Result<(u64, Result<PackedArt, RenderError>)>;

/// How the bytes of a picture are stored. A gzip file (`.svgz`) is known by its first bytes and
/// needs no word; raw deflate has no signature, so the document has to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Packing {
    Plain,
    /// Jembe's `imageFormat` 1: SVG, raw deflate.
    RawDeflate,
}

/// Which pass: the first frame only (shown at once), or the whole animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    First,
    Whole,
}

/// How the worker is started and held.
#[derive(Clone, Debug)]
pub struct Settings {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub limits: Limits,
    /// One picture, from the request to the last byte of the answer.
    pub timeout: Duration,
    /// The worker's memory cap: the Job Object on Windows, `RLIMIT_AS` elsewhere.
    pub memory: usize,
    /// Windows: no window or graphics system calls (Win32k). Only for a program that links
    /// no window code: `evolite-renderer`, not the app.
    pub win32k_off: bool,
}

impl Settings {
    /// `evolite-renderer` next to this executable, with Win32k off; if it is not there, this
    /// executable with [`crate::worker::FLAG`] ([`Self::this_exe`]).
    pub fn bundled() -> io::Result<Self> {
        let exe = std::env::current_exe()?;
        let renderer =
            exe.with_file_name(format!("evolite-renderer{}", std::env::consts::EXE_SUFFIX));
        if renderer.is_file() {
            let mut settings = Self::new(renderer, Vec::new());
            settings.win32k_off = true;
            Ok(settings)
        } else {
            Self::this_exe()
        }
    }

    /// This executable started with [`crate::worker::FLAG`]: the app ships one file.
    pub fn this_exe() -> io::Result<Self> {
        Ok(Self::new(
            std::env::current_exe()?,
            vec![crate::worker::FLAG.into()],
        ))
    }

    pub fn new(program: PathBuf, args: Vec<OsString>) -> Self {
        Self {
            program,
            args,
            limits: Limits::default(),
            timeout: Duration::from_secs(5),
            // Frames are at most `max_total` (192 MB); fonts, the parsed SVG and one frame being
            // drawn fit in the rest. Measured on Windows: about 120 MB for an animation at
            // 512 px. `RLIMIT_AS` elsewhere counts reserved address space too, so it stays higher.
            memory: if cfg!(windows) { 256 } else { 512 } * 1024 * 1024,
            win32k_off: false,
        }
    }
}

struct Worker {
    process: Sandboxed,
    input: BufWriter<File>,
    answers: Receiver<Answer>,
}

impl Worker {
    fn start(settings: &Settings) -> io::Result<Self> {
        let process = sandbox::spawn(
            &settings.program,
            &settings.args,
            settings.memory,
            settings.win32k_off,
        )?;
        let input = BufWriter::new(process.stdin.try_clone()?);
        let output = process.stdout.try_clone()?;
        // Answers are read on their own thread, so a silent worker cannot block the caller
        // past the deadline.
        let (send, answers) = channel();
        let limits = settings.limits;
        std::thread::Builder::new()
            .name("evolite-render-answers".into())
            .spawn(move || {
                let mut output = BufReader::new(output);
                loop {
                    let answer = wire::read_answer(&mut output, &limits);
                    let failed = answer.is_err();
                    if send.send(answer).is_err() || failed {
                        break;
                    }
                }
            })?;
        Ok(Self {
            process,
            input,
            answers,
        })
    }

    fn stop(self) {
        self.process.kill();
        self.process.wait();
    }
}

/// Renders untrusted pictures in a worker process. Blocking: call it off the UI thread.
pub struct Renderer {
    settings: Settings,
    worker: Option<Worker>,
    next_id: u64,
}

impl Renderer {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            worker: None,
            next_id: 0,
        }
    }

    /// Starts the worker now (the process, its sandbox, the system fonts), so the first
    /// picture does not wait for it. A worker that has died since is started again.
    pub fn start(&mut self) -> Result<(), RenderError> {
        if self.worker.is_none() {
            self.worker = Some(Worker::start(&self.settings).map_err(|_| RenderError::Crashed)?);
        }
        Ok(())
    }

    /// Frames for `bytes` at their own size (within the limits), or why not.
    pub fn render(&mut self, bytes: &[u8]) -> Result<PackedArt, RenderError> {
        self.render_at(bytes, 0)
    }

    /// Frames for `bytes` shown `side` pixels across: an SVG is drawn exactly that large,
    /// a raster no larger. A worker that is late, dies or talks nonsense is stopped; the next
    /// picture gets a fresh one.
    pub fn render_at(&mut self, bytes: &[u8], side: u32) -> Result<PackedArt, RenderError> {
        self.draw(bytes, Packing::Plain, side, Pass::Whole)
    }

    /// Only the first frame, `side` pixels across: fast, to show the picture while its
    /// animation is drawn. One frame with `delay_ms` 0 is a still and nothing follows; with a
    /// delay, [`Self::render_at`] gives the animation.
    pub fn render_first(&mut self, bytes: &[u8], side: u32) -> Result<PackedArt, RenderError> {
        self.draw(bytes, Packing::Plain, side, Pass::First)
    }

    /// The general form: how the bytes are packed, the side (0: own size), and which pass.
    pub fn draw(
        &mut self,
        bytes: &[u8],
        packing: Packing,
        side: u32,
        pass: Pass,
    ) -> Result<PackedArt, RenderError> {
        let mut flags = 0;
        if pass == Pass::First {
            flags |= wire::FIRST_FRAME;
        }
        if packing == Packing::RawDeflate {
            flags |= wire::RAW_DEFLATE;
        }
        self.request(bytes, side, flags)
    }

    fn request(&mut self, bytes: &[u8], side: u32, flags: u32) -> Result<PackedArt, RenderError> {
        let limits = if side > 0 {
            self.settings.limits.for_side(side)
        } else {
            self.settings.limits
        };
        if bytes.len() > limits.max_input {
            return Err(RenderError::TooLarge);
        }
        let mut worker = match self.worker.take() {
            Some(worker) => worker,
            None => Worker::start(&self.settings).map_err(|_| RenderError::Crashed)?,
        };
        self.next_id += 1;
        let id = self.next_id;
        if wire::write_request(&mut worker.input, id, side, flags, bytes).is_err() {
            worker.stop();
            return Err(RenderError::Crashed);
        }
        match worker.answers.recv_timeout(self.settings.timeout) {
            Ok(Ok((answered, result))) if answered == id => {
                // The answer is held to the limits once more: the worker read untrusted bytes.
                let result = result.and_then(|art| art.check(&limits).map(|()| art));
                self.worker = Some(worker);
                result
            }
            Err(RecvTimeoutError::Timeout) => {
                worker.stop();
                Err(RenderError::Timeout)
            }
            // A wrong id, a broken message, or the worker gone.
            _ => {
                worker.stop();
                Err(RenderError::Crashed)
            }
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            let _ = worker.input.flush();
            worker.stop();
        }
    }
}
