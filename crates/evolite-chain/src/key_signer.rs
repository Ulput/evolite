//! SEC-5 (П4): the signer of an operation, with the private keys it was given wiped when it goes,
//! and never printed. dpp's `SimpleSigner` keeps its keys as plain `[u8; 32]` until the memory
//! is reused, and its `Debug` writes them (base64): one `{:?}` would put a key in the journal.
//! Every signer in this crate is a [`KeySigner`] (a test of the sources checks it). The
//! `SimpleSigner` inside is never handed out (review r1, Should 1): no copy, no `take`, no
//! `{:?}` of it — only the two ways of adding keys and the `Signer` trait.

use simple_signer::signer::SimpleSigner;
use zeroize::Zeroize;

#[derive(Default)]
pub struct KeySigner(SimpleSigner);

impl KeySigner {
    /// Identity keys and their private keys, as `SimpleSigner` takes them.
    pub fn add_identity_public_keys(
        &mut self,
        keys: impl IntoIterator<Item = (dpp::identity::IdentityPublicKey, [u8; 32])>,
    ) {
        self.0.add_identity_public_keys(keys);
    }

    /// A Platform address's private key.
    pub fn add_address_key(&mut self, address: [u8; 20], private_key: [u8; 32]) {
        self.0.add_address_key(address, private_key);
    }
}

impl Drop for KeySigner {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// Zeroes every private key `s` holds.
fn wipe(s: &mut SimpleSigner) {
    {
        for key in s
            .private_keys
            .values_mut()
            .chain(s.private_keys_in_creation.values_mut())
            .chain(s.address_private_keys.values_mut())
            .chain(s.address_private_keys_in_creation.values_mut())
        {
            key.zeroize();
        }
    }
}

/// Signs as the `SimpleSigner` inside does, for every kind of key it signs with: an API that
/// takes a `Signer` takes a `KeySigner` as it is.
#[async_trait::async_trait]
impl<K> dpp::identity::signer::Signer<K> for KeySigner
where
    K: Send + Sync,
    SimpleSigner: dpp::identity::signer::Signer<K>,
{
    async fn sign(
        &self,
        key: &K,
        data: &[u8],
    ) -> Result<dpp::platform_value::BinaryData, dpp::ProtocolError> {
        self.0.sign(key, data).await
    }

    async fn sign_create_witness(
        &self,
        key: &K,
        data: &[u8],
    ) -> Result<dpp::address_funds::AddressWitness, dpp::ProtocolError> {
        self.0.sign_create_witness(key, data).await
    }

    fn can_sign_with(&self, key: &K) -> bool {
        self.0.can_sign_with(key)
    }
}

impl std::fmt::Debug for KeySigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeySigner(hidden)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SEC-5 (П4): no key in `Debug`, and the keys are zeroed when it goes.
    #[test]
    fn a_signer_shows_no_key_and_wipes_them() {
        let mut signer = KeySigner::default();
        signer.add_address_key([1; 20], [0xAB; 32]);
        assert_eq!(format!("{signer:?}"), "KeySigner(hidden)");
        wipe(&mut signer.0);
        assert_eq!(signer.0.address_private_keys[&[1; 20]], [0; 32]);
    }

    /// SEC-5 (П4): every signer of the crate is a `KeySigner`, nothing names `SimpleSigner`
    /// outside it, and nothing prints one with `dbg!`.
    #[test]
    fn no_bare_simple_signer_in_the_sources() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut dirs = vec![root];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") || path.ends_with("key_signer.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for (n, line) in text.lines().enumerate() {
                    // No `SimpleSigner` at all outside its wrapper (review r1, Should 1).
                    if line.contains("SimpleSigner") || line.contains("dbg!(") {
                        found.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                }
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }
}
