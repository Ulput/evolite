//! «Fit into the record» (FEAT-0038 R-21, the architect's answers 1–3): a picture too large
//! for a model's write is made into a smaller copy that fits, here in the worker — the wallet
//! never decodes it. The copy is encoded from pixels, so nothing of the file's metadata goes
//! with it; an animation gives its first frame.
//!
//! - **Opaque** (every pixel fully opaque): JPEG, the long side at most [`START_SIDE`], the
//!   quality by binary search in [`QUALITY`] for the highest that fits; not even at the lowest —
//!   the long side × 0.8 and again, while it is at least [`MIN_SIDE`].
//! - **With transparency**: PNG with a palette (NeuQuant over RGBA, the alpha in `tRNS`),
//!   [`COLOURS`] in turn, then the long side × 0.8 the same way.
//! - **A deadline** over the whole search: past it, the best copy that fitted, if any; else
//!   «does not fit». So is a picture that does not fit even at the smallest.

use std::io::Cursor;
use std::time::{Duration, Instant};

use image::codecs::gif::GifDecoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::{FilterType, resize};
use image::{
    AnimationDecoder, ExtendedColorType, ImageDecoder, ImageFormat, ImageReader, RgbaImage,
};

use crate::frames::{Limits, RenderError};
use crate::raster::{decoder_limits, refusal};
use crate::sniff::{Format, sniff};

/// The long side a JPEG copy starts from.
pub const START_SIDE: u32 = 1024;
/// The long side no copy goes below.
pub const MIN_SIDE: u32 = 128;
/// JPEG quality, lowest and highest tried.
pub const QUALITY: (u8, u8) = (40, 85);
/// Palette sizes tried in turn.
pub const COLOURS: [u16; 3] = [256, 128, 64];
/// The whole search, inside the worker.
pub const DEADLINE: Duration = Duration::from_secs(5);

/// What the copy was made as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitFormat {
    Jpeg,
    /// PNG with a palette.
    Png,
}

/// A copy that fits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fitted {
    pub bytes: Vec<u8>,
    pub format: FitFormat,
    pub width: u32,
    pub height: u32,
    /// The JPEG quality, or the palette's colours.
    pub setting: u16,
    /// The source's own size.
    pub source_width: u32,
    pub source_height: u32,
}

/// The first frame of a raster as straight (not premultiplied) RGBA, within `limits`.
fn first_frame(bytes: &[u8], format: Format, limits: &Limits) -> Result<RgbaImage, RenderError> {
    let still = |format: ImageFormat| -> Result<RgbaImage, RenderError> {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(decoder_limits(limits));
        Ok(reader.decode().map_err(refusal)?.into_rgba8())
    };
    let first = |frames: image::Frames| -> Result<RgbaImage, RenderError> {
        frames
            .into_iter()
            .next()
            .ok_or(RenderError::Invalid("no frames"))?
            .map(image::Frame::into_buffer)
            .map_err(refusal)
    };
    match format {
        Format::Jpeg => still(ImageFormat::Jpeg),
        // An APNG's default image is its first frame.
        Format::Png => still(ImageFormat::Png),
        Format::Gif => {
            let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(refusal)?;
            decoder
                .set_limits(decoder_limits(limits))
                .map_err(refusal)?;
            first(decoder.into_frames())
        }
        Format::Webp => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(refusal)?;
            decoder
                .set_limits(decoder_limits(limits))
                .map_err(refusal)?;
            if decoder.has_animation() {
                first(decoder.into_frames())
            } else {
                still(ImageFormat::WebP)
            }
        }
        // Vector pictures and models are never made smaller here (R-21).
        Format::Svg | Format::Glb => Err(RenderError::Unsupported),
    }
}

/// `(w, h)` with the long side `long`, the proportion kept, each side at least 1.
fn scaled(w: u32, h: u32, long: u32) -> (u32, u32) {
    let longest = w.max(h).max(1);
    if longest <= long {
        return (w, h);
    }
    let k = long as f64 / longest as f64;
    (
        ((w as f64 * k).round() as u32).max(1),
        ((h as f64 * k).round() as u32).max(1),
    )
}

fn jpeg(image: &RgbaImage, quality: u8) -> Result<Vec<u8>, RenderError> {
    let rgb = image::DynamicImage::ImageRgba8(image.clone()).into_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|_| RenderError::Invalid("cannot decode"))?;
    Ok(out)
}

/// PNG with a palette of at most `colours`, transparency in `tRNS`.
fn palette_png(image: &RgbaImage, colours: u16) -> Result<Vec<u8>, RenderError> {
    let quant = color_quant::NeuQuant::new(10, colours as usize, image.as_raw());
    let map = quant.color_map_rgba();
    let indices: Vec<u8> = image
        .as_raw()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|px| quant.index_of(px) as u8)
        .collect();
    let colours = map.as_chunks::<4>().0;
    let rgb: Vec<u8> = colours.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
    let mut alpha: Vec<u8> = colours.iter().map(|c| c[3]).collect();
    // Fully opaque entries at the end need not be listed.
    while alpha.last() == Some(&255) {
        alpha.pop();
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width(), image.height());
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_palette(rgb);
        if !alpha.is_empty() {
            encoder.set_trns(alpha);
        }
        encoder.set_compression(png::Compression::High);
        let mut writer = encoder
            .write_header()
            .map_err(|_| RenderError::Invalid("cannot decode"))?;
        writer
            .write_image_data(&indices)
            .map_err(|_| RenderError::Invalid("cannot decode"))?;
    }
    Ok(out)
}

/// A copy of `bytes` of at most `limit` bytes, found within `deadline`.
pub fn fit(
    bytes: &[u8],
    limit: usize,
    limits: &Limits,
    deadline: Duration,
) -> Result<Fitted, RenderError> {
    let started = Instant::now();
    let format = sniff(bytes).ok_or(RenderError::Unsupported)?;
    let source = first_frame(bytes, format, limits)?;
    let (source_width, source_height) = source.dimensions();
    let opaque = source
        .as_raw()
        .as_chunks::<4>()
        .0
        .iter()
        .all(|px| px[3] == 255);
    let late = || started.elapsed() >= deadline;
    let mut best: Option<Fitted> = None;
    let mut long = source_width.max(source_height).min(START_SIDE);
    loop {
        let (w, h) = scaled(source_width, source_height, long);
        let image = if (w, h) == source.dimensions() {
            source.clone()
        } else {
            resize(&source, w, h, FilterType::Triangle)
        };
        let made = |bytes: Vec<u8>, format, setting| Fitted {
            bytes,
            format,
            width: w,
            height: h,
            setting,
            source_width,
            source_height,
        };
        if opaque {
            // The highest quality that fits, by halving 40…85.
            let (mut low, mut high) = (QUALITY.0, QUALITY.1);
            while low <= high {
                if late() {
                    return best.ok_or(RenderError::Limit("does not fit"));
                }
                let q = low + (high - low) / 2;
                let out = jpeg(&image, q)?;
                if out.len() <= limit {
                    best = Some(made(out, FitFormat::Jpeg, q as u16));
                    low = q + 1;
                } else if q == 0 {
                    break;
                } else {
                    high = q - 1;
                }
            }
        } else {
            for colours in COLOURS {
                if late() {
                    return best.ok_or(RenderError::Limit("does not fit"));
                }
                let out = palette_png(&image, colours)?;
                if out.len() <= limit {
                    best = Some(made(out, FitFormat::Png, colours));
                    break;
                }
            }
        }
        if let Some(found) = best {
            return Ok(found);
        }
        let next = (long as f64 * 0.8) as u32;
        if next < MIN_SIDE || long <= MIN_SIDE {
            return Err(RenderError::Limit("does not fit"));
        }
        long = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(img: RgbaImage, format: ImageFormat) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgba8(img);
        let img = match format {
            ImageFormat::Jpeg => image::DynamicImage::ImageRgb8(img.to_rgb8()),
            _ => img,
        };
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    /// A photo-like picture: smooth colour with some texture.
    fn photo(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            let n = ((x * 7919 + y * 104_729) % 61) as u8;
            image::Rgba([
                (x * 255 / w) as u8 ^ (n & 7),
                (y * 255 / h) as u8,
                ((x + y) % 256) as u8 ^ n,
                255,
            ])
        })
    }

    fn noise(w: u32, h: u32) -> RgbaImage {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        RgbaImage::from_fn(w, h, |_, _| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let b = seed.to_le_bytes();
            image::Rgba([b[0], b[1], b[2], 255])
        })
    }

    fn has(bytes: &[u8], what: &[u8]) -> bool {
        bytes.windows(what.len()).any(|w| w == what)
    }

    /// The architect's test: a photo of about 300 KB fits into the limit as a JPEG; its type by
    /// the first bytes is JPEG, it reads back, and it keeps its proportion.
    #[test]
    fn a_photo_fits_as_a_jpeg() {
        let file = encoded(photo(1600, 1000), ImageFormat::Png);
        assert!(file.len() > 300_000, "{}", file.len());
        let limit = 18_000;
        let copy = fit(&file, limit, &Limits::default(), DEADLINE).unwrap();
        assert_eq!(copy.format, FitFormat::Jpeg);
        assert!(copy.bytes.len() <= limit);
        assert_eq!(sniff(&copy.bytes), Some(Format::Jpeg));
        let back = image::load_from_memory(&copy.bytes).unwrap();
        assert_eq!((back.width(), back.height()), (copy.width, copy.height));
        assert_eq!((copy.source_width, copy.source_height), (1600, 1000));
        let ratio = copy.width as f64 / copy.height as f64;
        assert!((ratio - 1.6).abs() < 0.02, "{ratio}");
    }

    /// The architect's test: a PNG with transparency fits as a PNG with a palette and keeps
    /// transparent pixels.
    #[test]
    fn a_picture_with_alpha_fits_as_a_palette_png_and_stays_transparent() {
        let img = RgbaImage::from_fn(900, 900, |x, y| {
            let inside = (x as i64 - 450).pow(2) + (y as i64 - 450).pow(2) < 350i64.pow(2);
            image::Rgba([
                (x / 4) as u8,
                (y / 4) as u8,
                ((x ^ y) & 0xFF) as u8,
                if inside { 255 } else { 0 },
            ])
        });
        let file = encoded(img, ImageFormat::Png);
        let limit = 18_000;
        assert!(file.len() > limit);
        let copy = fit(&file, limit, &Limits::default(), DEADLINE).unwrap();
        assert_eq!(copy.format, FitFormat::Png);
        assert!(copy.bytes.len() <= limit);
        assert_eq!(sniff(&copy.bytes), Some(Format::Png));
        let back = image::load_from_memory(&copy.bytes).unwrap().to_rgba8();
        assert!(back.pixels().any(|p| p[3] == 0), "transparent pixels kept");
        assert!(back.pixels().any(|p| p[3] == 255));
    }

    /// The architect's test: noise of 2000 × 2000 does not fit; the answer is an honest
    /// «does not fit» within the deadline (with a little room for the last encode).
    #[test]
    fn noise_does_not_fit_and_says_so_in_time() {
        let file = encoded(noise(2000, 2000), ImageFormat::Png);
        let started = Instant::now();
        let answer = fit(&file, 2_000, &Limits::default(), DEADLINE);
        assert_eq!(answer, Err(RenderError::Limit("does not fit")));
        assert!(
            started.elapsed() < DEADLINE + Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    /// AC-A21: a JPEG with an EXIF geotag gives a copy with neither «Exif» nor «GPS» in it.
    #[test]
    fn the_copy_has_no_exif() {
        let clean = encoded(photo(1200, 900), ImageFormat::Jpeg);
        let exif = b"Exif\0\0MM\0*GPSLatitude 55.7558";
        let mut file = clean[..2].to_vec();
        file.extend_from_slice(&[0xFF, 0xE1]);
        file.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        file.extend_from_slice(exif);
        file.extend_from_slice(&clean[2..]);
        let limit = (file.len() / 2).min(18_000);
        let copy = fit(&file, limit, &Limits::default(), DEADLINE).unwrap();
        assert!(!has(&copy.bytes, b"Exif") && !has(&copy.bytes, b"GPS"));
    }

    /// An animated GIF gives its first frame, still.
    #[test]
    fn an_animation_gives_its_first_frame() {
        use image::codecs::gif::GifEncoder;
        let mut file = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut file);
            for shade in [10u8, 200] {
                let frame = image::Frame::new(RgbaImage::from_fn(300, 300, |x, y| {
                    image::Rgba([shade, (x % 256) as u8, (y % 256) as u8, 255])
                }));
                enc.encode_frame(frame).unwrap();
            }
        }
        let copy = fit(&file, file.len() / 3, &Limits::default(), DEADLINE).unwrap();
        let back = image::load_from_memory(&copy.bytes).unwrap().to_rgba8();
        let red = back.get_pixel(0, 0)[0];
        assert!(red < 60, "the first frame's shade, not the second's: {red}");
    }

    /// SVG and GLB are never made smaller here.
    #[test]
    fn vector_pictures_and_models_are_not_fitted() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="9" height="9"/></svg>"#;
        assert_eq!(
            fit(svg, 10, &Limits::default(), DEADLINE),
            Err(RenderError::Unsupported)
        );
    }
}
