//! What a typed recipient is, by the network the app runs on (TECH-address-kind, BUG-211): the one
//! place that knows Platform's address prefix. Platform addresses and the pool's private (Orchard)
//! ones share it; which of the two an address is, and whether it is valid at all, is the core's to
//! say (it parses them) — the screens only need the kind to pick a fee, a route and their words.

use crate::Network;

/// The start of a Platform or private address on `network`: `tdash1` on testnet, `dash1` on
/// mainnet (bech32m's human-readable part and its separator).
pub fn hrp(network: Network) -> &'static str {
    match network {
        Network::Testnet => "tdash1",
        Network::Mainnet => "dash1",
    }
}

/// The other network: the one whose addresses are refused here.
fn other(network: Network) -> Network {
    match network {
        Network::Testnet => Network::Mainnet,
        Network::Mainnet => Network::Testnet,
    }
}

/// What a recipient typed or pasted is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressKind {
    /// `@name`: a DPNS name (the leading `@` of FEAT-0039's recipient rule).
    Name,
    /// This network's Platform or private address, in any case.
    Platform,
    /// A Platform or private address of the other network: it would take the money somewhere
    /// the person did not mean.
    WrongNetwork { found: Network },
    /// Anything else — a Core address, an identity id, or not an address at all: the core says.
    Other,
}

/// The kind of `text` on `network`, trimmed and in any case.
pub fn kind(text: &str, network: Network) -> AddressKind {
    let text = text.trim();
    if text.starts_with('@') {
        return AddressKind::Name;
    }
    let lower = text.to_ascii_lowercase();
    // Testnet's `tdash1` ends with mainnet's `dash1`: each is matched at the start only.
    if lower.starts_with(hrp(network)) {
        AddressKind::Platform
    } else if lower.starts_with(hrp(other(network))) {
        AddressKind::WrongNetwork {
            found: other(network),
        }
    } else {
        AddressKind::Other
    }
}

/// Whether `text` is written like a Platform or private address of either network: a field that
/// only asks «is this an address at all» (a QR code, a token's recipient, words to keep whole).
pub fn looks_platform(text: &str) -> bool {
    !matches!(
        kind(text, Network::Testnet),
        AddressKind::Name | AddressKind::Other
    )
}

/// Whether `text` is this network's Platform or private address.
pub fn is_platform(text: &str, network: Network) -> bool {
    kind(text, network) == AddressKind::Platform
}

#[cfg(test)]
mod tests {
    use super::*;
    use Network::{Mainnet, Testnet};

    const T: &str = "tdash1kz0xs3dle5xwmq4mk8rh9pkcr8txmqmeyk3xg6";
    const M: &str = "dash1kz0xs3dle5xwmq4mk8rh9pkcr8txmqmeyk3xg6";

    /// AC-T1: both networks, either case, the other network's, a name, a Core address, junk.
    #[test]
    fn the_kind_is_by_the_networks_own_prefix() {
        assert_eq!(kind(M, Mainnet), AddressKind::Platform);
        assert_eq!(kind(T, Testnet), AddressKind::Platform);
        assert_eq!(kind(&M.to_uppercase(), Mainnet), AddressKind::Platform);
        assert_eq!(kind(&format!("  {T}\n"), Testnet), AddressKind::Platform);
        assert_eq!(
            kind(T, Mainnet),
            AddressKind::WrongNetwork { found: Testnet }
        );
        assert_eq!(
            kind(&M.to_uppercase(), Testnet),
            AddressKind::WrongNetwork { found: Mainnet }
        );
        for network in [Testnet, Mainnet] {
            assert_eq!(kind("@alice", network), AddressKind::Name);
            assert_eq!(kind(" @Alice ", network), AddressKind::Name);
            assert_eq!(
                kind("yNk2Hc8pWq4mR7vT3sLb9XeJ6uZa5Df1Gh", network),
                AddressKind::Other
            );
            assert_eq!(
                kind("XpESxaUmonkq8RaLLp46Brx2K39ggQe226", network),
                AddressKind::Other
            );
            assert_eq!(kind("", network), AddressKind::Other);
            assert_eq!(kind("hello", network), AddressKind::Other);
            // The fullwidth ＠ is not FEAT-0039's name sign.
            assert_eq!(kind("＠alice", network), AddressKind::Other);
        }
        assert!(is_platform(M, Mainnet) && !is_platform(M, Testnet));
        assert!(looks_platform(M) && looks_platform(T) && looks_platform(&T.to_uppercase()));
        assert!(!looks_platform("@alice") && !looks_platform("yNk2Hc8pWq4mR7vT3sLb9XeJ6uZa5Df1Gh"));
        assert!(is_platform(T, Testnet) && !is_platform(T, Mainnet));
    }

    /// AC-T2 (TECH-address-kind): outside this file and the tests, no source of the screens, the
    /// core, the chain or this API writes a Platform prefix — every check goes through [`kind`].
    /// Searched with the opening quote: `dash1` is the end of `tdash1`.
    #[test]
    fn no_other_source_writes_a_platform_prefix() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut found = Vec::new();
        for krate in ["evolite-ui", "evolite-core", "evolite-chain", "evolite-api"] {
            let mut dirs = vec![crates.join(krate).join("src")];
            while let Some(dir) = dirs.pop() {
                for entry in std::fs::read_dir(&dir).expect("a source folder").flatten() {
                    let path = entry.path();
                    let name = path.file_name().unwrap().to_string_lossy().into_owned();
                    if path.is_dir() {
                        if !matches!(name.as_str(), "tests" | "ui_tests") {
                            dirs.push(path);
                        }
                        continue;
                    }
                    let test_file = name == "tests.rs" || name.ends_with("_tests.rs");
                    if !name.ends_with(".rs") || test_file || path.ends_with("src/address.rs") {
                        continue;
                    }
                    let text = std::fs::read_to_string(&path).expect("a source file");
                    for (n, line) in code_lines(&text) {
                        if line.contains("\"tdash1") || line.contains("\"dash1") {
                            found.push(format!("{}:{n}", path.display()));
                        }
                    }
                }
            }
        }
        assert!(found.is_empty(), "use evolite_api::address: {found:#?}");
    }

    /// The lines of `text` before its first test module (`#[cfg(test)]` or `#[cfg(all(test, …))]`
    /// over a `mod`), without comments, numbered from 1.
    fn code_lines(text: &str) -> Vec<(usize, &str)> {
        let lines: Vec<&str> = text.lines().collect();
        let mut out = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            let test_attr = t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test");
            let next_is_mod = lines[i + 1..]
                .iter()
                .map(|l| l.trim_start())
                .find(|l| !l.starts_with("#["))
                .is_some_and(|l| {
                    l.starts_with("mod ") || (l.starts_with("pub") && l.contains(" mod "))
                });
            if test_attr && next_is_mod {
                break;
            }
            if !t.starts_with("//") {
                out.push((i + 1, *line));
            }
        }
        out
    }

    #[test]
    fn each_network_has_its_prefix() {
        assert_eq!(hrp(Testnet), "tdash1");
        assert_eq!(hrp(Mainnet), "dash1");
    }
}
