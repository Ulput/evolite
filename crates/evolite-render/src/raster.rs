//! PNG (and APNG), JPEG, GIF, WebP through `image`, with its limits on: a header that claims
//! a huge picture is refused before the memory for it is taken.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::{FilterType, resize};
use image::{AnimationDecoder, ImageDecoder, ImageFormat, ImageReader, RgbaImage};

use crate::frames::{Art, Frame, Limits, RenderError, fit, premultiply};
use crate::sniff::Format;

/// Browsers show a frame of 0–10 ms for 100 ms; so do we, or such a GIF spins the CPU.
const SHORTEST_FRAME_MS: u32 = 20;
const DEFAULT_FRAME_MS: u32 = 100;

fn decoder_limits(limits: &Limits) -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(limits.max_source_side);
    l.max_image_height = Some(limits.max_source_side);
    l.max_alloc = Some(limits.max_total as u64);
    l
}

fn refusal(e: image::ImageError) -> RenderError {
    match e {
        image::ImageError::Limits(_) => RenderError::Limit("picture size"),
        image::ImageError::Unsupported(_) => RenderError::Unsupported,
        _ => RenderError::Invalid("cannot decode"),
    }
}

/// A frame scaled down to fit the limits.
fn frame(image: RgbaImage, delay_ms: u32, limits: &Limits) -> (u32, u32, Frame) {
    let (w, h) = fit(image.width(), image.height(), limits.max_side);
    let image = if (w, h) == image.dimensions() {
        image
    } else {
        resize(&image, w, h, FilterType::Triangle)
    };
    (
        w,
        h,
        Frame {
            rgba: {
                let mut rgba = image.into_raw();
                premultiply(&mut rgba);
                rgba
            },
            delay_ms,
        },
    )
}

fn delay_ms(frame: &image::Frame) -> u32 {
    let (numer, denom) = frame.delay().numer_denom_ms();
    let ms = numer.checked_div(denom.max(1)).unwrap_or(0);
    if ms <= 10 {
        DEFAULT_FRAME_MS
    } else {
        ms.max(SHORTEST_FRAME_MS)
    }
}

/// All frames of an animation, stopping as soon as one limit is crossed.
fn animation<'a>(decoder: impl AnimationDecoder<'a>, limits: &Limits) -> Result<Art, RenderError> {
    let mut size = None;
    let mut frames = Vec::new();
    let mut total = 0usize;
    for next in decoder.into_frames() {
        let next = next.map_err(refusal)?;
        if frames.len() == limits.max_frames {
            return Err(RenderError::Limit("frames"));
        }
        let delay = delay_ms(&next);
        let (w, h, f) = frame(next.into_buffer(), delay, limits);
        total = total.saturating_add(f.rgba.len());
        if total > limits.max_total {
            return Err(RenderError::Limit("decoded size"));
        }
        if *size.get_or_insert((w, h)) != (w, h) {
            return Err(RenderError::Invalid("frames of different sizes"));
        }
        frames.push(f);
    }
    let (width, height) = size.ok_or(RenderError::Invalid("no frames"))?;
    if let [only] = frames.as_mut_slice() {
        only.delay_ms = 0;
    }
    Ok(Art {
        width,
        height,
        frames,
    })
}

fn still(bytes: &[u8], format: ImageFormat, limits: &Limits) -> Result<Art, RenderError> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(decoder_limits(limits));
    let image = reader.decode().map_err(refusal)?.into_rgba8();
    let (width, height, frame) = frame(image, 0, limits);
    Ok(Art {
        width,
        height,
        frames: vec![frame],
    })
}

pub(crate) fn render(bytes: &[u8], format: Format, limits: &Limits) -> Result<Art, RenderError> {
    match format {
        Format::Gif => {
            let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(refusal)?;
            decoder
                .set_limits(decoder_limits(limits))
                .map_err(refusal)?;
            animation(decoder, limits)
        }
        Format::Webp => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(refusal)?;
            decoder
                .set_limits(decoder_limits(limits))
                .map_err(refusal)?;
            if decoder.has_animation() {
                animation(decoder, limits)
            } else {
                still(bytes, ImageFormat::WebP, limits)
            }
        }
        Format::Png => {
            let mut decoder = PngDecoder::new(Cursor::new(bytes)).map_err(refusal)?;
            decoder
                .set_limits(decoder_limits(limits))
                .map_err(refusal)?;
            if decoder.is_apng().map_err(refusal)? {
                animation(decoder.apng().map_err(refusal)?, limits)
            } else {
                still(bytes, ImageFormat::Png, limits)
            }
        }
        Format::Jpeg => still(bytes, ImageFormat::Jpeg, limits),
        Format::Svg | Format::Glb => Err(RenderError::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Rgba};

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, Rgba([200, 10, 10, 255]));
        let mut out = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    fn gif(frames: usize) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            for i in 0..frames {
                let image = RgbaImage::from_pixel(4, 3, Rgba([i as u8 * 40, 0, 0, 255]));
                let frame =
                    image::Frame::from_parts(image, 0, 0, Delay::from_numer_denom_ms(70, 1));
                encoder.encode_frame(frame).unwrap();
            }
        }
        out
    }

    /// A PNG whose header claims `width × height`, with no pixel data behind it.
    fn png_claiming(width: u32, height: u32) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = !0u32;
            for &b in data {
                crc ^= b as u32;
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        (crc >> 1) ^ 0xedb8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend(width.to_be_bytes());
        ihdr.extend(height.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        // IHDR with the claimed size, an IDAT with an empty zlib stream, IEND.
        for chunk in [
            ihdr,
            b"IDAT\x78\x9c\x03\x00\x00\x00\x00\x01".to_vec(),
            b"IEND".to_vec(),
        ] {
            out.extend((chunk.len() as u32 - 4).to_be_bytes());
            out.extend(&chunk);
            out.extend(crc32(&chunk).to_be_bytes());
        }
        out
    }

    #[test]
    fn a_still_png() {
        let art = render(&png(5, 3), Format::Png, &Limits::default()).unwrap();
        assert_eq!((art.width, art.height, art.frames.len()), (5, 3, 1));
        assert_eq!(&art.frames[0].rgba[..4], &[200, 10, 10, 255]);
        assert_eq!(art.frames[0].delay_ms, 0);
    }

    #[test]
    fn a_large_picture_is_scaled_down() {
        let limits = Limits {
            max_side: 16,
            ..Limits::default()
        };
        let art = render(&png(64, 32), Format::Png, &limits).unwrap();
        assert_eq!((art.width, art.height), (16, 8));
    }

    #[test]
    fn a_header_claiming_a_huge_picture_is_refused_before_decoding() {
        let bomb = png_claiming(100_000, 100_000);
        assert_eq!(
            render(&bomb, Format::Png, &Limits::default()),
            Err(RenderError::Limit("picture size"))
        );
    }

    #[test]
    fn a_gif_keeps_its_frames_and_delays() {
        let art = render(&gif(3), Format::Gif, &Limits::default()).unwrap();
        assert_eq!(art.frames.len(), 3);
        assert!(art.frames.iter().all(|f| f.delay_ms == 70));
        assert_ne!(art.frames[0].rgba, art.frames[2].rgba);
    }

    #[test]
    fn too_many_frames_are_refused() {
        let limits = Limits {
            max_frames: 2,
            ..Limits::default()
        };
        assert_eq!(
            render(&gif(3), Format::Gif, &limits),
            Err(RenderError::Limit("frames"))
        );
    }

    #[test]
    fn garbage_after_a_good_header_is_refused() {
        let mut bytes = png(5, 3);
        bytes.truncate(40);
        assert!(matches!(
            render(&bytes, Format::Png, &Limits::default()),
            Err(RenderError::Invalid(_))
        ));
    }
}
