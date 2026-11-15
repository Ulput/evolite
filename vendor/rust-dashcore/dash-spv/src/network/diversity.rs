//! evolite (SPV item 2b): outbound peers spread over subnets, so one operator with one block
//! of addresses can't fill all our connections (an eclipse). At most
//! [`MAX_PEERS_PER_SUBNET`] peers, connected or being connected, share a /16 (IPv4) or a /32
//! (IPv6). Peers the user configured themselves are not held to it.

use std::net::{IpAddr, SocketAddr};

/// The most outbound peers one subnet may give us.
pub const MAX_PEERS_PER_SUBNET: usize = 2;

/// The subnet a peer's address belongs to: the first two bytes of an IPv4 address (/16), the
/// first four of an IPv6 one (/32). An IPv4-mapped IPv6 address counts as its IPv4 address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subnet {
    V4([u8; 2]),
    V6([u8; 4]),
}

pub fn subnet(addr: &SocketAddr) -> Subnet {
    match addr.ip() {
        IpAddr::V4(ip) => {
            let [a, b, ..] = ip.octets();
            Subnet::V4([a, b])
        }
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(ip) => {
                let [a, b, ..] = ip.octets();
                Subnet::V4([a, b])
            }
            None => {
                let [a, b, c, d, ..] = ip.octets();
                Subnet::V6([a, b, c, d])
            }
        },
    }
}

/// Whether `addr`'s subnet already gives us [`MAX_PEERS_PER_SUBNET`] of `taken` (the peers
/// connected and being connected). `addr` itself among them doesn't count.
pub fn subnet_full<'a>(addr: &SocketAddr, taken: impl IntoIterator<Item = &'a SocketAddr>) -> bool {
    let own = subnet(addr);
    taken.into_iter().filter(|other| *other != addr && subnet(other) == own).count()
        >= MAX_PEERS_PER_SUBNET
}

/// Of `candidates`, in their order, the ones that may be connected next to `taken`: none from
/// a subnet already full, and at most `wanted` in all.
pub fn pick_diverse(
    candidates: &[SocketAddr],
    taken: &[SocketAddr],
    wanted: usize,
) -> Vec<SocketAddr> {
    let mut chosen: Vec<SocketAddr> = Vec::new();
    for addr in candidates {
        if chosen.len() >= wanted {
            break;
        }
        if taken.contains(addr) || chosen.contains(addr) {
            continue;
        }
        if !subnet_full(addr, taken.iter().chain(chosen.iter())) {
            chosen.push(*addr);
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn a_subnet_is_a_slash_16_or_a_slash_32() {
        assert_eq!(subnet(&addr("1.2.3.4:9999")), subnet(&addr("1.2.200.7:19999")));
        assert_ne!(subnet(&addr("1.2.3.4:9999")), subnet(&addr("1.3.3.4:9999")));
        assert_eq!(subnet(&addr("[2001:db8:1::1]:9999")), subnet(&addr("[2001:db8:ffff::2]:9999")));
        assert_ne!(subnet(&addr("[2001:db8::1]:9999")), subnet(&addr("[2001:db9::1]:9999")));
        // An IPv4-mapped address is its IPv4 address.
        assert_eq!(subnet(&addr("[::ffff:1.2.9.9]:9999")), subnet(&addr("1.2.3.4:9999")));
    }

    /// AC-T2б.2: from a list where five share a /16, at most two are connected.
    #[test]
    fn of_five_in_one_subnet_two_are_picked() {
        let candidates: Vec<SocketAddr> = [
            "5.9.1.1:9999",
            "5.9.1.2:9999",
            "5.9.2.3:9999",
            "5.9.3.4:9999",
            "5.9.4.5:9999",
            "8.8.1.1:9999",
            "[2a01:4f8:1::1]:9999",
            "[2a01:4f8:2::1]:9999",
            "[2a01:4f8:3::1]:9999",
        ]
        .iter()
        .map(|a| addr(a))
        .collect();

        let picked = pick_diverse(&candidates, &[], 8);

        assert_eq!(
            picked,
            vec![
                addr("5.9.1.1:9999"),
                addr("5.9.1.2:9999"),
                addr("8.8.1.1:9999"),
                addr("[2a01:4f8:1::1]:9999"),
                addr("[2a01:4f8:2::1]:9999"),
            ]
        );
    }

    /// The pool's gate, which every outbound connect goes through: of five addresses in one
    /// /16, two are marked; one from elsewhere still is; a peer that leaves frees its place.
    #[tokio::test]
    async fn the_pool_lets_two_peers_of_a_subnet_connect() {
        let pool = crate::network::pool::PeerPool::new(8);
        let mut marked = Vec::new();
        for last in 1..=5 {
            let candidate = addr(&format!("5.9.{last}.1:9999"));
            if pool.mark_connecting_within_subnets(candidate).await {
                marked.push(candidate);
            }
        }
        assert_eq!(marked, vec![addr("5.9.1.1:9999"), addr("5.9.2.1:9999")]);
        assert!(pool.mark_connecting_within_subnets(addr("8.8.8.8:9999")).await);
        assert!(!pool.mark_connecting_within_subnets(addr("5.9.1.1:9999")).await, "already");

        pool.remove_peer(&addr("5.9.1.1:9999")).await;
        assert!(pool.mark_connecting_within_subnets(addr("5.9.3.1:9999")).await);
    }

    #[test]
    fn peers_already_taken_fill_their_subnet() {
        let taken = [addr("5.9.1.1:9999"), addr("5.9.1.2:9999")];
        let candidates = [addr("5.9.7.7:9999"), addr("8.8.8.8:9999"), addr("8.8.4.4:9999")];

        assert_eq!(pick_diverse(&candidates, &taken, 1), vec![addr("8.8.8.8:9999")]);
        assert!(subnet_full(&addr("5.9.7.7:9999"), &taken));
        assert!(!subnet_full(&addr("5.9.1.1:9999"), &taken), "itself doesn't count");
    }
}
