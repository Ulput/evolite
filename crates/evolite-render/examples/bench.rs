//! How long pictures take: drawing alone (in this process) and the whole way through the
//! sandboxed worker (start, fonts, lockdown, pipes, checks).
//!
//! `cargo run --release -p evolite-render --example bench -- <worker.exe> picture.svg …`
//! with the worker from `cargo build --release -p evolite-render --bin evolite-render-worker`.

use std::time::{Duration, Instant};

use evolite_render::{Limits, Renderer, Settings, render};

fn ms(d: Duration) -> String {
    format!("{:.0} ms", d.as_secs_f64() * 1000.0)
}

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((worker, files)) = args.split_first() else {
        eprintln!("usage: bench <worker.exe> <picture>…");
        std::process::exit(2);
    };
    for file in files {
        let bytes = std::fs::read(file).expect("read the picture");
        let name = std::path::Path::new(file)
            .file_name()
            .unwrap()
            .to_string_lossy();

        // Drawing alone; the first call also finds the system fonts.
        let started = Instant::now();
        let art = render(&bytes, &Limits::default()).expect("drawn");
        let first = started.elapsed();
        let runs: Vec<Duration> = (0..5)
            .map(|_| {
                let t = Instant::now();
                render(&bytes, &Limits::default()).unwrap();
                t.elapsed()
            })
            .collect();
        let size: usize = art.frames.iter().map(|f| f.rgba.len()).sum();

        // Through the sandbox: a fresh worker, then the same worker again.
        let mut renderer = Renderer::new(Settings::new(worker.into(), Vec::new()));
        let t = Instant::now();
        renderer.render(&bytes).expect("drawn by the worker");
        let cold = t.elapsed();
        let warm: Vec<Duration> = (0..5)
            .map(|_| {
                let t = Instant::now();
                renderer.render(&bytes).unwrap();
                t.elapsed()
            })
            .collect();

        println!(
            "{name}: {} × {}, {} frames, {:.1} MB of frames\n  drawing: first {} (fonts too), then {}\n  sandboxed worker: cold {}, warm {}",
            art.width,
            art.height,
            art.frames.len(),
            size as f64 / 1_048_576.0,
            ms(first),
            ms(median(runs)),
            ms(cold),
            ms(median(warm)),
        );
    }
}
