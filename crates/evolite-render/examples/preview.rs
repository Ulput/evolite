//! A developer's look at what the renderer makes of a file: an animated GIF of all frames and
//! a strip of up to eight of them side by side. Runs the decoders in this process, not in the
//! sandbox — for files you trust.
//!
//! `cargo run -p evolite-render --example preview -- picture.svg out.gif strip.png` (`-` for no
//! GIF)

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, gif, strip] = args.as_slice() else {
        eprintln!("usage: preview <picture> <out.gif> <strip.png>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(input).expect("read the picture");
    let art = evolite_render::render(&bytes, &evolite_render::Limits::default())
        .unwrap_or_else(|e| panic!("not drawn: {e}"));
    println!(
        "{} × {}, {} frames, {} ms in all",
        art.width,
        art.height,
        art.frames.len(),
        art.frames.iter().map(|f| f.delay_ms as u64).sum::<u64>()
    );
    let image = |f: &evolite_render::Frame| {
        RgbaImage::from_raw(art.width, art.height, f.rgba.clone()).expect("frame size")
    };

    // `-` for no GIF: encoding one is slower than drawing the frames.
    if gif != "-" {
        let mut encoder = GifEncoder::new(std::fs::File::create(gif).expect("create the GIF"));
        encoder.set_repeat(Repeat::Infinite).unwrap();
        for f in &art.frames {
            let delay = Delay::from_numer_denom_ms(f.delay_ms.max(20), 1);
            encoder
                .encode_frame(Frame::from_parts(image(f), 0, 0, delay))
                .expect("write a frame");
        }
    }

    let picked: Vec<_> = (0..art.frames.len().min(8))
        .map(|i| &art.frames[i * art.frames.len() / art.frames.len().min(8)])
        .collect();
    let mut sheet = RgbaImage::from_pixel(
        art.width * picked.len() as u32 + 4 * (picked.len() as u32 - 1),
        art.height,
        image::Rgba([255, 255, 255, 255]),
    );
    for (i, f) in picked.iter().enumerate() {
        image::imageops::overlay(
            &mut sheet,
            &image(f),
            (i as u32 * (art.width + 4)) as i64,
            0,
        );
    }
    sheet.save(strip).expect("write the strip");
}
