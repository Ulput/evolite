//! The wallet's identities on Platform: their keys (DIP-13 identity slots of the phrase, as Dash
//! Evo Tool derives them), the signer of the asset lock, and what the interface shows of them.
//! Private keys exist only in signers built for one operation from the open wallet's keys.

use async_trait::async_trait;
use dpp::identity::accessors::IdentityGettersV0;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dpp::identity::{IdentityPublicKey, KeyType, Purpose, SecurityLevel};
use dpp::platform_value::BinaryData;
use dpp::platform_value::string_encoding::Encoding;
use evolite_api::{DpnsName, Identity, IdentityKey, KeyKind, KeyLevel, KeyPurpose};
use key_wallet::bip32::{DerivationPath, ExtendedPubKey};
use key_wallet::dashcore::ecdsa::{EcdsaPublicKey, EcdsaSignature};
use key_wallet::signer::{ExtendedPubKeySigner, Signer, SignerMethod};
use platform_wallet::{ManagedIdentity, derive_identity_auth_keypair};

/// A new identity's keys: (id, purpose, level). DashPay needs ENCRYPTION to send contact
/// requests and DECRYPTION to read them; TRANSFER moves credits.
const KEYS: [(u32, Purpose, SecurityLevel); 6] = [
    (0, Purpose::AUTHENTICATION, SecurityLevel::MASTER),
    (1, Purpose::AUTHENTICATION, SecurityLevel::HIGH),
    (2, Purpose::AUTHENTICATION, SecurityLevel::CRITICAL),
    (3, Purpose::TRANSFER, SecurityLevel::CRITICAL),
    (4, Purpose::ENCRYPTION, SecurityLevel::MEDIUM),
    (5, Purpose::DECRYPTION, SecurityLevel::MEDIUM),
];

/// The keys of identity slot `index` with their private halves (key id = key index, as
/// platform-wallet's discovery expects).
pub fn new_identity_keys(
    keys: &key_wallet::Wallet,
    index: u32,
) -> Result<Vec<(IdentityPublicKey, [u8; 32])>, String> {
    KEYS.iter()
        .map(|(id, purpose, level)| {
            let (_, xprv, public) =
                derive_identity_auth_keypair(keys, crate::network::dash(), index, *id)
                    .map_err(|e| e.to_string())?;
            let key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id: *id,
                purpose: *purpose,
                security_level: *level,
                contract_bounds: None,
                key_type: KeyType::ECDSA_SECP256K1,
                read_only: false,
                data: BinaryData::new(public.to_compressed().to_vec()),
                disabled_at: None,
            });
            Ok((key, *xprv.private_key.to_bytes()))
        })
        .collect()
}

/// The private keys of an identity found in slot `index`: each published key that this
/// phrase reproduces.
pub fn found_identity_keys(
    keys: &key_wallet::Wallet,
    index: u32,
    identity: &dpp::identity::Identity,
) -> Vec<(IdentityPublicKey, [u8; 32])> {
    identity
        .public_keys()
        .iter()
        .filter(|(_, key)| key.key_type() == KeyType::ECDSA_SECP256K1)
        .filter_map(|(key_id, key)| {
            let (_, xprv, public) =
                derive_identity_auth_keypair(keys, crate::network::dash(), index, *key_id).ok()?;
            (key.data().as_slice() == public.to_compressed().as_slice())
                .then(|| (key.clone(), *xprv.private_key.to_bytes()))
        })
        .collect()
}

/// Signs the asset lock (a Core transaction) with the open wallet's keys.
pub struct AssetLockSigner(pub key_wallet::Wallet);

#[async_trait]
impl Signer for AssetLockSigner {
    type Error = String;

    fn supported_methods(&self) -> &[SignerMethod] {
        &[SignerMethod::Digest]
    }

    async fn sign_ecdsa(
        &self,
        path: &DerivationPath,
        sighash: [u8; 32],
    ) -> Result<(EcdsaSignature, EcdsaPublicKey), Self::Error> {
        let key = self.0.derive_private_key(path).map_err(|e| e.to_string())?;
        let signature = key
            .sign_recoverable(&sighash)
            .signature()
            .map_err(|e| e.to_string())?;
        Ok((signature, key.public_key()))
    }

    async fn public_key(&self, path: &DerivationPath) -> Result<EcdsaPublicKey, Self::Error> {
        Ok(self
            .0
            .derive_private_key(path)
            .map_err(|e| e.to_string())?
            .public_key())
    }
}

#[async_trait]
impl ExtendedPubKeySigner for AssetLockSigner {
    async fn extended_public_key(&self, path: &DerivationPath) -> Result<ExtendedPubKey, String> {
        self.0
            .derive_extended_public_key(path)
            .map_err(|e| e.to_string())
    }
}

/// What the interface shows of an identity platform-wallet keeps. Platform does not say when an
/// identity was made: `created_at` and `created_block` stay 0.
pub fn to_api(managed: &ManagedIdentity) -> Identity {
    let identity = &managed.identity;
    let names = managed
        .dpns_names
        .iter()
        .enumerate()
        .map(|(i, n)| DpnsName {
            label: n.label.clone(),
            primary: i == 0,
            voting_days_left: None,
        })
        .collect();
    let keys = identity
        .public_keys()
        .values()
        .filter(|k| k.disabled_at().is_none())
        .map(|k| IdentityKey {
            id: k.id(),
            level: match k.security_level() {
                SecurityLevel::MASTER => KeyLevel::Master,
                SecurityLevel::CRITICAL => KeyLevel::Critical,
                SecurityLevel::HIGH => KeyLevel::High,
                _ => KeyLevel::Medium,
            },
            purpose: match k.purpose() {
                Purpose::AUTHENTICATION if k.security_level() == SecurityLevel::MASTER => {
                    KeyPurpose::Identity
                }
                Purpose::TRANSFER => KeyPurpose::Transfers,
                Purpose::ENCRYPTION | Purpose::DECRYPTION => KeyPurpose::Encryption,
                _ => KeyPurpose::Documents,
            },
            kind: match k.key_type() {
                KeyType::ECDSA_SECP256K1 => KeyKind::Ecdsa,
                KeyType::ECDSA_HASH160 => KeyKind::EcdsaHash160,
                KeyType::BLS12_381 => KeyKind::Bls,
                KeyType::EDDSA_25519_HASH160 => KeyKind::EddsaHash160,
                KeyType::BIP13_SCRIPT_HASH => KeyKind::Bip13Script,
            },
            decrypts: k.purpose() == Purpose::DECRYPTION,
        })
        .collect();
    Identity {
        id: identity.id().to_string(Encoding::Base58),
        created_at: 0,
        created_block: 0,
        revision: identity.revision() as u32,
        key_source: managed
            .identity_index
            .map_or(tr!("not from this phrase").into(), |i| {
                format!("DIP-13 · identity {i}")
            }),
        names,
        keys,
        profile: None,
    }
}

/// Credits as duffs (1 duff = 1000 credits), as the interface counts every balance.
pub fn credits_as_duffs(credits: u64) -> i64 {
    (credits / 1000) as i64
}

/// The identity an asset lock would register: Platform makes a new identity's id from the
/// lock's outpoint (its txid and the credit output, the first), so the txid alone tells it —
/// for good, also after a restore or a restart.
pub fn registered_by(txid: &str) -> Option<String> {
    let txid: key_wallet::dashcore::Txid = txid.parse().ok()?;
    let outpoint: [u8; 36] = key_wallet::dashcore::OutPoint::new(txid, 0).into();
    Some(
        dpp::prelude::Identifier::new(dpp::util::hash::hash_double(outpoint))
            .to_string(Encoding::Base58),
    )
}

/// What an asset lock is for, by the wallet's funding account its credit output pays: an
/// identity's registration, or the top-up of the identity in a slot. The address is the
/// wallet's own, so this holds after a restart (when platform-wallet no longer tracks a
/// finished lock) and after a restore, as platform-wallet's own reconstruction reads it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LockFor {
    Registration,
    TopUp(u32),
}

/// [`LockFor`] of the asset lock `tx`, from the wallet's `accounts`; `None` for another
/// transaction or a lock of another kind (the Platform address, the pool).
pub fn lock_for(
    accounts: &key_wallet::managed_account::managed_account_collection::ManagedAccountCollection,
    tx: &key_wallet::dashcore::Transaction,
) -> Option<LockFor> {
    use key_wallet::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    let Some(TransactionPayload::AssetLockPayloadType(lock)) = &tx.special_transaction_payload
    else {
        return None;
    };
    let pays = |account: &key_wallet::managed_account::ManagedCoreKeysAccount| {
        lock.credit_outputs
            .iter()
            .any(|out| account.contains_script_pub_key(&out.script_pubkey))
    };
    if accounts.identity_registration.as_ref().is_some_and(pays) {
        return Some(LockFor::Registration);
    }
    accounts
        .identity_topup
        .iter()
        .find(|(_, account)| pays(account))
        .map(|(slot, _)| LockFor::TopUp(*slot))
}

/// What platform-wallet tracks of an asset lock: its funding, its slot and whether Platform
/// consumed it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tracked {
    pub funding: platform_wallet::AssetLockFundingType,
    pub slot: u32,
    pub consumed: bool,
}

/// How the History shows a Core asset lock (`txid`), BUG-57: the identity it funded (`@name`,
/// else the id; empty when not known), whether it registered it, and whether Platform took it.
/// `paid`: [`lock_for`]; `tracked`: platform-wallet's entry, if it still has one; `consumed`:
/// this data folder's store keeps the lock as consumed ([`super::chain::consumed_locks`]);
/// `unfinished`: it is the lock of a cut-short top-up or creation, which Identity offers to
/// continue.
pub fn lock_line(
    txid: &str,
    paid: Option<LockFor>,
    tracked: Option<Tracked>,
    consumed: bool,
    unfinished: bool,
    mine: &[evolite_api::IdentityChoice],
) -> (String, bool, evolite_api::Credits) {
    use evolite_api::Credits;
    use platform_wallet::AssetLockFundingType as Funding;
    let shown = |c: &evolite_api::IdentityChoice| {
        c.name
            .as_ref()
            .map_or_else(|| c.id.clone(), |n| format!("@{n}"))
    };
    // A registration's identity is in its txid: for good, whatever the slot platform-wallet
    // gives a lock found by a restore (always 0).
    let registered = registered_by(txid).and_then(|id| mine.iter().find(|c| c.id == id));
    let tracked_for = tracked.and_then(|t| match t.funding {
        Funding::IdentityRegistration => Some(LockFor::Registration),
        Funding::IdentityTopUp => Some(LockFor::TopUp(t.slot)),
        _ => None,
    });
    let what = if registered.is_some() {
        Some(LockFor::Registration)
    } else {
        paid.or(tracked_for)
    };
    let not_added = |known: bool| match (unfinished, known) {
        (true, _) => Credits::NotAdded,
        (false, true) => Credits::Added,
        (false, false) => Credits::Unknown,
    };
    match what {
        // The identity exists: Platform took the lock that made it.
        Some(LockFor::Registration) => (
            registered.map(shown).unwrap_or_default(),
            true,
            not_added(registered.is_some()),
        ),
        Some(LockFor::TopUp(slot)) => {
            let identity = mine.iter().find(|c| c.slot == slot).map(shown);
            // Only what platform-wallet knows was consumed: tracked so, or kept so in this
            // folder's store (it keeps consumed rows for good, and does not reload them). Not
            // tracked says nothing: a lock built in another data folder, or one whose
            // transaction was swept, is not tracked either (BUG-57 r1).
            let taken = consumed || tracked.is_some_and(|t| t.consumed);
            (
                identity.clone().unwrap_or_default(),
                false,
                not_added(identity.is_some() && taken),
            )
        }
        None => (String::new(), false, not_added(false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_asset_lock_names_the_identity_it_funded() {
        use evolite_api::IdentityChoice;
        use platform_wallet::AssetLockFundingType as Funding;
        let txid = "35af26a8000000000000000000000000000000000000000000000000c0c81f18";
        let registered = registered_by(txid).unwrap();
        let mine = [
            IdentityChoice {
                id: registered.clone(),
                name: Some("alice".into()),
                slot: 0,
            },
            IdentityChoice {
                id: "Bob1111111111111111111111111111111111111111".into(),
                name: None,
                slot: 1,
            },
        ];
        let name = |paid, tracked| lock_line(txid, paid, tracked, false, false, &mine).0;
        let other = "11".repeat(32);
        let named = |paid, tracked| lock_line(&other, paid, tracked, false, false, &mine).0;
        let tracked = |funding, slot| {
            Some(Tracked {
                funding,
                slot,
                consumed: false,
            })
        };
        // Registration: known from the txid alone, tracked or not.
        assert_eq!(name(None, None), "@alice");
        // A top-up still tracked: the identity of its slot, by id when it has no name.
        assert_eq!(named(None, tracked(Funding::IdentityTopUp, 1)), mine[1].id);
        // No longer tracked (after a restart): the account its credit output pays says it.
        assert_eq!(named(Some(LockFor::TopUp(1)), None), mine[1].id);
        // A Platform address, an unknown lock, a broken txid: not said.
        assert_eq!(named(None, tracked(Funding::AssetLockAddressTopUp, 0)), "");
        assert_eq!(named(None, None), "");
        assert_eq!(lock_line("nope", None, None, false, false, &mine).0, "");
        // Platform's own formula: the double SHA-256 of the 36-byte outpoint.
        assert_ne!(registered, registered_by(&other).unwrap());
    }

    /// BUG-57: a lock says whether it made the identity or topped it up, and whether Platform
    /// took it — "not added" only for the lock Identity offers to continue; once Platform took
    /// it the same line says "added".
    #[test]
    fn an_asset_lock_says_what_it_made_and_whether_credits_came() {
        use evolite_api::{Credits, IdentityChoice};
        use platform_wallet::AssetLockFundingType as Funding;
        let made = "35af26a8000000000000000000000000000000000000000000000000c0c81f18";
        let mine = [
            IdentityChoice {
                id: registered_by(made).unwrap(),
                name: Some("alice".into()),
                slot: 0,
            },
            IdentityChoice {
                id: "Bob1111111111111111111111111111111111111111".into(),
                name: Some("bob".into()),
                slot: 3,
            },
        ];
        let topup = "22".repeat(32);
        let tracked = |funding, slot, consumed| {
            Some(Tracked {
                funding,
                slot,
                consumed,
            })
        };
        // The creation of @alice: made, so taken.
        assert_eq!(
            lock_line(made, None, None, false, false, &mine),
            ("@alice".into(), true, Credits::Added)
        );
        // A registration found by a restore sits in slot 0 as platform-wallet tracks it: its
        // identity is by its txid only, never the identity of slot 0.
        let lost = "33".repeat(32);
        assert_eq!(
            lock_line(
                &lost,
                None,
                tracked(Funding::IdentityRegistration, 0, false),
                false,
                false,
                &mine
            ),
            (String::new(), true, Credits::Unknown)
        );
        // A creation cut short: no identity yet, the lock waits.
        assert_eq!(
            lock_line(
                &lost,
                Some(LockFor::Registration),
                tracked(Funding::IdentityRegistration, 1, false),
                false,
                true,
                &mine
            ),
            (String::new(), true, Credits::NotAdded)
        );
        // A top-up cut short (werner555's 0.5 DASH): @bob's, not added.
        let stopped = tracked(Funding::IdentityTopUp, 3, false);
        assert_eq!(
            lock_line(&topup, Some(LockFor::TopUp(3)), stopped, false, true, &mine),
            ("@bob".into(), false, Credits::NotAdded)
        );
        // The same lock once "Continue" went through: consumed, added.
        let done = tracked(Funding::IdentityTopUp, 3, true);
        assert_eq!(
            lock_line(&topup, Some(LockFor::TopUp(3)), done, false, false, &mine),
            ("@bob".into(), false, Credits::Added)
        );
        // After a restart platform-wallet no longer tracks it, and this folder's store keeps it
        // as consumed (kestrel7's 0.01): still @bob's, added.
        assert_eq!(
            lock_line(&topup, Some(LockFor::TopUp(3)), None, true, false, &mine),
            ("@bob".into(), false, Credits::Added)
        );
        // Not tracked and no consumed row here (r1 must): built in another data folder, or its
        // transaction swept — @bob's, but not said to be added.
        assert_eq!(
            lock_line(&topup, Some(LockFor::TopUp(3)), None, false, false, &mine),
            ("@bob".into(), false, Credits::Unknown)
        );
        // A consumed row outweighs a tracked entry not yet marked consumed.
        assert_eq!(
            lock_line(&topup, Some(LockFor::TopUp(3)), stopped, true, false, &mine).2,
            Credits::Added
        );
        // Tracked and not consumed, not offered to continue (being sent now, found by a
        // restore, awaiting its ChainLock): not said either way.
        assert_eq!(
            lock_line(
                &topup,
                Some(LockFor::TopUp(3)),
                stopped,
                false,
                false,
                &mine
            )
            .2,
            Credits::Unknown
        );
        // The Platform address's lock: nothing to say of credits or an identity.
        assert_eq!(
            lock_line(
                &topup,
                None,
                tracked(Funding::AssetLockAddressTopUp, 0, true),
                false,
                false,
                &mine
            ),
            (String::new(), false, Credits::Unknown)
        );
    }

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    /// BUG-57: the wallet's own funding accounts tell what a lock is for, without
    /// platform-wallet's tracking (gone for a finished lock after a restart): the top-up of
    /// slot 2's identity, a registration, or neither.
    #[test]
    fn a_lock_is_known_by_the_account_its_credit_output_pays() {
        use key_wallet::account::AccountType;
        use key_wallet::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
        use key_wallet::dashcore::blockdata::transaction::special_transaction::asset_lock::AssetLockPayload;
        use key_wallet::dashcore::{ScriptBuf, Transaction, TxOut};
        use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
        use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
        let mut keys = super::super::keys_of(PHRASE).unwrap();
        keys.add_account(
            AccountType::IdentityTopUp {
                registration_index: 2,
            },
            None,
        )
        .unwrap();
        let wallet = ManagedWalletInfo::from_wallet(&keys, 0);
        let first = |account: &key_wallet::managed_account::ManagedCoreKeysAccount| {
            account.managed_account_type().address_pools()[0]
                .addresses
                .values()
                .next()
                .expect("an address made")
                .address
                .script_pubkey()
        };
        let top_up = first(&wallet.accounts.identity_topup[&2]);
        let registration = first(wallet.accounts.identity_registration.as_ref().unwrap());
        let lock = |to: ScriptBuf, payload: bool| Transaction {
            version: 3,
            lock_time: 0,
            input: vec![],
            output: vec![],
            special_transaction_payload: payload.then(|| {
                TransactionPayload::AssetLockPayloadType(AssetLockPayload {
                    version: 1,
                    credit_outputs: vec![TxOut {
                        value: 50_000_000,
                        script_pubkey: to,
                    }],
                })
            }),
        };
        let accounts = &wallet.accounts;
        assert_eq!(
            lock_for(accounts, &lock(top_up.clone(), true)),
            Some(LockFor::TopUp(2))
        );
        assert_eq!(
            lock_for(accounts, &lock(registration, true)),
            Some(LockFor::Registration)
        );
        assert_eq!(lock_for(accounts, &lock(ScriptBuf::new(), true)), None);
        assert_eq!(lock_for(accounts, &lock(top_up, false)), None, "no lock");
    }

    #[test]
    fn found_keys_are_the_keys_made_for_the_slot() {
        let keys = super::super::keys_of(PHRASE).unwrap();
        let made = new_identity_keys(&keys, 2).unwrap();
        let identity = dpp::identity::Identity::V0(dpp::identity::v0::IdentityV0 {
            id: dpp::prelude::Identifier::random(),
            public_keys: made.iter().map(|(k, _)| (k.id(), k.clone())).collect(),
            balance: 0,
            revision: 0,
        });
        assert_eq!(found_identity_keys(&keys, 2, &identity).len(), KEYS.len());
        assert!(found_identity_keys(&keys, 1, &identity).is_empty());
    }
}
