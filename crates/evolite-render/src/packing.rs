//! Compressed pictures: raw deflate (how Jembe stores an NFT's SVG in its document,
//! `imageFormat` 1, told by the document since the stream has no signature of its own) and
//! gzip (`.svgz`, known by its first bytes). Unpacked inside the worker like any other
//! decoding, never past [`crate::Limits::max_input`]: a "bomb" of a few kilobytes that would
//! grow to gigabytes stops at the limit, without the memory for the rest.

use crate::frames::RenderError;

const GZIP: [u8; 3] = [0x1f, 0x8b, 8];

pub(crate) fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&GZIP)
}

/// A raw deflate stream, unpacked to at most `limit` bytes.
pub(crate) fn inflate(bytes: &[u8], limit: usize) -> Result<Vec<u8>, RenderError> {
    use miniz_oxide::inflate::{TINFLStatus, decompress_to_vec_with_limit};
    decompress_to_vec_with_limit(bytes, limit).map_err(|e| match e.status {
        TINFLStatus::HasMoreOutput => RenderError::Limit("unpacked size"),
        _ => RenderError::Invalid("cannot unpack"),
    })
}

/// A gzip member (RFC 1952), unpacked to at most `limit` bytes, its CRC and length checked.
pub(crate) fn gunzip(bytes: &[u8], limit: usize) -> Result<Vec<u8>, RenderError> {
    const FHCRC: u8 = 2;
    const FEXTRA: u8 = 4;
    const FNAME: u8 = 8;
    const FCOMMENT: u8 = 16;
    let bad = RenderError::Invalid("cannot unpack");
    if !is_gzip(bytes) || bytes.len() < 18 {
        return Err(bad);
    }
    let flags = bytes[3];
    let mut at = 10;
    if flags & FEXTRA != 0 {
        let len = bytes.get(at..at + 2).ok_or(bad.clone())?;
        at += 2 + usize::from(u16::from_le_bytes([len[0], len[1]]));
    }
    for flag in [FNAME, FCOMMENT] {
        if flags & flag != 0 {
            let end = bytes
                .get(at..)
                .and_then(|rest| rest.iter().position(|b| *b == 0));
            at += end.ok_or(bad.clone())? + 1;
        }
    }
    if flags & FHCRC != 0 {
        at += 2;
    }
    let body = bytes.get(at..bytes.len() - 8).ok_or(bad.clone())?;
    let out = inflate(body, limit)?;
    let trailer = &bytes[bytes.len() - 8..];
    let crc = u32::from_le_bytes(trailer[..4].try_into().expect("4 bytes"));
    let size = u32::from_le_bytes(trailer[4..].try_into().expect("4 bytes"));
    if crc32fast::hash(&out) != crc || size != out.len() as u32 {
        return Err(bad);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniz_oxide::deflate::compress_to_vec;

    /// A gzip member around raw deflate, as `gzip` writes it (with a file name).
    pub(crate) fn gzip(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, 8, 0, 0, 0, 0, 0, 0xff];
        out.extend(b"picture.svg\0");
        out.extend(compress_to_vec(data, 9));
        out.extend(crc32fast::hash(data).to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out
    }

    #[test]
    fn raw_deflate_and_gzip_come_back_whole() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".repeat(20);
        assert_eq!(inflate(&compress_to_vec(&svg, 9), 1 << 20).unwrap(), svg);
        assert_eq!(gunzip(&gzip(&svg), 1 << 20).unwrap(), svg);
    }

    #[test]
    fn a_bomb_stops_at_the_limit() {
        // 64 MB of zeros pack into some 60 KB; the limit is a megabyte.
        let bomb = compress_to_vec(&vec![0u8; 64 << 20], 9);
        assert!(bomb.len() < 100_000);
        assert_eq!(
            inflate(&bomb, 1 << 20),
            Err(RenderError::Limit("unpacked size"))
        );
    }

    #[test]
    fn damaged_streams_are_refused() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        let mut wrong_crc = gzip(svg);
        let n = wrong_crc.len();
        wrong_crc[n - 8] ^= 1;
        let cut = &gzip(svg)[..20];
        for bad in [
            &wrong_crc[..],
            cut,
            &[0x1f, 0x8b, 8][..],
            b"not deflate at all",
        ] {
            assert!(gunzip(bad, 1 << 20).is_err());
        }
        assert_eq!(
            inflate(&[0xff; 8], 1 << 20),
            Err(RenderError::Invalid("cannot unpack"))
        );
    }
}
