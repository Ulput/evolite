//! What a picture becomes: frames of one size, each with how long it stays on screen.

use std::fmt;

/// How much a picture may cost. Checked while decoding (so a lie in a header is caught before
/// the memory is taken) and again on the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes handed in.
    pub max_input: usize,
    /// Width or height of a source raster before any decoding.
    pub max_source_side: u32,
    /// Width or height of the frames handed out; larger pictures are scaled down to fit.
    pub max_side: u32,
    pub max_frames: usize,
    /// All frames together, RGBA bytes.
    pub max_total: usize,
    /// XML nodes of an SVG.
    pub max_svg_nodes: u32,
    /// SMIL animations in one SVG; with more, it is drawn as a still.
    pub max_animations: usize,
    /// One loop of an animated SVG; longer timelines are cut here.
    pub max_animation_ms: u32,
    /// Frames a second for an animated SVG, at most.
    pub animation_fps: u32,
    /// Width or height of an animated SVG's frames (every frame is a full drawing).
    pub max_animation_side: u32,
    /// Drawing all frames of an animated SVG; past it the animation gets fewer frames.
    pub max_render_ms: u32,
    /// Draw an SVG this many pixels on its longer side, larger or smaller than its own size
    /// (a vector picture loses nothing); `None`: its own size, within the limits.
    pub svg_size: Option<u32>,
    /// Only the first frame of an animation (with its delay, so it reads as one to come): the
    /// picture is shown at once, the animation follows.
    pub first_frame_only: bool,
    /// Threads that draw the frames of one animation.
    pub threads: usize,
    /// The bytes are a raw deflate stream (Jembe's `imageFormat` 1): they have no signature,
    /// so the document says so and the request carries it ([`crate::wire::RAW_DEFLATE`]).
    pub raw_deflate: bool,
}

impl Limits {
    /// The limits for a picture shown `side` pixels across: nothing larger is drawn, and an
    /// SVG is drawn exactly that large.
    pub fn for_side(self, side: u32) -> Limits {
        let side = side.max(1);
        Limits {
            max_side: self.max_side.min(side),
            max_animation_side: self.max_animation_side.min(side),
            svg_size: Some(side.min(self.max_side)),
            ..self
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input: 16 * 1024 * 1024,
            max_source_side: 8192,
            max_side: 2048,
            max_frames: 300,
            max_total: 192 * 1024 * 1024,
            max_svg_nodes: 50_000,
            max_animations: 1_000,
            max_animation_ms: 10_000,
            animation_fps: 20,
            max_animation_side: 512,
            max_render_ms: 3_000,
            svg_size: None,
            first_frame_only: false,
            threads: default_threads(),
            raw_deflate: false,
        }
    }
}

/// Half the cores, from one to eight: a picture is drawn fast, and the rest of the machine
/// (the window, the wallet's sync, other programs) keeps its share. 2 cores: 1 thread, 4: 2,
/// 8: 4, 16 and more: 8.
pub fn default_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    (cores / 2).clamp(1, 8)
}

/// Straight-alpha RGBA (as image decoders give it) into premultiplied, in place.
pub(crate) fn premultiply(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// `width × height` pixels, RGBA, premultiplied alpha (as the screen takes them), row by
    /// row.
    pub rgba: Vec<u8>,
    /// How long it stays on screen; 0 for a still picture.
    pub delay_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Art {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<Frame>,
}

impl Art {
    /// Every frame has the size it claims, and the whole is within `limits`.
    pub fn check(&self, limits: &Limits) -> Result<(), RenderError> {
        if self.width == 0
            || self.height == 0
            || self.width > limits.max_side
            || self.height > limits.max_side
        {
            return Err(RenderError::Invalid("frame size"));
        }
        if self.frames.is_empty() || self.frames.len() > limits.max_frames {
            return Err(RenderError::Limit("frames"));
        }
        let frame_len = self.width as usize * self.height as usize * 4;
        if frame_len.saturating_mul(self.frames.len()) > limits.max_total {
            return Err(RenderError::Limit("decoded size"));
        }
        if self.frames.iter().any(|f| f.rgba.len() != frame_len) {
            return Err(RenderError::Invalid("frame length"));
        }
        Ok(())
    }

    /// Every frame packed (LZ4 block, as `evolite_api::PictureFrame` keeps it).
    pub fn pack(self) -> PackedArt {
        PackedArt {
            width: self.width,
            height: self.height,
            frames: self
                .frames
                .into_iter()
                .map(|f| PackedFrame::pack(&f.rgba, f.delay_ms))
                .collect(),
        }
    }
}

/// One frame, packed: the worker packs each frame as soon as it is drawn, so neither it, nor
/// the pipe, nor the wallet holds an animation's frames unpacked (120 frames at 512 px are
/// 120 MB unpacked, a few MB packed). Only the frame on screen is unpacked, in the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedFrame {
    /// `width × height × 4` premultiplied RGBA, LZ4 block (no length prefix).
    pub lz4: Vec<u8>,
    pub delay_ms: u32,
}

impl PackedFrame {
    pub fn pack(rgba: &[u8], delay_ms: u32) -> Self {
        Self {
            lz4: lz4_flex::block::compress(rgba),
            delay_ms,
        }
    }
}

/// The longest an LZ4 block of `len` bytes can be: anything longer is not one.
pub fn packed_bound(len: usize) -> usize {
    lz4_flex::block::get_maximum_output_size(len)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedArt {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<PackedFrame>,
}

impl PackedArt {
    pub fn frame_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// Sizes within `limits` and every frame no longer than LZ4 could make it. Whether a frame
    /// unpacks to exactly its size is checked when it is unpacked ([`Self::unpack`]).
    pub fn check(&self, limits: &Limits) -> Result<(), RenderError> {
        if self.width == 0
            || self.height == 0
            || self.width > limits.max_side
            || self.height > limits.max_side
        {
            return Err(RenderError::Invalid("frame size"));
        }
        if self.frames.is_empty() || self.frames.len() > limits.max_frames {
            return Err(RenderError::Limit("frames"));
        }
        let frame_len = self.frame_len();
        if frame_len.saturating_mul(self.frames.len()) > limits.max_total {
            return Err(RenderError::Limit("decoded size"));
        }
        if self
            .frames
            .iter()
            .any(|f| f.lz4.len() > packed_bound(frame_len))
        {
            return Err(RenderError::Invalid("frame length"));
        }
        Ok(())
    }

    /// Frame `i` unpacked, or `None` if it does not come to exactly its size.
    pub fn unpack(&self, i: usize) -> Option<Vec<u8>> {
        let frame = self.frames.get(i)?;
        let mut out = vec![0; self.frame_len()];
        match lz4_flex::block::decompress_into(&frame.lz4, &mut out) {
            Ok(n) if n == out.len() => Some(out),
            _ => None,
        }
    }

    /// All frames unpacked (tests, the preview): `Invalid` if one does not unpack whole.
    pub fn unpacked(&self) -> Result<Art, RenderError> {
        let frames = (0..self.frames.len())
            .map(|i| {
                self.unpack(i)
                    .map(|rgba| Frame {
                        rgba,
                        delay_ms: self.frames[i].delay_ms,
                    })
                    .ok_or(RenderError::Invalid("frame length"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Art {
            width: self.width,
            height: self.height,
            frames,
        })
    }
}

/// Why a picture is not shown. None of these carry the picture's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    /// More bytes than [`Limits::max_input`].
    TooLarge,
    /// Not one of the accepted formats.
    Unsupported,
    /// Over one of the [`Limits`]; which one.
    Limit(&'static str),
    /// Damaged, or not what it claims to be.
    Invalid(&'static str),
    /// The worker did not answer in time and was stopped.
    Timeout,
    /// The worker died (crashed, or the memory cap ended it) or could not be started.
    Crashed,
    /// The worker would not serve: this system cannot close it in (`lockdown.rs`). Not about
    /// this picture: none will be drawn.
    Refused(SandboxRefusal),
}

/// Why a worker would not serve, told by its exit code: a fixed list, nothing free-form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxRefusal {
    /// Linux without Landlock: the user's files would stay open to it.
    NoFileIsolation,
    /// macOS would not take the memory cap.
    NoMemoryCap,
    /// The sandbox itself could not be set up: seccomp, Seatbelt.
    NoSandbox,
}

impl SandboxRefusal {
    /// The worker's exit code. 1 and 3 mean other things; a signal reads as 128 and up.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::NoSandbox => 2,
            Self::NoFileIsolation => 4,
            Self::NoMemoryCap => 5,
        }
    }

    pub fn from_exit_code(code: u32) -> Option<Self> {
        [Self::NoSandbox, Self::NoFileIsolation, Self::NoMemoryCap]
            .into_iter()
            .find(|refusal| refusal.exit_code() as u32 == code)
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => write!(f, "the file is too large"),
            Self::Unsupported => write!(f, "not a supported picture format"),
            Self::Limit(what) => write!(f, "over the limit: {what}"),
            Self::Invalid(what) => write!(f, "damaged picture: {what}"),
            Self::Timeout => write!(f, "took too long to draw"),
            Self::Crashed => write!(f, "the renderer stopped"),
            Self::Refused(why) => write!(f, "this system cannot close the renderer in: {why:?}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// The size that fits `width × height` into a `max × max` square, keeping the proportions.
pub(crate) fn fit(width: u32, height: u32, max: u32) -> (u32, u32) {
    if width <= max && height <= max {
        return (width.max(1), height.max(1));
    }
    let scale = max as f64 / width.max(height) as f64;
    (
        ((width as f64 * scale).round() as u32).clamp(1, max),
        ((height as f64 * scale).round() as u32).clamp(1, max),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_multiplied_by_their_alpha() {
        let mut rgba = [255, 128, 0, 128, 10, 20, 30, 255, 200, 200, 200, 0];
        premultiply(&mut rgba);
        assert_eq!(rgba, [128, 64, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn fits_into_the_square() {
        assert_eq!(fit(100, 50, 2048), (100, 50));
        assert_eq!(fit(4096, 1024, 2048), (2048, 512));
        assert_eq!(fit(1, 1_000_000, 2048), (1, 2048));
    }

    #[test]
    fn a_frame_shorter_than_it_claims_is_refused() {
        let art = Art {
            width: 2,
            height: 2,
            frames: vec![Frame {
                rgba: vec![0; 15],
                delay_ms: 0,
            }],
        };
        assert_eq!(
            art.check(&Limits::default()),
            Err(RenderError::Invalid("frame length"))
        );
    }
}
