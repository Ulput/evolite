//! Pictures from outside (NFT art, avatars) become plain RGBA frames here, and nowhere else.
//!
//! The decoding runs in a separate process ([`worker`]): a picture that hangs, eats memory or
//! crashes a decoder takes down only that process, never the wallet with its keys. The process
//! is sandboxed ([`sandbox`]: no network, no user files, no executable memory, a memory cap)
//! and restarted after every failure. What it answers is checked again by the [`client`]
//! before anything is shown.
//!
//! What is accepted is a white list, recognised by the first bytes, not the name: PNG, JPEG,
//! GIF, WebP (still or animated, APNG too) and SVG without external references (its SMIL
//! animations as frames), also gzip-compressed (`.svgz`) or as raw deflate when the request
//! says so ([`Limits::raw_deflate`]). Everything is decoded within [`Limits`] and scaled down to
//! fit them.

mod anim;
pub mod client;
mod frames;
mod glb;
#[cfg(windows)]
mod job;
mod lockdown;
mod packing;
mod raster;
pub mod sandbox;
mod sniff;
mod svg;
pub mod wire;
pub mod worker;

/// Bumped when the same picture would be drawn differently: pictures kept from an older
/// renderer are drawn again.
pub const RENDER_VERSION: u32 = 3;

pub use client::{Packing, Pass, Renderer, Settings};
pub use frames::{Art, Frame, Limits, PackedArt, PackedFrame, RenderError, packed_bound};
pub use sniff::{Format, sniff};

/// Decodes `bytes` into frames, in this process, unpacked: for tests and tools. The app goes
/// through [`Renderer`], whose worker calls [`render_packed`].
pub fn render(bytes: &[u8], limits: &Limits) -> Result<Art, RenderError> {
    render_packed(bytes, limits)?.unpacked()
}

/// Decodes `bytes` into packed frames, in this process: what the worker answers with. A model
/// packs each frame as it is drawn; a picture packs its frames once decoded.
pub fn render_packed(bytes: &[u8], limits: &Limits) -> Result<PackedArt, RenderError> {
    if bytes.len() > limits.max_input {
        return Err(RenderError::TooLarge);
    }
    // Unpacked first, within the same limit as a plain picture. A gzip file may only hold an
    // SVG (`.svgz`), and nothing is unpacked twice.
    let unpacked;
    let bytes = if limits.raw_deflate {
        unpacked = packing::inflate(bytes, limits.max_input)?;
        &unpacked[..]
    } else if packing::is_gzip(bytes) {
        unpacked = packing::gunzip(bytes, limits.max_input)?;
        if sniff(&unpacked) != Some(Format::Svg) {
            return Err(RenderError::Unsupported);
        }
        &unpacked[..]
    } else {
        bytes
    };
    let art = match sniff(bytes) {
        Some(Format::Glb) => {
            let packed = glb::render(bytes, limits)?;
            packed.check(limits)?;
            return Ok(packed);
        }
        Some(Format::Svg) => svg::render(bytes, limits)?,
        Some(format @ (Format::Png | Format::Jpeg | Format::Gif | Format::Webp)) => {
            raster::render(bytes, format, limits)?
        }
        None => return Err(RenderError::Unsupported),
    };
    // An animated GIF in the still pass: its first frame, with its delay.
    let art = if limits.first_frame_only && art.frames.len() > 1 {
        Art {
            frames: art.frames.into_iter().take(1).collect(),
            ..art
        }
    } else {
        art
    };
    art.check(limits)?;
    Ok(art.pack())
}
