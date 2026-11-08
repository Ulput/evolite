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
use evolite_api::{CoreError, DpnsName, Identity, IdentityKey, KeyKind, KeyLevel, KeyPurpose};
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
        // As the choice of identities counts it (`Real::wallet`).
        slot: managed.identity_index.unwrap_or_default(),
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

/// [`LockFor`] of each asset lock in `locks` (txid, transaction): by the wallet's accounts,
/// else by the top-up addresses of the found identities' `slots` (`top_ups`, derived from
/// `keys` as needed) — a wallet restored from the phrase has no top-up account at all.
pub fn lock_fors(
    accounts: &key_wallet::managed_account::managed_account_collection::ManagedAccountCollection,
    top_ups: &mut TopUpScripts,
    wallet: key_wallet_manager::WalletId,
    keys: Option<&key_wallet::Wallet>,
    slots: impl IntoIterator<Item = u32>,
    locks: &[(String, &key_wallet::dashcore::Transaction)],
) -> std::collections::HashMap<String, LockFor> {
    let mut known = std::collections::HashMap::new();
    let mut unpaid = Vec::new();
    for (txid, tx) in locks {
        match lock_for(accounts, tx) {
            Some(what) => {
                known.insert(txid.clone(), what);
            }
            None => unpaid.push((txid, *tx)),
        }
    }
    if unpaid.is_empty() {
        return known;
    }
    let txs: Vec<_> = unpaid.iter().map(|(_, tx)| *tx).collect();
    top_ups.cover(wallet, keys, slots, &txs);
    for (txid, tx) in unpaid {
        if let Some(what) = top_ups.lock_for(tx) {
            known.insert(txid.clone(), what);
        }
    }
    known
}

/// The first top-up addresses derived of a slot, and how many more past the last one a lock
/// paid (key-wallet's gap for the funding accounts).
const TOP_UP_GAP: usize = key_wallet::gap_limit::DEFAULT_SPECIAL_GAP_LIMIT as usize;
/// Never more addresses of one slot than key-wallet's own most.
const TOP_UP_MOST: usize = key_wallet::gap_limit::MAX_GAP_LIMIT as usize;

/// The DIP-9 top-up addresses (`IdentityTopUp { registration_index: slot }`) of the wallet's
/// identity slots, as scripts, derived from the open keys (BUG-57 follow-up). platform-wallet
/// makes that account only when it builds a top-up, and a restore from the phrase has none,
/// so [`lock_for`] cannot tell a restored top-up from another lock. Only public data is kept
/// (the account xpub and its addresses); nothing is written to the wallet or the store.
#[derive(Default)]
pub struct TopUpScripts {
    wallet: Option<key_wallet_manager::WalletId>,
    slots: std::collections::BTreeMap<
        u32,
        (
            key_wallet::bip32::ExtendedPubKey,
            Vec<key_wallet::dashcore::ScriptBuf>,
        ),
    >,
}

impl TopUpScripts {
    /// Makes the scripts of `slots` cover `locks`: the first [`TOP_UP_GAP`] of a slot new to
    /// the cache, and more while a lock pays one of a slot's last [`TOP_UP_GAP`]. Another
    /// wallet starts afresh. Without `keys` (none open) a new slot waits for them; the scripts
    /// already derived still extend, from the xpub.
    pub fn cover(
        &mut self,
        wallet: key_wallet_manager::WalletId,
        keys: Option<&key_wallet::Wallet>,
        slots: impl IntoIterator<Item = u32>,
        locks: &[&key_wallet::dashcore::Transaction],
    ) {
        if self.wallet != Some(wallet) {
            self.slots.clear();
            self.wallet = Some(wallet);
        }
        let paid: Vec<&key_wallet::dashcore::ScriptBuf> = locks
            .iter()
            .filter_map(|tx| credit_outputs(tx))
            .flatten()
            .map(|out| &out.script_pubkey)
            .collect();
        for slot in slots {
            use std::collections::btree_map::Entry;
            let (xpub, scripts) = match self.slots.entry(slot) {
                Entry::Occupied(known) => known.into_mut(),
                Entry::Vacant(new) => {
                    let Some(xpub) = keys.and_then(|keys| top_up_xpub(keys, slot)) else {
                        continue;
                    };
                    new.insert((xpub, Vec::new()))
                }
            };
            loop {
                let used = scripts.iter().rposition(|s| paid.contains(&s));
                let want = used
                    .map_or(TOP_UP_GAP, |i| i + 1 + TOP_UP_GAP)
                    .min(TOP_UP_MOST);
                if scripts.len() >= want {
                    break;
                }
                let before = scripts.len();
                scripts.extend((before..want).map_while(|i| top_up_script(xpub, i as u32)));
                if scripts.len() == before {
                    break;
                }
            }
        }
    }

    /// The slot whose top-up address the lock `tx`'s credit output pays.
    pub fn lock_for(&self, tx: &key_wallet::dashcore::Transaction) -> Option<LockFor> {
        let outs = credit_outputs(tx)?;
        self.slots
            .iter()
            .find(|(_, (_, scripts))| outs.iter().any(|o| scripts.contains(&o.script_pubkey)))
            .map(|(slot, _)| LockFor::TopUp(*slot))
    }

    /// How many scripts of `slot` are derived (tests).
    #[cfg(test)]
    pub fn derived(&self, slot: u32) -> usize {
        self.slots.get(&slot).map_or(0, |(_, s)| s.len())
    }
}

/// The credit outputs of an asset lock; `None` for another transaction.
fn credit_outputs(
    tx: &key_wallet::dashcore::Transaction,
) -> Option<&[key_wallet::dashcore::TxOut]> {
    use key_wallet::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
    match &tx.special_transaction_payload {
        Some(TransactionPayload::AssetLockPayloadType(lock)) => Some(&lock.credit_outputs),
        _ => None,
    }
}

/// The xpub of slot `slot`'s top-up account (its path is hardened, so it takes the private
/// key, which is dropped here).
fn top_up_xpub(keys: &key_wallet::Wallet, slot: u32) -> Option<key_wallet::bip32::ExtendedPubKey> {
    let path = key_wallet::account::AccountType::IdentityTopUp {
        registration_index: slot,
    }
    .derivation_path(crate::network::dash())
    .ok()?;
    let xprv = keys.derive_extended_private_key(&path).ok()?;
    Some(key_wallet::bip32::ExtendedPubKey::from_priv(&xprv))
}

/// The P2PKH script of the top-up address `index` of an account (`xpub/index`, as key-wallet's
/// pool of a funding account derives it).
fn top_up_script(
    xpub: &key_wallet::bip32::ExtendedPubKey,
    index: u32,
) -> Option<key_wallet::dashcore::ScriptBuf> {
    let child = key_wallet::bip32::ChildNumber::from_normal_idx(index).ok()?;
    let key = xpub.derive_pub(&[child]).ok()?;
    let key = key_wallet::dashcore::PublicKey::new(key.public_key);
    Some(key_wallet::dashcore::Address::p2pkh(&key, crate::network::dash()).script_pubkey())
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

/// The DPNS label a new identity is to get: `None` for no name or one of only spaces
/// (FEAT-0014 R-7); a contested one is refused before any money moves.
pub(super) fn new_identity_label(name: Option<&str>) -> Result<Option<String>, CoreError> {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    let label = crate::dpns::label(name)?;
    if crate::dpns::is_contested(&label) {
        return Err(CoreError::NotAvailable(
            tr!("Contested names are decided by a masternode vote, which evolite cannot do yet. Add a digit from 2 to 9 to the name or make it longer than 19 characters").into(),
        ));
    }
    Ok(Some(label))
}

/// Keeps the name slot `index` is being made with, so a run cut short finishes it. A nameless
/// identity keeps none (R-7) and drops one an earlier attempt in the same slot left there (one
/// that failed before its lock): `nameless_slot` would take the slot for one cut short and
/// «Continue» would register that old name (review r1 should 1).
pub(super) fn remember_name(
    pending: &super::pending::PendingNames,
    index: u32,
    label: Option<&str>,
) -> Result<(), String> {
    match label {
        Some(label) => pending.set(index, label),
        None => pending.remove(index),
    }
}

/// After "Create an identity": the identity made in slot `made`
/// is the one the wallet acts as from now on (analyst-44), through the same choice as
/// `ChooseIdentity`; a failure leaves the choice as it was.
pub(super) fn choose_created(
    wallet: key_wallet_manager::WalletId,
    made: Result<u32, CoreError>,
) -> Result<(), CoreError> {
    made.map(|slot| super::chain::choose_identity(wallet, slot))
}

/// "Continue" finished the identity of slot `index`: no name is pending for it any more, and
/// it is the one actions go from, as one made at once (analyst-44): the banner the form put up
/// waits for this choice (review r1 should 2).
pub(super) fn finish_resumed(
    pending: &super::pending::PendingNames,
    wallet: key_wallet_manager::WalletId,
    index: u32,
) -> Result<(), String> {
    pending.remove(index)?;
    super::chain::choose_identity(wallet, index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review r1 should 2: an identity finished by «Continue» leaves the pending names and
    /// becomes the chosen one, as one made at once.
    #[test]
    fn a_resumed_identity_is_chosen_and_keeps_no_pending_name() {
        use super::super::chain;
        let dir = std::env::temp_dir().join(format!("evolite-f14-resumed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pending = super::super::pending::PendingNames::new(dir.join("pending-names.json"));
        pending.set(2, "half-made").unwrap();
        let wallet: key_wallet_manager::WalletId = [0x15; 32];
        chain::choose_identity(wallet, 0);
        finish_resumed(&pending, wallet, 2).unwrap();
        assert!(pending.load().is_empty(), "the name is no longer pending");
        assert_eq!(chain::shown_slot(wallet, [0, 1, 2].into_iter()), Some(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    /// An asset lock paying `to` (its one credit output).
    fn lock_to(to: key_wallet::dashcore::ScriptBuf) -> key_wallet::dashcore::Transaction {
        use key_wallet::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
        use key_wallet::dashcore::blockdata::transaction::special_transaction::asset_lock::AssetLockPayload;
        key_wallet::dashcore::Transaction {
            version: 3,
            lock_time: 0,
            input: vec![],
            output: vec![],
            special_transaction_payload: Some(TransactionPayload::AssetLockPayloadType(
                AssetLockPayload {
                    version: 1,
                    credit_outputs: vec![key_wallet::dashcore::TxOut {
                        value: 50_000_000,
                        script_pubkey: to,
                    }],
                },
            )),
        }
    }

    /// BUG-57 follow-up: a wallet restored from the phrase has no top-up account (platform-wallet
    /// makes one only to build a top-up), yet its past top-ups get their identity — by the
    /// top-up addresses of the found identities' slots, the very ones platform-wallet pays —
    /// and stay `Unknown` until the store says Platform took them.
    #[test]
    fn a_restored_top_up_gets_its_identity_by_its_slots_addresses() {
        use evolite_api::{Credits, IdentityChoice};
        use key_wallet::account::AccountType;
        use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
        let keys = super::super::keys_of(PHRASE).unwrap();
        let restored = ManagedWalletInfo::from_wallet(&keys, 0);
        assert!(
            restored.accounts.identity_topup.is_empty(),
            "a restore has none"
        );
        // Where platform-wallet's top-ups went: the accounts it adds to build them.
        let mut built = super::super::keys_of(PHRASE).unwrap();
        for slot in [0, 2] {
            built
                .add_account(
                    AccountType::IdentityTopUp {
                        registration_index: slot,
                    },
                    None,
                )
                .unwrap();
        }
        let built = ManagedWalletInfo::from_wallet(&built, 0);
        let paid_to = |slot: u32| -> Vec<key_wallet::dashcore::ScriptBuf> {
            use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
            built.accounts.identity_topup[&slot]
                .managed_account_type()
                .address_pools()[0]
                .addresses
                .values()
                .map(|a| a.address.script_pubkey())
                .collect()
        };
        let (two, zero) = (paid_to(2), paid_to(0));
        assert!(two.len() >= TOP_UP_GAP);
        let xpub = top_up_xpub(&keys, 2).unwrap();
        for (i, script) in two.iter().enumerate() {
            assert_eq!(
                top_up_script(&xpub, i as u32).as_ref(),
                Some(script),
                "address {i}"
            );
        }
        let (a, b, c, d) = (
            lock_to(two[0].clone()),
            lock_to(two[3].clone()),
            lock_to(zero[1].clone()),
            lock_to(key_wallet::dashcore::ScriptBuf::new()),
        );
        let txid = |n: u8| format!("{n:02x}").repeat(32);
        let locks = [(txid(1), &a), (txid(2), &b), (txid(3), &c), (txid(4), &d)];
        let mut top_ups = TopUpScripts::default();
        let found = lock_fors(
            &restored.accounts,
            &mut top_ups,
            [7; 32],
            Some(&keys),
            [0, 2],
            &locks,
        );
        assert_eq!(found.get(&txid(1)), Some(&LockFor::TopUp(2)));
        assert_eq!(found.get(&txid(2)), Some(&LockFor::TopUp(2)));
        assert_eq!(found.get(&txid(3)), Some(&LockFor::TopUp(0)));
        assert_eq!(found.get(&txid(4)), None, "another lock stays unnamed");
        // The History line: whose top-up it is; credits only by the store's consumed row.
        let mine = [
            IdentityChoice {
                id: "Alice111111111111111111111111111111111111111".into(),
                name: Some("alice".into()),
                slot: 0,
            },
            IdentityChoice {
                id: "Bob1111111111111111111111111111111111111111".into(),
                name: Some("bob".into()),
                slot: 2,
            },
        ];
        let line =
            |t: &str, consumed| lock_line(t, found.get(t).copied(), None, consumed, false, &mine);
        assert_eq!(
            line(&txid(1), false),
            ("@bob".into(), false, Credits::Unknown)
        );
        assert_eq!(line(&txid(1), true), ("@bob".into(), false, Credits::Added));
        assert_eq!(
            line(&txid(3), false),
            ("@alice".into(), false, Credits::Unknown)
        );
        assert_eq!(
            line(&txid(4), true),
            (String::new(), false, Credits::Unknown)
        );
        // Locked again (no keys): what was derived still names them.
        let again = lock_fors(
            &restored.accounts,
            &mut top_ups,
            [7; 32],
            None,
            [0, 2],
            &locks,
        );
        assert_eq!(again, found);
    }

    /// The addresses of a slot go on while top-ups use them: the first gap, then a gap past the
    /// last one paid; past that, nothing; a new slot waits for the keys; another wallet starts
    /// afresh.
    #[test]
    fn top_up_addresses_go_on_past_the_last_one_paid() {
        let keys = super::super::keys_of(PHRASE).unwrap();
        let xpub = top_up_xpub(&keys, 1).unwrap();
        let at = |i: u32| lock_to(top_up_script(&xpub, i).unwrap());
        let mut t = TopUpScripts::default();
        t.cover([1; 32], Some(&keys), [1], &[]);
        assert_eq!(t.derived(1), TOP_UP_GAP);
        assert_eq!(t.lock_for(&at(TOP_UP_GAP as u32)), None, "not derived yet");
        let gap = TOP_UP_GAP as u32;
        // Each top-up within the gap of the one before; without keys, from the xpub.
        let chain = [at(gap - 1), at(2 * gap - 1), at(3 * gap - 1)];
        t.cover([1; 32], None, [1], &chain.iter().collect::<Vec<_>>());
        assert_eq!(t.derived(1), 4 * TOP_UP_GAP);
        assert_eq!(t.lock_for(&chain[2]), Some(LockFor::TopUp(1)));
        assert_eq!(t.lock_for(&at(8 * gap)), None, "past the gap");
        t.cover([1; 32], None, [3], &[]);
        assert_eq!(t.derived(3), 0, "a new slot needs the keys");
        t.cover([2; 32], Some(&keys), [1], &[]);
        assert_eq!(t.derived(1), TOP_UP_GAP, "another wallet starts afresh");
        assert_eq!(t.lock_for(&chain[2]), None);
    }

    /// FEAT-0014 AC-T2: an empty name (after trimming) is no name; a name is checked as
    /// before, a contested one refused before any money moves.
    #[test]
    fn an_empty_name_makes_a_nameless_identity() {
        assert_eq!(new_identity_label(None), Ok(None));
        assert_eq!(new_identity_label(Some("")), Ok(None));
        assert_eq!(new_identity_label(Some("   ")), Ok(None));
        assert_eq!(
            new_identity_label(Some(" Satoshi-Apps2 ")),
            Ok(Some("satoshi-apps2".into()))
        );
        assert_eq!(new_identity_label(Some("ab")), Err(CoreError::BadName));
        assert!(matches!(
            new_identity_label(Some("alice")),
            Err(CoreError::NotAvailable(_))
        ));
    }

    /// FEAT-0014 AC-T2, R-7, review r1 should 1: a nameless identity keeps no name in
    /// `pending-names.json` — none of its own, and not the one an earlier named attempt in the
    /// same slot left there (it failed before its lock) — so after a restart `nameless_slot`
    /// (which looks only at the slots kept there) finds nothing and «Creation stopped» with the
    /// old name is not shown; other slots stay; a named one is kept until its name is in DPNS
    /// (AC-T3).
    #[test]
    fn a_nameless_identity_keeps_no_pending_name() {
        let dir = std::env::temp_dir().join(format!("evolite-f14-pending-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pending-names.json");
        let pending = super::super::pending::PendingNames::new(path.clone());
        pending.set(1, "older").unwrap();
        let before = std::fs::read(&path).unwrap();
        remember_name(&pending, 3, None).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "nothing for slot 3: the file is untouched"
        );
        assert!(!pending.load().contains_key(&3));
        // The slot an earlier named attempt left behind.
        pending.set(3, "stale").unwrap();
        remember_name(&pending, 3, None).unwrap();
        assert!(
            !pending.load().contains_key(&3),
            "the old name of slot 3 goes"
        );
        assert_eq!(
            pending.load().get(&1).map(String::as_str),
            Some("older"),
            "other slots stay"
        );
        // With a name: kept before the lock, for «Continue».
        remember_name(&pending, 4, Some("satoshi-apps")).unwrap();
        assert_eq!(
            pending.load().get(&4).map(String::as_str),
            Some("satoshi-apps")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-0014 AC-T4 (analyst-44): once made, the new identity is the chosen one — the same
    /// choice `ChooseIdentity` makes; a failure leaves the choice alone.
    #[test]
    fn the_new_identity_is_chosen_only_once_made() {
        use super::super::chain;
        let wallet: key_wallet_manager::WalletId = [0x14; 32];
        chain::choose_identity(wallet, 0);
        let slots = || [0, 1, 2, 3].into_iter();
        assert!(choose_created(wallet, Err(CoreError::NotEnough)).is_err());
        assert_eq!(chain::shown_slot(wallet, slots()), Some(0), "a failure");
        assert_eq!(choose_created(wallet, Ok(3)), Ok(()));
        assert_eq!(chain::shown_slot(wallet, slots()), Some(3));
    }
}
