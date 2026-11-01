//! Core payments of this run, from "Sign" until the wallet's history shows them. The
//! history knows a transaction only by its amounts, so who it went to is kept here; and a
//! payment the network has not echoed back yet, or answered unclearly, stays in the list as
//! "sending" instead of vanishing — so nobody pays twice because the first one is not there.
//! Kept in memory: after a restart the history shows what the network took.

use evolite_api::{Activity, ActivityKind, Chain, Duffs, Proof, Timestamp};
use std::collections::HashMap;

/// How far a block's time may be from ours (a block's timestamp is the miner's).
const CLOCK_SLACK: Timestamp = 2 * 60 * 60;

/// A payment handed to the network, not in the history yet.
struct Pending {
    key: u64,
    to: String,
    amount: Duffs,
    /// What the network took when it answered, otherwise the most the fee can be.
    fee: Duffs,
    at: Timestamp,
    /// Known once the network has answered; a contact payment with an unclear answer has none.
    txid: Option<String>,
}

/// What the network said about a payment.
pub enum Answer {
    /// Taken, or maybe taken: the txid when known, and the fee.
    Sent { txid: Option<String>, fee: Duffs },
    /// Refused before anything left the wallet.
    Refused,
}

#[derive(Default)]
pub struct Outgoing {
    next: u64,
    pending: Vec<Pending>,
    /// Who each payment of this run went to, by txid.
    payees: HashMap<String, String>,
}

impl Outgoing {
    /// A payment to `to` is being signed: it shows at once. The key is for [`Self::answered`].
    pub fn start(&mut self, to: &str, amount: Duffs, fee_max: Duffs, at: Timestamp) -> u64 {
        self.next += 1;
        self.pending.push(Pending {
            key: self.next,
            to: to.trim().to_owned(),
            amount,
            fee: fee_max,
            at,
            txid: None,
        });
        self.next
    }

    pub fn answered(&mut self, key: u64, answer: Answer) {
        let Some(i) = self.pending.iter().position(|p| p.key == key) else {
            return;
        };
        match answer {
            Answer::Refused => {
                self.pending.remove(i);
            }
            Answer::Sent { txid, fee } => {
                let p = &mut self.pending[i];
                if fee > 0 {
                    p.fee = fee;
                }
                if let Some(txid) = txid.filter(|t| !t.is_empty()) {
                    self.payees.insert(txid.clone(), p.to.clone());
                    p.txid = Some(txid);
                }
            }
        }
    }

    /// The history (txid and operation, as the wallet has them) with the payee of this run's
    /// payments named and the payments it does not show yet added. A payment it shows is no
    /// longer pending: by its txid, or — when the network's answer had none — by an outgoing
    /// operation of that amount (with its fee) since it was signed.
    pub fn apply(&mut self, mut history: Vec<(String, Activity)>) -> Vec<Activity> {
        let mut found = Vec::new();
        for p in &self.pending {
            if let Some(txid) = &p.txid {
                if history.iter().any(|(t, _)| t == txid) {
                    found.push(p.key);
                }
                continue;
            }
            let matched = history.iter().find(|(t, a)| {
                matches!(a.kind, ActivityKind::Sent { .. })
                    && !self.payees.contains_key(t)
                    && a.at >= p.at - CLOCK_SLACK
                    && (p.amount..=p.amount + p.fee).contains(&-a.amount)
            });
            if let Some((txid, _)) = matched {
                self.payees.insert(txid.clone(), p.to.clone());
                found.push(p.key);
            }
        }
        self.pending.retain(|p| !found.contains(&p.key));
        for (txid, act) in &mut history {
            // The history may already name an address or a contact; a typed @name says more.
            if let (ActivityKind::Sent { to }, Some(payee)) = (&mut act.kind, self.payees.get(txid))
                && (to.is_empty() || payee.starts_with('@'))
            {
                *to = payee.clone();
            }
        }
        let mut shown: Vec<Activity> = history.into_iter().map(|(_, a)| a).collect();
        shown.extend(self.pending.iter().map(|p| Activity {
            // Out of the range of txid-based ids.
            id: u64::MAX - p.key,
            kind: ActivityKind::Sent { to: p.to.clone() },
            chain: Chain::Core,
            at: p.at,
            amount: -(p.amount + p.fee),
            proof: Proof::Sending,
            txid: p.txid.clone(),
            // The most it can be until the network has answered.
            fee: Some(p.fee),
            block: None,
            address: None,
        }));
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sent(at: Timestamp, amount: Duffs) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Sent { to: String::new() },
            chain: Chain::Core,
            at,
            amount,
            proof: Proof::Mempool,
            txid: None,
            fee: None,
            block: None,
            address: None,
        }
    }

    fn sending(shown: &[Activity]) -> Vec<&Activity> {
        shown.iter().filter(|a| a.proof == Proof::Sending).collect()
    }

    #[test]
    fn a_payment_shows_until_the_history_has_it() {
        let mut out = Outgoing::default();
        let key = out.start(" @bob ", 100_000, 500, 1_000);
        let shown = out.apply(Vec::new());
        assert_eq!(sending(&shown).len(), 1, "shown while signing");
        assert_eq!(shown[0].kind, ActivityKind::Sent { to: "@bob".into() });
        assert_eq!(shown[0].amount, -100_500);

        out.answered(
            key,
            Answer::Sent {
                txid: Some("aa".into()),
                fee: 226,
            },
        );
        let shown = out.apply(vec![("bb".into(), sent(1_001, -5))]);
        assert_eq!(sending(&shown)[0].amount, -100_226, "with the fee it took");

        let shown = out.apply(vec![("aa".into(), sent(1_002, -100_226))]);
        assert!(sending(&shown).is_empty(), "the history has it now");
        assert_eq!(
            shown[0].kind,
            ActivityKind::Sent { to: "@bob".into() },
            "and it says to whom"
        );
    }

    #[test]
    fn a_refused_payment_is_forgotten() {
        let mut out = Outgoing::default();
        let key = out.start("yX", 100_000, 500, 1_000);
        out.answered(key, Answer::Refused);
        assert!(out.apply(Vec::new()).is_empty());
    }

    #[test]
    fn an_unclear_answer_stays_until_a_payment_of_that_amount_shows() {
        let mut out = Outgoing::default();
        let key = out.start("@bob", 100_000, 500, 1_000);
        out.answered(key, Answer::Sent { txid: None, fee: 0 });
        // Another amount, or before it was signed: not this payment.
        let shown = out.apply(vec![
            ("cc".into(), sent(1_005, -90_000)),
            ("dd".into(), sent(1_000 - CLOCK_SLACK - 1, -100_300)),
        ]);
        assert_eq!(sending(&shown).len(), 1);

        let shown = out.apply(vec![("ee".into(), sent(1_010, -100_300))]);
        assert!(sending(&shown).is_empty());
        assert_eq!(shown[0].kind, ActivityKind::Sent { to: "@bob".into() });
    }
}
