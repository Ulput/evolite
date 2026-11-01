//! Draws the app icon files from `assets/app/evolite-app.svg` (the brandbook's "App icon"):
//! `evolite-app.png` (256 px, the window icon) and `evolite-app.ico` (16–256 px, the
//! executable's icon on Windows). Run after changing the SVG:
//!
//!   cargo run -p evolite-ui --example app_icon

use std::path::Path;

/// The sizes Windows picks from: title bar, taskbar, Explorer, at 100–200 %.
const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app");
    let svg = std::fs::read(dir.join("evolite-app.svg")).expect("the icon's SVG");
    let tree =
        resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default()).expect("a valid SVG");
    std::fs::write(dir.join("evolite-app.png"), png(&tree, 256)).expect("the PNG written");
    let images: Vec<(u32, Vec<u8>)> = ICO_SIZES.iter().map(|&s| (s, png(&tree, s))).collect();
    std::fs::write(dir.join("evolite-app.ico"), ico(&images)).expect("the ICO written");
    println!("assets/app: evolite-app.png, evolite-app.ico");
}

/// `tree` drawn `size` × `size`, as a PNG.
fn png(tree: &resvg::usvg::Tree, size: u32) -> Vec<u8> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("a non-empty size");
    let scale = size as f32 / tree.size().width();
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().expect("a PNG")
}

/// An ICO holding each image as PNG (Windows Vista and later read these).
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in images {
        // 256 is written as 0.
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        out.extend_from_slice(data);
    }
    out
}
