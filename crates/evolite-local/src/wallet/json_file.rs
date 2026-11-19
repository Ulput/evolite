//! The core's small files in the data folder (history, contacts' paid addresses, the address
//! book, names being registered, the own Core node): written whole beside the file, flushed to the disk,
//! then renamed over it, so a crash or a power cut leaves the old file or the new one — never
//! an empty one. An empty paid-addresses file would pay a contact to an address used before.
//!
//! [`Store`] (TECH-json-file-aside) is one file in memory: read once, changed under a lock
//! ([`Store::update`]: read, change, write as one step), never written over when it did not
//! read — it is set aside as `<name>.unreadable-<unix>` and nothing more is written until the
//! person decides ([`Store::start_anew`]). [`Entries`] is a list read entry by entry: an entry
//! this build cannot read (a kind a newer build added) is kept as it was and written back.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The largest of these files read back. Each holds a few kilobytes to a few hundred; a bigger
/// one was put there, and reading it whole would only fill the memory.
pub const MAX_READ: u64 = 16 << 20;

/// How many times a file that is there is tried before it is set aside: an antivirus or a
/// backup holds it for a moment.
const READ_TRIES: u32 = 3;

/// The pause between those tries.
const READ_PAUSE: std::time::Duration = std::time::Duration::from_millis(100);

/// Reads `path` whole, if it is no bigger than [`MAX_READ`]; a bigger file is `InvalidData`, as
/// a damaged one. A missing file is `NotFound`, as with `std::fs::read`.
pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_READ + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_READ {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file too big",
        ));
    }
    Ok(bytes)
}

/// Writes `value` as pretty JSON to `path` that way.
pub fn save(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    write(path, &text).map_err(|e| e.to_string())
}

/// Writes `bytes` to `path` that way: beside it as a temporary file of this write's own
/// (`<name>.<pid>.<n>.tmp`: two writers never share one), flushed, renamed over it, and on Unix
/// the folder flushed too, so the rename itself survives a power cut. A failure goes to the
/// journal too (the file's name and the kind of error only).
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_whole(path, bytes).inspect_err(|e| evolite_common::diag::storage_failed(path, "write", e))
}

/// The temporary file of the next write of `path`.
fn tmp_of(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    PathBuf::from(tmp)
}

/// Deletes the temporary files of `path` another process left (`<name>.<pid>.<n>.tmp` of a pid
/// not this one's), a crash's leftovers (review r1, Should 2). One window per data folder, so
/// another pid is a process gone; this process's own may be another thread's write under way.
fn remove_orphans(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let own = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(middle) = file
            .strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(".tmp"))
        else {
            continue;
        };
        let mut parts = middle.split('.');
        let (Some(pid), Some(n), None) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let numbers = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
        if numbers(pid) && numbers(n) && pid != own {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        crate::folders::create(dir)?;
    }
    let tmp = tmp_of(path);
    let written = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return written;
    }
    // The temporary file of builds before TECH-json-file-aside, left by a crash.
    let mut legacy = path.as_os_str().to_owned();
    legacy.push(".tmp");
    let _ = std::fs::remove_file(legacy);
    remove_orphans(path);
    // The file is new already: a folder that would not flush is said in the journal, and the
    // write is not called failed (review r1, Should 1: a store would keep the old value and the
    // next change would drop this one).
    #[cfg(unix)]
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::File::open(dir).and_then(|d| d.sync_all())
    {
        evolite_common::diag::storage_failed(path, "sync dir", &e);
    }
    Ok(())
}

/// Why a [`Store`]'s file was set aside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// It could not be read, try after try (no access, held by another program).
    Read,
    /// It is not what it should be (damaged, or another format).
    Parse,
    /// It is bigger than [`MAX_READ`].
    Size,
}

impl Unreadable {
    fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Parse => "parse",
            Self::Size => "size",
        }
    }
}

/// A [`Store`]'s file as it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Read (or not there: empty), and kept up to date with what is written.
    Readable,
    /// It did not read and was moved to `path` (or, when even that failed, left where it was):
    /// nothing is written until [`Store::start_anew`].
    SetAside { path: PathBuf, reason: Unreadable },
}

/// Why a change was not kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// The file was set aside: changes wait for the person.
    SetAside,
    /// It could not be written: the error's text (the file's name and its kind).
    Write(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SetAside => f.write_str("the file did not read and was set aside"),
            Self::Write(why) => f.write_str(why),
        }
    }
}

struct Inner<T> {
    /// `None` until first read.
    value: Option<T>,
    state: State,
    /// A change kept in memory whose write failed ([`Store::update_kept`]): the next change
    /// writes it even when it changes nothing.
    unsaved: bool,
}

/// One file of the data folder in memory (see the module): what it holds, read the first time
/// it is needed, and a lock that makes each change one step.
pub struct Store<T> {
    path: PathBuf,
    inner: Mutex<Inner<T>>,
    /// Applied to what was read, before anything looks at it (a journal gives its lines ids).
    prepare: fn(&mut T),
}

impl<T> Store<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Default + Clone,
{
    pub fn new(path: PathBuf) -> Self {
        Self::with_prepare(path, |_| {})
    }

    /// [`Self::new`] whose file, once read, goes through `prepare` first.
    pub fn with_prepare(path: PathBuf, prepare: fn(&mut T)) -> Self {
        Self {
            path,
            inner: Mutex::new(Inner {
                value: None,
                state: State::Readable,
                unsaved: false,
            }),
            prepare,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `look` at what the file holds; a missing file, or one set aside, holds the default.
    pub fn read<R>(&self, look: impl FnOnce(&T) -> R) -> R {
        let mut inner = self.lock();
        self.load(&mut inner);
        look(inner.value.as_ref().expect("loaded"))
    }

    /// Changes what the file holds with `change` and writes it, as one step no other change of
    /// this store comes between. `change` works on a copy: nothing changes in memory unless it
    /// is on the disk. Nothing is written when `change` says nothing changed (`false`), nor
    /// into a file set aside. `change` runs under this store's lock: it must not use the same
    /// store (that would wait for itself).
    pub fn update<R>(&self, change: impl FnOnce(&mut T) -> (R, bool)) -> Result<R, StoreError> {
        let mut inner = self.lock();
        self.load(&mut inner);
        if inner.state != State::Readable {
            return Err(StoreError::SetAside);
        }
        let mut value = inner.value.clone().expect("loaded");
        let (answer, changed) = change(&mut value);
        if changed || inner.unsaved {
            save(&self.path, &value).map_err(StoreError::Write)?;
            inner.value = Some(value);
            inner.unsaved = false;
        }
        Ok(answer)
    }

    /// [`Self::update`] whose change stays in memory even when it could not be written (the
    /// journal's rule, BUG-58 r2): the next change writes it then, and until then what is read
    /// has it. The second half of the answer is whether the file now has everything. Refused,
    /// as any change, while the file is set aside.
    pub fn update_kept<R>(
        &self,
        change: impl FnOnce(&mut T) -> (R, bool),
    ) -> Result<(R, Result<(), StoreError>), StoreError> {
        let mut inner = self.lock();
        self.load(&mut inner);
        if inner.state != State::Readable {
            return Err(StoreError::SetAside);
        }
        let (answer, changed) = change(inner.value.as_mut().expect("loaded"));
        let mut written = Ok(());
        if changed || inner.unsaved {
            written =
                save(&self.path, inner.value.as_ref().expect("loaded")).map_err(StoreError::Write);
            inner.unsaved = written.is_err();
        }
        Ok((answer, written))
    }

    /// The file as it stands: whether it was set aside, where and why.
    pub fn state(&self) -> State {
        let mut inner = self.lock();
        self.load(&mut inner);
        inner.state.clone()
    }

    /// The person chose to go on without the file set aside: empty, written anew from the next
    /// change. The file aside stays where it is.
    pub fn start_anew(&self) {
        let mut inner = self.lock();
        inner.value = Some(T::default());
        inner.state = State::Readable;
        inner.unsaved = false;
    }

    /// «Try again» (the analyst, BUG-204): the file set aside goes back under its own name and
    /// is read again — for a file another program only held for a while. One that still does not
    /// read is set aside again. Nothing is moved when a new file has taken its name since.
    pub fn try_again(&self) -> State {
        let mut inner = self.lock();
        let State::SetAside { path: aside, .. } = inner.state.clone() else {
            return State::Readable;
        };
        if aside != self.path {
            if self.path.exists() {
                return inner.state.clone();
            }
            if let Err(e) = std::fs::rename(&aside, &self.path) {
                evolite_common::diag::storage_failed(&self.path, "back", &e);
                return inner.state.clone();
            }
        }
        inner.value = None;
        inner.unsaved = false;
        self.load(&mut inner);
        inner.state.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner<T>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn load(&self, inner: &mut Inner<T>) {
        if inner.value.is_some() {
            return;
        }
        let (value, state) = match read_tries(&self.path, READ_TRIES, READ_PAUSE, read) {
            Ok(None) => (T::default(), State::Readable),
            Ok(Some(bytes)) => match serde_json::from_slice(&bytes) {
                Ok(value) => (value, State::Readable),
                Err(_) => (T::default(), set_aside(&self.path, Unreadable::Parse)),
            },
            Err(reason) => (T::default(), set_aside(&self.path, reason)),
        };
        let mut value = value;
        (self.prepare)(&mut value);
        inner.value = Some(value);
        inner.state = state;
    }
}

/// A cache file that the network can fill again (BUG-235: the chain's lock statuses, mined
/// blocks, contact rescans, Platform marks, the token search): what `parse` makes of it, or
/// `None` — no file, or one that did not read. One that did not read (the tries of [`Store`],
/// too big, or `parse` refuses it) is set aside as `<name>.unreadable-<unix>`, with its name and
/// why in the journal, as a [`Store`]'s: what it held stays on the disk, and the next write
/// starts a new file instead of writing over it. Unlike a [`Store`], nothing waits for the
/// person — what it held is read from the network again.
pub fn read_cache<T>(path: &Path, parse: impl FnOnce(&[u8]) -> Option<T>) -> Option<T> {
    let bytes = match read_tries(path, READ_TRIES, READ_PAUSE, read) {
        Ok(None) => return None,
        Ok(Some(bytes)) => bytes,
        Err(reason) => {
            set_aside(path, reason);
            return None;
        }
    };
    let value = parse(&bytes);
    if value.is_none() {
        set_aside(path, Unreadable::Parse);
    }
    value
}

/// What the core does with any store's file that was set aside, whatever it holds (BUG-204).
pub trait SetAsideFile {
    fn state(&self) -> State;
    /// [`Store::try_again`].
    fn try_again(&self) -> State;
    /// [`Store::start_anew`].
    fn start_anew(&self);
}

impl<T> SetAsideFile for Store<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Default + Clone,
{
    fn state(&self) -> State {
        Store::state(self)
    }

    fn try_again(&self) -> State {
        Store::try_again(self)
    }

    fn start_anew(&self) {
        Store::start_anew(self)
    }
}

/// The file's bytes, `None` for no file, tried `tries` times `pause` apart while it is there and
/// cannot be read.
fn read_tries(
    path: &Path,
    tries: u32,
    pause: std::time::Duration,
    mut read: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<Option<Vec<u8>>, Unreadable> {
    let mut left = tries.max(1);
    loop {
        match read(path) {
            Ok(bytes) => return Ok(Some(bytes)),
            // No file, or a folder above it that is a file: nothing there to keep.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(None);
            }
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return Err(Unreadable::Size),
            Err(e) => {
                left -= 1;
                if left == 0 {
                    evolite_common::diag::storage_failed(path, "read", &e);
                    return Err(Unreadable::Read);
                }
                std::thread::sleep(pause);
            }
        }
    }
}

/// Moves a file that did not read to `<name>.unreadable-<unix>` (the next free second): what it
/// held stays on the disk and nothing is written over it. Only its name and why go to the
/// journal.
fn set_aside(path: &Path, reason: Unreadable) -> State {
    evolite_common::diag::record(evolite_common::diag::Record::Storage {
        file: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        op: "set aside",
        error: reason.name(),
    });
    let mut at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let aside = loop {
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".unreadable-{at}"));
        let aside = PathBuf::from(name);
        if !aside.exists() {
            break aside;
        }
        at += 1;
    };
    match std::fs::rename(path, &aside) {
        Ok(()) => State::SetAside {
            path: aside,
            reason,
        },
        Err(e) => {
            // Left where it is: still nothing is written over it.
            evolite_common::diag::storage_failed(path, "set aside", &e);
            State::SetAside {
                path: path.to_owned(),
                reason,
            }
        }
    }
}

/// A list read entry by entry (TECH-json-file-aside, item 2): the entries this build reads, and
/// those it cannot (a kind a newer build added), kept as they were and written back after the
/// others. A file that is no list at all does not read.
#[derive(Clone, Debug, PartialEq)]
pub struct Entries<E> {
    pub known: Vec<E>,
    unknown: Vec<serde_json::Value>,
}

impl<E> Default for Entries<E> {
    fn default() -> Self {
        Self {
            known: Vec::new(),
            unknown: Vec::new(),
        }
    }
}

impl<E> Entries<E> {
    /// How many entries this build could not read.
    pub fn unknown(&self) -> usize {
        self.unknown.len()
    }
}

impl<E: serde::Serialize> serde::Serialize for Entries<E> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(self.known.len() + self.unknown.len()))?;
        for entry in &self.known {
            seq.serialize_element(entry)?;
        }
        for entry in &self.unknown {
            seq.serialize_element(entry)?;
        }
        seq.end()
    }
}

impl<'de, E: serde::de::DeserializeOwned> serde::Deserialize<'de> for Entries<E> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut entries = Self::default();
        for value in Vec::<serde_json::Value>::deserialize(d)? {
            match E::deserialize(&value) {
                Ok(entry) => entries.known.push(entry),
                Err(_) => entries.unknown.push(value),
            }
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "evolite-json-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The names in `dir` that start with `prefix`, sorted.
    fn names(dir: &Path, prefix: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(prefix))
            .collect();
        names.sort();
        names
    }

    /// BUG-235: a cache that does not read is moved aside — kept, never written over — and
    /// read as none; a missing one is none with nothing moved; a good one is read.
    #[test]
    fn a_cache_that_does_not_read_is_set_aside_not_written_over() {
        let dir = dir("cache");
        let path = dir.join("lock-status.json");
        let parse = |b: &[u8]| serde_json::from_slice::<Vec<u32>>(b).ok();
        assert_eq!(read_cache(&path, parse), None);
        assert!(
            names(&dir, "lock-status.json.unreadable-").is_empty(),
            "nothing to move"
        );
        std::fs::write(&path, b"[1, 2").unwrap();
        assert_eq!(read_cache(&path, parse), None);
        assert!(!path.exists(), "moved, so the next write is a new file");
        let aside = names(&dir, "lock-status.json.unreadable-");
        assert_eq!(aside.len(), 1);
        assert_eq!(
            std::fs::read(dir.join(&aside[0])).unwrap(),
            b"[1, 2",
            "what it held"
        );
        save(&path, &vec![3u32]).unwrap();
        assert_eq!(read_cache(&path, parse), Some(vec![3]));
        // A file that parses but is not this build's (`parse` says no) is set aside too.
        assert_eq!(read_cache(&path, |_| None::<Vec<u32>>), None);
        assert_eq!(names(&dir, "lock-status.json.unreadable-").len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_too_big_is_not_read_into_memory() {
        let dir = std::env::temp_dir().join(format!("evolite-json-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.json");
        let file = std::fs::File::create(&path).unwrap();
        // Sparse where the system allows: no 16 MB written for real.
        file.set_len(MAX_READ + 1).unwrap();
        drop(file);
        assert_eq!(
            read(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        std::fs::write(&path, b"[]").unwrap();
        assert_eq!(read(&path).unwrap(), b"[]");
        assert_eq!(
            read(&dir.join("none.json")).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_whole_and_leaves_no_temporary_file() {
        let dir = std::env::temp_dir().join(format!("evolite-json-file-{}", std::process::id()));
        let path = dir.join("book.json");
        save(&path, &vec!["yA", "yB"]).unwrap();
        // A stale temporary file from a crash is written over.
        std::fs::write(path.with_extension("json.tmp"), b"half").unwrap();
        save(&path, &vec!["yC"]).unwrap();
        let back: Vec<String> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(back, ["yC"]);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// AC-T1: a file bigger than the limit is set aside for its size.
    #[test]
    fn a_file_too_big_is_set_aside() {
        let dir = dir("big-store");
        let path = dir.join("journal.json");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_READ + 1).unwrap();
        drop(file);
        let store = Store::<Vec<u32>>::new(path);
        assert!(matches!(
            store.state(),
            State::SetAside {
                reason: Unreadable::Size,
                ..
            }
        ));
        assert_eq!(names(&dir, "journal.json.unreadable-").len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// AC-T1: a damaged file is set aside, not written over, and a change to it is refused
    /// until the person starts anew; a missing file is simply empty.
    #[test]
    fn a_damaged_file_is_set_aside_and_not_written() {
        let dir = dir("aside");
        let path = dir.join("journal.json");
        let empty = Store::<Vec<u32>>::new(path.clone());
        assert_eq!(empty.read(|v| v.len()), 0);
        assert_eq!(empty.state(), State::Readable);
        assert!(names(&dir, "journal.json").is_empty(), "no file made");
        std::fs::write(&path, b"[1, 2, {half").unwrap();
        let store = Store::<Vec<u32>>::new(path.clone());
        assert_eq!(store.read(|v| v.len()), 0);
        let aside = names(&dir, "journal.json.unreadable-");
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(
            store.state(),
            State::SetAside {
                path: dir.join(&aside[0]),
                reason: Unreadable::Parse
            }
        );
        assert_eq!(std::fs::read(dir.join(&aside[0])).unwrap(), b"[1, 2, {half");
        assert_eq!(
            store.update(|v| {
                v.push(3);
                ((), true)
            }),
            Err(StoreError::SetAside)
        );
        assert!(!path.exists(), "nothing written");
        // The person chose to go on: a new file, the old one still aside.
        store.start_anew();
        store
            .update(|v| {
                v.push(3);
                ((), true)
            })
            .unwrap();
        assert_eq!(Store::<Vec<u32>>::new(path).read(Vec::clone), [3]);
        assert_eq!(names(&dir, "journal.json.unreadable-").len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// «Try again»: the file set aside goes back and is read; still damaged, it goes aside again
    /// (one file aside, not two).
    #[test]
    fn try_again_puts_the_file_back_and_reads_it() {
        let dir = dir("again");
        let path = dir.join("connected-apps.json");
        std::fs::write(&path, b"[1, 2").unwrap();
        let store = Store::<Vec<u32>>::new(path.clone());
        assert!(matches!(store.state(), State::SetAside { .. }));
        assert!(matches!(store.try_again(), State::SetAside { .. }));
        assert_eq!(names(&dir, "connected-apps.json.unreadable-").len(), 1);
        assert!(!path.exists());
        // The program that held it let go: it reads as it was.
        let State::SetAside { path: aside, .. } = store.state() else {
            panic!("set aside");
        };
        std::fs::write(&aside, b"[1, 2]").unwrap();
        assert_eq!(store.try_again(), State::Readable);
        assert_eq!(store.read(Vec::clone), [1, 2]);
        assert!(names(&dir, "connected-apps.json.unreadable-").is_empty());
        store
            .update(|v| {
                v.push(3);
                ((), true)
            })
            .unwrap();
        assert_eq!(Store::<Vec<u32>>::new(path).read(Vec::clone), [1, 2, 3]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// AC-T1: a read that fails three times sets the file aside; one that fails once reads at
    /// the second try.
    #[test]
    fn a_read_error_is_tried_three_times() {
        let dir = dir("tries");
        let path = dir.join("apps.json");
        std::fs::write(&path, b"[7]").unwrap();
        let denied = || std::io::Error::new(std::io::ErrorKind::PermissionDenied, "held");
        let mut calls = 0;
        let once = read_tries(&path, 3, std::time::Duration::ZERO, |p| {
            calls += 1;
            if calls == 1 { Err(denied()) } else { read(p) }
        });
        assert_eq!(once, Ok(Some(b"[7]".to_vec())));
        assert_eq!(calls, 2);
        let mut calls = 0;
        let never = read_tries(&path, 3, std::time::Duration::ZERO, |_| {
            calls += 1;
            Err(denied())
        });
        assert_eq!(never, Err(Unreadable::Read));
        assert_eq!(calls, 3);
        // A store over a file it cannot read (a folder in its place) sets it aside.
        let held = dir.join("held.json");
        std::fs::create_dir_all(held.join("in-the-way")).unwrap();
        let store = Store::<Vec<u32>>::new(held.clone());
        assert!(matches!(
            store.state(),
            State::SetAside {
                reason: Unreadable::Read,
                ..
            }
        ));
        assert!(store.update(|v| ((), v.is_empty())).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// AC-T2: writers of one store at once each keep their change, through temporary files
    /// of their own.
    #[test]
    fn two_writers_at_once_keep_both_changes() {
        let dir = dir("writers");
        let path = dir.join("name-requests.json");
        let store = Store::<Vec<u32>>::new(path.clone());
        std::thread::scope(|s| {
            for writer in 0..4u32 {
                let store = &store;
                s.spawn(move || {
                    for n in 0..25u32 {
                        store
                            .update(|all| {
                                all.push(writer * 100 + n);
                                ((), true)
                            })
                            .unwrap();
                    }
                });
            }
        });
        let all = Store::<Vec<u32>>::new(path.clone()).read(Vec::clone);
        assert_eq!(all.len(), 100, "no change lost");
        assert_eq!(names(&dir, "name-requests.json"), ["name-requests.json"]);
        assert_ne!(
            tmp_of(&path),
            tmp_of(&path),
            "each write its own temporary file"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// AC-T3: an entry of a kind this build does not know is read past, the others are there,
    /// and it is written back as it was.
    #[test]
    fn an_entry_of_an_unknown_kind_is_kept_as_it_was() {
        #[derive(Clone, serde::Serialize, serde::Deserialize, Debug, PartialEq)]
        enum Kind {
            Sent { to: String },
        }
        let dir = dir("unknown");
        let path = dir.join("journal.json");
        let later = serde_json::json!({"Later": {"what": 7}});
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!([
                {"Sent": {"to": "@a"}}, later, {"Sent": {"to": "@b"}}
            ]))
            .unwrap(),
        )
        .unwrap();
        let store = Store::<Entries<Kind>>::new(path.clone());
        assert_eq!(store.read(|e| (e.known.len(), e.unknown())), (2, 1));
        assert_eq!(store.state(), State::Readable);
        store
            .update(|e| {
                e.known.push(Kind::Sent { to: "@c".into() });
                ((), true)
            })
            .unwrap();
        let back: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(back.len(), 4);
        assert!(back.contains(&later), "{back:?}");
        // Not a list at all: set aside.
        std::fs::write(&path, b"{\"a\": 1}").unwrap();
        let store = Store::<Entries<Kind>>::new(path);
        assert!(matches!(store.state(), State::SetAside { .. }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// AC-T4: a write cut short between the temporary file and the rename leaves the old file
    /// whole, and the next write goes through a temporary file of its own.
    #[test]
    fn a_write_cut_short_leaves_the_old_file() {
        let dir = dir("cut");
        let path = dir.join("contact-addresses.json");
        let store = Store::<Vec<u32>>::new(path.clone());
        store
            .update(|v| {
                v.push(1);
                ((), true)
            })
            .unwrap();
        // What a crash leaves: the next temporary file half written, never renamed.
        let left = tmp_of(&path);
        std::fs::write(&left, b"[1, 2").unwrap();
        assert_eq!(Store::<Vec<u32>>::new(path.clone()).read(Vec::clone), [1]);
        store
            .update(|v| {
                v.push(2);
                ((), true)
            })
            .unwrap();
        assert_eq!(
            Store::<Vec<u32>>::new(path.clone()).read(Vec::clone),
            [1, 2]
        );
        assert_eq!(
            std::fs::read(&left).unwrap(),
            b"[1, 2",
            "not the next write's"
        );
        // A failed write changes nothing in memory either.
        let blocked = dir.join("wallet");
        std::fs::write(&blocked, b"").unwrap();
        let store = Store::<Vec<u32>>::new(blocked.join("journal.json"));
        assert!(matches!(
            store.update(|v| {
                v.push(9);
                ((), true)
            }),
            Err(StoreError::Write(_))
        ));
        assert_eq!(store.read(Vec::clone), Vec::<u32>::new());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Review r1, Should 2: a write deletes the temporary files a process gone left, and not this
    /// process's own (another thread may be writing one).
    #[test]
    fn a_write_removes_what_a_crashed_process_left() {
        let dir = dir("orphans");
        let path = dir.join("journal.json");
        let gone = dir.join("journal.json.1.0.tmp");
        let mine = dir.join(format!("journal.json.{}.999999.tmp", std::process::id()));
        let other = dir.join("journal.json.unreadable-1.tmp");
        for f in [&gone, &mine, &other] {
            std::fs::write(f, b"half").unwrap();
        }
        save(&path, &vec![1]).unwrap();
        assert!(!gone.exists(), "another pid's: gone");
        assert!(mine.exists(), "this process's: left");
        assert!(other.exists(), "not a temporary file of this write: left");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
