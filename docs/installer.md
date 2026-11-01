# Installer and uninstaller

evolite's installer and uninstaller for Windows, Linux and macOS are one program,
`evolite-setup` (`crates/evolite-ui/src/setup/`), with the same screens on all three. Screens:
Figma `vvKJgusBbXy91ZjiM30b0V`, page **Installer** (I-01…I-05, U-01…U-04, both themes, an
Arabic right-to-left frame). What differs per system is in «Linux and macOS» below.

## Why our own installer on egui

The installer had to do three things the usual tools make hard:

- **15 languages with Arabic right to left**, the same texts and fonts as evolite. Inno Setup
  and NSIS ship their own translations for some of the 15 (not Hindi or Vietnamese at
  the same quality) and render Arabic with the system's controls, not with evolite's look.
- **A screen with every disk**: free and used space, what evolite needs on which disk. In Inno
  Setup this is a custom page in Pascal Script; in WiX, a custom action and a hand-built dialog.
- **The uninstaller's warning**: wallets listed by name (read from evolite's own index) and a
  separate confirmation before the data goes.

Built from evolite's own parts — the palette, the fonts, `i18n` (catalog per language) and the
right-to-left mirror (`rtl.rs`) — the installer looks and reads like the app, in every
language, with no second toolchain in the build. The cost is code we keep ourselves: shortcuts
(COM `IShellLinkW`), the «Installed apps» entry, the folder picker. These are small and per user.

## What it does

### Install (`evolite-setup.exe`)

1. **Language** — the 15 languages, each in its own name and script. The choice becomes
   evolite's language at its first start (written to `%APPDATA%\evolite\settings.json` only if
   there is no settings file yet) and the uninstaller's language.
   - **Already installed** (only when evolite is found): the installed version and folder, this
     installer's version, and what installing does — the same version (install again, e.g. to
     repair), an update, a **newer version installed** (said plainly, with both versions; the
     button reads «Install the older version»), an unknown version, or an incomplete installation
     (files missing). Installing is always offered and goes to the installed folder; wallets and
     settings stay. Found through the «Installed apps» entry (`InstallLocation`,
     `DisplayVersion`), else the default program folder when `evolite.exe` is there — a cleaner
     or an interrupted run may have taken the entry; a reinstall writes it again.
2. **What will be installed** — the program, the Start menu shortcut, the uninstaller; an
   optional desktop shortcut. **Next** stays disabled until «I have read … and agree» is ticked.
3. **Program folder and disk space** — the folder (typed or **Browse…**), and:
   - the program's size (measured from the payload);
   - the testnet data after a full sync, and the mainnet estimate;
   - the recommended free space;
   - every fixed disk with free and total space.
   **Install** stays disabled when evolite does not fit (see «Space» below) or the folder is not
   on a local disk. Nothing is written before **Install**.
4. **Installing** — the files are written next to their place, checked against their SHA-256
   and renamed in; then the uninstaller, the shortcuts and the «Installed apps» entry.
5. **Done** — «Launch evolite» (on by default) starts it as the same user.

### Uninstall (`uninstall.exe --uninstall`, or «Installed apps → evolite → Uninstall»)

1. **What will be removed**:
   - the program (by file name — `evolite.exe`, `evolite-renderer.exe`, `uninstall.exe` — never
     the whole folder: the folder goes only once it is empty);
   - the shortcuts;
   - the «Installed apps» entry;
   - the picture cache (`%LOCALAPPDATA%\evolite`).
   «Also delete wallet data» is off by default.
2. **Only when that box is ticked and data exists**: a large red warning — *Without the recovery
   phrase the money will be lost* — every wallet by its name and network (from
   `wallets/<network>/wallets.json`, also wallets from before there were several), the size to
   be deleted, and a second, separate confirmation. **Delete the data forever** stays disabled
   until it is ticked; **Keep the data** removes everything else and keeps the wallets.
3. **Removing**, then **Done**, saying what was removed and, if kept, where the data is.

If evolite is running when **Install** or **Remove** is pressed, the **evolite is still
running** screen comes instead of the work: **Try again**, and **Close evolite** when its
processes are found — after a crash evolite may stay without a window or stop responding. Close
evolite ends only processes whose full path is the program folder's `evolite.exe` or
`evolite-renderer.exe` (`system::processes_in`); evolite started from elsewhere is left to the
person. evolite holds `evolite.lock` in its data folder while it runs (`lock_data_dir`); the
installer tries that lock (`system::running`) on every system, and on Windows also whether
`evolite.exe` can be opened for writing. A lock left by a crashed process is free again: the
system releases it with the process.

### When the window does not open

The installer and the uninstaller are windowed programs with no console: before 2026-11-02 a
failed start ended them without a word (№10, №11: after a graphics driver reset — Windows'
`LiveKernelEvent 141` — OpenGL did not start). Now:

- **Two renderers** in turn, as in the app (no OpenGL-only `glow` any more): wgpu on the graphics
  card (DirectX 12, Vulkan, Metal or OpenGL); wgpu on the processor (Windows' WARP adapter,
  Mesa's on Linux) — needs no driver at all.
- **A system message box** when none starts, or on a panic: what failed, «restart the computer
  or update the graphics driver», and where the journal is.
- **The journal** `%TEMP%\evolite-setup.log` (`$TMPDIR` on Linux and macOS): each start (mode,
  version, path), the installation found, the renderer that failed and why, the work and its
  result, processes ended.

`--renderer software` starts with the processor, and `--screenshot out.png` without `--screen`
saves this computer's first screen and quits: to check both where the graphics card works.

## Linux and macOS

The same program and screens; only where things go differs (`plan.rs`, `system.rs`). The
folders are those the app itself uses (`cli.rs`: `XDG_DATA_HOME` / `XDG_CACHE_HOME` when set,
on macOS too).

| | Windows | Linux | macOS |
|---|---|---|---|
| Program (default) | `%LOCALAPPDATA%\Programs\evolite` | `~/.local/opt/evolite` | `~/Applications/evolite` |
| In it | `evolite.exe`, `evolite-renderer.exe`, `uninstall.exe` | `evolite`, `evolite-renderer`, `uninstall`, `evolite.png`, `uninstall.json` | `evolite.app`, `Uninstall evolite.app` (each with `Info.plist` and an icon), `uninstall.json` |
| Testnet / mainnet data | `%APPDATA%\evolite[-mainnet]` | `~/.local/share/evolite[-mainnet]` | `~/Library/Application Support/evolite[-mainnet]` |
| Picture cache | `%LOCALAPPDATA%\evolite` | `~/.cache/evolite` | `~/Library/Caches/evolite` |
| Shortcuts | Start menu, desktop (`.lnk`) | `~/.local/share/applications/evolite.desktop` and `evolite-uninstall.desktop` («Remove evolite» in the installer's language); desktop copy | Launchpad shows `~/Applications` by itself; a desktop link to `evolite.app` |
| Uninstaller's language | the «Installed apps» entry | `uninstall.json` | `uninstall.json` |
| Disks | fixed drives | real file systems from `/proc/self/mounts` (no `/boot`, snaps, network); `statvfs` free space for this user | local volumes the Finder shows (`getmntinfo`, `MNT_LOCAL`, not `MNT_DONTBROWSE`) |
| Folder picker | the system dialog | `zenity`, else `kdialog` | AppleScript `choose folder` |
| Uninstaller started by | «Installed apps», `uninstall.exe --uninstall` | the «Remove evolite» menu entry, `uninstall` | `Uninstall evolite.app` |

- The uninstaller knows itself by its name (`uninstall`) or `--uninstall`, and finds the program
  folder from its own place (`plan::program_dir_of`).
- On Linux and macOS a running program can delete its own file: the uninstaller removes itself
  at once; on macOS the two bundles are deleted whole (what macOS adds to a bundle goes with
  them), under the program folder only.
- Desktop entries quote `Exec` as the specification asks; a folder name with a line break in it
  is refused rather than written.
- Executables get mode 755; the payload names for Unix have no `.exe` (`plan::APP_NAME`).

## Systems and architectures

| System | x86_64 | ARM64 |
|---|---|---|
| Windows 10/11 | built and tested here; CI | type-checked here (`--no-default-features`); built in CI on `windows-11-arm` — needs the MSVC ARM64 tools, not installed on this computer |
| Linux (glibc) | built, tested (live install cycle included) and screenshotted in Docker (`linux-check/`); CI | cross-built in Docker (`aarch64-linux-gnu-gcc`), packed and tested under Docker Desktop's ARM emulation (live cycle included); CI on `ubuntu-24.04-arm` |
| macOS 11+ | type-checked here; cross-built in CI on Apple silicon | type-checked here; built and tested in CI on `macos-15` |

Packed Linux installers of 2026-11-01 (release, not stripped): x86_64 132.2 MB (installer
27.3 MB, program 105.0 MB), ARM64 123.4 MB (installer 25.5 MB, program 97.9 MB).

macOS cannot be built in full off a Mac: the C parts of the wallet core (BLAKE3, X11, SQLite)
need Apple's SDK. `.github/workflows/build.yml` builds all six, runs the installer's tests (the
live cycle too — a fresh runner) and uploads each packed installer.

## Space

Measured on 2026-11-01 (`plan.rs` keeps the numbers and how they were made):

| What | Size | Where |
|---|---|---|
| Program (`evolite.exe` 69.4 MB + `evolite-renderer.exe` 4.0 MB + `uninstall.exe` 14.8 MB, release) | 84 MiB | the program folder's disk |
| Testnet data after a full sync (height 1.58 M) | 360 MiB | `%APPDATA%` — always the user's profile disk |
| — headers / filter headers / filters / wallet's blocks / masternodes / store | 171 / 49 / 63 / 67 / 1 / 4 MB | |
| Mainnet data after a full sync, **estimate** | ~1 GiB | `%APPDATA%\evolite-mainnet` |
| — headers (measured, 2.37 M blocks) | 255 MB | |
| Recommended free, testnet (its data + ~0.65 GiB headroom) | 1 GiB | the data disk |
| Recommended free with mainnet turned on (+ mainnet's data) | 2 GiB | the data disk |

The data folder is not chosen by the installer: evolite keeps it in `%APPDATA%`. So the check
is made on **two** disks — the program on its disk, the data on the profile disk:

- **does not fit** (Install disabled): the program disk has less than the program + 64 MB, or
  the data disk less than the testnet data + 64 MB (+ the program if it is the same disk);
- **tight** (a notice, Install allowed): less than the recommended 1 GiB would stay free;
- otherwise nothing is said. The recommendation with mainnet (2 GiB) is shown for the person's
  later choice and never blocks: evolite starts on testnet.

The mainnet estimate: headers measured at 255 MB; filter headers are 32 bytes a block
(~77 MB); filters and the wallet's blocks scale with busier blocks than testnet's (~0.6 GB).
It is a range, not a measurement — replace it when a full mainnet sync has been measured.

## Safety

- **No administrator or root rights** on any system: Linux and macOS write only under the home
  folder (and the program folder chosen). On Windows the manifest (`setup.manifest`, embedded by `build.rs` through
  `setup.rc`) says `asInvoker`; without it Windows takes an executable named «setup» for a
  legacy installer and asks for elevation. Everything is per user: `%LOCALAPPDATA%\Programs`,
  `HKEY_CURRENT_USER`, the user's Start menu.
- **Known paths only.** The folders deleted whole are computed from the user's known folders
  (`SHGetKnownFolderPath`: `%LOCALAPPDATA%`, `%APPDATA%`) plus fixed names, and checked again
  right before deletion (`Known::is_own_folder`). Nothing typed reaches a recursive delete: the
  program folder the person chose loses only evolite's own files by name, and the folder itself
  only when empty (`rmdir` without `/S`).
- **No silent deletion.** Wallet data needs the box on the first screen *and* the confirmation
  under the warning that lists the wallets.
- **Integrity.** Each payload file carries its SHA-256; a damaged or cut installer installs
  nothing. File names in the payload are plain names (no folders), so a crafted payload cannot
  write outside the program folder. The installer is not code-signed yet (see below).
- The uninstaller cannot delete itself while it runs: a hidden `cmd` waits two seconds after it
  exits, deletes `uninstall.exe` and removes the folder if it is empty.

## Building a release

```
cargo build --release -p evolite-ui
Start-Process -Wait -NoNewWindow target\release\evolite-setup.exe -ArgumentList '--pack','dist\evolite-setup.exe'
```

(A release build is a windowed program: PowerShell does not wait for it unless told to;
`--pack` writes to the console it was started from.) Build it without `--features dev-tools`:
the packed program must not carry the screenshot flags and the demo core
(`docs/development.md`, "Screenshots and checks"). The installer's own `--screen` below is a separate program's and stays. On 2026-11-01 the packed installer was
88.3 MB: the installer 14.8 MB and the program 73.5 MB.

Linux and macOS: `./target/release/evolite-setup --pack dist/evolite-setup`; the download is
that one file (`chmod +x`, then run it). Linux in Docker: `linux-check/README.md`.

`--pack` appends `evolite` and `evolite-renderer` (`.exe` on Windows) from its own folder to a
copy of itself (layout in `payload.rs`). The packed file is the download; the uninstaller is the
same program without the payload.

Screenshots of every screen, with the Figma example instead of this computer's folders and
disks (nothing is read or written):

```
evolite-setup.exe --screen language|existing|existing-newer|existing-older|existing-damaged|what|space|space-low|running|installing|installed|confirm|warning|running-remove|removing|removed --theme dark|light --lang ru --screenshot out.png
```

The second renderer costs size: wgpu made the installer 21.5 MB (was 14.9) and the app 75.6 MB
(was 69.1); the packed installer of 2026-11-02 is 101.1 MB.

## Publishing a release

A tag `v*` pushed to GitHub runs `.github/workflows/build.yml` for the tag, and once all six
builds pass, its `release` job publishes them as a GitHub release (a pre-release while evolite is a
testnet demo): `evolite-setup-<system>-<architecture>[.exe]` and `SHA256SUMS`.

```
git tag v0.1.0
git push origin v0.1.0
```

If a build fails, nothing is published: fix it, move the tag (`git tag -f v0.1.0`,
`git push -f origin v0.1.0`) or take the next version. The installers are not code-signed
(SmartScreen on Windows, Gatekeeper on macOS warn about them); the release notes say so.

## Languages

The installer's strings are English in the code (`tr!`) and translated in
`crates/evolite-ui/locales/setup/<code>.json`, one file per language (`ar`, `de`, `es`, `fr`,
`hi`, `id`, `ja`, `ko`, `pt-BR`, `ru`, `tr`, `uk`, `vi`, `zh-Hans`). evolite's `i18n` module reads
them (`i18n::use_catalogs`). A test checks that every language has every string and keeps the
`{}` blanks in order.

## Tests

- `payload`: round trip, an installer without payload, a damaged file installs nothing, names
  cannot leave the folder, a cut download is refused.
- `plan`: the install folder, which disk decides, only evolite's own folders are deleted whole,
  wallets found by name, sizes as shown.
- `plan` on Linux and macOS: a path is on the mount it is inside by whole folder names
  (`/home2` is not on `/home`), the folders match the app's, the uninstaller finds its folder,
  the macOS `Info.plist` and `.icns`.
- `work::live_install_and_remove` (ignored, run by hand; CI runs it): the whole cycle with the
  program, data and cache in a temporary folder — files (mode 755 on Unix), uninstaller, the
  language for it, shortcuts (Linux menu entries go to the temporary folder too), first
  language — then all removed, data too; a file of the person's beside the program folder
  stays. On Windows it writes the real Start menu and HKCU entry, so it refuses to run where
  evolite is installed.
- `work::a_running_evolite_is_seen_by_its_lock`: while the lock is held, nothing is removed.
- `work::a_running_program_is_found_by_its_full_path`, `work::a_hung_process_is_ended`: Close
  evolite finds processes by full path only, and ends one that does not end by itself.
- `plan`: versions compared by their numbers (`0.10` is newer than `0.9`), an installation
  without its uninstaller is incomplete.

## Not done yet

- **Code signing**: Authenticode on Windows (SmartScreen warns on first download); on macOS
  signing and notarization — until then Gatekeeper asks to confirm the first start (Control-click
  → Open). Appending the payload to a signed Mach-O breaks its signature: a signed macOS
  release needs the payload as a separate file in a setup bundle's `Resources`.
- **Closing evolite on macOS**: no process list there, only the lock — the person closes it.
- **A data folder given with `--data-dir`** is not known to the uninstaller and stays.
- **Packages**: no `.deb`/`.rpm`/AppImage/`.dmg` yet; the packed installer is one executable on
  every system. Linux needs glibc (built on Debian 12: glibc 2.36) and X11 or Wayland.
- **Notifications** stay inside the app on Linux and macOS (`notify.rs` uses the Windows tray).
