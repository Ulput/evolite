use super::*;
use dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dpp::identity::{KeyType, Purpose, SecurityLevel};

const NOW_MS: u64 = 1_800_000_000_000;

fn key(id: u32, disabled: bool) -> IdentityPublicKey {
    IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id,
        purpose: Purpose::AUTHENTICATION,
        security_level: SecurityLevel::HIGH,
        contract_bounds: None,
        key_type: KeyType::ECDSA_HASH160,
        read_only: false,
        data: dpp::platform_value::BinaryData::new(vec![id as u8; 20]),
        disabled_at: disabled.then_some(1),
    })
}

fn stored(id: u32, expires_at: u64) -> StoredKey {
    StoredKey {
        id,
        critical: id == 8,
        kind: if id == 8 { "group" } else { "document_type" }.into(),
        target: "T".into(),
        document_type: (id != 8).then(|| "profile".into()),
        budget: 50_000_000_000,
        expires_at,
    }
}

fn app(keys: Vec<StoredKey>) -> App {
    App {
        contract: "C".into(),
        name: "@yappr".into(),
        identity: "I".into(),
        generation: 0,
        keys: None,
        last_used: 1,
        session: Some(Session {
            connection: Some("00".repeat(16)),
            keys,
            response: None,
            label: "Yappr".into(),
            disconnected_at: None,
            spent_seen: None,
        }),
    }
}

/// UC-2/UC-4: each session key is shown live, expired or disabled, with its budget in duffs and
/// what Platform said is left once it was read.
#[test]
fn a_session_shows_each_keys_state_and_what_is_left() {
    let app = app(vec![
        stored(8, NOW_MS + 1),
        stored(9, NOW_MS),
        stored(10, NOW_MS + 1),
    ]);
    let mut reads = Reads::new();
    let at = |id| ("C".to_owned(), "I".to_owned(), id);
    reads.insert(
        at(8),
        KeyRead {
            remaining: Some(20_000_000_000),
            disabled: false,
        },
    );
    reads.insert(
        at(10),
        KeyRead {
            remaining: Some(0),
            disabled: true,
        },
    );
    let shown = shown_session(&app, &reads, NOW_MS);
    let states: Vec<_> = shown.iter().map(|k| k.state).collect();
    assert_eq!(
        states,
        [
            SessionKeyState::Live,
            SessionKeyState::Expired,
            SessionKeyState::Disabled
        ]
    );
    assert_eq!(shown[0].level, KeyLevel::Critical);
    assert_eq!(shown[0].binding, AppBinding::Group { id: "T".into() });
    assert_eq!(
        shown[1].binding,
        AppBinding::DocumentType {
            contract: "T".into(),
            name: "profile".into()
        }
    );
    assert_eq!(shown[0].budget, 50_000_000, "0.5 DASH in duffs");
    assert_eq!(shown[0].remaining, Some(20_000_000));
    assert_eq!(shown[1].remaining, None, "not read yet");
    assert_eq!(shown[0].expires_at, ((NOW_MS + 1) / 1_000) as Timestamp);
    let older = App {
        session: None,
        ..app.clone()
    };
    assert!(
        shown_session(&older, &reads, NOW_MS).is_empty(),
        "a v1 sign-in"
    );
}

/// UC-5: "Disconnect" disables the session's keys still enabled, nothing else: not a key
/// already disabled, not a key of the identity outside the session (its encryption keys).
#[test]
fn disconnect_takes_only_the_sessions_live_keys() {
    let session = Session {
        connection: None,
        keys: vec![stored(8, NOW_MS), stored(9, NOW_MS), stored(12, NOW_MS)],
        ..Default::default()
    };
    let identity = [key(8, false), key(9, true), key(10, false), key(11, false)];
    assert_eq!(live_session_keys(&session, &identity), [8]);
}

/// The response's values come back from the store as they went in, or not at all.
#[test]
fn a_kept_response_reads_back_for_its_delete() {
    let response = grant::Response {
        app_key_hash: [0xaa; 20],
        wallet_ephemeral: [0x02; 33],
        payload: vec![0xff; 60],
    };
    let kept = Stored {
        document: base58(&[7; 32]),
        app_key_hash: evolite_common::hex(&response.app_key_hash),
        wallet_ephemeral: evolite_common::hex(&response.wallet_ephemeral),
        payload: evolite_common::hex(&response.payload),
    };
    assert_eq!(stored_response(&kept), Some(([7; 32], response)));
    let broken = Stored {
        app_key_hash: "zz".into(),
        ..kept
    };
    assert_eq!(stored_response(&broken), None);
}

/// Round 4 (E1–E3): each refusal is told by its kind, with the link's own version or network;
/// a link not of App Connect is no link.
#[test]
fn a_refusal_says_which_kind() {
    let why = |status, part, text: &str, system| match refusal(
        Refusal::Refused(status, part),
        text,
        system,
    ) {
        CoreError::AppLink(why) => why,
        other => panic!("{other:?}"),
    };
    let link = "dashpay://connect?v=3&n=m&r=AQ";
    use AppLinkRefusal as R;
    assert_eq!(why(Status::Stale, Part::CreatedAt, "", false), R::Stale);
    assert_eq!(
        why(Status::Unsupported, Part::Version, link, false),
        R::Version { v: "3".into() }
    );
    assert_eq!(why(Status::Invalid, Part::Network, link, false), R::Mainnet);
    assert_eq!(
        why(Status::Invalid, Part::Network, "dash-key:x?n=d&v=2", false),
        R::OtherNetwork { n: "d".into() }
    );
    assert_eq!(
        why(Status::Unsupported, Part::Carrier, link, false),
        R::Mainnet
    );
    assert_eq!(why(Status::Invalid, Part::App, "", true), R::SystemContract);
    assert_eq!(why(Status::Invalid, Part::App, "", false), R::NoContract);
    assert_eq!(
        why(Status::Invalid, Part::AuthBinding, "", false),
        R::OthersContract
    );
    assert_eq!(
        why(Status::Invalid, Part::EncBinding, "", false),
        R::NoEncryptionKeys
    );
    assert_eq!(
        why(Status::Unsupported, Part::Field, "", false),
        R::UnknownField
    );
    assert_eq!(why(Status::Invalid, Part::Body, "", false), R::Malformed);
    assert!(matches!(
        refusal(Refusal::NotOurs, "", false),
        CoreError::BadLink
    ));
}

/// D1/D2: "Disconnect" keeps the record, marked, its response gone, and its keys show
/// disabled; only a disconnected app can be removed from the list.
#[test]
fn a_disconnected_app_stays_until_removed() {
    let mut kept = app(vec![stored(8, NOW_MS + 1)]);
    kept.session.as_mut().unwrap().response = Some(Stored {
        document: "D".into(),
        app_key_hash: String::new(),
        wallet_ephemeral: String::new(),
        payload: String::new(),
    });
    let off = disconnected(kept.clone(), 77);
    let session = off.session.as_ref().unwrap();
    assert_eq!(
        (session.disconnected_at, &session.response),
        (Some(77), &None)
    );
    let shown = shown_session(&off, &Reads::new(), NOW_MS);
    assert_eq!(shown[0].state, SessionKeyState::Disabled);

    let dir = std::env::temp_dir().join(format!("evolite-forget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let apps = crate::real::connected_apps::ConnectedApps::new(dir.join("apps.json"));
    apps.put(kept).unwrap();
    assert!(
        matches!(forget(&apps, "C", "I"), Err(CoreError::NotAvailable(_))),
        "connected: not removed"
    );
    apps.put(off).unwrap();
    assert_eq!(forget(&apps, "C", "I"), Ok(()));
    assert!(apps.get("C", "I").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// R2: a request with the connection of a kept sign-in renews it, told with what its primary key
/// spent; another connection, none, or a disconnected sign-in is no renewal.
#[test]
fn a_request_of_the_same_connection_is_a_renewal() {
    let kept = app(vec![stored(8, NOW_MS + 1)]);
    let same = [0u8; 16];
    let mut reads = Reads::new();
    assert_eq!(
        renewal(&kept, Some(&same), &reads),
        Some(Renewal {
            budget: 50_000_000,
            spent: None,
            until: ((NOW_MS + 1) / 1_000) as Timestamp,
        })
    );
    reads.insert(
        ("C".to_owned(), "I".to_owned(), 8),
        KeyRead {
            remaining: Some(48_000_000_000),
            disabled: false,
        },
    );
    assert_eq!(
        renewal(&kept, Some(&same), &reads).and_then(|r| r.spent),
        Some(2_000_000)
    );
    assert_eq!(renewal(&kept, Some(&[1u8; 16]), &reads), None);
    assert_eq!(renewal(&kept, None, &reads), None);
    assert_eq!(renewal(&disconnected(kept, 1), Some(&same), &reads), None);
}

/// DashPay's `profile` is named as such, in a request and in the list.
#[test]
fn dashpay_profile_is_told_by_its_name() {
    let dashpay = dpp::system_data_contracts::SystemDataContract::Dashpay
        .id()
        .to_buffer();
    assert_eq!(
        binding(&Bounds::DocumentType {
            contract: dashpay,
            name: "profile".into()
        }),
        AppBinding::DashPayProfile
    );
    assert!(matches!(
        binding(&Bounds::DocumentType {
            contract: dashpay,
            name: "contactRequest".into()
        }),
        AppBinding::DocumentType { .. }
    ));
    let mut profile = stored(9, NOW_MS);
    profile.target = base58(&dashpay);
    assert_eq!(stored_binding(&profile), AppBinding::DashPayProfile);
}

#[test]
fn the_return_link_is_named_by_its_host() {
    assert_eq!(
        callback_host("https://yap.pr/staging/login?ac=1").as_deref(),
        Some("yap.pr")
    );
    assert_eq!(callback_host("http://yap.pr/"), None);
    assert_eq!(callback_host("https:///x"), None);
}

/// BUG-145 AC-T3: what an app spent is its keys' budgets less what Platform says is left; a
/// key not read counts nothing, and with none read there is nothing to compare.
#[test]
fn an_apps_spending_is_its_budgets_less_what_is_left() {
    let app = app(vec![stored(8, NOW_MS + 1), stored(9, NOW_MS + 1)]);
    let mut reads = Reads::new();
    assert_eq!(spent_by_reads(&app, &reads), None);
    let left = |remaining| KeyRead {
        remaining: Some(remaining),
        disabled: false,
    };
    reads.insert(("C".into(), "I".into(), 8), left(49_999_000_000));
    assert_eq!(spent_by_reads(&app, &reads), Some(1_000_000));
    reads.insert(("C".into(), "I".into(), 9), left(49_999_500_000));
    assert_eq!(spent_by_reads(&app, &reads), Some(1_500_000));
}

/// BUG-145 AC-T3: each read adds the growth since the last one, in whole duffs of the totals
/// (nothing lost to rounding over many reads); a fall (renewal, a new budget) adds nothing and
/// only moves the baseline; never read before, everything spent so far once.
#[test]
fn a_read_adds_only_what_was_spent_since_the_last() {
    assert_eq!(app_spent_step(Some(0), 1_500), (1, 1_500));
    assert_eq!(app_spent_step(Some(1_500), 2_600), (1, 2_600));
    assert_eq!(app_spent_step(Some(2_600), 3_000), (1, 3_000));
    assert_eq!(app_spent_step(Some(3_000), 3_000), (0, 3_000));
    assert_eq!(app_spent_step(Some(3_000), 500), (0, 500));
    assert_eq!(app_spent_step(None, 122_264_000), (122_264, 122_264_000));
}
