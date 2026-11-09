//! The core's small files in the data folder (history, contacts' paid addresses, the address
//! book, names being registered, the own Core node): written whole beside the file, flushed to the disk,
//! then renamed over it, so a crash or a power cut leaves the old file or the new one — never
//! an empty one. An empty paid-addresses file would pay a contact to an address used before.

use std::io::{Read, Write};
use std::path::Path;

/// The largest of these files read back. Each holds a few kilobytes to a few hundred; a bigger
/// one was put there, and reading it whole would only fill the memory.
pub const MAX_READ: u64 = 16 << 20;

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

/// Writes `bytes` to `path` that way: beside it as `<name>.tmp`, flushed, renamed over it. A
/// failure goes to the journal too (the file's name and the kind of error only).
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_whole(path, bytes).inspect_err(|e| evolite_common::diag::storage_failed(path, "write", e))
}

fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        crate::folders::create(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
