//! The window itself: how it is drawn (wgpu, with the system's software renderer where there is
//! no graphics driver), what happens when it cannot open (a message, not a silent exit), and a
//! start after a crash (the saved maximized window is not opened again at once).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// What the mark holds while a window is open, and after it closed.
const OPEN: &[u8] = b"open";
const CLOSED: &[u8] = b"closed";

/// "A window is open": written at the start, "closed" at a clean exit. Locked while the app
/// runs, so a second copy (another data folder) does not take a running one for a crash; the
/// system lets go of the lock when the process dies, the word stays.
pub struct OpenMark {
    file: File,
}

/// The mark's file: in the app's cache folder, one for all data folders (the window's saved
/// size and placement are one for all too).
pub fn mark_path() -> PathBuf {
    crate::cli::cache_dir().join("window.open")
}

impl OpenMark {
    /// Takes the mark. `None`: another copy of the app holds it. The flag: the last run ended
    /// with its window open.
    pub fn take(path: &Path) -> std::io::Result<(Option<Self>, bool)> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok((None, false)),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
        let mut was = Vec::new();
        file.read_to_end(&mut was)?;
        let mut mark = Self { file };
        mark.write(OPEN)?;
        Ok((Some(mark), was == OPEN))
    }

    /// The window closed as it should: the next start restores it as it was.
    pub fn close(mut self) -> std::io::Result<()> {
        self.write(CLOSED)
    }

    fn write(&mut self, word: &[u8]) -> std::io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(word)?;
        self.file.sync_all()
    }
}

/// Drops the window's saved size, placement and maximized state (eframe's `app.ron`; the app's
/// own settings live elsewhere): the next window opens at the default size.
pub fn forget_saved_window(app_id: &str) {
    if let Some(dir) = eframe::storage_dir(app_id) {
        let _ = std::fs::remove_file(dir.join("app.ron"));
    }
}

/// wgpu with every backend the build has (Direct3D 12, Vulkan, Metal, OpenGL). The adapter: a
/// real graphics card if there is one, else the system's software renderer (WARP on Windows,
/// llvmpipe on Linux), slower but enough for these screens.
pub fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
    use std::sync::Arc;
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.native_adapter_selector = Some(Arc::new(|adapters, surface| {
        pick_adapter(adapters, surface)
            .cloned()
            .ok_or_else(|| "no graphics adapter can draw to this window".to_owned())
    }));
    eframe::egui_wgpu::WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}

/// The best adapter that can draw to `surface`: by kind, hardware first, the software renderer
/// last.
fn pick_adapter<'a>(
    adapters: &'a [eframe::wgpu::Adapter],
    surface: Option<&eframe::wgpu::Surface<'_>>,
) -> Option<&'a eframe::wgpu::Adapter> {
    adapters
        .iter()
        .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
        .min_by_key(|a| {
            let info = a.get_info();
            (rank(info.device_type), backend_rank(info.backend))
        })
}

/// Hardware first, the software renderer (`Cpu`) last.
fn rank(kind: eframe::wgpu::DeviceType) -> u8 {
    use eframe::wgpu::DeviceType::*;
    match kind {
        DiscreteGpu => 0,
        IntegratedGpu => 1,
        VirtualGpu => 2,
        Other => 3,
        Cpu => 4,
    }
}

/// Among adapters of one kind: the system's own API first, OpenGL last.
fn backend_rank(backend: eframe::wgpu::Backend) -> u8 {
    use eframe::wgpu::Backend::*;
    match backend {
        Dx12 | Metal => 0,
        Vulkan => 1,
        Gl => 2,
        _ => 3,
    }
}

/// The window did not open: the reason goes to the journal and to the person — a message box
/// on Windows (a release has no console), the terminal elsewhere.
pub fn report_start_failure(error: &eframe::Error, lang: crate::i18n::Lang) {
    let text = error.to_string();
    evolite_core::diag::record_now(evolite_core::diag::Record::StartFailed { text: text.clone() });
    crate::i18n::set(lang);
    let message = format!(
        "{}\n\n{text}\n\n{}",
        crate::i18n::t(tr!(
            "evolite could not open its window: the graphics system did not start."
        )),
        crate::i18n::t(tr!(
            "Updating the graphics driver may help; on a virtual machine or a remote desktop, turn on 3D acceleration. The wallet and its files are not affected."
        ))
    );
    show_message(&message);
}

#[cfg(windows)]
fn show_message(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(message), wide("evolite"));
    // SAFETY: both strings are NUL-terminated UTF-16 that live through the call; no owner window.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_message(message: &str) {
    eprintln!("{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evolite-window-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("window.open")
    }

    #[test]
    fn a_window_left_open_is_seen_at_the_next_start() {
        let path = temp("crash");
        let (mark, crashed) = OpenMark::take(&path).unwrap();
        assert!(!crashed, "the first start");
        mark.unwrap().close().unwrap();
        let (mark, crashed) = OpenMark::take(&path).unwrap();
        assert!(!crashed, "closed as it should");
        // A crash: the process goes, its lock with it, the word stays.
        drop(mark);
        let (mark, crashed) = OpenMark::take(&path).unwrap();
        assert!(crashed, "the window was left open");
        mark.unwrap().close().unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_running_copy_is_not_a_crash() {
        let path = temp("second");
        let (first, _) = OpenMark::take(&path).unwrap();
        let (second, crashed) = OpenMark::take(&path).unwrap();
        assert!(second.is_none() && !crashed);
        first.unwrap().close().unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn hardware_before_the_software_renderer() {
        use eframe::wgpu::{Backend, DeviceType};
        assert!(rank(DeviceType::IntegratedGpu) < rank(DeviceType::Cpu));
        assert!(rank(DeviceType::DiscreteGpu) < rank(DeviceType::IntegratedGpu));
        assert!(backend_rank(Backend::Dx12) < backend_rank(Backend::Gl));
        assert!(backend_rank(Backend::Vulkan) < backend_rank(Backend::Gl));
    }
}
