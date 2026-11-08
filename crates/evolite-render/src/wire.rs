//! What goes through the worker's stdin and stdout. Little-endian, fixed layout:
//!
//! - request: `EVR1`, job id `u64`, side `u32` (0: own size), flags `u32` ([`FIRST_FRAME`]),
//!   length `u32`, the bytes;
//! - answer: `EVR1`, job id `u64`, then `0` and the art (`width u32`, `height u32`,
//!   `frames u32`, per frame `delay_ms u32`, `packed length u32` and the frame's
//!   `width × height × 4` bytes as an LZ4 block), or `1` and an error as two codes from the
//!   fixed lists below.
//!
//! Nothing free-form crosses back: the worker has read untrusted bytes, so its answer is held
//! to the limits before any memory is taken for it. Frames stay packed: the window unpacks
//! the one it shows, and checks then that it comes to exactly its size.

use std::io::{self, Read, Write};

use crate::frames::{Limits, PackedArt, PackedFrame, RenderError, packed_bound};

const MAGIC: &[u8; 4] = b"EVR1";

/// Request flag: only the first frame of an animation ([`Limits::first_frame_only`]).
pub const FIRST_FRAME: u32 = 1;
/// Request flag: the bytes are raw deflate ([`Limits::raw_deflate`]).
pub const RAW_DEFLATE: u32 = 2;

/// The reasons a worker may give, by index. Anything else reads as `"unknown"`.
const LIMITS: [&str; 6] = [
    "picture size",
    "frames",
    "decoded size",
    "SVG nodes",
    "unpacked size",
    "model size",
];
const INVALID: [&str; 12] = [
    "cannot decode",
    "frames of different sizes",
    "no frames",
    "not UTF-8",
    "not well-formed XML",
    "not an SVG",
    "size",
    "frame size",
    "frame length",
    "decoder panic",
    "cannot unpack",
    "not a GLB",
];

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

fn u32_from(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn u64_from(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn u8_from(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

/// `false` on a clean end of the stream before a message.
fn magic(r: &mut impl Read) -> io::Result<bool> {
    let mut m = [0; 4];
    match r.read_exact(&mut m) {
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
        Ok(()) if &m == MAGIC => Ok(true),
        Ok(()) => Err(bad("not a renderer message")),
    }
}

/// A picture to draw `side` pixels across (`0`: its own size, within the limits).
#[derive(Debug, PartialEq)]
pub struct Request {
    pub id: u64,
    pub side: u32,
    pub flags: u32,
    pub bytes: Vec<u8>,
}

pub fn write_request(
    w: &mut impl Write,
    id: u64,
    side: u32,
    flags: u32,
    bytes: &[u8],
) -> io::Result<()> {
    let len = u32::try_from(bytes.len()).map_err(|_| bad("request too large"))?;
    w.write_all(MAGIC)?;
    w.write_all(&id.to_le_bytes())?;
    w.write_all(&side.to_le_bytes())?;
    w.write_all(&flags.to_le_bytes())?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(bytes)?;
    w.flush()
}

/// `None`: the other side closed the stream.
pub fn read_request(r: &mut impl Read, limits: &Limits) -> io::Result<Option<Request>> {
    if !magic(r)? {
        return Ok(None);
    }
    let id = u64_from(r)?;
    let side = u32_from(r)?;
    let flags = u32_from(r)?;
    let len = u32_from(r)? as usize;
    if len > limits.max_input {
        return Err(bad("request over the input limit"));
    }
    let mut bytes = vec![0; len];
    r.read_exact(&mut bytes)?;
    Ok(Some(Request {
        id,
        side,
        flags,
        bytes,
    }))
}

fn error_codes(e: &RenderError) -> (u8, u8) {
    let index =
        |list: &[&str], what: &str| list.iter().position(|w| *w == what).unwrap_or(255) as u8;
    match e {
        RenderError::TooLarge => (0, 0),
        RenderError::Unsupported => (1, 0),
        RenderError::Limit(what) => (2, index(&LIMITS, what)),
        RenderError::Invalid(what) => (3, index(&INVALID, what)),
        // Only the client decides these; a worker never sends them.
        RenderError::Timeout | RenderError::Crashed | RenderError::Refused(_) => (3, 255),
    }
}

fn error_from(kind: u8, reason: u8) -> io::Result<RenderError> {
    let pick = |list: &[&'static str]| list.get(reason as usize).copied().unwrap_or("unknown");
    Ok(match kind {
        0 => RenderError::TooLarge,
        1 => RenderError::Unsupported,
        2 => RenderError::Limit(pick(&LIMITS)),
        3 => RenderError::Invalid(pick(&INVALID)),
        _ => return Err(bad("unknown error kind")),
    })
}

pub fn write_answer(
    w: &mut impl Write,
    id: u64,
    answer: &Result<PackedArt, RenderError>,
) -> io::Result<()> {
    w.write_all(MAGIC)?;
    w.write_all(&id.to_le_bytes())?;
    match answer {
        Ok(art) => {
            w.write_all(&[0])?;
            w.write_all(&art.width.to_le_bytes())?;
            w.write_all(&art.height.to_le_bytes())?;
            w.write_all(&(art.frames.len() as u32).to_le_bytes())?;
            for frame in &art.frames {
                w.write_all(&frame.delay_ms.to_le_bytes())?;
                w.write_all(&(frame.lz4.len() as u32).to_le_bytes())?;
                w.write_all(&frame.lz4)?;
            }
        }
        Err(e) => {
            let (kind, reason) = error_codes(e);
            w.write_all(&[1, kind, reason])?;
        }
    }
    w.flush()
}

/// An answer, held to `limits` before the memory for its frames is taken.
pub fn read_answer(
    r: &mut impl Read,
    limits: &Limits,
) -> io::Result<(u64, Result<PackedArt, RenderError>)> {
    if !magic(r)? {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    let id = u64_from(r)?;
    match u8_from(r)? {
        0 => {}
        1 => {
            let kind = u8_from(r)?;
            let reason = u8_from(r)?;
            return Ok((id, Err(error_from(kind, reason)?)));
        }
        _ => return Err(bad("unknown answer")),
    }
    let width = u32_from(r)?;
    let height = u32_from(r)?;
    let count = u32_from(r)? as usize;
    if width == 0 || height == 0 || width > limits.max_side || height > limits.max_side {
        return Err(bad("frame size over the limit"));
    }
    if count == 0 || count > limits.max_frames {
        return Err(bad("frame count over the limit"));
    }
    let frame_len = width as usize * height as usize * 4;
    if frame_len.saturating_mul(count) > limits.max_total {
        return Err(bad("frames over the limit"));
    }
    // A packed frame is never longer than LZ4 could make it, and all of them together never
    // more than the unpacked limit: memory is taken only within those.
    let max_packed = packed_bound(frame_len);
    let mut total = 0usize;
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        let delay_ms = u32_from(r)?;
        let len = u32_from(r)? as usize;
        total = total.saturating_add(len);
        if len > max_packed || total > limits.max_total {
            return Err(bad("packed frame over the limit"));
        }
        let mut lz4 = vec![0; len];
        r.read_exact(&mut lz4)?;
        frames.push(PackedFrame { lz4, delay_ms });
    }
    Ok((
        id,
        Ok(PackedArt {
            width,
            height,
            frames,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use crate::frames::{Art, Frame};

    fn art() -> PackedArt {
        Art {
            width: 2,
            height: 1,
            frames: vec![
                Frame {
                    rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
                    delay_ms: 50,
                },
                Frame {
                    rgba: vec![9; 8],
                    delay_ms: 70,
                },
            ],
        }
        .pack()
    }

    #[test]
    fn a_packed_frame_longer_than_lz4_could_make_it_is_refused() {
        let mut wire = Vec::new();
        wire.extend(MAGIC);
        wire.extend(1u64.to_le_bytes());
        wire.push(0);
        for v in [2u32, 1, 1, 50] {
            wire.extend(v.to_le_bytes());
        }
        // 8 unpacked bytes cannot take a megabyte packed.
        wire.extend((1u32 << 20).to_le_bytes());
        assert_eq!(
            read_answer(&mut Cursor::new(wire), &Limits::default())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn requests_and_answers_go_through() {
        let limits = Limits::default();
        let mut wire = Vec::new();
        write_request(&mut wire, 7, 256, FIRST_FRAME, b"picture").unwrap();
        let mut r = Cursor::new(wire);
        assert_eq!(
            read_request(&mut r, &limits).unwrap(),
            Some(Request {
                id: 7,
                side: 256,
                flags: FIRST_FRAME,
                bytes: b"picture".to_vec()
            })
        );
        assert_eq!(read_request(&mut r, &limits).unwrap(), None);

        for answer in [
            Ok(art()),
            Err(RenderError::Limit("SVG nodes")),
            Err(RenderError::Unsupported),
        ] {
            let mut wire = Vec::new();
            write_answer(&mut wire, 9, &answer).unwrap();
            assert_eq!(
                read_answer(&mut Cursor::new(wire), &limits).unwrap(),
                (9, answer)
            );
        }
    }

    #[test]
    fn an_answer_over_the_limits_is_refused_before_allocating() {
        let mut wire = Vec::new();
        wire.extend(MAGIC);
        wire.extend(1u64.to_le_bytes());
        wire.push(0);
        wire.extend(u32::MAX.to_le_bytes());
        wire.extend(u32::MAX.to_le_bytes());
        wire.extend(u32::MAX.to_le_bytes());
        assert_eq!(
            read_answer(&mut Cursor::new(wire), &Limits::default())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn a_reason_off_the_list_reads_as_unknown() {
        assert_eq!(error_from(2, 200).unwrap(), RenderError::Limit("unknown"));
        assert!(error_from(9, 0).is_err());
    }
}
