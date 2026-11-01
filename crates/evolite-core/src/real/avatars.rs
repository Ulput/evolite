//! Profile pictures. A DashPay profile carries a link and, usually, the SHA-256 of the
//! picture: the core downloads each link once (https only, at most 1 MB, 15 s), keeps the
//! bytes only when they match that hash, and hands them to the sandboxed renderer like an NFT
//! picture. The interface sees a key (`avatar:…`), never the bytes or the link's content.
//!
//! Downloading tells the picture's host this computer's address; it is the same as any DashPay
//! client does. A link to this computer or the local network is never fetched: a stranger's
//! profile must not make the wallet knock on the person's router or printer. What comes back is
//! kept only when its first bytes say it is a picture the renderer draws.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use evolite_render::Format;
use sha2::{Digest, Sha256};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// The largest picture taken: DashPay clients keep avatars small.
pub const MAX_BYTES: u64 = 1 << 20;

/// Why a link to a private address is not fetched.
const PRIVATE: &str =
    tr!("an address on this computer or the local network — such links are not opened");
const TOO_BIG: &str = tr!("the picture is over 1 MB");
const NOT_PICTURE: &str = tr!("the link is not a picture: PNG, JPEG, GIF, WebP or SVG is needed");
/// A picture that did not download for a network reason is tried again after this long.
const RETRY: Duration = Duration::from_secs(5 * 60);

/// An address on the public internet: not this computer, the local network, link-local,
/// carrier-grade NAT, documentation or multicast ranges.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                // 240.0.0.0/4, reserved.
                || a >= 240
                // 192.0.0.0/24, protocol assignments.
                || (a == 192 && b == 0 && v4.octets()[2] == 0)
                // 100.64.0.0/10, shared address space (CGNAT).
                || (a == 100 && (64..128).contains(&b))
                // 198.18.0.0/15, benchmarking.
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            let embedded = |hi: u16, lo: u16| {
                std::net::Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8)
            };
            // NAT64 (64:ff9b::/96, 64:ff9b:1::/48) and 6to4 (2002::/16) carry an IPv4 address
            // that a gateway reaches for us: it must be public too.
            if s[0] == 0x64 && s[1] == 0xff9b && (s[2..6] == [0; 4] || s[2] == 1) {
                return is_public(IpAddr::V4(embedded(s[6], s[7])));
            }
            if s[0] == 0x2002 {
                return is_public(IpAddr::V4(embedded(s[1], s[2])));
            }
            let first = s[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // ::a.b.c.d (deprecated IPv4-compatible) and the rest of ::/96.
                || s[..6] == [0; 6]
                // 2001::/32, Teredo: an IPv4 address hidden inside, reached by a relay.
                || (first == 0x2001 && s[1] == 0)
                // 2001:db8::/32, documentation; 100::/64, discard.
                || (first == 0x2001 && s[1] == 0x0db8)
                || (first == 0x0100 && s[1..4] == [0; 3])
                // fc00::/7, unique local; fe80::/10, link-local.
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// The system's name lookup with private addresses left out. Checked here, at connection time,
/// so a name that resolves to a public address for a first look and a private one later
/// (DNS rebinding) still never reaches the local network.
#[derive(Debug, Default)]
struct PublicOnly(DefaultResolver);

impl Resolver for PublicOnly {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let mut public = self.empty();
        for addr in self.0.resolve(uri, config, timeout)?.iter() {
            if is_public(addr.ip()) {
                public.push(*addr);
            }
        }
        if public.is_empty() {
            return Err(ureq::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                PRIVATE,
            )));
        }
        Ok(public)
    }
}

/// What a downloaded picture is, by its first bytes: one of the formats the renderer draws as a
/// picture (PNG and animated PNG, JPEG, GIF, WebP, SVG), or `None`. A web page with an inline
/// `<svg>` is a page, not a picture.
pub fn picture_format(bytes: &[u8]) -> Option<Format> {
    match evolite_render::sniff(bytes)? {
        Format::Glb => None,
        Format::Svg => {
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_lowercase();
            (!head.contains("<html") && !head.contains("<!doctype html")).then_some(Format::Svg)
        }
        format => Some(format),
    }
}

#[derive(Default)]
struct State {
    /// By key.
    pictures: HashMap<String, Arc<[u8]>>,
    /// Links tried (with their hash), so a dead link is not fetched on every snapshot.
    tried: HashSet<String>,
    /// Downloads going on now.
    running: usize,
}

/// At most this many downloads at a time: a burst of profiles waits for the next snapshot
/// instead of starting a thread each.
const MAX_DOWNLOADS: usize = 4;
/// At most this many pictures kept (each up to [`MAX_BYTES`]); more are not kept until a restart.
const MAX_PICTURES: usize = 256;

#[derive(Clone, Default)]
pub struct Avatars {
    state: Arc<Mutex<State>>,
}

/// The key of a picture: the identity and the picture itself, so a new picture is a new key
/// and the interface draws it again.
fn key(id: &str, url: &str, hash: Option<&[u8; 32]>) -> String {
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    if let Some(hash) = hash {
        h.update(hash);
    }
    let tag: String = h.finalize()[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("avatar:{id}:{tag}")
}

impl Avatars {
    /// The bytes behind a key, for the renderer.
    pub fn picture(&self, key: &str) -> Option<Arc<[u8]>> {
        self.lock().pictures.get(key).cloned()
    }

    /// The key of identity `id`'s picture when it is here; otherwise starts its download
    /// (once per link) and returns `None`. `ready` is called when a picture arrives.
    pub fn want(
        &self,
        id: &str,
        url: Option<&str>,
        hash: Option<[u8; 32]>,
        ready: impl Fn() + Send + 'static,
    ) -> Option<String> {
        let url = url?.trim();
        if !url.starts_with("https://") {
            return None;
        }
        let key = key(id, url, hash.as_ref());
        {
            let mut state = self.lock();
            if state.pictures.contains_key(&key) {
                return Some(key);
            }
            if state.running >= MAX_DOWNLOADS || state.pictures.len() >= MAX_PICTURES {
                return None;
            }
            if !state.tried.insert(key.clone()) {
                return None;
            }
            state.running += 1;
        }
        let url = url.to_owned();
        let me = self.clone();
        let started = std::thread::Builder::new()
            .name("evolite-avatar".into())
            .spawn(move || {
                let result = download(&url);
                me.lock().running -= 1;
                match result {
                    Ok(bytes) if hash.is_none_or(|h| Sha256::digest(&bytes)[..] == h[..]) => {
                        me.lock().pictures.insert(key, bytes.into());
                        ready();
                    }
                    // Not the profile's picture: the same link will not become it.
                    Ok(_) => {}
                    // The network or the host: tried again later, not only after a restart.
                    Err(e) if ![PRIVATE, TOO_BIG, NOT_PICTURE].contains(&e.as_str()) => {
                        std::thread::sleep(RETRY);
                        me.lock().tried.remove(&key);
                        ready();
                    }
                    Err(_) => {}
                }
            });
        if started.is_err() {
            self.lock().running -= 1;
        }
        None
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// GET `url` over https from a public address (redirects included), no more than
/// [`MAX_BYTES`], and only a picture the renderer draws.
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let config = ureq::Agent::config_builder()
        .https_only(true)
        // Not HTTPS_PROXY / ALL_PROXY from the environment: through a proxy the public-address
        // check would see the proxy, not the picture's host. evolite has no proxy setting.
        .proxy(None)
        .max_redirects(3)
        .timeout_global(Some(Duration::from_secs(15)))
        .build();
    let agent = ureq::Agent::with_parts(config, DefaultConnector::new(), PublicOnly::default());
    let mut response = agent.get(url).call().map_err(|e| match e {
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => PRIVATE.into(),
        e => e.to_string(),
    })?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(TOO_BIG.into());
    }
    if picture_format(&bytes).is_none() {
        return Err(NOT_PICTURE.into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_links_that_are_not_https() {
        let avatars = Avatars::default();
        assert_eq!(
            avatars.want("id", Some("http://example.com/a.png"), None, || {}),
            None
        );
        assert_eq!(
            avatars.want("id", Some("file:///etc/passwd"), None, || {}),
            None
        );
        assert!(avatars.lock().tried.is_empty(), "nothing fetched");
        assert!(download("http://example.com/a.png").is_err());
    }

    #[test]
    fn only_public_addresses_are_fetched() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:192.168.1.1",
            "240.0.0.1",
            "192.0.0.8",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b:1::c0a8:101",
            "2002:c0a8:101::1",
            "2002:7f00:1::",
            "::7f00:1",
            "::c0a8:101",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2001:db8::1",
            "100::1",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private}");
        }
        for public in [
            "1.1.1.1",
            "140.82.121.4",
            "2606:4700::1111",
            "64:ff9b::101:101",
            "2002:101:101::1",
        ] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
        // Refused at the lookup, before any connection; the reason is said in words.
        assert_eq!(download("https://127.0.0.1/a.png"), Err(PRIVATE.into()));
        assert_eq!(download("https://localhost/a.png"), Err(PRIVATE.into()));
    }

    #[test]
    fn keeps_only_pictures_the_renderer_draws() {
        assert_eq!(picture_format(b"\x89PNG\r\n\x1a\n...."), Some(Format::Png));
        assert_eq!(
            picture_format(&[0xff, 0xd8, 0xff, 0xe0]),
            Some(Format::Jpeg)
        );
        assert_eq!(picture_format(b"GIF89a...."), Some(Format::Gif));
        assert_eq!(picture_format(b"RIFF\0\0\0\0WEBPVP8 "), Some(Format::Webp));
        assert_eq!(
            picture_format(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
            Some(Format::Svg)
        );
        // A web page, even one with an inline icon; a 3D model; text.
        assert_eq!(
            picture_format(b"<!DOCTYPE html><html><body><svg></svg></body></html>"),
            None
        );
        assert_eq!(picture_format(b"glTF\x02\0\0\0...."), None);
        assert_eq!(picture_format(b"Not Found"), None);
    }

    #[test]
    fn a_burst_of_profiles_does_not_start_a_thread_each() {
        let avatars = Avatars::default();
        avatars.lock().running = MAX_DOWNLOADS;
        assert_eq!(
            avatars.want("id", Some("https://127.0.0.1/a.png"), None, || {}),
            None
        );
        // Not marked as tried: the next snapshot asks again once a download is done.
        assert!(avatars.lock().tried.is_empty());
        avatars.lock().running = 0;
        avatars.want("id", Some("https://127.0.0.1/a.png"), None, || {});
        assert_eq!(avatars.lock().tried.len(), 1);
        // The download (refused: a private address) gives its place back.
        for _ in 0..100 {
            if avatars.lock().running == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(avatars.lock().running, 0);
    }

    #[test]
    fn a_new_picture_is_a_new_key() {
        let a = key("id", "https://x/a.png", Some(&[1; 32]));
        assert_ne!(a, key("id", "https://x/a.png", Some(&[2; 32])));
        assert_ne!(a, key("id", "https://x/b.png", Some(&[1; 32])));
        assert!(a.starts_with("avatar:id:"));
    }
}
