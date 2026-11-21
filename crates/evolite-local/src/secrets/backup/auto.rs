//! FEAT-0045 step 2 (R-12): the automatic copies — written into the folder the person chose,
//! one a week while the wallet is open, the last [`KEEP`] of this wallet and network kept. A file
//! is deleted only when its name is an automatic copy's **and** its open part says this network
//! **and** this wallet's fingerprint: nothing else in the folder is ever touched (AC-A11).

use super::{BackupKey, Contents, EXTENSION, header, seal};
use std::path::{Path, PathBuf};

/// How many automatic copies of a wallet are kept.
pub const KEEP: usize = 5;
/// How often a copy is written: a week (R-12).
pub const EVERY: i64 = 7 * 24 * 60 * 60;
/// What an automatic copy's name ends with (a manual one has no `-auto`).
pub const SUFFIX: &str = "-auto";

/// A wallet's name kept to what a file name may hold on every system.
pub fn file_safe(name: &str) -> String {
    let safe: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let safe = safe.trim_matches(['.', ' ']);
    if safe.is_empty() {
        "wallet".into()
    } else {
        safe.into()
    }
}

/// The name of the automatic copy of `day` (`YYYY-MM-DD`): `evolite-{wallet}-{network}-{day}-
/// auto.evbackup`; `n` from 2 when that name is taken (two computers writing one cloud folder).
pub fn name(wallet: &str, network: &str, day: &str, n: u32) -> String {
    let wallet = file_safe(wallet);
    match n {
        0 | 1 => format!("evolite-{wallet}-{network}-{day}{SUFFIX}.{EXTENSION}"),
        n => format!("evolite-{wallet}-{network}-{day}-{n}{SUFFIX}.{EXTENSION}"),
    }
}

/// A copy written: its name, and whether an old one could not be deleted (it stays; the next
/// copy tries again).
#[derive(Debug)]
pub struct Written {
    pub name: String,
    pub rotate_failed: bool,
}

/// Writes this week's copy of `contents` into `dir` under a name not taken, then lets go of the
/// oldest of this wallet's copies beyond [`KEEP`]. Once the copy is written it is the answer,
/// whatever the old ones do (review r1, step 2 Must: else a copy every hour without end).
pub fn write(
    dir: &Path,
    key: &BackupKey,
    network: &str,
    (wallet, day): (&str, &str),
    contents: &Contents,
) -> std::io::Result<Written> {
    if !dir.is_dir() {
        return Err(std::io::ErrorKind::NotFound.into());
    }
    let bytes = seal(key, network, contents).map_err(std::io::Error::other)?;
    let file = (1..)
        .map(|n| name(wallet, network, day, n))
        .find(|n| !dir.join(n).exists())
        .expect("a free name");
    crate::json_file::write(&dir.join(&file), &bytes)?;
    let (_, failed) = rotate(dir, key, network);
    Ok(Written {
        name: file,
        rotate_failed: failed > 0,
    })
}

/// The order of a copy by its name: its day, then its number (`-2`…; none is 1). `None` for a
/// name that is not an automatic copy's (review r1, step 2 Should: not by the file's time).
pub fn order_of(name: &str) -> Option<(String, u32)> {
    let stem = name.strip_suffix(&format!("{SUFFIX}.{EXTENSION}"))?;
    let parts: Vec<&str> = stem.rsplit('-').collect();
    let date = |p: &[&str]| -> Option<String> {
        let (d, m, y) = (p.first()?, p.get(1)?, p.get(2)?);
        let digits = |s: &str, n| s.len() == n && s.chars().all(|c| c.is_ascii_digit());
        (digits(y, 4) && digits(m, 2) && digits(d, 2)).then(|| format!("{y}-{m}-{d}"))
    };
    match date(&parts[1..]) {
        Some(day) if parts[0].chars().all(|c| c.is_ascii_digit()) && parts[0].len() <= 3 => {
            Some((day, parts[0].parse().ok()?))
        }
        _ => date(&parts).map(|day| (day, 1)),
    }
}

/// The automatic copies of this wallet and network in `dir`, newest first: an automatic copy's
/// name, and an open part with this network and this wallet's fingerprint.
pub fn own_copies(dir: &Path, key: &BackupKey, network: &str) -> Vec<PathBuf> {
    let ending = format!("{SUFFIX}.{EXTENSION}");
    let mut found: Vec<((String, u32), PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("evolite-") && name.ends_with(&ending)) {
                return None;
            }
            let order = order_of(&name)?;
            let path = e.path();
            let bytes = super::read(&path).ok()?;
            let h = header(&bytes)?;
            (h.network == network && h.wallet == key.fingerprint()).then_some((order, path))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    found.into_iter().map(|(_, p)| p).collect()
}

/// Deletes this wallet's automatic copies beyond the newest [`KEEP`]; one that cannot be deleted
/// (read only, held) is left, the others go. How many went, and how many stayed against it.
pub fn rotate(dir: &Path, key: &BackupKey, network: &str) -> (usize, usize) {
    let (mut gone, mut failed) = (0, 0);
    for old in own_copies(dir, key, network).into_iter().skip(KEEP) {
        match std::fs::remove_file(old) {
            Ok(()) => gone += 1,
            Err(_) => failed += 1,
        }
    }
    (gone, failed)
}

/// Whether a copy is due: none yet, or the last is [`EVERY`] old.
pub fn due(last: Option<i64>, now: i64) -> bool {
    last.is_none_or(|last| now.saturating_sub(last) >= EVERY)
}
