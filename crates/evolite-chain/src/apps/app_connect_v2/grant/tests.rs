use super::*;
use dpp::identity::identity_public_key::accessors::v1::IdentityPublicKeyGettersV1;

const NOW: u64 = 1_800_000_000_000;
const OWNER: [u8; 32] = [0x11; 32];
const APP: [u8; 32] = [0xa1; 32];
const GROUP: [u8; 32] = [0x6a; 32];
const OTHER: [u8; 32] = [0x99; 32];

fn binding(bounds: Bounds, total_budget: u64, expires_at: u64) -> AuthBinding {
    AuthBinding {
        bounds,
        level: Level::High,
        total_budget,
        expires_at,
    }
}

fn request(auth: Vec<AuthBinding>, enc: Vec<EncBinding>) -> Request {
    Request {
        app_ephemeral: [0x02; 33],
        app_id: APP,
        label: "Yappr".into(),
        identity_hint: None,
        auth,
        connection_id: None,
        enc,
        renew: false,
        created_at: NOW,
    }
}

/// The app's contract and group owned by `OWNER`, another contract by someone else.
fn facts() -> Facts {
    let mut facts = Facts::default();
    facts.contract_owner.insert(APP, OWNER);
    facts.contract_owner.insert(OTHER, [0x22; 32]);
    facts.contract_owner.insert(dashpay(), [0x33; 32]);
    facts.group_owner.insert(GROUP, OWNER);
    facts
}

fn refused(part: Part) -> Result<(), Refusal> {
    Err(Refusal::Refused(Status::Invalid, part))
}

/// R-1/R-4: no proposal gets the defaults; a proposal above the caps is brought down and said so;
/// an end within the hour is moved to an hour; one already passed is refused.
#[test]
fn the_wallet_offers_the_apps_limits_within_its_own() {
    let none = offered(&binding(Bounds::SingleContract(APP), 0, 0), NOW).unwrap();
    assert_eq!(
        (none.budget, none.expires_at, none.changed),
        (DEFAULT_BUDGET, NOW + DEFAULT_TERM_MS, false)
    );
    let yappr = binding(
        Bounds::ContractGroup(GROUP),
        CREDITS_PER_DASH / 2,
        NOW + DEFAULT_TERM_MS,
    );
    let asked = offered(&yappr, NOW).unwrap();
    assert_eq!(
        (asked.budget, asked.expires_at, asked.changed),
        (CREDITS_PER_DASH / 2, NOW + DEFAULT_TERM_MS, false)
    );
    let greedy = binding(
        Bounds::ContractGroup(GROUP),
        50 * CREDITS_PER_DASH,
        NOW + 10 * MAX_TERM_MS,
    );
    let capped = offered(&greedy, NOW).unwrap();
    assert_eq!(
        (capped.budget, capped.expires_at, capped.changed),
        (MAX_BUDGET, NOW + MAX_TERM_MS, true)
    );
    let short = offered(&binding(Bounds::SingleContract(APP), 1, NOW + 1_000), NOW).unwrap();
    assert_eq!((short.expires_at, short.changed), (NOW + MIN_TERM_MS, true));
    assert_eq!(
        offered(&binding(Bounds::SingleContract(APP), 1, NOW), NOW),
        Err(Refusal::Refused(Status::Invalid, Part::AuthBinding)),
        "an end already passed"
    );
}

/// The dialog's choice is taken only inside the wallet's limits, and never without a limit.
#[test]
fn a_chosen_limit_stays_inside_the_wallets() {
    assert!(chosen(CREDITS_PER_DASH / 10, NOW + 30 * DAY_MS, NOW).is_some());
    assert!(chosen(MAX_BUDGET, NOW + MAX_TERM_MS, NOW).is_some());
    assert!(chosen(0, NOW + 30 * DAY_MS, NOW).is_none(), "no budget");
    assert!(chosen(MAX_BUDGET + 1, NOW + 30 * DAY_MS, NOW).is_none());
    assert!(
        chosen(1, NOW + MIN_TERM_MS - 1, NOW).is_none(),
        "within the hour"
    );
    assert!(
        chosen(1, NOW + MAX_TERM_MS + 1, NOW).is_none(),
        "beyond a year"
    );
    assert_eq!(BUDGET_PRESETS[3], MAX_BUDGET);
    assert_eq!(TERM_PRESETS_DAYS[2] * DAY_MS, MAX_TERM_MS);
}

/// Round 4: the keys of one sign-in share its limit, so together they take at most 5 DASH.
#[test]
fn the_keys_of_a_sign_in_together_stay_within_the_cap() {
    let end = NOW + 30 * DAY_MS;
    let half = MAX_BUDGET / 2;
    assert_eq!(
        chosen_all(&[(half, end), (half, end)], NOW).map(|l| l.len()),
        Some(2)
    );
    assert_eq!(
        chosen_all(&[(half, end), (half + 1, end)], NOW),
        None,
        "over 5 DASH"
    );
    assert_eq!(
        chosen_all(&[(half, end), (0, end)], NOW),
        None,
        "a key without a budget"
    );
    assert_eq!(
        chosen_all(&[(u64::MAX, end), (1, end)], NOW),
        None,
        "no overflow"
    );
}

/// analyst-72's recommendation, as the wallet's constants say it in credits and milliseconds.
#[test]
fn the_wallets_policy_is_the_recommended_one() {
    assert_eq!(MAX_BUDGET, 5 * CREDITS_PER_DASH);
    assert_eq!(
        DEFAULT_BUDGET,
        5 * CREDITS_PER_DASH,
        "the customer, 2026-11-11"
    );
    assert_eq!(
        BUDGET_PRESETS,
        [
            CREDITS_PER_DASH / 10,
            CREDITS_PER_DASH / 2,
            2 * CREDITS_PER_DASH,
            5 * CREDITS_PER_DASH
        ]
    );
    assert_eq!(DEFAULT_TERM_MS, 90 * DAY_MS);
    assert_eq!(MAX_TERM_MS, 365 * DAY_MS);
    assert_eq!(MIN_TERM_MS, 60 * 60 * 1_000);
    assert_eq!(TERM_PRESETS_DAYS, [30, 90, 365]);
}

/// R-5, each rule of "What an App May Request" on its own.
#[test]
fn only_bindings_the_dip_allows_pass() {
    let facts = facts();
    let ok = request(
        vec![
            binding(Bounds::ContractGroup(GROUP), 1, 0),
            binding(
                Bounds::DocumentType {
                    contract: dashpay(),
                    name: "profile".into(),
                },
                1,
                0,
            ),
            binding(Bounds::SingleContract(APP), 1, 0),
            binding(
                Bounds::DocumentType {
                    contract: APP,
                    name: "post".into(),
                },
                1,
                0,
            ),
        ],
        vec![],
    );
    assert_eq!(check(&ok, &facts), Ok(()));

    let mut system = ok.clone();
    system.app_id = dashpay();
    assert_eq!(
        check(&system, &facts),
        refused(Part::App),
        "a system contract"
    );
    let mut missing = ok.clone();
    missing.app_id = [0x77; 32];
    assert_eq!(
        check(&missing, &facts),
        refused(Part::App),
        "no such contract"
    );

    let one = |bounds| request(vec![binding(bounds, 1, 0)], vec![]);
    for (bounds, why) in [
        (Bounds::SingleContract(OTHER), "someone else's contract"),
        (Bounds::ContractGroup([0x6b; 32]), "a group not found"),
        (
            Bounds::DocumentType {
                contract: dashpay(),
                name: "contactRequest".into(),
            },
            "DashPay contactRequest",
        ),
        (
            Bounds::DocumentType {
                contract: dpp::system_data_contracts::SystemDataContract::DPNS
                    .id()
                    .to_buffer(),
                name: "domain".into(),
            },
            "a DPNS document type",
        ),
        (
            Bounds::DocumentType {
                contract: OTHER,
                name: "profile".into(),
            },
            "another contract's profile",
        ),
    ] {
        assert_eq!(
            check(&one(bounds), &facts),
            refused(Part::AuthBinding),
            "{why}"
        );
    }
    let mut other_group = facts.clone();
    other_group.group_owner.insert(GROUP, [0x22; 32]);
    assert_eq!(
        check(&one(Bounds::ContractGroup(GROUP)), &other_group),
        refused(Part::AuthBinding),
        "a group of another owner"
    );
}

/// An encryption binding needs a contract-level rule that keeps a current key: 0 or 2.
#[test]
fn an_encryption_binding_needs_a_contract_that_keeps_a_current_key() {
    let enc = |purpose| EncBinding {
        contract: OTHER,
        purpose,
    };
    let primary = vec![binding(Bounds::SingleContract(APP), 1, 0)];
    for (rule, ok) in [
        (Some(KeyRule::Unique), true),
        (Some(KeyRule::Latest), true),
        (Some(KeyRule::Multiple), false),
        (None, false),
    ] {
        let mut facts = facts();
        facts.key_rule.insert((OTHER, EncPurpose::Encryption), rule);
        let asked = request(primary.clone(), vec![enc(EncPurpose::Encryption)]);
        assert_eq!(
            check(&asked, &facts).is_ok(),
            ok,
            "{rule:?}: a contract not owned by the app's owner may be named"
        );
        let other_purpose = request(primary.clone(), vec![enc(EncPurpose::Decryption)]);
        assert_eq!(
            check(&other_purpose, &facts),
            refused(Part::EncBinding),
            "the rule is per purpose"
        );
    }
}

/// Declining the primary binding declines the request; the others may be left out.
#[test]
fn the_primary_binding_cannot_be_left_out() {
    let three = request(
        vec![
            binding(Bounds::ContractGroup(GROUP), 1, 0),
            binding(Bounds::SingleContract(APP), 1, 0),
            binding(Bounds::SingleContract(APP), 1, 0),
        ],
        vec![],
    );
    assert_eq!(kept(&three, &[true, false, true]), Some(vec![0, 2]));
    assert_eq!(kept(&three, &[false, true, true]), None);
    assert_eq!(kept(&three, &[true, true]), None, "one answer per binding");
}

/// The keys the identity gets: a V1 session key bound and limited as granted, a V0 encryption
/// key bound to its contract.
#[test]
fn keys_are_built_as_the_dip_says() {
    let secret = Zeroizing::new([0x42u8; SLOT]);
    let public = compressed(&secret).unwrap();
    let limits = Limits {
        budget: 7,
        expires_at: NOW + DAY_MS,
        changed: false,
    };
    let mut critical = binding(Bounds::ContractGroup(GROUP), 7, 0);
    critical.level = Level::Critical;
    let key = session_key(9, &critical, &limits, &secret).unwrap();
    assert!(matches!(key, IdentityPublicKey::V1(_)));
    assert_eq!(key.id(), 9);
    assert_eq!(key.purpose(), Purpose::AUTHENTICATION);
    assert_eq!(key.security_level(), SecurityLevel::CRITICAL);
    assert_eq!(key.key_type(), KeyType::ECDSA_HASH160);
    assert_eq!(
        key.data().as_slice(),
        hash160::Hash::hash(&public).to_byte_array()
    );
    assert_eq!(
        key.contract_bounds(),
        Some(&ContractBounds::ContractGroup {
            id: Identifier::from(GROUP)
        })
    );
    assert_eq!(
        (key.total_budget(), key.expires_at()),
        (Some(7), Some(NOW + DAY_MS))
    );

    let enc = EncBinding {
        contract: OTHER,
        purpose: EncPurpose::Decryption,
    };
    let key = encryption_key(10, &enc, &secret).unwrap();
    assert!(matches!(key, IdentityPublicKey::V0(_)));
    assert_eq!(key.purpose(), Purpose::DECRYPTION);
    assert_eq!(key.security_level(), SecurityLevel::MEDIUM);
    assert_eq!(key.key_type(), KeyType::ECDSA_SECP256K1);
    assert_eq!(key.data().as_slice(), public);
    assert_eq!(
        key.contract_bounds(),
        Some(&ContractBounds::SingleContract {
            id: Identifier::from(OTHER)
        })
    );
}

/// "Encryption Keys": register only when no key is there; send only our current key; else omit.
#[test]
fn an_encryption_key_is_registered_sent_or_left_out() {
    let enc = EncBinding {
        contract: OTHER,
        purpose: EncPurpose::Encryption,
    };
    let ours = |rotation: u32| compressed(&Zeroizing::new([0x50 + rotation as u8; SLOT])).ok();
    let key_at = |id, rotation: u32| {
        encryption_key(id, &enc, &Zeroizing::new([0x50 + rotation as u8; SLOT])).unwrap()
    };
    assert_eq!(enc_step(&enc, &[], None, ours), EncStep::Register);

    let (r0, r1) = (key_at(3, 0), key_at(4, 1));
    let both = [r0.clone(), r1.clone()];
    assert_eq!(enc_step(&enc, &both, Some(&r1), ours), EncStep::Send(1));
    assert_eq!(enc_step(&enc, &both, Some(&r0), ours), EncStep::Send(0));

    let theirs = encryption_key(5, &enc, &Zeroizing::new([0x07; SLOT])).unwrap();
    assert_eq!(
        enc_step(&enc, std::slice::from_ref(&theirs), Some(&theirs), ours),
        EncStep::Omit,
        "a current key not ours"
    );
    assert_eq!(
        enc_step(&enc, std::slice::from_ref(&r0), None, ours),
        EncStep::Omit,
        "a key there, none current"
    );
    let mut disabled = r0.clone();
    if let IdentityPublicKey::V0(k) = &mut disabled {
        k.disabled_at = Some(1);
    }
    assert_eq!(
        enc_step(&enc, std::slice::from_ref(&disabled), Some(&disabled), ours),
        EncStep::Omit,
        "the current key is disabled"
    );
    let other_purpose = EncBinding {
        contract: OTHER,
        purpose: EncPurpose::Decryption,
    };
    assert_eq!(
        enc_step(&other_purpose, &both, None, ours),
        EncStep::Register,
        "keys of another purpose do not count"
    );
}

/// Review r2: the slots are the session keys in request order, then the encryption keys; each
/// response has its own nonce and wallet key, and the app opens both to the same keys.
#[test]
fn each_response_is_sealed_anew_with_the_slots_in_order() {
    let app_secret = SecretKey::from_secret_bytes([0x5a; 32]).unwrap();
    let app_ephemeral = PublicKey::from_secret_key(&app_secret).serialize();
    let slot = |b: u8| Zeroizing::new([b; SLOT]);
    let auth = [slot(1), slot(2)];
    let enc = [slot(3)];
    let first = response(&auth, &enc, &app_ephemeral, &OWNER).unwrap();
    let second = response(&auth, &enc, &app_ephemeral, &OWNER).unwrap();
    assert_eq!(
        first.app_key_hash,
        hash160::Hash::hash(&app_ephemeral).to_byte_array()
    );
    assert_eq!(first.payload.len(), 28 + 32 * 3);
    assert_ne!(first.payload[..12], second.payload[..12], "a new nonce");
    assert_ne!(
        first.wallet_ephemeral, second.wallet_ephemeral,
        "a new wallet key"
    );
    for answer in [&first, &second] {
        let wallet = PublicKey::from_slice(&answer.wallet_ephemeral).unwrap();
        let slots = keys::open(
            &answer.payload,
            &app_secret,
            &wallet,
            &app_ephemeral,
            &OWNER,
        )
        .expect("the app opens it");
        assert_eq!(slots, [[1; SLOT], [2; SLOT], [3; SLOT]]);
    }
}
