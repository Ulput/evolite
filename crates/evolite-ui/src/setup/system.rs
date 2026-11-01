//! The system's side of the installer: the user's known folders, the local disks and their free
//! room, what the uninstaller reads back (the «Installed apps» entry on Windows,
//! `uninstall.json` on Linux and macOS), shortcuts, the folder picker. Everything is per user:
//! `HKEY_CURRENT_USER`, the user's own folders — nothing needs an administrator or root, and
//! nothing outside the user's profile and the chosen program folder is touched.

use super::plan::{Disk, Known};
use std::path::{Path, PathBuf};

/// The «Installed apps» entry: `HKCU\…\Uninstall\evolite`.
#[cfg(windows)]
pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\evolite";

/// What goes into the «Installed apps» entry.
pub struct Entry<'a> {
    pub dir: &'a Path,
    pub version: &'a str,
    pub size: u64,
    /// The installer's language, for the uninstaller (`ru`, `ar`, …).
    pub language: &'a str,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, IPersistFile,
    };
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_LocalAppData, FOLDERID_Programs, FOLDERID_RoamingAppData,
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
        IShellLinkW, KF_FLAG_DEFAULT, SHCreateItemFromParsingName, SIGDN_FILESYSPATH, ShellLink,
    };
    use windows::core::{GUID, HSTRING, Interface, PCWSTR};
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW,
    };
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn wide_path(p: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn com() {
        // SAFETY: called on the window's thread before any COM object is made; a second call
        // only adds a reference, a thread already in another mode keeps it.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    }

    fn known_folder(id: &GUID) -> Result<PathBuf, String> {
        // SAFETY: a known folder id; the returned string is freed with CoTaskMemFree.
        unsafe {
            let path = windows::Win32::UI::Shell::SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)
                .map_err(|e| e.to_string())?;
            let out = path.to_string().map_err(|e| e.to_string());
            CoTaskMemFree(Some(path.0 as *const _));
            out.map(PathBuf::from)
        }
    }

    pub fn known() -> Result<Known, String> {
        let local = known_folder(&FOLDERID_LocalAppData)?;
        Ok(Known {
            programs: local.join("Programs"),
            local,
            roaming: known_folder(&FOLDERID_RoamingAppData)?,
        })
    }

    pub fn start_menu() -> Result<PathBuf, String> {
        known_folder(&FOLDERID_Programs)
    }

    pub fn desktop() -> Result<PathBuf, String> {
        known_folder(&FOLDERID_Desktop)
    }

    /// The fixed disks, with their label and free room (what this user may still write).
    pub fn disks() -> Vec<Disk> {
        let mut buf = [0u16; 512];
        // SAFETY: the buffer is as long as the length passed.
        let len = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) } as usize;
        let mut out = Vec::new();
        for root in buf[..len.min(buf.len())]
            .split(|c| *c == 0)
            .filter(|r| !r.is_empty())
        {
            let mut z = root.to_vec();
            z.push(0);
            // SAFETY: `z` is a NUL-terminated root such as "C:\".
            if unsafe { GetDriveTypeW(z.as_ptr()) } != 3 {
                continue; // DRIVE_FIXED only: no network, removable or optical drives.
            }
            let (mut free, mut total, mut all_free) = (0u64, 0u64, 0u64);
            // SAFETY: the out-pointers are valid u64s.
            if unsafe { GetDiskFreeSpaceExW(z.as_ptr(), &mut free, &mut total, &mut all_free) } == 0
            {
                continue;
            }
            let mut name = [0u16; 261];
            // SAFETY: the name buffer is as long as the size passed; the other outputs unused.
            let ok = unsafe {
                GetVolumeInformationW(
                    z.as_ptr(),
                    name.as_mut_ptr(),
                    name.len() as u32,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                )
            } != 0;
            let label = if ok {
                let end = name.iter().position(|c| *c == 0).unwrap_or(0);
                String::from_utf16_lossy(&name[..end])
            } else {
                String::new()
            };
            out.push(Disk {
                root: PathBuf::from(String::from_utf16_lossy(root)),
                label,
                free,
                total,
            });
        }
        out
    }

    fn set_string(key: HKEY, name: &str, value: &str) -> Result<(), String> {
        let v = wide(value);
        // SAFETY: an open key; the data is the value's UTF-16 with its NUL, length in bytes.
        let r = unsafe {
            RegSetValueExW(
                key,
                wide(name).as_ptr(),
                0,
                REG_SZ,
                v.as_ptr() as *const u8,
                (v.len() * 2) as u32,
            )
        };
        if r == 0 {
            Ok(())
        } else {
            Err(format!("registry {name}: {r}"))
        }
    }

    fn set_dword(key: HKEY, name: &str, value: u32) -> Result<(), String> {
        // SAFETY: an open key; four bytes of data.
        let r = unsafe {
            RegSetValueExW(
                key,
                wide(name).as_ptr(),
                0,
                REG_DWORD,
                &value as *const u32 as *const u8,
                4,
            )
        };
        if r == 0 {
            Ok(())
        } else {
            Err(format!("registry {name}: {r}"))
        }
    }

    pub fn write_entry(e: &Entry) -> Result<(), String> {
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: HKCU and a subkey of our own; the key is closed below.
        let r = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(UNINSTALL_KEY).as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        };
        if r != 0 {
            return Err(format!("registry: {r}"));
        }
        let uninstaller = e.dir.join(crate::plan::UNINSTALLER);
        let app = e.dir.join(crate::plan::APP);
        let result = (|| {
            set_string(key, "DisplayName", "evolite")?;
            set_string(key, "DisplayVersion", e.version)?;
            set_string(key, "Publisher", "evolite")?;
            set_string(key, "InstallLocation", &e.dir.to_string_lossy())?;
            set_string(key, "DisplayIcon", &app.to_string_lossy())?;
            set_string(
                key,
                "UninstallString",
                &format!("\"{}\" --uninstall", uninstaller.display()),
            )?;
            set_string(key, "Language", e.language)?;
            set_dword(key, "EstimatedSize", (e.size / 1024) as u32)?;
            set_dword(key, "NoModify", 1)?;
            set_dword(key, "NoRepair", 1)
        })();
        // SAFETY: the key opened above.
        unsafe { RegCloseKey(key) };
        result
    }

    pub fn remove_entry() -> Result<(), String> {
        // SAFETY: HKCU and our own subkey.
        let r = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide(UNINSTALL_KEY).as_ptr()) };
        // 2: not there — nothing to remove.
        if r == 0 || r == 2 {
            Ok(())
        } else {
            Err(format!("registry: {r}"))
        }
    }

    pub fn shortcut(lnk: &Path, target: &Path, description: &str) -> Result<(), String> {
        com();
        // SAFETY: COM is initialised on this thread; the objects are released when dropped.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
            link.SetPath(&HSTRING::from(target.as_os_str()))
                .map_err(|e| e.to_string())?;
            if let Some(dir) = target.parent() {
                link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))
                    .map_err(|e| e.to_string())?;
            }
            link.SetDescription(&HSTRING::from(description))
                .map_err(|e| e.to_string())?;
            link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)
                .map_err(|e| e.to_string())?;
            let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
            let path = wide_path(lnk);
            file.Save(PCWSTR(path.as_ptr()), true)
                .map_err(|e| e.to_string())
        }
    }

    pub fn pick_folder(start: &Path) -> Option<PathBuf> {
        com();
        // SAFETY: COM is initialised on this thread; the dialog is modal and released on drop.
        unsafe {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            dialog
                .SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
                .ok()?;
            let mut at = Some(start);
            while let Some(p) = at {
                if p.is_dir() {
                    if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                        &HSTRING::from(p.as_os_str()),
                        None,
                    ) {
                        let _ = dialog.SetFolder(&item);
                    }
                    break;
                }
                at = p.parent();
            }
            dialog.Show(None).ok()?;
            let item = dialog.GetResult().ok()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let out = name.to_string().ok();
            CoTaskMemFree(Some(name.0 as *const _));
            out.map(PathBuf::from)
        }
    }

    /// The program in `dir` runs (its executable cannot be opened for writing).
    pub fn in_use(exe: &Path) -> bool {
        exe.exists() && std::fs::OpenOptions::new().write(true).open(exe).is_err()
    }

    /// Deletes the uninstaller `exe` once this process has exited, then its folder if nothing
    /// else is left in it: a hidden `cmd` waits two seconds — a running executable cannot delete
    /// itself on Windows. `rmdir` without `/S` never removes a folder with anything in it.
    pub fn delete_after_exit(exe: &Path) {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let dir = exe
            .parent()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        let _ = std::process::Command::new("cmd.exe")
            .arg("/C")
            .raw_arg(format!(
                "ping -n 3 127.0.0.1 >NUL & del /F /Q \"{}\" & rmdir \"{dir}\"",
                exe.display()
            ))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    /// A string value of the «Installed apps» entry; `None` when there is no entry or no value.
    fn entry_value(name: &str) -> Option<String> {
        use windows_sys::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: the buffer and its length in bytes; a string value of our own key.
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                wide(UNINSTALL_KEY).as_ptr(),
                wide(name).as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut _,
                &mut len,
            )
        };
        if r != 0 {
            return None;
        }
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }

    /// The language the installer ran in, kept in the «Installed apps» entry for the uninstaller.
    pub fn installed_language() -> Option<String> {
        entry_value("Language")
    }

    /// evolite as installed: where the «Installed apps» entry says, with its version; without
    /// an entry (removed by a cleaner, or a run that stopped half way) the default folder if
    /// the program is there.
    pub fn installed(known: &Known) -> Option<crate::plan::Installed> {
        use crate::plan::{APP, Installed};
        if let Some(dir) = entry_value("InstallLocation").filter(|d| !d.trim().is_empty()) {
            return Some(Installed::at(
                PathBuf::from(dir),
                entry_value("DisplayVersion"),
            ));
        }
        let dir = known.default_program();
        dir.join(APP).is_file().then(|| Installed::at(dir, None))
    }

    /// The processes of this user running `exe` (compared by full path).
    pub fn processes_of(exe: &Path) -> Vec<u32> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        };
        let want = exe.to_string_lossy().to_lowercase();
        let name = exe
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let mut found = Vec::new();
        // SAFETY: a snapshot handle closed below; the entry's size is set before the walk, and
        // each process handle is closed after its name is read.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return found;
            }
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snap, &mut e) != 0;
            while more {
                let end = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(0);
                let short = String::from_utf16_lossy(&e.szExeFile[..end]).to_lowercase();
                if short == name {
                    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, e.th32ProcessID);
                    if !h.is_null() {
                        let mut buf = [0u16; 1024];
                        let mut len = buf.len() as u32;
                        if QueryFullProcessImageNameW(
                            h,
                            PROCESS_NAME_WIN32,
                            buf.as_mut_ptr(),
                            &mut len,
                        ) != 0
                            && String::from_utf16_lossy(&buf[..len as usize]).to_lowercase() == want
                        {
                            found.push(e.th32ProcessID);
                        }
                        CloseHandle(h);
                    }
                }
                more = Process32NextW(snap, &mut e) != 0;
            }
            CloseHandle(snap);
        }
        found
    }

    /// Ends the process `pid` (a hung evolite); true when it is gone.
    pub fn end_process(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
            WaitForSingleObject,
        };
        // SAFETY: a handle of this user's process, closed after waiting up to five seconds.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return false;
            }
            let ended =
                TerminateProcess(h, 1) != 0 && WaitForSingleObject(h, 5000) == WAIT_OBJECT_0;
            CloseHandle(h);
            ended
        }
    }

    /// A message in a plain system window: shown even when the graphics driver cannot draw the
    /// installer's own.
    pub fn message_box(title: &str, text: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        // SAFETY: NUL-terminated wide strings that outlive the call; no owner window.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                wide(text).as_ptr(),
                wide(title).as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

#[cfg(windows)]
mod links {
    //! The Start menu and desktop shortcuts.
    use super::*;

    pub fn add(dir: &Path, desktop: bool, _known: &Known, _language: &str) -> Result<(), String> {
        let app = dir.join(crate::plan::APP);
        imp::shortcut(&imp::start_menu()?.join("evolite.lnk"), &app, "evolite")?;
        if desktop {
            imp::shortcut(&imp::desktop()?.join("evolite.lnk"), &app, "evolite")?;
        }
        Ok(())
    }

    pub fn remove(_known: &Known) {
        for folder in [imp::start_menu(), imp::desktop()].into_iter().flatten() {
            let _ = std::fs::remove_file(folder.join("evolite.lnk"));
        }
    }
}

#[cfg(unix)]
mod imp {
    //! Linux and macOS: the user's home folder, the mounted disks, `uninstall.json` beside the
    //! program instead of a registry entry. Nothing here needs root.
    use super::*;

    fn home() -> Result<PathBuf, String> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|h| h.is_absolute())
            .ok_or_else(|| "HOME is not set".to_owned())
    }

    pub fn known() -> Result<Known, String> {
        let var = |k: &str| {
            std::env::var_os(k)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Ok(Known::of_home(
            &home()?,
            var("XDG_DATA_HOME"),
            var("XDG_CACHE_HOME"),
        ))
    }

    /// The user's desktop folder: `XDG_DESKTOP_DIR` from `user-dirs.dirs` (its name is
    /// translated on many systems: `~/Schreibtisch`), else `~/Desktop`.
    pub fn desktop() -> Result<PathBuf, String> {
        let home = home()?;
        if cfg!(target_os = "linux") {
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"));
            if let Ok(dirs) = std::fs::read_to_string(config.join("user-dirs.dirs")) {
                for line in dirs.lines() {
                    if let Some(v) = line.trim().strip_prefix("XDG_DESKTOP_DIR=") {
                        let v = v.trim_matches('"');
                        let path = match v.strip_prefix("$HOME") {
                            Some(rest) => home.join(rest.trim_start_matches('/')),
                            None => PathBuf::from(v),
                        };
                        if path.is_absolute() && path != home {
                            return Ok(path);
                        }
                    }
                }
            }
        }
        Ok(home.join("Desktop"))
    }

    fn statvfs(path: &Path) -> Option<(u64, u64)> {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: a NUL-terminated path and a zeroed structure the call fills.
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
            return None;
        }
        let unit = s.f_frsize as u64;
        // Free for this user (not root's reserve) and the size.
        Some((s.f_bavail as u64 * unit, s.f_blocks as u64 * unit))
    }

    /// The local disks this user may install to: Linux — real file systems from
    /// `/proc/self/mounts` (no system, network or snap mounts; one entry per device); macOS —
    /// local volumes the Finder shows. The home folder's disk is always among them.
    pub fn disks() -> Vec<Disk> {
        let mut out = Vec::new();
        #[cfg(target_os = "linux")]
        {
            const LOCAL: &[&str] = &[
                "ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "zfs", "jfs", "reiserfs",
                "bcachefs", "ntfs", "ntfs3", "fuseblk", "exfat", "vfat",
            ];
            let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
            let mut devices = Vec::new();
            for line in mounts.lines() {
                let mut f = line.split_whitespace();
                let (Some(dev), Some(at), Some(fs)) = (f.next(), f.next(), f.next()) else {
                    continue;
                };
                // The table writes a space in a path as \040.
                let at = at.replace("\\040", " ");
                if !LOCAL.contains(&fs)
                    || !dev.starts_with("/dev/")
                    || at.starts_with("/boot")
                    || at.starts_with("/snap")
                    || devices.contains(&dev)
                {
                    continue;
                }
                let Some((free, total)) = statvfs(Path::new(&at)) else {
                    continue;
                };
                devices.push(dev);
                let label = Path::new(&at)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push(Disk {
                    root: PathBuf::from(&at),
                    label,
                    free,
                    total,
                });
            }
        }
        #[cfg(target_os = "macos")]
        {
            let mut list: *mut libc::statfs = std::ptr::null_mut();
            // SAFETY: getmntinfo points `list` at its own array of `n` entries.
            let n = unsafe { libc::getmntinfo(&mut list, libc::MNT_NOWAIT) };
            for i in 0..n.max(0) as usize {
                // SAFETY: `i` is below the count the call returned.
                let s = unsafe { &*list.add(i) };
                let flags = s.f_flags as u32;
                if flags & libc::MNT_LOCAL as u32 == 0 || flags & libc::MNT_DONTBROWSE as u32 != 0 {
                    continue;
                }
                // SAFETY: the kernel writes NUL-terminated names.
                let at = unsafe { std::ffi::CStr::from_ptr(s.f_mntonname.as_ptr()) }
                    .to_string_lossy()
                    .into_owned();
                let Some((free, total)) = statvfs(Path::new(&at)) else {
                    continue;
                };
                let label = Path::new(&at)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push(Disk {
                    root: PathBuf::from(&at),
                    label,
                    free,
                    total,
                });
            }
        }
        // A container or an unusual system: the disk of the home folder all the same.
        if let Ok(home) = home()
            && crate::plan::disk_of(&out, &home).is_none()
            && let Some((free, total)) = statvfs(Path::new("/"))
        {
            out.push(Disk {
                root: PathBuf::from("/"),
                label: String::new(),
                free,
                total,
            });
        }
        out
    }

    /// What the uninstaller reads back: the language and version, in `uninstall.json`.
    pub fn write_entry(e: &Entry) -> Result<(), String> {
        let record = serde_json::json!({
            "version": e.version,
            "language": e.language,
            "size": e.size,
        });
        let path = e.dir.join(crate::plan::RECORD);
        std::fs::write(&path, format!("{record:#}\n"))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The record goes with the program's files.
    pub fn remove_entry() -> Result<(), String> {
        Ok(())
    }

    fn record(dir: &Path, field: &str) -> Option<String> {
        let bytes = std::fs::read(dir.join(crate::plan::RECORD)).ok()?;
        let record: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        record[field].as_str().map(str::to_owned)
    }

    pub fn installed_language(dir: &Path) -> Option<String> {
        record(dir, "language")
    }

    /// evolite in the default program folder, with the version its record names.
    pub fn installed(known: &Known) -> Option<crate::plan::Installed> {
        let dir = known.default_program();
        (dir.join(crate::plan::APP).is_file() || dir.join(crate::plan::RECORD).is_file()).then(
            || {
                let version = record(&dir, "version");
                crate::plan::Installed::at(dir, version)
            },
        )
    }

    /// The processes of this user running `exe`: Linux reads `/proc/<pid>/exe`; on macOS the
    /// data folder's lock alone tells that evolite runs.
    pub fn processes_of(exe: &Path) -> Vec<u32> {
        let mut found = Vec::new();
        if cfg!(target_os = "linux") {
            for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
                let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
                    continue;
                };
                if std::fs::read_link(entry.path().join("exe")).is_ok_and(|p| p == exe) {
                    found.push(pid);
                }
            }
        }
        found
    }

    /// Ends the process `pid` (a hung evolite); true when it is gone.
    pub fn end_process(pid: u32) -> bool {
        // SAFETY: a signal to a process of this user's; nothing else is touched.
        if unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } != 0 {
            return false;
        }
        // Ended but not yet collected by its parent (a zombie, state `Z`): gone all the same.
        let zombie = || {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|s| {
                    s.rsplit_once(')')
                        .map(|(_, rest)| rest.trim_start().starts_with('Z'))
                })
                .unwrap_or(false)
        };
        for _ in 0..50 {
            // SAFETY: signal 0 only asks whether the process is there.
            if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 || zombie() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    }

    /// A message in the system's own dialog (zenity or kdialog, AppleScript on macOS): shown
    /// even when the installer's window cannot be drawn. The text goes as an argument.
    pub fn message_box(title: &str, text: &str) {
        if cfg!(target_os = "macos") {
            let _ = std::process::Command::new("osascript")
                .args([
                    "-e",
                    "on run argv",
                    "-e",
                    "display alert (item 1 of argv) message (item 2 of argv) as critical",
                    "-e",
                    "end run",
                    title,
                    text,
                ])
                .status();
        } else {
            let shown = std::process::Command::new("zenity")
                .args(["--error", "--title", title, "--text", text])
                .status()
                .is_ok();
            if !shown {
                let _ = std::process::Command::new("kdialog")
                    .args(["--title", title, "--error", text])
                    .status();
            }
        }
    }

    /// A folder chosen in the system's own dialog: zenity (GNOME and most others) or kdialog
    /// (KDE) on Linux, AppleScript's `choose folder` on macOS. The start folder is passed as an
    /// argument, never pasted into a command. `None` when cancelled or there is no dialog.
    pub fn pick_folder(start: &Path) -> Option<PathBuf> {
        let start = start
            .ancestors()
            .find(|p| p.is_dir())
            .unwrap_or(Path::new("/"));
        let run = |program: &str, args: &[&std::ffi::OsStr]| {
            let out = std::process::Command::new(program)
                .args(args)
                .output()
                .ok()?;
            let chosen = String::from_utf8(out.stdout).ok()?;
            let chosen = chosen.trim_end_matches(['\n', '\r']);
            (out.status.success() && !chosen.is_empty()).then(|| PathBuf::from(chosen))
        };
        let os = |s: &'static str| std::ffi::OsStr::new(s);
        if cfg!(target_os = "macos") {
            run(
                "osascript",
                &[
                    os("-e"),
                    os("on run argv"),
                    os("-e"),
                    os("POSIX path of (choose folder default location (POSIX file (item 1 of argv)))"),
                    os("-e"),
                    os("end run"),
                    start.as_os_str(),
                ],
            )
            .map(|p| PathBuf::from(p.to_string_lossy().trim_end_matches('/')))
        } else {
            let mut with_slash = start.as_os_str().to_owned();
            with_slash.push("/");
            run(
                "zenity",
                &[
                    os("--file-selection"),
                    os("--directory"),
                    os("--filename"),
                    &with_slash,
                ],
            )
            .or_else(|| {
                run(
                    "kdialog",
                    &[os("--getexistingdirectory"), start.as_os_str()],
                )
            })
        }
    }

    /// Whether the program in its folder runs is told by its data folder's lock
    /// ([`super::running`]); a running file can be replaced here.
    pub fn in_use(_exe: &Path) -> bool {
        false
    }

    /// A running program can delete its own file on Linux and macOS: at once.
    pub fn delete_after_exit(exe: &Path) {
        let _ = std::fs::remove_file(exe);
    }
}

#[cfg(target_os = "linux")]
mod links {
    //! The applications menu entries (`~/.local/share/applications`: the app and its removal),
    //! and the desktop one if chosen.
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const APP: &str = "evolite.desktop";
    const REMOVE: &str = "evolite-uninstall.desktop";

    fn menu(known: &Known) -> PathBuf {
        known.roaming.join("applications")
    }

    /// One argument of `Exec`, quoted as the desktop entry spec asks: `"`, `` ` ``, `$` and
    /// `\` escaped inside the quotes (the backslash then once more for the string), `%`
    /// doubled. A path with a line break in it cannot be written.
    pub fn exec_arg(path: &Path) -> Result<String, String> {
        let s = path.to_string_lossy();
        if s.chars().any(char::is_control) {
            return Err(format!("a control character in {s:?}"));
        }
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' | '`' | '$' => {
                    out.push_str("\\\\");
                    out.push(c);
                }
                '\\' => out.push_str("\\\\\\\\"),
                '%' => out.push_str("%%"),
                c => out.push(c),
            }
        }
        out.push('"');
        Ok(out)
    }

    pub fn entry(name: &str, exec: &Path, icon: &Path) -> Result<String, String> {
        let icon = icon.to_string_lossy();
        if icon.chars().any(char::is_control) || name.chars().any(char::is_control) {
            return Err("a control character in the entry".into());
        }
        Ok(format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={}\nIcon={icon}\nTerminal=false\n\
             Categories=Finance;Network;\nStartupWMClass=evolite\n",
            exec_arg(exec)?
        ))
    }

    fn write(path: &Path, text: &str) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        // Desktops start only entries that may be executed.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())
    }

    pub fn add(dir: &Path, desktop: bool, known: &Known, language: &str) -> Result<(), String> {
        let icon = dir.join(crate::plan::ICON);
        let app = entry("evolite", &dir.join(crate::plan::APP), &icon)?;
        let lang = crate::i18n::Lang::from_code(language).unwrap_or(crate::i18n::Lang::En);
        let remove = entry(
            &crate::i18n::t_in(lang, tr!("Remove evolite")),
            &dir.join(crate::plan::UNINSTALLER),
            &icon,
        )?;
        write(&menu(known).join(APP), &app)?;
        write(&menu(known).join(REMOVE), &remove)?;
        if desktop {
            let file = imp::desktop()?.join(APP);
            write(&file, &app)?;
            // GNOME asks before starting a desktop file it was not told to trust.
            let _ = std::process::Command::new("gio")
                .args(["set", "-t", "string"])
                .arg(&file)
                .args(["metadata::trusted", "true"])
                .output();
        }
        Ok(())
    }

    pub fn remove(known: &Known) {
        let _ = std::fs::remove_file(menu(known).join(APP));
        let _ = std::fs::remove_file(menu(known).join(REMOVE));
        if let Ok(desktop) = imp::desktop() {
            let _ = std::fs::remove_file(desktop.join(APP));
        }
    }
}

#[cfg(target_os = "macos")]
mod links {
    //! Apps in `~/Applications` are in Launchpad by themselves; the desktop gets a link to the
    //! app if chosen.
    use super::*;

    pub fn add(dir: &Path, desktop: bool, _known: &Known, _language: &str) -> Result<(), String> {
        if desktop {
            let link = imp::desktop()?.join("evolite");
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(dir.join("evolite.app"), &link)
                .map_err(|e| format!("{}: {e}", link.display()))?;
        }
        Ok(())
    }

    /// Only a link of that name: a file or folder the person made there stays.
    pub fn remove(_known: &Known) {
        if let Ok(desktop) = imp::desktop() {
            let link = desktop.join("evolite");
            if std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) {
                let _ = std::fs::remove_file(link);
            }
        }
    }
}

#[cfg(windows)]
pub fn installed_language(_dir: &Path) -> Option<String> {
    imp::installed_language()
}

#[cfg(unix)]
pub use imp::installed_language;
#[cfg(all(windows, test))]
pub use imp::start_menu;
pub use imp::{
    delete_after_exit, disks, end_process, in_use, installed, known, message_box, pick_folder,
    processes_of, remove_entry, write_entry,
};

/// The evolite processes started from the program folder `dir` (the app and its picture
/// helper): what «Close evolite» ends. Programs from other folders are never touched.
pub fn processes_in(dir: &Path) -> Vec<u32> {
    let mut found = processes_of(&dir.join(crate::plan::APP));
    found.extend(processes_of(&dir.join(crate::plan::RENDERER)));
    found
}
/// The shortcuts: Start menu (Windows), applications menu (Linux); the desktop if chosen.
pub use links::{add as add_shortcuts, remove as remove_shortcuts};

/// Whether evolite runs: it holds its data folder's lock (`evolite.lock`) while it does — on
/// every system, whichever folder it was installed to.
pub fn running(known: &Known) -> bool {
    known.data_folders().iter().any(|data| {
        std::fs::OpenOptions::new()
            .write(true)
            .open(data.join("evolite.lock"))
            .is_ok_and(|f| match f.try_lock() {
                Ok(()) => {
                    let _ = f.unlock();
                    false
                }
                Err(std::fs::TryLockError::WouldBlock) => true,
                Err(std::fs::TryLockError::Error(_)) => false,
            })
    })
}
