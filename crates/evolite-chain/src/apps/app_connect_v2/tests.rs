//! The reference vectors are the DIP's ("Test Vectors", dip-pasta-app-connect.md, MIT License,
//! Copyright (c) 2026 Dash Core Group, Inc.), made by the reference app implementation:
//! https://github.com/PastaPastaPasta/dips/blob/dec23c7f2a5efe94434f9c18c19e04e6e323960f/dip-pasta-app-connect.md
//! (branch `dip-pasta-yappr`, commit of 2026-11-09, PR dashpay/dips#181).

use super::*;

fn hex(text: &str) -> Vec<u8> {
    let text: String = text.split_whitespace().collect();
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

fn id(base58: &str) -> [u8; 32] {
    dpp::platform_value::string_encoding::decode(
        base58,
        dpp::platform_value::string_encoding::Encoding::Base58,
    )
    .expect("base58")
    .try_into()
    .expect("32 bytes")
}

/// The DIP's request body: label `Yappr`, a CONNECTION_ID, no IDENTITY_HINT, a contractGroup
/// binding (CRITICAL, 50 000 000 000 credits), a DashPay `profile` binding (HIGH,
/// 1 000 000 000 credits) and an ENCRYPTION binding on the app's contract.
const BODY: &str = "
0203e83cb927e85f96ae50a95ca3be21cc7a3755139a941b15d99b01af4fecf0d270f8833c7a2d58ab816c24ae915fe2aa94
676588b467ba1e9f2b24c1a3df7fdb7805596170707202320201b0590e59269c6885acf71d27ddfce85187e2cc9736c42e95
8127cd35be6d38e80000000ba43b7400000001a2f7beb400023a0102a2a1b4ac6fef22ea2a1a68e8123644b357875f6b412c
18109281c146e7b271bc000000003b9aca00000001a2f7beb4000770726f66696c650310a69b1b4f666681453dec3734667a
5ab20421f8833c7a2d58ab816c24ae915fe2aa94676588b467ba1e9f2b24c1a3df7fdb78010608000001a128425c00";

/// The same request as a `connect` URI on devnet, with a return link.
const URI: &str = "dashpay://connect?v=2&n=d&r=AgPoPLkn6F-WrlCpXKO-Icx6N1UTmpQbFdmbAa9P7PDScPiDPHotWKuBbCSukV_iqpRnZYi0Z7oenyskwaPff9t4BVlhcHByAjICAbBZDlkmnGiFrPcdJ9386FGH4syXNsQulYEnzTW-bTjoAAAAC6Q7dAAAAAGi9760AAI6AQKiobSsb-8i6ioaaOgSNkSzV4dfa0EsGBCSgcFG57JxvAAAAAA7msoAAAABove-tAAHcHJvZmlsZQMQppsbT2ZmgUU97Dc0ZnpasgQh-IM8ei1Yq4FsJK6RX-KqlGdliLRnuh6fKyTBo99_23gBBggAAAGhKEJcAA&cb=https%3A%2F%2Fyap.pr%2Fconnect%3Fr%3Dd77dc0da9df83a47755a15c42c5d9db9d7a7fd62";

const CREATED: u64 = 1_791_676_800_000;
const EXPIRES: u64 = 1_799_452_800_000;
const APP_ID: &str = "Hj6B9qRVHcVB97LZGEQvKm7zFsCP85cuSkEAssc1XWL7";

fn expected() -> Request {
    Request {
        app_ephemeral: hex("03e83cb927e85f96ae50a95ca3be21cc7a3755139a941b15d99b01af4fecf0d270")
            .try_into()
            .unwrap(),
        app_id: id(APP_ID),
        label: "Yappr".into(),
        identity_hint: None,
        auth: vec![
            AuthBinding {
                bounds: Bounds::ContractGroup(id("CsPWxFwNvLPjACBCQBSXJLHAimU44DDFimzA2Jbxm9GK")),
                level: Level::Critical,
                total_budget: 50_000_000_000,
                expires_at: EXPIRES,
            },
            AuthBinding {
                bounds: Bounds::DocumentType {
                    contract: id("Bwr4WHCPz5rFVAD87RqTs3izo4zpzwsEdKPWUT1NS1C7"),
                    name: "profile".into(),
                },
                level: Level::High,
                total_budget: 1_000_000_000,
                expires_at: EXPIRES,
            },
        ],
        connection_id: Some(hex("a69b1b4f666681453dec3734667a5ab2").try_into().unwrap()),
        enc: vec![EncBinding {
            contract: id(APP_ID),
            purpose: EncPurpose::Encryption,
        }],
        renew: false,
        created_at: CREATED,
    }
}

#[test]
fn the_dips_request_body_reads_as_it_says() {
    let bytes = hex(BODY);
    assert_eq!(bytes.len(), 247);
    assert_eq!(body_of(&bytes), Ok(expected()));
}

#[test]
fn the_dips_connect_uri_reads_on_devnet_with_its_return_link() {
    let link = parse(URI, Net::Dev, CREATED).expect("the vector");
    assert_eq!(link.request, expected());
    assert_eq!(link.carrier, Carrier::Connect);
    assert_eq!(
        link.callback.as_deref(),
        Some("https://yap.pr/connect?r=d77dc0da9df83a47755a15c42c5d9db9d7a7fd62")
    );
    // Scheme and verb compare case-insensitively.
    let upper = URI.replacen("dashpay://connect", "DashPay://Connect", 1);
    assert_eq!(
        parse(&upper, Net::Dev, CREATED).map(|l| l.request),
        Ok(expected())
    );
}

fn dash_key(n: &str, v: &str) -> String {
    let body = dpp::platform_value::string_encoding::encode(
        &hex(BODY),
        dpp::platform_value::string_encoding::Encoding::Base58,
    );
    format!("dash-key:{body}?n={n}&v={v}")
}

#[test]
fn dash_key_carries_v2_on_testnet_and_devnet_but_never_on_mainnet() {
    let link = parse(&dash_key("t", "2"), Net::Test, CREATED).expect("testnet");
    assert_eq!((link.request, link.carrier), (expected(), Carrier::DashKey));
    assert!(parse(&dash_key("d", "2"), Net::Dev, CREATED).is_ok());
    assert_eq!(
        parse(&dash_key("m", "2"), Net::Main, CREATED),
        Err(Refusal::Refused(Unsupported, Part::Carrier))
    );
    assert_eq!(
        parse(&dash_key("t", "1"), Net::Test, CREATED),
        Err(Refusal::NotOurs),
        "a v1 dash-key: is the v1 reader's"
    );
    // Round 3: v2 is read first now, so any dash-key: that does not say v=2 goes to the v1
    // reader to be read or refused in its own words, malformed ones included.
    for v1 in [
        "dash-key:3mJr7AoUXx2Wqd",
        "dash-key:3mJr7AoUXx2Wqd?n=t&n=t&v=1",
        "dash-key:3mJr7AoUXx2Wqd?n=t",
    ] {
        assert_eq!(parse(v1, Net::Test, CREATED), Err(Refusal::NotOurs), "{v1}");
    }
}

#[test]
fn another_network_or_version_is_refused() {
    assert_eq!(
        parse(URI, Net::Test, CREATED),
        Err(Refusal::Refused(Invalid, Part::Network))
    );
    let v3 = URI.replacen("v=2", "v=3", 1);
    assert_eq!(
        parse(&v3, Net::Dev, CREATED),
        Err(Refusal::Refused(Unsupported, Part::Version))
    );
    let no_v = URI.replacen("v=2&", "", 1);
    assert_eq!(
        parse(&no_v, Net::Dev, CREATED),
        Err(Refusal::Refused(Invalid, Part::Version))
    );
    let mut body = hex(BODY);
    body[0] = 0x03;
    assert_eq!(
        body_of(&body),
        Err(Refusal::Refused(Unsupported, Part::Version))
    );
}

#[test]
fn a_request_older_than_10_minutes_or_over_2_ahead_is_stale() {
    let minute = 60_000;
    assert!(parse(URI, Net::Dev, CREATED + 10 * minute).is_ok());
    assert_eq!(
        parse(URI, Net::Dev, CREATED + 10 * minute + 1),
        Err(Refusal::Refused(Stale, Part::CreatedAt))
    );
    assert!(parse(URI, Net::Dev, CREATED - 2 * minute).is_ok());
    assert_eq!(
        parse(URI, Net::Dev, CREATED - 2 * minute - 1),
        Err(Refusal::Refused(Stale, Part::CreatedAt))
    );
}

#[test]
fn the_uri_has_one_of_each_known_parameter_and_no_fragment() {
    let repeated = format!("{URI}&n=d");
    assert_eq!(
        parse(&repeated, Net::Dev, CREATED),
        Err(Refusal::Refused(Invalid, Part::Uri))
    );
    let unknown = format!("{URI}&utm=x");
    assert!(
        parse(&unknown, Net::Dev, CREATED).is_ok(),
        "unknown ones are ignored"
    );
    let fragment = format!("{URI}#x");
    assert_eq!(
        parse(&fragment, Net::Dev, CREATED),
        Err(Refusal::Refused(Invalid, Part::Uri))
    );
    let slash = URI.replacen("connect?", "connect/?", 1);
    assert!(
        parse(&slash, Net::Dev, CREATED).is_ok(),
        "a single / is ignored"
    );
    let long = format!("{URI}&x={}", "a".repeat(MAX_URI));
    assert_eq!(
        parse(&long, Net::Dev, CREATED),
        Err(Refusal::Refused(Invalid, Part::Uri))
    );
    assert_eq!(
        parse("dashpay://invite?x=1", Net::Dev, CREATED),
        Err(Refusal::NotOurs)
    );
}

#[test]
fn the_body_is_canonical_base64url() {
    assert_eq!(base64url("AQID"), Some(vec![1, 2, 3]));
    assert_eq!(base64url("AQ"), Some(vec![1]));
    assert_eq!(base64url("AR"), None, "stray bits");
    assert_eq!(base64url("AQ=="), None, "padding");
    assert_eq!(base64url("+/8"), None, "the standard alphabet");
    assert_eq!(base64url("AQIDB"), None, "1 mod 4");
}

#[test]
fn the_return_link_is_https_only() {
    let http = URI.replacen("https%3A", "http%3A", 1);
    assert_eq!(
        parse(&http, Net::Dev, CREATED),
        Err(Refusal::Refused(Invalid, Part::Callback))
    );
    let none = URI.split("&cb=").next().unwrap();
    assert_eq!(parse(none, Net::Dev, CREATED).map(|l| l.callback), Ok(None));
}

/// A body with the vector's header and these fields.
fn body_with(label: &[u8], fields: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut out = hex(BODY)[..66].to_vec();
    out.push(label.len() as u8);
    out.extend_from_slice(label);
    for (kind, value) in fields {
        out.push(*kind);
        out.push(value.len() as u8);
        out.extend_from_slice(value);
    }
    out
}

fn auth(kind: u8, level: u8, budget: u64, expires: u64, name: Option<&[u8]>) -> Vec<u8> {
    let mut out = vec![kind, level];
    out.extend_from_slice(&[7; 32]);
    out.extend_from_slice(&budget.to_be_bytes());
    out.extend_from_slice(&expires.to_be_bytes());
    if let Some(name) = name {
        out.push(name.len() as u8);
        out.extend_from_slice(name);
    }
    out
}

fn created() -> (u8, Vec<u8>) {
    (CREATED_AT, CREATED.to_be_bytes().to_vec())
}

fn group() -> (u8, Vec<u8>) {
    (AUTH_BINDING, auth(2, 1, 1, EXPIRES, None))
}

fn refused(fields: &[(u8, Vec<u8>)]) -> Result<Request, Refusal> {
    body_of(&body_with(b"App", fields))
}

#[test]
fn the_fields_come_in_order_with_their_counts() {
    assert!(refused(&[group(), created()]).is_ok());
    assert_eq!(
        refused(&[created(), group()]),
        Err(Refusal::Refused(Invalid, Part::Field)),
        "out of order"
    );
    assert_eq!(
        refused(&[created()]),
        Err(Refusal::Refused(Invalid, Part::AuthBinding)),
        "no binding"
    );
    assert_eq!(
        refused(&[group()]),
        Err(Refusal::Refused(Invalid, Part::CreatedAt)),
        "no time"
    );
    let five: Vec<_> = (0..5).map(|_| group()).chain([created()]).collect();
    assert_eq!(
        refused(&five),
        Err(Refusal::Refused(Invalid, Part::AuthBinding))
    );
    let twice = [
        group(),
        (CONNECTION_ID, vec![1; 16]),
        (CONNECTION_ID, vec![1; 16]),
        created(),
    ];
    assert_eq!(
        refused(&twice),
        Err(Refusal::Refused(Invalid, Part::ConnectionId))
    );
    assert_eq!(
        refused(&[(IDENTITY_HINT, vec![1; 31]), group(), created()]),
        Err(Refusal::Refused(Invalid, Part::IdentityHint)),
        "wrong length"
    );
}

#[test]
fn an_unknown_odd_field_is_skipped_and_an_even_one_refused() {
    let odd = [group(), created(), (0x07, vec![9, 9]), (0x07, vec![])];
    assert!(refused(&odd).is_ok());
    assert_eq!(
        refused(&[group(), created(), (0x08, vec![])]),
        Err(Refusal::Refused(Unsupported, Part::Field))
    );
}

#[test]
fn mode_is_renew_or_absent() {
    assert_eq!(
        refused(&[group(), (MODE, vec![1]), created()]).map(|r| r.renew),
        Ok(true)
    );
    assert_eq!(
        refused(&[group(), (MODE, vec![2]), created()]),
        Err(Refusal::Refused(Invalid, Part::Mode))
    );
}

#[test]
fn integers_too_large_for_javascript_or_a_zero_time_are_invalid() {
    assert_eq!(
        refused(&[group(), (CREATED_AT, 0u64.to_be_bytes().to_vec())]),
        Err(Refusal::Refused(Invalid, Part::CreatedAt))
    );
    assert_eq!(
        refused(&[group(), (CREATED_AT, (1u64 << 53).to_be_bytes().to_vec())]),
        Err(Refusal::Refused(Invalid, Part::CreatedAt))
    );
    assert_eq!(
        refused(&[(AUTH_BINDING, auth(2, 1, 1, 1 << 53, None)), created()]),
        Err(Refusal::Refused(Invalid, Part::AuthBinding))
    );
}

#[test]
fn a_binding_knows_its_kind_level_and_name() {
    let case = |value: Vec<u8>| refused(&[(AUTH_BINDING, value), created()]);
    assert_eq!(
        case(auth(3, 1, 1, 1, None)),
        Err(Refusal::Refused(Unsupported, Part::AuthBinding)),
        "unknown kind"
    );
    assert_eq!(
        case(auth(0, 3, 1, 1, None)),
        Err(Refusal::Refused(Invalid, Part::AuthBinding)),
        "level"
    );
    assert_eq!(
        case(auth(0, 2, 1, 1, Some(b"x"))),
        Err(Refusal::Refused(Invalid, Part::AuthBinding)),
        "a name on singleContract"
    );
    assert_eq!(
        case(auth(1, 2, 1, 1, None)),
        Err(Refusal::Refused(Invalid, Part::AuthBinding)),
        "documentType without a name"
    );
    assert_eq!(
        case(auth(1, 2, 1, 1, Some(b"bad name"))),
        Err(Refusal::Refused(Invalid, Part::AuthBinding))
    );
    let ok = case(auth(1, 2, 0, 0, Some(b"profile-2_x"))).expect("a name of [A-Za-z0-9_-]");
    assert_eq!(ok.auth[0].total_budget, 0, "0: none proposed");
    let enc = |value: Vec<u8>| refused(&[group(), (ENC_BINDING, value), created()]);
    let mut decryption = vec![5; 32];
    decryption.push(2);
    assert_eq!(
        enc(decryption).map(|r| r.enc[0].purpose),
        Ok(EncPurpose::Decryption)
    );
    let mut other = vec![5; 32];
    other.push(3);
    assert_eq!(enc(other), Err(Refusal::Refused(Invalid, Part::EncBinding)));
}

#[test]
fn the_label_is_plain_text_of_at_most_64_bytes() {
    let fields = [group(), created()];
    assert!(body_of(&body_with(&[b'a'; 64], &fields)).is_ok());
    assert_eq!(
        body_of(&body_with(&[b'a'; 65], &fields)),
        Err(Refusal::Refused(Invalid, Part::Label))
    );
    assert_eq!(
        body_of(&body_with("Ya\u{202e}ppr".as_bytes(), &fields)),
        Err(Refusal::Refused(Invalid, Part::Label)),
        "a text-direction override"
    );
    assert_eq!(
        body_of(&body_with(&[b'a', 0x07], &fields)),
        Err(Refusal::Refused(Invalid, Part::Label))
    );
}

#[test]
fn a_body_that_is_too_long_cut_short_or_off_the_curve_is_invalid() {
    let mut long = body_with(b"App", &[group(), created()]);
    long.extend(std::iter::repeat_n(0, MAX_BODY));
    assert_eq!(body_of(&long), Err(Refusal::Refused(Invalid, Part::Body)));
    let mut cut = body_with(b"App", &[group(), created()]);
    cut.truncate(cut.len() - 3);
    assert_eq!(body_of(&cut), Err(Refusal::Refused(Invalid, Part::Field)));
    let mut off = body_with(b"App", &[group(), created()]);
    off[1] = 0x05;
    assert_eq!(body_of(&off), Err(Refusal::Refused(Invalid, Part::AppKey)));
}
