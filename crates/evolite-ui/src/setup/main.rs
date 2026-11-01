//! evolite's installer and uninstaller for Windows, Linux and macOS (docs/installer.md): one
//! executable, with the same interface parts as evolite — palette, fonts, the 15 languages and
//! right-to-left Arabic.
//!
//! - `evolite-setup` — installs the program it carries (see [`payload`]) for this user.
//! - `uninstall` (`uninstall.exe --uninstall` on Windows) — the same program without the
//!   payload, in the program folder.
//! - `evolite-setup --pack OUT` — writes the installer for release: a copy of itself with
//!   `evolite` and `evolite-renderer` (`.exe` on Windows) from its own folder after it.
//!
//! It asks for no administrator or root rights (the Windows manifest says `asInvoker`) and
//! touches only the user's own folders (and `HKEY_CURRENT_USER` on Windows).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Marks interface text, as in evolite: the English literal is the catalogs' key
/// (`locales/setup/<code>.json`).
macro_rules! tr {
    ($text:literal) => {
        $text
    };
}

// evolite's own modules, shared as they are: languages, palette and fonts, the right-to-left
// mirror, the logo. The installer uses part of each.
#[allow(dead_code)]
#[path = "../i18n/mod.rs"]
mod i18n;
#[allow(dead_code)]
#[path = "../icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../rtl.rs"]
mod rtl;
#[allow(dead_code)]
#[path = "../theme.rs"]
mod theme;

mod payload;
mod plan;
mod system;
mod ui;
mod work;

use i18n::Lang;

/// The installer's own strings, a catalog per language (English is the source).
fn catalog(lang: Lang) -> Option<&'static str> {
    Some(match lang {
        Lang::En => return None,
        Lang::Ru => include_str!("../../locales/setup/ru.json"),
        Lang::ZhHans => include_str!("../../locales/setup/zh-Hans.json"),
        Lang::Hi => include_str!("../../locales/setup/hi.json"),
        Lang::Es => include_str!("../../locales/setup/es.json"),
        Lang::Fr => include_str!("../../locales/setup/fr.json"),
        Lang::Ar => include_str!("../../locales/setup/ar.json"),
        Lang::PtBr => include_str!("../../locales/setup/pt-BR.json"),
        Lang::Id => include_str!("../../locales/setup/id.json"),
        Lang::De => include_str!("../../locales/setup/de.json"),
        Lang::Ja => include_str!("../../locales/setup/ja.json"),
        Lang::Tr => include_str!("../../locales/setup/tr.json"),
        Lang::Ko => include_str!("../../locales/setup/ko.json"),
        Lang::Vi => include_str!("../../locales/setup/vi.json"),
        Lang::Uk => include_str!("../../locales/setup/uk.json"),
    })
}

/// `--pack OUT`: this installer with the program from its own folder after it.
fn pack(out: &std::path::Path) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = me.parent().ok_or("no folder")?;
    let mut file = std::fs::File::open(&me).map_err(|e| e.to_string())?;
    let carried = payload::read(&mut file)?;
    let stub = payload::stub(&mut file, carried.as_ref())?;
    let mut files = Vec::new();
    for name in [plan::APP_NAME, plan::RENDERER_NAME] {
        let bytes = std::fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
        files.push((name.to_owned(), bytes));
    }
    let packed = payload::pack(&stub, &files)?;
    std::fs::write(out, &packed).map_err(|e| e.to_string())?;
    println!(
        "{}: {} bytes (installer {}, program {})",
        out.display(),
        packed.len(),
        stub.len(),
        files.iter().map(|(_, b)| b.len()).sum::<usize>()
    );
    Ok(())
}

/// The app's icon, 256 pixels: the window's, and on Linux and macOS the installed app's.
const APP_PNG: &[u8] = include_bytes!("../../assets/app/evolite-app.png");

fn app_icon() -> egui::IconData {
    let image = image::load_from_memory_with_format(APP_PNG, image::ImageFormat::Png)
        .expect("the bundled icon")
        .into_rgba8();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

/// The installer's journal: what it did, and why it stopped — the place to look when its window
/// did not open. `%TEMP%\evolite-setup.log` (`$TMPDIR` elsewhere), the user's own.
fn log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("evolite-setup.log")
}

/// One line in the journal, with the local time. Never fails: a journal that cannot be written
/// must not stop the installer.
fn log(line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let _ = writeln!(
            f,
            "{} {line}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
    }
}

/// Why the window could not open, in the interface language, with where the details are.
fn could_not_start(why: &str) -> String {
    i18n::t(tr!(
        "evolite setup could not open its window: {}. Restart the computer or update the graphics driver, then start it again. Details are in {}."
    ))
    .replacen("{}", why, 1)
    .replacen("{}", &log_path().display().to_string(), 1)
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--pack") {
        // A release build has no console of its own: what `--pack` says goes to the one it was
        // started from.
        #[cfg(windows)]
        // SAFETY: attaching to the parent's console; nothing else uses the console here.
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
        let out = args.get(i + 1).map(std::path::PathBuf::from);
        let result = out
            .ok_or_else(|| "--pack OUT".to_owned())
            .and_then(|o| pack(&o));
        if let Err(e) = result {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    i18n::use_catalogs(catalog);
    let me = std::env::current_exe().ok();
    let uninstall =
        args.iter().any(|a| a == "--uninstall") || me.as_deref().is_some_and(plan::is_uninstaller);
    let uninstall_dir = me.as_deref().and_then(plan::program_dir_of);
    let mode = if uninstall {
        match &uninstall_dir {
            Some(dir) => ui::Mode::Uninstall { dir: dir.clone() },
            None => ui::Mode::Broken(tr!("The installer is damaged. Download it again.").into()),
        }
    } else {
        let read = me
            .as_ref()
            .ok_or_else(String::new)
            .and_then(|m| std::fs::File::open(m).map_err(|e| e.to_string()))
            .and_then(|mut f| payload::read(&mut f));
        match (read, me.clone()) {
            (Ok(Some(payload)), Some(source)) => ui::Mode::Install { source, payload },
            (Ok(None), _) => ui::Mode::Broken(
                tr!("This installer carries no program: it is built with --pack.").into(),
            ),
            _ => ui::Mode::Broken(tr!("The installer is damaged. Download it again.").into()),
        }
    };
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    // For screenshots: `--screen NAME [--screenshot out.png] [--theme dark|light] [--lang ar]`
    // (names in `ui::Setup::example`).
    let screen = arg("--screen");
    let screenshot = arg("--screenshot");
    let dark = arg("--theme").map(|t| t != "light");
    let lang = arg("--lang")
        .and_then(|c| Lang::from_code(&c))
        .or_else(|| {
            uninstall_dir
                .as_deref()
                .filter(|_| uninstall)
                .and_then(system::installed_language)
                .and_then(|c| Lang::from_code(&c))
        })
        .unwrap_or_else(i18n::system);
    i18n::set(lang);
    log(&format!(
        "start {} {} ({})",
        if uninstall { "uninstall" } else { "install" },
        env!("CARGO_PKG_VERSION"),
        me.as_ref()
            .map(|m| m.display().to_string())
            .unwrap_or_default()
    ));
    // A windowed program has no console: a panic would end it without a word. Say it in the
    // system's own message box and in the journal.
    std::panic::set_hook(Box::new(|info| {
        log(&format!("panic: {info}"));
        system::message_box("evolite", &could_not_start(&info.to_string()));
    }));
    // wgpu on the graphics card (DirectX 12, Vulkan, Metal or OpenGL); when it does not start (a
    // graphics driver that has just reset or crashed), wgpu on the processor.
    // `--renderer software` starts with the second: to check it where the card works.
    let skip = match arg("--renderer").as_deref() {
        Some("software") => 1,
        _ => 0,
    };
    let mut last = None;
    for (renderer, software) in [
        (eframe::Renderer::Wgpu, false),
        (eframe::Renderer::Wgpu, true),
    ]
    .into_iter()
    .skip(skip)
    {
        let mut options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title(if uninstall {
                    "evolite"
                } else {
                    "evolite setup"
                })
                .with_icon(app_icon())
                .with_inner_size([800.0, 600.0])
                .with_resizable(false),
            persist_window: false,
            renderer,
            ..Default::default()
        };
        // Last: the processor draws (Windows' WARP adapter, Mesa's on Linux), no driver needed.
        if software
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) =
                &mut options.wgpu_options.wgpu_setup
        {
            setup.native_adapter_selector = Some(std::sync::Arc::new(|adapters, _surface| {
                adapters
                    .iter()
                    .find(|a| a.get_info().device_type == eframe::wgpu::DeviceType::Cpu)
                    .cloned()
                    .ok_or_else(|| "no software adapter".to_owned())
            }));
        }
        let (mode, screen, screenshot) = (mode.clone(), screen.clone(), screenshot.clone());
        match eframe::run_native(
            "evolite-setup",
            options,
            Box::new(move |cc| {
                let setup = ui::Setup::new(cc, mode, lang);
                Ok(Box::new(match screen {
                    Some(name) => setup.example(&name, screenshot, dark),
                    None => setup.with_screenshot(screenshot),
                }))
            }),
        ) {
            Ok(()) => return Ok(()),
            Err(e) => {
                log(&format!(
                    "the {renderer:?} renderer{} did not start: {e}",
                    if software { " (software)" } else { "" }
                ));
                last = Some(e);
            }
        }
    }
    let e = last.expect("a renderer was tried");
    system::message_box("evolite", &could_not_start(&e.to_string()));
    Err(e)
}

#[cfg(test)]
mod tests;
