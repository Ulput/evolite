//! App Connect v2 (DIP dashpay/dips#181, "App Connect: Wallet Sign-In and Transition Signing for
//! Platform Applications"): reading a sign-in request (FEAT-0023, round 1).
//!
//! An app asks to sign in with one body on two carriers:
//! - `dashpay://connect?v=2&n=<m|t|d>&r=<base64url(body)>[&cb=<https URL>]`;
//! - `dash-key:<base58(body)>?n=<t|d>&v=2[&cb=…]`, transitional, never on mainnet.
//!
//! The body is `0x02 ‖ appEphPub (33) ‖ appId (32) ‖ labelLen (1) ‖ label ‖ field*`, each field a
//! TLV (type, length, value) in non-decreasing type order, at most 1 024 bytes in all. Everything
//! here is pure: the network and the clock are arguments, nothing is fetched or signed. Whether
//! the app's contract exists, who owns it and what the wallet allows are the core's (round 3).
//!
//! A refusal says the DIP's status (what the app is told in `ac=`) and the part of the request at
//! fault, so the screen can say why in words of its own (round 4).

use dpp::secp256k1::PublicKey;

/// The connect verb's scheme, and the transitional carrier's.
const CONNECT: &str = "dashpay://connect";
const DASH_KEY: &str = "dash-key:";

/// Sizes the DIP sets.
const MAX_URI: usize = 4_096;
const MAX_R: usize = 1_366;
pub const MAX_BODY: usize = 1_024;
const MAX_LABEL: usize = 64;
const MAX_NAME: usize = 64;
/// Integers above this are refused: apps written in JavaScript cannot hold them exactly.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// How far `CREATED_AT` may be from the wallet's clock.
const STALE_BEHIND_MS: u64 = 10 * 60 * 1_000;
const STALE_AHEAD_MS: u64 = 2 * 60 * 1_000;

/// The body's version byte.
const BODY_VERSION: u8 = 0x02;

/// Field types.
const IDENTITY_HINT: u8 = 0x01;
const AUTH_BINDING: u8 = 0x02;
const CONNECTION_ID: u8 = 0x03;
const ENC_BINDING: u8 = 0x04;
const MODE: u8 = 0x05;
const CREATED_AT: u8 = 0x06;

/// The network a request names (`n=`) and the wallet runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Net {
    Main,
    Test,
    Dev,
}

impl Net {
    fn code(self) -> &'static str {
        match self {
            Net::Main => "m",
            Net::Test => "t",
            Net::Dev => "d",
        }
    }

    /// The network evolite runs on.
    pub fn current() -> Self {
        match crate::network::current() {
            evolite_api::Network::Mainnet => Net::Main,
            evolite_api::Network::Testnet => Net::Test,
        }
    }
}

/// The DIP's status for a refused request (the `ac=` value of the return link).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// `CREATED_AT` too old or too far ahead.
    Stale,
    /// A version, a carrier, a kind or a field type this wallet does not know.
    Unsupported,
    /// Malformed, or breaks a rule of the DIP.
    Invalid,
}

/// The part of the request a refusal is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Uri,
    /// The app's contract (`appId`): not found, or a system contract.
    App,
    Version,
    Network,
    /// The transitional `dash-key:` carrier on mainnet.
    Carrier,
    Body,
    AppKey,
    Label,
    Field,
    IdentityHint,
    AuthBinding,
    ConnectionId,
    EncBinding,
    Mode,
    CreatedAt,
    Callback,
}

/// Why a request is not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not an App Connect v2 link at all: another reader may take it.
    NotOurs,
    Refused(Status, Part),
}

use Status::{Invalid, Stale, Unsupported};

fn refuse<T>(status: Status, part: Part) -> Result<T, Refusal> {
    Err(Refusal::Refused(status, part))
}

/// What a session key is bound to (Platform's `ContractBounds`, by its bincode tag).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bounds {
    /// Kind 0: one contract.
    SingleContract([u8; 32]),
    /// Kind 1: one document type of one contract.
    DocumentType { contract: [u8; 32], name: String },
    /// Kind 2: a group of contracts.
    ContractGroup([u8; 32]),
}

/// The level a session key signs at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Critical,
    High,
}

/// One session key the app asks for (`AUTH_BINDING`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthBinding {
    pub bounds: Bounds,
    pub level: Level,
    /// Credits the app proposes; 0: none proposed.
    pub total_budget: u64,
    /// Block time in milliseconds the app proposes; 0: none proposed.
    pub expires_at: u64,
}

/// Which half of an encryption pair (`ENC_BINDING`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EncPurpose {
    Encryption,
    Decryption,
}

/// One encryption key the app asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncBinding {
    pub contract: [u8; 32],
    pub purpose: EncPurpose,
}

/// A sign-in request as the body gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub app_ephemeral: [u8; 33],
    /// The app's contract id.
    pub app_id: [u8; 32],
    /// What the app calls itself: its own words, not a checked name.
    pub label: String,
    pub identity_hint: Option<[u8; 32]>,
    /// 1–4; the first is the primary binding, the rest the person may leave out.
    pub auth: Vec<AuthBinding>,
    pub connection_id: Option<[u8; 16]>,
    /// 0–8.
    pub enc: Vec<EncBinding>,
    /// `MODE = 1`: renewing an earlier sign-in.
    pub renew: bool,
    /// Milliseconds since the Unix epoch, when the app made the request.
    pub created_at: u64,
}

/// Which carrier a request came on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carrier {
    Connect,
    DashKey,
}

/// A request read from a link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub request: Request,
    pub carrier: Carrier,
    /// The return link (`cb=`), decoded: only a hint to the app of how it went.
    pub callback: Option<String>,
}

/// Reads an App Connect v2 link for the wallet on `here` at `now_ms`.
pub fn parse(text: &str, here: Net, now_ms: u64) -> Result<Link, Refusal> {
    let text = text.trim();
    let lower = text.get(..CONNECT.len()).map(str::to_ascii_lowercase);
    let link = if lower.as_deref() == Some(CONNECT) {
        connect_uri(text, here)?
    } else if text
        .get(..DASH_KEY.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(DASH_KEY))
    {
        dash_key_uri(text, here)?
    } else {
        return Err(Refusal::NotOurs);
    };
    fresh(link.request.created_at, now_ms)?;
    Ok(link)
}

/// The query of a carrier: `v`, `n`, the body (`r`, or none on `dash-key:`) and `cb`, each at
/// most once; unknown parameters ignored.
struct Query<'a> {
    v: Option<&'a str>,
    n: Option<&'a str>,
    r: Option<&'a str>,
    cb: Option<&'a str>,
}

fn query(text: &str) -> Result<Query<'_>, Refusal> {
    let mut q = Query {
        v: None,
        n: None,
        r: None,
        cb: None,
    };
    for pair in text.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let slot = match key {
            "v" => &mut q.v,
            "n" => &mut q.n,
            "r" => &mut q.r,
            "cb" => &mut q.cb,
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return refuse(Invalid, Part::Uri);
        }
    }
    Ok(q)
}

/// `v` exactly 2, `n` the wallet's network.
fn version_and_network(q: &Query, here: Net) -> Result<(), Refusal> {
    match q.v {
        Some("2") => {}
        Some(_) => return refuse(Unsupported, Part::Version),
        None => return refuse(Invalid, Part::Version),
    }
    match q.n {
        Some(n) if n == here.code() => Ok(()),
        _ => refuse(Invalid, Part::Network),
    }
}

/// `dashpay://connect[/]?v=2&n=…&r=…[&cb=…]`.
fn connect_uri(text: &str, here: Net) -> Result<Link, Refusal> {
    if text.len() > MAX_URI {
        return refuse(Invalid, Part::Uri);
    }
    let rest = &text[CONNECT.len()..];
    // No userinfo, port or fragment; an empty path, a single `/` ignored.
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let Some(query_text) = rest.strip_prefix('?') else {
        return refuse(Invalid, Part::Uri);
    };
    if query_text.contains('#') {
        return refuse(Invalid, Part::Uri);
    }
    let q = query(query_text)?;
    version_and_network(&q, here)?;
    let r = q.r.ok_or(Refusal::Refused(Invalid, Part::Body))?;
    if r.len() > MAX_R {
        return refuse(Invalid, Part::Body);
    }
    let body = base64url(r).ok_or(Refusal::Refused(Invalid, Part::Body))?;
    Ok(Link {
        request: body_of(&body)?,
        carrier: Carrier::Connect,
        callback: callback(q.cb)?,
    })
}

/// `dash-key:<base58(body)>?n=…&v=2[&cb=…]`: testnet and devnet only.
fn dash_key_uri(text: &str, here: Net) -> Result<Link, Refusal> {
    let rest = &text[DASH_KEY.len()..];
    // A `dash-key:` that does not say `v=2` is the v1 reader's (`app_connect`), malformed or
    // not: it reads or refuses it in its own words.
    let Some((body, query_text)) = rest
        .split_once('?')
        .filter(|(_, q)| q.split('&').any(|p| p == "v=2"))
    else {
        return Err(Refusal::NotOurs);
    };
    let q = query(query_text)?;
    if here == Net::Main {
        return refuse(Unsupported, Part::Carrier);
    }
    version_and_network(&q, here)?;
    let body = dpp::platform_value::string_encoding::decode(
        body,
        dpp::platform_value::string_encoding::Encoding::Base58,
    )
    .ok()
    .ok_or(Refusal::Refused(Invalid, Part::Body))?;
    Ok(Link {
        request: body_of(&body)?,
        carrier: Carrier::DashKey,
        callback: callback(q.cb)?,
    })
}

/// The return link, percent-decoded: an `https://` URL or nothing.
fn callback(cb: Option<&str>) -> Result<Option<String>, Refusal> {
    let Some(cb) = cb else {
        return Ok(None);
    };
    let url = percent_decode(cb).ok_or(Refusal::Refused(Invalid, Part::Callback))?;
    let ok = url
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
        && url.len() > 8
        && !url.chars().any(char::is_control);
    if ok {
        Ok(Some(url))
    } else {
        refuse(Invalid, Part::Callback)
    }
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Canonical base64url: no padding, no `+` or `/`, never a length of 1 mod 4, no stray bits.
fn base64url(text: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32)
    }
    let bytes = text.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= value(c)? << (18 - 6 * i);
        }
        let take = chunk.len() - 1;
        // Bits past the last whole byte must be zero, or two strings decode the same.
        let used = 8 * take;
        let unused = 6 * chunk.len() - used;
        if unused > 0 && (acc >> (24 - 6 * chunk.len())) & ((1 << unused) - 1) != 0 {
            return None;
        }
        for i in 0..take {
            out.push((acc >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

/// Whether a request made at `created_at` is still fresh at `now_ms` (DIP: at most 10 minutes
/// behind and 2 ahead): checked when the link is read and again when it is approved.
pub fn fresh(created_at: u64, now_ms: u64) -> Result<(), Refusal> {
    let behind = now_ms.saturating_sub(created_at);
    let ahead = created_at.saturating_sub(now_ms);
    if behind > STALE_BEHIND_MS || ahead > STALE_AHEAD_MS {
        refuse(Stale, Part::CreatedAt)
    } else {
        Ok(())
    }
}

/// A label shown on screen: UTF-8 of at most 64 bytes, no control or text-direction characters.
fn label_ok(label: &str) -> bool {
    !label.chars().any(|c| {
        c.is_control()
            || matches!(
                c,
                '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            )
    })
}

/// Reads a body: header, label, then the fields in order.
pub fn body_of(bytes: &[u8]) -> Result<Request, Refusal> {
    const BODY: Refusal = Refusal::Refused(Invalid, Part::Body);
    if bytes.len() > MAX_BODY {
        return Err(BODY);
    }
    let (&version, rest) = bytes.split_first().ok_or(BODY)?;
    if version != BODY_VERSION {
        return refuse(Unsupported, Part::Version);
    }
    let (app_ephemeral, rest) = rest.split_first_chunk::<33>().ok_or(BODY)?;
    // A point on the curve before it goes near ECDH: the link is anyone's text.
    PublicKey::from_slice(app_ephemeral).map_err(|_| Refusal::Refused(Invalid, Part::AppKey))?;
    let (app_id, rest) = rest.split_first_chunk::<32>().ok_or(BODY)?;
    let (&len, rest) = rest.split_first().ok_or(BODY)?;
    let len = len as usize;
    if len > MAX_LABEL || rest.len() < len {
        return refuse(Invalid, Part::Label);
    }
    let (label, mut rest) = rest.split_at(len);
    let label = std::str::from_utf8(label)
        .ok()
        .filter(|l| label_ok(l))
        .ok_or(Refusal::Refused(Invalid, Part::Label))?;
    let mut request = Request {
        app_ephemeral: *app_ephemeral,
        app_id: *app_id,
        label: label.to_owned(),
        identity_hint: None,
        auth: Vec::new(),
        connection_id: None,
        enc: Vec::new(),
        renew: false,
        created_at: 0,
    };
    let mut last_type = 0u8;
    let mut seen_mode = false;
    let mut seen_created = false;
    while !rest.is_empty() {
        let (&kind, after) = rest.split_first().ok_or(BODY)?;
        let (&len, after) = after
            .split_first()
            .ok_or(Refusal::Refused(Invalid, Part::Field))?;
        let len = len as usize;
        if after.len() < len {
            return refuse(Invalid, Part::Field);
        }
        let (value, after) = after.split_at(len);
        rest = after;
        if kind < last_type {
            return refuse(Invalid, Part::Field);
        }
        let repeat = kind == last_type;
        last_type = kind;
        match kind {
            IDENTITY_HINT => {
                if repeat {
                    return refuse(Invalid, Part::IdentityHint);
                }
                request.identity_hint = Some(
                    value
                        .try_into()
                        .map_err(|_| Refusal::Refused(Invalid, Part::IdentityHint))?,
                );
            }
            AUTH_BINDING => {
                if request.auth.len() == 4 {
                    return refuse(Invalid, Part::AuthBinding);
                }
                request.auth.push(auth_binding(value)?);
            }
            CONNECTION_ID => {
                if repeat {
                    return refuse(Invalid, Part::ConnectionId);
                }
                request.connection_id = Some(
                    value
                        .try_into()
                        .map_err(|_| Refusal::Refused(Invalid, Part::ConnectionId))?,
                );
            }
            ENC_BINDING => {
                if request.enc.len() == 8 {
                    return refuse(Invalid, Part::EncBinding);
                }
                request.enc.push(enc_binding(value)?);
            }
            MODE => {
                if seen_mode || value != [1] {
                    return refuse(Invalid, Part::Mode);
                }
                seen_mode = true;
                request.renew = true;
            }
            CREATED_AT => {
                if seen_created {
                    return refuse(Invalid, Part::CreatedAt);
                }
                seen_created = true;
                let at = u64::from_be_bytes(
                    value
                        .try_into()
                        .map_err(|_| Refusal::Refused(Invalid, Part::CreatedAt))?,
                );
                if at == 0 || at > MAX_SAFE_INTEGER {
                    return refuse(Invalid, Part::CreatedAt);
                }
                request.created_at = at;
            }
            // An odd type is optional: skipped, and may repeat. An even one would change what is
            // granted.
            unknown if unknown % 2 == 1 => {}
            _ => return refuse(Unsupported, Part::Field),
        }
    }
    if request.auth.is_empty() {
        return refuse(Invalid, Part::AuthBinding);
    }
    if !seen_created {
        return refuse(Invalid, Part::CreatedAt);
    }
    Ok(request)
}

/// `kind (1) ‖ securityLevel (1) ‖ id (32) ‖ totalBudget (u64) ‖ expiresAt (u64)
/// [‖ nameLen (1) ‖ name]`, the name on documentType only.
fn auth_binding(value: &[u8]) -> Result<AuthBinding, Refusal> {
    const BAD: Refusal = Refusal::Refused(Invalid, Part::AuthBinding);
    let (&kind, rest) = value.split_first().ok_or(BAD)?;
    let (&level, rest) = rest.split_first().ok_or(BAD)?;
    let (id, rest) = rest.split_first_chunk::<32>().ok_or(BAD)?;
    let (budget, rest) = rest.split_first_chunk::<8>().ok_or(BAD)?;
    let (expires, rest) = rest.split_first_chunk::<8>().ok_or(BAD)?;
    let level = match level {
        1 => Level::Critical,
        2 => Level::High,
        _ => return Err(BAD),
    };
    let expires_at = u64::from_be_bytes(*expires);
    if expires_at > MAX_SAFE_INTEGER {
        return Err(BAD);
    }
    let bounds = match kind {
        0 | 2 if !rest.is_empty() => return Err(BAD),
        0 => Bounds::SingleContract(*id),
        2 => Bounds::ContractGroup(*id),
        1 => {
            let (&len, name) = rest.split_first().ok_or(BAD)?;
            let ok = (1..=MAX_NAME).contains(&(len as usize))
                && name.len() == len as usize
                && name
                    .iter()
                    .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-');
            if !ok {
                return Err(BAD);
            }
            Bounds::DocumentType {
                contract: *id,
                name: String::from_utf8(name.to_vec()).map_err(|_| BAD)?,
            }
        }
        _ => return refuse(Unsupported, Part::AuthBinding),
    };
    Ok(AuthBinding {
        bounds,
        level,
        total_budget: u64::from_be_bytes(*budget),
        expires_at,
    })
}

/// `contractId (32) ‖ purpose (1)`.
fn enc_binding(value: &[u8]) -> Result<EncBinding, Refusal> {
    const BAD: Refusal = Refusal::Refused(Invalid, Part::EncBinding);
    let (contract, rest) = value.split_first_chunk::<32>().ok_or(BAD)?;
    let purpose = match rest {
        [1] => EncPurpose::Encryption,
        [2] => EncPurpose::Decryption,
        _ => return Err(BAD),
    };
    Ok(EncBinding {
        contract: *contract,
        purpose,
    })
}

pub mod grant;
pub mod keys;

#[cfg(test)]
mod tests;
