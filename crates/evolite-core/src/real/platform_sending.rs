//! Sends on Platform of this run — credits and the pool's — from "Sign" until the history has
//! them (BUG-12). Platform answers in seconds for credits and in tens of seconds for the pool
//! (the proof is built first); until then the history showed nothing, and a payment that is not
//! there gets made twice. A send shows as "being sent" at once and goes when its own line comes:
//! the journal's for credits, the pool's for the pool. Kept in memory: after a restart the
//! history shows what Platform took. The Core payments' counterpart is [`super::outgoing`].

use super::journal::What;
use evolite_api::{Activity, ActivityKind, Chain, Duffs, Proof, Timestamp};

/// How long a send Platform took may wait for its line before it goes anyway: the journal's
/// line comes with the answer, the pool's with the next read of the pool's store.
const LINE_WAIT: Timestamp = 5 * 60;

/// A send Platform has not answered, or whose line is not in the history yet.
struct Pending {
    key: u64,
    /// The line it shows as, and the line that takes its place has.
    kind: ActivityKind,
    amount: Duffs,
    at: Timestamp,
    /// When Platform took it; `None` while it has not answered.
    taken: Option<Timestamp>,
}

#[derive(Default)]
pub struct PlatformSending {
    next: u64,
    pending: Vec<Pending>,
    /// The lines that took a send's place: never another one's.
    used: std::collections::HashSet<u64>,
}

impl PlatformSending {
    /// A send of `amount` that the history will show as `kind` (`CreditTransfer`, `Withdrawal`
    /// or `PoolSent`) is being signed: it shows at once. The key is for [`Self::answered`].
    pub fn start(&mut self, kind: ActivityKind, amount: Duffs, at: Timestamp) -> u64 {
        self.next += 1;
        self.pending.push(Pending {
            key: self.next,
            kind,
            amount,
            at,
            taken: None,
        });
        self.next
    }

    /// Platform answered: refused (`taken` false) — gone at once, nothing left the wallet; taken,
    /// or maybe taken — shown until its line is in the history.
    pub fn answered(&mut self, key: u64, taken: bool, at: Timestamp) {
        if taken {
            if let Some(p) = self.pending.iter_mut().find(|p| p.key == key) {
                p.taken = Some(at);
            }
        } else {
            self.pending.retain(|p| p.key != key);
        }
    }

    /// The history with the sends it does not show yet added. A send is shown until a line of
    /// its kind made since it was signed is there (each line stands for one send, in this
    /// snapshot and every later one; the pool's store has its own "being sent" line once the
    /// proof is built), or — taken by Platform — [`LINE_WAIT`] after the answer with no such line.
    pub fn apply(&mut self, mut history: Vec<Activity>, now: Timestamp) -> Vec<Activity> {
        let used = &mut self.used;
        self.pending.retain(|p| {
            let line = history.iter().position(|a| {
                a.chain == Chain::Platform
                    && a.at >= p.at
                    && same_kind(&p.kind, &a.kind)
                    && !used.contains(&a.id)
            });
            if let Some(i) = line {
                used.insert(history[i].id);
                return false;
            }
            p.taken.is_none_or(|t| now < t + LINE_WAIT)
        });
        history.extend(self.pending.iter().map(|p| Activity {
            // Below this run's Core payments' ids (`u64::MAX - key`).
            id: u64::MAX - (1 << 32) - p.key,
            kind: p.kind.clone(),
            chain: Chain::Platform,
            at: p.at,
            // Platform's fee is known once it answers, with the line.
            amount: -p.amount,
            proof: Proof::Sending,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers: crate::layers::of(&p.kind, Chain::Platform).0,
            flow: crate::layers::of(&p.kind, Chain::Platform).1,
            key: String::new(),
            note: None,
        }));
        history
    }
}

/// The line a send of credits becomes, as [`super::journal::activity`] shows its entry.
pub fn credits_line(what: &What) -> ActivityKind {
    match what {
        // No status before Platform's document is read (BUG-58).
        What::Withdrawal { to, .. } => ActivityKind::Withdrawal {
            to: to.clone(),
            status: None,
        },
        What::Credits { to, .. } => ActivityKind::CreditTransfer { to: to.clone() },
        // Not a send; never asked for.
        other => {
            debug_assert!(false, "not a send of credits: {other:?}");
            ActivityKind::CreditTransfer { to: String::new() }
        }
    }
}

/// Whether a line of the history (`line`) is the one a send shown as `sent` becomes. The pool
/// names no payee: any send out of it; one to the wallet's own private address is a self
/// transfer there. Credits by the payee alone: the line may say more (a withdrawal's status).
fn same_kind(sent: &ActivityKind, line: &ActivityKind) -> bool {
    use ActivityKind as K;
    match (sent, line) {
        (K::PoolSent, K::PoolSent | K::SelfTransfer { .. }) => true,
        (K::CreditTransfer { to: a, .. }, K::CreditTransfer { to: b, .. }) => a == b,
        (K::Withdrawal { to: a, .. }, K::Withdrawal { to: b, .. }) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: u64, kind: ActivityKind, at: Timestamp) -> Activity {
        Activity {
            id,
            kind,
            chain: Chain::Platform,
            at,
            amount: -10_500,
            proof: Proof::Proved,
            txid: None,
            fee: Some(500),
            block: None,
            address: None,
            layers: evolite_api::layer::IDENTITY,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            note: None,
        }
    }

    fn sending(shown: &[Activity]) -> Vec<&Activity> {
        shown.iter().filter(|a| a.proof == Proof::Sending).collect()
    }

    fn credits(to: &str) -> ActivityKind {
        ActivityKind::CreditTransfer { to: to.into() }
    }

    #[test]
    fn credits_show_at_once_and_until_the_journal_has_them() {
        let mut out = PlatformSending::default();
        let key = out.start(credits("@bob"), 10_000, 1_000);
        let shown = out.apply(Vec::new(), 1_000);
        assert_eq!(sending(&shown).len(), 1, "shown while Platform answers");
        assert_eq!(shown[0].kind, credits("@bob"));
        assert_eq!(
            (shown[0].amount, shown[0].chain),
            (-10_000, Chain::Platform)
        );

        // An older line to the same payee, and one to another: not this send.
        let old = vec![
            line(1, credits("@bob"), 999),
            line(2, credits("@eve"), 1_003),
        ];
        assert_eq!(sending(&out.apply(old.clone(), 1_003)).len(), 1);

        // Taken: still shown until the line is there (a snapshot between the two).
        out.answered(key, true, 1_004);
        assert_eq!(sending(&out.apply(old.clone(), 1_004)).len(), 1);
        let mut now = old;
        now.push(line(3, credits("@bob"), 1_004));
        let shown = out.apply(now.clone(), 1_004);
        assert!(sending(&shown).is_empty(), "the journal has it");
        assert_eq!(shown.len(), 3, "and only once");
        assert!(sending(&out.apply(now, 1_005)).is_empty(), "gone for good");
    }

    #[test]
    fn a_refused_send_is_gone_and_a_taken_one_without_a_line_goes_later() {
        let mut out = PlatformSending::default();
        let key = out.start(credits("yXaddress"), 10_000, 1_000);
        out.answered(key, false, 1_002);
        assert!(out.apply(Vec::new(), 1_002).is_empty());

        let key = out.start(ActivityKind::PoolSent, 10_000, 2_000);
        out.answered(key, true, 2_030);
        assert_eq!(
            sending(&out.apply(Vec::new(), 2_030 + LINE_WAIT - 1)).len(),
            1
        );
        assert!(out.apply(Vec::new(), 2_030 + LINE_WAIT).is_empty());
    }

    #[test]
    fn each_line_takes_the_place_of_one_send() {
        let mut out = PlatformSending::default();
        out.start(ActivityKind::PoolSent, 10_000, 1_000);
        out.start(ActivityKind::PoolSent, 20_000, 1_001);
        // The pool's store has the first one: its own "being sent" line, not two.
        let mut pool = line(7, ActivityKind::PoolSent, 1_020);
        pool.proof = Proof::Sending;
        let shown = out.apply(vec![pool.clone()], 1_020);
        assert_eq!(shown.len(), 2, "the store's line and the second send");
        assert_eq!(shown[1].amount, -20_000);
        pool.proof = Proof::Proved;
        let shown = out.apply(vec![pool], 1_040);
        assert_eq!(sending(&shown).len(), 1, "one line, one send");
        let own = line(8, ActivityKind::SelfTransfer { moved: 0 }, 1_041);
        assert!(sending(&out.apply(vec![own], 1_041)).is_empty());
    }
}
