use super::*;
use evolite_api::{EncryptionOffer, KeyLevel};

const NOW: Timestamp = 1_800_000_000;

fn offer(budget: Duffs, proposed: Option<Duffs>, days: i64, primary: bool) -> SessionOffer {
    SessionOffer {
        binding: if primary {
            AppBinding::Group { id: "G".into() }
        } else {
            AppBinding::DashPayProfile
        },
        level: KeyLevel::High,
        proposed_budget: proposed,
        proposed_expiry: Some(NOW + days * DAY),
        budget,
        expires_at: NOW + days * DAY,
        changed: false,
        primary,
    }
}

fn request(sessions: Vec<SessionOffer>) -> AppRequest {
    AppRequest {
        id: 4,
        app: "@yappr".into(),
        kind: AppRequestKind::Connect2 {
            app_id: "A".into(),
            label: "Yappr".into(),
            owner: "O".into(),
            owner_name: Some("yappr".into()),
            identity: "@me".into(),
            sessions,
            encryption: vec![EncryptionOffer {
                contract: "A".into(),
                encryption: true,
                given: true,
                added: true,
            }],
            limits: SessionLimits::wallet(),
            breaks_old_clients: true,
            renews: None,
            created_at: NOW - 60,
            callback_host: None,
            cost: 10_000,
        },
    }
}

/// AC-D2: what the app asks is selected — for all its keys together (the architect, round 4),
/// 90 days; an amount off the presets is «Your amount» with it typed in; a period off the
/// presets is «Your date».
#[test]
fn the_apps_ask_is_what_is_selected() {
    let yappr = request(vec![
        offer(50_000_000, Some(50_000_000), 90, true),
        offer(1_000_000, Some(1_000_000), 90, false),
    ]);
    let choice = Choice::of(&yappr, NOW);
    let limits = SessionLimits::wallet();
    assert_eq!(choice.limit(&limits), Some(51_000_000), "0.5 + 0.01");
    assert_eq!(choice.budget, limits.budget_presets.len(), "«Your amount»");
    assert_eq!(choice.term, 1, "90 days");
    assert_eq!(choice.end(&limits, NOW), Some(NOW + 90 * DAY));
    assert_eq!(choice.kept, [true, true]);
    let alone = request(vec![offer(50_000_000, Some(50_000_000), 90, true)]);
    assert_eq!(Choice::of(&alone, NOW).budget, 1, "0.5 DASH, a preset");
    let greedy = request(vec![
        offer(500_000_000, Some(500_000_000), 90, true),
        offer(1_000_000, Some(1_000_000), 90, false),
    ]);
    assert_eq!(
        Choice::of(&greedy, NOW).limit(&limits),
        Some(500_000_000),
        "at most 5 DASH"
    );

    let odd = request(vec![offer(30_000_000, Some(30_000_000), 45, true)]);
    let choice = Choice::of(&odd, NOW);
    assert_eq!(choice.budget, limits.budget_presets.len(), "«Your amount»");
    assert_eq!(choice.limit(&limits), Some(30_000_000));
    assert_eq!(choice.term, limits.term_presets_days.len(), "«Your date»");
    let end = choice.end(&limits, NOW).expect("a date the wallet takes");
    assert!((end - (NOW + 45 * DAY)).abs() < DAY, "the app's day");
}

/// Nothing named: 5 DASH (the customer, 2026-11-11), as the core offers it.
#[test]
fn no_amount_named_is_5_dash() {
    let none = request(vec![offer(SessionLimits::DEFAULT_BUDGET, None, 90, true)]);
    let choice = Choice::of(&none, NOW);
    assert_eq!(choice.limit(&SessionLimits::wallet()), Some(500_000_000));
    assert!(explanation(&[none_primary(&none)], local()).contains("did not name an amount"));
}

fn none_primary(r: &AppRequest) -> SessionOffer {
    match &r.kind {
        AppRequestKind::Connect2 { sessions, .. } => sessions[0].clone(),
    }
}

fn local() -> DateTime<Local> {
    chrono::TimeZone::timestamp_opt(&Local, NOW, 0).unwrap()
}

/// One limit for the connection (the architect, round 4): the optional keys what they ask
/// while the primary key keeps something, else scaled down in proportion; the primary key the
/// rest. Total ≤ the limit, the primary key > 0, each optional ≤ its ask — over many limits.
#[test]
fn the_keys_share_one_limit() {
    let sessions = [
        offer(50_000_000, None, 90, true),
        offer(1_000_000, None, 90, false),
        offer(3_000_000, None, 90, false),
    ];
    let end = NOW + DAY;
    let budgets = |limit, kept: &[bool]| {
        split(&sessions, kept, limit, end)
            .map(|g| g.keys.iter().map(|k| k.map(|(b, _)| b)).collect::<Vec<_>>())
    };
    assert_eq!(
        budgets(54_000_000, &[true, true, true]),
        Some(vec![Some(50_000_000), Some(1_000_000), Some(3_000_000)]),
        "enough for all: each its ask"
    );
    assert_eq!(
        budgets(4_000_000, &[true, true, true]),
        Some(vec![Some(2_000_000), Some(500_000), Some(1_500_000)]),
        "the optional ones scaled to half the limit, in proportion"
    );
    assert_eq!(
        budgets(4_000_000, &[true, false, true]),
        Some(vec![Some(1_000_000), None, Some(3_000_000)]),
        "a key left off is not granted; the other one asks less than the limit and gets it"
    );
    assert_eq!(
        budgets(3, &[true, true, true]),
        None,
        "less than the app needs"
    );
    for limit in [1, 2, 5, 999, 4_000_000, 4_000_001, 54_000_000, 500_000_000] {
        for kept in [
            [true, true, true],
            [true, false, true],
            [true, false, false],
        ] {
            let Some(grant) = split(&sessions, &kept, limit, end) else {
                continue;
            };
            let total: Duffs = grant.keys.iter().flatten().map(|(b, _)| b).sum();
            assert!(total <= limit, "{limit}: total {total}");
            assert!(
                grant.keys[0].is_some_and(|(b, _)| b > 0),
                "{limit}: the primary"
            );
            for (key, offer) in grant.keys.iter().zip(&sessions).skip(1) {
                assert!(
                    key.is_none_or(|(b, _)| 0 < b && b <= offer.budget),
                    "{limit}: {key:?}"
                );
            }
            assert!(grant.keys.iter().flatten().all(|(_, e)| *e == end));
        }
    }
    assert!(
        split(&sessions, &[true, false, false], 1, end).is_some(),
        "the primary alone"
    );
}

/// The limit and the date the wallet takes: up to 5 DASH, an hour to a year; else no grant.
#[test]
fn a_choice_outside_the_limits_grants_nothing() {
    let yappr = request(vec![offer(50_000_000, Some(50_000_000), 90, true)]);
    let limits = SessionLimits::wallet();
    let mut choice = Choice::of(&yappr, NOW);
    choice.budget = limits.budget_presets.len();
    choice.amount = "6".into();
    assert_eq!(grant(&yappr, &choice, NOW), None, "over 5 DASH");
    choice.amount = "0".into();
    assert_eq!(grant(&yappr, &choice, NOW), None, "nothing");
    choice.amount = "0.25".into();
    assert!(grant(&yappr, &choice, NOW).is_some());
    choice.term = limits.term_presets_days.len();
    choice.date = text::date_input(NOW + 2 * 365 * DAY);
    assert_eq!(grant(&yappr, &choice, NOW), None, "past a year");
    choice.date = "31.02.2027".into();
    assert_eq!(grant(&yappr, &choice, NOW), None, "no such day");
}

/// E1–E3 and the rest: a title, what happened, what to do; each says nothing was spent.
#[test]
fn each_refusal_says_nothing_was_spent() {
    use AppLinkRefusal as R;
    for why in [
        R::Version { v: "1".into() },
        R::Stale,
        R::MainnetCarrier,
        R::LinkForNetwork {
            network: evolite_api::Network::Mainnet,
        },
        R::LinkForNetwork {
            network: evolite_api::Network::Testnet,
        },
        R::SystemContract,
        R::NoContract,
        R::OthersContract,
        R::NoEncryptionKeys,
        R::UnknownField,
        R::OldNetwork,
        R::Malformed,
    ] {
        let (title, body, hint) = refusal_words(&why, "Yappr");
        assert!(!title.is_empty() && !body.is_empty(), "{why:?}");
        assert!(hint.contains("Nothing was spent"), "{why:?}: {hint}");
    }
    let (_, body, _) = refusal_words(&R::Version { v: "1".into() }, "Yappr");
    assert!(body.contains("(v=1)"), "{body}");
}

#[test]
fn the_banner_says_the_limit_and_the_end() {
    let yappr = request(vec![offer(50_000_000, Some(50_000_000), 90, true)]);
    let grant = SessionGrant {
        keys: vec![Some((50_000_000, NOW + 90 * DAY))],
    };
    let banner = connected_banner(&yappr, &grant);
    assert!(banner.starts_with("Yappr connected: limit 0.5"), "{banner}");
}

/// A1b, point 9: the app's ask is what it asks for all its keys together, and its period in
/// whole days (the request was made a moment before); above 5 DASH, evolite says it lowered it.
#[test]
fn the_explanation_says_the_whole_ask() {
    let yappr = [
        offer(50_000_000, Some(50_000_000), 90, true),
        offer(1_000_000, Some(1_000_000), 90, false),
    ];
    let later = chrono::TimeZone::timestamp_opt(&Local, NOW + 30, 0).unwrap();
    let said = explanation(&yappr, later);
    assert!(said.contains("asks for 0.51 DASH and 90 days"), "{said}");
    assert!(!said.contains("lowered"), "{said}");
    let mut greedy = yappr.clone();
    greedy[0].proposed_budget = Some(1_000_000_000);
    greedy[0].budget = SessionLimits::MAX_BUDGET;
    greedy[0].changed = true;
    let said = explanation(&greedy, local());
    assert!(
        said.contains("asks for 10.01 DASH; evolite lowered it to 5 DASH"),
        "{said}"
    );
}

fn enc(contract: &str, encryption: bool, given: bool, added: bool) -> EncryptionOffer {
    EncryptionOffer {
        contract: contract.into(),
        encryption,
        given,
        added,
    }
}

const OTHER: &str = "9Mz3Rt6wQmRz8LpXcV4nB7sYd3Ke1JaPu5W2x";

/// Every encryption key the app gets is said — the identity's own current one as much as a new
/// one added — with which half of the pair it is, whose contract, and that the app receives its
/// private key; another contract is said to be another's. A key not sent is not said.
#[test]
fn every_key_the_app_gets_is_said() {
    let sent = enc(OTHER, false, true, false);
    let (icon, title, under) = key_words(&sent, "A", "Yappr").expect("sent, not added");
    assert_eq!(icon, "warning");
    assert_eq!(
        title,
        format!(
            "Get your private decryption key for another contract, {}",
            text::whole(OTHER)
        )
    );
    for words in [
        "receives the private key itself",
        "encrypted to you under this contract",
        "already has",
        "not the app's own contract",
    ] {
        assert!(under.contains(words), "«{words}»: {under}");
    }
    assert!(!under.contains("adds this new key"), "{under}");

    let added = enc("A", true, true, true);
    let (icon, title, under) = key_words(&added, "A", "Yappr").expect("added");
    assert_eq!(icon, "key");
    assert_eq!(
        title,
        "Get your private encryption key for the Yappr contract"
    );
    assert!(
        under.contains("receives the private key itself") && under.contains("adds this new key"),
        "{under}"
    );
    assert!(!under.contains("not the app's own"), "{under}");

    assert_eq!(
        key_words(&enc(OTHER, false, false, false), "A", "Yappr"),
        None,
        "not sent"
    );
}

/// The same words in ru and uk, each from the catalog: no English left in them.
#[test]
fn the_keys_words_are_translated() {
    for lang in [crate::i18n::Lang::Ru, crate::i18n::Lang::Uk] {
        crate::i18n::set(lang);
        for offer in [
            enc("A", true, true, true),
            enc("A", false, true, false),
            enc(OTHER, true, true, true),
            enc(OTHER, false, true, false),
        ] {
            let (_, title, under) = key_words(&offer, "A", "Yappr").expect("said");
            let title = crate::i18n::t_in(lang, &title).into_owned();
            for said in [&title, &under] {
                assert!(
                    !said.contains("key") && !said.contains("contract"),
                    "{lang:?}: {said}"
                );
            }
        }
    }
    crate::i18n::set(crate::i18n::Lang::En);
}

/// BUG-161, App Connect's «Your amount» limit: «0,002» and «0.002» are the same limit in every
/// language; «1,000,000» is no limit (none is granted) and the line under the field says so.
#[test]
fn your_amount_reads_one_mark_and_refuses_two() {
    let limits = SessionLimits::wallet();
    for lang in crate::i18n::Lang::ALL {
        crate::i18n::set(lang);
        crate::format::set_display(Default::default());
        let limit = |typed: &str| {
            Choice {
                budget: limits.budget_presets.len(),
                amount: typed.into(),
                term: 0,
                date: String::new(),
                kept: Vec::new(),
                for_request: 0,
            }
            .limit(&limits)
        };
        assert_eq!(limit("0,002"), Some(200_000), "{lang:?}");
        assert_eq!(limit("0.002"), Some(200_000), "{lang:?}");
        assert_eq!(limit("1,000,000"), None, "{lang:?}");
        assert!(
            crate::format::amount_hint("1,000,000").is_some(),
            "{lang:?}: its line"
        );
    }
    crate::i18n::set(crate::i18n::Lang::En);
}
