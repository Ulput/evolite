//! BUG-328: whether a line of «Paste a list» names a Core address of this network, before it
//! becomes a row — base58check (two SHA-256), 25 bytes, the version of a pay-to-key or
//! pay-to-script address of `network`. The core still parses every recipient at «Check the
//! payment» ([`evolite_api::Command::PreviewPayment`]); this only keeps a line that is no address
//! out of the rows, said by its number (FEAT-0056 UC-3).

use evolite_api::Network;
use sha2::{Digest, Sha256};

/// What a typed Core address is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CoreAddress {
    /// A Core address of this network.
    Here,
    /// A Core address of the other network.
    Other(Network),
    /// Base58 of an address's length whose checksum does not match: a typo.
    Checksum,
    /// Not a Core address at all.
    Not,
}

const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Dash's version bytes: pay-to-key and pay-to-script.
fn versions(network: Network) -> [u8; 2] {
    match network {
        Network::Mainnet => [76, 16],
        Network::Testnet => [140, 19],
    }
}

/// The bytes of base58 `text`; `None` for a character outside the alphabet.
fn decode(text: &str) -> Option<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::new();
    for c in text.bytes() {
        let mut carry = ALPHABET.iter().position(|a| *a == c)? as u32;
        for b in bytes.iter_mut().rev() {
            carry += u32::from(*b) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let zeros = text.bytes().take_while(|c| *c == b'1').count();
    let mut out = vec![0; zeros];
    out.extend(bytes);
    Some(out)
}

/// What `text`, trimmed, is as a Core address on `network`.
pub(crate) fn check(text: &str, network: Network) -> CoreAddress {
    let Some(bytes) = decode(text.trim()) else {
        return CoreAddress::Not;
    };
    if bytes.len() != 25 {
        return CoreAddress::Not;
    }
    let (body, sum) = bytes.split_at(21);
    let twice = Sha256::digest(Sha256::digest(body));
    if twice[..4] != *sum {
        return CoreAddress::Checksum;
    }
    let other = match network {
        Network::Mainnet => Network::Testnet,
        Network::Testnet => Network::Mainnet,
    };
    if versions(network).contains(&body[0]) {
        CoreAddress::Here
    } else if versions(other).contains(&body[0]) {
        CoreAddress::Other(other)
    } else {
        CoreAddress::Not
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encodes `version` + `hash` as an address (the test's own, to build ones of each kind).
    fn address(version: u8, hash: [u8; 20]) -> String {
        let mut body = vec![version];
        body.extend(hash);
        let twice = Sha256::digest(Sha256::digest(&body));
        body.extend(&twice[..4]);
        let mut digits: Vec<u8> = Vec::new();
        for b in &body {
            let mut carry = u32::from(*b);
            for d in digits.iter_mut().rev() {
                carry += u32::from(*d) << 8;
                *d = (carry % 58) as u8;
                carry /= 58;
            }
            while carry > 0 {
                digits.insert(0, (carry % 58) as u8);
                carry /= 58;
            }
        }
        let zeros = body.iter().take_while(|b| **b == 0).count();
        std::iter::repeat_n('1', zeros)
            .chain(digits.iter().map(|d| ALPHABET[*d as usize] as char))
            .collect()
    }

    /// BUG-328 (b): testnet's and mainnet's pay-to-key and pay-to-script addresses, each on its
    /// network and the other; a typo in the checksum; words, a Platform address, a bare id.
    /// Mutant: the checksum not compared → the typo is «here», red.
    #[test]
    fn a_core_address_is_known_by_its_network_and_its_checksum() {
        // The QA wallet's own (BUG-328's screenshots).
        let qa = "yMEwQ1PLRtUprLAKxJgYnWsGRmcSAtBdr7";
        assert_eq!(check(qa, Network::Testnet), CoreAddress::Here);
        assert_eq!(
            check(qa, Network::Mainnet),
            CoreAddress::Other(Network::Testnet)
        );
        for version in [140, 19] {
            assert_eq!(
                check(&address(version, [7; 20]), Network::Testnet),
                CoreAddress::Here
            );
        }
        for version in [76, 16] {
            let a = address(version, [7; 20]);
            assert_eq!(check(&a, Network::Mainnet), CoreAddress::Here);
            assert_eq!(
                check(&a, Network::Testnet),
                CoreAddress::Other(Network::Mainnet)
            );
        }
        // One character changed: base58 of the length, the checksum wrong.
        let typo: String = qa.replacen('W', "X", 1);
        assert_eq!(check(&typo, Network::Testnet), CoreAddress::Checksum);
        for not in [
            "not-an-address",
            "",
            "tdash1kz0xs3dle5xwmq4mk8rh9pkcr8txmqmeyk3xg6",
            "@alice",
            "yMEwQ1PLRtUprLAKxJgYnWsG",
        ] {
            assert_eq!(check(not, Network::Testnet), CoreAddress::Not, "{not}");
        }
        // Bitcoin's version (0): base58check, not Dash.
        assert_eq!(
            check(&address(0, [7; 20]), Network::Testnet),
            CoreAddress::Not
        );
        assert_eq!(
            check(&format!("  {qa} "), Network::Testnet),
            CoreAddress::Here
        );
    }
}
