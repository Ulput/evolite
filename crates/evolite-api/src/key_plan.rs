//! FEAT-0020: what one change of an identity's keys does, worked out before the password (АРХ-4,
//! АРХ-5, R-2, R-3, AC-T2, AC-T3). Pure: the core plans from the identity it just read, the
//! interface from the snapshot to offer only what can be done; both by the same rules.

use crate::{IdentityKey, KeyLevel, KeyOwner, KeyPurpose};

/// The DashPay data contract, the same on every network.
pub const DASHPAY_CONTRACT: &str = "Bwr4WHCPz5rFVAD87RqTs3izo4zpzwsEdKPWUT1NS1C7";
/// The document type DashPay's contract asks encryption keys to be bound to.
pub const DASHPAY_DOCUMENT: &str = "contactRequest";

/// At most this many keys added in one IdentityUpdate (protocol 14).
pub const MAX_ADD: usize = 6;
/// At most this many keys disabled in one IdentityUpdate.
pub const MAX_DISABLE: usize = 10;

/// A key the person asks for: the designer's ready purposes (K5), and the main key's
/// replacement (K4). Each new key comes from this wallet's phrase (R-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRequest {
    /// AUTHENTICATION CRITICAL: sending and managing tokens.
    Tokens,
    /// ENCRYPTION and DECRYPTION, MEDIUM, bound to DashPay's contact requests.
    DashPay,
    /// TRANSFER CRITICAL: credits to an address and into the pool.
    Transfers,
    /// A new AUTHENTICATION MASTER, the acting one disabled in the same update.
    ReplaceMaster,
    /// BUG-286: one DECRYPTION MEDIUM bound to DashPay's contact requests, for an identity that
    /// has none (one made by another wallet): other wallets (Dash Desktop Wallet) send contact
    /// requests only to a DECRYPTION key, and see no contacts without it. The ENCRYPTION key
    /// stays, so the identity's ECDH root does not change.
    DashPayDecryption,
    /// BUG-298: AUTHENTICATION HIGH, unbound: signing in to apps (App Connect), NFTs, and what
    /// other wallets take as «ready for DashPay».
    SignIn,
}

impl KeyRequest {
    /// How many keys it adds.
    pub fn keys(self) -> usize {
        match self {
            KeyRequest::DashPay => 2,
            _ => 1,
        }
    }

    /// How many of them are bound to a contract (DashPay's pair).
    pub fn bound_keys(self) -> usize {
        match self {
            KeyRequest::DashPay => 2,
            KeyRequest::DashPayDecryption => 1,
            _ => 0,
        }
    }
}

/// What a new key without a contract binding costs, roughly (BUG-183): testnet charged 17 428
/// and 17 810 duffs for one (2026-11-15), and a third on top, rounded up.
pub const KEY_ADD_UNBOUND: crate::Duffs = 24_000;
/// A new key bound to a contract: DashPay's pair cost 59 125 for its two keys (with the old pair
/// disabled), ≈ 29 563 a key, and a third on top, rounded up.
pub const KEY_ADD_BOUND: crate::Duffs = 40_000;

/// The fee of one update adding `add`, in duffs, roughly (АРХ-3: no proved bound): each new key
/// at its rate ([`fee_of_keys`]); one update's worth when only disabling.
pub fn fee(add: &[KeyRequest], fees: &crate::Fees) -> crate::Duffs {
    let bound: usize = add.iter().map(|r| r.bound_keys()).sum();
    let all: usize = add.iter().map(|r| r.keys()).sum();
    fee_of_keys(bound, all - bound, fees)
}

/// The fee of an update adding `bound` keys bound to a contract and `unbound` others (BUG-183):
/// App Connect's keys are bound to the app's contract. Only disabling: `Fees::add_key`.
pub fn fee_of_keys(bound: usize, unbound: usize, fees: &crate::Fees) -> crate::Duffs {
    if bound + unbound == 0 {
        return fees.add_key;
    }
    let of = |n: usize, rate: crate::Duffs| rate.saturating_mul(n as crate::Duffs);
    of(bound, fees.key_add_bound).saturating_add(of(unbound, fees.key_add_unbound))
}

/// What a new key is, by Platform's purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewPurpose {
    Authentication,
    Transfer,
    Encryption,
    Decryption,
}

/// One key to add: its number, purpose and level; `dashpay`: bound to DashPay's contact
/// requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewKey {
    pub id: u32,
    pub purpose: NewPurpose,
    pub level: KeyLevel,
    pub dashpay: bool,
}

/// One IdentityUpdate: the keys added, in number order, and those disabled.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct KeyPlan {
    pub add: Vec<NewKey>,
    pub disable: Vec<u32>,
}

/// Why a change cannot be made, said before the password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyRefusal {
    /// Nothing asked.
    Nothing,
    TooManyAdded,
    TooManyDisabled,
    /// No such key, or it is disabled already.
    NoSuchKey(u32),
    /// The main key is only replaced, one for one (R-2).
    MasterOnlyReplaced,
    /// The acting main key is another wallet's: only that wallet changes the keys.
    MasterNotThisWallets,
    /// DashPay's encryption keys go only as a pair, replaced (АРХ-2, R-3).
    DashPayOnlyReplaced,
    /// Other encryption keys are on: DashPay would take one of them instead of the new pair.
    OtherEncryptionKeys,
    /// The identity has such a key already.
    AlreadyHas(KeyRequest),
}

fn active(keys: &[IdentityKey]) -> impl Iterator<Item = &IdentityKey> {
    keys.iter().filter(|k| k.disabled_at.is_none())
}

fn is_master(k: &IdentityKey) -> bool {
    k.level == KeyLevel::Master
}

/// A key known to be DashPay's (AC-T19, the customer: «unbound» is not «DashPay»): an
/// ENCRYPTION or DECRYPTION key bound to DashPay (its contract or its `contactRequest`), or
/// this wallet's own unbound one — evolite's set or K7's pair. An unbound one of another
/// wallet is not.
pub fn is_dashpay(k: &IdentityKey) -> bool {
    k.purpose == KeyPurpose::Encryption
        && match k.bound_to.as_deref() {
            Some(contract) => contract == DASHPAY_CONTRACT,
            None => k.owner == KeyOwner::This,
        }
}

/// An acting ECDSA encryption or decryption key: one DashPay may pick, whoever's it is.
fn pickable(k: &IdentityKey) -> bool {
    k.disabled_at.is_none()
        && k.purpose == KeyPurpose::Encryption
        && k.kind == crate::KeyKind::Ecdsa
}

/// AC-T19: the keys DashPay takes, by platform-wallet's rule (575d961,
/// `contact_requests.rs`), whatever their bounds and owner — the first ENCRYPTION key by
/// number (this identity's ECDH root), and the one others encrypt to: the first DECRYPTION,
/// else the first ENCRYPTION.
pub fn dashpay_picks(keys: &[IdentityKey]) -> Vec<u32> {
    let mut pickable: Vec<&IdentityKey> = keys.iter().filter(|k| pickable(k)).collect();
    pickable.sort_by_key(|k| k.id);
    let root = pickable.iter().find(|k| !k.decrypts).map(|k| k.id);
    let to = pickable.iter().find(|k| k.decrypts).map(|k| k.id).or(root);
    let mut picks: Vec<u32> = root.into_iter().chain(to).collect();
    picks.dedup();
    picks
}

/// Whether DashPay works here: it has a key to send from, and every key it takes is this
/// wallet's.
pub fn dashpay_works(keys: &[IdentityKey]) -> bool {
    let picks = dashpay_picks(keys);
    let root = keys
        .iter()
        .any(|k| pickable(k) && !k.decrypts && picks.contains(&k.id));
    root && picks.iter().all(|id| {
        keys.iter()
            .any(|k| k.id == *id && k.owner == KeyOwner::This)
    })
}

/// AC-T19: the acting encryption and decryption keys that are not this wallet's — an app's,
/// another wallet's. DashPay would take one of them before a new pair (that gets max+1), so
/// «Enable DashPay» disables them in the same update, once the person agrees.
pub fn foreign_encryption(keys: &[IdentityKey]) -> Vec<u32> {
    let mut ids: Vec<u32> = keys
        .iter()
        .filter(|k| pickable(k) && k.owner != KeyOwner::This)
        .map(|k| k.id)
        .collect();
    ids.sort_unstable();
    ids
}

/// Whether the identity already has what `request` adds, acting. DashPay: it works here.
pub fn has(keys: &[IdentityKey], request: KeyRequest) -> bool {
    match request {
        KeyRequest::Tokens => active(keys)
            .any(|k| k.purpose == KeyPurpose::Documents && k.level == KeyLevel::Critical),
        KeyRequest::Transfers => active(keys).any(|k| k.purpose == KeyPurpose::Transfers),
        KeyRequest::DashPay => dashpay_works(keys),
        KeyRequest::ReplaceMaster => false,
        // BUG-286: any acting DECRYPTION: the one others encrypt to.
        KeyRequest::DashPayDecryption => keys.iter().any(|k| pickable(k) && k.decrypts),
        KeyRequest::SignIn => signs_in(keys, &[]),
    }
}

/// BUG-298: an acting AUTHENTICATION HIGH of this wallet, unbound, `leaving` aside (disabled in
/// the same update). App Connect and NFTs sign with exactly HIGH and no bound key: the same rule
/// as the chain's `SignInIdentity::can_sign_in` (r1 Should 3).
fn signs_in(keys: &[IdentityKey], leaving: &[u32]) -> bool {
    active(keys).any(|k| {
        k.purpose == KeyPurpose::Documents
            && k.level == KeyLevel::High
            && k.owner == KeyOwner::This
            && k.bound_to.is_none()
            && !leaving.contains(&k.id)
    })
}

/// BUG-286, BUG-298: what the identity lacks to be ready for other wallets (Dash Desktop Wallet
/// shows «Needs keys» otherwise): a sign-in key (AUTH HIGH) of this wallet, and a DECRYPTION
/// key. One update adds them all («Add the missing keys»). Empty: ready.
pub fn missing_for_other_wallets(keys: &[IdentityKey]) -> Vec<KeyRequest> {
    [KeyRequest::SignIn, KeyRequest::DashPayDecryption]
        .into_iter()
        .filter(|request| !has(keys, *request))
        .collect()
}

/// FEAT-0057 П1 (R-1.2): what `keys` let this wallet do, by the same rule as
/// [`missing_for_other_wallets`] — `None` while whose a key is is not known (no second parse of
/// the keys: «not known» is not «missing», BUG-243).
pub fn ready(keys: &[IdentityKey]) -> Option<crate::IdentityReady> {
    if keys.iter().any(|k| k.owner == KeyOwner::Unknown) {
        return None;
    }
    let ours = |k: &&IdentityKey| k.owner == KeyOwner::This;
    let missing = missing_for_other_wallets(keys);
    Some(crate::IdentityReady {
        ours: active(keys).filter(ours).any(|k| {
            matches!(k.purpose, KeyPurpose::Identity | KeyPurpose::Documents)
                || k.level == KeyLevel::Master
        }),
        sign_in: !missing.contains(&KeyRequest::SignIn),
        dashpay: !missing.contains(&KeyRequest::DashPayDecryption),
        master_ours: active(keys).filter(ours).any(is_master),
    })
}

/// BUG-298: whether `plan` disables the last acting sign-in key of this wallet and adds none:
/// said before the password, with «Replace with a new one» (not refused: the person decides).
pub fn disables_last_sign_in(keys: &[IdentityKey], plan: &KeyPlan) -> bool {
    signs_in(keys, &[])
        && !signs_in(keys, &plan.disable)
        && !plan
            .add
            .iter()
            .any(|k| k.purpose == NewPurpose::Authentication && k.level == KeyLevel::High)
}

/// The keys of one update, numbered from max+1 over every key, the disabled ones too (AC-T3),
/// or why it cannot be made (AC-T2).
pub fn plan(
    keys: &[IdentityKey],
    add: &[KeyRequest],
    disable: &[u32],
) -> Result<KeyPlan, KeyRefusal> {
    if add.is_empty() && disable.is_empty() {
        return Err(KeyRefusal::Nothing);
    }
    let mut off: Vec<u32> = Vec::new();
    for &id in disable {
        let key = active(keys)
            .find(|k| k.id == id)
            .ok_or(KeyRefusal::NoSuchKey(id))?;
        if is_master(key) {
            return Err(KeyRefusal::MasterOnlyReplaced);
        }
        if is_dashpay(key) && !add.contains(&KeyRequest::DashPay) {
            return Err(KeyRefusal::DashPayOnlyReplaced);
        }
        if !off.contains(&id) {
            off.push(id);
        }
    }
    let mut next = keys.iter().map(|k| k.id + 1).max().unwrap_or(0);
    let mut new = Vec::new();
    let mut take = |purpose, level, dashpay| {
        new.push(NewKey {
            id: next,
            purpose,
            level,
            dashpay,
        });
        next += 1;
    };
    let mut asked: Vec<KeyRequest> = Vec::new();
    for &request in add {
        if asked.contains(&request) {
            continue;
        }
        asked.push(request);
        match request {
            KeyRequest::ReplaceMaster => {
                let master = active(keys)
                    .find(|k| is_master(k))
                    .ok_or(KeyRefusal::MasterNotThisWallets)?;
                if master.owner != KeyOwner::This {
                    return Err(KeyRefusal::MasterNotThisWallets);
                }
                off.push(master.id);
                take(NewPurpose::Authentication, KeyLevel::Master, false);
            }
            KeyRequest::DashPay => {
                // A known pair is replaced whole: its keys go in this update.
                for k in active(keys).filter(|k| is_dashpay(k)) {
                    if !off.contains(&k.id) {
                        off.push(k.id);
                    }
                }
                // AC-T19: another's encryption keys would be taken before the new pair; they go
                // too, but only named in `disable` — the person agreed to it (R-4).
                if foreign_encryption(keys).iter().any(|id| !off.contains(id)) {
                    return Err(KeyRefusal::OtherEncryptionKeys);
                }
                take(NewPurpose::Encryption, KeyLevel::Medium, true);
                take(NewPurpose::Decryption, KeyLevel::Medium, true);
            }
            KeyRequest::DashPayDecryption => {
                if has(keys, request) {
                    return Err(KeyRefusal::AlreadyHas(request));
                }
                // The key DashPay encrypts from must be this wallet's: else it is another
                // wallet's DashPay, which that wallet changes (as for the pair, AC-T19).
                let root = dashpay_picks(keys).first().copied();
                let ours = root
                    .is_some_and(|id| keys.iter().any(|k| k.id == id && k.owner == KeyOwner::This));
                if !ours {
                    return Err(KeyRefusal::OtherEncryptionKeys);
                }
                take(NewPurpose::Decryption, KeyLevel::Medium, true);
            }
            KeyRequest::SignIn => {
                // Not already one that stays: a replacement of the last one is asked with it
                // disabled in the same update («Replace with a new one»).
                if signs_in(keys, &off) {
                    return Err(KeyRefusal::AlreadyHas(request));
                }
                take(NewPurpose::Authentication, KeyLevel::High, false);
            }
            KeyRequest::Tokens | KeyRequest::Transfers => {
                if has(keys, request) {
                    return Err(KeyRefusal::AlreadyHas(request));
                }
                if request == KeyRequest::Tokens {
                    take(NewPurpose::Authentication, KeyLevel::Critical, false);
                } else {
                    take(NewPurpose::Transfer, KeyLevel::Critical, false);
                }
            }
        }
    }
    // The main key never goes without its replacement, nor is it the only one changed twice.
    if off
        .iter()
        .any(|id| active(keys).any(|k| k.id == *id && is_master(k)))
        && !asked.contains(&KeyRequest::ReplaceMaster)
    {
        return Err(KeyRefusal::MasterOnlyReplaced);
    }
    if new.len() > MAX_ADD {
        return Err(KeyRefusal::TooManyAdded);
    }
    if off.len() > MAX_DISABLE {
        return Err(KeyRefusal::TooManyDisabled);
    }
    Ok(KeyPlan {
        add: new,
        disable: off,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeyKind;

    fn key(id: u32, level: KeyLevel, purpose: KeyPurpose, owner: KeyOwner) -> IdentityKey {
        IdentityKey {
            id,
            level,
            purpose,
            kind: KeyKind::Ecdsa,
            decrypts: false,
            disabled_at: None,
            bound_to: None,
            owner,
        }
    }

    /// evolite's own six keys (АРХ-2).
    fn evolite() -> Vec<IdentityKey> {
        let mut decrypt = key(5, KeyLevel::Medium, KeyPurpose::Encryption, KeyOwner::This);
        decrypt.decrypts = true;
        vec![
            key(0, KeyLevel::Master, KeyPurpose::Identity, KeyOwner::This),
            key(1, KeyLevel::High, KeyPurpose::Documents, KeyOwner::This),
            key(2, KeyLevel::Critical, KeyPurpose::Documents, KeyOwner::This),
            key(3, KeyLevel::Critical, KeyPurpose::Transfers, KeyOwner::This),
            key(4, KeyLevel::Medium, KeyPurpose::Encryption, KeyOwner::This),
            decrypt,
        ]
    }

    /// @ozzieq31's keys (BUG-286): made by another wallet, the phrase's all the same: MASTER,
    /// HIGH, CRITICAL, TRANSFER and one ENCRYPTION bound to DashPay; no DECRYPTION.
    fn imported() -> Vec<IdentityKey> {
        let mut keys = evolite();
        keys.retain(|k| !k.decrypts);
        keys[4].bound_to = Some(DASHPAY_CONTRACT.into());
        keys
    }

    /// BUG-286 (а): no DECRYPTION → one bound DECRYPTION, max+1, the ENCRYPTION kept; one there →
    /// «already has»; the root another wallet's → refused, as the pair.
    #[test]
    fn a_missing_decryption_key_is_one_bound_key() {
        let keys = imported();
        assert!(!has(&keys, KeyRequest::DashPayDecryption));
        let plan = plan(&keys, &[KeyRequest::DashPayDecryption], &[]).unwrap();
        assert_eq!(
            plan.add,
            vec![NewKey {
                id: 5,
                purpose: NewPurpose::Decryption,
                level: KeyLevel::Medium,
                dashpay: true,
            }]
        );
        assert!(plan.disable.is_empty(), "the ENCRYPTION stays");
        let request = KeyRequest::DashPayDecryption;
        assert_eq!(
            (request.keys(), request.bound_keys()),
            (1, 1),
            "one key, bound"
        );

        assert_eq!(
            super::plan(&evolite(), &[KeyRequest::DashPayDecryption], &[]),
            Err(KeyRefusal::AlreadyHas(KeyRequest::DashPayDecryption))
        );
        let mut foreign = imported();
        foreign[4].owner = KeyOwner::Other;
        assert_eq!(
            super::plan(&foreign, &[KeyRequest::DashPayDecryption], &[]),
            Err(KeyRefusal::OtherEncryptionKeys)
        );
    }

    /// BUG-298: the sign-in key: added when none of this wallet acts; «already has» otherwise,
    /// unless the one there goes in the same update (the replacement).
    #[test]
    fn a_sign_in_key_is_added_or_replaces_the_last() {
        let mut keys = evolite();
        assert_eq!(
            plan(&keys, &[KeyRequest::SignIn], &[]),
            Err(KeyRefusal::AlreadyHas(KeyRequest::SignIn))
        );
        let replace = plan(&keys, &[KeyRequest::SignIn], &[1]).unwrap();
        assert_eq!(replace.disable, vec![1]);
        assert_eq!(
            replace.add,
            vec![NewKey {
                id: 6,
                purpose: NewPurpose::Authentication,
                level: KeyLevel::High,
                dashpay: false,
            }]
        );
        keys[1].disabled_at = Some(1);
        assert!(!has(&keys, KeyRequest::SignIn));
        assert_eq!(
            plan(&keys, &[KeyRequest::SignIn], &[]).unwrap().add.len(),
            1
        );
        assert_eq!(
            (KeyRequest::SignIn.keys(), KeyRequest::SignIn.bound_keys()),
            (1, 0)
        );
    }

    /// BUG-298 r1 Should 3: a HIGH bound to a contract (an import may bring one) is no sign-in
    /// key, as App Connect's `can_sign_in` says: missing, and added. Mutant: no `bound_to`
    /// check → «already has».
    #[test]
    fn a_bound_high_is_no_sign_in_key() {
        let mut keys = evolite();
        keys[1].bound_to = Some(DASHPAY_CONTRACT.into());
        assert!(!has(&keys, KeyRequest::SignIn));
        assert_eq!(missing_for_other_wallets(&keys), vec![KeyRequest::SignIn]);
        assert_eq!(
            plan(&keys, &[KeyRequest::SignIn], &[]).unwrap().add.len(),
            1
        );
    }

    /// FEAT-0057 П1 (R-1.2): the picker's state by the same rule as BUG-286/298 — evolite's own
    /// six ready; an import without DECRYPTION needs DashPay; without the sign-in key needs it;
    /// another wallet's keys only — none ours; whose a key is not known — not known.
    #[test]
    fn readiness_follows_what_other_wallets_need() {
        let full = ready(&evolite()).expect("owners known");
        assert!(full.ours && full.sign_in && full.dashpay && full.master_ours);
        let imported = ready(&imported()).expect("owners known");
        assert!(imported.sign_in && !imported.dashpay);
        let mut keys = evolite();
        keys[1].disabled_at = Some(1);
        assert!(!ready(&keys).expect("known").sign_in);
        let others: Vec<IdentityKey> = evolite()
            .into_iter()
            .map(|mut k| {
                k.owner = KeyOwner::Other;
                k
            })
            .collect();
        let others = ready(&others).expect("known");
        assert!(!others.ours && !others.sign_in && !others.master_ours);
        let mut unknown = evolite();
        unknown[2].owner = KeyOwner::Unknown;
        assert_eq!(ready(&unknown), None, "not known is not missing");
    }

    /// BUG-286, BUG-298: what other wallets need, in one update.
    #[test]
    fn what_other_wallets_need_is_added_in_one_update() {
        assert!(
            missing_for_other_wallets(&evolite()).is_empty(),
            "evolite's own six"
        );
        let mut keys = imported();
        assert_eq!(
            missing_for_other_wallets(&keys),
            vec![KeyRequest::DashPayDecryption]
        );
        keys[1].disabled_at = Some(1);
        let missing = missing_for_other_wallets(&keys);
        assert_eq!(
            missing,
            vec![KeyRequest::SignIn, KeyRequest::DashPayDecryption]
        );
        let plan = plan(&keys, &missing, &[]).unwrap();
        assert_eq!(
            plan.add.iter().map(|k| k.id).collect::<Vec<_>>(),
            vec![5, 6]
        );
        let bound: usize = missing.iter().map(|r| r.bound_keys()).sum();
        assert_eq!(bound, 1, "the HIGH unbound, the DECRYPTION bound");
    }

    /// BUG-298: disabling the last sign-in key is said before the password; another one acting,
    /// or a new one in the same update, is not.
    #[test]
    fn disabling_the_last_sign_in_key_is_said() {
        let keys = evolite();
        let off = plan(&keys, &[], &[1]).unwrap();
        assert!(disables_last_sign_in(&keys, &off));
        let replaced = plan(&keys, &[KeyRequest::SignIn], &[1]).unwrap();
        assert!(!disables_last_sign_in(&keys, &replaced));
        let mut two = keys.clone();
        two.push(key(
            6,
            KeyLevel::High,
            KeyPurpose::Documents,
            KeyOwner::This,
        ));
        assert!(!disables_last_sign_in(
            &two,
            &plan(&two, &[], &[1]).unwrap()
        ));
        assert!(!disables_last_sign_in(
            &keys,
            &plan(&keys, &[], &[2]).unwrap()
        ));
    }

    /// AC-T3: a new key is max+1 over every key, the disabled ones too.
    #[test]
    fn a_new_key_takes_the_number_after_the_highest() {
        let mut keys = evolite();
        keys.retain(|k| k.id != 2);
        let mut gone = key(9, KeyLevel::High, KeyPurpose::Documents, KeyOwner::This);
        gone.disabled_at = Some(1);
        keys.push(gone);
        let plan = plan(&keys, &[KeyRequest::Tokens], &[]).unwrap();
        assert_eq!(
            plan.add,
            vec![NewKey {
                id: 10,
                purpose: NewPurpose::Authentication,
                level: KeyLevel::Critical,
                dashpay: false
            }]
        );
        assert!(plan.disable.is_empty());
    }

    /// R-2, AC-T2: the main key is only replaced one for one, and only when it is this
    /// wallet's.
    #[test]
    fn the_main_key_is_only_replaced() {
        let keys = evolite();
        assert_eq!(plan(&keys, &[], &[0]), Err(KeyRefusal::MasterOnlyReplaced));
        let replaced = plan(&keys, &[KeyRequest::ReplaceMaster], &[]).unwrap();
        assert_eq!(replaced.disable, vec![0]);
        assert_eq!(replaced.add[0].id, 6);
        assert_eq!(replaced.add[0].level, KeyLevel::Master);
        let mut other = evolite();
        other[0].owner = KeyOwner::Other;
        assert_eq!(
            plan(&other, &[KeyRequest::ReplaceMaster], &[]),
            Err(KeyRefusal::MasterNotThisWallets)
        );
    }

    /// АРХ-2, R-3, AC-T2: DashPay's keys go only as a pair replaced, and not while another
    /// encryption key would be taken instead.
    #[test]
    fn dashpays_keys_go_only_as_a_replaced_pair() {
        let keys = evolite();
        assert_eq!(plan(&keys, &[], &[4]), Err(KeyRefusal::DashPayOnlyReplaced));
        let pair = plan(&keys, &[KeyRequest::DashPay], &[4, 5]).unwrap();
        assert_eq!(pair.disable, vec![4, 5]);
        assert_eq!(
            pair.add
                .iter()
                .map(|k| (k.id, k.purpose, k.dashpay))
                .collect::<Vec<_>>(),
            vec![
                (6, NewPurpose::Encryption, true),
                (7, NewPurpose::Decryption, true)
            ]
        );
        // Asked without naming the old ones, the pair is still replaced whole.
        assert_eq!(
            plan(&keys, &[KeyRequest::DashPay], &[]).unwrap().disable,
            vec![4, 5]
        );
        let mut with_app = evolite();
        with_app.push(key(
            6,
            KeyLevel::Medium,
            KeyPurpose::Encryption,
            KeyOwner::App("Jembe".into()),
        ));
        assert_eq!(
            plan(&with_app, &[KeyRequest::DashPay], &[]),
            Err(KeyRefusal::OtherEncryptionKeys)
        );
    }

    /// K7: «Enable DashPay» on an identity without encryption keys adds the bound pair.
    #[test]
    fn dashpay_is_enabled_where_there_are_no_encryption_keys() {
        let mut ios = evolite();
        ios.retain(|k| k.purpose != KeyPurpose::Encryption);
        assert!(!has(&ios, KeyRequest::DashPay));
        let pair = plan(&ios, &[KeyRequest::DashPay], &[]).unwrap();
        assert!(pair.disable.is_empty());
        assert_eq!(pair.add.len(), 2);
        assert!(
            pair.add
                .iter()
                .all(|k| k.dashpay && k.level == KeyLevel::Medium)
        );
    }

    /// AC-T2: what the identity has already, an unknown or disabled key, nothing asked, too
    /// many keys — each refused.
    #[test]
    fn refused_before_the_password() {
        let keys = evolite();
        assert_eq!(
            plan(&keys, &[KeyRequest::Tokens], &[]),
            Err(KeyRefusal::AlreadyHas(KeyRequest::Tokens))
        );
        assert_eq!(
            plan(&keys, &[KeyRequest::Transfers], &[]),
            Err(KeyRefusal::AlreadyHas(KeyRequest::Transfers))
        );
        assert_eq!(plan(&keys, &[], &[42]), Err(KeyRefusal::NoSuchKey(42)));
        assert_eq!(plan(&keys, &[], &[]), Err(KeyRefusal::Nothing));
        let mut gone = evolite();
        gone[1].disabled_at = Some(1);
        assert_eq!(plan(&gone, &[], &[1]), Err(KeyRefusal::NoSuchKey(1)));
        // Eleven to disable.
        let mut many = evolite();
        for id in 6..17 {
            many.push(key(
                id,
                KeyLevel::High,
                KeyPurpose::Documents,
                KeyOwner::Other,
            ));
        }
        let ids: Vec<u32> = (6..17).collect();
        assert_eq!(plan(&many, &[], &ids), Err(KeyRefusal::TooManyDisabled));
        // An app's key may go alone (R-8).
        assert_eq!(plan(&many, &[], &[6]).unwrap().disable, vec![6]);
    }

    /// AC-T19: an unbound encryption key of another wallet is not DashPay's — it is disabled
    /// alone; one bound to DashPay and this wallet's own are; the picks follow platform-wallet's
    /// rule by number, whoever's the keys are; with another's key first, «Enable DashPay»
    /// disables it in the same update only when it is named (the person agreed).
    #[test]
    fn dashpay_is_what_is_known_and_what_is_picked() {
        let mut keys = evolite();
        assert!(
            is_dashpay(&keys[4]) && is_dashpay(&keys[5]),
            "this wallet's own"
        );
        assert_eq!(dashpay_picks(&keys), vec![4, 5]);
        assert!(dashpay_works(&keys));
        // Another wallet's unbound encryption key, numbered before ours would be.
        keys.retain(|k| k.id != 4 && k.id != 5);
        let mut theirs = key(4, KeyLevel::Medium, KeyPurpose::Encryption, KeyOwner::Other);
        assert!(!is_dashpay(&theirs));
        theirs.bound_to = Some(DASHPAY_CONTRACT.into());
        assert!(is_dashpay(&theirs), "bound to DashPay: known");
        theirs.bound_to = None;
        keys.push(theirs);
        assert_eq!(plan(&keys, &[], &[4]).unwrap().disable, vec![4], "alone");
        assert_eq!(dashpay_picks(&keys), vec![4]);
        assert!(!dashpay_works(&keys), "DashPay would take another's key");
        assert!(!has(&keys, KeyRequest::DashPay));
        assert_eq!(foreign_encryption(&keys), vec![4]);
        assert_eq!(
            plan(&keys, &[KeyRequest::DashPay], &[]),
            Err(KeyRefusal::OtherEncryptionKeys),
            "not without the person's yes"
        );
        let k7 = plan(&keys, &[KeyRequest::DashPay], &[4]).unwrap();
        assert_eq!(k7.disable, vec![4]);
        assert_eq!(k7.add.iter().map(|k| k.id).collect::<Vec<_>>(), vec![5, 6]);
    }
}
