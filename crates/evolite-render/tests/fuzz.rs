//! Mutation fuzzing on stable Rust: real pictures (the Jembe samples, SMIL and CSS animations,
//! rasters) are broken in many small ways, and every result must be a picture within the
//! limits or a refusal: never a panic, a hang or memory past the limits. The same goes for the
//! wire, which reads what a compromised worker might send.
//!
//! A quick pass runs with the tests; a long one with `EVOLITE_FUZZ_ITERS`, for example
//! `EVOLITE_FUZZ_ITERS=20000 cargo test --release -p evolite-render --test fuzz -- --nocapture`.
//! A failing case is saved to the temp folder and its seed printed.

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use evolite_render::{Limits, render, wire};

/// xorshift64*: the same cases on every run for a seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Pieces that steer mutations into the parsers' corners.
const WORDS: &[&str] = &[
    "<",
    ">",
    "\"",
    "'",
    ";",
    ":",
    "{",
    "}",
    "(",
    ")",
    ",",
    "-",
    ".",
    "e",
    "%",
    "#",
    "&amp;",
    "0",
    "-1",
    "1e309",
    "-1e-309",
    "NaN",
    "inf",
    "999999999",
    "0.000001",
    "steps(0)",
    "steps(99999, jump-none)",
    "cubic-bezier(9,-9,9,9)",
    "infinite",
    "alternate-reverse",
    "@keyframes x{",
    "from{",
    "to{",
    "50%{",
    "animation:x 0s",
    "animation:x 1s steps(3) 2",
    "transform:rotate(9e9deg)",
    "transform-origin:",
    "transform-box:fill-box",
    "<animate attributeName=\"x\" values=\";;\" dur=\"1s\" repeatCount=\"indefinite\"/>",
    "<animateTransform attributeName=\"transform\" type=\"scale\" values=\"0;1e30\" dur=\"0.001s\"/>",
    "<set attributeName=\"fill\" to=\"url(#a)\" begin=\"-5s\"/>",
    "keyTimes=\"0;2;1\"",
    "calcMode=\"discrete\"",
    "dur=\"0s\"",
    "dur=\"indefinite\"",
    "begin=\"9999s\"",
    "repeatDur=\"0.1s\"",
    "href=\"#self\"",
    "xlink:href=\"file:///c:/x\"",
    "<image href=\"data:image/png;base64,AAAA\"/>",
    "<use href=\"#a\"/>",
    "<g id=\"a\">",
    "</g>",
    "<style>",
    "</style>",
    "<!DOCTYPE x>",
    "<?xml version=\"1.0\"?>",
];

fn mutate(rng: &mut Rng, input: &[u8]) -> Vec<u8> {
    let mut out = input.to_vec();
    for _ in 0..1 + rng.below(4) {
        let at = rng.below(out.len() + 1);
        match rng.below(6) {
            0 if !out.is_empty() => {
                let i = rng.below(out.len());
                out[i] ^= 1 << rng.below(8);
            }
            1 if !out.is_empty() => {
                let end = (at + rng.below(64)).min(out.len());
                out.drain(at.min(end)..end);
            }
            2 if !out.is_empty() => {
                let start = rng.below(out.len());
                let end = (start + rng.below(256)).min(out.len());
                let piece = out[start..end].to_vec();
                out.splice(at..at, piece);
            }
            3 => {
                let word = WORDS[rng.below(WORDS.len())].as_bytes();
                out.splice(at..at, word.iter().copied());
            }
            4 if !out.is_empty() => {
                let i = rng.below(out.len());
                out[i] = rng.next() as u8;
            }
            _ => {
                // A number somewhere made huge or negative.
                if let Some(i) = (at..out.len()).find(|i| out[*i].is_ascii_digit()) {
                    let word = ["9999999", "-0", "1e38", "-1"][rng.below(4)].as_bytes();
                    out.splice(i..i + 1, word.iter().copied());
                }
            }
        }
    }
    out
}

fn assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../evolite-core/assets/nft")
}

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(6, 4, image::Rgba([10, 200, 30, 128]))
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn gif() -> Vec<u8> {
    use image::codecs::gif::GifEncoder;
    let mut out = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut out);
        for shade in [0u8, 120, 240] {
            let frame = image::Frame::new(image::RgbaImage::from_pixel(
                5,
                5,
                image::Rgba([shade, 0, 0, 255]),
            ));
            encoder.encode_frame(frame).unwrap();
        }
    }
    out
}

fn seeds() -> Vec<(String, Vec<u8>)> {
    let mut seeds: Vec<(String, Vec<u8>)> = std::fs::read_dir(assets())
        .expect("the demo pictures")
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    seeds.sort();
    seeds.push((
        "smil.svg".into(),
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect id="r" width="5" height="5" fill="red"><animate attributeName="x" values="0;10;5" keyTimes="0;0.5;1" dur="1s" repeatCount="indefinite"/><animateTransform attributeName="transform" type="rotate" from="0 10 10" to="360 10 10" dur="2s" repeatCount="indefinite"/><set attributeName="fill" to="blue" begin="0.5s"/></rect></svg>"##.to_vec(),
    ));
    seeds.push((
        "css.svg".into(),
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><style>#a{animation:spin 1s cubic-bezier(.2,.8,.2,1) infinite alternate;transform-origin:50% 50%;transform-box:fill-box}@keyframes spin{0%{transform:rotate(0) scale(1);opacity:1}50%{fill:#0f0}100%{transform:rotate(90deg) translate(2px,3px);opacity:.2}}</style><rect id="a" x="5" y="5" width="10" height="10"/></svg>"##.to_vec(),
    ));
    seeds.push(("still.png".into(), png()));
    seeds.push(("anim.gif".into(), gif()));
    // Compressed: the unpacking is fuzzed too, broken streams and all. `.deflate` seeds are
    // raw deflate (Jembe's documents), drawn with `raw_deflate`.
    let svg = std::fs::read(assets().join("coffin-dance-animated.svg")).unwrap();
    let deflated = miniz_oxide::deflate::compress_to_vec(&svg, 9);
    let mut svgz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    svgz.extend(&deflated);
    svgz.extend(crc32fast::hash(&svg).to_le_bytes());
    svgz.extend((svg.len() as u32).to_le_bytes());
    seeds.push(("coffin-dance.svgz".into(), svgz));
    seeds.push(("coffin-dance.deflate".into(), deflated));
    // The 3D ring, as is and deflated (Jembe's proposed `imageFormat` 2).
    let ring = std::fs::read(assets().join("diamond-ring-3d.glb")).unwrap();
    seeds.push((
        "diamond-ring.deflate".into(),
        miniz_oxide::deflate::compress_to_vec(&ring, 9),
    ));
    seeds.push(("diamond-ring.glb".into(), ring));
    seeds
}

/// A GLB's JSON and BIN chunks, and a GLB put back together from them with lengths that add
/// up: mutations then reach the model, not only the container's header.
fn glb_parts(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    let json_len = u32_at(12);
    let json = bytes[20..20 + json_len].to_vec();
    let bin_at = 20 + json_len;
    let bin = bytes[bin_at + 8..bin_at + 8 + u32_at(bin_at)].to_vec();
    (json, bin)
}

fn glb(json: &[u8], bin: &[u8]) -> Vec<u8> {
    let pad = |mut v: Vec<u8>, with: u8| {
        while !v.len().is_multiple_of(4) {
            v.push(with);
        }
        v
    };
    let (json, bin) = (pad(json.to_vec(), b' '), pad(bin.to_vec(), 0));
    let mut out = b"glTF".to_vec();
    out.extend(2u32.to_le_bytes());
    out.extend(((28 + json.len() + bin.len()) as u32).to_le_bytes());
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(0x4E4F_534Au32.to_le_bytes());
    out.extend(json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(0x004E_4942u32.to_le_bytes());
    out.extend(bin);
    out
}

/// Pieces of glTF JSON that steer mutations into the model's corners.
const GLTF_WORDS: &[&str] = &[
    r#""count":4294967295"#,
    r#""byteOffset":4294967290"#,
    r#""byteStride":3"#,
    r#""componentType":5126"#,
    r#""componentType":5125"#,
    r#""type":"MAT4""#,
    r#""mode":0"#,
    r#""children":[0]"#,
    r#""matrix":[1e38,0,0,0,0,1e38,0,0,0,0,1e38,0,0,0,0,1]"#,
    r#""scale":[0,0,0]"#,
    r#""rotation":[0,0,0,0]"#,
    r#""uri":"file:///etc/passwd""#,
    r#""sparse":{"count":1}"#,
    r#""extensionsRequired":["KHR_draco_mesh_compression"]"#,
    r#""material":99"#,
    r#""indices":99"#,
    "NaN",
    "-1",
    "1e309",
    "[",
    "]",
    "{",
    "}",
    ",",
];

#[test]
fn broken_models_are_drawn_within_the_limits_or_refused() {
    let ring = std::fs::read(assets().join("diamond-ring-3d.glb")).unwrap();
    let (json, bin) = glb_parts(&ring);
    let limits = limits();
    let mut rng = Rng(0xA076_1D64_78BD_642F);
    for _ in 0..iterations() * 4 {
        let mut json = json.clone();
        let mut bin = bin.clone();
        for _ in 0..1 + rng.below(3) {
            if rng.below(3) == 0 && !bin.is_empty() {
                let i = rng.below(bin.len());
                bin[i] = rng.next() as u8;
            } else {
                let at = rng.below(json.len() + 1);
                if rng.below(2) == 0 {
                    let word = GLTF_WORDS[rng.below(GLTF_WORDS.len())].as_bytes();
                    json.splice(at..at, word.iter().copied());
                } else {
                    let end = (at + rng.below(16)).min(json.len());
                    json.drain(at..end);
                }
            }
        }
        let case = glb(&json, &bin);
        let started = std::time::Instant::now();
        // In this process, as the worker would: never a panic, the limits held.
        if let Ok(art) = render(&case, &limits) {
            assert!(art.check(&limits).is_ok());
        }
        assert!(started.elapsed().as_secs() < 3, "slow model");
    }
}

/// Small and quick: every case runs the whole way, in a fraction of a second.
fn limits() -> Limits {
    Limits {
        max_side: 48,
        max_animation_side: 48,
        svg_size: Some(48),
        max_frames: 24,
        max_render_ms: 40,
        threads: 1,
        ..Limits::default()
    }
}

fn iterations() -> usize {
    std::env::var("EVOLITE_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40)
}

/// Where the last panic came from: `file:line`. One for all threads: frames are drawn on
/// several, and a panic there reaches the caller.
static PANIC_AT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// A panic in our own code is a failure. One inside a library (usvg, tiny-skia) is caught by
/// the worker and the picture refused, like any broken file: it is listed, not failed.
fn ours(at: &str) -> bool {
    at.contains("evolite-render")
}

#[test]
fn broken_pictures_are_drawn_within_the_limits_or_refused() {
    std::panic::set_hook(Box::new(|info| {
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        *PANIC_AT.lock().unwrap_or_else(|e| e.into_inner()) = at;
    }));
    let mut contained: std::collections::BTreeMap<String, usize> = Default::default();
    let mut failures = Vec::new();
    for (name, seed) in seeds() {
        let limits = Limits {
            raw_deflate: name.ends_with(".deflate"),
            ..limits()
        };
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ name.len() as u64 ^ seed.len() as u64);
        for i in 0..iterations() {
            let case = mutate(&mut rng, &seed);
            let started = std::time::Instant::now();
            let outcome = catch_unwind(AssertUnwindSafe(|| render(&case, &limits)));
            let spent = started.elapsed();
            let problem = match outcome {
                Err(_) => {
                    let at = PANIC_AT.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    if ours(&at) {
                        Some(format!("panic at {at}"))
                    } else {
                        *contained.entry(at).or_default() += 1;
                        None
                    }
                }
                Ok(Ok(art)) => art
                    .check(&limits)
                    .err()
                    .map(|e| format!("over the limits: {e}")),
                Ok(Err(_)) => None,
            }
            .or_else(|| {
                // The budget plus parsing: far less than the client's deadline.
                (spent.as_secs() >= 3).then(|| format!("slow: {spent:?}"))
            });
            if let Some(problem) = problem {
                let path = std::env::temp_dir().join(format!("evolite-fuzz-{name}-{i}"));
                let _ = std::fs::write(&path, &case);
                failures.push(format!("{name} #{i}: {problem} ({})", path.display()));
            }
        }
    }
    let _ = std::panic::take_hook();
    for (at, count) in &contained {
        println!("caught inside a library, picture refused: {count} × {at}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// One saved case again, with the fuzzing limits and a full backtrace:
/// `EVOLITE_FUZZ_CASE=<file> cargo test -p evolite-render --test fuzz replay -- --ignored`.
#[test]
#[ignore = "replays the case named in EVOLITE_FUZZ_CASE"]
fn replay() {
    let path = std::env::var("EVOLITE_FUZZ_CASE").expect("EVOLITE_FUZZ_CASE");
    let bytes = std::fs::read(path).unwrap();
    let art = render(&bytes, &limits());
    println!("{:?}", art.map(|a| (a.width, a.height, a.frames.len())));
}

#[test]
fn the_wire_survives_any_bytes() {
    let limits = Limits::default();
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut valid = Vec::new();
    wire::write_request(&mut valid, 1, 64, wire::FIRST_FRAME, b"<svg/>").unwrap();
    for _ in 0..iterations() * 50 {
        let case = if rng.below(2) == 0 {
            mutate(&mut rng, &valid)
        } else {
            let mut bytes: Vec<u8> = b"EVR1".to_vec();
            bytes.extend((0..rng.below(64)).map(|_| rng.next() as u8));
            bytes
        };
        // Either end may be handed garbage; neither panics nor takes memory past the limits.
        let _ = wire::read_request(&mut Cursor::new(&case), &limits);
        let _ = wire::read_answer(&mut Cursor::new(&case), &limits);
    }
}
