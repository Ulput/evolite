//! A model's picture as the «Workshop» writes it (FEAT-0038 UC-5, FEAT-0031 R-5): its type by
//! its first bytes, an SVG refused if it can run anything or reach out, packed with raw deflate
//! when that is smaller (checked by unpacking it again), and stored in the «image» field or,
//! larger, in up to four parts. No network.

/// The most a stored picture unpacks to: the «Collection»'s reader refuses more (a «bomb»), so
/// the «Workshop» never writes more (architect, 2026-11-20). One constant for both.
pub const MAX_UNPACKED: usize = 1 << 20;

/// One field of a document: Platform's `max_field_value_size`.
pub const FIELD: usize = 5_120;

/// The parts a picture may take (`part0…part3`).
pub const PARTS: usize = 4;

/// All the parts together, leaving room in the 20 480 bytes of one write for the name, the
/// description and the signature (FEAT-0031: «about 19 KB»).
pub const PARTS_TOTAL: usize = 19_000;

/// `imageType` of the profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ImageType {
    Svg = 0,
    AnimatedSvg = 1,
    Glb = 2,
    Jpeg = 3,
    Webp = 4,
    Png = 5,
    Gif = 6,
}

/// Why a picture is not written, said before the password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageRefusal {
    /// Not one of the profile's types, by its first bytes.
    Unknown,
    /// An SVG with a script, an event handler, a link out or a `<foreignObject>`: it could run
    /// something or reach a server when shown.
    Unsafe(&'static str),
    /// Larger than the parts hold even packed: its stored size.
    TooBig(usize),
    /// Unpacks to more than [`MAX_UNPACKED`].
    TooLargeUnpacked,
    /// Packing and unpacking did not give it back: not written.
    RoundTrip,
}

/// A picture ready to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed {
    pub image_type: ImageType,
    /// One: the «image» field; more: `part0…`.
    pub parts: Vec<Vec<u8>>,
    /// Packed with raw deflate (SVG and GLB, when smaller).
    pub deflated: bool,
}

impl Packed {
    pub fn stored(&self) -> usize {
        self.parts.iter().map(Vec::len).sum()
    }
}

/// The type of `bytes` by their first bytes.
pub fn image_type(bytes: &[u8]) -> Option<ImageType> {
    let starts = |p: &[u8]| bytes.starts_with(p);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some(ImageType::Png)
    } else if starts(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageType::Jpeg)
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some(ImageType::Gif)
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some(ImageType::Webp)
    } else if starts(b"glTF") {
        Some(ImageType::Glb)
    } else {
        let text = std::str::from_utf8(bytes).ok()?;
        let head = text
            .trim_start_matches('\u{feff}')
            .trim_start()
            .to_lowercase();
        let svg = head.starts_with("<svg")
            || (head.starts_with("<?xml") || head.starts_with("<!--")) && head.contains("<svg");
        if !svg {
            return None;
        }
        Some(if head.contains("<animate") || head.contains("<set ") {
            ImageType::AnimatedSvg
        } else {
            ImageType::Svg
        })
    }
}

/// What in an SVG could run something or reach out (architect, 2026-11-20; read as XML,
/// series r1): an element `script` or `foreignObject` under any prefix, an attribute `on…`
/// (with a prefix too: `ev:onload`), an `href` (or `xlink:href`) not inside the picture, and
/// `url(…)` out of it or `@import` in a style. Not XML at all, or with a DTD (entities), is
/// refused as well, and so is any processing instruction (`<?xml-stylesheet href=…?>` would
/// load a style; the `<?xml …?>` declaration is not a node, so it passes; series r2).
pub fn unsafe_svg(svg: &str) -> Option<&'static str> {
    let Ok(doc) = roxmltree::Document::parse(svg) else {
        return Some("not XML");
    };
    if doc.descendants().any(|n| n.is_pi()) {
        return Some("processing instruction");
    }
    for node in doc.descendants().filter(roxmltree::Node::is_element) {
        let name = node.tag_name().name();
        if name.eq_ignore_ascii_case("script") {
            return Some("script");
        }
        if name.eq_ignore_ascii_case("foreignObject") {
            return Some("foreignObject");
        }
        if name.eq_ignore_ascii_case("style")
            && node.children().filter_map(|c| c.text()).any(reaches_out)
        {
            return Some("link");
        }
        for a in node.attributes() {
            let attr = a.name();
            if attr.len() > 2 && attr[..2].eq_ignore_ascii_case("on") {
                return Some("event handler");
            }
            if attr.eq_ignore_ascii_case("href") && !a.value().trim_start().starts_with('#') {
                return Some("link");
            }
            if reaches_out(a.value()) {
                return Some("link");
            }
        }
    }
    None
}

/// A style (or an attribute) that reaches out: `@import`, or `url(…)` not inside the picture.
fn reaches_out(css: &str) -> bool {
    let css = css.to_lowercase();
    if css.contains("@import") {
        return true;
    }
    css.match_indices("url(").any(|(i, _)| {
        !css[i + 4..]
            .trim_start()
            .trim_start_matches(['"', '\''])
            .starts_with('#')
    })
}

/// `bytes` as the «Workshop» writes them, or why not.
pub fn pack(bytes: &[u8]) -> Result<Packed, ImageRefusal> {
    let image_type = image_type(bytes).ok_or(ImageRefusal::Unknown)?;
    if bytes.len() > MAX_UNPACKED {
        return Err(ImageRefusal::TooLargeUnpacked);
    }
    if matches!(image_type, ImageType::Svg | ImageType::AnimatedSvg) {
        let text = std::str::from_utf8(bytes).map_err(|_| ImageRefusal::Unknown)?;
        if let Some(why) = unsafe_svg(text) {
            return Err(ImageRefusal::Unsafe(why));
        }
    }
    // SVG and GLB pack well; the others are packed already.
    let packable = matches!(
        image_type,
        ImageType::Svg | ImageType::AnimatedSvg | ImageType::Glb
    );
    let (stored, deflated) = if packable {
        let packed = miniz_oxide::deflate::compress_to_vec(bytes, 9);
        // The round trip, as the reader will unpack it: within its limit, and the same bytes.
        let back = miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, MAX_UNPACKED)
            .map_err(|_| ImageRefusal::RoundTrip)?;
        if back != bytes {
            return Err(ImageRefusal::RoundTrip);
        }
        if packed.len() < bytes.len() {
            (packed, true)
        } else {
            (bytes.to_vec(), false)
        }
    } else {
        (bytes.to_vec(), false)
    };
    let parts = if stored.len() <= FIELD {
        vec![stored]
    } else if stored.len() <= PARTS_TOTAL {
        stored.chunks(FIELD).map(<[u8]>::to_vec).collect()
    } else {
        return Err(ImageRefusal::TooBig(stored.len()));
    };
    debug_assert!(parts.len() <= PARTS);
    Ok(Packed {
        image_type,
        parts,
        deflated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8" fill="#1c75bc"/><use href="#a"/></svg>"##;

    #[test]
    fn the_type_is_told_by_the_first_bytes() {
        assert_eq!(image_type(SVG.as_bytes()), Some(ImageType::Svg));
        assert_eq!(
            image_type(b"<?xml version=\"1.0\"?><svg><animate/></svg>"),
            Some(ImageType::AnimatedSvg)
        );
        assert_eq!(image_type(b"\x89PNG\r\n\x1a\nrest"), Some(ImageType::Png));
        assert_eq!(image_type(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageType::Jpeg));
        assert_eq!(image_type(b"GIF89a.."), Some(ImageType::Gif));
        assert_eq!(image_type(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageType::Webp));
        assert_eq!(image_type(b"glTF\x02\0\0\0"), Some(ImageType::Glb));
        assert_eq!(image_type(b"hello"), None);
        assert_eq!(pack(b"hello"), Err(ImageRefusal::Unknown));
    }

    /// The architect's list: script, `on*=`, links out, `<foreignObject>` — refused before the
    /// password. The xmlns declaration and a link inside the picture are fine.
    #[test]
    fn an_svg_that_could_run_or_reach_out_is_refused() {
        assert_eq!(unsafe_svg(SVG), None);
        let with = |inner: &str| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">{inner}</svg>"#
            )
        };
        for (inner, why) in [
            ("<script>alert(1)</script>", "script"),
            (r#"<rect onload="x()"/>"#, "event handler"),
            (r#"<rect ONCLICK = 'x()'/>"#, "event handler"),
            ("<foreignObject><div/></foreignObject>", "foreignObject"),
            (r#"<image href="https://x.test/a.png"/>"#, "link"),
            (r#"<image xlink:href="data:image/png;base64,AA"/>"#, "link"),
            (r#"<rect style="fill:url(https://x.test/p)"/>"#, "link"),
            ("<style>@import 'https://x.test/a.css';</style>", "link"),
        ] {
            assert_eq!(unsafe_svg(&with(inner)), Some(why), "{inner}");
            assert_eq!(
                pack(with(inner).as_bytes()),
                Err(ImageRefusal::Unsafe(why)),
                "{inner}"
            );
        }
        // Words that only look like a handler.
        assert_eq!(
            unsafe_svg(&with(r#"<text>one = two</text><rect fill="url(#g)"/>"#)),
            None
        );
    }

    /// Series r1: the checks are by XML names, so a prefix, a bare `@import` or a prefixed
    /// handler do not get through; nor does a DTD.
    #[test]
    fn an_svg_trick_does_not_get_through() {
        for (svg, why) in [
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="http://www.w3.org/2000/svg"><x:script>alert(1)</x:script></svg>"#,
                "script",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="http://www.w3.org/2000/svg"><x:foreignObject/></svg>"#,
                "foreignObject",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg"><style>@import "a.css";</style></svg>"#,
                "link",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:ev="http://www.w3.org/2001/xml-events"><rect ev:onload="x()"/></svg>"#,
                "event handler",
            ),
            (
                r#"<!DOCTYPE svg [<!ENTITY a "aaaa">]><svg xmlns="http://www.w3.org/2000/svg">&a;</svg>"#,
                "not XML",
            ),
            (
                r#"<?xml version="1.0"?><?xml-stylesheet href="https://example.com/a.css"?><svg xmlns="http://www.w3.org/2000/svg"/>"#,
                "processing instruction",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg"><?xml-stylesheet href="https://example.com/a.css"?></svg>"#,
                "processing instruction",
            ),
        ] {
            assert_eq!(unsafe_svg(svg), Some(why), "{svg}");
        }
        // The declaration alone is not a processing-instruction node: a plain picture passes.
        {
            let svg = r#"<?xml version="1.0" encoding="UTF-8"?><svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#;
            assert_eq!(unsafe_svg(svg), None, "{svg}");
        }
    }

    /// An SVG packs, comes back the same, and fits one field.
    #[test]
    fn an_svg_is_packed_when_smaller_and_comes_back() {
        let rects = "<rect width=\"1\" height=\"1\"/>".repeat(200) + "</svg>";
        let big = SVG.replace("</svg>", &rects);
        let p = pack(big.as_bytes()).unwrap();
        assert!(p.deflated);
        assert_eq!(p.parts.len(), 1);
        let back =
            miniz_oxide::inflate::decompress_to_vec_with_limit(&p.parts[0], MAX_UNPACKED).unwrap();
        assert_eq!(back, big.as_bytes());
    }

    /// Larger than one field: parts of [`FIELD`] at most; past them: refused with its size.
    #[test]
    fn a_larger_picture_takes_parts_up_to_the_limit() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(12_000, 7);
        let p = pack(&png).unwrap();
        assert!(!p.deflated);
        assert_eq!(p.parts.len(), 3);
        assert!(p.parts.iter().all(|x| x.len() <= FIELD));
        assert_eq!(p.parts.concat(), png);
        png.resize(PARTS_TOTAL + 1, 7);
        assert_eq!(pack(&png), Err(ImageRefusal::TooBig(PARTS_TOTAL + 1)));
    }

    #[test]
    fn more_than_the_readers_limit_is_refused() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(MAX_UNPACKED + 1, 0);
        assert_eq!(pack(&png), Err(ImageRefusal::TooLargeUnpacked));
    }
}
