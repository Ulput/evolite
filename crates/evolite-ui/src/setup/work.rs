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
    /// The picture cache that could not go (the removal went on without it): the last screen
    /// names it when the wallet data was to go too.
    pub cache_left: Option<PathBuf>,
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
    // What is missing above the program folder is above the wallet's data too (`~/.local`).
    crate::private_dir::create_above(&job.dir).map_err(|e| e.to_string())?;
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
    let _ = crate::private_dir::create(&data);
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

/// The wallet data, when the person ticked it (`data`): every one of evolite's data folders —
/// both networks' (mainnet's holds real money) and the app's common one — and nothing beside
/// them. Untouched without the tick.
fn remove_data(known: &Known, data: bool) -> Result<(), String> {
    if data {
        for folder in known.data_folders() {
            remove_own(known, &folder)?;
        }
    }
    Ok(())
}

/// The running uninstaller, when the removal left it to go after this process exits.
static SELF_DELETE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// BUG-246: called as the window closes — the uninstaller's file and its folder go once this
/// process is gone, however long the last screen stayed open.
pub fn delete_self_at_exit() {
    if let Some(exe) = SELF_DELETE.lock().unwrap_or_else(|e| e.into_inner()).take() {
        system::delete_after_exit(&exe);
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
    system::forget_crash_exclusion(&job.dir.join(plan::APP));
    set(p, |p| p.step = 1);
    remove_data(&job.known, job.data)?;
    set(p, |p| p.step = 2);
    // QA 2026-11-20 (os error 4395, a reparse point under the cache in a sandboxed run): the
    // picture cache is not worth a failed removal. What cannot go stays and is said in the log;
    // the entry and the uninstaller still go. The wallet data above still fails loudly: that is
    // the person's choice to see through.
    if let Err(e) = remove_own(&job.known, &job.known.cache()) {
        crate::log(&format!("the picture cache is left: {e}"));
        let left = job.known.cache();
        set(p, |p| p.cache_left = Some(left));
    }
    system::remove_entry()?;
    if cfg!(windows) && me.as_deref() == Some(uninstaller.as_path()) {
        system::delete_after_exit(&uninstaller);
        // BUG-246: and once more when the window closes ([`delete_self_at_exit`]).
        *SELF_DELETE.lock().unwrap_or_else(|e| e.into_inner()) = Some(uninstaller);
    } else {
        let _ = std::fs::remove_dir(&job.dir);
    }
    set(p, |p| p.step = 3);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new process holds a copy of this process's open files until its `exec` closes them, and
    /// a file lock lasts while any copy is open (BUG-98: about 45 ms on macOS's CI). Every test
    /// in this binary that starts a process takes this, and so does the test of the lock, so
    /// that none starts while the lock is tested.
    static PROCESS_STARTS: Mutex<()> = Mutex::new(());

    /// A test that failed holding it says nothing about the others.
    fn no_process_starts() -> std::sync::MutexGuard<'static, ()> {
        PROCESS_STARTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn temp_known(root: &Path) -> Known {
        Known {
            local: root.join("Local"),
            roaming: root.join("Roaming"),
            programs: root.join("Programs"),
        }
    }

    /// Installer review r1, Should: the removal of the data itself. Without the tick all three
    /// folders stay; with it exactly Roaming/evolite, evolite-mainnet and evolite-app go, and a
    /// folder and a file beside them named alike stay — the guard of the folder with mainnet's
    /// money.
    #[test]
    fn the_data_removal_takes_exactly_evolites_folders() {
        let root = std::env::temp_dir().join(format!("evolite-setup-data-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let known = temp_known(&root);
        let ours = [known.testnet_data(), known.mainnet_data(), known.app_data()];
        for folder in &ours {
            std::fs::create_dir_all(folder.join("wallets")).unwrap();
            std::fs::write(folder.join("wallets").join("w.json"), b"{}").unwrap();
        }
        let beside = known.roaming.join("evolite-backup");
        std::fs::create_dir_all(&beside).unwrap();
        let file = known.roaming.join("evolite.txt");
        std::fs::write(&file, b"notes").unwrap();
        remove_data(&known, false).unwrap();
        assert!(ours.iter().all(|f| f.exists()), "no tick: all three stay");
        remove_data(&known, true).unwrap();
        for folder in &ours {
            assert!(!folder.exists(), "{} gone", folder.display());
        }
        assert_eq!(
            [
                known.testnet_data().ends_with("Roaming/evolite"),
                known.mainnet_data().ends_with("Roaming/evolite-mainnet"),
                known.app_data().ends_with("Roaming/evolite-app"),
            ],
            [true; 3],
            "the folders the test names"
        );
        assert!(beside.exists(), "Roaming/evolite-backup stays");
        assert!(file.exists(), "Roaming/evolite.txt stays");
        let _ = std::fs::remove_dir_all(&root);
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
        let _no_starts = no_process_starts();
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
        assert!(!system::running(&known), "{}", still_held(&known));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// BUG-98, the failure's message: this test's own lock is gone, yet the file counts as
    /// locked (seen once, on macOS, cause not proven). Says for how long, and who has it open
    /// if that lasts past 200 ms; the test fails all the same.
    fn still_held(known: &Known) -> String {
        const NOT_TAKEN: &str =
            "lsof not taken: it runs 200 ms after the drop, a shorter holder is not named";
        let file = known.testnet_data().join("evolite.lock");
        let start = std::time::Instant::now();
        let mut lsof: Option<String> = None;
        loop {
            let said = lsof.as_deref().unwrap_or(NOT_TAKEN);
            if !system::running(known) {
                return format!(
                    "held after the drop, free {:?} later; {said}",
                    start.elapsed()
                );
            }
            if start.elapsed() >= std::time::Duration::from_secs(5) {
                return format!("still held 5 s after the drop; {said}");
            }
            if lsof.is_none() && start.elapsed().as_millis() >= 200 {
                lsof = Some(who_has_open(&file));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// `lsof` on the file, for at most three seconds: CI's job has no time limit of its own.
    fn who_has_open(file: &Path) -> String {
        let spawned = std::process::Command::new("lsof")
            .arg(file)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => return format!("lsof did not start: {e}"),
        };
        let start = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() < std::time::Duration::from_secs(3) => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return "lsof did not finish in 3 s".to_owned();
                }
            }
        }
        match child.wait_with_output() {
            Ok(o) => format!("lsof: {:?}", String::from_utf8_lossy(&o.stdout)),
            Err(e) => format!("lsof: {e}"),
        }
    }

    /// A number listed when the «still running» screen opened gets a signal only if it still
    /// runs from the program folder: by the click it may be another program's. Nothing is
    /// signalled here.
    #[test]
    fn only_a_process_still_listed_is_ended() {
        assert_eq!(system::still_listed(&[1, 2, 3], &[2, 3, 4]), [2, 3]);
        assert!(system::still_listed(&[1, 2], &[]).is_empty(), "all gone");
        assert!(system::still_listed(&[], &[1]).is_empty(), "never offered");
        // This test's own process really runs, but not from that folder.
        let nowhere = std::env::temp_dir().join("evolite-nowhere");
        let now = system::processes_in_all(&[nowhere]);
        assert!(system::still_listed(&[std::process::id()], &now).is_empty());
    }

    /// «Close evolite» ends only processes started from the program folder: found by their
    /// full path, never by a name alone.
    #[test]
    fn a_running_program_is_found_by_its_full_path() {
        let me = std::env::current_exe().unwrap();
        assert!(system::processes_of(&me).contains(&std::process::id()));
        // macOS only: its kernel names the resolved path, so `processes_of` resolves the one it
        // is given; Linux compares the path as given.
        #[cfg(target_os = "macos")]
        {
            let link = std::env::temp_dir().join(format!("evolite-link-{}", std::process::id()));
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(&me, &link).unwrap();
            let by_link = system::processes_of(&link);
            let _ = std::fs::remove_file(&link);
            assert!(by_link.contains(&std::process::id()), "through a link");
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
        // For the whole test: on macOS `end_process` starts `ps` as well.
        let _no_starts = no_process_starts();
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

    /// BUG-246: the running uninstaller and then its empty folder go once it has exited, even
    /// when it ran on for a while after the removal asked (its last screen open). A copy of
    /// `ping.exe` running for about 4 s stands in for it. Review r1: in a folder whose name
    /// `cmd` would expand (`%i`, `%TEMP%`, `!x!`, `&`, `^`, brackets), and the folders such an
    /// expansion would name instead are left alone.
    #[cfg(windows)]
    #[test]
    fn the_running_uninstaller_and_its_folder_go_after_it_exits() {
        use std::time::{Duration, Instant};
        let base = std::env::temp_dir().join(format!("evolite-bug246-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let name = "a %i %TEMP% !x! & ^ (b)";
        let dir = base.join("Programs").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        // What `%i`, an empty `!x!`, a cut at `&` or a dropped `^` would make of the name.
        let decoys: Vec<_> = [
            "a 1 %TEMP% !x! & ^ (b)",
            "a %i %TEMP%  & ^ (b)",
            // A cut at `&` (Windows drops the trailing space of a folder's name).
            "a %i %TEMP% !x!",
            "a",
        ]
        .iter()
        .map(|d| {
            let d = base.join("Programs").join(d);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(plan::UNINSTALLER), b"decoy").unwrap();
            d
        })
        .collect();
        let exe = dir.join(plan::UNINSTALLER);
        let system32 = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        std::fs::copy(
            std::path::Path::new(&system32).join(r"System32\PING.EXE"),
            &exe,
        )
        .unwrap();
        let mut running = std::process::Command::new(&exe)
            .args(["-n", "5", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        system::delete_after_exit(&exe);
        std::thread::sleep(Duration::from_secs(2));
        assert!(exe.exists(), "still running: still there");
        running.wait().unwrap();
        let start = Instant::now();
        while dir.exists() && start.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(!exe.exists(), "the uninstaller is gone");
        assert!(!dir.exists(), "and its empty folder");
        assert!(
            base.join("Programs").exists(),
            "the folder above is not ours"
        );
        for d in &decoys {
            assert!(
                d.join(plan::UNINSTALLER).exists(),
                "{} left alone",
                d.display()
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Review r1: no path goes into `cmd`'s line: it is fixed, the paths only by the names of
    /// variables of its own environment, read by delayed expansion, AutoRun skipped.
    #[cfg(windows)]
    #[test]
    fn no_path_is_in_the_command_line() {
        let line = system::AFTER_EXIT;
        assert!(line.starts_with("/D /V:ON /C "), "{line}");
        assert!(
            !line.replace("%i", "").contains('%'),
            "no %…% but the loop's own %i: {line}"
        );
        assert!(
            // No folder separator, no drive (`/V:ON` is cmd's own switch).
            !line.contains('\\') && !line.replace("/V:ON", "").contains(':'),
            "no path: {line}"
        );
        assert!(line.contains("!EVOLITE_SELF!") && line.contains("!EVOLITE_DIR!"));
    }
}
