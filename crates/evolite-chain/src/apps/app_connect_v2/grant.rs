//! What the wallet grants for an App Connect v2 request (FEAT-0023, round 3), as the DIP sets it:
//! "What an App May Request", "Limits", "Session Keys", "Encryption Keys" and the response.
//!
//! Everything here is pure: what Platform says (owners, contract settings, the identity's keys)
//! comes in as arguments, so each rule is checked before the password and before anything is
//! written. The core fetches the facts and does the writing.

use std::collections::HashMap;

use dpp::dashcore::hashes::{Hash, hash160};
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dpp::identity::identity_public_key::v1::IdentityPublicKeyV1;
use dpp::identity::{IdentityPublicKey, KeyType, Purpose, SecurityLevel};
use dpp::platform_value::BinaryData;
use dpp::prelude::Identifier;
use dpp::secp256k1::{PublicKey, SecretKey};
use zeroize::Zeroizing;

use super::keys::{self, SLOT};
use super::{AuthBinding, Bounds, EncBinding, EncPurpose, Level, Part, Refusal, Request, Status};

use evolite_api::SessionLimits as Policy;

/// Platform's credits in one duff and in one DASH.
pub const CREDITS_PER_DUFF: u64 = 1_000;
pub const CREDITS_PER_DASH: u64 = 100_000_000 * CREDITS_PER_DUFF;

/// The wallet's limits on a session key, in credits and milliseconds: [`Policy`]'s, the one
/// place they are set (the demo core and the dialog read them there too).
pub const MAX_BUDGET: u64 = Policy::MAX_BUDGET as u64 * CREDITS_PER_DUFF;
/// The budget of a binding that proposes none.
pub const DEFAULT_BUDGET: u64 = Policy::DEFAULT_BUDGET as u64 * CREDITS_PER_DUFF;
pub const BUDGET_PRESETS: [u64; 4] = [
    Policy::BUDGET_PRESETS[0] as u64 * CREDITS_PER_DUFF,
    Policy::BUDGET_PRESETS[1] as u64 * CREDITS_PER_DUFF,
    Policy::BUDGET_PRESETS[2] as u64 * CREDITS_PER_DUFF,
    Policy::BUDGET_PRESETS[3] as u64 * CREDITS_PER_DUFF,
];

const DAY_MS: u64 = 24 * 60 * 60 * 1_000;
/// The term of a binding that proposes none.
pub const DEFAULT_TERM_MS: u64 = Policy::DEFAULT_TERM_DAYS as u64 * DAY_MS;
pub const MAX_TERM_MS: u64 = Policy::MAX_TERM_DAYS as u64 * DAY_MS;
/// Apps do not take a key that ends within the hour (the DIP's `APP_CONNECT_EXPIRY_MARGIN`).
pub const MIN_TERM_MS: u64 = Policy::MIN_TERM_SECS as u64 * 1_000;
pub const TERM_PRESETS_DAYS: [u64; 3] = [
    Policy::TERM_PRESETS_DAYS[0] as u64,
    Policy::TERM_PRESETS_DAYS[1] as u64,
    Policy::TERM_PRESETS_DAYS[2] as u64,
];

/// DashPay's `profile`: the one document type of a contract the app does not own it may ask for.
const FOREIGN_DOCUMENT_TYPE: &str = "profile";

fn invalid<T>(part: Part) -> Result<T, Refusal> {
    Err(Refusal::Refused(Status::Invalid, part))
}

/// A session key's budget (credits) and end (block time, ms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub budget: u64,
    pub expires_at: u64,
    /// The wallet changed what the app proposed (the screen says so, R-4).
    pub changed: bool,
}

/// The limits the wallet offers for `binding` at `now_ms`: the app's proposal within the
/// wallet's caps, the defaults where it proposes nothing. A proposed end already passed is
/// refused (DIP, "Limits").
pub fn offered(binding: &AuthBinding, now_ms: u64) -> Result<Limits, Refusal> {
    if binding.expires_at != 0 && binding.expires_at <= now_ms {
        return invalid(Part::AuthBinding);
    }
    let budget = match binding.total_budget {
        0 => DEFAULT_BUDGET,
        b => b.min(MAX_BUDGET),
    };
    let expires_at = match binding.expires_at {
        0 => now_ms + DEFAULT_TERM_MS,
        e => e.clamp(now_ms + MIN_TERM_MS, now_ms + MAX_TERM_MS),
    };
    let changed = (binding.total_budget != 0 && budget != binding.total_budget)
        || (binding.expires_at != 0 && expires_at != binding.expires_at);
    Ok(Limits {
        budget,
        expires_at,
        changed,
    })
}

/// The limits the person chose in the dialog, if the wallet allows them: a budget of at least a
/// credit and at most [`MAX_BUDGET`], an end at least [`MIN_TERM_MS`] and at most
/// [`MAX_TERM_MS`] away. A limit is never dropped.
pub fn chosen(budget: u64, expires_at: u64, now_ms: u64) -> Option<Limits> {
    let budget_ok = (1..=MAX_BUDGET).contains(&budget);
    let term_ok = (now_ms + MIN_TERM_MS..=now_ms + MAX_TERM_MS).contains(&expires_at);
    (budget_ok && term_ok).then_some(Limits {
        budget,
        expires_at,
        changed: false,
    })
}

/// The limits of all the keys a sign-in grants, each as [`chosen`] takes it, and together at
/// most [`MAX_BUDGET`]: the connection's one spending limit is shared by its keys (round 4).
pub fn chosen_all(choices: &[(u64, u64)], now_ms: u64) -> Option<Vec<Limits>> {
    let limits = choices
        .iter()
        .map(|&(budget, end)| chosen(budget, end, now_ms))
        .collect::<Option<Vec<_>>>()?;
    let total = limits
        .iter()
        .try_fold(0u64, |sum, l| sum.checked_add(l.budget))?;
    (total <= MAX_BUDGET).then_some(limits)
}

/// How a contract keeps an identity's encryption keys for it (`requiresIdentity…BoundedKey`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRule {
    /// 0: one key forever, never rotated.
    Unique,
    /// 1: keeps no current key: not for an encryption binding.
    Multiple,
    /// 2: the latest key is the current one.
    Latest,
}

/// What Platform says about the contracts and groups a request names (all proved).
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// The owner of each contract found; a missing entry is a contract not found.
    pub contract_owner: HashMap<[u8; 32], [u8; 32]>,
    /// The owner of each contract group found.
    pub group_owner: HashMap<[u8; 32], [u8; 32]>,
    /// Each contract's contract-level rule for encryption and decryption keys; `None` when it
    /// sets none (a rule on a document type only counts as none).
    pub key_rule: HashMap<([u8; 32], EncPurpose), Option<KeyRule>>,
}

/// Whether `id` is one of Platform's system contracts.
pub fn is_system(id: &[u8; 32]) -> bool {
    use dpp::system_data_contracts::SystemDataContract;
    SystemDataContract::ALL
        .iter()
        .any(|c| c.id().to_buffer() == *id)
}

fn dashpay() -> [u8; 32] {
    dpp::system_data_contracts::SystemDataContract::Dashpay
        .id()
        .to_buffer()
}

/// The DIP's rules on what an app may request ("What an App May Request"), on what Platform
/// says. Checked before the request is shown.
pub fn check(request: &Request, facts: &Facts) -> Result<(), Refusal> {
    if is_system(&request.app_id) {
        return invalid(Part::App);
    }
    let Some(owner) = facts.contract_owner.get(&request.app_id) else {
        return invalid(Part::App);
    };
    for binding in &request.auth {
        let allowed = match &binding.bounds {
            Bounds::SingleContract(c) => facts.contract_owner.get(c) == Some(owner),
            Bounds::ContractGroup(g) => facts.group_owner.get(g) == Some(owner),
            Bounds::DocumentType { contract, name } => {
                facts.contract_owner.get(contract) == Some(owner)
                    || (*contract == dashpay() && name == FOREIGN_DOCUMENT_TYPE)
            }
        };
        if !allowed {
            return invalid(Part::AuthBinding);
        }
    }
    for binding in &request.enc {
        let rule = facts
            .key_rule
            .get(&(binding.contract, binding.purpose))
            .copied()
            .flatten();
        if !matches!(rule, Some(KeyRule::Unique | KeyRule::Latest)) {
            return invalid(Part::EncBinding);
        }
    }
    Ok(())
}

/// The bindings the person kept: the primary one always (declining it declines the request).
pub fn kept(request: &Request, selected: &[bool]) -> Option<Vec<usize>> {
    if selected.len() != request.auth.len() || !selected.first().copied().unwrap_or(false) {
        return None;
    }
    Some(
        selected
            .iter()
            .enumerate()
            .filter(|(_, on)| **on)
            .map(|(i, _)| i)
            .collect(),
    )
}

/// The compressed public key of a private one.
pub fn compressed(secret: &[u8; SLOT]) -> Result<[u8; 33], String> {
    let secret = SecretKey::from_secret_bytes(*secret).map_err(|e| e.to_string())?;
    let mut secp = secret;
    let public = PublicKey::from_secret_key(&secp).serialize();
    secp.non_secure_erase();
    Ok(public)
}

fn level(level: Level) -> SecurityLevel {
    match level {
        Level::Critical => SecurityLevel::CRITICAL,
        Level::High => SecurityLevel::HIGH,
    }
}

fn bounds(bounds: &Bounds) -> ContractBounds {
    match bounds {
        Bounds::SingleContract(c) => ContractBounds::SingleContract {
            id: Identifier::from(*c),
        },
        Bounds::DocumentType { contract, name } => ContractBounds::SingleContractDocumentType {
            id: Identifier::from(*contract),
            document_type_name: name.clone(),
        },
        Bounds::ContractGroup(g) => ContractBounds::ContractGroup {
            id: Identifier::from(*g),
        },
    }
}

/// A session key as the identity gets it: AUTHENTICATION at the binding's level, bound as the
/// binding says, `ECDSA_HASH160` (as DashPay and Yappr write it), always V1 with its limits.
pub fn session_key(
    id: u32,
    binding: &AuthBinding,
    limits: &Limits,
    secret: &[u8; SLOT],
) -> Result<IdentityPublicKey, String> {
    let data = hash160::Hash::hash(&compressed(secret)?).to_byte_array();
    Ok(IdentityPublicKey::V1(IdentityPublicKeyV1 {
        id,
        purpose: Purpose::AUTHENTICATION,
        security_level: level(binding.level),
        contract_bounds: Some(bounds(&binding.bounds)),
        key_type: KeyType::ECDSA_HASH160,
        read_only: false,
        data: BinaryData::new(data.to_vec()),
        disabled_at: None,
        total_budget: Some(limits.budget),
        expires_at: Some(limits.expires_at),
    }))
}

fn purpose(purpose: EncPurpose) -> Purpose {
    match purpose {
        EncPurpose::Encryption => Purpose::ENCRYPTION,
        EncPurpose::Decryption => Purpose::DECRYPTION,
    }
}

/// An encryption key as the identity gets it: MEDIUM, `ECDSA_SECP256K1`, bound to its contract,
/// no limits (it signs nothing, and Platform allows limits only on authentication keys).
pub fn encryption_key(
    id: u32,
    binding: &EncBinding,
    secret: &[u8; SLOT],
) -> Result<IdentityPublicKey, String> {
    Ok(IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id,
        purpose: purpose(binding.purpose),
        security_level: SecurityLevel::MEDIUM,
        contract_bounds: Some(ContractBounds::SingleContract {
            id: Identifier::from(binding.contract),
        }),
        key_type: KeyType::ECDSA_SECP256K1,
        read_only: false,
        data: BinaryData::new(compressed(secret)?.to_vec()),
        disabled_at: None,
    }))
}

/// What a login does for one encryption binding (DIP, "Encryption Keys").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncStep {
    /// No key for the contract and purpose on the identity: register rotation 0 and send it.
    Register,
    /// The identity's current key is ours at this rotation: send it, register nothing.
    Send(u32),
    /// Leave the slot out: a key is there that is not ours, or none of them is current.
    Omit,
}

/// The rotations tried for our own current key; the DIP stops at the first one not on the
/// identity, and a login never registers one next to a current key.
const MAX_ROTATIONS: u32 = 64;

/// The step for `binding` on `identity_keys`, given Platform's current key for the contract and
/// purpose (`current`, if enabled) and our public key at each rotation (`ours`).
pub fn enc_step(
    binding: &EncBinding,
    identity_keys: &[IdentityPublicKey],
    current: Option<&IdentityPublicKey>,
    mut ours: impl FnMut(u32) -> Option<[u8; 33]>,
) -> EncStep {
    let wanted = purpose(binding.purpose);
    let bound_here = |k: &IdentityPublicKey| {
        k.purpose() == wanted
            && matches!(
                k.contract_bounds(),
                Some(ContractBounds::SingleContract { id }) if id.to_buffer() == binding.contract
            )
    };
    if let Some(current) = current.filter(|k| k.disabled_at().is_none() && bound_here(k)) {
        for rotation in 0..MAX_ROTATIONS {
            let Some(data) = ours(rotation) else {
                break;
            };
            if current.data().as_slice() == data {
                return EncStep::Send(rotation);
            }
            if !identity_keys.iter().any(|k| k.data().as_slice() == data) {
                break;
            }
        }
        return EncStep::Omit;
    }
    if identity_keys.iter().any(bound_here) {
        EncStep::Omit
    } else {
        EncStep::Register
    }
}

/// The `loginKeyResponse` document's three values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// `hash160(appEphPub)`.
    pub app_key_hash: [u8; 20],
    pub wallet_ephemeral: [u8; 33],
    pub payload: Vec<u8>,
}

fn random<const N: usize>() -> Zeroizing<[u8; N]> {
    let mut bytes = Zeroizing::new([0u8; N]);
    getrandom::fill(bytes.as_mut()).expect("the OS random generator is available");
    bytes
}

/// The response to `app_ephemeral` for `owner`: the session keys of the granted auth bindings in
/// request order, then the sent encryption keys in request order (review r2 note 2). The wallet's
/// ephemeral key and the nonce are new for every response and the ephemeral key is forgotten
/// once sealed (review r2 note 1).
pub fn response(
    auth: &[Zeroizing<[u8; SLOT]>],
    enc: &[Zeroizing<[u8; SLOT]>],
    app_ephemeral: &[u8; 33],
    owner: &[u8; 32],
) -> Result<Response, String> {
    let mut ephemeral = loop {
        if let Ok(key) = SecretKey::from_secret_bytes(*random::<32>()) {
            break key;
        }
    };
    let slots: Vec<Zeroizing<[u8; SLOT]>> = auth.iter().chain(enc).cloned().collect();
    let sealed = keys::seal(&slots, &ephemeral, app_ephemeral, owner, *random::<12>());
    let wallet_ephemeral = PublicKey::from_secret_key(&ephemeral).serialize();
    ephemeral.non_secure_erase();
    Ok(Response {
        app_key_hash: hash160::Hash::hash(app_ephemeral).to_byte_array(),
        wallet_ephemeral,
        payload: sealed?,
    })
}

#[cfg(test)]
mod tests;
