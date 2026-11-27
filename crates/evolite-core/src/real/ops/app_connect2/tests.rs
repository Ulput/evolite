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
        spent_before: Some(0),
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
    // TECH-mainnet-all: both directions by name; a code of no network evolite knows is a
    // link that is not right.
    use evolite_api::Network;
    assert_eq!(
        why(Status::Invalid, Part::Network, link, false),
        R::LinkForNetwork {
            network: Network::Mainnet
        }
    );
    assert_eq!(
        why(
            Status::Invalid,
            Part::Network,
            "dashpay://connect?v=2&n=t",
            false
        ),
        R::LinkForNetwork {
            network: Network::Testnet
        }
    );
    assert_eq!(
        why(Status::Invalid, Part::Network, "dash-key:x?n=d&v=2", false),
        R::Malformed
    );
    assert_eq!(
        why(Status::Unsupported, Part::Carrier, link, false),
        R::MainnetCarrier
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
            limit: 50_000_000,
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

/// BUG-145 AC-T3: what an app spent is its keys' budgets less what Platform says is left; with
/// a key not read there is nothing to compare (BUG-350).
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
    assert_eq!(spent_by_reads(&app, &reads), None, "key 9 not read");
    reads.insert(("C".into(), "I".into(), 9), left(49_999_500_000));
    assert_eq!(spent_by_reads(&app, &reads), Some(1_500_000));
}

/// BUG-350: reads that alternate — key A alone, then A and B, then A alone (B's read failed),
/// then both again — put B's spending into History once, not twice: a read without B is not
/// known and leaves the baseline where it was.
#[test]
fn alternating_reads_count_a_keys_spending_once() {
    let app = app(vec![stored(8, NOW_MS + 1), stored(9, NOW_MS + 1)]);
    let budget = stored(8, NOW_MS + 1).budget;
    let read = |remaining: Option<u64>| KeyRead {
        remaining,
        disabled: false,
    };
    // A spent 1 000 000 credits (1 000 duffs), B 2 000 000 (2 000 duffs); nothing more after.
    let a = Some(budget - 1_000_000);
    let b = Some(budget - 2_000_000);
    let mut reads = Reads::new();
    let mut seen = None;
    let mut lines = Vec::new();
    for (left_a, left_b) in [(a, None), (a, b), (a, None), (a, b)] {
        reads.insert(("C".into(), "I".into(), 8), read(left_a));
        reads.insert(("C".into(), "I".into(), 9), read(left_b));
        // As `read_budgets` does: not known — no line, the baseline stays.
        let Some(spent) = spent_by_reads(&app, &reads) else {
            continue;
        };
        let (line, base) = app_spent_step(seen, spent);
        seen = Some(base);
        if line > 0 {
            lines.push(line);
        }
    }
    assert_eq!(lines, vec![3_000], "A and B once");
}

/// BUG-350 (review nit): a key whose budget is not read is journaled, once a run.
#[test]
fn a_key_not_read_is_journaled_once() {
    let mut app = app(vec![stored(8, NOW_MS + 1), stored(9, NOW_MS + 1)]);
    app.contract = "BUG-350".into();
    let mut reads = Reads::new();
    reads.insert(
        ("BUG-350".into(), "I".into(), 8),
        KeyRead {
            remaining: Some(1),
            disabled: false,
        },
    );
    assert_eq!(note_unknown_keys(&app, &reads), vec![9]);
    assert_eq!(note_unknown_keys(&app, &reads), Vec::<u32>::new(), "once");
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

/// BUG-208: the response is published only once every new key is on the identity, active; the
/// keys read back with one missing or disabled are not «all on».
#[test]
fn the_response_waits_for_every_new_key_on_the_identity() {
    let added = [key(5, false), key(6, false)];
    assert!(all_on(
        &added,
        &[key(1, false), key(5, false), key(6, false)]
    ));
    assert!(!all_on(&added, &[key(5, false)]), "one missing");
    assert!(
        !all_on(&added, &[key(5, false), key(6, true)]),
        "one disabled"
    );
    assert!(all_on(&[], &[key(1, false)]), "a sign-in again adds none");
}

/// A session key's data depends only on `appEphPub` and the binding's index: the key already on
/// the identity is reused only if Platform holds it to no more than this «Allow», and kept as
/// Platform holds it. A retry (the same bounds and budget, its end worked out again a little
/// later) is; the same `appEphPub` again for a lower budget, an earlier end, other bounds, another
/// level, or a key with no limits on Platform is not. And the path refuses before anything is
/// sent. Mutant: the found key reused as before, whatever Platform holds it to.
#[test]
fn a_key_on_the_identity_is_reused_only_within_this_allow() {
    const DAY_MS: u64 = 24 * 60 * 60 * 1_000;
    let secret = [0x42u8; SLOT];
    let binding = v2::AuthBinding {
        bounds: Bounds::SingleContract([7; 32]),
        level: Level::High,
        total_budget: 0,
        expires_at: 0,
    };
    let limits = |budget, expires_at| Limits {
        budget,
        expires_at,
        changed: false,
    };
    let asked = |binding: &v2::AuthBinding, chosen: Limits| {
        grant::session_key(9, binding, &chosen, &secret).unwrap()
    };
    let first = limits(5 * grant::CREDITS_PER_DASH, NOW_MS + 365 * DAY_MS);
    let there = grant::session_key(5, &binding, &first, &secret).unwrap();

    let held = |b: &v2::AuthBinding, l: Limits| held_within(&there, &asked(b, l));

    assert_eq!(held(&binding, first), Some(first));
    let retry = limits(first.budget, first.expires_at + 120_000);
    assert_eq!(held(&binding, retry), Some(first), "Platform's end");

    let lower = limits(grant::CREDITS_PER_DASH / 2, NOW_MS + 30 * DAY_MS);
    assert_eq!(held(&binding, lower), None, "lower limits");
    let less = limits(grant::CREDITS_PER_DASH / 2, first.expires_at);
    assert_eq!(held(&binding, less), None, "a lower budget");
    let sooner = limits(first.budget, NOW_MS + 30 * DAY_MS);
    assert_eq!(held(&binding, sooner), None, "an earlier end");
    let elsewhere = v2::AuthBinding {
        bounds: Bounds::SingleContract([8; 32]),
        ..binding.clone()
    };
    assert_eq!(held(&elsewhere, first), None, "other bounds");
    let critical = v2::AuthBinding {
        level: Level::Critical,
        ..binding.clone()
    };
    assert_eq!(held(&critical, first), None, "another level");
    let unlimited = IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id: 5,
        purpose: there.purpose(),
        security_level: there.security_level(),
        contract_bounds: there.contract_bounds().cloned(),
        key_type: there.key_type(),
        read_only: false,
        data: there.data().clone(),
        disabled_at: None,
    });
    assert_eq!(
        held_within(&unlimited, &asked(&binding, first)),
        None,
        "no budget and no end on Platform"
    );

    let source = include_str!("../app_connect2.rs");
    let f = &source[source.find("fn approve_connect2(").expect("approve")..];
    let reuse = f
        .find("held_within(there, &key)")
        .expect("the found key checked");
    let kept = f
        .find("stored_key(id, &b.bounds, b.level, &held)")
        .expect("kept as Platform holds it");
    let update = f.find("sent::run(").expect("the update");
    assert!(reuse < kept && kept < update);
}

/// BUG-208: the response not published after the keys were paid: their History line and the
/// record (keys, no response) are written then, not lost; with nothing paid, neither. And the
/// path writes them in that branch, after the keys went through the one rule. Mutant: returning
/// the publish's error as before loses the paid line.
#[test]
fn a_paid_key_update_keeps_its_line_when_the_response_fails() {
    assert_eq!(after_response_failed(true), (true, true));
    assert_eq!(after_response_failed(false), (false, false));
    let source = include_str!("../app_connect2.rs");
    let f = &source[source.find("fn approve_connect2(").expect("approve")..];
    let f = &f[..f
        .find("\n    pub(in crate::real) fn read_app_budgets")
        .expect("its end")];
    let update = f.find("sent::run(").expect("the keys by the one rule");
    let publish = f.find("publish_connect_response(").expect("the response");
    let failed = f
        .find("after_response_failed(paid_keys)")
        .expect("its failure");
    assert!(update < publish && publish < failed);
    let branch = &f[failed..];
    let line = branch
        .find("journal.add(own_line(")
        .expect("the keys' line");
    let record = branch.find("apps.put(record(None))").expect("the record");
    let back = branch.find("return Err(").expect("the answer");
    assert!(line < back && record < back, "written before the answer");
}

/// BUG-208 review r1 S1: a second «Allow» while the first one's key update may still land does
/// not reach the update: the check comes before the identity is taken and before the task that
/// sends it. Mutant: the check removed, or put after `sent::run(`.
#[test]
fn a_second_allow_waits_for_the_first_key_update() {
    let source = include_str!("../app_connect2.rs");
    let f = &source[source.find("fn approve_connect2(").expect("approve")..];
    let check = f
        .find("self.unclear.keys_on_their_way(&base58(&job.identity))")
        .expect("the check");
    let busy = f
        .find("self.identity_busy.take()")
        .expect("the identity taken");
    let task = f.find("self.answer_later(").expect("the task");
    let update = f.find("sent::run(").expect("the update");
    assert!(check < busy && busy < task && task < update);
}

/// BUG-236: an App Connect request is priced with the fees every other form shows — measured by
/// the journal, at the SDK's protocol — not the defaults, so its cost is not below the keys
/// screen's for the same keys. Mutant: `convert::fees(0)` back in `open_connect2`.
#[test]
fn a_request_is_priced_with_the_fees_of_every_form() {
    let file = include_str!("../app_connect2.rs");
    let open = &file[file.find("fn open_connect2").expect("open_connect2")..];
    let open = &open[..open.find("\n    }\n").unwrap_or(open.len())];
    assert!(!open.contains("convert::fees("), "the defaults");
    assert!(
        open.contains("snapshot::fees_from("),
        "not the measured fees"
    );
}

/// BUG-301: a renewal knows the connection's limit as chosen — all its keys' budgets together.
/// BUG-302: the renewing keys take over what the renewed key of the same binding spent (read
/// just before) and had taken over; an earlier key with no pair, to the first key of its target;
/// a key of another target, nothing.
#[test]
fn a_renewal_keeps_the_limit_chosen_and_what_was_spent() {
    let mut second = stored(9, NOW_MS + 1);
    second.budget = 10_000_000_000;
    let kept = app(vec![stored(8, NOW_MS + 1), second]);
    let reads = Reads::new();
    let renewed = renewal(&kept, Some(&[0u8; 16]), &reads).expect("a renewal");
    assert_eq!(renewed.limit, 60_000_000, "50 + 10 million duffs");

    let mut old = kept.clone();
    if let Some(s) = old.session.as_mut() {
        s.keys[0].spent_before = Some(1_000_000_000);
    }
    let read = |left| KeyRead {
        remaining: Some(left),
        disabled: false,
    };
    let mut reads = Reads::new();
    reads.insert(("C".to_owned(), "I".to_owned(), 8), read(48_000_000_000));
    reads.insert(("C".to_owned(), "I".to_owned(), 9), read(9_500_000_000));
    let fresh = || {
        let mut new = vec![stored(10, NOW_MS + 2), stored(11, NOW_MS + 2)];
        new[0].kind = "group".into();
        new[0].document_type = None;
        new[1].kind = "contract".into();
        new[1].document_type = None;
        new[1].target = "other".into();
        new
    };
    let mut new = fresh();
    carry_spent(&mut new, &old, &reads);
    assert_eq!(
        new[0].spent_before,
        Some(3_500_000_000),
        "2 spent by key 8, 1 it had taken over, 0.5 by key 9 without a pair"
    );
    assert_eq!(new[1].spent_before, Some(0), "another target");
}

/// BUG-302 r1: an earlier key whose budget was not read hands over «not known», never 0 — the
/// 0 would be kept for good, the earlier keys disabled in the same update.
#[test]
fn a_renewal_without_a_read_does_not_know_what_was_spent() {
    let old = app(vec![stored(8, NOW_MS + 1), stored(9, NOW_MS + 1)]);
    let new_keys = || {
        let mut new = vec![stored(10, NOW_MS + 2)];
        new[0].kind = "group".into();
        new[0].document_type = None;
        new
    };
    let mut new = new_keys();
    carry_spent(&mut new, &old, &Reads::new());
    assert_eq!(new[0].spent_before, None, "no read: not known");

    // Key 8 read, key 9 (no pair, the same target) not: still not known.
    let mut reads = Reads::new();
    reads.insert(
        ("C".to_owned(), "I".to_owned(), 8),
        KeyRead {
            remaining: Some(50_000_000_000),
            disabled: false,
        },
    );
    let mut new = new_keys();
    carry_spent(&mut new, &old, &reads);
    assert_eq!(new[0].spent_before, None, "a part not known: not known");

    // What an earlier renewal did not know stays not known.
    let mut was = old.clone();
    if let Some(s) = was.session.as_mut() {
        s.keys.truncate(1);
        s.keys[0].spent_before = None;
    }
    let mut new = new_keys();
    carry_spent(&mut new, &was, &reads);
    assert_eq!(new[0].spent_before, None, "taken over not known");

    let shown = shown_session(&app(new), &Reads::new(), NOW_MS);
    assert_eq!(shown[0].spent_before, None);
}

/// BUG-323: each failed read of `open_connect2` is its own refusal — the stage, the id in base58,
/// the error's text for «Details», the link to open again — not «the network refused».
/// Mutant: `connect_facts(..).map_err(CoreError::Failed)` back.
#[test]
fn a_failed_read_says_which_and_keeps_the_link() {
    use app_chain_v2::{FactsError, ReadError};
    let link = "dashpay://connect?v=2&n=t&r=AQ";
    let error = || ReadError {
        text: "protocol error: decoding".into(),
        debug: "Protocol(DecodingError)".into(),
    };
    let id = [7u8; 32];
    for (failed, stage) in [
        (FactsError::Group(id, error()), ConnectRead::Group),
        (FactsError::Contract(id, error()), ConnectRead::Contract),
    ] {
        assert_eq!(
            facts_failed(failed, link, Some(11)),
            CoreError::AppLink(AppLinkRefusal::ReadFailed {
                stage,
                id: base58(&id),
                undecoded: true,
                details: "protocol error: decoding".into(),
                link: link.into(),
            })
        );
    }
    assert_eq!(
        read_failed(ConnectRead::EncKeys, &id, &error(), link, None),
        CoreError::AppLink(AppLinkRefusal::ReadFailed {
            stage: ConnectRead::EncKeys,
            id: base58(&id),
            undecoded: true,
            details: "protocol error: decoding".into(),
            link: link.into(),
        })
    );
    assert_eq!(
        facts_failed(FactsError::Sdk("no SDK".into()), link, None),
        CoreError::Failed("no SDK".into())
    );
    // r1 (Should 1): a timeout or an unreachable node is not an answer evolite could not take.
    for (debug, taken) in [
        ("Protocol(DecodingError(..))", true),
        ("Proof(InvalidProof(..))", true),
        ("Protocol(UnknownVersionMismatch { .. })", true),
        ("DapiClientError(Transport(..))", false),
        ("TimeoutReached(30s, ..)", false),
    ] {
        assert_eq!(undecoded(debug), taken, "{debug}");
    }
    // r1 (Should 2): after the password too the keys' read says its stage. Mutant:
    // `CoreError::Failed(e.text)` back in `approve_connect2`.
    let file = include_str!("../app_connect2.rs");
    let approve = &file[file.find("fn approve_connect2").expect("approve_connect2")..];
    assert!(
        approve.contains("read_failed(ConnectRead::EncKeys"),
        "the keys' read after the password by its stage"
    );
    let file = include_str!("../app_connect2.rs");
    let open = &file[file.find("fn open_connect2").expect("open_connect2")..];
    let open = &open[..open.find("\n    }\n").unwrap_or(open.len())];
    assert!(
        open.contains("facts_failed("),
        "connect_facts' failure by its stage"
    );
    assert!(
        open.contains("ConnectRead::EncKeys"),
        "enc_steps' failure by its stage"
    );
}
