//! Signing in to apps with the wallet's identity: the key exchange Yappr's wallet sign-in uses
//! and DashPay for Android answers (contract `key-exchange-v2` on testnet,
//! [`CONTRACT`]). Two links:
//!
//! 1. `dash-key:` — the app's fresh ephemeral key and its contract. The wallet answers with a
//!    `loginKeyResponse` document: a login key for (identity, app), encrypted to the app
//!    (ECDH, HKDF-SHA256, AES-256-GCM). The app derives two keys from it.
//! 2. `dash-st:` — when those keys are not on the identity yet, the app asks for them: an
//!    IdentityUpdate adding an authentication key (HIGH, `ECDSA_HASH160`) and an encryption key
//!    (MEDIUM, `ECDSA_SECP256K1`). The wallet checks that it adds exactly the keys of a login
//!    it answered, then builds and signs its own update with the MASTER key: it never signs the
//!    app's bytes.
//!
//! What the wallet derives is its own business (the app checks keys on chain): the login key
//! comes from the identity's MASTER key, the app's contract and a generation. "Disconnect"
//! disables the two keys, and Platform never takes a disabled key back, so the next sign-in to
//! that app takes the next generation — found again from the identity's keys after a restore.
//!
//! Only testnet links are taken; no key of another purpose or level, no transfer, nothing that
//! moves credits but the fees of the wallet's own writes.

use dpp::dashcore::hashes::{Hash, hash160};
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::{IdentityPublicKey, KeyType, Purpose, SecurityLevel};
use dpp::secp256k1::{self, PublicKey, SecretKey};
use dpp::serialization::PlatformDeserializableUntrusted;
use dpp::state_transition::StateTransition;
use dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dpp::state_transition::identity_update_transition::accessors::IdentityUpdateTransitionAccessorsV0;
use dpp::state_transition::public_key_in_creation::accessors::{
    IdentityPublicKeyInCreationV0Getters, IdentityPublicKeyInCreationV1Getters,
};
use zeroize::Zeroizing;

/// The key-exchange contract on testnet (Yappr's `contracts/key-exchange-v2.json`).
pub const CONTRACT: &str = "7UaqHGBJBbRLJ4fUWS45cnud8PPUugJWoGTt1SKwHJ2P";
pub const DOCUMENT_TYPE: &str = "loginKeyResponse";

const KEY_SCHEME: &str = "dash-key:";
const ST_SCHEME: &str = "dash-st:";
/// The only version of both links there is.
const VERSION: &str = "1";
const BODY_VERSION: u8 = 0x01;
const MAX_LABEL: usize = 64;

/// HKDF salt of the envelope key, and the infos of the two keys the app derives.
const ENVELOPE_SALT: &[u8] = b"dash:key-exchange:v1";
const AUTH_INFO: &[u8] = b"auth";
const ENCRYPTION_INFO: &[u8] = b"encryption";
/// evolite's own login-key derivation: never changes, or every app sign-in breaks.
const LOGIN_KEY_INFO: &[u8] = b"evolite:login-key:v1";

/// The encrypted login key: nonce (12), ciphertext (32), tag (16).
pub const PAYLOAD_LEN: usize = 60;

/// Why a link is not taken.
#[derive(Debug, PartialEq)]
pub enum LinkError {
    /// Not an app link at all.
    NotOurs,
    /// An app link, refused for this reason (shown as is).
    Refused(&'static str),
}

/// A `dash-key:` sign-in request.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyRequest {
    pub app_ephemeral: [u8; 33],
    pub contract_id: [u8; 32],
    /// What the app calls itself: anyone can write anything here.
    pub label: String,
}

#[derive(Debug, PartialEq)]
pub enum Link {
    Key(KeyRequest),
    /// The serialized transition of a `dash-st:` link.
    Transition(Vec<u8>),
}

/// The `n=` of links this network takes: `t` on testnet; none on mainnet, where the
/// key-exchange contract ([`CONTRACT`]) does not exist.
pub fn link_network(network: evolite_api::Network) -> Option<&'static str> {
    match network {
        evolite_api::Network::Testnet => Some("t"),
        evolite_api::Network::Mainnet => None,
    }
}

/// Reads a link: the scheme, no authority, `n=t` and `v=1` once each, a Base58 body.
pub fn parse(text: &str) -> Result<Link, LinkError> {
    let text = text.trim();
    let (scheme, rest) = if let Some(rest) = text.strip_prefix(KEY_SCHEME) {
        (KEY_SCHEME, rest)
    } else if let Some(rest) = text.strip_prefix(ST_SCHEME) {
        (ST_SCHEME, rest)
    } else if text.starts_with("dash-kx:") {
        return Err(LinkError::Refused(tr!(
            "Apps do not use connecting by code (dash-kx:) yet: sign in with a dash-key: link"
        )));
    } else {
        return Err(LinkError::NotOurs);
    };
    if rest.starts_with("//") {
        return Err(LinkError::NotOurs);
    }
    let (body, query) = rest.split_once('?').ok_or(LinkError::Refused(tr!(
        "The link has no network and version (?n=…&v=…): it was cut short"
    )))?;
    let (mut network, mut version) = (None, None);
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let slot = match key {
            "n" => &mut network,
            "v" => &mut version,
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return Err(LinkError::Refused(tr!(
                "The link is damaged: a parameter repeats"
            )));
        }
    }
    // The contract and the keys are testnet's: a link is never taken on another network, and a
    // testnet link must not sign with the same identity's keys on mainnet.
    let Some(here) = link_network(crate::network::current()) else {
        return Err(LinkError::Refused(tr!(
            "App sign-in works on testnet only for now: start evolite without --network mainnet"
        )));
    };
    match network {
        Some(n) if n == here => {}
        Some("m") => {
            return Err(LinkError::Refused(tr!(
                "The link is for mainnet, and evolite runs on testnet"
            )));
        }
        Some(_) => {
            return Err(LinkError::Refused(tr!(
                "The link is for another network, not testnet"
            )));
        }
        None => return Err(LinkError::Refused(tr!("The link names no network"))),
    }
    if version != Some(VERSION) {
        return Err(LinkError::Refused(tr!(
            "evolite does not know this request version: update the app or the wallet"
        )));
    }
    let bytes = dpp::platform_value::string_encoding::decode(
        body,
        dpp::platform_value::string_encoding::Encoding::Base58,
    )
    .ok()
    .filter(|b| !b.is_empty())
    .ok_or(LinkError::Refused(tr!(
        "The link is damaged: its body cannot be read"
    )))?;
    if scheme == ST_SCHEME {
        return Ok(Link::Transition(bytes));
    }
    key_request(&bytes).map(Link::Key)
}

/// `0x01 ‖ appEphemeralPubKey (33) ‖ contractId (32) ‖ labelLen (1) ‖ label`, nothing after.
fn key_request(bytes: &[u8]) -> Result<KeyRequest, LinkError> {
    const BROKEN: LinkError =
        LinkError::Refused(tr!("The link is damaged: the request cannot be read"));
    let (&version, rest) = bytes.split_first().ok_or(BROKEN)?;
    if version != BODY_VERSION {
        return Err(LinkError::Refused(tr!(
            "evolite does not know this request version: update the app or the wallet"
        )));
    }
    let (app_ephemeral, rest) = rest.split_first_chunk::<33>().ok_or(BROKEN)?;
    let (contract_id, rest) = rest.split_first_chunk::<32>().ok_or(BROKEN)?;
    let (&len, label) = rest.split_first().ok_or(BROKEN)?;
    if len as usize > MAX_LABEL || label.len() != len as usize {
        return Err(BROKEN);
    }
    let label = std::str::from_utf8(label).map_err(|_| BROKEN)?;
    // Shown on screen: no control characters, no text-direction tricks.
    if label.chars().any(|c| {
        c.is_control() || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        return Err(BROKEN);
    }
    // A point on the curve before it goes near ECDH: the link is anyone's text.
    PublicKey::from_slice(app_ephemeral).map_err(|_| BROKEN)?;
    Ok(KeyRequest {
        app_ephemeral: *app_ephemeral,
        contract_id: *contract_id,
        label: label.to_owned(),
    })
}

/// `hash160(appEphemeralPubKey)`: the request's id in the response document.
pub fn request_hash(app_ephemeral: &[u8; 33]) -> [u8; 20] {
    hash160::Hash::hash(app_ephemeral).to_byte_array()
}

fn hkdf(ikm: &[u8], salt: &[u8], info: &[&[u8]]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    hkdf::Hkdf::<sha2::Sha256>::new(Some(salt), ikm)
        .expand_multi_info(info, out.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 length");
    out
}

/// The X coordinate of `secret · point`, unhashed, as the app computes it.
fn ecdh_x(secret: &SecretKey, point: &PublicKey) -> Zeroizing<[u8; 32]> {
    let xy = Zeroizing::new(secp256k1::ecdh::shared_secret_point(point, secret));
    let mut x = Zeroizing::new([0u8; 32]);
    x.copy_from_slice(&xy[..32]);
    x
}

/// The login key encrypted to the app: `nonce ‖ AES-256-GCM(key, nonce, login key) ‖ tag`, the
/// key from ECDH of the wallet's ephemeral key and the app's, no AAD.
pub fn seal(
    login_key: &[u8; 32],
    wallet_ephemeral: &SecretKey,
    app_ephemeral: &PublicKey,
    nonce: [u8; 12],
) -> [u8; PAYLOAD_LEN] {
    use aes_gcm::aead::{Aead, KeyInit};
    let key = hkdf(
        ecdh_x(wallet_ephemeral, app_ephemeral).as_ref(),
        ENVELOPE_SALT,
        &[],
    );
    let sealed = aes_gcm::Aes256Gcm::new_from_slice(key.as_ref())
        .expect("a 32-byte key")
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), login_key.as_ref())
        .expect("AES-GCM encrypts 32 bytes");
    let mut out = [0u8; PAYLOAD_LEN];
    out[..12].copy_from_slice(&nonce);
    out[12..].copy_from_slice(&sealed);
    out
}

/// What the app does with the response: the login key back, or `None` (another key, a changed
/// byte). Used by the tests and the live test, which play the app.
#[cfg(test)]
pub fn open(
    payload: &[u8; PAYLOAD_LEN],
    secret: &SecretKey,
    point: &PublicKey,
) -> Option<Zeroizing<[u8; 32]>> {
    use aes_gcm::aead::{Aead, KeyInit};
    let key = hkdf(ecdh_x(secret, point).as_ref(), ENVELOPE_SALT, &[]);
    let plain = aes_gcm::Aes256Gcm::new_from_slice(key.as_ref())
        .ok()?
        .decrypt(aes_gcm::Nonce::from_slice(&payload[..12]), &payload[12..])
        .ok()?;
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(plain.get(..32).filter(|_| plain.len() == 32)?);
    Some(out)
}

/// The login key for (identity, app contract, generation), from the identity's MASTER private
/// key: the same after a restore, another for every app and after every "Disconnect".
pub fn login_key(
    master_private: &[u8; 32],
    identity_id: &[u8; 32],
    contract_id: &[u8; 32],
    generation: u32,
) -> Zeroizing<[u8; 32]> {
    hkdf(
        master_private,
        identity_id,
        &[LOGIN_KEY_INFO, contract_id, &generation.to_be_bytes()],
    )
}

/// The two keys an app derives from the login key and wants on the identity.
pub struct AppKeys {
    pub auth: Zeroizing<[u8; 32]>,
    pub encryption: Zeroizing<[u8; 32]>,
}

impl AppKeys {
    pub fn of(login_key: &[u8; 32], identity_id: &[u8; 32]) -> Self {
        Self {
            auth: hkdf(login_key, identity_id, &[AUTH_INFO]),
            encryption: hkdf(login_key, identity_id, &[ENCRYPTION_INFO]),
        }
    }

    fn public(secret: &[u8; 32]) -> [u8; 33] {
        let secret = SecretKey::from_secret_bytes(*secret).expect("an HKDF output is a valid key");
        PublicKey::from_secret_key(&secret).serialize()
    }

    /// The authentication key as the identity holds it: `hash160` of the public key.
    pub fn auth_data(&self) -> [u8; 20] {
        hash160::Hash::hash(&Self::public(&self.auth)).to_byte_array()
    }

    /// The encryption key as the identity holds it: the compressed public key.
    pub fn encryption_data(&self) -> [u8; 33] {
        Self::public(&self.encryption)
    }

    /// The keys to add, with these ids: authentication HIGH, encryption MEDIUM, both unbound.
    pub fn identity_keys(&self, auth_id: u32, encryption_id: u32) -> [IdentityPublicKey; 2] {
        use dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
        let key = |id, purpose, security_level, key_type, data: &[u8]| {
            IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id,
                purpose,
                security_level,
                contract_bounds: None,
                key_type,
                read_only: false,
                data: dpp::platform_value::BinaryData::new(data.to_vec()),
                disabled_at: None,
            })
        };
        [
            key(
                auth_id,
                Purpose::AUTHENTICATION,
                SecurityLevel::HIGH,
                KeyType::ECDSA_HASH160,
                &self.auth_data(),
            ),
            key(
                encryption_id,
                Purpose::ENCRYPTION,
                SecurityLevel::MEDIUM,
                KeyType::ECDSA_SECP256K1,
                &self.encryption_data(),
            ),
        ]
    }
}

/// Where an app's keys of one generation stand on the identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OnIdentity {
    /// Neither is there.
    Absent,
    /// Both, enabled: the app is signed in. Their key ids.
    Active { auth: u32, encryption: u32 },
    /// One or both, disabled ("Disconnect"): this generation is spent.
    Disabled,
    /// One of them only, enabled.
    Partial,
}

/// Where `keys` stand among `identity_keys`.
pub fn on_identity<'a>(
    keys: &AppKeys,
    identity_keys: impl Iterator<Item = &'a IdentityPublicKey>,
) -> OnIdentity {
    let (auth_data, encryption_data) = (keys.auth_data(), keys.encryption_data());
    let (mut auth, mut encryption, mut disabled) = (None, None, false);
    for key in identity_keys {
        let data = key.data().as_slice();
        let found = if data == auth_data {
            &mut auth
        } else if data == encryption_data {
            &mut encryption
        } else {
            continue;
        };
        disabled |= key.disabled_at().is_some();
        *found = Some(key.id());
    }
    match (auth, encryption) {
        _ if disabled => OnIdentity::Disabled,
        (Some(auth), Some(encryption)) => OnIdentity::Active { auth, encryption },
        (None, None) => OnIdentity::Absent,
        _ => OnIdentity::Partial,
    }
}

/// The first generation whose keys this identity has not disabled: what the next sign-in to the
/// app uses. `keys_of(g)`: the app's keys of generation `g`.
pub fn current_generation(
    keys_of: impl Fn(u32) -> AppKeys,
    identity_keys: &[IdentityPublicKey],
) -> u32 {
    (0..u32::MAX)
        .find(|g| on_identity(&keys_of(*g), identity_keys.iter()) != OnIdentity::Disabled)
        .unwrap_or(u32::MAX)
}

/// What a `dash-st:` asks, once checked.
#[derive(Debug, PartialEq)]
pub struct KeyRegistration {
    pub identity_id: [u8; 32],
    /// The keys it adds, as (purpose, level, type, data): only the two of a login.
    pub added: Vec<(Purpose, SecurityLevel, KeyType, Vec<u8>)>,
}

/// Reads a `dash-st:` transition: an IdentityUpdate, framed as apps send it (the bare transition)
/// or as a whole state transition. Anything else is refused: evolite signs only the keys of an
/// app sign-in from a link.
pub fn read_transition(bytes: &[u8]) -> Result<KeyRegistration, &'static str> {
    let update = match IdentityUpdateTransition::deserialize_from_bytes_untrusted_exact(bytes) {
        Ok(update) => update,
        Err(_) => match StateTransition::deserialize_from_bytes_untrusted_exact(bytes) {
            Ok(StateTransition::IdentityUpdate(update)) => update,
            Ok(_) => {
                return Err(tr!(
                    "By link evolite signs only app sign-in keys, and this operation is something else"
                ));
            }
            Err(_) => return Err(tr!("The operation in the link cannot be read")),
        },
    };
    if !update.public_key_ids_to_disable().is_empty() {
        return Err(tr!(
            "The operation disables identity keys: evolite does not sign that by link"
        ));
    }
    let mut added = Vec::new();
    for key in update.public_keys_to_add() {
        if key.contract_bounds().is_some() || key.read_only() || key.has_limits() {
            return Err(tr!(
                "The operation adds a key of a different kind than app sign-in uses"
            ));
        }
        added.push((
            key.purpose(),
            key.security_level(),
            key.key_type(),
            key.data().to_vec(),
        ));
    }
    if added.is_empty() || added.len() > 2 {
        return Err(tr!(
            "The operation adds keys other than the app sign-in ones"
        ));
    }
    Ok(KeyRegistration {
        identity_id: update.identity_id().to_buffer(),
        added,
    })
}

impl KeyRegistration {
    /// Whether it adds the keys of `keys` and nothing else — at most both, each once, each of
    /// its own purpose, level and type.
    pub fn adds_only(&self, keys: &AppKeys) -> bool {
        let expected = [
            (
                Purpose::AUTHENTICATION,
                SecurityLevel::HIGH,
                KeyType::ECDSA_HASH160,
                keys.auth_data().to_vec(),
            ),
            (
                Purpose::ENCRYPTION,
                SecurityLevel::MEDIUM,
                KeyType::ECDSA_SECP256K1,
                keys.encryption_data().to_vec(),
            ),
        ];
        let mut seen = [false; 2];
        self.added
            .iter()
            .all(|key| match expected.iter().position(|e| e == key) {
                Some(i) if !seen[i] => {
                    seen[i] = true;
                    true
                }
                _ => false,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn key(byte: u8) -> SecretKey {
        SecretKey::from_secret_bytes([byte; 32]).unwrap()
    }

    fn point(secret: &SecretKey) -> PublicKey {
        PublicKey::from_secret_key(secret)
    }

    /// The login key of the DApp fixtures (`platform-auth` `yappr-vectors.ts`, as DashPay for
    /// Android's `KeyExchangeCryptoTest` checks them).
    fn fixture_login_key() -> [u8; 32] {
        std::array::from_fn(|i| ((i * 7 + 3) & 0xff) as u8)
    }

    #[test]
    fn derives_the_app_keys_as_the_app_does() {
        let keys = AppKeys::of(&fixture_login_key(), &[0xab; 32]);
        assert_eq!(
            keys.auth.to_vec(),
            hex("e06ee7ae45f257741dab7379793c854829b67171af111238c664d9fd90603706")
        );
        assert_eq!(
            keys.encryption.to_vec(),
            hex("6879c11819d1a60026adae34c296e83d13d06559ad63da41d03c44b203a90f80")
        );
        assert_eq!(
            hash160::Hash::hash(&[1, 2, 3]).to_byte_array().to_vec(),
            hex("9bc4860bb936abf262d7a51f74b4304833fee3b2")
        );
    }

    #[test]
    fn the_app_opens_what_the_wallet_seals_and_nothing_else() {
        let (app, wallet) = (key(1), key(2));
        let login = fixture_login_key();
        let payload = seal(
            &login,
            &wallet,
            &point(&app),
            std::array::from_fn(|i| i as u8),
        );
        assert_eq!(
            &payload[..12],
            &std::array::from_fn::<u8, 12, _>(|i| i as u8)
        );
        // The app: its private key and the wallet's ephemeral public key.
        assert_eq!(*open(&payload, &app, &point(&wallet)).unwrap(), login);
        // Another app's key does not open it.
        assert!(open(&payload, &key(3), &point(&wallet)).is_none());
        // A changed byte does not open either.
        let mut broken = payload;
        broken[20] ^= 1;
        assert!(open(&broken, &app, &point(&wallet)).is_none());
    }

    fn uri(body: &[u8], query: &str) -> String {
        format!(
            "dash-key:{}?{query}",
            dpp::platform_value::string_encoding::encode(
                body,
                dpp::platform_value::string_encoding::Encoding::Base58
            )
        )
    }

    fn body(label: &[u8]) -> Vec<u8> {
        let mut body = vec![BODY_VERSION];
        body.extend(point(&key(1)).serialize());
        body.extend([0xcd; 32]);
        body.push(label.len() as u8);
        body.extend(label);
        body
    }

    #[test]
    fn reads_a_sign_in_link_as_yappr_writes_it() {
        let Ok(Link::Key(request)) = parse(&uri(&body(b"Login to Yappr"), "n=t&v=1")) else {
            panic!("a good link");
        };
        assert_eq!(request.label, "Login to Yappr");
        assert_eq!(request.contract_id, [0xcd; 32]);
        assert_eq!(request.app_ephemeral, point(&key(1)).serialize());
        assert!(
            parse(&format!("{}&x=1", uri(&body(b""), "n=t&v=1"))).is_ok(),
            "other parameters are ignored"
        );
    }

    #[test]
    fn refuses_links_that_are_not_well_formed() {
        let refused = |text: &str| matches!(parse(text), Err(LinkError::Refused(_)));
        assert_eq!(parse("https://evil.example"), Err(LinkError::NotOurs));
        assert_eq!(parse("dash-key://x?n=t&v=1"), Err(LinkError::NotOurs));
        assert!(refused(&uri(&body(b"a"), "n=m&v=1")), "mainnet");
        assert!(refused(&uri(&body(b"a"), "n=d&v=1")), "devnet");
        assert!(refused(&uri(&body(b"a"), "n=t&v=2")), "another version");
        assert!(refused(&uri(&body(b"a"), "n=t&n=t&v=1")), "repeated");
        assert!(refused(&uri(&body(b"a"), "v=1")), "no network");
        assert!(refused("dash-key:3mJr7AoUXx2Wqd"), "no query");
        assert!(refused("dash-key:0OIl?n=t&v=1"), "not Base58");
        assert!(refused("dash-kx:abc?n=t&v=1"), "the code exchange");
        let mut wrong_version = body(b"a");
        wrong_version[0] = 2;
        assert!(refused(&uri(&wrong_version, "n=t&v=1")));
        let mut trailing = body(b"a");
        trailing.push(0);
        assert!(refused(&uri(&trailing, "n=t&v=1")), "bytes after the label");
        let mut off_curve = body(b"a");
        off_curve[1..34].copy_from_slice(&[0xff; 33]);
        off_curve[1] = 0x02;
        assert!(refused(&uri(&off_curve, "n=t&v=1")), "not a point");
        assert!(
            refused(&uri(&body(&[b'a'; 65]), "n=t&v=1")),
            "label too long"
        );
        assert!(
            refused(&uri(&body("Yappr\u{202e}rppaY".as_bytes()), "n=t&v=1")),
            "a direction override"
        );
        assert!(
            refused(&uri(&body(b"Yap\npr"), "n=t&v=1")),
            "a control character"
        );
    }

    #[test]
    fn a_login_key_is_per_identity_app_and_generation() {
        let master = [7u8; 32];
        let base = login_key(&master, &[1; 32], &[2; 32], 0);
        assert_eq!(
            *base,
            *login_key(&master, &[1; 32], &[2; 32], 0),
            "deterministic"
        );
        for other in [
            login_key(&master, &[9; 32], &[2; 32], 0),
            login_key(&master, &[1; 32], &[9; 32], 0),
            login_key(&master, &[1; 32], &[2; 32], 1),
            login_key(&[8; 32], &[1; 32], &[2; 32], 0),
        ] {
            assert_ne!(*base, *other);
        }
    }

    fn keys_of(generation: u32) -> AppKeys {
        AppKeys::of(
            &login_key(&[7; 32], &[1; 32], &[2; 32], generation),
            &[1; 32],
        )
    }

    fn disabled(mut key: IdentityPublicKey) -> IdentityPublicKey {
        use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
        key.set_disabled_at(1);
        key
    }

    #[test]
    fn a_disconnected_app_signs_in_with_the_next_generation() {
        let [auth, encryption] = keys_of(0).identity_keys(6, 7);
        assert_eq!(current_generation(keys_of, &[]), 0, "a first sign-in");
        let active = [auth.clone(), encryption.clone()];
        assert_eq!(
            on_identity(&keys_of(0), active.iter()),
            OnIdentity::Active {
                auth: 6,
                encryption: 7
            }
        );
        assert_eq!(current_generation(keys_of, &active), 0, "signed in");
        let revoked = [disabled(auth), disabled(encryption)];
        assert_eq!(
            on_identity(&keys_of(0), revoked.iter()),
            OnIdentity::Disabled
        );
        assert_eq!(current_generation(keys_of, &revoked), 1, "the next one");
        let [auth, _] = keys_of(1).identity_keys(8, 9);
        assert_eq!(on_identity(&keys_of(1), [auth].iter()), OnIdentity::Partial);
    }

    /// An IdentityUpdate as Yappr builds it: these keys, unsigned, bare.
    fn update(identity: [u8; 32], keys: Vec<IdentityPublicKey>, disable: Vec<u32>) -> Vec<u8> {
        use dpp::serialization::PlatformSerializable;
        use dpp::state_transition::identity_update_transition::v0::IdentityUpdateTransitionV0;
        IdentityUpdateTransition::V0(IdentityUpdateTransitionV0 {
            identity_id: identity.into(),
            revision: 2,
            nonce: 5,
            add_public_keys: keys.into_iter().map(Into::into).collect(),
            disable_public_keys: disable,
            user_fee_increase: 0,
            signature_public_key_id: 0,
            signature: Default::default(),
        })
        .serialize_to_bytes()
        .unwrap()
    }

    #[test]
    fn signs_only_the_keys_of_a_login_it_answered() {
        let keys = keys_of(0);
        let [auth, encryption] = keys.identity_keys(6, 7);
        let bytes = update([1; 32], vec![auth.clone(), encryption.clone()], vec![]);
        let asked = read_transition(&bytes).unwrap();
        assert_eq!(asked.identity_id, [1; 32]);
        assert!(asked.adds_only(&keys));
        // Framed as a whole state transition too.
        let mut tagged = vec![6u8];
        tagged.extend(&bytes);
        assert!(read_transition(&tagged).unwrap().adds_only(&keys));
        // One of the two (the other is already there).
        assert!(
            read_transition(&update([1; 32], vec![encryption.clone()], vec![]))
                .unwrap()
                .adds_only(&keys)
        );
        // Another login's keys, or a third key.
        assert!(!asked.adds_only(&keys_of(1)));
        let [extra, _] = keys_of(1).identity_keys(8, 9);
        assert!(
            read_transition(&update(
                [1; 32],
                vec![auth.clone(), encryption.clone(), extra],
                vec![]
            ))
            .is_err()
        );
        // Disabling a key, a stronger key, a bound key.
        assert!(read_transition(&update([1; 32], vec![auth.clone()], vec![0])).is_err());
        let master = {
            use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
            let mut k = auth.clone();
            k.set_security_level(SecurityLevel::MASTER);
            k
        };
        assert!(
            !read_transition(&update([1; 32], vec![master], vec![]))
                .unwrap()
                .adds_only(&keys)
        );
        assert!(read_transition(b"not a transition").is_err());
    }
}
