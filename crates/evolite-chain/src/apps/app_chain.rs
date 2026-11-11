//! The identity an app signs in as (App Connect v2, [`crate::app_chain_v2`]): the shown one,
//! read from Platform again, with the keys this phrase reproduces to sign its updates and the
//! response. App Connect v1 (`dash-key:` v1, `dash-st:`) is not supported (the customer,
//! 2026-11-11): its keys left on identities are for key management (FEAT-0020) to show.

use dpp::identity::accessors::IdentityGettersV0;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::{IdentityPublicKey, KeyType, Purpose, SecurityLevel};
use dpp::prelude::Identifier;
use platform_wallet::PlatformWallet;
use simple_signer::signer::SimpleSigner;

use crate::chain::Chain;
use crate::identity;

/// The identity an app signs in as: the shown one, with its keys as Platform has them now.
pub struct SignInIdentity {
    pub id: Identifier,
    pub keys: Vec<IdentityPublicKey>,
    /// Every key of it this phrase reproduces, with its private half (for signing).
    found: Vec<(IdentityPublicKey, [u8; 32])>,
}

impl SignInIdentity {
    pub fn id_bytes(&self) -> [u8; 32] {
        self.id.to_buffer()
    }

    /// The id the next added key takes.
    pub fn next_key_id(&self) -> u32 {
        self.keys.iter().map(|k| k.id() + 1).max().unwrap_or(0)
    }

    /// A signer of only the key of `purpose` and `level` this phrase reproduces (ECDSA), unbound
    /// (App Connect v2 signs its response with no bound, limited key).
    pub(crate) fn signer(
        &self,
        purpose: Purpose,
        level: SecurityLevel,
    ) -> Option<(IdentityPublicKey, SimpleSigner)> {
        let (key, private) = self.found.iter().find(|(k, _)| {
            k.purpose() == purpose
                && k.security_level() == level
                && k.key_type() == KeyType::ECDSA_SECP256K1
                && k.disabled_at().is_none()
                && k.contract_bounds().is_none()
        })?;
        let mut signer = SimpleSigner::default();
        signer.add_identity_public_keys([(key.clone(), *private)]);
        Some((key.clone(), signer))
    }
}

impl Drop for SignInIdentity {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for (_, private) in &mut self.found {
            private.zeroize();
        }
    }
}

impl Chain {
    /// The shown identity, read from Platform again (an app may have added keys elsewhere). Its
    /// MASTER key must be from this phrase: it signs the updates that add or disable keys.
    pub async fn sign_in_identity(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<SignInIdentity, String> {
        let (shown, _) = Self::first_identity(wallet, keys).await?;
        let _ = wallet.identity().refresh_identity(&shown.id()).await;
        let (identity, slot) = {
            let state = wallet.state().await;
            let managed = state
                .identity_manager
                .managed_identity(&shown.id())
                .ok_or(tr!("The wallet has no identity"))?;
            (
                managed.identity.clone(),
                managed
                    .identity_index
                    .ok_or(tr!("The wallet has no identity"))?,
            )
        };
        let found = identity::found_identity_keys(keys, slot, &identity);
        if !found.iter().any(|(k, _)| {
            k.purpose() == Purpose::AUTHENTICATION
                && k.security_level() == SecurityLevel::MASTER
                && k.disabled_at().is_none()
        }) {
            return Err(tr!("This identity's MASTER key is not from your phrase: you cannot sign in to apps as it from here").into());
        }
        Ok(SignInIdentity {
            id: identity.id(),
            keys: identity.public_keys().values().cloned().collect(),
            found,
        })
    }
}
