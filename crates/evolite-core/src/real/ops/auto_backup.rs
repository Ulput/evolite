//! FEAT-0045 step 2 (R-12): the automatic copies. Checked after the unlock and then each hour
//! while the wallet is open, off the loop's thread; one is written when a week has passed since
//! the last. Nothing is written once the wallet locks: its key goes with the lock. The folder and
//! the times live in the wallet's `auto-backup.json` — not in a backup: a new computer, a new
//! folder. The journal has the outcome's kind only (R-14).

use crate::real::*;
use evolite_api::backup::{AutoBackup, AutoFailed};
use evolite_local::backup::auto;

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

#[derive(Default)]
pub(in crate::real) struct AutoState {
    file: Option<AutoFile>,
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

impl Real {
    fn auto_path(&self) -> PathBuf {
        self.paths.auto_backup.clone()
    }

    fn auto_load(&self, state: &mut AutoState) -> AutoFile {
        let path = self.auto_path();
        state
            .file
            .get_or_insert_with(|| {
                evolite_local::json_file::read_cache(&path, |b| serde_json::from_slice(b).ok())
                    .unwrap_or_default()
            })
            .clone()
    }

    fn auto_save(&self, state: &mut AutoState, file: AutoFile) {
        let _ = evolite_local::json_file::save(&self.auto_path(), &file);
        state.file = Some(file);
    }

    /// The snapshot's view of the automatic copies.
    pub(in crate::real) fn auto_backup_view(&self) -> Option<AutoBackup> {
        let mut state = self.auto.lock().unwrap_or_else(|e| e.into_inner());
        let file = self.auto_load(&mut state);
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
        let mut file = self.auto_load(&mut state);
        file.folder = folder.map(|f| f.to_string_lossy().into_owned());
        file.failed = None;
        file.last_at = None;
        if settings.is_some() {
            file.settings = settings;
        }
        self.auto_save(&mut state, file);
        state.checked = None;
        Ok(())
    }

    /// A manual save's settings are the automatic copies' from now on.
    pub(in crate::real) fn auto_backup_settings(&self, settings: &Option<String>) {
        let mut state = self.auto.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = self.auto_load(&mut state);
        if file.folder.is_some() && settings.is_some() && file.settings != *settings {
            file.settings = settings.clone();
            self.auto_save(&mut state, file);
        }
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
            state.checked = Some(std::time::Instant::now());
            self.auto_load(&mut state)
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
            .unwrap_or_default();
        self.auto.lock().unwrap_or_else(|e| e.into_inner()).running = true;
        let (auto_state, path) = (self.auto.clone(), self.auto_path());
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
            let mut file = state.file.clone().unwrap_or_default();
            let result = match &written {
                Ok(done) => {
                    file.last_at = Some(now);
                    file.failed = None;
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
                    file.failed = Some(failed_code(e).into());
                    Err(CoreError::NotAvailable(failed_code(e).into()))
                }
            };
            let _ = evolite_local::json_file::save(&path, &file);
            state.file = Some(file);
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
