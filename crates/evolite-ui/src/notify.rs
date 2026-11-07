//! Settings → Notifications: system notifications for an incoming payment, a lock on a
//! payment of this wallet and a request from an app — while evolite is open but not in front.
//!
//! What happened is found by comparing the core's snapshots ([`changes`]); the history a sync
//! or a resync brings in at once is not news, only what is recent. On Windows the note is the
//! notification-area balloon (`Shell_NotifyIconW`, `NIF_INFO`), which Windows 10 and 11 show
//! as a toast — through `windows-sys`, already a dependency, no new crate. Elsewhere there is
//! none yet: the switches say so.

use evolite_api::{Activity, ActivityKind, Proof, Timestamp};
use std::collections::HashMap;

/// A history entry older than this when it first appears is not news (a sync, a rescan).
const RECENT: Timestamp = 10 * 60;

/// One system notification.
#[derive(Clone, PartialEq, Debug)]
pub struct Note {
    pub title: String,
    pub body: String,
}

/// Which notifications Settings allow.
#[derive(Clone, Copy, Default)]
pub struct Wanted {
    pub incoming: bool,
    pub locks: bool,
}

fn locked(proof: Proof) -> bool {
    matches!(proof, Proof::InstantLock | Proof::ChainLock)
}

fn incoming(a: &Activity) -> bool {
    a.amount > 0
        && matches!(
            a.kind,
            ActivityKind::Received { .. } | ActivityKind::PlatformReceived
        )
}

/// What changed between two snapshots of the wallet that is worth a notification. `seen`: the
/// history's proofs as of the last snapshot, by id; `None` the first time (nothing is news).
pub fn changes(
    seen: Option<&HashMap<u64, Proof>>,
    activity: &[Activity],
    now: Timestamp,
    wanted: Wanted,
) -> Vec<Note> {
    let Some(seen) = seen else {
        return Vec::new();
    };
    let hide = crate::format::display().hide;
    let amount = |a: &Activity| {
        if hide {
            String::new()
        } else {
            format!(" {}", crate::format::amount(a.amount.abs()))
        }
    };
    let mut notes = Vec::new();
    for a in activity {
        match seen.get(&a.id) {
            None if a.at + RECENT < now => {}
            None if wanted.incoming && incoming(a) => notes.push(Note {
                // The bare word is its own key: a template needs something in its place.
                title: if hide {
                    tr!("Incoming").to_string()
                } else {
                    format!(tr!("Received{}"), amount(a))
                },
                body: format!(
                    "{} · {}",
                    crate::text::activity_title(a),
                    if locked(a.proof) {
                        tr!("confirmed")
                    } else {
                        tr!("waiting for the network to confirm")
                    }
                ),
            }),
            // A payment of ours locked: InstantLock or, without one, the ChainLock.
            Some(before) if wanted.locks && !locked(*before) && locked(a.proof) => {
                notes.push(Note {
                    title: format!(
                        "{}{}",
                        if a.proof == Proof::InstantLock {
                            "InstantLock:"
                        } else {
                            "ChainLock:"
                        },
                        amount(a)
                    ),
                    body: format!(
                        tr!("{} · the payment is confirmed and cannot be undone"),
                        crate::text::activity_title(a)
                    ),
                })
            }
            _ => {}
        }
    }
    notes
}

/// The history's proofs, for the next [`changes`].
pub fn proofs(activity: &[Activity]) -> HashMap<u64, Proof> {
    activity.iter().map(|a| (a.id, a.proof)).collect()
}

/// A request from an app (Apps), when Settings allow it.
pub fn app_request(app: &str) -> Note {
    Note {
        title: format!(tr!("Request from {app}"), app = app),
        body: tr!("Open evolite to answer: nothing is signed without you").into(),
    }
}

/// Whether this system can show notifications at all.
pub const SUPPORTED: bool = cfg!(windows);

/// The system side: one icon in the notification area while evolite runs.
#[derive(Default)]
pub struct Notifier {
    #[cfg(windows)]
    tray: Option<windows::Tray>,
}

impl Notifier {
    /// Shows `note`; false when the system refused or has no notifications.
    pub fn show(&mut self, note: &Note) -> bool {
        #[cfg(windows)]
        {
            if self.tray.is_none() {
                self.tray = windows::Tray::new();
            }
            self.tray.as_mut().is_some_and(|t| t.show(note))
        }
        #[cfg(not(windows))]
        {
            let _ = note;
            false
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::Note;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
        Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE, IDI_APPLICATION, LoadIconW,
    };

    /// A message-only window owning the notification-area icon.
    pub struct Tray {
        hwnd: HWND,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// `text` into a fixed UTF-16 field, cut to fit with its terminating zero.
    fn copy_into(field: &mut [u16], text: &str) {
        let units: Vec<u16> = text.encode_utf16().take(field.len() - 1).collect();
        field[..units.len()].copy_from_slice(&units);
        field[units.len()] = 0;
    }

    impl Tray {
        fn data(&self) -> NOTIFYICONDATAW {
            // SAFETY: NOTIFYICONDATAW is plain data; all zeros is its empty value.
            let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = self.hwnd;
            data.uID = 1;
            data
        }

        pub fn new() -> Option<Self> {
            let class = wide("STATIC");
            let name = wide("evolite-notify");
            // SAFETY: a message-only window of a system class, no callbacks of ours.
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    name.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            if hwnd.is_null() {
                return None;
            }
            let tray = Tray { hwnd };
            let mut data = tray.data();
            data.uFlags = NIF_ICON | NIF_TIP;
            // SAFETY: the icon resource 1 of this executable (`evolite.rc`); the system's own
            // application icon when a build has none.
            data.hIcon = unsafe {
                let own = LoadIconW(GetModuleHandleW(std::ptr::null()), 1 as _);
                if own.is_null() {
                    LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
                } else {
                    own
                }
            };
            copy_into(&mut data.szTip, "evolite");
            // SAFETY: `data` is filled as the call expects.
            if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
                return None;
            }
            Some(tray)
        }

        pub fn show(&mut self, note: &Note) -> bool {
            let mut data = self.data();
            data.uFlags = NIF_INFO;
            data.dwInfoFlags = NIIF_INFO;
            // Straight to Windows, past the screens: translated here.
            copy_into(&mut data.szInfoTitle, &crate::i18n::t(&note.title));
            copy_into(&mut data.szInfo, &crate::i18n::t(&note.body));
            // SAFETY: as in `new`.
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) != 0 }
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            let data = self.data();
            // SAFETY: the icon and the window are ours.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &data);
                DestroyWindow(self.hwnd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::Chain;

    fn act(id: u64, amount: i64, proof: Proof, at: Timestamp) -> Activity {
        Activity {
            id,
            kind: if amount > 0 {
                ActivityKind::Received {
                    from: Some("@alice".into()),
                }
            } else {
                ActivityKind::Sent { to: "@bob".into() }
            },
            chain: Chain::Core,
            at,
            amount,
            proof,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
        }
    }

    fn wallet(activity: Vec<Activity>) -> Vec<Activity> {
        activity
    }

    const ALL: Wanted = Wanted {
        incoming: true,
        locks: true,
    };

    #[test]
    fn nothing_is_news_the_first_time() {
        let w = wallet(vec![act(1, 25_000_000, Proof::Mempool, 1000)]);
        assert!(changes(None, &w, 1000, ALL).is_empty());
    }

    #[test]
    fn a_new_payment_in_and_a_lock_on_one_out() {
        let before = wallet(vec![act(2, -10_000, Proof::Sending, 990)]);
        let seen = proofs(&before);
        let after = wallet(vec![
            act(1, 25_000_000, Proof::Mempool, 1000),
            act(2, -10_000, Proof::InstantLock, 990),
        ]);
        let notes = changes(Some(&seen), &after, 1000, ALL);
        assert_eq!(notes.len(), 2);
        assert!(
            notes[0].title.starts_with("Received 0.2500"),
            "{:?}",
            notes[0]
        );
        assert!(notes[1].title.starts_with("InstantLock:"), "{:?}", notes[1]);
        // The ChainLock after an InstantLock is not a second note.
        let seen = proofs(&after);
        let later = wallet(vec![
            act(1, 25_000_000, Proof::InstantLock, 1000),
            act(2, -10_000, Proof::ChainLock, 990),
        ]);
        let notes = changes(Some(&seen), &later, 1100, ALL);
        assert_eq!(notes.len(), 1, "the incoming one locked: {notes:?}");
    }

    #[test]
    fn old_history_from_a_rescan_and_switched_off_kinds_are_quiet() {
        let seen = HashMap::new();
        let w = wallet(vec![act(1, 25_000_000, Proof::ChainLock, 100)]);
        assert!(changes(Some(&seen), &w, 100_000, ALL).is_empty());
        let w = wallet(vec![act(1, 25_000_000, Proof::Mempool, 1000)]);
        let off = Wanted {
            incoming: false,
            ..ALL
        };
        assert!(changes(Some(&seen), &w, 1000, off).is_empty());
    }

    #[test]
    fn hidden_amounts_stay_hidden_in_a_note() {
        crate::format::set_display(crate::format::Display {
            hide: true,
            ..Default::default()
        });
        let w = wallet(vec![act(1, 25_000_000, Proof::Mempool, 1000)]);
        let notes = changes(Some(&HashMap::new()), &w, 1000, ALL);
        assert_eq!(notes[0].title, tr!("Incoming"));
        crate::format::set_display(Default::default());
    }

    /// Shows a real notification on this Windows: `cargo t system_notification -- --ignored`.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn system_notification() {
        let mut n = Notifier::default();
        assert!(n.show(&Note {
            title: "Received 0.2500 DASH".into(),
            body: "From @alice · evolite notification check".into(),
        }));
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}
