//! The wallet vault: the recovery phrase encrypted on disk with the wallet password.
//!
//! Format `evolite/vault/v1` (JSON): Argon2id stretches the password into a 256-bit key,
//! AES-256-GCM encrypts the phrase with a fresh random 96-bit nonce, and the associated data
//! binds the ciphertext to the network, wallet ID and birth height, so a file edited by hand
//! fails to open instead of syncing the wrong wallet. Changing any of this needs a new version.
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

const FORMAT: &str = "evolite/vault/v1";

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
    Io(String),
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongPassword => write!(f, "wrong password"),
            Self::Unsupported => write!(f, "unsupported vault format"),
            Self::Corrupt => write!(f, "the vault file is damaged"),
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

/// Associated data: ties the ciphertext to what the file says about the wallet.
fn associated_data(meta: &VaultMeta) -> Vec<u8> {
    format!(
        "{FORMAT}|{}|{}|{}|{}",
        meta.network, meta.wallet_id, meta.birth_height, meta.word_count
    )
    .into_bytes()
}

/// Encrypts the phrase with the password.
pub fn seal(
    phrase: &str,
    password: &str,
    meta: VaultMeta,
    kdf: KdfParams,
) -> Result<VaultFile, VaultError> {
    let salt: [u8; 16] = random();
    let nonce: [u8; 12] = random();
    let key = derive_key(password, &salt, kdf)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| VaultError::Corrupt)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: phrase.as_bytes(),
                aad: &associated_data(&meta),
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

/// Decrypts the phrase. A wrong password and a tampered file both give `WrongPassword`.
pub fn open(file: &VaultFile, password: &str) -> Result<Zeroizing<String>, VaultError> {
    if file.format != FORMAT {
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
                    aad: &associated_data(&file.meta),
                },
            )
            .map_err(|_| VaultError::WrongPassword)?,
    );
    let phrase = std::str::from_utf8(&plain).map_err(|_| VaultError::Corrupt)?;
    Ok(Zeroizing::new(phrase.to_owned()))
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
        let file = seal(PHRASE, "correct horse", meta(), FAST).unwrap();
        assert_eq!(open(&file, "correct horse").unwrap().as_str(), PHRASE);
        assert!(!file.ciphertext.contains("abandon"));
    }

    #[test]
    fn wrong_password_fails() {
        let file = seal(PHRASE, "correct horse", meta(), FAST).unwrap();
        assert_eq!(
            open(&file, "battery staple"),
            Err(VaultError::WrongPassword)
        );
    }

    #[test]
    fn edited_metadata_fails() {
        let mut file = seal(PHRASE, "pw", meta(), FAST).unwrap();
        file.meta.birth_height = 0;
        assert_eq!(open(&file, "pw"), Err(VaultError::WrongPassword));
        let mut file = seal(PHRASE, "pw", meta(), FAST).unwrap();
        file.meta.network = "mainnet".into();
        assert_eq!(open(&file, "pw"), Err(VaultError::WrongPassword));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let mut file = seal(PHRASE, "pw", meta(), FAST).unwrap();
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
        let a = seal(PHRASE, "pw", meta(), FAST).unwrap();
        let b = seal(PHRASE, "pw", meta(), FAST).unwrap();
        assert_ne!(a.salt, b.salt);
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn a_changed_file_is_refused_not_obeyed() {
        let mut file = seal(PHRASE, "pw", meta(), FAST).unwrap();
        file.kdf.memory_kib = u32::MAX;
        assert_eq!(open(&file, "pw"), Err(VaultError::Corrupt));
        let mut file = seal(PHRASE, "pw", meta(), FAST).unwrap();
        file.salt = "ω".repeat(16);
        assert_eq!(open(&file, "pw"), Err(VaultError::Corrupt));
        assert!(KdfParams::default().within_limits());
    }

    #[test]
    fn unknown_format_is_refused() {
        let mut file = seal(PHRASE, "pw", meta(), FAST).unwrap();
        file.format = "evolite/vault/v2".into();
        assert_eq!(open(&file, "pw"), Err(VaultError::Unsupported));
    }

    #[test]
    fn saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("evolite-vault-test-{}", std::process::id()));
        let path = dir.join("wallet.json");
        assert_eq!(load(&path).unwrap(), None);
        let file = seal(PHRASE, "pw", meta(), FAST).unwrap();
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

    #[cfg(unix)]
    #[test]
    fn only_the_owner_reads_the_vault() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("evolite-vault-mode-{}", std::process::id()));
        let path = dir.join("wallet.json");
        save(&path, &seal(PHRASE, "pw", meta(), FAST).unwrap()).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&dir), 0o700);
        fs::remove_dir_all(dir).unwrap();
    }
}
