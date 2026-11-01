//! The format by the first bytes. A name or a MIME type from outside says nothing.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
    /// Binary glTF, version 2: a 3D model.
    Glb,
}

/// `None`: not on the white list (glTF as JSON with outside files included). A gzip file is
/// unpacked before this is asked ([`crate::render`]).
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"glTF\x02\0\0\0") {
        return Some(Format::Glb);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Format::Png);
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(Format::Jpeg);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(Format::Gif);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(Format::Webp);
    }
    // SVG is text: an optional BOM, an XML declaration or comments, then an <svg> element.
    let head = &bytes[..bytes.len().min(4096)];
    let head = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head);
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start();
    if text.starts_with('<') && text.contains("<svg") {
        return Some(Format::Svg);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_the_bytes_not_the_name() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some(Format::Png));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some(Format::Jpeg));
        assert_eq!(sniff(b"GIF89a...."), Some(Format::Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Format::Webp));
        assert_eq!(
            sniff(
                b"\xef\xbb\xbf<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>"
            ),
            Some(Format::Svg)
        );
        assert_eq!(sniff(b"glTF\x02\0\0\0...."), Some(Format::Glb));
        // Compressed SVG (unpacked before), glTF 1, glTF as JSON, HTML, nothing.
        assert_eq!(sniff(&[0x1f, 0x8b, 8, 0]), None);
        assert_eq!(sniff(b"glTF\x01\0\0\0"), None);
        assert_eq!(sniff(br#"{"asset":{"version":"2.0"}}"#), None);
        assert_eq!(sniff(b"<html><body></body></html>"), None);
        assert_eq!(sniff(b""), None);
    }
}
