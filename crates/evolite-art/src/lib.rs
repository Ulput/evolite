//! Pictures of NFTs, drawn by the sandboxed renderer (`evolite-render`) at the size the
//! interface shows them. The bytes come from documents anyone could have written, so they are
//! never decoded in the app's own process: a worker draws them, and only frames held to the
//! limits come back.
//!
//! The order: pictures already on disk ([`crate::art_cache`]) at once; then the first frame
//! of every picture asked for, those on screen first; then the animations.

#![forbid(unsafe_code)]

#[macro_use]
extern crate evolite_common;

pub mod art_cache;

use crate::art_cache::DiskCache;
use evolite_api::{CoreError, Event, Picture, PictureFrame};
use evolite_render::{PackedArt, Packing, Pass, RenderError, Renderer, Settings};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// The largest side the interface may ask for: four times a big card on a HiDPI screen is
/// already more than any display needs.
pub const MAX_SIDE: u32 = 1024;

/// The key of a picture: its SHA-256, in hex.
pub fn key(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A picture as the document stores it: its bytes and how they are packed (Jembe's
/// `imageFormat` 1 is SVG in raw deflate). The key is the SHA-256 of the unpacked picture,
/// the document's `imageHash`.
#[derive(Clone)]
pub struct Stored {
    pub bytes: Arc<[u8]>,
    pub packing: Packing,
}

struct Job {
    art: String,
    side: u32,
    urgent: bool,
    stored: Stored,
}

/// One thread with one renderer: one picture at a time, its frames on several threads.
pub struct Artist {
    jobs: Sender<Job>,
}

impl Artist {
    /// The worker ([`crate::ArtOptions::worker`]) is started right away.
    pub fn start(
        out: Sender<Event>,
        wake: Arc<dyn Fn() + Send + Sync>,
        options: crate::ArtOptions,
    ) -> Self {
        let (jobs, inbox) = channel::<Job>();
        std::thread::Builder::new()
            .name("evolite-art".into())
            .spawn(move || {
                // `ArtOptions::off`: no renderer, every picture fails as with a stopped one; and
                // nothing kept on the disk, whatever `cache` says.
                let (settings, cache) = if options.off {
                    (None, None)
                } else {
                    (
                        options.worker.or_else(|| Settings::bundled().ok()),
                        options.cache,
                    )
                };
                let mut renderer = settings.map(Renderer::new);
                // Started with the wallet, not with the first picture: by the time the
                // collection is opened, the worker has its sandbox and fonts ready.
                if let Some(renderer) = &mut renderer {
                    let _ = renderer.start();
                }
                let mut studio = Studio {
                    renderer,
                    cache: cache.map(DiskCache::new),
                    out,
                    wake,
                };
                while let Some(batch) = next_batch(&inbox) {
                    if studio.draw(batch).is_err() {
                        return;
                    }
                }
            })
            .expect("start the art thread");
        Self { jobs }
    }

    pub fn draw(&self, art: String, side: u32, urgent: bool, stored: Stored) {
        let _ = self.jobs.send(Job {
            art,
            side: side.clamp(1, MAX_SIDE),
            urgent,
            stored,
        });
    }
}

/// Everything asked for since the last batch, each picture once, those on screen first: a
/// picture asked for again once it came on screen keeps its first place in line and becomes
/// urgent.
fn next_batch(inbox: &Receiver<Job>) -> Option<Vec<Job>> {
    let first = inbox.recv().ok()?;
    let mut place = HashMap::new();
    let mut batch: Vec<Job> = Vec::new();
    for job in std::iter::once(first).chain(inbox.try_iter()) {
        match place.get(&(job.art.clone(), job.side)) {
            Some(&i) => {
                let earlier: &mut Job = &mut batch[i];
                earlier.urgent |= job.urgent;
            }
            None => {
                place.insert((job.art.clone(), job.side), batch.len());
                batch.push(job);
            }
        }
    }
    batch.sort_by_key(|job| !job.urgent);
    Some(batch)
}

/// The UI went away.
struct Gone;

struct Studio {
    renderer: Option<Renderer>,
    cache: Option<DiskCache>,
    out: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Studio {
    fn draw(&mut self, batch: Vec<Job>) -> Result<(), Gone> {
        // Drawn before: from the disk, no renderer.
        let mut todo = Vec::new();
        for job in batch {
            match self.cache.as_ref().and_then(|c| c.load(&job.art, job.side)) {
                Some(picture) => self.send(&job, false, Ok(Arc::new(picture)))?,
                None => todo.push(job),
            }
        }
        // The first frame of each: a still is done here, an animation shows its start.
        let mut animated = Vec::new();
        for job in todo {
            match self.render(&job, true) {
                Ok(art) if art.frames.len() == 1 && art.frames[0].delay_ms == 0 => {
                    self.finish(&job, Ok(art))?
                }
                Ok(art) => {
                    self.send(&job, true, Ok(Arc::new(picture(art))))?;
                    animated.push(job);
                }
                Err(e) => self.finish(&job, Err(e))?,
            }
        }
        for job in animated {
            let result = self.render(&job, false);
            self.finish(&job, result)?;
        }
        Ok(())
    }

    fn render(&mut self, job: &Job, first: bool) -> Result<PackedArt, CoreError> {
        let Some(renderer) = &mut self.renderer else {
            return Err(error(&RenderError::Crashed));
        };
        let pass = if first { Pass::First } else { Pass::Whole };
        let Stored { bytes, packing } = &job.stored;
        let art = renderer.draw(bytes, *packing, job.side, pass);
        art.map_err(|e| error(&e))
    }

    /// The whole picture: to the interface and to the disk.
    fn finish(&mut self, job: &Job, art: Result<PackedArt, CoreError>) -> Result<(), Gone> {
        let result = art.map(|art| Arc::new(picture(art)));
        if let (Ok(picture), Some(cache)) = (&result, &self.cache) {
            cache.store(&job.art, job.side, picture);
        }
        self.send(job, false, result)
    }

    fn send(
        &self,
        job: &Job,
        partial: bool,
        result: Result<Arc<Picture>, CoreError>,
    ) -> Result<(), Gone> {
        let event = Event::Art {
            art: job.art.clone(),
            side: job.side,
            partial,
            result,
        };
        self.out.send(event).map_err(|_| Gone)?;
        (self.wake)();
        Ok(())
    }
}

/// The frames as the worker packed them: the window unpacks the one it shows, and checks then
/// that it comes to its size.
fn picture(art: PackedArt) -> Picture {
    Picture {
        width: art.width,
        height: art.height,
        frames: art
            .frames
            .into_iter()
            .map(|f| PictureFrame::from_packed(f.lz4.into_boxed_slice(), f.delay_ms))
            .collect(),
    }
}

/// What the interface is told. A refusal of the system is its own answer: it is said once for
/// all pictures, not on each.
fn error(e: &RenderError) -> CoreError {
    use evolite_api::PicturesOff;
    use evolite_render::SandboxRefusal;
    match e {
        RenderError::Refused(SandboxRefusal::NoFileIsolation) => {
            CoreError::PicturesOff(PicturesOff::NoFileIsolation)
        }
        RenderError::Refused(SandboxRefusal::NoMemoryCap) => {
            CoreError::PicturesOff(PicturesOff::NoMemoryCap)
        }
        RenderError::Refused(SandboxRefusal::NoSandbox) => {
            CoreError::PicturesOff(PicturesOff::NoSandbox)
        }
        other => CoreError::Failed(reason(other).into()),
    }
}

/// Why a picture is not shown, for the details line.
fn reason(e: &RenderError) -> &'static str {
    match e {
        RenderError::TooLarge | RenderError::Limit(_) => tr!("the picture is too large"),
        RenderError::Unsupported => tr!("the picture's format is not supported"),
        RenderError::Invalid(_) => tr!("the picture is damaged"),
        RenderError::Timeout => tr!("the picture takes too long to draw"),
        RenderError::Crashed | RenderError::Refused(_) => tr!("the renderer stopped"),
    }
}

/// How NFT pictures are drawn and kept.
#[derive(Clone, Debug, Default)]
pub struct ArtOptions {
    /// Where drawn pictures are kept between runs; `None`: drawn again every time.
    pub cache: Option<PathBuf>,
    /// The renderer process; `None`: `evolite-renderer` next to the app (Win32k off), or the
    /// app itself with `--render-worker`.
    pub worker: Option<evolite_render::Settings>,
    /// No pictures at all: no renderer process and nothing kept, `worker` and `cache` aside
    /// ([`Self::off`]).
    pub off: bool,
}

impl ArtOptions {
    /// Pictures are not drawn: no renderer process is started and nothing is written to the
    /// disk. For the UI tests: their executable is not the app (it would be started as the
    /// renderer), and they must not write to the user's folder.
    pub fn off() -> Self {
        Self {
            cache: None,
            worker: None,
            off: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_sha256_hex() {
        assert_eq!(
            key(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_batch_has_each_picture_once_and_those_on_screen_first() {
        let (send, inbox) = channel();
        let job = |art: &str, side, urgent| Job {
            art: art.into(),
            side,
            urgent,
            stored: Stored {
                bytes: Arc::from(&b""[..]),
                packing: Packing::Plain,
            },
        };
        for j in [
            job("a", 64, false),
            job("b", 64, true),
            job("a", 64, true),
            job("a", 128, false),
        ] {
            send.send(j).unwrap();
        }
        let batch = next_batch(&inbox).unwrap();
        let order: Vec<_> = batch.iter().map(|j| (j.art.as_str(), j.side)).collect();
        assert_eq!(order, [("a", 64), ("b", 64), ("a", 128)]);
        // "a" at 64 was asked for again from the screen: urgent, in its first place.
        assert!(batch[0].urgent && batch[1].urgent && !batch[2].urgent);
    }
}
