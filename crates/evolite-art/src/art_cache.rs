//! Drawn pictures on disk, by key and size: a picture drawn once shows at once on the next
//! run. The files are written by the core, never by the renderer, and read back as if anyone
//! could have changed them: sizes are held to the limits, a damaged file is deleted and the
//! picture drawn again. Frames stay packed, as the interface takes them.
//!
//! File: `EVA1`, `width u32`, `height u32`, `frames u32`, then per frame `delay_ms u32`,
//! `packed length u32` and the LZ4 block. Little-endian. The folder carries the renderer's
//! version: when drawing changes, old pictures are not shown.

use crate::MAX_SIDE;
use evolite_api::{Picture, PictureFrame};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAGIC: &[u8; 4] = b"EVA1";
/// The renderer gives no more frames than this (`Limits::max_frames`).
const MAX_FRAMES: usize = 300;
/// The whole folder; past it, the pictures used longest ago go.
const MAX_BYTES: u64 = 256 * 1024 * 1024;
/// What is left after a cleanup, so it is not done on every picture.
const KEEP_BYTES: u64 = 192 * 1024 * 1024;

pub struct DiskCache {
    dir: PathBuf,
}

impl DiskCache {
    /// `base`: the app's cache folder; the pictures go in a folder of their own in it.
    pub fn new(base: PathBuf) -> Self {
        Self {
            dir: base
                .join("art")
                .join(format!("r{}", evolite_render::RENDER_VERSION)),
        }
    }

    /// Only our own keys become file names: 64 hex digits, nothing that walks out of the folder.
    fn path(&self, art: &str, side: u32) -> Option<PathBuf> {
        let ours = art.len() == 64 && art.bytes().all(|b| b.is_ascii_hexdigit());
        ours.then(|| self.dir.join(format!("{art}-{side}.evart")))
    }

    pub fn load(&self, art: &str, side: u32) -> Option<Picture> {
        let path = self.path(art, side)?;
        let bytes = fs::read(&path).ok()?;
        match parse(&bytes, side) {
            Some(picture) => {
                // Used now: the cleanup keeps it longer.
                let _ = fs::File::options()
                    .append(true)
                    .open(&path)
                    .and_then(|f| f.set_modified(SystemTime::now()));
                Some(picture)
            }
            None => {
                let _ = fs::remove_file(&path);
                None
            }
        }
    }

    /// Best effort: a picture that cannot be saved is drawn again next time.
    pub fn store(&self, art: &str, side: u32, picture: &Picture) {
        let Some(path) = self.path(art, side) else {
            return;
        };
        if fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        // Written aside and renamed: a picture is whole on disk or not there.
        let partial = path.with_extension("part");
        let written = fs::File::create(&partial)
            .and_then(|mut f| f.write_all(&encode(picture)).and_then(|()| f.sync_all()));
        if written.is_ok() && fs::rename(&partial, &path).is_ok() {
            prune(&self.dir);
        } else {
            let _ = fs::remove_file(&partial);
        }
    }
}

fn encode(picture: &Picture) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(MAGIC);
    out.extend(picture.width.to_le_bytes());
    out.extend(picture.height.to_le_bytes());
    out.extend((picture.frames.len() as u32).to_le_bytes());
    for frame in &picture.frames {
        out.extend(frame.delay_ms.to_le_bytes());
        out.extend((frame.packed().len() as u32).to_le_bytes());
        out.extend(frame.packed());
    }
    out
}

/// A picture of side `side` at most, or `None` for anything else. Frames are unpacked (and
/// checked to their size) only when shown.
fn parse(bytes: &[u8], side: u32) -> Option<Picture> {
    fn take_u32(rest: &mut &[u8]) -> Option<u32> {
        let (head, tail) = rest.split_first_chunk::<4>()?;
        *rest = tail;
        Some(u32::from_le_bytes(*head))
    }
    let mut rest = bytes.strip_prefix(MAGIC)?;
    let width = take_u32(&mut rest)?;
    let height = take_u32(&mut rest)?;
    let count = take_u32(&mut rest)? as usize;
    let max = side.min(MAX_SIDE);
    if width == 0
        || height == 0
        || width > max
        || height > max
        || !(1..=MAX_FRAMES).contains(&count)
    {
        return None;
    }
    // LZ4's worst case for one frame: a little over its own size.
    let frame_len = width as usize * height as usize * 4;
    let max_packed = frame_len + frame_len / 255 + 16;
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        let delay_ms = take_u32(&mut rest)?;
        let len = take_u32(&mut rest)? as usize;
        if len > max_packed || len > rest.len() {
            return None;
        }
        let (packed, tail) = rest.split_at(len);
        rest = tail;
        frames.push(PictureFrame::from_packed(packed.into(), delay_ms));
    }
    rest.is_empty().then_some(Picture {
        width,
        height,
        frames,
    })
}

/// Keeps the folder under [`MAX_BYTES`]: the pictures used longest ago go first.
fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= MAX_BYTES {
        return;
    }
    files.sort();
    for (_, len, path) in files {
        if total <= KEEP_BYTES {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture() -> Picture {
        let rgba: Vec<u8> = (0..8 * 4 * 4).map(|i| i as u8).collect();
        Picture {
            width: 8,
            height: 4,
            frames: vec![
                PictureFrame::pack(&rgba, 50),
                PictureFrame::pack(&[7; 8 * 4 * 4], 70),
            ],
        }
    }

    fn cache(name: &str) -> DiskCache {
        let base = std::env::temp_dir().join(format!("evolite-art-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        DiskCache::new(base)
    }

    const KEY: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn a_stored_picture_comes_back_whole() {
        let cache = cache("back");
        assert!(cache.load(KEY, 64).is_none());
        cache.store(KEY, 64, &picture());
        assert_eq!(cache.load(KEY, 64), Some(picture()));
        // Another size is another picture.
        assert!(cache.load(KEY, 128).is_none());
    }

    #[test]
    fn a_damaged_or_oversized_file_is_dropped() {
        let cache = cache("damaged");
        cache.store(KEY, 64, &picture());
        let path = cache.path(KEY, 64).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes.pop();
        fs::write(&path, &bytes).unwrap();
        assert!(cache.load(KEY, 64).is_none());
        assert!(!path.exists(), "the damaged file is deleted");
        // Larger than asked for: not ours.
        let encoded = encode(&picture());
        assert!(parse(&encoded, 4).is_none());
        assert!(parse(&encoded, 8).is_some());
    }

    #[test]
    fn keys_that_are_not_hashes_never_become_paths() {
        let cache = cache("paths");
        for bad in ["..", "../../etc/passwd", &"g".repeat(64), "abc"] {
            assert!(cache.path(bad, 64).is_none(), "{bad}");
        }
    }
}
