//! The worker as a real process: pictures go through, and a worker that hangs, crashes, eats
//! memory or lies is stopped without taking the caller down.

use std::io::Cursor;
use std::time::{Duration, Instant};

use evolite_render::{RenderError, Renderer, SandboxRefusal, Settings};
use image::{ImageFormat, Rgba, RgbaImage};

const WORKER: &str = env!("CARGO_BIN_EXE_evolite-render-worker");

fn renderer(mode: Option<&str>) -> Renderer {
    let mut settings = Settings::new(
        WORKER.into(),
        mode.map(|m| vec![m.into()]).unwrap_or_default(),
    );
    // A loaded macOS runner reads its fonts slowly; the deadline counts them.
    settings.timeout = Duration::from_secs(if cfg!(target_os = "macos") { 10 } else { 3 });
    settings.memory = 256 * 1024 * 1024;
    Renderer::new(settings)
}

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255]))
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

#[test]
fn pictures_go_through_the_worker() {
    let mut r = renderer(None);
    for _ in 0..3 {
        let art = r.render(&png()).unwrap();
        assert_eq!((art.width, art.height), (3, 2));
        assert_eq!(&art.unpack(0).unwrap()[..4], &[10, 20, 30, 255]);
    }
    assert_eq!(r.render(b"<html></html>"), Err(RenderError::Unsupported));
    // The worker is still there after a refusal.
    assert!(r.render(&png()).is_ok());
}

#[test]
fn a_hanging_worker_is_stopped_at_the_deadline() {
    let mut r = renderer(Some("--test-hang"));
    let started = Instant::now();
    assert_eq!(r.render(&png()), Err(RenderError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn a_crashing_worker_is_reported() {
    let mut r = renderer(Some("--test-crash"));
    assert_eq!(r.render(&png()), Err(RenderError::Crashed));
    // And a new one is started for the next picture.
    assert_eq!(r.render(&png()), Err(RenderError::Crashed));
}

#[test]
fn an_answer_over_the_limits_is_refused() {
    let mut r = renderer(Some("--test-lie"));
    assert_eq!(r.render(&png()), Err(RenderError::Crashed));
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn the_memory_cap_ends_a_greedy_worker() {
    // A deadline far past what the cap takes even on a loaded machine: the check is how the
    // worker ended, not how fast. Without the cap it would wait here and answer `Timeout`.
    let mut settings = Settings::new(WORKER.into(), vec!["--test-allocate".into()]);
    settings.timeout = Duration::from_secs(20);
    settings.memory = 256 * 1024 * 1024;
    let mut r = Renderer::new(settings);
    let started = Instant::now();
    assert_eq!(
        r.render(&png()),
        Err(RenderError::Crashed),
        "ended by the cap, not the deadline"
    );
    // About 0.3 s quietly, 2.5 s more with Windows Error Reporting holding the dead process;
    // the bound only catches a worker that hung on to the very end.
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
}

/// A cap the worker cannot set: it does not serve, and the wallet is told so.
#[cfg(target_os = "macos")]
#[test]
fn a_memory_cap_that_cannot_be_set_refuses_the_picture() {
    let mut settings = Settings::new(WORKER.into(), Vec::new());
    settings.memory = usize::MAX;
    let mut r = Renderer::new(settings);
    assert_eq!(
        r.render(&png()),
        Err(RenderError::Refused(SandboxRefusal::NoMemoryCap))
    );
}

/// A worker that will not serve is told apart from one that crashed, by its exit code.
#[test]
fn a_refusal_is_not_a_crash() {
    let mut r = renderer(Some("--test-refuse"));
    assert_eq!(
        r.render(&png()),
        Err(RenderError::Refused(SandboxRefusal::NoFileIsolation))
    );
    for (code, refusal) in [
        (2, SandboxRefusal::NoSandbox),
        (4, SandboxRefusal::NoFileIsolation),
        (5, SandboxRefusal::NoMemoryCap),
    ] {
        assert_eq!(SandboxRefusal::from_exit_code(code), Some(refusal));
    }
    for code in [0, 1, 3, 134, 137] {
        assert_eq!(SandboxRefusal::from_exit_code(code), None);
    }
}

/// A worker the system will not start is a refusal: the picture is not drawn some other way.
#[test]
fn a_worker_that_cannot_start_refuses_the_picture() {
    let mut r = Renderer::new(Settings::new("does-not-exist".into(), Vec::new()));
    assert_eq!(r.start(), Err(RenderError::Crashed));
    assert_eq!(r.render(&png()), Err(RenderError::Crashed));
}

#[test]
fn input_over_the_limit_never_reaches_the_worker() {
    let mut settings = Settings::new("does-not-exist".into(), Vec::new());
    settings.limits.max_input = 10;
    let mut r = Renderer::new(settings);
    assert_eq!(r.render(&[0; 11]), Err(RenderError::TooLarge));
}

#[test]
fn pictures_are_drawn_at_the_size_they_are_shown() {
    let mut r = renderer(None);
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5"><rect width="10" height="5" fill="red"/></svg>"#;
    // A vector picture grows to the size it is shown at, and shrinks.
    let art = r.render_at(svg, 64).unwrap();
    assert_eq!((art.width, art.height), (64, 32));
    assert_eq!(&art.unpack(0).unwrap()[..4], &[255, 0, 0, 255]);
    let art = r.render_at(svg, 4).unwrap();
    assert_eq!((art.width, art.height), (4, 2));
    // A raster only shrinks: growing it would only blur it.
    assert_eq!(
        r.render_at(&png(), 64).map(|a| (a.width, a.height)),
        Ok((3, 2))
    );
    assert_eq!(
        r.render_at(&png(), 2).map(|a| (a.width, a.height)),
        Ok((2, 1))
    );
}

#[test]
fn a_worker_started_ahead_draws_the_first_picture() {
    let mut r = renderer(None);
    r.start().unwrap();
    // A second start keeps the same worker.
    r.start().unwrap();
    assert!(r.render(&png()).is_ok());
}

/// Compressed SVG through the worker: `.svgz` by its signature, raw deflate (Jembe's
/// `imageFormat` 1) because the request says so, and a bomb stopped at the input limit.
#[test]
fn compressed_svg_is_unpacked_in_the_worker() {
    use evolite_render::{Packing, Pass};
    use miniz_oxide::deflate::compress_to_vec;
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="lime"/></svg>"#;
    let deflated = compress_to_vec(svg, 9);
    let mut svgz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    svgz.extend(&deflated);
    svgz.extend(crc32fast::hash(svg).to_le_bytes());
    svgz.extend((svg.len() as u32).to_le_bytes());

    let mut r = renderer(None);
    for (bytes, packing) in [(&svgz, Packing::Plain), (&deflated, Packing::RawDeflate)] {
        let art = r.draw(bytes, packing, 0, Pass::Whole).unwrap();
        assert_eq!((art.width, art.height), (4, 2));
        assert_eq!(&art.unpack(0).unwrap()[..4], &[0, 255, 0, 255]);
    }
    // Raw deflate is never guessed: without the word it is not a picture.
    assert_eq!(r.render(&deflated), Err(RenderError::Unsupported));
    // 64 MB of zeros in some 60 KB: refused at the 16 MB limit, the worker stays.
    let bomb = compress_to_vec(&vec![0u8; 64 << 20], 9);
    assert_eq!(
        r.draw(&bomb, Packing::RawDeflate, 0, Pass::Whole),
        Err(RenderError::Limit("unpacked size"))
    );
    assert!(r.render(svg).is_ok());
}

/// A 3D model through the sandboxed worker at the largest animation size: its frames and the
/// frames in the making fit the memory cap (in this process there is none to catch it).
#[test]
fn a_model_turns_inside_the_memory_cap() {
    let ring = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../evolite-core/assets/nft/diamond-ring-3d.glb"),
    )
    .unwrap();
    let mut settings = Settings::new(WORKER.into(), Vec::new());
    settings.timeout = Duration::from_secs(10);
    let mut r = Renderer::new(settings);
    let art = r.render_at(&ring, 512).unwrap();
    assert_eq!((art.width, art.height), (512, 512));
    assert!(art.frames.len() > 1);
}
