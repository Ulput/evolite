//! The token search's pace (FEAT-0019 R-11): at most one request a second to a node, the nodes
//! in turn, and a node that answers «too many requests» (429) left alone until its
//! `ratelimit-reset` — never marked bad, as `rs-dapi-client` would ban it (BUG-78). Pure: time
//! is milliseconds the caller gives.

/// The least time between two requests to one node: a node allows 150 a minute
/// (`ratelimit-limit`), so one a second leaves it plenty.
pub const GAP_MS: u64 = 1_000;

/// What to do now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Turn<N> {
    /// Ask this node now.
    Ask(N),
    /// No node may be asked yet: wait this long.
    Wait(u64),
    /// No nodes to ask.
    NoNodes,
}

struct Node<N> {
    id: N,
    last: Option<u64>,
    paused_until: u64,
}

/// The nodes the search asks, in turn.
pub struct Pacer<N> {
    nodes: Vec<Node<N>>,
    next: usize,
    gap_ms: u64,
}

impl<N: Clone + PartialEq> Pacer<N> {
    /// `nodes`: two or three evonodes of the masternode list.
    pub fn new(nodes: Vec<N>, gap_ms: u64) -> Self {
        Pacer {
            nodes: nodes
                .into_iter()
                .map(|id| Node {
                    id,
                    last: None,
                    paused_until: 0,
                })
                .collect(),
            next: 0,
            gap_ms,
        }
    }

    /// When `node` may be asked next.
    fn free_at(&self, node: &Node<N>) -> u64 {
        let after_gap = node.last.map_or(0, |l| l.saturating_add(self.gap_ms));
        after_gap.max(node.paused_until)
    }

    /// The node to ask at `now`, the next one in turn that may be asked; it counts as asked.
    pub fn turn(&mut self, now: u64) -> Turn<N> {
        let n = self.nodes.len();
        if n == 0 {
            return Turn::NoNodes;
        }
        for k in 0..n {
            let i = (self.next + k) % n;
            if self.free_at(&self.nodes[i]) <= now {
                self.nodes[i].last = Some(now);
                self.next = (i + 1) % n;
                return Turn::Ask(self.nodes[i].id.clone());
            }
        }
        let first = self.nodes.iter().map(|node| self.free_at(node)).min();
        Turn::Wait(first.unwrap_or(now).saturating_sub(now))
    }

    /// `node` answered «too many requests»: not asked again before `until` (its
    /// `ratelimit-reset`). It stays among the nodes.
    pub fn slow_down(&mut self, node: &N, until: u64) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == *node) {
            n.paused_until = n.paused_until.max(until);
        }
    }

    /// Until when every node is paused, if every one is (the screen says «paused»).
    pub fn all_paused_until(&self, now: u64) -> Option<u64> {
        if self.nodes.is_empty() || self.nodes.iter().any(|n| n.paused_until <= now) {
            return None;
        }
        self.nodes.iter().map(|n| n.paused_until).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_are_asked_in_turn_at_most_once_a_second_each() {
        let mut p = Pacer::new(vec!["a", "b", "c"], GAP_MS);
        assert_eq!(p.turn(0), Turn::Ask("a"));
        assert_eq!(p.turn(0), Turn::Ask("b"));
        assert_eq!(p.turn(10), Turn::Ask("c"));
        // Each was asked within the second: «a» is free at 1000.
        assert_eq!(p.turn(20), Turn::Wait(980));
        assert_eq!(p.turn(1_000), Turn::Ask("a"));
        assert_eq!(p.turn(1_000), Turn::Ask("b"));
        // Over a minute: never more than 60 asks to one node.
        let mut p = Pacer::new(vec!["a", "b"], GAP_MS);
        let mut asks = std::collections::HashMap::new();
        for now in (0..60_000).step_by(50) {
            if let Turn::Ask(n) = p.turn(now) {
                *asks.entry(n).or_insert(0) += 1;
            }
        }
        assert_eq!(asks.get("a"), Some(&60));
        assert_eq!(asks.get("b"), Some(&60));
    }

    #[test]
    fn too_many_requests_pauses_the_node_and_keeps_it() {
        let mut p = Pacer::new(vec!["a", "b"], GAP_MS);
        assert_eq!(p.turn(0), Turn::Ask("a"));
        p.slow_down(&"a", 30_000);
        assert_eq!(p.turn(0), Turn::Ask("b"));
        // «a» is skipped while paused, «b» goes on.
        assert_eq!(p.turn(1_000), Turn::Ask("b"));
        assert_eq!(p.turn(2_000), Turn::Ask("b"));
        assert_eq!(p.all_paused_until(2_000), None);
        // After the reset «a» is asked again: not banned.
        assert_eq!(p.turn(30_000), Turn::Ask("a"));
    }

    #[test]
    fn all_nodes_paused_waits_for_the_first_reset() {
        let mut p = Pacer::new(vec!["a", "b"], GAP_MS);
        p.slow_down(&"a", 40_000);
        p.slow_down(&"b", 25_000);
        assert_eq!(p.all_paused_until(0), Some(25_000));
        assert_eq!(p.turn(0), Turn::Wait(25_000));
        assert_eq!(p.turn(25_000), Turn::Ask("b"));
        assert_eq!(p.all_paused_until(25_000), None);
        assert_eq!(
            Pacer::<&str>::new(Vec::new(), GAP_MS).turn(0),
            Turn::NoNodes
        );
    }
}
