//! Р1 (beta list §5а item 7; design `docs/tasks/tech/REL-verify-design-2026-11-20.md`, review
//! `docs/tasks/reviews/REL-verify-design-r1.md`): is there a newer evolite?
//!
//! - One HTTPS GET of the release's `latest.json` and its minisign signature
//!   (`latest.json.minisig`), from a list of places (GitHub only for the beta): 10 s at most,
//!   64 KB at most each, no proxy, a few redirects (GitHub's «latest» links redirect),
//!   `User-Agent: evolite` and nothing about the computer, the wallet or the network.
//! - What it says is believed only when signed by one of the keys built in (main and backup).
//! - It writes no files and starts no programs (the source guard below): it only reads and
//!   answers. Downloading and installing stay the person's, after «How to check the installer».

use std::time::Duration;

use serde::Deserialize;

/// Where `latest.json` and `latest.json.minisig` are read, in turn: the first that answers with a
/// signed file wins (review r1, answer 4: a list, GitHub only for the beta).
pub const PLACES: &[&str] = &["https://github.com/Ulput/evolite/releases/latest/download/"];

/// The whole request may take this long (review r1).
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Neither file may be longer (review r1): a real `latest.json` is under 2 KB.
pub const MAX_BYTES: u64 = 64 * 1024;

/// The evolite release keys built in, from the API (About shows them too):
/// [`evolite_api::RELEASE_KEYS`].
pub use evolite_api::RELEASE_KEYS as KEYS;

/// Whether `bytes` (the file `file`) are signed, with `signature` (a `.minisig`'s text), by one of
/// `keys`. Only prehashed signatures (`ED`, minisign's default) are taken, and the trusted comment
/// — covered by the signature — must name `file`, so a signature of another file does not pass
/// (review r1).
pub fn signed_by(bytes: &[u8], signature: &str, file: &str, keys: &[&str]) -> Result<(), String> {
    use minisign_verify::{PublicKey, Signature};
    let signature = Signature::decode(signature).map_err(|e| format!("signature: {e}"))?;
    // A whole token, not a substring: `file:SHA256SUMS` is not inside `file:SHA256SUMS.minisig`
    // (review r1, WIP point 1).
    let named = signature
        .trusted_comment()
        .split(['\t', ' '])
        .any(|field| field == format!("file:{file}"));
    if !named {
        return Err(format!("the signature is not of {file}"));
    }
    let by_one = keys.iter().any(|key| {
        PublicKey::from_base64(key).is_ok_and(|key| key.verify(bytes, &signature, false).is_ok())
    });
    if by_one {
        Ok(())
    } else {
        Err("not signed by an evolite release key".into())
    }
}

/// What a release says of itself in `latest.json`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Latest {
    /// Semver, prerelease tags allowed: `1.0.1`, `1.1.0-beta.2`.
    pub version: String,
    /// The day it was published, `YYYY-MM-DD`: a file older than one already seen is a replay.
    pub date: String,
    pub notes_url: String,
    pub verify_url: String,
    /// A security fix: also said on the Overview until installed.
    #[serde(default)]
    pub critical: bool,
}

/// `latest.json`'s bytes as a release, or why not (too long, not JSON, no version that parses).
pub fn parse(bytes: &[u8]) -> Result<Latest, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("latest.json is too long".into());
    }
    let latest: Latest = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    semver::Version::parse(latest.version.trim_start_matches('v'))
        .map_err(|e| format!("version: {e}"))?;
    if !is_date(&latest.date) {
        return Err("date: not YYYY-MM-DD".into());
    }
    Ok(latest)
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Whether `latest` is newer than `own` (both semver; a leading `v` is allowed). A prerelease is
/// older than its release (`1.0.0-beta.2` < `1.0.0`), as semver orders them. A version that
/// does not parse is never newer.
pub fn is_newer(latest: &str, own: &str) -> bool {
    let parse = |v: &str| semver::Version::parse(v.trim_start_matches('v')).ok();
    match (parse(latest), parse(own)) {
        (Some(latest), Some(own)) => latest > own,
        _ => false,
    }
}

/// Whether `latest` (a release's date) may be believed after `seen`, the newest date already
/// taken from a signed file: an older one is a replay of an old signed file (review r1).
pub fn not_older(latest: &str, seen: Option<&str>) -> bool {
    // `YYYY-MM-DD` orders as text.
    seen.is_none_or(|seen| latest >= seen)
}

/// How often the check runs while the app is open (the design: once a day).
pub const EVERY: Duration = Duration::from_secs(24 * 3600);

/// Why a check believed nothing, for the journal: a kind, never a URL or a text from the network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No release key is built in yet: nothing can be believed.
    NoKeys,
    Unreachable,
    BadSignature,
    BadFile,
    /// Signed, but older than a file already believed (review r1).
    Replay,
}

impl Refused {
    pub fn kind(self) -> &'static str {
        match self {
            Refused::NoKeys => "no_keys",
            Refused::Unreachable => "unreachable",
            Refused::BadSignature => "bad_signature",
            Refused::BadFile => "bad_file",
            Refused::Replay => "replay",
        }
    }
}

/// One check: the first of [`PLACES`] whose `latest.json` is signed by a key of [`KEYS`], reads,
/// and is not older than `seen` (the newest date already believed). Blocking.
pub fn check(seen: Option<&str>) -> Result<Latest, Refused> {
    check_with(seen, KEYS, read)
}

/// [`check`] with the keys and the reader given: tests never ask the network (review r1).
fn check_with(
    seen: Option<&str>,
    keys: &[&str],
    mut read: impl FnMut(&str, &str) -> Result<Vec<u8>, String>,
) -> Result<Latest, Refused> {
    if keys.is_empty() {
        return Err(Refused::NoKeys);
    }
    let mut why = Refused::Unreachable;
    for place in PLACES {
        let (Ok(json), Ok(sig)) = (
            read(place, "latest.json"),
            read(place, "latest.json.minisig"),
        ) else {
            why = Refused::Unreachable;
            continue;
        };
        let signed = std::str::from_utf8(&sig)
            .map_err(|e| e.to_string())
            .and_then(|sig| signed_by(&json, sig, "latest.json", keys));
        if signed.is_err() {
            why = Refused::BadSignature;
            continue;
        }
        let Ok(latest) = parse(&json) else {
            why = Refused::BadFile;
            continue;
        };
        if !not_older(&latest.date, seen) {
            why = Refused::Replay;
            continue;
        }
        return Ok(latest);
    }
    Err(why)
}

/// One file of `place`, read as the review asks: HTTPS only, no proxy, at most 3 redirects, the
/// whole request within [`TIMEOUT`], at most [`MAX_BYTES`]. Blocking (ureq): the core runs it
/// off its own tasks.
pub fn read(place: &str, file: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let config = ureq::Agent::config_builder()
        .https_only(true)
        .proxy(None)
        .max_redirects(3)
        .timeout_global(Some(TIMEOUT))
        .user_agent("evolite")
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent
        .get(&format!("{place}{file}"))
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("{file} is too long"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &[u8] = br#"{"version":"1.0.1","date":"2026-12-03",
        "notes_url":"https://github.com/Ulput/evolite/releases/tag/v1.0.1",
        "verify_url":"https://github.com/Ulput/evolite/blob/main/docs/verify.md",
        "critical":false}"#;

    #[test]
    fn a_release_file_is_read_and_checked() {
        let latest = parse(GOOD).unwrap();
        assert_eq!(latest.version, "1.0.1");
        assert!(!latest.critical);
        assert!(parse(b"not json").is_err());
        assert!(
            parse(br#"{"version":"one","date":"2026-12-03","notes_url":"","verify_url":""}"#)
                .is_err()
        );
        assert!(
            parse(br#"{"version":"1.0.1","date":"3 Dec","notes_url":"","verify_url":""}"#).is_err()
        );
        let long = vec![b' '; (MAX_BYTES + 1) as usize];
        assert!(parse(&long).is_err(), "over 64 KB");
    }

    /// Review r1: semver with prerelease tags.
    #[test]
    fn newer_is_semver_with_prereleases() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("v1.1.0", "1.0.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.9", "1.0.0"));
        assert!(
            is_newer("1.0.0", "1.0.0-beta.2"),
            "a release after its beta"
        );
        assert!(is_newer("1.0.0-beta.3", "1.0.0-beta.2"));
        assert!(!is_newer("1.0.0-beta.2", "1.0.0"));
        assert!(!is_newer("garbage", "1.0.0"));
    }

    /// Review r1: a signed but older file (a replay) is not believed after a newer one.
    #[test]
    fn an_older_file_is_a_replay() {
        assert!(not_older("2026-12-03", None));
        assert!(not_older("2026-12-03", Some("2026-12-03")));
        assert!(not_older("2026-12-04", Some("2026-12-03")));
        assert!(!not_older("2026-11-20", Some("2026-12-03")));
    }

    /// Base64, standard alphabet with padding, for the test's own minisign files.
    fn b64(bytes: &[u8]) -> String {
        const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// A minisign key (`seed` makes it and its id) and its signature of `bytes` as `file`:
    /// prehashed (BLAKE2b-512) unless `legacy`, with the trusted comment minisign writes.
    fn sign(seed: u8, file: &str, bytes: &[u8], legacy: bool) -> (String, String) {
        use ed25519_dalek::{Signer, SigningKey};
        let key = SigningKey::from_bytes(&[seed; 32]);
        let id = [seed; 8];
        let public = [b"Ed".as_slice(), &id, &key.verifying_key().to_bytes()].concat();
        let hash = blake2b_simd::Params::new().hash_length(64).hash(bytes);
        let (alg, signed): (&[u8], &[u8]) = if legacy {
            (b"Ed", bytes)
        } else {
            (b"ED", hash.as_bytes())
        };
        let sig = key.sign(signed).to_bytes();
        let comment = format!("timestamp:1764720000\tfile:{file}\tprehashed");
        let global = key
            .sign(&[sig.as_slice(), comment.as_bytes()].concat())
            .to_bytes();
        let text = format!(
            "untrusted comment: signature from evolite\n{}\ntrusted comment: {comment}\n{}\n",
            b64(&[alg, &id, &sig].concat()),
            b64(&global)
        );
        (b64(&public), text)
    }

    /// Review r1: a file is believed only when signed by the main or the backup key, prehashed,
    /// with its own name in the trusted comment, and unchanged.
    #[test]
    fn only_a_release_key_signs_latest_json() {
        let (main, by_main) = sign(1, "latest.json", GOOD, false);
        let (backup, by_backup) = sign(2, "latest.json", GOOD, false);
        let keys = [main.as_str(), backup.as_str()];
        assert_eq!(signed_by(GOOD, &by_main, "latest.json", &keys), Ok(()));
        assert_eq!(
            signed_by(GOOD, &by_backup, "latest.json", &keys),
            Ok(()),
            "the backup key"
        );
        let (_, by_other) = sign(3, "latest.json", GOOD, false);
        assert!(
            signed_by(GOOD, &by_other, "latest.json", &keys).is_err(),
            "another key"
        );
        let (_, of_sums) = sign(1, "SHA256SUMS", GOOD, false);
        assert!(
            signed_by(GOOD, &of_sums, "latest.json", &keys).is_err(),
            "another file's"
        );
        let changed = [GOOD, b" "].concat();
        assert!(
            signed_by(&changed, &by_main, "latest.json", &keys).is_err(),
            "changed"
        );
        let (_, legacy) = sign(1, "latest.json", GOOD, true);
        assert!(
            signed_by(GOOD, &legacy, "latest.json", &keys).is_err(),
            "not prehashed"
        );
        assert!(
            signed_by(GOOD, &by_main, "latest.json", &[]).is_err(),
            "no key built in"
        );
        assert!(signed_by(GOOD, "not a signature", "latest.json", &keys).is_err());
        // A whole token: a signature of SHA256SUMS.minisig is not one of SHA256SUMS.
        let (_, of_minisig) = sign(1, "SHA256SUMS.minisig", GOOD, false);
        assert!(signed_by(GOOD, &of_minisig, "SHA256SUMS", &keys).is_err());
        let (_, of_sums_too) = sign(1, "SHA256SUMS", GOOD, false);
        assert_eq!(signed_by(GOOD, &of_sums_too, "SHA256SUMS", &keys), Ok(()));
    }

    /// The release keys written in `text` (README.md, docs/verify.md): the `main:` and `backup:`
    /// lines; a `<…>` placeholder is no key.
    fn keys_in(text: &str) -> Vec<String> {
        ["main:", "backup:"]
            .iter()
            .filter_map(|label| {
                text.lines()
                    .map(str::trim)
                    .find_map(|line| line.strip_prefix(label))
                    .map(str::trim)
                    .filter(|key| !key.starts_with('<') && !key.is_empty())
                    .map(str::to_owned)
            })
            .collect()
    }

    /// One check over a fake place (tests never ask the network): a signed file is believed; no
    /// keys, a bad signature, a replay and no answer are refused, each said by its kind.
    #[test]
    fn a_check_believes_only_a_signed_new_file() {
        let (main, sig) = sign(1, "latest.json", GOOD, false);
        let keys = [main.as_str()];
        let place = |json: &'static [u8], sig: String| {
            move |_: &str, file: &str| -> Result<Vec<u8>, String> {
                match file {
                    "latest.json" => Ok(json.to_vec()),
                    _ => Ok(sig.clone().into_bytes()),
                }
            }
        };
        let latest = check_with(None, &keys, place(GOOD, sig.clone())).unwrap();
        assert_eq!(latest.version, "1.0.1");
        assert_eq!(
            check_with(None, &[], place(GOOD, sig.clone())),
            Err(Refused::NoKeys)
        );
        let (_, other) = sign(2, "latest.json", GOOD, false);
        assert_eq!(
            check_with(None, &keys, place(GOOD, other)),
            Err(Refused::BadSignature)
        );
        assert_eq!(
            check_with(Some("2026-12-31"), &keys, place(GOOD, sig.clone())),
            Err(Refused::Replay)
        );
        let down = |_: &str, _: &str| -> Result<Vec<u8>, String> { Err("timed out".into()) };
        assert_eq!(check_with(None, &keys, down), Err(Refused::Unreachable));
        assert_eq!(Refused::BadSignature.kind(), "bad_signature");
    }

    /// Review r1, WIP point 2: the keys built in are the keys README.md and docs/verify.md
    /// (and its Russian, review r1 Should 1) publish, in that order — red the day either side
    /// gets keys the other has not.
    #[test]
    fn the_keys_built_in_are_the_published_ones() {
        let readme = include_str!("../../../README.md");
        let verify = include_str!("../../../docs/verify.md");
        let verify_ru = include_str!("../../../docs/verify.ru.md");
        let built_in: Vec<String> = KEYS.iter().map(|k| (*k).to_owned()).collect();
        assert_eq!(keys_in(verify), built_in, "docs/verify.md");
        assert_eq!(keys_in(verify_ru), built_in, "docs/verify.ru.md");
        assert_eq!(keys_in(readme), built_in, "README.md");
    }

    /// Review r1: this module writes no files and starts no programs.
    #[test]
    fn it_writes_no_file_and_starts_nothing() {
        let source = include_str!("update.rs");
        let code = &source[..source.find("#[cfg(test)]").unwrap()];
        for forbidden in [
            ["std::fs::", "write"].concat(),
            ["File::", "create"].concat(),
            ["OpenOptions", ""].concat(),
            ["std::process", ""].concat(),
            ["Command::", "new"].concat(),
        ] {
            assert!(!code.contains(&forbidden), "{forbidden}");
        }
    }
}
