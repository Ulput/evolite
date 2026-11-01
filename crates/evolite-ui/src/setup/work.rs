//! The installation and the removal themselves, on a thread of their own: the window shows the
//! step and the share done, and the result at the end.

use super::payload::{self, Payload};
use super::plan::{self, Known};
use super::system;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where the work is, for the window.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// Steps finished (the window ticks them off).
    pub step: usize,
    /// Of the bytes to copy (installation only).
    pub copied: u64,
    pub total: u64,
    /// Set once: `Ok` when all went through, the reason when not.
    pub result: Option<Result<(), String>>,
}

pub type Shared = Arc<Mutex<Progress>>;

fn set(p: &Shared, f: impl FnOnce(&mut Progress)) {
    if let Ok(mut p) = p.lock() {
        f(&mut p);
    }
}

/// What the person chose for the installation.
#[derive(Clone, Debug)]
pub struct Install {
    /// The installer itself, which carries the files.
    pub source: PathBuf,
    pub payload: Payload,
    pub dir: PathBuf,
    pub desktop: bool,
    pub known: Known,
    /// The interface language chosen, for evolite's first start and the uninstaller.
    pub language: &'static str,
}

pub fn install(job: Install, p: Shared) {
    let total = job.payload.installed_size();
    set(&p, |p| p.total = total);
    let result = run_install(&job, &p);
    set(&p, |p| p.result = Some(result));
}

/// The folder a file goes to, made if it is not there yet (macOS bundles nest).
fn place(dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    Ok(path)
}

/// Linux and macOS start a file only when it may be executed.
fn executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let _ = path;
    Ok(())
}

/// Writes a small file whole: beside its place first, then renamed over.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let part = path.with_extension("part");
    std::fs::write(&part, bytes).map_err(|e| format!("{}: {e}", part.display()))?;
    std::fs::rename(&part, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// What the program needs around it besides the payload: the icon of the Linux menu entries;
/// macOS's two bundles, each with its `Info.plist` and icon.
fn extras(dir: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    write_file(&place(dir, plan::ICON)?, crate::APP_PNG)?;
    #[cfg(target_os = "macos")]
    {
        let version = env!("CARGO_PKG_VERSION");
        let icon = plan::icns(crate::APP_PNG);
        for (bundle, exe, name, id) in [
            ("evolite.app", "evolite", "evolite", "org.evolite.app"),
            (
                "Uninstall evolite.app",
                "uninstall",
                "Uninstall evolite",
                "org.evolite.uninstall",
            ),
        ] {
            let contents = format!("{bundle}/Contents");
            write_file(
                &place(dir, &format!("{contents}/Info.plist"))?,
                plan::info_plist(exe, name, id, version).as_bytes(),
            )?;
            write_file(
                &place(dir, &format!("{contents}/Resources/evolite.icns"))?,
                &icon,
            )?;
        }
    }
    let _ = dir;
    Ok(())
}

fn run_install(job: &Install, p: &Shared) -> Result<(), String> {
    if system::running(&job.known) || system::in_use(&job.dir.join(plan::APP)) {
        return Err(tr!("evolite is running. Close it and try again.").into());
    }
    std::fs::create_dir_all(&job.dir).map_err(|e| e.to_string())?;
    set(p, |p| p.step = 1);
    let mut source = std::fs::File::open(&job.source).map_err(|e| e.to_string())?;
    let mut done = 0;
    for entry in &job.payload.entries {
        let Some(rel) = plan::target_of(&entry.name) else {
            return Err(format!("{} is not for this system", entry.name));
        };
        let dest = place(&job.dir, rel)?;
        payload::extract(&mut source, entry, &dest, |n| {
            set(p, |p| p.copied = done + n)
        })?;
        executable(&dest)?;
        done += entry.size;
    }
    // The uninstaller: this installer without the files it carries.
    let stub = payload::stub(&mut source, Some(&job.payload))?;
    let uninstaller = place(&job.dir, plan::UNINSTALLER)?;
    write_file(&uninstaller, &stub)?;
    executable(&uninstaller)?;
    extras(&job.dir)?;
    set(p, |p| p.step = 2);
    system::add_shortcuts(&job.dir, job.desktop, &job.known, job.language)?;
    set(p, |p| p.step = 3);
    system::write_entry(&system::Entry {
        dir: &job.dir,
        version: env!("CARGO_PKG_VERSION"),
        size: job.payload.installed_size() + stub.len() as u64,
        language: job.language,
    })?;
    first_language(&job.known, job.language);
    set(p, |p| p.step = 4);
    Ok(())
}

/// evolite starts in the installer's language: written to its settings only when there are
/// none yet — a language chosen in the app before stays.
fn first_language(known: &Known, language: &str) {
    let data = known.testnet_data();
    let settings = data.join("settings.json");
    if settings.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(&data);
    let _ = std::fs::write(settings, format!("{{\n  \"lang\": \"{language}\"\n}}\n"));
}

/// What the person chose for the removal.
#[derive(Clone, Debug)]
pub struct Remove {
    /// The program folder: the uninstaller's own.
    pub dir: PathBuf,
    pub data: bool,
    pub known: Known,
}

pub const REMOVE_STEPS: usize = 3;

pub fn remove(job: Remove, p: Shared) {
    let result = run_remove(&job, &p);
    set(&p, |p| p.result = Some(result));
}

/// Deletes one of evolite's own folders whole — only after checking it is one.
fn remove_own(known: &Known, path: &Path) -> Result<(), String> {
    if !known.is_own_folder(path) {
        return Err(format!("not evolite's folder: {}", path.display()));
    }
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn run_remove(job: &Remove, p: &Shared) -> Result<(), String> {
    if system::running(&job.known) || system::in_use(&job.dir.join(plan::APP)) {
        return Err(tr!("evolite is running. Close it and try again.").into());
    }
    // The program: only its own files, by name; the folder goes once it is empty.
    let me = std::env::current_exe().ok();
    let uninstaller = job.dir.join(plan::UNINSTALLER);
    for name in plan::program_files() {
        let file = job.dir.join(name);
        // Windows: the running uninstaller goes after it exits.
        if cfg!(windows) && file == uninstaller && me.as_deref() == Some(file.as_path()) {
            continue;
        }
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", file.display())),
        }
    }
    // macOS: what the system added to the bundles (signatures, caches) goes with them.
    for bundle in plan::BUNDLES {
        match std::fs::remove_dir_all(job.dir.join(bundle)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{bundle}: {e}")),
        }
    }
    system::remove_shortcuts(&job.known);
    set(p, |p| p.step = 1);
    if job.data {
        for folder in job.known.data_folders() {
            remove_own(&job.known, &folder)?;
        }
    }
    set(p, |p| p.step = 2);
    remove_own(&job.known, &job.known.cache())?;
    system::remove_entry()?;
    if cfg!(windows) && me.as_deref() == Some(uninstaller.as_path()) {
        system::delete_after_exit(&uninstaller);
    } else {
        let _ = std::fs::remove_dir(&job.dir);
    }
    set(p, |p| p.step = 3);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_known(root: &Path) -> Known {
        Known {
            local: root.join("Local"),
            roaming: root.join("Roaming"),
            programs: root.join("Programs"),
        }
    }

    /// The whole cycle on this computer, with the program, data and cache folders in a
    /// temporary folder: the program and the uninstaller in place, the shortcuts, the language
    /// for the uninstaller, evolite's first language — then all of it removed again, the data
    /// too. On Windows it writes the real `HKCU\…\Uninstall\evolite` key and Start menu
    /// shortcut, so it refuses to run where evolite is installed; on Linux the menu entries go
    /// to the temporary folder as well (`XDG_DATA_HOME/applications`).
    /// `cargo test -p evolite-ui --bin evolite-setup live_install -- --ignored`
    #[test]
    #[ignore = "Windows: writes the user's Start menu and HKCU; run by hand"]
    fn live_install_and_remove() {
        #[cfg(windows)]
        let menu = system::start_menu().unwrap().join("evolite.lnk");
        #[cfg(windows)]
        assert!(
            !menu.exists() && system::installed_language(Path::new("")).is_none(),
            "evolite is installed here: not touching it"
        );
        let root = std::env::temp_dir().join(format!("evolite-setup-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let known = temp_known(&root);
        #[cfg(target_os = "linux")]
        let menu = known.roaming.join("applications").join("evolite.desktop");
        let dir = known.default_program();
        let source = root.join("evolite-setup");
        std::fs::create_dir_all(&root).unwrap();
        let packed = payload::pack(
            b"MZ the installer",
            &[
                (plan::APP_NAME.into(), b"the app".to_vec()),
                (plan::RENDERER_NAME.into(), b"the renderer".to_vec()),
            ],
        )
        .unwrap();
        std::fs::write(&source, &packed).unwrap();
        let payload = payload::read(&mut std::fs::File::open(&source).unwrap())
            .unwrap()
            .unwrap();

        let p = Shared::default();
        install(
            Install {
                source,
                payload,
                dir: dir.clone(),
                desktop: false,
                known: known.clone(),
                language: "ru",
            },
            p.clone(),
        );
        assert_eq!(p.lock().unwrap().result, Some(Ok(())));
        assert_eq!(std::fs::read(dir.join(plan::APP)).unwrap(), b"the app");
        assert_eq!(
            std::fs::read(dir.join(plan::RENDERER)).unwrap(),
            b"the renderer"
        );
        assert_eq!(
            std::fs::read(dir.join(plan::UNINSTALLER)).unwrap(),
            b"MZ the installer",
            "the uninstaller is the installer without the program"
        );
        for extra in plan::EXTRA {
            assert!(dir.join(extra).is_file(), "{extra}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for exe in [plan::APP, plan::RENDERER, plan::UNINSTALLER] {
                let mode = std::fs::metadata(dir.join(exe))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o111, 0o111, "{exe} may be executed");
            }
        }
        #[cfg(not(target_os = "macos"))]
        assert!(menu.exists());
        #[cfg(target_os = "linux")]
        {
            let entry = std::fs::read_to_string(&menu).unwrap();
            assert!(entry.contains(&format!("Exec=\"{}\"", dir.join(plan::APP).display())));
            assert!(
                known
                    .roaming
                    .join("applications")
                    .join("evolite-uninstall.desktop")
                    .exists()
            );
        }
        assert_eq!(system::installed_language(&dir).as_deref(), Some("ru"));
        let settings = std::fs::read_to_string(known.testnet_data().join("settings.json")).unwrap();
        assert!(settings.contains("\"lang\": \"ru\""));

        // A wallet in the data folder, and pictures in the cache: deleted with «data too».
        let wallet = known
            .testnet_data()
            .join("wallets")
            .join("testnet")
            .join("aaaa0001");
        std::fs::create_dir_all(&wallet).unwrap();
        std::fs::write(wallet.join("wallet.json"), b"{}").unwrap();
        std::fs::create_dir_all(known.cache()).unwrap();
        std::fs::write(known.cache().join("a.png"), b"png").unwrap();
        // Something of the person's own beside the program folder stays.
        std::fs::write(known.programs.join("notes.txt"), b"mine").unwrap();
        let p = Shared::default();
        remove(
            Remove {
                dir: dir.clone(),
                data: true,
                known: known.clone(),
            },
            p.clone(),
        );
        assert_eq!(p.lock().unwrap().result, Some(Ok(())));
        assert!(!dir.exists(), "the program folder, empty, is gone");
        #[cfg(not(target_os = "macos"))]
        assert!(!menu.exists());
        assert!(
            system::installed_language(&dir).is_none(),
            "the entry is gone"
        );
        assert!(!known.testnet_data().exists() && !known.cache().exists());
        // Nothing outside evolite's own folders: the roaming and local folders stay.
        assert!(known.roaming.exists() && known.local.exists());
        assert!(known.programs.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The program runs while it holds its data folder's lock: neither installed over nor
    /// removed then.
    #[test]
    fn a_running_evolite_is_seen_by_its_lock() {
        let root = std::env::temp_dir().join(format!("evolite-setup-lock-{}", std::process::id()));
        let known = temp_known(&root);
        assert!(!system::running(&known), "no data folder yet");
        std::fs::create_dir_all(known.testnet_data()).unwrap();
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(known.testnet_data().join("evolite.lock"))
            .unwrap();
        assert!(!system::running(&known), "a lock file nobody holds");
        lock.try_lock().unwrap();
        assert!(system::running(&known));
        let p = Shared::default();
        remove(
            Remove {
                dir: known.default_program(),
                data: true,
                known: known.clone(),
            },
            p.clone(),
        );
        assert!(matches!(p.lock().unwrap().result, Some(Err(_))));
        assert!(known.testnet_data().exists(), "nothing deleted");
        drop(lock);
        assert!(!system::running(&known));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// «Close evolite» ends only processes started from the program folder: found by their
    /// full path, never by a name alone.
    #[test]
    fn a_running_program_is_found_by_its_full_path() {
        let me = std::env::current_exe().unwrap();
        if cfg!(any(windows, target_os = "linux")) {
            assert!(system::processes_of(&me).contains(&std::process::id()));
        }
        let elsewhere = std::env::temp_dir()
            .join("evolite-elsewhere")
            .join(me.file_name().unwrap());
        assert!(
            system::processes_of(&elsewhere).is_empty(),
            "same name, another folder"
        );
        assert!(system::processes_in(&std::env::temp_dir().join("evolite-nowhere")).is_empty());
    }

    /// What «Close evolite» does to a process that does not end by itself.
    #[test]
    fn a_hung_process_is_ended() {
        let mut child = if cfg!(windows) {
            std::process::Command::new("ping")
                .args(["-n", "60", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .spawn()
        } else {
            std::process::Command::new("sleep").arg("60").spawn()
        }
        .unwrap();
        assert!(child.try_wait().unwrap().is_none(), "still running");
        assert!(system::end_process(child.id()));
        assert!(child.wait().is_ok_and(|s| !s.success()));
    }

    #[test]
    fn only_own_folders_are_deleted_whole() {
        let known = temp_known(&std::env::temp_dir().join("evolite-nowhere"));
        assert!(remove_own(&known, &known.roaming).is_err());
        assert!(remove_own(&known, &std::env::temp_dir()).is_err());
        assert!(
            remove_own(&known, &known.testnet_data()).is_ok(),
            "not there: nothing to do"
        );
    }
}
