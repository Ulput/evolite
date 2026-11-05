//! What the app puts on the clipboard and wipes again. The recovery phrase always: 60 s later
//! and when the app closes, whatever screen is open by then. Addresses, names, txids and links
//! too while Settings → "Clipboard" is on. Either only while the clipboard still
//! holds it: what the person copied in between is theirs and stays. Only a hash of the copied
//! text is kept to compare with, not the text.

use sha2::{Digest, Sha256};
use std::cell::Cell;
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

/// Notes what to wipe, only if it was copied.
fn remember(ctx: &egui::Context, text: &str, put: bool) -> bool {
    if put {
        let copied = Copied {
            hash: hash(text),
            at: Instant::now(),
        };
        ctx.data_mut(|d| d.insert_temp(id(), Some(copied)));
    }
    put
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
    if let Some(copied) = copied
        && let Some(mut board) = system_board()
    {
        wipe_if_still(copied.hash, &mut board, true);
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
        use arboard::SetExtWindows;
        self.0.set().exclude_from_monitoring().text(text).is_ok()
    }
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
}
