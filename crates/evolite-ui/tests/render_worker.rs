//! `evolite --render-worker` is the picture renderer: the app's own executable, started by the
//! wallet with this flag, answers over its pipes and never opens a window.

use evolite_render::{RenderError, Renderer, Settings, worker};

/// As long as a loaded machine needs to start a sandboxed worker, find its fonts and draw: the
/// app's 5 s deadline and 3 s budget made these fail under a full check beside other builds
/// (BUG-144). What they guard is what is drawn, not how fast; the deadline's own behaviour is
/// evolite-render's worker tests'.
fn patient(mut settings: Settings) -> Settings {
    settings.timeout = std::time::Duration::from_secs(60);
    settings.limits.max_render_ms = 60_000;
    settings
}

#[test]
fn the_app_executable_serves_as_the_renderer() {
    let settings = Settings::new(
        env!("CARGO_BIN_EXE_evolite").into(),
        vec![worker::FLAG.into()],
    );
    let mut renderer = Renderer::new(patient(settings));
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="lime"/></svg>"#;
    let art = renderer.render(svg).unwrap().unpacked().unwrap();
    assert_eq!((art.width, art.height, art.frames.len()), (4, 2, 1));
    assert_eq!(&art.frames[0].rgba[..4], &[0, 255, 0, 255]);
    assert_eq!(
        renderer.render(b"not a picture"),
        Err(RenderError::Unsupported)
    );
}

#[test]
fn text_in_an_svg_is_drawn_inside_the_sandbox() {
    // Fonts are read from the system folder, which the AppContainer may read.
    let settings = Settings::new(
        env!("CARGO_BIN_EXE_evolite").into(),
        vec![worker::FLAG.into()],
    );
    let mut renderer = Renderer::new(patient(settings));
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><text x="4" y="30" font-size="28" font-family="Arial, sans-serif" fill="black">Ag</text></svg>"#;
    let art = renderer.render(svg).unwrap().unpacked().unwrap();
    let inked = art.frames[0].rgba.chunks(4).filter(|p| p[3] > 0).count();
    assert!(inked > 50, "no text drawn: {inked} pixels");
}

#[test]
fn an_animated_svg_comes_back_as_frames() {
    let settings = Settings::new(
        env!("CARGO_BIN_EXE_evolite").into(),
        vec![worker::FLAG.into()],
    );
    let mut renderer = Renderer::new(patient(settings));
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="10"><rect width="10" height="10" fill="red"><animate attributeName="x" from="0" to="30" dur="1s" repeatCount="indefinite"/></rect></svg>"#;
    let art = renderer.render(svg).unwrap().unpacked().unwrap();
    assert_eq!(art.frames.len(), 20);
    assert_ne!(art.frames[0].rgba, art.frames[10].rgba);
}

/// Win32k off is really applied: the app itself links window code, so started that way it
/// cannot even load, and the picture is refused instead of drawn with Win32k on.
#[cfg(windows)]
#[test]
fn the_app_cannot_run_as_a_renderer_with_win32k_off() {
    let mut settings = Settings::new(
        env!("CARGO_BIN_EXE_evolite").into(),
        vec![worker::FLAG.into()],
    );
    settings.win32k_off = true;
    settings.timeout = std::time::Duration::from_secs(3);
    let mut renderer = Renderer::new(settings);
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"/>"#;
    assert!(renderer.render(svg).is_err());
}

/// The shipped renderer runs with the window and graphics system calls turned off, and still
/// draws text: fonts are read and shaped in Rust, not by GDI.
#[test]
fn the_shipped_renderer_draws_with_win32k_off() {
    let mut settings = Settings::new(env!("CARGO_BIN_EXE_evolite-renderer").into(), Vec::new());
    settings.win32k_off = true;
    let mut r = Renderer::new(patient(settings));
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><text x="4" y="30" font-size="28" font-family="Arial, sans-serif" fill="black">Ag</text></svg>"#;
    let art = r.render(svg).unwrap().unpacked().unwrap();
    let inked = art.frames[0].rgba.chunks(4).filter(|p| p[3] > 0).count();
    assert!(inked > 50, "no text drawn: {inked} pixels");
}
