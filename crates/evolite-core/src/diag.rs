//! The diagnostic journal: what happened, for a problem
//! report, and nothing that is the person's. Records are typed — numbers, flags and names from
//! the code — so a phrase, a key, an address, an amount or a name has nowhere to go. The only
//! text is a panic's and a sync error's, and it goes through [`scrub()`] first.
//!
//! One JSON line per record in `logs/evolite-YYYY-MM-DD[.n].log` of the data folder; a file is
//! at most 2 MB, at most 5 files are kept, none older than 7 days. A thread writes them: no
//! frame and no core loop waits for the disk. A journal that cannot be written loses records,
//! never anything else.

mod core_events;
mod scrub;

pub use core_events::error_kind;
pub(crate) use core_events::{Watch, answer, command};
pub use scrub::{phrase_words, scrub};

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_FILES: usize = 5;
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What the app is and where it runs, at the start.
#[derive(Clone, Debug, PartialEq)]
pub struct Build {
    pub version: &'static str,
    pub commit: &'static str,
    /// `release` or `debug`.
    pub profile: &'static str,
    pub os: &'static str,
    pub os_version: String,
    pub language: &'static str,
    pub theme: &'static str,
    /// Points per pixel of the window, e.g. 1.5.
    pub scale: f32,
    pub network: &'static str,
    /// "Own node" is set: whether, never which.
    pub own_node: bool,
    pub demo: bool,
}

/// One record of the journal.
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    Start(Build),
    /// The sync's stage, heights and peers, when the stage or the number of peers changes.
    Sync {
        stage: &'static str,
        headers: u32,
        filters: u32,
        tip: u32,
        chainlocked: bool,
        peers: u32,
    },
    /// A new sync error, scrubbed.
    SyncError {
        text: String,
    },
    /// A command went to the core.
    Command {
        name: &'static str,
    },
    /// The core answered: what, and `ok` or the kind of error.
    Answer {
        what: &'static str,
        outcome: &'static str,
    },
    Vault {
        state: &'static str,
    },
    /// A picture the renderer did not draw, and why.
    Art {
        reason: &'static str,
    },
    Screen {
        name: &'static str,
    },
    SlowFrame {
        ms: u64,
    },
    /// A core file that did not save: its name only, what was done and the I/O error's kind.
    Storage {
        file: String,
        op: &'static str,
        error: &'static str,
    },
    /// A Platform operation failed: what it was and the error's kind (its type names only,
    /// as `Sdk/Protocol`), never its text, ids or amounts.
    Platform {
        op: &'static str,
        error: String,
    },
    /// A panic: where, and its message and stack, scrubbed.
    Panic {
        location: String,
        message: String,
        backtrace: String,
    },
    /// The window did not open (no graphics adapter, a driver error): the error, scrubbed.
    StartFailed {
        text: String,
    },
    /// The last run ended without closing its window (a crash, a reset): the saved window
    /// size and placement were dropped, so a maximized window does not open again at once.
    WindowReset,
    /// What the load did with a wallet's saved DashPay backfill record (BUG-77): `kept`, `none`
    /// or `rejected_*` and why, and how many contacts the record taken back covers.
    Backfill {
        outcome: &'static str,
        contacts: u32,
    },
    /// The store's spent coins claimed at the load (BUG-77): claimed, spent but with no spender
    /// in a block (not claimed), and claimed but still among the wallet's coins.
    SpentClaims {
        claimed: u32,
        not_in_block: u32,
        still_held: u32,
    },
}

impl Record {
    /// The JSON line, without the time.
    fn fields(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Record::Start(b) => json!({
                "k": "start", "version": b.version, "commit": b.commit, "profile": b.profile,
                "os": b.os, "osVersion": b.os_version, "language": b.language,
                "theme": b.theme, "scale": b.scale, "network": b.network,
                "ownNode": b.own_node, "demo": b.demo,
            }),
            Record::Sync {
                stage,
                headers,
                filters,
                tip,
                chainlocked,
                peers,
            } => json!({
                "k": "sync", "stage": stage, "headers": headers, "filters": filters,
                "tip": tip, "chainlocked": chainlocked, "peers": peers,
            }),
            Record::SyncError { text } => json!({"k": "sync_error", "text": scrub(text)}),
            Record::Command { name } => json!({"k": "command", "name": name}),
            Record::Answer { what, outcome } => {
                json!({"k": "answer", "what": what, "outcome": outcome})
            }
            Record::Vault { state } => json!({"k": "vault", "state": state}),
            Record::Art { reason } => json!({"k": "art", "reason": reason}),
            Record::Screen { name } => json!({"k": "screen", "name": name}),
            Record::SlowFrame { ms } => json!({"k": "slow_frame", "ms": ms}),
            Record::Storage { file, op, error } => {
                json!({"k": "storage", "file": scrub(file), "op": op, "error": error})
            }
            Record::Panic {
                location,
                message,
                backtrace,
            } => json!({
                "k": "panic", "location": scrub(location), "message": scrub(message),
                "backtrace": scrub(backtrace),
            }),
            Record::StartFailed { text } => json!({"k": "start_failed", "text": scrub(text)}),
            Record::Platform { op, error } => {
                json!({"k": "platform", "op": op, "error": error})
            }
            Record::WindowReset => json!({"k": "window_reset"}),
            Record::Backfill { outcome, contacts } => {
                json!({"k": "backfill", "outcome": outcome, "contacts": contacts})
            }
            Record::SpentClaims {
                claimed,
                not_in_block,
                still_held,
            } => json!({
                "k": "spent_claims", "claimed": claimed, "notInBlock": not_in_block,
                "stillHeld": still_held,
            }),
        }
    }

    /// The line written: the time first, then the fields.
    fn line(&self, at: SystemTime) -> String {
        let mut fields = self.fields();
        let mut line = serde_json::Map::new();
        line.insert("t".into(), timestamp(at).into());
        if let Some(map) = fields.as_object_mut() {
            line.append(map);
        }
        serde_json::Value::Object(line).to_string()
    }
}

/// The kind of an I/O error, without its text (which may carry a path).
pub fn io_error_kind(error: &std::io::Error) -> &'static str {
    use std::io::ErrorKind::*;
    match error.kind() {
        NotFound => "not_found",
        PermissionDenied => "permission_denied",
        AlreadyExists => "already_exists",
        StorageFull => "storage_full",
        ReadOnlyFilesystem => "read_only",
        IsADirectory => "is_a_directory",
        InvalidData => "invalid_data",
        Interrupted => "interrupted",
        _ => "other",
    }
}

/// A Platform operation of the core that failed, by the kind of its error: `debug` is the
/// error's `{:?}` form, of which only the leading type names are kept.
pub fn platform_failed(op: &'static str, debug: &str) {
    record(Record::Platform {
        op,
        error: error_kind_of_debug(debug),
    });
}

/// `Sdk(Protocol(ConsensusError(..)))` → `Sdk/Protocol/ConsensusError`: up to three nested
/// variant names of a `{:?}` form, nothing of the data inside.
pub fn error_kind_of_debug(debug: &str) -> String {
    let mut names = Vec::new();
    let mut rest = debug.trim_start();
    while names.len() < 3 {
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_uppercase()) {
            break;
        }
        rest = &rest[name.len()..];
        names.push(name);
        match rest.strip_prefix('(') {
            Some(inner) => rest = inner.trim_start(),
            None => break,
        }
    }
    if names.is_empty() {
        "other".into()
    } else {
        names.join("/")
    }
}

/// A file of the core that did not save: its name only.
pub fn storage_failed(path: &Path, op: &'static str, error: &std::io::Error) {
    record(Record::Storage {
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        op,
        error: io_error_kind(error),
    });
}

/// `2026-11-01T12:00:00.123Z`.
fn timestamp(at: SystemTime) -> String {
    let d = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (y, m, day) = civil(days as i64);
    format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60,
        d.subsec_millis()
    )
}

/// The date of a day number since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

struct Journal {
    dir: PathBuf,
    lines: Mutex<Sender<String>>,
}

static JOURNAL: OnceLock<Journal> = OnceLock::new();

/// Starts the journal in `dir` (made if needed), removes files past the limits. Once per run;
/// later calls are ignored.
pub fn init(dir: &Path) {
    if JOURNAL.get().is_some() {
        return;
    }
    let _ = std::fs::create_dir_all(dir);
    prune(dir, SystemTime::now());
    let (tx, rx) = channel::<String>();
    let folder = dir.to_owned();
    let started = std::thread::Builder::new()
        .name("evolite-journal".into())
        .spawn(move || {
            let mut writer = Writer::new(folder);
            while let Ok(line) = rx.recv() {
                writer.write(&line);
            }
        });
    if started.is_ok() {
        let _ = JOURNAL.set(Journal {
            dir: dir.to_owned(),
            lines: Mutex::new(tx),
        });
    }
}

/// Keeps `record`, when the journal is on.
pub fn record(record: Record) {
    if let Some(journal) = JOURNAL.get() {
        let line = record.line(SystemTime::now());
        let _ = journal
            .lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(line);
    }
}

/// Writes `record` now, on this thread: for a panic, after which the writer may not run.
pub fn record_now(record: Record) {
    if let Some(journal) = JOURNAL.get() {
        let mut writer = Writer::new(journal.dir.clone());
        writer.write(&record.line(SystemTime::now()));
    }
}

/// The journal's folder, when it is on.
pub fn dir() -> Option<PathBuf> {
    JOURNAL.get().map(|j| j.dir.clone())
}

/// The journal's files, oldest first.
pub fn files() -> Vec<PathBuf> {
    dir().map(|d| journal_files(&d)).unwrap_or_default()
}

/// "Clear journal": every file goes; what comes next starts a new one.
pub fn clear() -> std::io::Result<()> {
    for file in files() {
        std::fs::remove_file(file)?;
    }
    Ok(())
}

/// The day and number of a journal file (`evolite-2026-11-01.3.log` → ("2026-11-01", 3)), or
/// `None` for any other file.
fn order(path: &Path) -> Option<(String, u32)> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_prefix("evolite-")?.strip_suffix(".log")?;
    let (day, n) = match stem.split_once('.') {
        Some((day, n)) => (day, n.parse().ok()?),
        None => (stem, 0),
    };
    (day.len() == 10).then(|| (day.to_owned(), n))
}

/// The journal files in `dir`, oldest first.
fn journal_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<((String, u32), PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| Some((order(&p)?, p)))
        .collect();
    files.sort();
    files.into_iter().map(|(_, p)| p).collect()
}

/// Removes files older than [`MAX_AGE`] and all but the newest [`MAX_FILES`].
fn prune(dir: &Path, now: SystemTime) {
    let files = journal_files(dir);
    let excess = files.len().saturating_sub(MAX_FILES);
    for (i, file) in files.iter().enumerate() {
        let old = std::fs::metadata(file)
            .and_then(|m| m.modified())
            .is_ok_and(|t| now.duration_since(t).unwrap_or_default() > MAX_AGE);
        if i < excess || old {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// Appends lines to today's file, moving to the next one past [`MAX_FILE`].
struct Writer {
    dir: PathBuf,
    current: Option<PathBuf>,
}

impl Writer {
    fn new(dir: PathBuf) -> Self {
        Self { dir, current: None }
    }

    /// Today's file with room for `len` more bytes: the last one of today, else the next.
    fn file_for(&mut self, len: u64) -> PathBuf {
        let day = timestamp(SystemTime::now())[..10].to_owned();
        let fits = |p: &Path| std::fs::metadata(p).map_or(0, |m| m.len()) + len <= MAX_FILE;
        let today = |p: &Path| order(p).is_some_and(|(d, _)| d == day);
        if self.current.as_deref().is_none_or(|p| !today(p)) {
            // At the start, or past midnight: today's last file, if there is one.
            self.current = journal_files(&self.dir).into_iter().rfind(|p| today(p));
        }
        if let Some(current) = &self.current
            && fits(current)
        {
            return current.clone();
        }
        let next = self
            .current
            .as_deref()
            .and_then(order)
            .map_or(0, |(_, n)| n + 1);
        let path = self.dir.join(if next == 0 {
            format!("evolite-{day}.log")
        } else {
            format!("evolite-{day}.{next}.log")
        });
        // Made before the limits are applied, so they count it.
        let _ = std::fs::File::create(&path);
        prune(&self.dir, SystemTime::now());
        self.current = Some(path.clone());
        path
    }

    fn write(&mut self, line: &str) {
        let path = self.file_for(line.len() as u64 + 1);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_platform_error_keeps_its_type_names_only() {
        assert_eq!(
            error_kind_of_debug("AssetLockAlreadyConsumed(OutPoint { txid: 0a1b, vout: 0 })"),
            "AssetLockAlreadyConsumed/OutPoint"
        );
        assert_eq!(
            error_kind_of_debug("Sdk(Protocol(ConsensusError(Basic(X))))"),
            "Sdk/Protocol/ConsensusError"
        );
        assert_eq!(
            error_kind_of_debug("Sdk(Generic(\"yJ1kAddress 12 DASH\"))"),
            "Sdk/Generic"
        );
        assert_eq!(error_kind_of_debug("\"text\""), "other");
        let line = Record::Platform {
            op: "top_up",
            error: "Sdk/Generic".into(),
        }
        .line(UNIX_EPOCH);
        assert!(line.contains(r#""k":"platform","op":"top_up","error":"Sdk/Generic""#));
    }

    #[test]
    fn writes_the_time_and_typed_fields_only() {
        let at = UNIX_EPOCH + Duration::from_millis(1_761_998_400_123);
        let line = Record::Answer {
            what: "done:send",
            outcome: "not_enough",
        }
        .line(at);
        assert_eq!(
            line,
            r#"{"t":"2025-11-01T12:00:00.123Z","k":"answer","what":"done:send","outcome":"not_enough"}"#
        );
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn the_text_in_a_record_is_scrubbed() {
        let line = Record::Panic {
            location: r"C:\Users\Ελένη\evolite\src\app.rs:12:5".into(),
            message: "called unwrap on yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ".into(),
            backtrace: String::new(),
        }
        .line(UNIX_EPOCH);
        assert!(!line.contains("Ελένη") && !line.contains("yMrDQ"), "{line}");
        let line = Record::SyncError {
            text: "peer 192.168.1.10:19999 disconnected".into(),
        }
        .line(UNIX_EPOCH);
        assert!(!line.contains("192.168"), "{line}");
    }

    #[test]
    fn rotates_and_keeps_at_most_five_files() {
        let dir = std::env::temp_dir().join(format!("evolite-diag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut writer = Writer::new(dir.clone());
        let line = "x".repeat(400 * 1024);
        for _ in 0..40 {
            writer.write(&line);
        }
        let files = journal_files(&dir);
        assert!(files.len() <= MAX_FILES, "{files:?}");
        assert!(
            files
                .iter()
                .all(|f| std::fs::metadata(f).unwrap().len() <= MAX_FILE)
        );
        // Files that are not the journal's are left alone.
        std::fs::write(dir.join("notes.txt"), b"mine").unwrap();
        prune(&dir, SystemTime::now() + MAX_AGE * 2);
        assert!(journal_files(&dir).is_empty(), "past seven days: gone");
        assert!(dir.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
