//! FEAT-0045 step 2 (R-12): the automatic copies. Checked after the unlock and then each hour
//! while the wallet is open, off the loop's thread; one is written when a week has passed since
//! the last. Nothing is written once the wallet locks: its key goes with the lock. The folder and
//! the times live in the wallet's `auto-backup.json` — not in a backup: a new computer, a new
//! folder. The journal has the outcome's kind only (R-14).
//!
//! TECH-bug-prevention (а2), TEAM.md §8d-4: the file is a [`json_file::Store`]. One that does
//! not read is set aside (`LocalFile::AutoBackup`): the copies stop and the settings screen says
//! «The backup setting could not be read», with «Try again» and «Start over» — never quietly
//! «off», and nothing is written over the file set aside.

use crate::real::*;
use evolite_api::backup::{AutoBackup, AutoFailed};
use evolite_local::backup::auto;
use evolite_local::json_file::{self, State, StoreError};

/// How often the copy is checked while the wallet is open.
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(in crate::real) struct AutoFile {
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    last_at: Option<Timestamp>,
    /// The last try's failure, as a code (`folder_not_found`, `no_space`, `no_access`, `other`).
    #[serde(default)]
    failed: Option<String>,
    /// The interface's settings to carry, as the last `SetAutoBackup` or save gave them.
    #[serde(default)]
    settings: Option<String>,
}

/// The settings file, `auto-backup.json`.
pub(in crate::real) type AutoStore = json_file::Store<AutoFile>;

#[derive(Default)]
pub(in crate::real) struct AutoState {
    /// When it was last checked; `None` after an unlock: checked at once.
    checked: Option<std::time::Instant>,
    running: bool,
}

fn failed_code(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::NotFound => "folder_not_found",
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
            "no_access"
        }
        std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded => "no_space",
        _ => "other",
    }
}

fn failed_of(code: &str) -> AutoFailed {
    match code {
        "folder_not_found" => AutoFailed::FolderNotFound,
        "no_access" => AutoFailed::NoAccess,
        "no_space" => AutoFailed::NoSpace,
        _ => AutoFailed::Other,
    }
}

/// `YYYY-MM-DD` of `unix` (UTC), for the copy's name.
pub(in crate::real) fn day_of(unix: Timestamp) -> String {
    // Howard Hinnant's days-to-civil.
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// What a refused change of the settings answers: set aside, the line the screen shows.
fn refused(e: StoreError) -> CoreError {
    match e {
        StoreError::SetAside => {
            CoreError::NotAvailable(tr!("The backup setting could not be read").into())
        }
        StoreError::Write(why) => CoreError::NotAvailable(why),
    }
}

impl Real {
    /// Whether the settings file read; `false` while it is set aside.
    fn auto_readable(&self) -> bool {
        self.auto_file.state() == State::Readable
    }

    /// The snapshot's view of the automatic copies: `None` when off, and while the file is set
    /// aside (its banner says so, `LocalFile::AutoBackup`).
    pub(in crate::real) fn auto_backup_view(&self) -> Option<AutoBackup> {
        if !self.auto_readable() {
            return None;
        }
        let file = self.auto_file.read(AutoFile::clone);
        Some(AutoBackup {
            folder: file.folder?,
            last_at: file.last_at,
            failed: file.failed.as_deref().map(failed_of),
        })
    }

    /// `Command::SetAutoBackup`: on — into `folder`, the first copy at once (UC-4); off.
    pub(in crate::real) fn set_auto_backup(
        &mut self,
        folder: Option<PathBuf>,
        settings: Option<String>,
    ) -> Result<(), CoreError> {
        if self.unlocked.is_none() {
            return Err(CoreError::NotAvailable(tr!("The wallet is locked").into()));
        }
        let mut state = self.auto.lock().unwrap_or_else(|e| e.into_inner());
        self.auto_file
            .update(|file| {
                file.folder = folder.map(|f| f.to_string_lossy().into_owned());
                file.failed = None;
                file.last_at = None;
                if settings.is_some() {
                    file.settings = settings;
                }
                ((), true)
            })
            .map_err(refused)?;
        state.checked = None;
        Ok(())
    }

    /// A manual save's settings are the automatic copies' from now on.
    pub(in crate::real) fn auto_backup_settings(&self, settings: &Option<String>) {
        // Set aside or not written: the copies keep the settings they had (the banner, or the
        // next save, says the rest).
        let _ = self.auto_file.update(|file| {
            let changed = file.folder.is_some() && settings.is_some() && file.settings != *settings;
            if changed {
                file.settings = settings.clone();
            }
            ((), changed)
        });
    }

    /// The lock: the next unlock checks at once.
    pub(in crate::real) fn auto_backup_locked(&self) {
        self.auto.lock().unwrap_or_else(|e| e.into_inner()).checked = None;
    }

    /// From the core's loop: a copy when one is due, written off the loop's thread; its outcome
    /// goes out as `Done { what: DoneWhat::AutoBackup }` and into the next snapshot.
    pub(in crate::real) fn auto_backup_tick(&mut self) {
        let Some(key) = self
            .backup_key
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            return;
        };
        let file = {
            let mut state = self.auto.lock().unwrap_or_else(|e| e.into_inner());
            if state.running || state.checked.is_some_and(|c| c.elapsed() < CHECK_EVERY) {
                return;
            }
            // Set aside: no copy, and nothing written over the file, until the person says.
            if !self.auto_readable() {
                return;
            }
            state.checked = Some(std::time::Instant::now());
            self.auto_file.read(AutoFile::clone)
        };
        let now = now();
        let Some(folder) = file.folder.clone() else {
            return;
        };
        if !auto::due(file.last_at, now) {
            return;
        }
        let Ok(contents) = self.backup_contents(file.settings.clone()) else {
            return;
        };
        let name = self
            .wallet_entries()
            .into_iter()
            .find(|w| w.id == self.wallet)
            .map(|w| w.name)
            .unwrap_or_default(); // default-ok: a wallet name for the file name: none is empty
        self.auto.lock().unwrap_or_else(|e| e.into_inner()).running = true;
        let (auto_state, store) = (self.auto.clone(), self.auto_file.clone());
        let (dirty, out, wake) = (self.dirty.clone(), self.out.clone(), self.wake.clone());
        tokio::task::spawn_blocking(move || {
            let day = day_of(now);
            let written = auto::write(
                std::path::Path::new(&folder),
                &key,
                network(),
                (&name, &day),
                &contents,
            );
            let mut state = auto_state.lock().unwrap_or_else(|e| e.into_inner());
            state.running = false;
            let mut failed = None;
            let result = match &written {
                Ok(done) => {
                    // An old copy that could not be deleted stays: said to the journal by its
                    // code only, no plate (review r1, step 2 Must).
                    if done.rotate_failed {
                        evolite_common::diag::record(evolite_common::diag::Record::Storage {
                            file: "auto-backup".into(),
                            op: "auto rotate",
                            error: "rotate_failed",
                        });
                    }
                    Ok(())
                }
                Err(e) => {
                    failed = Some(failed_code(e).to_string());
                    Err(CoreError::NotAvailable(failed_code(e).into()))
                }
            };
            // Set aside meanwhile: nothing written over it; the next unlock checks again.
            let _ = store.update(|f| {
                if written.is_ok() {
                    f.last_at = Some(now);
                }
                f.failed = failed;
                ((), true)
            });
            drop(state);
            dirty.store(true, Ordering::Relaxed);
            let _ = out.send(Event::Done {
                what: DoneWhat::AutoBackup,
                result,
                ask: None,
            });
            wake();
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_day_of_a_time() {
        assert_eq!(super::day_of(0), "1970-01-01");
        assert_eq!(super::day_of(1_763_683_200), "2025-11-21");
        assert_eq!(super::day_of(1_709_164_800), "2024-02-29");
    }
}
