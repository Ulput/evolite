//! The wallet vault: the recovery phrase encrypted on disk with the wallet password.
//!
//! Format `evolite/vault/v1` (JSON): Argon2id stretches the password into a 256-bit key,
//! AES-256-GCM encrypts the phrase with a fresh random 96-bit nonce, and the associated data
//! binds the ciphertext to the network, wallet ID and birth height, so a file edited by hand
//! fails to open instead of syncing the wrong wallet. Changing any of this needs a new version.
//!
//! Format `evolite/vault/v2` (FEAT-0041 R-15): the same, but what is encrypted is the phrase
//! **and its BIP-39 passphrase** (empty: none), with their lengths, padded with zeros to
//! [`CONTENT`] bytes — every v2 file is as long as any other, so neither the format nor the
//! length tells that a wallet has a passphrase. Every wallet this build writes is v2; a v1 file
//! is still read (its passphrase is empty). The format is in the associated data.
//!
//! Secrets live in `Zeroizing` buffers and never reach errors or logs. Moved from spv-wallet
//! (src-tauri/src/vault.rs) unchanged, so both apps read the same files.

use std::fs;
use std::io::Write;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// The format of the files written before FEAT-0041: read, never written.
const FORMAT_V1: &str = "evolite/vault/v1";
const FORMAT: &str = "evolite/vault/v2";

/// The most bytes a phrase takes: 24 words of the longest word lists (Japanese, 3 bytes a
/// character) with their separators.
pub const PHRASE_BYTES: usize = 1024;
/// The most bytes a BIP-39 passphrase takes after NFKD (FEAT-0041 R-10).
pub const PASSPHRASE_BYTES: usize = 256;
/// What v2 encrypts, always this long: a version byte, the two lengths (2 bytes each, little
/// endian), the phrase, the passphrase, zeros. 1 024 + 256 + 5 = 1 285 fit, the rest is room.
const CONTENT: usize = 1536;
const CONTENT_VERSION: u8 = 1;

/// What the vault keeps secret: the phrase and its BIP-39 passphrase (empty: none, and every
/// v1 file). Wiped on drop; `Debug` shows nothing, not whether there is a passphrase.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret {
    pub phrase: Zeroizing<String>,
    /// NFKD, as BIP-39 salts it.
    pub passphrase: Zeroizing<String>,
}

impl Secret {
    pub fn new(phrase: &str, passphrase: &str) -> Self {
        Self {
            phrase: Zeroizing::new(phrase.to_owned()),
            passphrase: Zeroizing::new(passphrase.to_owned()),
        }
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(hidden)")
    }
}

/// What the vault says about the wallet without the password. Nothing here is secret.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultMeta {
    pub network: String,
    /// `WalletId` of key-wallet, hex: derived from the root public key, not secret.
    pub wallet_id: String,
    /// Block height the wallet was created at; the scan for its history starts there.
    pub birth_height: u32,
    pub word_count: u8,
    pub created_at: u64,
}

/// Argon2id cost. The defaults follow OWASP's first recommendation (m=46 MiB, t=1 is the
/// minimum; we take 64 MiB and 3 passes, about half a second on a laptop).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParams {
    /// Costs a file may ask for. The parameters are not encrypted: a changed file must not make
    /// unlocking allocate gigabytes or run for hours.
    fn within_limits(&self) -> bool {
        self.memory_kib <= 1024 * 1024 && self.iterations <= 64 && self.parallelism <= 16
    }
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            memory_kib: 64 * 1024,
            iterations: 3,
            parallelism: 1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultFile {
    pub format: String,
    pub meta: VaultMeta,
    pub kdf: KdfParams,
    pub salt: String,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum VaultError {
    /// Wrong password, or the file was changed: AES-GCM cannot tell the two apart.
    WrongPassword,
    Unsupported,
    Corrupt,
    /// The phrase or the passphrase is longer than v2 holds: refused before anything is
    /// written, never cut (FEAT-0041 R-10).
    TooLong,
    Io(String),
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongPassword => write!(f, "wrong password"),
            Self::Unsupported => write!(f, "unsupported vault format"),
            Self::Corrupt => write!(f, "the vault file is damaged"),
            Self::TooLong => write!(f, "the phrase or the passphrase is too long"),
            Self::Io(e) => write!(f, "vault file: {e}"),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Result<Vec<u8>, VaultError> {
    // Byte slicing below needs ASCII: a changed file must not panic the core.
    if !text.is_ascii() || !text.len().is_multiple_of(2) {
        return Err(VaultError::Corrupt);
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| VaultError::Corrupt))
        .collect()
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the OS random generator is available");
    bytes
}

fn derive_key(
    password: &str,
    salt: &[u8],
    params: KdfParams,
) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    let params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(32),
    )
    .map_err(|_| VaultError::Corrupt)?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|_| VaultError::Corrupt)?;
    Ok(key)
}

/// Associated data: ties the ciphertext to its format and what the file says about the wallet.
fn associated_data(format: &str, meta: &VaultMeta) -> Vec<u8> {
    format!(
        "{format}|{}|{}|{}|{}",
        meta.network, meta.wallet_id, meta.birth_height, meta.word_count
    )
    .into_bytes()
}

/// v2's content: always [`CONTENT`] bytes.
fn content(secret: &Secret) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let phrase = secret.phrase.as_bytes();
    let passphrase = secret.passphrase.as_bytes();
    if phrase.len() > PHRASE_BYTES || passphrase.len() > PASSPHRASE_BYTES {
        return Err(VaultError::TooLong);
    }
    let mut out = Zeroizing::new(Vec::with_capacity(CONTENT));
    out.push(CONTENT_VERSION);
    out.extend_from_slice(&(phrase.len() as u16).to_le_bytes());
    out.extend_from_slice(&(passphrase.len() as u16).to_le_bytes());
    out.extend_from_slice(phrase);
    out.extend_from_slice(passphrase);
    out.resize(CONTENT, 0);
    Ok(out)
}

/// The phrase and passphrase back from v2's content; anything else in it is `Corrupt`.
fn parse(plain: &[u8]) -> Result<Secret, VaultError> {
    if plain.len() != CONTENT || plain[0] != CONTENT_VERSION {
        return Err(VaultError::Corrupt);
    }
    let phrase_len = u16::from_le_bytes([plain[1], plain[2]]) as usize;
    let passphrase_len = u16::from_le_bytes([plain[3], plain[4]]) as usize;
    if phrase_len > PHRASE_BYTES || passphrase_len > PASSPHRASE_BYTES {
        return Err(VaultError::Corrupt);
    }
    let phrase = &plain[5..5 + phrase_len];
    let passphrase = &plain[5 + phrase_len..5 + phrase_len + passphrase_len];
    if plain[5 + phrase_len + passphrase_len..]
        .iter()
        .any(|b| *b != 0)
    {
        return Err(VaultError::Corrupt);
    }
    let text = |bytes: &[u8]| {
        std::str::from_utf8(bytes)
            .map(|t| Zeroizing::new(t.to_owned()))
            .map_err(|_| VaultError::Corrupt)
    };
    Ok(Secret {
        phrase: text(phrase)?,
        passphrase: text(passphrase)?,
    })
}

/// Encrypts the phrase and its BIP-39 passphrase with the password, as v2.
pub fn seal(
    secret: &Secret,
    password: &str,
    meta: VaultMeta,
    kdf: KdfParams,
) -> Result<VaultFile, VaultError> {
    let plain = content(secret)?;
    let salt: [u8; 16] = random();
    let nonce: [u8; 12] = random();
    let key = derive_key(password, &salt, kdf)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| VaultError::Corrupt)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &associated_data(FORMAT, &meta),
            },
        )
        .map_err(|_| VaultError::Corrupt)?;
    Ok(VaultFile {
        format: FORMAT.to_owned(),
        meta,
        kdf,
        salt: hex(&salt),
        nonce: hex(&nonce),
        ciphertext: hex(&ciphertext),
    })
}

/// Decrypts the phrase and its passphrase (v1: none). A wrong password and a tampered file both
/// give `WrongPassword`.
pub fn open(file: &VaultFile, password: &str) -> Result<Secret, VaultError> {
    if file.format != FORMAT && file.format != FORMAT_V1 {
        return Err(VaultError::Unsupported);
    }
    let salt = unhex(&file.salt)?;
    let nonce = unhex(&file.nonce)?;
    let ciphertext = unhex(&file.ciphertext)?;
    if nonce.len() != 12 || salt.len() < 16 || !file.kdf.within_limits() {
        return Err(VaultError::Corrupt);
    }
    let key = derive_key(password, &salt, file.kdf)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| VaultError::Corrupt)?;
    let plain = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &associated_data(&file.format, &file.meta),
                },
            )
            .map_err(|_| VaultError::WrongPassword)?,
    );
    if file.format == FORMAT {
        return parse(&plain);
    }
    let phrase = std::str::from_utf8(&plain).map_err(|_| VaultError::Corrupt)?;
    Ok(Secret::new(phrase, ""))
}

pub fn load(path: &Path) -> Result<Option<VaultFile>, VaultError> {
    match crate::json_file::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| VaultError::Corrupt),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(VaultError::Io(e.kind().to_string())),
    }
}

/// Writes the vault next to its final place and renames it, so a crash never leaves half a file.
/// On Unix only the owner may read it (0600, its folder 0700): other users of the computer
/// get no copy to guess the password on. On Windows the per-user folder's ACL does that.
pub fn save(path: &Path, file: &VaultFile) -> Result<(), VaultError> {
    let io = |e: std::io::Error| VaultError::Io(e.kind().to_string());
    if let Some(dir) = path.parent() {
        crate::folders::create(dir).map_err(io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io)?;
        }
    }
    let tmp = path.with_extension("tmp");
    let json = serde_json::to_vec_pretty(file).map_err(|_| VaultError::Corrupt)?;
    let _ = fs::remove_file(&tmp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut out = options.open(&tmp).map_err(io)?;
    out.write_all(&json).map_err(io)?;
    out.sync_all().map_err(io)?;
    drop(out);
    fs::rename(&tmp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap parameters: the tests check the format, not the cost.
    const FAST: KdfParams = KdfParams {
        memory_kib: 256,
        iterations: 1,
        parallelism: 1,
    };
    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn secret() -> Secret {
        Secret::new(PHRASE, "")
    }

    fn meta() -> VaultMeta {
        VaultMeta {
            network: "testnet".into(),
            wallet_id: "ab".repeat(32),
            birth_height: 1_581_000,
            word_count: 12,
            created_at: 1_793_000_000,
        }
    }

    #[test]
    fn round_trip() {
        let file = seal(&secret(), "correct horse", meta(), FAST).unwrap();
        assert_eq!(
            open(&file, "correct horse").unwrap().phrase.as_str(),
            PHRASE
        );
        assert!(!file.ciphertext.contains("abandon"));
    }

    #[test]
    fn wrong_password_fails() {
        let file = seal(&secret(), "correct horse", meta(), FAST).unwrap();
        assert_eq!(
            open(&file, "battery staple"),
            Err(VaultError::WrongPassword)
        );
    }

    #[test]
    fn edited_metadata_fails() {
        let mut file = seal(&secret(), "pw", meta(), FAST).unwrap();
        file.meta.birth_height = 0;
        assert_eq!(open(&file, "pw"), Err(VaultError::WrongPassword));
        let mut file = seal(&secret(), "pw", meta(), FAST).unwrap();
        file.meta.network = "mainnet".into();
        assert_eq!(open(&file, "pw"), Err(VaultError::WrongPassword));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let mut file = seal(&secret(), "pw", meta(), FAST).unwrap();
        let flipped = if file.ciphertext.starts_with('0') {
            "1"
        } else {
            "0"
        };
        file.ciphertext.replace_range(0..1, flipped);
        assert_eq!(open(&file, "pw"), Err(VaultError::WrongPassword));
    }

    #[test]
    fn fresh_salt_and_nonce_every_time() {
        let a = seal(&secret(), "pw", meta(), FAST).unwrap();
        let b = seal(&secret(), "pw", meta(), FAST).unwrap();
        assert_ne!(a.salt, b.salt);
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn a_changed_file_is_refused_not_obeyed() {
        let mut file = seal(&secret(), "pw", meta(), FAST).unwrap();
        file.kdf.memory_kib = u32::MAX;
        assert_eq!(open(&file, "pw"), Err(VaultError::Corrupt));
        let mut file = seal(&secret(), "pw", meta(), FAST).unwrap();
        file.salt = "ω".repeat(16);
        assert_eq!(open(&file, "pw"), Err(VaultError::Corrupt));
        assert!(KdfParams::default().within_limits());
    }

    #[test]
    fn unknown_format_is_refused() {
        let mut file = seal(&secret(), "pw", meta(), FAST).unwrap();
        file.format = "evolite/vault/v3".into();
        assert_eq!(open(&file, "pw"), Err(VaultError::Unsupported));
    }

    #[test]
    fn saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("evolite-vault-test-{}", std::process::id()));
        let path = dir.join("wallet.json");
        assert_eq!(load(&path).unwrap(), None);
        let file = seal(&secret(), "pw", meta(), FAST).unwrap();
        save(&path, &file).unwrap();
        assert_eq!(load(&path).unwrap(), Some(file.clone()));
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("abandon") && !text.contains("\"pw\""));
        // A stale half-written file from a crash does not stop the next save.
        fs::write(path.with_extension("tmp"), b"half").unwrap();
        save(&path, &file).unwrap();
        assert_eq!(load(&path).unwrap(), Some(file));
        fs::remove_dir_all(dir).unwrap();
    }

    /// A file as the builds before FEAT-0041 wrote it: the phrase alone, format v1.
    fn seal_v1(phrase: &str, password: &str) -> VaultFile {
        let salt: [u8; 16] = random();
        let nonce: [u8; 12] = random();
        let key = derive_key(password, &salt, FAST).unwrap();
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).unwrap();
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: phrase.as_bytes(),
                    aad: &associated_data(FORMAT_V1, &meta()),
                },
            )
            .unwrap();
        VaultFile {
            format: FORMAT_V1.to_owned(),
            meta: meta(),
            kdf: FAST,
            salt: hex(&salt),
            nonce: hex(&nonce),
            ciphertext: hex(&ciphertext),
        }
    }

    /// AC-A4, AC-T-P1: a v1 file opens as it is, with no passphrase; resealed, it is v2.
    #[test]
    fn a_v1_file_opens_and_is_resealed_as_v2() {
        let old = seal_v1(PHRASE, "pw");
        let opened = open(&old, "pw").unwrap();
        assert_eq!(opened, Secret::new(PHRASE, ""));
        assert_eq!(open(&old, "other"), Err(VaultError::WrongPassword));
        let new = seal(&opened, "pw", old.meta.clone(), FAST).unwrap();
        assert_eq!(new.format, "evolite/vault/v2");
        assert_eq!(open(&new, "pw").unwrap(), opened);
        // The format is in the associated data: a v2 file relabelled v1 does not open.
        let mut relabelled = new.clone();
        relabelled.format = FORMAT_V1.into();
        assert_eq!(open(&relabelled, "pw"), Err(VaultError::WrongPassword));
    }

    /// AC-T-P1: a build before FEAT-0041 knew only v1 and answered any other format
    /// «unsupported» (`format != "evolite/vault/v1"`), leaving the file as it was.
    #[test]
    fn an_older_build_refuses_v2_and_leaves_the_file() {
        let dir = std::env::temp_dir().join(format!("evolite-vault-v2-{}", std::process::id()));
        let path = dir.join("wallet.json");
        save(
            &path,
            &seal(&Secret::new(PHRASE, "TREZOR"), "pw", meta(), FAST).unwrap(),
        )
        .unwrap();
        let before = fs::read(&path).unwrap();
        let file = load(&path).unwrap().unwrap();
        let old_reader = |f: &VaultFile| f.format != "evolite/vault/v1";
        assert!(
            old_reader(&file),
            "an old build says «unsupported vault format»"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }

    /// FEAT-0041: the passphrase is sealed with the phrase and comes back as it was.
    #[test]
    fn the_passphrase_comes_back() {
        for passphrase in ["", "TREZOR", " abc", "пароль", "㍿😀"] {
            let secret = Secret::new(PHRASE, passphrase);
            let file = seal(&secret, "pw", meta(), FAST).unwrap();
            assert_eq!(open(&file, "pw").unwrap(), secret);
        }
    }

    /// R-15, AC-A13, AC-A15: files with no passphrase, a short one and the longest one, the
    /// longest phrase among them, are one length and one look outside: nothing tells a
    /// passphrase.
    #[test]
    fn every_v2_file_is_one_length() {
        let longest_phrase = "あ".repeat(PHRASE_BYTES / 3);
        assert_eq!(longest_phrase.len(), 1023);
        let files: Vec<VaultFile> = [
            Secret::new(PHRASE, ""),
            Secret::new(PHRASE, "TREZOR"),
            Secret::new(PHRASE, &"a".repeat(PASSPHRASE_BYTES)),
            Secret::new(&longest_phrase, &"a".repeat(PASSPHRASE_BYTES)),
            Secret::new(&longest_phrase, ""),
        ]
        .iter()
        .map(|s| seal(s, "pw", meta(), FAST).unwrap())
        .collect();
        for file in &files {
            assert_eq!(file.ciphertext.len(), (CONTENT + 16) * 2);
            assert_eq!(file.format, files[0].format);
            assert_eq!(file.meta, files[0].meta);
        }
        let text = serde_json::to_string(&files[1]).unwrap();
        assert!(!text.to_lowercase().contains("passphrase"));
    }

    /// R-10: a passphrase over 256 bytes is refused before anything is written, not cut; so is
    /// a phrase over its room.
    #[test]
    fn too_long_is_refused_not_cut() {
        let over = Secret::new(PHRASE, &"a".repeat(PASSPHRASE_BYTES + 1));
        assert_eq!(seal(&over, "pw", meta(), FAST), Err(VaultError::TooLong));
        let over = Secret::new(&"a".repeat(PHRASE_BYTES + 1), "");
        assert_eq!(seal(&over, "pw", meta(), FAST), Err(VaultError::TooLong));
        // Counted in bytes: 64 four-byte characters fit, 65 do not.
        assert!(seal(&Secret::new(PHRASE, &"😀".repeat(64)), "pw", meta(), FAST).is_ok());
        let over = Secret::new(PHRASE, &"😀".repeat(65));
        assert_eq!(seal(&over, "pw", meta(), FAST), Err(VaultError::TooLong));
    }

    /// Content that is not what v2 writes (lengths past the room, bytes after the padding's
    /// start) is damaged, not read.
    #[test]
    fn odd_content_is_corrupt() {
        let mut plain = content(&secret()).unwrap();
        assert_eq!(parse(&plain).unwrap(), secret());
        plain[CONTENT - 1] = 1;
        assert_eq!(parse(&plain), Err(VaultError::Corrupt));
        let mut plain = content(&secret()).unwrap();
        plain[3] = 0xff;
        plain[4] = 0xff;
        assert_eq!(parse(&plain), Err(VaultError::Corrupt));
        assert_eq!(parse(&plain[..CONTENT - 1]), Err(VaultError::Corrupt));
    }

    #[test]
    fn debug_shows_nothing() {
        assert_eq!(
            format!("{:?}", Secret::new(PHRASE, "TREZOR")),
            "Secret(hidden)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_the_owner_reads_the_vault() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("evolite-vault-mode-{}", std::process::id()));
        let path = dir.join("wallet.json");
        save(&path, &seal(&secret(), "pw", meta(), FAST).unwrap()).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&dir), 0o700);
        fs::remove_dir_all(dir).unwrap();
    }
}
