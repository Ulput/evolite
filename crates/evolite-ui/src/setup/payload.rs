//! The program's files inside the installer: appended to the installer's own executable, so one
//! file is the whole download and the build needs no second tool. `evolite-setup --pack OUT`
//! writes a copy of itself with `evolite.exe` and `evolite-renderer.exe` (from its own folder)
//! after it.
//!
//! Layout: the installer as built, then each file — name length (u16 LE), name (UTF-8), size
//! (u64 LE), SHA-256, bytes — then a footer: where the files start (u64 LE), how many (u32 LE)
//! and [`MAGIC`]. A file is written next to its place, checked against its hash and only then
//! renamed into place: a damaged or cut download installs nothing.

use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// The last bytes of a packed installer.
pub const MAGIC: &[u8; 16] = b"EVOLITE-SETUP-v1";
const FOOTER: u64 = 8 + 4 + 16;

/// One file of the payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub sha256: [u8; 32],
    /// Where its bytes start in the installer.
    offset: u64,
}

/// What a packed installer carries.
#[derive(Clone, Debug, PartialEq)]
pub struct Payload {
    /// The installer's own bytes: what the uninstaller is a copy of.
    pub stub_len: u64,
    pub entries: Vec<Entry>,
}

impl Payload {
    /// The sizes of a real build, for the screens' examples (`--screen`): nothing to extract.
    pub fn example() -> Payload {
        let entry = |name: &str, size| Entry {
            name: name.into(),
            size,
            sha256: [0; 32],
            offset: 0,
        };
        // The release build of 2026-11-01.
        Payload {
            stub_len: 14_803_968,
            entries: vec![
                entry("evolite.exe", 69_440_000),
                entry("evolite-renderer.exe", 4_026_368),
            ],
        }
    }

    /// The bytes all files take once installed.
    pub fn installed_size(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }
}

/// A file name the payload may carry: a plain name, no folder in it, nothing to climb out of
/// the install folder with.
pub fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// `stub` (the installer as built, without any payload) with `files` after it.
pub fn pack(stub: &[u8], files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut out = stub.to_vec();
    for (name, bytes) in files {
        if !plain_name(name) {
            return Err(format!("not a plain file name: {name}"));
        }
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(&Sha256::digest(bytes));
        out.extend_from_slice(bytes);
    }
    out.extend_from_slice(&(stub.len() as u64).to_le_bytes());
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(MAGIC);
    Ok(out)
}

/// The payload of the installer in `file`; `None` when it carries none (as built, before
/// `--pack`, or the uninstaller).
pub fn read(file: &mut (impl Read + Seek)) -> Result<Option<Payload>, String> {
    let len = file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    if len < FOOTER {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-(FOOTER as i64)))
        .map_err(|e| e.to_string())?;
    let mut footer = [0u8; FOOTER as usize];
    file.read_exact(&mut footer).map_err(|e| e.to_string())?;
    if &footer[12..] != MAGIC {
        return Ok(None);
    }
    let start = u64::from_le_bytes(footer[..8].try_into().expect("8 bytes"));
    let count = u32::from_le_bytes(footer[8..12].try_into().expect("4 bytes"));
    if start > len - FOOTER || count > 16 {
        return Err("the installer is damaged".into());
    }
    file.seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    let mut at = start;
    for _ in 0..count {
        let mut n = [0u8; 2];
        file.read_exact(&mut n).map_err(|e| e.to_string())?;
        let n = u16::from_le_bytes(n) as usize;
        let mut name = vec![0u8; n];
        file.read_exact(&mut name).map_err(|e| e.to_string())?;
        let name = String::from_utf8(name).map_err(|_| "the installer is damaged".to_owned())?;
        let mut size = [0u8; 8];
        file.read_exact(&mut size).map_err(|e| e.to_string())?;
        let size = u64::from_le_bytes(size);
        let mut sha256 = [0u8; 32];
        file.read_exact(&mut sha256).map_err(|e| e.to_string())?;
        let offset = at + 2 + n as u64 + 8 + 32;
        if !plain_name(&name)
            || offset
                .checked_add(size)
                .is_none_or(|end| end > len - FOOTER)
        {
            return Err("the installer is damaged".into());
        }
        entries.push(Entry {
            name,
            size,
            sha256,
            offset,
        });
        at = offset + size;
        file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
    }
    Ok(Some(Payload {
        stub_len: start,
        entries,
    }))
}

/// Writes `entry` from `file` to `dest`: beside it first, checked against its hash, then renamed
/// over. `progress` gets the bytes written so far.
pub fn extract(
    file: &mut (impl Read + Seek),
    entry: &Entry,
    dest: &Path,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    let tmp = dest.with_extension("part");
    let result = (|| {
        file.seek(SeekFrom::Start(entry.offset))
            .map_err(|e| e.to_string())?;
        let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut left = entry.size;
        let mut buf = vec![0u8; 1 << 20];
        while left > 0 {
            let take = left.min(buf.len() as u64) as usize;
            file.read_exact(&mut buf[..take])
                .map_err(|e| e.to_string())?;
            hash.update(&buf[..take]);
            out.write_all(&buf[..take]).map_err(|e| e.to_string())?;
            left -= take as u64;
            progress(entry.size - left);
        }
        out.sync_all().map_err(|e| e.to_string())?;
        drop(out);
        if hash.finalize().as_slice() != entry.sha256 {
            return Err(format!("{} is damaged in the installer", entry.name));
        }
        std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The installer without its payload: the uninstaller.
pub fn stub(file: &mut (impl Read + Seek), payload: Option<&Payload>) -> Result<Vec<u8>, String> {
    let len = match payload {
        Some(p) => p.stub_len,
        None => file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?,
    };
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut out = vec![0u8; len as usize];
    file.read_exact(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn packed() -> Vec<u8> {
        pack(
            b"MZ-stub",
            &[
                ("evolite.exe".into(), b"app bytes".to_vec()),
                ("evolite-renderer.exe".into(), vec![7u8; 3_000_000]),
            ],
        )
        .unwrap()
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("evolite-payload-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn what_is_packed_comes_back_whole() {
        let bytes = packed();
        let mut file = Cursor::new(bytes.clone());
        let payload = read(&mut file).unwrap().unwrap();
        assert_eq!(payload.stub_len, 7);
        let names: Vec<_> = payload.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["evolite.exe", "evolite-renderer.exe"]);
        assert_eq!(payload.installed_size(), 9 + 3_000_000);
        let d = dir("whole");
        let mut seen = 0;
        for e in &payload.entries {
            extract(&mut file, e, &d.join(&e.name), |n| seen = n).unwrap();
        }
        assert_eq!(seen, 3_000_000, "progress reaches the size");
        assert_eq!(std::fs::read(d.join("evolite.exe")).unwrap(), b"app bytes");
        assert_eq!(
            std::fs::read(d.join("evolite-renderer.exe")).unwrap().len(),
            3_000_000
        );
        assert_eq!(stub(&mut file, Some(&payload)).unwrap(), b"MZ-stub");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_unpacked_installer_carries_nothing() {
        assert_eq!(
            read(&mut Cursor::new(b"MZ just the installer".to_vec())).unwrap(),
            None
        );
        assert_eq!(read(&mut Cursor::new(Vec::new())).unwrap(), None);
    }

    #[test]
    fn a_damaged_file_installs_nothing() {
        let mut bytes = packed();
        // One byte of the app changed after packing.
        let at = bytes.windows(9).position(|w| w == b"app bytes").unwrap();
        bytes[at] ^= 1;
        let mut file = Cursor::new(bytes);
        let payload = read(&mut file).unwrap().unwrap();
        let d = dir("damaged");
        let dest = d.join("evolite.exe");
        assert!(extract(&mut file, &payload.entries[0], &dest, |_| {}).is_err());
        assert!(!dest.exists() && !dest.with_extension("part").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_name_cannot_leave_the_install_folder() {
        for bad in [
            "..\\evil.exe",
            "../evil.exe",
            "C:\\x.exe",
            "a/b.exe",
            "",
            ".hidden",
        ] {
            assert!(!plain_name(bad), "{bad}");
            assert!(pack(b"MZ", &[(bad.into(), vec![1])]).is_err(), "{bad}");
        }
        assert!(plain_name("evolite-renderer.exe"));
    }

    #[test]
    fn a_cut_download_is_damaged_not_trusted() {
        let bytes = packed();
        // The footer kept, the middle cut out: offsets point past the end.
        let mut cut = bytes[..20].to_vec();
        cut.extend_from_slice(&bytes[bytes.len() - FOOTER as usize..]);
        assert!(read(&mut Cursor::new(cut)).is_err());
    }
}
