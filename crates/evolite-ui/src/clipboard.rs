//! What the app puts on the clipboard and wipes again. The recovery phrase always: 60 s later
//! and when the app closes, whatever screen is open by then. Addresses, names, txids and links
//! too while Settings → "Clipboard" is on. Either only while the clipboard still
//! holds it: what the person copied in between is theirs and stays. Only a hash of the copied
//! text is kept to compare with, not the text.

use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// How long the phrase stays, as the button promises ("Copy for 60 s"); and anything
/// else copied, while "Clipboard" is on.
pub const KEEP: Duration = Duration::from_secs(60);

/// What was copied, kept in egui's memory so that any screen can copy and the app wipes.
#[derive(Clone, Copy)]
struct Copied {
    hash: [u8; 32],
    at: Instant,
}

fn id() -> egui::Id {
    egui::Id::new("evolite-secret-clipboard")
}

/// The last address, name or link copied.
fn copy_id() -> egui::Id {
    egui::Id::new("evolite-copied-clipboard")
}

thread_local! {
    /// "Clipboard" in Settings, put here by the app every frame.
    static WIPE_COPIES: Cell<bool> = const { Cell::new(true) };
}

/// From the app: whether what is copied besides the phrase is wiped too.
pub fn set_wipe_copies(on: bool) {
    WIPE_COPIES.with(|w| w.set(on));
}

fn hash(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// FEAT-0042 R-9: public text that should not stay in the clipboard's history or the cloud (an
/// account public key) — past them as a secret is, but not forced away after [`KEEP`]: it
/// spends nothing and is meant to be pasted. Wiped as an ordinary copy is when «Clipboard» is on
/// (review r1, Should 1). `false`: it could not go past the history, so it was not copied.
pub fn copy_public_excluded(ctx: &egui::Context, text: &str) -> bool {
    if !put_secret(ctx, text) {
        return false;
    }
    let copied = Copied {
        hash: hash(text),
        at: Instant::now(),
    };
    ctx.data_mut(|d| d.insert_temp(copy_id(), Some(copied)));
    true
}

/// Copies a secret (the phrase) for [`KEEP`]. On Windows it is kept out of the clipboard
/// history (Win+V), the cloud clipboard and clipboard managers: a copy there is not wiped
/// with the clipboard. If it cannot be copied that way, it is not copied at all (false).
pub fn copy_secret(ctx: &egui::Context, text: Zeroizing<String>) -> bool {
    let put = put_secret(ctx, &text);
    remember(ctx, &text, put)
}

#[cfg(all(windows, not(test)))]
fn put_secret(_ctx: &egui::Context, text: &str) -> bool {
    arboard::Clipboard::new().is_ok_and(|board| System(board).put_secret(text))
}

/// Elsewhere egui's clipboard keeps the text while the app runs (an arboard one would
/// lose it on Linux when dropped); there is no history to keep it out of.
#[cfg(any(not(windows), test))]
fn put_secret(ctx: &egui::Context, text: &str) -> bool {
    ctx.copy_text(text.to_owned());
    true
}

/// Notes what to wipe, only if it was copied; and, on disk, that a secret is on the clipboard
/// (its hash only), for a start after a crash to wipe it (SEC-2 B5).
fn remember(ctx: &egui::Context, text: &str, put: bool) -> bool {
    if put {
        let copied = Copied {
            hash: hash(text),
            at: Instant::now(),
        };
        ctx.data_mut(|d| d.insert_temp(id(), Some(copied)));
        mark(Some(copied.hash));
    }
    put
}

/// The file that says a secret is on the clipboard: its SHA-256 in hex, never the text
/// (SEC-2 B5). Set once at the start; none in demo runs without a data folder.
static MARKER: Mutex<Option<PathBuf>> = Mutex::new(None);

/// At the start: where the mark is kept, and a secret a run that ended without wiping it (a
/// crash) left on the clipboard is wiped now, if the clipboard still holds it.
pub fn start(marker: PathBuf) {
    if let Some(mut board) = system_board() {
        wipe_left(&marker, &mut board);
    }
    *MARKER.lock().unwrap_or_else(|e| e.into_inner()) = Some(marker);
}

/// Wipes what the mark at `marker` names, if the clipboard still holds it, and removes the
/// mark. Whether it wiped.
fn wipe_left(marker: &Path, board: &mut impl Board) -> bool {
    let Some(left) = std::fs::read_to_string(marker)
        .ok()
        .and_then(|hex| hash_from_hex(hex.trim()))
    else {
        let _ = std::fs::remove_file(marker);
        return false;
    };
    // Not read: nothing to compare, and a crash's copy may be long gone; it is left.
    let wiped = board.text().is_some() && wipe_if_still(left, board, false);
    let _ = std::fs::remove_file(marker);
    wiped
}

fn mark(secret: Option<[u8; 32]>) {
    let Some(path) = MARKER.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return;
    };
    match secret {
        Some(hash) => {
            let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
            let _ = std::fs::write(path, hex);
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn hash_from_hex(hex: &str) -> Option<[u8; 32]> {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect::<Option<_>>()?;
    bytes.try_into().ok()
}

/// A phrase pasted into the restore (SEC-2 B4): the clipboard is cleared if it still holds
/// exactly that text. Whether it was.
pub fn wipe_pasted(text: &str) -> bool {
    system_board().is_some_and(|mut board| wipe_if_pasted(text, &mut board))
}

/// Clears the clipboard if it holds `pasted`, line breaks aside: egui hands a Ctrl+V over with
/// Windows' `\r\n` made `\n`, so a phrase copied from Notepad never matched (QA, batch 32).
fn wipe_if_pasted(pasted: &str, board: &mut impl Board) -> bool {
    let lf = |s: &str| Zeroizing::new(s.replace("\r\n", "\n"));
    let still = board
        .text()
        .is_some_and(|on| hash(&lf(&on)) == hash(&lf(pasted)));
    if still {
        board.clear();
    }
    still
}

/// The lock (SEC-2 B5): a secret copied is wiped now, not 60 s later.
pub fn wipe_secret_now(ctx: &egui::Context) {
    wipe_secret(ctx);
}

/// Copies an address, a name, a txid or a link; wiped after [`KEEP`] while "Clipboard" is
/// on.
pub fn copy(ctx: &egui::Context, text: String) {
    let copied = Copied {
        hash: hash(&text),
        at: Instant::now(),
    };
    ctx.copy_text(text);
    ctx.data_mut(|d| d.insert_temp(copy_id(), Some(copied)));
}

/// How long "Copied" stands in place of the control that copied, in seconds.
pub const COPIED_FOR: f64 = 1.5;

/// The control that copied last (`what`) and when, by egui's clock.
fn answered_id() -> egui::Id {
    egui::Id::new("evolite-copied-answer")
}

/// [`copy`], and "Copied" on the control `what` for [`COPIED_FOR`] (see [`just_copied`]).
pub fn copy_answered(ctx: &egui::Context, text: String, what: egui::Id) {
    copy(ctx, text);
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(answered_id(), Some((what, now))));
}

/// Whether the control `what` copied less than [`COPIED_FOR`] ago: it then says "Copied". Asks
/// for the frame that takes the word away.
pub fn just_copied(ctx: &egui::Context, what: egui::Id) -> bool {
    let Some((last, at)) = ctx.data(|d| {
        d.get_temp::<Option<(egui::Id, f64)>>(answered_id())
            .flatten()
    }) else {
        return false;
    };
    let left = COPIED_FOR - (ctx.input(|i| i.time) - at);
    let shown = last == what && left > 0.0;
    if shown {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    shown
}

/// When the secret on the clipboard was copied, while the app still has to wipe it.
pub fn copied_at(ctx: &egui::Context) -> Option<Instant> {
    ctx.data(|d| d.get_temp::<Option<Copied>>(id()).flatten())
        .map(|c| c.at)
}

/// Every frame, from the app: wipes the secret, and a copy while "Clipboard" is on, once
/// its time is up.
pub fn tick(ctx: &egui::Context) {
    let copy = ctx.data(|d| d.get_temp::<Option<Copied>>(copy_id()).flatten());
    if let Some(copy) = copy {
        if !WIPE_COPIES.with(Cell::get) {
            // Switched off: the copy stays on the clipboard.
            ctx.data_mut(|d| d.remove_temp::<Option<Copied>>(copy_id()));
        } else if copy.at.elapsed() >= KEEP {
            ctx.data_mut(|d| d.remove_temp::<Option<Copied>>(copy_id()));
            if let Some(mut board) = system_board() {
                wipe_if_still(copy.hash, &mut board, false);
            }
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
    let Some(at) = copied_at(ctx) else {
        return;
    };
    if at.elapsed() >= KEEP {
        wipe_secret(ctx);
    } else {
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

/// Wipes now (the app is closing): the secret, and a copy while "Clipboard" is on — if the
/// clipboard still holds them.
pub fn wipe_now(ctx: &egui::Context) {
    wipe_secret(ctx);
    let copy = ctx.data_mut(|d| d.remove_temp::<Option<Copied>>(copy_id()).flatten());
    if let Some(copy) = copy
        && WIPE_COPIES.with(Cell::get)
        && let Some(mut board) = system_board()
    {
        wipe_if_still(copy.hash, &mut board, false);
    }
}

fn wipe_secret(ctx: &egui::Context) {
    let copied = ctx.data_mut(|d| d.remove_temp::<Option<Copied>>(id()).flatten());
    if let Some(copied) = copied {
        if let Some(mut board) = system_board() {
            wipe_if_still(copied.hash, &mut board, true);
        }
        mark(None);
    }
}

/// The system clipboard to wipe; none in tests, which must not touch the desktop they run on
/// (QA may be working at it): what they copy stays in egui (`ctx.copy_text`).
fn system_board() -> Option<System> {
    #[cfg(test)]
    return None;
    #[cfg(not(test))]
    return arboard::Clipboard::new().ok().map(System);
}

/// The system clipboard, as [`wipe_if_still`] needs it.
trait Board {
    fn text(&mut self) -> Option<Zeroizing<String>>;
    fn clear(&mut self);
}

// Never made in tests (`system_board`).
#[cfg_attr(test, allow(dead_code))]
struct System(arboard::Clipboard);

#[cfg(all(windows, not(test)))]
impl System {
    fn put_secret(&mut self, text: &str) -> bool {
        excluded(self.0.set()).text(text).is_ok()
    }
}

/// What a secret is kept out of on Windows: clipboard managers, the history (Win+V) and the
/// cloud clipboard (SEC-2 B1: Microsoft names the last two formats for those).
trait Exclusions: Sized {
    fn monitoring(self) -> Self;
    fn history(self) -> Self;
    fn cloud(self) -> Self;
}

#[cfg(windows)]
impl Exclusions for arboard::Set<'_> {
    fn monitoring(self) -> Self {
        arboard::SetExtWindows::exclude_from_monitoring(self)
    }
    fn history(self) -> Self {
        arboard::SetExtWindows::exclude_from_history(self)
    }
    fn cloud(self) -> Self {
        arboard::SetExtWindows::exclude_from_cloud(self)
    }
}

#[cfg_attr(not(all(windows, not(test))), allow(dead_code))]
fn excluded<S: Exclusions>(set: S) -> S {
    set.monitoring().history().cloud()
}

impl Board for System {
    fn text(&mut self) -> Option<Zeroizing<String>> {
        self.0.get_text().ok().map(Zeroizing::new)
    }

    fn clear(&mut self) {
        let _ = self.0.clear();
    }
}

/// Clears the clipboard if it holds what was copied. If it cannot be read, a secret is cleared
/// anyway: a phrase left behind is worse than losing something copied in between; an address
/// is not. Returns whether it cleared.
fn wipe_if_still(copied: [u8; 32], board: &mut impl Board, secret: bool) -> bool {
    let still = match board.text() {
        Some(text) => hash(&text) == copied,
        None => secret,
    };
    if still {
        board.clear();
    }
    still
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Option<String>);

    impl Board for Fake {
        fn text(&mut self) -> Option<Zeroizing<String>> {
            self.0.clone().map(Zeroizing::new)
        }

        fn clear(&mut self) {
            self.0 = Some(String::new());
        }
    }

    /// FEAT-0042 review r1, Should 1 (AC-T-X3): the account public key is copied past the
    /// history, recorded as an ordinary copy — wiped with «Clipboard» on, kept with it off — and
    /// never as a secret (no forced wipe, no mark).
    #[test]
    fn a_public_key_is_wiped_as_an_ordinary_copy() {
        let ctx = egui::Context::default();
        assert!(copy_public_excluded(&ctx, "tpubD6NzVbkr"));
        let ordinary = ctx.data(|d| d.get_temp::<Option<Copied>>(copy_id()).flatten());
        assert!(ordinary.is_some(), "recorded as an ordinary copy");
        assert!(copied_at(&ctx).is_none(), "not a secret");
    }

    #[test]
    fn the_phrase_is_wiped_while_it_is_still_there() {
        let mut board = Fake(Some("abandon ability".into()));
        assert!(wipe_if_still(hash("abandon ability"), &mut board, true));
        assert_eq!(board.0.as_deref(), Some(""));
    }

    #[test]
    fn something_copied_since_stays() {
        let mut board = Fake(Some("yTx7…kP2q".into()));
        assert!(!wipe_if_still(hash("abandon ability"), &mut board, true));
        assert_eq!(board.0.as_deref(), Some("yTx7…kP2q"));
    }

    #[test]
    fn a_clipboard_that_cannot_be_read_is_cleared() {
        let mut board = Fake(None);
        assert!(wipe_if_still(hash("abandon ability"), &mut board, true));
    }

    #[test]
    fn an_address_on_a_clipboard_that_cannot_be_read_stays() {
        let mut board = Fake(None);
        assert!(!wipe_if_still(hash("yTx7…kP2q"), &mut board, false));
        let mut board = Fake(Some("yTx7…kP2q".into()));
        assert!(wipe_if_still(hash("yTx7…kP2q"), &mut board, false));
    }

    #[test]
    fn a_copy_is_forgotten_when_the_switch_is_off() {
        let ctx = egui::Context::default();
        copy(&ctx, "yTx7…kP2q".into());
        assert!(
            ctx.data(|d| d.get_temp::<Option<Copied>>(copy_id()).flatten())
                .is_some()
        );
        set_wipe_copies(false);
        tick(&ctx);
        assert!(
            ctx.data(|d| d.get_temp::<Option<Copied>>(copy_id()).flatten())
                .is_none()
        );
        set_wipe_copies(true);
    }

    #[test]
    fn a_phrase_not_copied_is_not_promised_or_wiped() {
        let ctx = egui::Context::default();
        assert!(!remember(&ctx, "abandon ability", false));
        assert!(copied_at(&ctx).is_none());
        assert!(remember(&ctx, "abandon ability", true));
        assert!(copied_at(&ctx).is_some());
    }

    #[test]
    fn the_app_keeps_only_a_hash_and_the_time() {
        let ctx = egui::Context::default();
        assert!(copied_at(&ctx).is_none());
        ctx.data_mut(|d| {
            d.insert_temp(
                id(),
                Some(Copied {
                    hash: hash("abandon ability"),
                    at: Instant::now(),
                }),
            )
        });
        assert!(copied_at(&ctx).is_some());
    }

    /// SEC-2 B1: a secret is kept out of clipboard managers, the history and the cloud.
    #[test]
    fn a_secret_is_kept_out_of_the_history_and_the_cloud() {
        #[derive(Default)]
        struct Set(Vec<&'static str>);
        impl Exclusions for Set {
            fn monitoring(mut self) -> Self {
                self.0.push("monitoring");
                self
            }
            fn history(mut self) -> Self {
                self.0.push("history");
                self
            }
            fn cloud(mut self) -> Self {
                self.0.push("cloud");
                self
            }
        }
        let set = excluded(Set::default());
        assert_eq!(set.0, ["monitoring", "history", "cloud"]);
    }

    /// SEC-2 B5: a secret a crashed run left is wiped at the next start if still there, and
    /// the mark goes; a clipboard changed since keeps what is on it.
    #[test]
    fn a_secret_left_by_a_crash_is_wiped_at_the_start() {
        let dir = std::env::temp_dir().join(format!("evolite-clip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("clipboard-secret");
        let hex: String = hash("zebra walnut")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        std::fs::write(&marker, &hex).unwrap();
        let mut board = Fake(Some("zebra walnut".into()));
        assert!(wipe_left(&marker, &mut board));
        assert_eq!(board.0.as_deref(), Some(""), "wiped");
        assert!(!marker.exists(), "the mark goes");
        std::fs::write(&marker, &hex).unwrap();
        let mut other = Fake(Some("an address".into()));
        assert!(!wipe_left(&marker, &mut other));
        assert_eq!(other.0.as_deref(), Some("an address"), "copied since: kept");
        assert!(!marker.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SEC-2 B4, QA batch 32: a phrase copied with a Windows line break and pasted by Ctrl+V
    /// (egui makes `\r\n` `\n`) leaves the clipboard; one copied since stays.
    #[test]
    fn a_phrase_pasted_with_a_windows_line_break_is_wiped() {
        let words = ["abandon"; 11].join(" ") + " about";
        let mut board = Fake(Some(format!("{words}\r\n")));
        assert!(wipe_if_pasted(&format!("{words}\n"), &mut board));
        assert_eq!(board.0.as_deref(), Some(""));
        let mut button = Fake(Some(format!("{words}\r\n")));
        assert!(
            wipe_if_pasted(&format!("{words}\r\n"), &mut button),
            "«Paste»: as is"
        );
        let mut other = Fake(Some("an address".into()));
        assert!(!wipe_if_pasted(&format!("{words}\n"), &mut other));
        assert_eq!(other.0.as_deref(), Some("an address"));
    }

    /// SEC-12: every copy in the app goes through this module — wiped with the others when
    /// «Clipboard» is on, a secret kept out of the history — so no other file copies by itself.
    #[test]
    fn nothing_copies_past_the_clipboard_module() {
        fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    files(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut all = Vec::new();
        files(&src, &mut all);
        let direct = ["copy", "_text("].concat();
        let past: Vec<_> = all
            .iter()
            .filter(|p| !p.ends_with("clipboard.rs"))
            .filter(|p| {
                let text = std::fs::read_to_string(p).unwrap();
                text.contains(&direct)
            })
            .collect();
        assert!(all.len() > 50, "the scan found the sources");
        assert!(past.is_empty(), "copied past `clipboard`: {past:?}");
    }
}
