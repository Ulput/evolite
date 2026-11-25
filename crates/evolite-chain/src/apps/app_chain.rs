//! The identity an app signs in as (App Connect v2, [`crate::app_chain_v2`]): the shown one,
//! read from Platform again, with the keys this phrase reproduces to sign its updates and the
//! response. App Connect v1 (`dash-key:` v1, `dash-st:`) is not supported (the customer,
//! 2026-11-11): its keys left on identities are for key management (FEAT-0020) to show.

use dpp::identity::accessors::IdentityGettersV0;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::{IdentityPublicKey, KeyType, Purpose, SecurityLevel};
use dpp::prelude::Identifier;
use platform_wallet::PlatformWallet;

use crate::chain::Chain;
use crate::identity;

/// The identity an app signs in as: the shown one, with its keys as Platform has them now.
pub struct SignInIdentity {
    pub id: Identifier,
    pub keys: Vec<IdentityPublicKey>,
    /// Its slot in the phrase (DIP-13 identity index): new keys are derived there.
    pub slot: u32,
    /// Every key of it this phrase reproduces, with its private half (for signing).
    found: Vec<(IdentityPublicKey, [u8; 32])>,
}

impl SignInIdentity {
    /// An identity of `keys` as a test reads it again: no private halves (BUG-184's tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(id: Identifier, keys: Vec<IdentityPublicKey>) -> Self {
        Self {
            id,
            keys,
            slot: 0,
            found: Vec::new(),
        }
    }

    pub fn id_bytes(&self) -> [u8; 32] {
        self.id.to_buffer()
    }

    /// The id the next added key takes.
    pub fn next_key_id(&self) -> u32 {
        // default-ok: no keys yet: the first id is 0
        self.keys.iter().map(|k| k.id() + 1).max().unwrap_or(0)
    }

    /// BUG-298: whether the response can be signed: an acting AUTHENTICATION HIGH of this phrase,
    /// ECDSA, unbound (the one [`Self::signer`] takes). Checked before the dialog.
    pub fn can_sign_in(&self) -> bool {
        self.signer(Purpose::AUTHENTICATION, SecurityLevel::HIGH)
            .is_some()
    }

    /// A signer of only the key of `purpose` and `level` this phrase reproduces (ECDSA), unbound
    /// (App Connect v2 signs its response with no bound, limited key).
    pub(crate) fn signer(
        &self,
        purpose: Purpose,
        level: SecurityLevel,
    ) -> Option<(IdentityPublicKey, crate::key_signer::KeySigner)> {
        let (key, private) = self.found.iter().find(|(k, _)| {
            k.purpose() == purpose
                && k.security_level() == level
                && k.key_type() == KeyType::ECDSA_SECP256K1
                && k.disabled_at().is_none()
                && k.contract_bounds().is_none()
        })?;
        let mut signer = crate::key_signer::KeySigner::default();
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
        let shown = Self::shown_identity(wallet).await?;
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
            slot,
            found,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpp::identity::identity_public_key::contract_bounds::ContractBounds;
    use dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dpp::platform_value::BinaryData;

    fn key(id: u32, level: SecurityLevel, bound: bool, disabled: bool) -> IdentityPublicKey {
        IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id,
            purpose: Purpose::AUTHENTICATION,
            security_level: level,
            contract_bounds: bound.then(|| ContractBounds::SingleContract {
                id: Identifier::from([7; 32]),
            }),
            key_type: KeyType::ECDSA_SECP256K1,
            read_only: false,
            data: BinaryData::new(vec![2; 33]),
            disabled_at: disabled.then_some(1),
        })
    }

    fn reproduced(keys: Vec<IdentityPublicKey>) -> SignInIdentity {
        let mut who = SignInIdentity::for_tests(Identifier::from([1; 32]), keys.clone());
        who.found = keys.into_iter().map(|k| (k, [3; 32])).collect();
        who
    }

    /// BUG-298: App Connect refuses before its dialog unless an acting, unbound HIGH of this
    /// phrase signs the response: CRITICAL (DashPay takes it) is not one, nor a disabled or bound
    /// HIGH, nor a HIGH this phrase does not reproduce. Mutant: `can_sign_in` true → red.
    #[test]
    fn only_an_acting_unbound_high_of_this_phrase_signs_in() {
        let high = key(1, SecurityLevel::HIGH, false, false);
        assert!(reproduced(vec![high.clone()]).can_sign_in());
        assert!(!reproduced(vec![key(2, SecurityLevel::CRITICAL, false, false)]).can_sign_in());
        assert!(!reproduced(vec![key(1, SecurityLevel::HIGH, false, true)]).can_sign_in());
        assert!(!reproduced(vec![key(1, SecurityLevel::HIGH, true, false)]).can_sign_in());
        assert!(
            !SignInIdentity::for_tests(Identifier::from([1; 32]), vec![high]).can_sign_in(),
            "on the identity, not of this phrase"
        );
    }
}
