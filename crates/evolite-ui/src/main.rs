//! evolite on egui: the wallet screens. The wallet itself is the core (`evolite-core`), which
//! runs in its own thread; this crate shows its snapshots and sends it commands through
//! `evolite-api`, and never holds a key. The core is the real one (testnet; mainnet with
//! `--network mainnet`).
//!
//! `cargo run -- [--data-dir DIR] [--network testnet|mainnet] [--theme dark|light] [--lang CODE]`
//!
//! With the `dev-tools` feature, for screenshots and checks: `--demo` (the mock with the Figma
//! example), `--screen NAME` (a screen in a Figma state, over the mock), `--open NAME` (the same
//! over the real wallet), `--screenshot out.png`, `--window WxH`.

// A release build is a windowed program on Windows: no console opens with it. What it would have
// printed goes to the journal, and a start that fails says so in a message box.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Marks interface text: the literal as it is, in English — the key the catalogs
/// (`locales/<code>.json`) translate and `extract.pl` collects from these marks. The words are
/// translated where they reach the screen ([`i18n::t`]), not here.
macro_rules! tr {
    ($text:literal) => {
        $text
    };
}

/// The snapshot is the demo core's: never in a build without it (the release), where every
/// branch on it goes away.
fn is_demo(snap: &evolite_api::Snapshot) -> bool {
    cfg!(demo_core) && snap.demo
}

mod app;
mod art;
mod capture;
mod cli;
mod clipboard;
mod components;
mod context;
mod csv_export;
mod diagnostics;
mod focus;
mod format;
mod i18n;
mod icons;
mod links;
mod motion;
mod notify;
mod prefs;
mod private_dir;
mod rtl;
mod screens;
#[cfg(feature = "dev-tools")]
mod shot;
mod text;
mod theme;
mod window;

fn main() {
    // The same executable is the picture renderer (`evolite-render`): started with this flag
    // it decodes untrusted pictures for its parent and never opens the wallet or a window.
    if std::env::args().nth(1).as_deref() == Some(evolite_render::worker::FLAG) {
        evolite_render::worker::run();
    }
    // A windowed release has no console of its own: what the screenshot tools print goes to
    // the terminal they were started from.
    #[cfg(all(windows, feature = "dev-tools"))]
    // SAFETY: attaching to the parent's console; nothing else uses the console here.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
    let args: Vec<String> = std::env::args().collect();
    let launch = cli::Launch::from_args(&args);
    let lang = launch.lang.unwrap_or_else(i18n::system);
    // A data folder made now is its owner's alone, wherever --data-dir puts it — before the
    // journal below or the core create anything in it. (Without the demo core, the real one is
    // the only choice.)
    #[allow(irrefutable_let_patterns)]
    if let cli::CoreChoice::Real(dir) = &launch.core {
        let _ = private_dir::create(dir);
    }
    // The journal for problem reports, beside the wallet's files (the demo's in its cache).
    diagnostics::start(&cli::logs_for(&launch.core));
    // `--window 853x533` (dev-tools): a given size in points, smaller than the usual minimum
    // too — how a 1280 × 800 screen at 150 % looks, for screenshots.
    #[cfg(feature = "dev-tools")]
    let window = launch.dev.window;
    #[cfg(not(feature = "dev-tools"))]
    let window: Option<[f32; 2]> = None;
    // The smallest window fits a 1366 × 768 screen at 150 % (910 × 512 points, less the taskbar
    // and the title bar); screens scroll below that height and the menu becomes a rail.
    let (size, min) = match window {
        Some(size) => (size, size),
        None => ([1280.0, 800.0], [860.0, 440.0]),
    };
    // The last run ended with its window open (a crash, a driver reset): the saved size and the
    // maximized state are dropped, so the window that may have brought it down does not open
    // again at once.
    let (mark, crashed) = window::OpenMark::take(&window::mark_path()).unwrap_or((None, false));
    if crashed {
        window::forget_saved_window(APP_ID);
        evolite_core::diag::record(evolite_core::diag::Record::WindowReset);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("evolite")
            .with_icon(app_icon())
            .with_inner_size(size)
            .with_min_inner_size(min),
        // A size asked for is not overridden by the one remembered from the last run.
        persist_window: window.is_none(),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: window::wgpu_options(),
        ..Default::default()
    };
    let result = eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, launch)))),
    );
    match result {
        Ok(()) => {
            if let Some(mark) = mark {
                let _ = mark.close();
            }
        }
        // The window never opened: nothing of it to restore, and the person is told why.
        Err(e) => {
            if let Some(mark) = mark {
                let _ = mark.close();
            }
            window::report_start_failure(&e, lang);
            std::process::exit(1);
        }
    }
}

/// The name eframe keeps the window's saved state under.
const APP_ID: &str = "evolite";

/// The window's icon: the brandbook's app icon (`assets/app`, drawn by `examples/app_icon.rs`).
fn app_icon() -> egui::IconData {
    let png = include_bytes!("../assets/app/evolite-app.png");
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .expect("the bundled icon")
        .into_rgba8();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}
