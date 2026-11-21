//! A model's picture without what its file says besides the picture (FEAT-0038 R-22, the
//! architect's question-5 answer): EXIF and its geotag, XMP, text chunks, comments, times,
//! and whatever follows the picture's end. Byte by byte, by the file's structure only —
//! segments and chunks with every length checked — nothing is decoded or unpacked, so no
//! decoder runs in the wallet. A whitelist: what is not known to be the picture goes.
//!
//! A structure that cannot be followed (cut short, a length past the end, an unknown critical
//! part) is refused: such a file is never written «as it is». GLB is left whole (its details
//! are inside its JSON); the screen says so.

use super::image::{ImageType, image_type};

/// The picture as it will be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stripped {
    pub bytes: Vec<u8>,
    /// Something was taken out (R-22: «Details … were removed» shows only then).
    pub removed: bool,
}

/// The file's structure could not be followed: «Could not check the picture…». The words are
/// for the journal and the tests, never shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unreadable(pub &'static str);

/// `bytes` without their metadata (R-22). The same type comes out, never larger.
pub fn strip_metadata(bytes: &[u8]) -> Result<Stripped, Unreadable> {
    let kind = image_type(bytes).ok_or(Unreadable("not a picture"))?;
    let out = match kind {
        ImageType::Jpeg => jpeg(bytes)?,
        ImageType::Png => png(bytes)?,
        ImageType::Webp => webp(bytes)?,
        ImageType::Gif => gif(bytes)?,
        ImageType::Svg | ImageType::AnimatedSvg => svg(bytes)?,
        ImageType::Glb => bytes.to_vec(),
    };
    // Rule 2: the same type, never larger.
    if image_type(&out) != Some(kind) || out.len() > bytes.len() {
        return Err(Unreadable("changed its type"));
    }
    let removed = out.len() != bytes.len() || out != bytes;
    Ok(Stripped {
        bytes: out,
        removed,
    })
}

/// A big-endian u16 at `at`, inside `b`.
fn be16(b: &[u8], at: usize) -> Result<usize, Unreadable> {
    b.get(at..at + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]) as usize)
        .ok_or(Unreadable("cut short"))
}

fn be32(b: &[u8], at: usize) -> Result<usize, Unreadable> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize)
        .ok_or(Unreadable("cut short"))
}

fn le32(b: &[u8], at: usize) -> Result<usize, Unreadable> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
        .ok_or(Unreadable("cut short"))
}

/// `b[from..from + len]`, or «cut short».
fn span(b: &[u8], from: usize, len: usize) -> Result<&[u8], Unreadable> {
    from.checked_add(len)
        .and_then(|end| b.get(from..end))
        .ok_or(Unreadable("a length past the end"))
}

/// JPEG by its segments. Kept: SOI, SOFn, DHT, DQT, DRI, DAC, DNL, SOS with its scan data, EOI,
/// APP0 «JFIF», APP14 «Adobe» (colour), APP2 «ICC_PROFILE». Gone: APP1 (EXIF, XMP, and the EXIF
/// thumbnail in it), every other APPn (APP2 «MPF» too), COM, and everything after the first EOI
/// of the main picture (a «live photo»'s video). The end is found by walking the segments, not
/// by looking for FF D9: in scan data FF 00 is a stuffed byte and FF D0…D7 are restarts.
fn jpeg(b: &[u8]) -> Result<Vec<u8>, Unreadable> {
    let mut out = Vec::with_capacity(b.len());
    out.extend_from_slice(&b[..2]); // SOI, checked by `image_type`.
    let mut at = 2;
    let (mut frame, mut scan) = (false, false);
    loop {
        // A marker: FF, maybe fill bytes FF, then its code.
        if b.get(at) != Some(&0xFF) {
            return Err(Unreadable("no marker where one is due"));
        }
        while b.get(at) == Some(&0xFF) {
            at += 1;
        }
        let code = *b.get(at).ok_or(Unreadable("cut short"))?;
        at += 1;
        match code {
            0xD9 => {
                if !(frame && scan) {
                    return Err(Unreadable("ends before its picture"));
                }
                out.extend_from_slice(&[0xFF, 0xD9]);
                return Ok(out);
            }
            // Restart markers between scan pieces are copied as they are.
            0xD0..=0xD7 => out.extend_from_slice(&[0xFF, code]),
            0xDA => {
                if !frame {
                    return Err(Unreadable("a scan before its frame"));
                }
                let len = be16(b, at)?;
                if len < 2 {
                    return Err(Unreadable("a segment shorter than its length"));
                }
                out.extend_from_slice(&[0xFF, code]);
                out.extend_from_slice(span(b, at, len)?);
                at += len;
                // The scan's data, up to the next marker that is not a stuffed byte or a
                // restart.
                let start = at;
                loop {
                    let byte = *b.get(at).ok_or(Unreadable("a scan cut short"))?;
                    if byte == 0xFF {
                        let next = *b.get(at + 1).ok_or(Unreadable("a scan cut short"))?;
                        if next != 0x00 && !(0xD0..=0xD7).contains(&next) {
                            break;
                        }
                        at += 2;
                    } else {
                        at += 1;
                    }
                }
                out.extend_from_slice(&b[start..at]);
                scan = true;
            }
            // Standalone markers that carry nothing but should not be here.
            0x01 | 0xD8 => return Err(Unreadable("an unexpected marker")),
            _ => {
                let len = be16(b, at)?;
                if len < 2 {
                    return Err(Unreadable("a segment shorter than its length"));
                }
                let body = span(b, at, len)?;
                let data = &body[2..];
                let keep = match code {
                    // SOFn (not DHT C4, JPG C8, DAC CC), DHT, DAC, DQT, DNL, DRI.
                    0xC0..=0xCF => {
                        if !matches!(code, 0xC4 | 0xC8 | 0xCC) {
                            frame = true;
                        }
                        code != 0xC8
                    }
                    0xDB..=0xDD => true,
                    0xE0 => data.starts_with(b"JFIF\0"),
                    0xE2 => data.starts_with(b"ICC_PROFILE\0"),
                    0xEE => data.starts_with(b"Adobe"),
                    // The other APPn and COM: metadata.
                    0xE1 | 0xE3..=0xED | 0xEF | 0xFE => false,
                    // DHP, EXP, JPGn and the reserved: not a picture this reads.
                    _ => return Err(Unreadable("an unknown segment")),
                };
                if keep {
                    out.extend_from_slice(&[0xFF, code]);
                    out.extend_from_slice(body);
                }
                at += len;
            }
        }
    }
}

/// CRC-32 (ISO 3309, as PNG uses) of `parts` in order.
fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for part in parts {
        for &byte in *part {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xEDB8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
    }
    !crc
}

/// PNG by its chunks, each one's CRC checked. Kept: IHDR, PLTE, IDAT, IEND, tRNS, gAMA, cHRM,
/// sRGB, iCCP, pHYs, sBIT, bKGD, and APNG's acTL, fcTL, fdAT, their CRCs untouched. Gone: tEXt,
/// zTXt, iTXt, eXIf, tIME and every other ancillary chunk, and anything after IEND. An unknown
/// critical chunk cannot be dropped: refused.
fn png(b: &[u8]) -> Result<Vec<u8>, Unreadable> {
    const KEEP: [&[u8; 4]; 15] = [
        b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"pHYs",
        b"sBIT", b"bKGD", b"acTL", b"fcTL", b"fdAT",
    ];
    let mut out = b[..8].to_vec();
    let mut at = 8;
    let mut first = true;
    loop {
        let len = be32(b, at)?;
        if len > 0x7FFF_FFFF {
            return Err(Unreadable("a chunk too long"));
        }
        let kind = span(b, at + 4, 4)?;
        let data = span(b, at + 8, len)?;
        let crc = be32(b, at + 8 + len)?;
        if crc32(&[kind, data]) as usize != crc {
            return Err(Unreadable("a chunk's CRC"));
        }
        if first && kind != b"IHDR" {
            return Err(Unreadable("IHDR is not first"));
        }
        first = false;
        let whole = span(b, at, 12 + len)?;
        if KEEP.iter().any(|k| k.as_slice() == kind) {
            out.extend_from_slice(whole);
        } else if kind[0].is_ascii_uppercase() {
            return Err(Unreadable("an unknown critical chunk"));
        }
        at += 12 + len;
        if kind == b"IEND" {
            return Ok(out);
        }
    }
}

/// WebP's RIFF chunks. Gone: EXIF and «XMP », their flags in VP8X cleared, everything past the
/// size RIFF gives, and chunks this does not know; the RIFF size counted again. Each chunk is
/// padded to an even length.
fn webp(b: &[u8]) -> Result<Vec<u8>, Unreadable> {
    const KEEP: [&[u8; 4]; 7] = [
        b"VP8 ", b"VP8L", b"VP8X", b"ALPH", b"ANIM", b"ANMF", b"ICCP",
    ];
    let riff = le32(b, 4)?;
    let end = riff
        .checked_add(8)
        .filter(|end| *end <= b.len())
        .ok_or(Unreadable("RIFF longer than the file"))?;
    let mut out = b[..12].to_vec();
    let mut at = 12;
    let mut image = false;
    while at < end {
        let kind = span(b, at, 4)?;
        let len = le32(b, at + 4)?;
        let padded = len
            .checked_add(len & 1)
            .ok_or(Unreadable("a chunk too long"))?;
        let whole = span(b, at, 8 + padded)?;
        if at + 8 + padded > end {
            return Err(Unreadable("a chunk past RIFF's end"));
        }
        if kind == b"VP8X" {
            let mut chunk = whole.to_vec();
            let flags = chunk.get_mut(8).ok_or(Unreadable("VP8X cut short"))?;
            // EXIF 0x08, XMP 0x04.
            *flags &= !0x0C;
            out.extend_from_slice(&chunk);
        } else if KEEP.iter().any(|k| k.as_slice() == kind) {
            image |= matches!(kind, b"VP8 " | b"VP8L" | b"ANMF");
            out.extend_from_slice(whole);
        }
        at += 8 + padded;
    }
    if !image {
        return Err(Unreadable("no picture in the RIFF"));
    }
    let size = u32::try_from(out.len() - 8).map_err(|_| Unreadable("too long"))?;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    Ok(out)
}

/// GIF's blocks. Gone: comment extensions, application extensions other than «NETSCAPE2.0» (the
/// animation's loop), and anything after the trailer.
fn gif(b: &[u8]) -> Result<Vec<u8>, Unreadable> {
    // Header 6, logical screen 7, then the global colour table if its flag is set.
    let packed = *b.get(10).ok_or(Unreadable("cut short"))?;
    let mut at = 13;
    if packed & 0x80 != 0 {
        at += 3 << ((packed & 0x07) + 1);
    }
    let mut out = span(b, 0, at)?.to_vec();
    // Sub-blocks from `at` up to and with the empty one; where they end.
    let blocks = |mut at: usize| -> Result<usize, Unreadable> {
        loop {
            let n = *b.get(at).ok_or(Unreadable("cut short"))? as usize;
            span(b, at + 1, n)?;
            at += 1 + n;
            if n == 0 {
                return Ok(at);
            }
        }
    };
    let mut frames = 0;
    loop {
        match *b.get(at).ok_or(Unreadable("no trailer"))? {
            0x3B => {
                if frames == 0 {
                    return Err(Unreadable("no frame"));
                }
                out.push(0x3B);
                return Ok(out);
            }
            0x2C => {
                let local = *b.get(at + 9).ok_or(Unreadable("cut short"))?;
                let mut data = at + 10;
                if local & 0x80 != 0 {
                    data += 3 << ((local & 0x07) + 1);
                }
                // The LZW code size, then its sub-blocks.
                span(b, data, 1)?;
                let end = blocks(data + 1)?;
                out.extend_from_slice(&b[at..end]);
                at = end;
                frames += 1;
            }
            0x21 => {
                let label = *b.get(at + 1).ok_or(Unreadable("cut short"))?;
                let end = blocks(at + 2)?;
                let keep = match label {
                    // Graphic control, plain text: part of what is drawn.
                    0xF9 | 0x01 => true,
                    0xFE => false,
                    0xFF => {
                        b.get(at + 2) == Some(&11) && b.get(at + 3..at + 14) == Some(b"NETSCAPE2.0")
                    }
                    _ => return Err(Unreadable("an unknown extension")),
                };
                if keep {
                    out.extend_from_slice(&b[at..end]);
                }
                at = end;
            }
            _ => return Err(Unreadable("an unknown block")),
        }
    }
}

/// SVG without `<metadata>` (under any prefix) and comments, by the XML's own ranges; the rest
/// byte for byte. Not XML: refused (`unsafe_svg` refuses it too).
fn svg(b: &[u8]) -> Result<Vec<u8>, Unreadable> {
    let text = std::str::from_utf8(b).map_err(|_| Unreadable("not UTF-8"))?;
    let doc = roxmltree::Document::parse(text).map_err(|_| Unreadable("not XML"))?;
    let mut cut: Vec<std::ops::Range<usize>> = doc
        .descendants()
        .filter(|n| {
            n.is_comment()
                || (n.is_element() && n.tag_name().name().eq_ignore_ascii_case("metadata"))
        })
        .map(|n| n.range())
        .collect();
    cut.sort_by_key(|r| r.start);
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for r in cut {
        // A comment inside a `<metadata>` already cut.
        if r.start < from {
            continue;
        }
        out.push_str(&text[from..r.start]);
        from = r.end;
    }
    out.push_str(&text[from..]);
    Ok(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, RgbaImage};

    /// A 16 × 16 picture with some colour in it, in `format`.
    fn encoded(format: ImageFormat, alpha: bool) -> Vec<u8> {
        let img = RgbaImage::from_fn(16, 16, |x, y| {
            let a = if alpha { (x * 8) as u8 } else { 255 };
            image::Rgba([(x * 16) as u8, (y * 16) as u8, 128, a])
        });
        let img = DynamicImage::ImageRgba8(img);
        let img = match format {
            ImageFormat::Jpeg => DynamicImage::ImageRgb8(img.to_rgb8()),
            _ => img,
        };
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    fn pixels(bytes: &[u8]) -> Vec<u8> {
        image::load_from_memory(bytes)
            .unwrap()
            .to_rgba8()
            .into_raw()
    }

    fn has(bytes: &[u8], what: &[u8]) -> bool {
        bytes.windows(what.len()).any(|w| w == what)
    }

    /// A JPEG segment with its length.
    fn seg(code: u8, body: &[u8]) -> Vec<u8> {
        let mut s = vec![0xFF, code];
        s.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        s.extend_from_slice(body);
        s
    }

    /// A PNG chunk with its length and CRC.
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = (data.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(kind);
        c.extend_from_slice(data);
        c.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
        c
    }

    /// R-22, AC-A22: a JPEG with a geotag, a comment and a «live photo»'s video after its end —
    /// no Exif, no GPS, it ends at its EOI, the pixels are the same.
    #[test]
    fn a_jpeg_loses_its_geotag_and_its_tail_not_its_pixels() {
        let clean = encoded(ImageFormat::Jpeg, false);
        let mut file = clean[..2].to_vec();
        file.extend(seg(
            0xE1,
            b"Exif\0\0MM\0*GPSLatitude 55.7558 GPSLongitude 37.6173",
        ));
        file.extend(seg(0xFE, b"camera"));
        file.extend_from_slice(&clean[2..]);
        file.extend_from_slice(b"ftypisom....moov....GPS 55.75 37.61");
        let out = strip_metadata(&file).unwrap();
        assert!(out.removed);
        assert!(!has(&out.bytes, b"Exif") && !has(&out.bytes, b"GPS"));
        assert!(out.bytes.ends_with(&[0xFF, 0xD9]));
        assert_eq!(out.bytes, clean, "only what was added went");
        assert_eq!(pixels(&out.bytes), pixels(&clean));
    }

    /// A JPEG with nothing to take out comes out byte for byte, «removed» not said.
    #[test]
    fn a_clean_jpeg_is_left_as_it_is() {
        let clean = encoded(ImageFormat::Jpeg, false);
        let out = strip_metadata(&clean).unwrap();
        assert!(!out.removed);
        assert_eq!(out.bytes, clean);
    }

    /// The architect's rule: the end is found by the segments — FF D9 after a stuffed FF 00 in a
    /// scan is data, a restart marker is data; a progressive file's several scans with tables
    /// between them stay whole.
    #[test]
    fn a_progressive_jpeg_keeps_every_scan_and_a_stuffed_ff_d9_is_data() {
        let mut file = vec![0xFF, 0xD8];
        file.extend(seg(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
        file.extend(seg(0xDB, &[0; 65]));
        file.extend(seg(0xC2, &[8, 0, 16, 0, 16, 1, 1, 0x11, 0]));
        file.extend(seg(0xC4, &[0; 20]));
        file.extend(seg(0xDA, &[1, 1, 0, 0, 0x3F, 0]));
        file.extend_from_slice(&[0x12, 0xFF, 0x00, 0xD9, 0x34, 0xFF, 0xD0, 0x56]);
        file.extend(seg(0xC4, &[0; 20]));
        file.extend(seg(0xDA, &[1, 1, 0, 1, 0x3F, 0]));
        file.extend_from_slice(&[0x78, 0xFF, 0x00, 0x9A]);
        file.extend_from_slice(&[0xFF, 0xD9]);
        let out = strip_metadata(&file).unwrap();
        assert!(!out.removed, "nothing was metadata");
        assert_eq!(out.bytes, file);
    }

    /// APP2 keeps only its ICC profile: «MPF» (a camera file's other pictures) goes; Adobe's
    /// APP14 stays (the colours need it); Photoshop's APP13 goes.
    #[test]
    fn only_the_colour_segments_stay() {
        let clean = encoded(ImageFormat::Jpeg, false);
        let mut file = clean[..2].to_vec();
        file.extend(seg(0xE2, b"ICC_PROFILE\0\x01\x01profile"));
        file.extend(seg(0xE2, b"MPF\0II*\0"));
        file.extend(seg(0xEE, b"Adobe\0\x64\0\0\0\0\x01"));
        file.extend(seg(0xED, b"Photoshop 3.0\08BIM"));
        file.extend_from_slice(&clean[2..]);
        let out = strip_metadata(&file).unwrap();
        assert!(out.removed);
        assert!(has(&out.bytes, b"ICC_PROFILE") && has(&out.bytes, b"Adobe"));
        assert!(!has(&out.bytes, b"MPF") && !has(&out.bytes, b"Photoshop"));
    }

    /// AC-A22: a PNG's text, EXIF and time chunks go, and what follows IEND; its pixels stay.
    #[test]
    fn a_png_loses_its_text_and_tail_not_its_pixels() {
        let clean = encoded(ImageFormat::Png, true);
        let ihdr_end = 8 + 12 + 13;
        let mut file = clean[..ihdr_end].to_vec();
        file.extend(chunk(b"tEXt", b"Author\0Satoshi"));
        file.extend(chunk(b"eXIf", b"MM\0*GPS"));
        file.extend(chunk(b"tIME", &[7, 234, 11, 21, 10, 0, 0]));
        file.extend_from_slice(&clean[ihdr_end..]);
        file.extend_from_slice(b"trailing bytes after IEND");
        let out = strip_metadata(&file).unwrap();
        assert!(out.removed);
        assert_eq!(out.bytes, clean);
        assert_eq!(pixels(&out.bytes), pixels(&clean));
    }

    /// AC-A22: a PNG with nothing to take out: the same bytes, nothing said.
    #[test]
    fn a_clean_png_is_left_as_it_is() {
        let clean = encoded(ImageFormat::Png, true);
        let out = strip_metadata(&clean).unwrap();
        assert!(!out.removed);
        assert_eq!(out.bytes, clean);
    }

    /// A PNG whose chunk is spoiled (its CRC does not match) is refused.
    #[test]
    fn a_png_with_a_wrong_crc_is_refused() {
        let mut file = encoded(ImageFormat::Png, true);
        let last_idat_byte = file.len() - 12 - 5;
        file[last_idat_byte] ^= 0x55;
        assert!(strip_metadata(&file).is_err());
    }

    /// A WebP's EXIF and XMP chunks go, their flags in VP8X are cleared, the RIFF size is counted
    /// again, and the file still reads to the same pixels.
    #[test]
    fn a_webp_loses_its_exif_and_its_flags() {
        let plain = encoded(ImageFormat::WebP, true);
        // The encoder's VP8L chunk, wrapped in a VP8X with EXIF and XMP after it.
        let vp8l = &plain[12..];
        let mut body = b"WEBP".to_vec();
        body.extend_from_slice(b"VP8X");
        body.extend_from_slice(&10u32.to_le_bytes());
        body.extend_from_slice(&[0x10 | 0x08 | 0x04, 0, 0, 0, 15, 0, 0, 15, 0, 0]);
        body.extend_from_slice(vp8l);
        body.extend_from_slice(b"EXIF");
        body.extend_from_slice(&8u32.to_le_bytes());
        body.extend_from_slice(b"MM\0*GPS!");
        body.extend_from_slice(b"XMP ");
        body.extend_from_slice(&5u32.to_le_bytes());
        body.extend_from_slice(b"<x/>\n\0");
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend(body);
        file.extend_from_slice(b"after the RIFF");
        let out = strip_metadata(&file).unwrap();
        assert!(out.removed);
        assert!(!has(&out.bytes, b"EXIF") && !has(&out.bytes, b"GPS"));
        assert!(!has(&out.bytes, b"XMP "));
        assert_eq!(out.bytes[20] & 0x0C, 0, "the flags cleared");
        assert_eq!(out.bytes[20] & 0x10, 0x10, "alpha kept");
        let riff = u32::from_le_bytes(out.bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff + 8, out.bytes.len());
        assert_eq!(pixels(&out.bytes), pixels(&plain));
    }

    /// A GIF's comment and a foreign application block go; the loop (NETSCAPE2.0) and the
    /// frames stay; nothing after the trailer.
    #[test]
    fn a_gif_loses_its_comment_not_its_loop() {
        let clean = encoded(ImageFormat::Gif, false);
        let trailer = clean.len() - 1;
        assert_eq!(clean[trailer], 0x3B);
        let mut file = clean[..trailer].to_vec();
        file.extend_from_slice(&[0x21, 0xFF, 11]);
        file.extend_from_slice(b"NETSCAPE2.0");
        file.extend_from_slice(&[3, 1, 0, 0, 0]);
        file.extend_from_slice(&[0x21, 0xFE, 4]);
        file.extend_from_slice(b"GPS!");
        file.push(0);
        file.extend_from_slice(&[0x21, 0xFF, 11]);
        file.extend_from_slice(b"XMP DataXMP");
        file.extend_from_slice(&[3, b'a', b'b', b'c', 0]);
        file.push(0x3B);
        file.extend_from_slice(b"tail");
        let out = strip_metadata(&file).unwrap();
        assert!(out.removed);
        assert!(has(&out.bytes, b"NETSCAPE2.0"));
        assert!(!has(&out.bytes, b"GPS!") && !has(&out.bytes, b"XMP Data"));
        assert_eq!(out.bytes.last(), Some(&0x3B));
        assert_eq!(pixels(&out.bytes), pixels(&clean));
    }

    /// An SVG's `<metadata>` and comments go; the drawing stays byte for byte.
    #[test]
    fn an_svg_loses_its_metadata_and_comments() {
        let file = concat!(
            "<!-- made by an editor at 55.7558,37.6173 -->",
            r#"<svg xmlns="http://www.w3.org/2000/svg"><metadata><rdf>author</rdf>"#,
            r#"<!-- inner --></metadata><rect width="4" height="4"/><!-- note --></svg>"#
        );
        let out = strip_metadata(file.as_bytes()).unwrap();
        assert!(out.removed);
        assert_eq!(
            std::str::from_utf8(&out.bytes).unwrap(),
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="4" height="4"/></svg>"#
        );
    }

    /// A GLB is left whole (its details are in its JSON; the screen says so).
    #[test]
    fn a_glb_is_left_as_it_is() {
        let mut glb = b"glTF".to_vec();
        glb.extend_from_slice(&[2, 0, 0, 0, 24, 0, 0, 0]);
        glb.extend_from_slice(b"{\"asset\":\"generator\"}  ");
        let out = strip_metadata(&glb).unwrap();
        assert!(!out.removed);
        assert_eq!(out.bytes, glb);
    }

    /// AC-A22, rule 1: a file cut short is refused, never written as it is.
    #[test]
    fn a_file_cut_short_is_refused() {
        for format in [
            ImageFormat::Jpeg,
            ImageFormat::Png,
            ImageFormat::Gif,
            ImageFormat::WebP,
        ] {
            let full = encoded(format, false);
            let cut = &full[..full.len() * 2 / 3];
            assert!(strip_metadata(cut).is_err(), "{format:?}");
        }
    }

    /// The architect's 200 random spoilings of each kind: an answer every time, never a panic;
    /// whatever comes out is no larger and of the same type.
    #[test]
    fn spoiled_files_never_panic() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut samples: Vec<Vec<u8>> = [
            ImageFormat::Jpeg,
            ImageFormat::Png,
            ImageFormat::Gif,
            ImageFormat::WebP,
        ]
        .into_iter()
        .map(|f| encoded(f, false))
        .collect();
        samples
            .push(br#"<svg xmlns="http://www.w3.org/2000/svg"><metadata/><rect/></svg>"#.to_vec());
        for sample in &samples {
            for _ in 0..200 {
                let mut spoiled = sample.clone();
                for _ in 0..1 + next() % 4 {
                    let at = (next() as usize) % spoiled.len();
                    match next() % 3 {
                        0 => spoiled[at] = next() as u8,
                        1 => spoiled.truncate(at.max(12)),
                        _ => spoiled.insert(at, next() as u8),
                    }
                }
                if let Ok(out) = strip_metadata(&spoiled) {
                    assert!(out.bytes.len() <= spoiled.len());
                    assert_eq!(image_type(&out.bytes), image_type(&spoiled));
                }
            }
        }
    }
}
