//! SVG through usvg/resvg with every way out closed.
//!
//! usvg's defaults are for trusted files: `<image href="…">` is read from disk (on Windows a
//! `\\host\share\…` path even makes the system log in to that host and hand it the user's
//! NTLM hash), a `data:` SVG is parsed as one more document, DTD entities are allowed, and an
//! embedded raster is decoded at whatever size it claims. Here: no paths at all, only `data:`
//! PNG/JPEG/GIF/WebP within the size limits, no DTD, a cap on XML nodes, and the picture
//! drawn no larger than `max_side`.

use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, ImageHrefResolver, ImageKind, fontdb};

use crate::frames::{Art, Frame, Limits, RenderError, fit};
use crate::sniff::{Format, sniff};

/// System fonts, read once per process (before any untrusted file is seen).
fn fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            generic_families(&mut db);
            Arc::new(db)
        })
        .clone()
}

/// The fonts behind `sans-serif`, `serif` and `monospace`: the first of each list this system
/// has. fontdb's own choice is Windows' (Arial, Times New Roman, Courier New); a Linux desktop
/// without them drew no text at all for a generic family.
const SANS: [&str; 6] = [
    "Arial",
    "Helvetica",
    "Liberation Sans",
    "DejaVu Sans",
    "Noto Sans",
    "Cantarell",
];
const SERIF: [&str; 5] = [
    "Times New Roman",
    "Times",
    "Liberation Serif",
    "DejaVu Serif",
    "Noto Serif",
];
const MONO: [&str; 5] = [
    "Courier New",
    "Menlo",
    "Liberation Mono",
    "DejaVu Sans Mono",
    "Noto Sans Mono",
];

fn generic_families(db: &mut fontdb::Database) {
    let first = |db: &fontdb::Database, names: &[&str]| {
        names
            .iter()
            .find(|name| {
                db.faces()
                    .any(|face| face.families.iter().any(|(family, _)| family == *name))
            })
            .map(|name| name.to_string())
    };
    if let Some(name) = first(db, &SANS) {
        db.set_sans_serif_family(name);
    }
    if let Some(name) = first(db, &SERIF) {
        db.set_serif_family(name);
    }
    if let Some(name) = first(db, &MONO) {
        db.set_monospace_family(name);
    }
}

/// Finds the system fonts now, before the first untrusted byte, and says which folders they
/// are in: a font's data may be read from its file when a `<text>` is drawn, so a sandbox
/// that closes the file system must leave these folders readable.
pub(crate) fn warm_up() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = fonts()
        .faces()
        .filter_map(|face| match &face.source {
            fontdb::Source::File(path) => path.parent().map(ToOwned::to_owned),
            #[allow(
                unreachable_patterns,
                reason = "fontdb has other sources with some of its features, none here"
            )]
            _ => None,
        })
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Wraps the bytes of an embedded raster in its usvg kind.
type Wrap = fn(Arc<Vec<u8>>) -> ImageKind;

/// An embedded raster, if it is one of ours and no larger than the limits; checked from its
/// header, before resvg decodes it.
fn embedded(data: Arc<Vec<u8>>, limits: Limits) -> Option<ImageKind> {
    if data.len() > limits.max_input {
        return None;
    }
    let (format, kind): (_, Wrap) = match sniff(&data)? {
        Format::Png => (image::ImageFormat::Png, ImageKind::PNG),
        Format::Jpeg => (image::ImageFormat::Jpeg, ImageKind::JPEG),
        Format::Gif => (image::ImageFormat::Gif, ImageKind::GIF),
        Format::Webp => (image::ImageFormat::WebP, ImageKind::WEBP),
        // A picture inside a picture: no.
        Format::Svg | Format::Glb => return None,
    };
    let (w, h) = image::ImageReader::with_format(Cursor::new(data.as_slice()), format)
        .into_dimensions()
        .ok()?;
    (w <= limits.max_source_side && h <= limits.max_source_side).then(|| kind(data))
}

fn options(limits: Limits) -> usvg::Options<'static> {
    usvg::Options {
        resources_dir: None,
        fontdb: fonts(),
        image_href_resolver: ImageHrefResolver {
            // The format by the bytes, whatever the MIME type says.
            resolve_data: Box::new(move |_mime, data, _| embedded(data, limits)),
            // A file path or a URL: never followed.
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    }
}

/// The XML, with no DTD and no more than `max_svg_nodes` nodes.
pub(crate) fn parse<'a>(
    text: &'a str,
    limits: &Limits,
) -> Result<roxmltree::Document<'a>, RenderError> {
    roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: limits.max_svg_nodes,
        },
    )
    .map_err(|e| match e {
        roxmltree::Error::NodesLimitReached => RenderError::Limit("SVG nodes"),
        _ => RenderError::Invalid("not well-formed XML"),
    })
}

/// The picture usvg makes of the XML, with every way out closed ([`options`]).
pub(crate) fn tree(xml: &roxmltree::Document, limits: &Limits) -> Result<usvg::Tree, RenderError> {
    usvg::Tree::from_xmltree(xml, &options(*limits)).map_err(|_| RenderError::Invalid("not an SVG"))
}

/// Draws `tree`, whose own size is `base`, into `width × height` pixels: premultiplied RGBA.
/// Frames of an animation share one `base`, so a frame whose size changes is still drawn on
/// the same canvas.
pub(crate) fn raster(
    tree: &usvg::Tree,
    base: usvg::Size,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, RenderError> {
    let mut pixmap = Pixmap::new(width, height).ok_or(RenderError::Invalid("size"))?;
    resvg::render(
        tree,
        Transform::from_scale(width as f32 / base.width(), height as f32 / base.height()),
        &mut pixmap.as_mut(),
    );
    // tiny-skia keeps premultiplied alpha, as frames do: its pixels go as they are.
    Ok(pixmap.take())
}

/// The canvas a picture of size `base` gets within `max` pixels a side; with `want`, its
/// longer side is exactly that (up to `max`), larger or smaller than the picture's own size.
pub(crate) fn canvas(base: usvg::Size, max: u32, want: Option<u32>) -> (u32, u32) {
    let (w, h) = (base.width() as f64, base.height() as f64);
    match want {
        Some(side) if w > 0.0 && h > 0.0 => {
            let scale = side.min(max).max(1) as f64 / w.max(h);
            (
                ((w * scale).round() as u32).clamp(1, max),
                ((h * scale).round() as u32).clamp(1, max),
            )
        }
        _ => fit(w.ceil() as u32, h.ceil() as u32, max),
    }
}

pub(crate) fn render(bytes: &[u8], limits: &Limits) -> Result<Art, RenderError> {
    let text = std::str::from_utf8(bytes).map_err(|_| RenderError::Invalid("not UTF-8"))?;
    let xml = parse(text, limits)?;
    // SMIL animations become frames; anything else (or nothing to animate) is a still.
    if let Some(art) = crate::anim::frames(text, &xml, limits)? {
        return Ok(art);
    }
    let tree = tree(&xml, limits)?;
    let base = tree.size();
    let (width, height) = canvas(base, limits.max_side, limits.svg_size);
    let rgba = raster(&tree, base, width, height)?;
    Ok(Art {
        width,
        height,
        frames: vec![Frame { rgba, delay_ms: 0 }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba, RgbaImage};

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, Rgba([0, 200, 0, 255]));
        let mut out = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    fn base64(bytes: &[u8]) -> String {
        const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().fold(0u32, |n, b| (n << 8) | *b as u32) << (8 * (3 - chunk.len()));
            for i in 0..4 {
                out.push(if i <= chunk.len() {
                    ABC[(n >> (18 - 6 * i)) as usize & 63] as char
                } else {
                    '='
                });
            }
        }
        out
    }

    fn svg(body: &str) -> Vec<u8> {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">{body}</svg>"#
        )
        .into_bytes()
    }

    /// The pixel at (5, 5): the middle of a 10 × 10 picture.
    fn middle(art: &Art) -> [u8; 4] {
        let i = (5 * art.width as usize + 5) * 4;
        art.frames[0].rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn draws_a_plain_svg() {
        let art = render(
            &svg(r#"<rect width="10" height="10" fill="red"/>"#),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!((art.width, art.height), (10, 10));
        assert_eq!(middle(&art), [255, 0, 0, 255]);
    }

    #[test]
    fn an_embedded_png_is_shown() {
        let href = format!("data:image/png;base64,{}", base64(&png(10, 10)));
        let art = render(
            &svg(&format!(r#"<image width="10" height="10" href="{href}"/>"#)),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(middle(&art), [0, 200, 0, 255]);
    }

    #[test]
    fn a_local_file_is_never_read() {
        let path = std::env::temp_dir().join(format!("evolite-render-{}.png", std::process::id()));
        std::fs::write(&path, png(10, 10)).unwrap();
        for href in [
            path.display().to_string(),
            format!("file:///{}", path.display()),
            // On Windows this would log in to the host with the user's NTLM hash.
            r"\\127.0.0.1\share\picture.png".to_string(),
        ] {
            let art = render(
                &svg(&format!(
                    r#"<image width="10" height="10" xlink:href="{href}"/>"#
                )),
                &Limits::default(),
            )
            .unwrap();
            assert_eq!(middle(&art), [0, 0, 0, 0], "{href}");
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_embedded_raster_claiming_a_huge_size_is_skipped() {
        let limits = Limits {
            max_source_side: 8,
            ..Limits::default()
        };
        let href = format!("data:image/png;base64,{}", base64(&png(10, 10)));
        let art = render(
            &svg(&format!(r#"<image width="10" height="10" href="{href}"/>"#)),
            &limits,
        )
        .unwrap();
        assert_eq!(middle(&art), [0, 0, 0, 0]);
    }

    #[test]
    fn a_nested_svg_is_not_followed() {
        let inner = svg(r#"<rect width="10" height="10" fill="red"/>"#);
        let href = format!("data:image/svg+xml;base64,{}", base64(&inner));
        let art = render(
            &svg(&format!(r#"<image width="10" height="10" href="{href}"/>"#)),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(middle(&art), [0, 0, 0, 0]);
    }

    #[test]
    fn entities_are_refused() {
        let laughs = br#"<?xml version="1.0"?>
<!DOCTYPE svg [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">]>
<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>&b;</text></svg>"#;
        assert_eq!(
            render(laughs, &Limits::default()),
            Err(RenderError::Invalid("not well-formed XML"))
        );
    }

    #[test]
    fn too_many_nodes_are_refused() {
        let body = "<g/>".repeat(100);
        let limits = Limits {
            max_svg_nodes: 50,
            ..Limits::default()
        };
        assert_eq!(
            render(&svg(&body), &limits),
            Err(RenderError::Limit("SVG nodes"))
        );
    }

    #[test]
    fn a_huge_canvas_is_drawn_small() {
        let bytes = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1000000" height="500000"><rect width="1000000" height="500000" fill="blue"/></svg>"#;
        let art = render(bytes, &Limits::default()).unwrap();
        assert_eq!((art.width, art.height), (2048, 1024));
    }
}
