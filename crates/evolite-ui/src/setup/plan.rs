//! What the installer and the uninstaller decide, without touching the system: how much room
//! evolite needs and where, which folder it goes to, what is removed and which wallets a data
//! folder holds, and where each file goes on each system. The system's side
//! ([`super::system`]) only reads disks and folders and acts.

use std::path::{Path, PathBuf};

/// The program as the payload carries it: the app and its picture helper.
pub const APP_NAME: &str = if cfg!(windows) {
    "evolite.exe"
} else {
    "evolite"
};
pub const RENDERER_NAME: &str = if cfg!(windows) {
    "evolite-renderer.exe"
} else {
    "evolite-renderer"
};

/// Where each file goes in the program folder. Windows and Linux: side by side. macOS: the app
/// is a bundle, `evolite.app`, and the uninstaller another, `Uninstall evolite.app`, both in the
/// program folder — the renderer stays next to the app, where it looks for it.
#[cfg(not(target_os = "macos"))]
mod layout {
    pub const APP: &str = super::APP_NAME;
    pub const RENDERER: &str = super::RENDERER_NAME;
    pub const UNINSTALLER: &str = if cfg!(windows) {
        "uninstall.exe"
    } else {
        "uninstall"
    };
    /// Linux: the icon the menu entry points to; the language for the uninstaller.
    #[cfg(not(windows))]
    pub const EXTRA: &[&str] = &[super::ICON, super::RECORD];
    #[cfg(windows)]
    pub const EXTRA: &[&str] = &[];
    pub const BUNDLES: &[&str] = &[];
}
#[cfg(target_os = "macos")]
mod layout {
    pub const APP: &str = "evolite.app/Contents/MacOS/evolite";
    pub const RENDERER: &str = "evolite.app/Contents/MacOS/evolite-renderer";
    pub const UNINSTALLER: &str = "Uninstall evolite.app/Contents/MacOS/uninstall";
    pub const EXTRA: &[&str] = &[
        "evolite.app/Contents/Info.plist",
        "evolite.app/Contents/Resources/evolite.icns",
        "Uninstall evolite.app/Contents/Info.plist",
        "Uninstall evolite.app/Contents/Resources/evolite.icns",
        super::RECORD,
    ];
    /// Deleted whole after their files: what macOS adds to a bundle (a signature, a cache).
    pub const BUNDLES: &[&str] = &["evolite.app", "Uninstall evolite.app"];
}
pub use layout::{APP, BUNDLES, EXTRA, RENDERER, UNINSTALLER};

/// The app's icon beside it (Linux menu entries).
#[cfg(not(any(windows, target_os = "macos")))]
pub const ICON: &str = "evolite.png";
/// The installed version and the installer's language, for the uninstaller (Linux and macOS;
/// Windows keeps them in the «Installed apps» entry).
#[cfg(not(windows))]
pub const RECORD: &str = "uninstall.json";

/// The program's files, the only ones the uninstaller removes from the program folder.
pub fn program_files() -> Vec<&'static str> {
    let mut files = vec![APP, RENDERER, UNINSTALLER];
    files.extend_from_slice(EXTRA);
    files
}

/// Where a file the payload carries goes in the program folder; `None` for one it should not.
pub fn target_of(name: &str) -> Option<&'static str> {
    match name {
        n if n == APP_NAME => Some(APP),
        n if n == RENDERER_NAME => Some(RENDERER),
        _ => None,
    }
}

/// The program folder of the running uninstaller `exe`: as many levels up as [`UNINSTALLER`]
/// is deep.
pub fn program_dir_of(exe: &Path) -> Option<PathBuf> {
    let depth = Path::new(UNINSTALLER).components().count();
    exe.ancestors().nth(depth).map(Path::to_path_buf)
}

/// Whether the running program is the uninstaller: started with `--uninstall` (the Windows
/// entry does) or named so (a Linux menu entry, macOS's `Uninstall evolite.app`).
pub fn is_uninstaller(exe: &Path) -> bool {
    exe.file_stem()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("uninstall"))
}

/// The program folder's own name: an install always ends in it.
pub const FOLDER: &str = "evolite";

/// A macOS bundle's `Info.plist`: its executable, a name and an identifier of its own, the icon.
#[cfg(any(target_os = "macos", test))]
pub fn info_plist(executable: &str, name: &str, id: &str, version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>{executable}</string>
  <key>CFBundleIdentifier</key><string>{id}</string>
  <key>CFBundleName</key><string>{name}</string>
  <key>CFBundleDisplayName</key><string>{name}</string>
  <key>CFBundleIconFile</key><string>evolite</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#
    )
}

/// An `.icns` holding one 256-pixel PNG (`ic08`): macOS reads PNG entries as they are.
#[cfg(any(target_os = "macos", test))]
pub fn icns(png: &[u8]) -> Vec<u8> {
    let entry = 8 + png.len() as u32;
    let mut out = Vec::with_capacity(8 + entry as usize);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(8 + entry).to_be_bytes());
    out.extend_from_slice(b"ic08");
    out.extend_from_slice(&entry.to_be_bytes());
    out.extend_from_slice(png);
    out
}

/// Testnet data after a full sync, measured on 2026-11-01 at height 1.58 million: headers
/// 171 MB, filter headers 49 MB, filters 63 MB, the wallet's blocks 67 MB, masternode lists
/// 1 MB, the wallet's store 4 MB — rounded up to 360 MiB. It grows by about 0.3 MB a day.
pub const TESTNET_SYNCED: u64 = 360 << 20;
/// Mainnet data after a full sync, estimated: headers measured at 255 MB (2.37 million
/// blocks), filter headers 32 bytes a block (~77 MB), filters and the wallet's blocks as on
/// testnet but for busier blocks (~0.6 GB) — about 1 GiB.
pub const MAINNET_SYNCED: u64 = 1 << 30;
/// What is recommended to keep free on the data disk for testnet: its data after the sync and
/// about 0.65 GiB more for its growth and the system.
pub const RECOMMENDED_FREE: u64 = 1 << 30;
/// The same with mainnet turned on: its data after the sync on top, with the same headroom.
pub const RECOMMENDED_FREE_MAINNET: u64 = RECOMMENDED_FREE + MAINNET_SYNCED;
/// What the installer itself needs beyond the files: the `.part` copies while it writes.
const MARGIN: u64 = 64 << 20;

/// Where the program goes, with the folder the person chose: inside an `evolite` folder there,
/// unless the chosen one already is one.
pub fn install_dir(chosen: &Path) -> PathBuf {
    if chosen
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(FOLDER))
    {
        chosen.to_path_buf()
    } else {
        chosen.join(FOLDER)
    }
}

/// A disk as the space screen shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Disk {
    /// `C:\`.
    pub root: PathBuf,
    pub label: String,
    pub free: u64,
    pub total: u64,
}

impl Disk {
    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (1.0 - self.free as f64 / self.total as f64).clamp(0.0, 1.0) as f32
    }
}

/// The disk `path` is on: the longest root it is inside, by whole folder names (`/home2` is not
/// on `/home`). Windows' drive letters in either case.
pub fn disk_of<'a>(disks: &'a [Disk], path: &Path) -> Option<&'a Disk> {
    let fold = |p: &Path| {
        if cfg!(windows) || p.to_string_lossy().contains('\\') {
            PathBuf::from(p.to_string_lossy().to_ascii_lowercase())
        } else {
            p.to_path_buf()
        }
    };
    let path = fold(path);
    disks
        .iter()
        .filter(|d| path.starts_with(fold(&d.root)))
        .max_by_key(|d| d.root.as_os_str().len())
}

/// Whether evolite fits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Room {
    /// Enough, and the recommended room left.
    Plenty,
    /// It fits, but less than the recommended room is left on the data disk.
    Tight,
    /// It does not fit: the program, or the testnet data after the sync, would not.
    TooLittle,
}

/// How much room is needed where: the program on its disk, the testnet data after the sync on
/// the data disk (the same disk when they are one).
pub fn room(program: u64, program_disk: &Disk, data_disk: &Disk) -> Room {
    let same = program_disk.root == data_disk.root;
    let on_data = TESTNET_SYNCED + MARGIN + if same { program } else { 0 };
    if program_disk.free < program + MARGIN || data_disk.free < on_data {
        Room::TooLittle
    } else if data_disk.free < on_data + RECOMMENDED_FREE {
        Room::Tight
    } else {
        Room::Plenty
    }
}

/// The folders of evolite's own, under the user's known folders — the only ones the
/// uninstaller deletes whole. Nothing here comes from what was typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Known {
    /// Where evolite keeps its picture cache: `%LOCALAPPDATA%`; `~/.cache`
    /// (`$XDG_CACHE_HOME`); `~/Library/Caches`.
    pub local: PathBuf,
    /// Where it keeps its data: `%APPDATA%`; `~/.local/share` (`$XDG_DATA_HOME`);
    /// `~/Library/Application Support` — as the app itself finds them (`cli.rs`).
    pub roaming: PathBuf,
    /// Where programs of this user's go: `%LOCALAPPDATA%\Programs`; `~/.local/opt`;
    /// `~/Applications`.
    pub programs: PathBuf,
}

impl Known {
    /// This user's folders on Linux and macOS, from the home folder and the XDG variables, as
    /// evolite's own `cli.rs` reads them (it honours the XDG ones on macOS too).
    #[cfg(not(windows))]
    pub fn of_home(home: &Path, xdg_data: Option<PathBuf>, xdg_cache: Option<PathBuf>) -> Known {
        let mac = cfg!(target_os = "macos");
        Known {
            local: xdg_cache.unwrap_or_else(|| {
                if mac {
                    home.join("Library").join("Caches")
                } else {
                    home.join(".cache")
                }
            }),
            roaming: xdg_data.unwrap_or_else(|| {
                if mac {
                    home.join("Library").join("Application Support")
                } else {
                    home.join(".local").join("share")
                }
            }),
            programs: if mac {
                home.join("Applications")
            } else {
                home.join(".local").join("opt")
            },
        }
    }

    /// Testnet's data folder: wallets, store, settings, journal, headers and filters.
    pub fn testnet_data(&self) -> PathBuf {
        self.roaming.join("evolite")
    }
    /// Mainnet's, kept apart.
    pub fn mainnet_data(&self) -> PathBuf {
        self.roaming.join("evolite-mainnet")
    }
    /// Pictures drawn by the renderer (NFTs, avatars).
    pub fn cache(&self) -> PathBuf {
        self.local.join("evolite")
    }
    /// The default program folder: per user, no administrator needed.
    pub fn default_program(&self) -> PathBuf {
        self.programs.join(FOLDER)
    }
    /// The folders deleted with «delete wallet data too».
    pub fn data_folders(&self) -> [PathBuf; 2] {
        [self.testnet_data(), self.mainnet_data()]
    }

    /// Whether `path` is one of evolite's own folders — a check before anything is deleted
    /// whole, so a mistake can never reach another folder.
    pub fn is_own_folder(&self, path: &Path) -> bool {
        [self.testnet_data(), self.mainnet_data(), self.cache()]
            .iter()
            .any(|own| own == path)
    }
}

/// A wallet found in a data folder, for the uninstaller's warning.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundWallet {
    /// Its name on this computer; empty when it was never given one.
    pub name: String,
    pub network: &'static str,
}

/// The wallets in a data folder: the index of each network (`wallets/<network>/wallets.json`),
/// and a wallet from before there were several (`wallets/<network>/wallet.json`). Only names
/// are read — no vault is opened.
pub fn wallets_in(data: &Path) -> Vec<FoundWallet> {
    let mut found = Vec::new();
    for network in ["testnet", "mainnet"] {
        let dir = data.join("wallets").join(network);
        if dir.join("wallet.json").is_file() {
            found.push(FoundWallet {
                name: String::new(),
                network,
            });
            continue;
        }
        let index: serde_json::Value = std::fs::read(dir.join("wallets.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let named: Vec<_> = index["wallets"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter(|w| {
                        w["id"]
                            .as_str()
                            .is_some_and(|id| dir.join(id).join("wallet.json").is_file())
                    })
                    .map(|w| FoundWallet {
                        name: w["name"].as_str().unwrap_or("").to_owned(),
                        network,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // A wallet folder the index does not list yet (a crash between the two writes).
        let listed = named.len();
        let folders = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("wallet.json").is_file())
            .count();
        found.extend(named);
        for _ in listed..folders {
            found.push(FoundWallet {
                name: String::new(),
                network,
            });
        }
    }
    found
}

/// The bytes under `path` (0 when it is not there).
pub fn size_of(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| size_of(&e.path()))
        .sum()
}

/// "73 MB", "1.2 GB", "118 GB": one figure after the point between one and ten gigabytes; the
/// interface language's decimal separator.
pub fn bytes(n: u64, decimal: char) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = n as f64 / MB;
    if mb < 1024.0 {
        format!("{} MB", mb.round().max(1.0) as u64)
    } else if mb < 10.0 * 1024.0 {
        format!("{:.1} GB", mb / 1024.0).replace('.', &decimal.to_string())
    } else {
        format!("{} GB", (mb / 1024.0).round() as u64)
    }
}

/// evolite as it is already installed on this computer.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    /// Its program folder.
    pub dir: PathBuf,
    /// The version the installer that put it there wrote down; `None` when nothing says.
    pub version: Option<String>,
    /// Both the program and its uninstaller are there (a crash or a cleaner may leave half).
    pub complete: bool,
}

impl Installed {
    /// What a program folder holds, with the version its record names.
    pub fn at(dir: PathBuf, version: Option<String>) -> Installed {
        let complete = dir.join(APP).is_file() && dir.join(UNINSTALLER).is_file();
        Installed {
            dir,
            version: version.filter(|v| !v.trim().is_empty()),
            complete,
        }
    }
}

/// How the installed version stands to the one this installer carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Against {
    Same,
    /// The installed one is older: this is an update.
    Older,
    /// The installed one is newer: installing goes back to an older version.
    Newer,
    /// The installed version is not known.
    Unknown,
}

/// `0.1.0`, `1.2`, `v2.0.1-beta`: the numbers before any suffix, compared one by one (missing
/// ones are 0); `None` for text without a number in front.
fn version_numbers(v: &str) -> Option<Vec<u64>> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+', ' ']).next().unwrap_or("");
    let parts: Option<Vec<u64>> = core.split('.').map(|n| n.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

pub fn against(installed: Option<&str>, this: &str) -> Against {
    let (Some(a), Some(b)) = (installed.and_then(version_numbers), version_numbers(this)) else {
        return Against::Unknown;
    };
    let len = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    for i in 0..len {
        match at(&a, i).cmp(&at(&b, i)) {
            std::cmp::Ordering::Less => return Against::Older,
            std::cmp::Ordering::Greater => return Against::Newer,
            std::cmp::Ordering::Equal => {}
        }
    }
    Against::Same
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_compared_by_their_numbers() {
        assert_eq!(against(Some("0.1.0"), "0.1.0"), Against::Same);
        assert_eq!(against(Some("0.1"), "0.1.0"), Against::Same);
        assert_eq!(against(Some("0.1.0"), "0.2.0"), Against::Older);
        assert_eq!(
            against(Some("0.10.0"), "0.9.3"),
            Against::Newer,
            "not as text"
        );
        assert_eq!(against(Some("v1.2.0-beta"), "1.1.9"), Against::Newer);
        assert_eq!(against(None, "0.1.0"), Against::Unknown);
        assert_eq!(against(Some("custom"), "0.1.0"), Against::Unknown);
    }

    #[test]
    fn a_half_left_installation_is_not_complete() {
        let dir =
            std::env::temp_dir().join(format!("evolite-plan-installed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let gone = Installed::at(dir.clone(), Some("0.1.0".into()));
        assert!(!gone.complete);
        std::fs::create_dir_all(dir.join(APP).parent().unwrap()).unwrap();
        std::fs::write(dir.join(APP), b"app").unwrap();
        assert!(!Installed::at(dir.clone(), None).complete, "no uninstaller");
        std::fs::create_dir_all(dir.join(UNINSTALLER).parent().unwrap()).unwrap();
        std::fs::write(dir.join(UNINSTALLER), b"stub").unwrap();
        let whole = Installed::at(dir.clone(), Some(" ".into()));
        assert!(whole.complete);
        assert_eq!(whole.version, None, "a blank version says nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn disk(root: &str, free_gb: f64) -> Disk {
        Disk {
            root: PathBuf::from(root),
            label: String::new(),
            free: (free_gb * (1u64 << 30) as f64) as u64,
            total: 500 << 30,
        }
    }

    // Windows paths: elsewhere `\` is part of a name, not a separator.
    #[cfg(windows)]
    #[test]
    fn evolite_goes_into_a_folder_of_its_own() {
        assert_eq!(
            install_dir(Path::new(r"D:\Apps")),
            PathBuf::from(r"D:\Apps\evolite")
        );
        assert_eq!(
            install_dir(Path::new(r"D:\Apps\Evolite")),
            PathBuf::from(r"D:\Apps\Evolite")
        );
    }

    #[test]
    fn the_data_disk_decides_whether_it_fits() {
        let app = 73 << 20;
        let (c, d) = (disk(r"C:\", 118.0), disk(r"D:\", 412.0));
        assert_eq!(room(app, &c, &c), Room::Plenty);
        // Little room on C: — a program folder on D: does not help, the data stays on C:.
        let small_c = disk(r"C:\", 0.3);
        assert_eq!(room(app, &d, &small_c), Room::TooLittle);
        assert_eq!(
            room(app, &disk(r"D:\", 0.05), &c),
            Room::TooLittle,
            "nor a full D:"
        );
        let tight = disk(r"C:\", 0.9);
        assert_eq!(
            room(app, &d, &tight),
            Room::Tight,
            "fits, less than recommended left"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_path_is_on_the_disk_whose_root_starts_it() {
        let disks = [disk(r"C:\", 1.0), disk(r"D:\", 1.0)];
        assert_eq!(
            disk_of(&disks, Path::new(r"d:\apps\evolite")).unwrap().root,
            PathBuf::from(r"D:\")
        );
        assert!(disk_of(&disks, Path::new(r"\\server\share")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_path_is_on_the_mount_it_is_inside() {
        let disks = [disk("/", 1.0), disk("/home", 1.0), disk("/media/data", 1.0)];
        let on = |p: &str| disk_of(&disks, Path::new(p)).unwrap().root.clone();
        assert_eq!(on("/home/anna/.local/opt"), PathBuf::from("/home"));
        assert_eq!(
            on("/home2/anna"),
            PathBuf::from("/"),
            "whole folder names only"
        );
        assert_eq!(on("/media/data/apps"), PathBuf::from("/media/data"));
        assert_eq!(on("/opt"), PathBuf::from("/"));
    }

    #[cfg(unix)]
    #[test]
    fn the_folders_are_the_apps_own() {
        let home = Path::new("/home/anna");
        let k = Known::of_home(home, None, None);
        let k2 = Known::of_home(
            home,
            Some(PathBuf::from("/data/share")),
            Some(PathBuf::from("/data/cache")),
        );
        if cfg!(target_os = "macos") {
            assert_eq!(
                k.testnet_data(),
                home.join("Library/Application Support/evolite")
            );
            assert_eq!(k.cache(), home.join("Library/Caches/evolite"));
            assert_eq!(k.default_program(), home.join("Applications/evolite"));
        } else {
            assert_eq!(k.testnet_data(), home.join(".local/share/evolite"));
            assert_eq!(k.mainnet_data(), home.join(".local/share/evolite-mainnet"));
            assert_eq!(k.cache(), home.join(".cache/evolite"));
            assert_eq!(k.default_program(), home.join(".local/opt/evolite"));
        }
        assert_eq!(k2.testnet_data(), PathBuf::from("/data/share/evolite"));
        assert_eq!(k2.cache(), PathBuf::from("/data/cache/evolite"));
    }

    #[test]
    fn the_uninstaller_finds_its_program_folder() {
        let dir = std::env::temp_dir().join("Apps").join(FOLDER);
        let exe = dir.join(UNINSTALLER);
        assert!(is_uninstaller(&exe));
        assert!(!is_uninstaller(&dir.join(APP)));
        assert_eq!(program_dir_of(&exe), Some(dir));
        assert_eq!(target_of(APP_NAME), Some(APP));
        assert_eq!(target_of(RENDERER_NAME), Some(RENDERER));
        assert_eq!(target_of("other"), None);
        // Every file stays inside the program folder.
        for f in program_files() {
            assert!(Path::new(f).is_relative() && !f.contains(".."), "{f}");
        }
    }

    #[test]
    fn only_evolites_own_folders_are_deleted_whole() {
        let base = std::env::temp_dir().join("Users").join("a");
        let k = Known {
            local: base.join("Local"),
            roaming: base.join("Roaming"),
            programs: base.join("Local").join("Programs"),
        };
        assert!(k.is_own_folder(&k.testnet_data()));
        assert!(k.is_own_folder(&k.mainnet_data()));
        assert!(k.is_own_folder(&k.cache()));
        for other in [
            &k.roaming,
            &k.local,
            &k.default_program(),
            &k.roaming.join("evolitex"),
        ] {
            assert!(!k.is_own_folder(other), "{other:?}");
        }
    }

    #[test]
    fn the_wallets_of_a_data_folder_are_named() {
        let data = std::env::temp_dir().join(format!("evolite-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let net = data.join("wallets").join("testnet");
        for id in ["aaaa0001", "bbbb0002", "cccc0003"] {
            std::fs::create_dir_all(net.join(id)).unwrap();
            std::fs::write(net.join(id).join("wallet.json"), b"{}").unwrap();
        }
        std::fs::write(
            net.join("wallets.json"),
            r#"{"wallets":[{"id":"aaaa0001","name":"Main"},{"id":"bbbb0002","name":"Savings"},{"id":"gone","name":"Deleted"}]}"#,
        )
        .unwrap();
        // Mainnet from before several wallets: one, unnamed.
        let main = data.join("wallets").join("mainnet");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::write(main.join("wallet.json"), b"{}").unwrap();
        let found = wallets_in(&data);
        let names: Vec<_> = found.iter().map(|w| (w.name.as_str(), w.network)).collect();
        assert_eq!(
            names,
            [
                ("Main", "testnet"),
                ("Savings", "testnet"),
                ("", "testnet"),
                ("", "mainnet")
            ]
        );
        assert!(wallets_in(&data.join("nothing")).is_empty());
        let _ = std::fs::remove_dir_all(&data);
    }

    /// macOS's bundle files: a plist naming the executable and the icon, an `.icns` that is a
    /// header and one PNG entry.
    #[test]
    fn the_macos_bundle_describes_itself() {
        let plist = info_plist(
            "uninstall",
            "Uninstall evolite",
            "org.evolite.uninstall",
            "0.1.0",
        );
        assert!(plist.contains("<key>CFBundleExecutable</key><string>uninstall</string>"));
        assert!(plist.contains("<key>CFBundleIconFile</key><string>evolite</string>"));
        assert!(plist.trim_end().ends_with("</plist>"));
        let png = b"\x89PNG\r\n\x1a\n...";
        let icon = icns(png);
        assert_eq!(&icon[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(icon[4..8].try_into().unwrap()) as usize,
            icon.len()
        );
        assert_eq!(&icon[8..12], b"ic08");
        assert_eq!(
            u32::from_be_bytes(icon[12..16].try_into().unwrap()) as usize,
            8 + png.len()
        );
        assert_eq!(&icon[16..], png);
    }

    #[test]
    fn sizes_read_as_on_the_screen() {
        assert_eq!(bytes(73 << 20, '.'), "73 MB");
        assert_eq!(bytes(1 << 30, ','), "1,0 GB");
        assert_eq!(bytes(1, '.'), "1 MB");
        assert_eq!(bytes(118 << 30, ','), "118 GB");
    }
}
