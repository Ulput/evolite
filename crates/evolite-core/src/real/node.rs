//! Settings → "Own node": Core's only peer, like Dash Core's `-connect`. dash-spv takes
//! the configured peers and then connects to nobody else (its "exclusive mode"): no DNS seeds,
//! no addresses from other nodes. Headers, ChainLocks and InstantLocks are checked as always —
//! the node is trusted with privacy, not with the chain. Platform (DAPI) keeps its own nodes.
//!
//! The choice is a line in `core-node.txt` in the data folder, next to `settings.json`, and is
//! read when the chain starts: like Dash Core, a new node takes effect at the next start.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

/// Dash Core's P2P port on the network of this process: 19999 on testnet, 9999 on mainnet.
pub fn default_port() -> u16 {
    match crate::network::current() {
        evolite_api::Network::Testnet => 19999,
        evolite_api::Network::Mainnet => 9999,
    }
}
const RESOLVE_FOR: Duration = Duration::from_secs(10);

/// What was typed, as it is kept: `host:port`, the network's port added when none is given.
/// Only the shape is checked here; [`resolve`] finds the node.
pub fn normalize(input: &str) -> Result<String, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err(
            tr!("Enter the node's address: an IP or a name, with a port if you like").into(),
        );
    }
    if s.contains("://") || s.contains('/') {
        return Err(tr!("A node address is needed, e.g. 192.168.1.10, not a link").into());
    }
    // Words are not a link: "not a node" is told what an address looks like (BUG-39 №11).
    if s.chars().any(char::is_whitespace) {
        return Err(tr!(
            "A node address is one word without spaces, e.g. 192.168.1.10 or node.example.org"
        )
        .into());
    }
    // An IPv6 address alone, without brackets and port.
    if s.parse::<std::net::Ipv6Addr>().is_ok() {
        return Ok(format!("[{s}]:{}", default_port()));
    }
    let (host, port) = match s.rsplit_once(':') {
        Some((host, port)) if !host.ends_with(':') => (host, Some(port)),
        _ => (s, None),
    };
    let port = match port {
        None => default_port(),
        Some(p) => match p.parse::<u16>() {
            Ok(p) if p > 0 => p,
            _ => {
                return Err(format!(
                    tr!("Port «{p}» is not a number from 1 to 65535"),
                    p = p
                ));
            }
        },
    };
    if host.is_empty() {
        return Err(tr!("A node address is needed before the port").into());
    }
    Ok(format!("{host}:{port}"))
}

/// The node's address on the network; a name is looked up (for at most ten seconds).
pub async fn resolve(address: &str) -> Result<SocketAddr, String> {
    if let Ok(addr) = address.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let found = tokio::time::timeout(RESOLVE_FOR, tokio::net::lookup_host(address.to_owned()))
        .await
        .map_err(|_| {
            format!(
                tr!("Node {address} was not found within 10 seconds"),
                address = address
            )
        })?
        .map_err(|_| {
            format!(
                tr!("Node {address} was not found: check the name"),
                address = address
            )
        })?
        .next();
    found.ok_or_else(|| {
        format!(
            tr!("Node {address} was not found: check the name"),
            address = address
        )
    })
}

/// The node saved for the chain's start, if any.
pub fn load(path: &Path) -> Option<String> {
    let text = String::from_utf8(super::json_file::read(path).ok()?).ok()?;
    normalize(text.lines().next()?).ok()
}

/// Keeps the node for the next start; `None` goes back to DNS seeds.
pub fn save(path: &Path, address: Option<&str>) -> std::io::Result<()> {
    match address {
        // Flushed before the rename: after a power cut an empty file would read as "no node",
        // and the next start would go to DNS seeds the person chose not to use.
        Some(address) => super::json_file::write(path, format!("{address}\n").as_bytes()),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_the_testnet_port_and_keeps_a_given_one() {
        assert_eq!(normalize(" 192.168.1.10 ").unwrap(), "192.168.1.10:19999");
        assert_eq!(normalize("127.0.0.1:19998").unwrap(), "127.0.0.1:19998");
        assert_eq!(
            normalize("node.example.org").unwrap(),
            "node.example.org:19999"
        );
        assert_eq!(normalize("::1").unwrap(), "[::1]:19999");
        assert_eq!(normalize("[::1]:20001").unwrap(), "[::1]:20001");
    }

    #[test]
    fn refuses_what_is_not_a_node() {
        for bad in [
            "",
            "https://node.example.org",
            "1.2.3.4:0",
            "1.2.3.4:70000",
            "1.2.3.4:port",
            ":19999",
            "two words",
        ] {
            assert!(normalize(bad).is_err(), "{bad}");
        }
    }

    /// BUG-39 №11: only a link is called a link.
    #[test]
    fn words_are_not_called_a_link() {
        let words = normalize("not a node").unwrap_err();
        assert!(!words.contains("link"), "{words}");
        assert!(
            normalize("https://node.example.org")
                .unwrap_err()
                .contains("link")
        );
    }

    #[tokio::test]
    async fn an_address_resolves_without_the_network() {
        assert_eq!(
            resolve("127.0.0.1:19999").await.unwrap(),
            "127.0.0.1:19999".parse().unwrap()
        );
        assert!(resolve("localhost:19999").await.is_ok());
    }

    #[test]
    fn is_kept_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("evolite-node-{}", std::process::id()));
        let path = dir.join("core-node.txt");
        assert_eq!(load(&path), None);
        save(&path, Some("127.0.0.1:19999")).unwrap();
        assert_eq!(load(&path).as_deref(), Some("127.0.0.1:19999"));
        // Through the flushed write: nothing left beside it, a stale one written over.
        assert!(!dir.join("core-node.txt.tmp").exists());
        std::fs::write(dir.join("core-node.txt.tmp"), b"half").unwrap();
        save(&path, Some("127.0.0.1:19998")).unwrap();
        assert_eq!(load(&path).as_deref(), Some("127.0.0.1:19998"));
        assert!(!dir.join("core-node.txt.tmp").exists());
        save(&path, None).unwrap();
        save(&path, None).unwrap();
        assert_eq!(load(&path), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
