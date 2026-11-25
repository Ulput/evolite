//! FEAT-0020-resend: after an identity's DashPay pair was replaced, its contacts' requests to it
//! point to a disabled key of ours, and they cannot pay us until we send a new one. A batch sends
//! them, one at a time, by the one rule (`sent::run`); what it could not is owed
//! ([`ResendDebt`]), sent with «Send requests again» and a password.
//!
//! The architect's review: `docs/tasks/reviews/FEAT-0020-resend-scheme.md` (answers 1–3,
//! conditions 1–5).

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;

use evolite_chain::chain::{SentRequest, sent_with, versions_used_up};
use evolite_local::resend_debt::{Owed, ResendDebt, Row};

use super::identity::own_line;
use crate::real::*;

/// Where batches stand, for the snapshot, and the lock's mark.
#[derive(Default)]
pub(in crate::real) struct ResendRun {
    /// Bumped by the lock: a batch started before it stops between two contacts (R-7).
    run: AtomicU64,
    /// By identity: (done, of) while a batch runs, and of the last one that stopped.
    shown: Mutex<HashMap<String, Shown>>,
}

/// What the keys screen shows of an identity's batches (R-5, R-6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::real) struct Shown {
    pub(in crate::real) running: Option<(u32, u32)>,
    pub(in crate::real) done: Option<(u32, u32)>,
}

impl ResendRun {
    /// The wallet is locked: a batch running stops before its next contact.
    pub(in crate::real) fn lock(&self) {
        self.run.fetch_add(1, Ordering::Relaxed);
    }

    fn current(&self) -> u64 {
        self.run.load(Ordering::Relaxed)
    }

    fn set(&self, identity: &str, shown: Shown) {
        self.shown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(identity.to_owned(), shown);
    }

    pub(in crate::real) fn of(&self, identity: &str) -> Shown {
        self.shown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(identity)
            .copied()
            .unwrap_or_default() // default-ok: no batch for this identity: nothing to show
    }
}

/// What became of one request of a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Sent now (its own answer, or a proved read shows it).
    Sent,
    /// A request of ours with the key now was there already: nothing sent (AC-T4).
    Already,
    /// Not sent: owed, for the button.
    Due,
    /// Refused by the unique index at the last version of the reference: no new request can go
    /// to this contact (R-11).
    Blocked,
}

/// One paying identity's numbers in a batch: its History line and diag (review answer 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
    of: u32,
    sent: u32,
    already: u32,
    failed: u32,
    version_full: u32,
    unclear: u32,
    left: u32,
}

impl Tally {
    fn count(&mut self, step: Step) {
        match step {
            Step::Sent => self.sent += 1,
            Step::Already => self.already += 1,
            Step::Due => {
                self.failed += 1;
                self.left += 1;
            }
            Step::Blocked => self.version_full += 1,
        }
    }

    /// Its History line: what got a request now or had one, of how many, still owed.
    fn kind(&self) -> journal::Kind {
        journal::Kind::ContactsResent {
            sent: self.sent + self.already,
            of: self.of,
            left: self.left,
        }
    }
}

/// Whether a refusal is Platform's unique index (`DuplicateUniqueIndexError`, «… has duplicate
/// unique properties …»): our request to this contact with the same reference is there.
fn duplicate(e: &str) -> bool {
    e.contains("DuplicateUniqueIndex") || e.to_lowercase().contains("duplicate unique")
}

/// A unique-index refusal, by what a proved read of our requests to the contact shows (review
/// condition 2): one with our key now → `Already` (a request of an unclear send that landed,
/// whose reference platform-wallet computed again); else the last version used up → `Blocked`
/// (R-11); else, or the read failed → `Due`.
fn after_duplicate(sent: Option<&[SentRequest]>, key: u32) -> Step {
    match sent {
        Some(s) if sent_with(s, key) => Step::Already,
        Some(s) if versions_used_up(s) => Step::Blocked,
        _ => Step::Due,
    }
}

/// The wait for an earlier request still on its way (review condition 5): before the
/// identity is taken and before anything is signed.
pub(in crate::real) fn request_on_its_way() -> CoreError {
    CoreError::NotAvailable(
        tr!("The earlier contact request may still go through: wait two minutes, then try again")
            .into(),
    )
}

/// What a batch asks of the network — the chain in the app, a fake in the tests.
pub(in crate::real) trait Side: Clone + Send + Sync + 'static {
    /// An identity's credits (`fresh`: read from Platform now).
    fn credits(&self, id: Identifier, fresh: bool) -> impl Future<Output = Option<u64>> + Send;
    /// `from`'s encryption key now, and its established contacts with the sender key of our
    /// latest request to each ([`Chain::resend_state`]).
    fn state(
        &self,
        from: Identifier,
    ) -> impl Future<Output = Option<(u32, Vec<(Identifier, u32)>)>> + Send;
    /// Sends `from`'s request to `to`.
    fn send(
        &self,
        from: Identifier,
        to: Identifier,
    ) -> impl Future<Output = Result<(), String>> + Send + 'static;
    /// Our requests from `from` to `to`, proved.
    fn read(
        &self,
        from: Identifier,
        to: Identifier,
    ) -> impl Future<Output = Result<Vec<SentRequest>, String>> + Send;
}

/// The chain, with the wallet and its keys of this opening.
#[derive(Clone)]
pub(in crate::real) struct OnChain {
    pub(in crate::real) chain: Arc<Chain>,
    pub(in crate::real) wallet: Arc<platform_wallet::PlatformWallet>,
    pub(in crate::real) keys: key_wallet::Wallet,
}

impl Side for OnChain {
    async fn credits(&self, id: Identifier, fresh: bool) -> Option<u64> {
        self.chain.identity_credits(&self.wallet, &id, fresh).await
    }

    async fn state(&self, from: Identifier) -> Option<(u32, Vec<(Identifier, u32)>)> {
        Chain::resend_state(&self.wallet, &from).await
    }

    fn send(
        &self,
        from: Identifier,
        to: Identifier,
    ) -> impl Future<Output = Result<(), String>> + Send + 'static {
        let (chain, wallet, keys) = (self.chain.clone(), self.wallet.clone(), self.keys.clone());
        async move { chain.add_contact(&wallet, &keys, &from, to).await }
    }

    /// Within a limit (TECH-read-deadlines).
    async fn read(&self, from: Identifier, to: Identifier) -> Result<Vec<SentRequest>, String> {
        super::names::within(&self.chain, |c| async move {
            c.sent_requests_to(from, to).await
        })
        .await
    }
}

/// A batch of requests sent again: who signs and pays, to whom.
pub(in crate::real) struct Batch<S: Side> {
    pub(in crate::real) side: S,
    /// The shown identity to its contacts; then, for a contact that is an identity of this
    /// wallet, it to us (decision 5).
    pub(in crate::real) pairs: Vec<(Identifier, Identifier)>,
    pub(in crate::real) debt: Arc<ResendDebt>,
    pub(in crate::real) unclear: sent::Unclear,
    pub(in crate::real) journal: Arc<journal::Journal>,
    pub(in crate::real) run: Arc<ResendRun>,
    /// The identity whose keys screen shows the batch.
    pub(in crate::real) shown: String,
}

/// One paying identity in a batch: its numbers, and its credits for its line.
struct Payer {
    tally: Tally,
    before: Option<u64>,
    /// Its credits before a request whose outcome is not known: its line ends there, the
    /// late answer writes its own (review condition 3).
    cut: Option<Option<u64>>,
    /// Its encryption key as last read, for the rows owed after a stop (review r1 S1).
    key: Option<u32>,
}

impl<S: Side> Batch<S> {
    /// Sends the batch, one contact at a time (the identity-contract nonce grows with each).
    /// Stops at no credits, at the first request not known (review answer 1) and at the lock;
    /// the rest are owed. Writes one History line for every identity that paid, whatever
    /// stopped it (review condition 4). `Err`: why it stopped early.
    pub(in crate::real) async fn send(self, refresh: impl Fn()) -> Result<(), CoreError> {
        let started = self.run.current();
        let of = self.pairs.len() as u32;
        let mut payers: BTreeMap<Identifier, Payer> = BTreeMap::new();
        let mut stopped: Option<CoreError> = None;
        let mut done = 0u32;
        for (from, to) in self.pairs.iter().copied() {
            self.run.set(
                &self.shown,
                Shown {
                    running: Some((done, of)),
                    done: None,
                },
            );
            let (from58, to58) = (base58(&from), base58(&to));
            let payer = match payers.entry(from) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e) => e.insert(Payer {
                    tally: Tally::default(),
                    before: self.side.credits(from, false).await,
                    cut: None,
                    key: None,
                }),
            };
            payer.tally.of += 1;
            if self.run.current() != started && stopped.is_none() {
                stopped = Some(CoreError::NotAvailable(
                    tr!("The wallet was locked, so the rest of the requests were not sent. Send them with “Send requests again” on the keys screen.")
                        .into(),
                ));
            }
            // Review r1 S1: stopped (no credits, not known, the lock) — the rest are owed with
            // nothing more asked of the network, the keys not used again (SEC-6 after a lock).
            if stopped.is_some() {
                payer.tally.count(Step::Due);
                // default-ok: stopped: nothing is signed, the key is not used
                self.owe(&from58, &to58, payer.key.unwrap_or(0), Owed::Due);
                continue;
            }
            let Some((key, contacts)) = self.side.state(from).await else {
                payer.tally.count(Step::Due);
                self.owe(&from58, &to58, 0, Owed::Due);
                continue;
            };
            payer.key = Some(key);
            let step = self
                .one(from, to, key, &contacts, payer, &mut stopped)
                .await;
            payer.tally.count(step);
            match step {
                Step::Sent | Step::Already => self.settle(&from58, &to58),
                Step::Due => self.owe(&from58, &to58, key, Owed::Due),
                Step::Blocked => self.owe(&from58, &to58, key, Owed::Blocked),
            }
            done += 1;
            refresh();
        }
        // One line for every identity that paid, whatever stopped the batch (condition 4).
        for (from, payer) in &payers {
            if payer.tally.sent == 0 {
                continue;
            }
            let after = match payer.cut {
                Some(cut) => cut,
                None => self.side.credits(*from, true).await,
            };
            self.journal.add(own_line(
                payer.tally.kind(),
                payer.before,
                after,
                Some(base58(from)),
            ));
        }
        let all = payers.values().fold(Tally::default(), |mut a, p| {
            a.of += p.tally.of;
            a.sent += p.tally.sent;
            a.already += p.tally.already;
            a.failed += p.tally.failed;
            a.version_full += p.tally.version_full;
            a.unclear += p.tally.unclear;
            a.left += p.tally.left;
            a
        });
        // Numbers only (AC-T9).
        crate::diag::record(crate::diag::Record::ContactResend {
            asked: all.of,
            sent: all.sent,
            already: all.already,
            failed: all.failed,
            version_full: all.version_full,
            unclear: all.unclear,
        });
        self.run.set(
            &self.shown,
            Shown {
                running: None,
                done: Some((all.sent + all.already, all.of)),
            },
        );
        refresh();
        stopped.map_or(Ok(()), Err)
    }

    /// One request, `from` to `to`, to be signed with our encryption key `key`.
    async fn one(
        &self,
        from: Identifier,
        to: Identifier,
        key: u32,
        contacts: &[(Identifier, u32)],
        payer: &mut Payer,
        stopped: &mut Option<CoreError>,
    ) -> Step {
        let (from58, to58) = (base58(&from), base58(&to));
        // Not twice (AC-T4): platform-wallet's latest request to the contact first; then, for
        // one owed (an earlier send may have landed unanswered), Platform's proved answer.
        if contacts.iter().any(|(c, k)| *c == to && *k == key) {
            return Step::Already;
        }
        let owed = self.debt.of(&from58).iter().any(|r| r.contact == to58);
        if owed
            && let Ok(sent) = self.side.read(from, to).await
            && sent_with(&sent, key)
        {
            return Step::Already;
        }
        let before = self.side.credits(from, true).await;
        let outcome = sent::run(
            self.side.send(from, to),
            sent::SEND_WAIT,
            |e: &String| {
                crate::diag::platform_failed("contact_resend", e);
                sent::refused_before(e) || duplicate(e)
            },
            || async { self.side.read(from, to).await.ok() },
            |sent: &Vec<SentRequest>| sent_with(sent, key),
        )
        .await;
        match outcome {
            sent::Outcome::Done(()) | sent::Outcome::Shown(_) => Step::Sent,
            sent::Outcome::Refused(e) if duplicate(&e) => {
                let sent = self.side.read(from, to).await.ok();
                after_duplicate(sent.as_deref(), key)
            }
            sent::Outcome::Refused(e) => {
                if super::super::platform_failure(e) == CoreError::NotEnough {
                    *stopped = Some(CoreError::NotEnough);
                }
                Step::Due
            }
            sent::Outcome::Unclear { late } => {
                // The batch stops here (review answer 1); this one is owed, the button waits
                // its quiet (`requests_on_their_way`). Its late success writes its own line
                // and is no longer owed (condition 3).
                payer.tally.unclear += 1;
                payer.cut = Some(before);
                *stopped = Some(CoreError::Unclear);
                let pending = sent::Pending::new(
                    from58.clone(),
                    sent::Watch::ContactRequest,
                    Some(journal::Kind::ContactsResent {
                        sent: 1,
                        of: 1,
                        left: 0,
                    }),
                    before,
                    Some(from58.clone()),
                );
                let (side, debt) = (self.side.clone(), self.debt.clone());
                let _ = self.unclear.not_known(
                    pending,
                    late,
                    self.journal.clone(),
                    move |_, p| async move {
                        let _ = debt.settle(&p.identity, &to58);
                        let after = side.credits(from, true).await;
                        let kind = p.kind.expect("a request's kind is known");
                        own_line(kind, p.before, after, p.acting)
                    },
                );
                Step::Due
            }
        }
    }

    fn owe(&self, identity: &str, contact: &str, key: u32, state: Owed) {
        let _ = self.debt.put(Row {
            identity: identity.to_owned(),
            contact: contact.to_owned(),
            key,
            state,
        });
    }

    fn settle(&self, identity: &str, contact: &str) {
        let _ = self.debt.settle(identity, contact);
    }
}

fn base58(id: &Identifier) -> String {
    id.to_string(Encoding::Base58)
}

/// What the keys screen and Contacts show of `me`'s requests sent again: the contacts still
/// owed one — `me`'s to them, or theirs to `me` when they are identities of this wallet — the
/// ones no request can go to, and the batches. `None` with nothing to show.
pub(in crate::real) fn shown(rows: &[Row], me: &str, run: Shown) -> Option<evolite_api::Resend> {
    let other = |r: &Row| {
        if r.identity == me {
            Some(r.contact.clone())
        } else if r.contact == me {
            Some(r.identity.clone())
        } else {
            None
        }
    };
    let of = |state: Owed| {
        let mut ids: Vec<String> = rows
            .iter()
            .filter(|r| r.state == state)
            .filter_map(other)
            .collect();
        ids.sort();
        ids.dedup();
        ids
    };
    let resend = evolite_api::Resend {
        due: of(Owed::Due),
        blocked: of(Owed::Blocked),
        running: run.running,
        done: run.done,
    };
    (resend != evolite_api::Resend::default()).then_some(resend)
}

fn identifier(id: &str) -> Option<Identifier> {
    Identifier::from_string(id, Encoding::Base58).ok()
}

/// Whether a change of keys replaces the DashPay pair (FEAT-0020): it disables an acting
/// ENCRYPTION key and adds this wallet's pair. Contacts' requests to us then point to a
/// disabled key, and they cannot pay us until we send them a new one — signed with the new
/// pair. A change that only disables leaves nothing to sign them with: no requests.
pub(in crate::real) fn replaces_the_pair(
    keys: &[evolite_api::IdentityKey],
    plan: &evolite_api::key_plan::KeyPlan,
) -> bool {
    plan.add.iter().any(|k| k.dashpay)
        && keys.iter().any(|k| {
            plan.disable.contains(&k.id)
                && k.purpose == evolite_api::KeyPurpose::Encryption
                && k.disabled_at.is_none()
        })
}

/// The requests a change of `me`'s pair calls for: `me` to each established contact, then,
/// for a contact that is an identity of this wallet, it to `me` (decision 5).
pub(in crate::real) async fn pairs_after_change(
    wallet: &platform_wallet::PlatformWallet,
    me: Identifier,
) -> Vec<(Identifier, Identifier)> {
    let Some((_, contacts)) = Chain::resend_state(wallet, &me).await else {
        return Vec::new();
    };
    let ours: Vec<Identifier> = crate::real::snapshot::wallet_identity_ids(wallet)
        .await
        .iter()
        .filter_map(|(id, _)| identifier(id))
        .collect();
    let mut pairs: Vec<(Identifier, Identifier)> =
        contacts.iter().map(|(to, _)| (me, *to)).collect();
    pairs.extend(
        contacts
            .iter()
            .filter(|(to, _)| ours.contains(to))
            .map(|(to, _)| (*to, me)),
    );
    pairs
}

/// Whether a request of `identity` or of any sender of `rows` may still land (review r1 S3).
fn senders_on_their_way(unclear: &sent::Unclear, identity: &str, rows: &[Row]) -> bool {
    std::iter::once(identity)
        .chain(rows.iter().map(|r| r.identity.as_str()))
        .any(|sender| unclear.requests_on_their_way(sender))
}

/// The owed rows «Send requests again» sends for `identity`: its own, and an identity of this
/// wallet's to it (decision 5); only `contact`'s when the contact card asks.
fn rows_to_send(rows: &[Row], identity: &str, contact: Option<&str>) -> Vec<Row> {
    rows.iter()
        .filter(|r| r.state == Owed::Due)
        .filter(|r| r.identity == identity || r.contact == identity)
        .filter(|r| {
            contact.is_none_or(|c| {
                (r.identity == identity && r.contact == c)
                    || (r.contact == identity && r.identity == c)
            })
        })
        .cloned()
        .collect()
}

impl Real {
    /// `Spend::ResendRequests` (R-8, R-9): the owed requests of `identity`, or to `contact`
    /// alone, past the password, one at a time. Waits while an earlier one may still land, and
    /// takes the identity, as every write of it (review condition 5).
    pub(in crate::real) fn resend_requests(
        &self,
        identity: String,
        contact: Option<String>,
    ) -> Result<(), CoreError> {
        let (wallet_id, keys) = self.dashpay_ready()?;
        let rows = rows_to_send(&self.resend_debt.load(), &identity, contact.as_deref());
        // Review r1 S3: the shown identity, and every other one of the rows (an own identity's
        // request to it, decision 5): a repeat of one still on its way is refused at a fee.
        if senders_on_their_way(&self.unclear, &identity, &rows) {
            return Err(request_on_its_way());
        }
        if rows.is_empty() {
            return Err(CoreError::NotAvailable(
                tr!("No contact is waiting for a request from this identity").into(),
            ));
        }
        let Some(busy) = self.identity_busy.take() else {
            return Err(super::identity_keys::identity_busy());
        };
        let (chain, refresh) = (self.chain.clone(), self.refresher());
        let (debt, unclear, journal, run) = (
            self.resend_debt.clone(),
            self.unclear.clone(),
            self.journal.clone(),
            self.resend_run.clone(),
        );
        self.answer_later("task:resend_requests", async move {
            let result = async {
                let wallet = loaded(&chain, &wallet_id).await?;
                let pairs = rows
                    .iter()
                    .filter_map(|r| Some((identifier(&r.identity)?, identifier(&r.contact)?)))
                    .collect();
                Batch {
                    side: OnChain {
                        chain: chain.clone(),
                        wallet,
                        keys,
                    },
                    pairs,
                    debt,
                    unclear,
                    journal,
                    run,
                    shown: identity,
                }
                .send(refresh.clone())
                .await
            }
            .await;
            drop(busy);
            refresh();
            Event::Done {
                what: DoneWhat::Resend,
                result,
                ask: None,
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(key: u32, version: u32) -> SentRequest {
        SentRequest {
            sender_key: key,
            account_reference: (version << 28) | 0x0abc_def0,
        }
    }

    /// Review condition 2: a unique-index refusal is R-11 only at the last version; with our
    /// key now among our requests it was already sent (an unclear send that landed); else it
    /// is owed. In any order of the requests (condition 1).
    #[test]
    fn a_unique_index_refusal_is_read_by_our_requests() {
        assert_eq!(
            after_duplicate(Some(&[at(6, 1), at(8, 2)]), 8),
            Step::Already
        );
        assert_eq!(
            after_duplicate(Some(&[at(8, 2), at(6, 1)]), 8),
            Step::Already
        );
        assert_eq!(
            after_duplicate(Some(&[at(6, 14), at(4, 15)]), 8),
            Step::Blocked
        );
        assert_eq!(after_duplicate(Some(&[at(6, 1)]), 8), Step::Due);
        assert_eq!(after_duplicate(None, 8), Step::Due, "the read failed");
        assert!(duplicate(
            "Document 4s… has duplicate unique properties [\"$ownerId\", \"toUserId\", \"accountReference\"] with other documents"
        ));
        assert!(!duplicate("Insufficient identity balance"));
    }

    /// The batch's numbers make its History line: sent now and already there, of all, owed.
    #[test]
    fn a_tally_makes_its_line() {
        let mut t = Tally {
            of: 4,
            ..Tally::default()
        };
        for step in [Step::Sent, Step::Already, Step::Due, Step::Blocked] {
            t.count(step);
        }
        assert_eq!(
            t.kind(),
            journal::Kind::ContactsResent {
                sent: 2,
                of: 4,
                left: 1
            }
        );
        assert_eq!((t.failed, t.version_full), (1, 1));
    }

    // ---- The batch, on a fake network ----

    use std::sync::Mutex as StdMutex;

    /// A request's fee, in credits.
    const FEE: u64 = 70_000_000;
    /// The new pair's sender key; the old one was 6.
    const KEY: u32 = 8;

    fn id(n: u8) -> Identifier {
        Identifier::from([n; 32])
    }

    #[derive(Clone)]
    enum Answer {
        Ok,
        /// Refused with these words.
        Refuse(&'static str),
        /// No answer in time; it lands when `gate` is notified.
        Late,
        /// The wallet is locked while it is sent; it goes through.
        LockMeanwhile,
    }

    #[derive(Default)]
    struct World {
        credits: HashMap<Identifier, u64>,
        /// platform-wallet's latest request of an identity to a contact: its sender key.
        latest: Vec<(Identifier, Identifier, u32)>,
        /// Our requests on Platform, by (owner, contact).
        docs: HashMap<(Identifier, Identifier), Vec<SentRequest>>,
        answers: HashMap<(Identifier, Identifier), Answer>,
        sends: Vec<(Identifier, Identifier)>,
        /// How often the network was asked for a sender's state, and for our requests.
        states: usize,
        reads: usize,
    }

    #[derive(Clone)]
    struct Fake {
        world: Arc<StdMutex<World>>,
        gate: Arc<tokio::sync::Notify>,
        run: Arc<ResendRun>,
    }

    impl Fake {
        fn world(&self) -> std::sync::MutexGuard<'_, World> {
            self.world.lock().unwrap()
        }
    }

    fn landed(world: &Arc<StdMutex<World>>, from: Identifier, to: Identifier) {
        let mut w = world.lock().unwrap();
        *w.credits.entry(from).or_default() -= FEE;
        w.docs.entry((from, to)).or_default().push(SentRequest {
            sender_key: KEY,
            account_reference: 1 << 28,
        });
        w.latest.retain(|(f, t, _)| !(*f == from && *t == to));
        w.latest.push((from, to, KEY));
    }

    impl Side for Fake {
        async fn credits(&self, id: Identifier, _fresh: bool) -> Option<u64> {
            self.world().credits.get(&id).copied()
        }

        async fn state(&self, from: Identifier) -> Option<(u32, Vec<(Identifier, u32)>)> {
            let mut w = self.world();
            w.states += 1;
            let contacts = w
                .latest
                .iter()
                .filter(|(f, _, _)| *f == from)
                .map(|(_, t, k)| (*t, *k))
                .collect();
            Some((KEY, contacts))
        }

        fn send(
            &self,
            from: Identifier,
            to: Identifier,
        ) -> impl Future<Output = Result<(), String>> + Send + 'static {
            let (world, gate, run) = (self.world.clone(), self.gate.clone(), self.run.clone());
            async move {
                let answer = {
                    let mut w = world.lock().unwrap();
                    w.sends.push((from, to));
                    w.answers.get(&(from, to)).cloned().unwrap_or(Answer::Ok)
                };
                match answer {
                    Answer::Ok => {
                        landed(&world, from, to);
                        Ok(())
                    }
                    Answer::Refuse(why) => Err(why.into()),
                    Answer::Late => {
                        gate.notified().await;
                        landed(&world, from, to);
                        Ok(())
                    }
                    Answer::LockMeanwhile => {
                        run.lock();
                        landed(&world, from, to);
                        Ok(())
                    }
                }
            }
        }

        async fn read(&self, from: Identifier, to: Identifier) -> Result<Vec<SentRequest>, String> {
            let mut w = self.world();
            w.reads += 1;
            Ok(w.docs.get(&(from, to)).cloned().unwrap_or_default())
        }
    }

    struct Bench {
        fake: Fake,
        debt: Arc<ResendDebt>,
        journal: Arc<journal::Journal>,
        unclear: sent::Unclear,
        dir: std::path::PathBuf,
    }

    impl Drop for Bench {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Identity 1 with contacts 2, 3, 4 (their latest requests from us signed with the old
    /// key 6), and `answers` for some of them.
    fn bench(name: &str, answers: &[(u8, Answer)]) -> Bench {
        let dir =
            std::env::temp_dir().join(format!("evolite-resend-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut world = World::default();
        world.credits.insert(id(1), 1_000_000_000);
        for c in [2, 3, 4] {
            world.latest.push((id(1), id(c), 6));
        }
        for (c, a) in answers {
            world.answers.insert((id(1), id(*c)), a.clone());
        }
        Bench {
            fake: Fake {
                world: Arc::new(StdMutex::new(world)),
                gate: Arc::new(tokio::sync::Notify::new()),
                run: Arc::new(ResendRun::default()),
            },
            debt: Arc::new(ResendDebt::new(dir.join("contact-resend.json"))),
            journal: Arc::new(journal::Journal::new(dir.join("platform-history.json"))),
            unclear: sent::Unclear::default(),
            dir,
        }
    }

    impl Bench {
        fn batch(&self, pairs: Vec<(Identifier, Identifier)>) -> Batch<Fake> {
            Batch {
                side: self.fake.clone(),
                pairs,
                debt: self.debt.clone(),
                unclear: self.unclear.clone(),
                journal: self.journal.clone(),
                run: self.fake.run.clone(),
                shown: base58(&id(1)),
            }
        }

        fn all_three(&self) -> Batch<Fake> {
            self.batch(vec![(id(1), id(2)), (id(1), id(3)), (id(1), id(4))])
        }

        /// The batch lines in History: (sent, of, left, spent).
        fn lines(&self) -> Vec<(u32, u32, u32, Duffs)> {
            self.journal
                .entries()
                .iter()
                .filter_map(|e| match e.kind {
                    Some(journal::Kind::ContactsResent { sent, of, left }) => {
                        Some((sent, of, left, e.spent))
                    }
                    _ => None,
                })
                .collect()
        }

        fn owed(&self) -> Vec<(String, Owed)> {
            self.debt
                .of(&base58(&id(1)))
                .into_iter()
                .map(|r| (r.contact, r.state))
                .collect()
        }

        fn sends(&self) -> usize {
            self.fake.world().sends.len()
        }
    }

    /// AC-T3: the 2nd of 3 refused — the 1st and the 3rd go, the 2nd is owed on disk (a new
    /// instance on the same file sees it); «Send requests again» sends only it, and after it
    /// nothing is owed. One History line a batch, with what it cost.
    #[tokio::test]
    async fn a_failed_request_is_owed_and_sent_again_alone() {
        let b = bench(
            "partial",
            &[(
                3,
                Answer::Refuse("state transition broadcast error: refused"),
            )],
        );
        assert!(b.all_three().send(|| {}).await.is_ok());
        assert_eq!(b.owed(), vec![(base58(&id(3)), Owed::Due)]);
        let again = ResendDebt::new(b.dir.join("contact-resend.json"));
        assert_eq!(again.of(&base58(&id(1))).len(), 1, "on disk");
        assert_eq!(b.lines(), vec![(2, 3, 1, 140_000)]);
        b.fake.world().answers.clear();
        let rows = rows_to_send(&b.debt.load(), &base58(&id(1)), None);
        let pairs = rows
            .iter()
            .map(|r| {
                (
                    identifier(&r.identity).unwrap(),
                    identifier(&r.contact).unwrap(),
                )
            })
            .collect();
        let before = b.sends();
        assert!(b.batch(pairs).send(|| {}).await.is_ok());
        assert_eq!(b.sends() - before, 1, "only the one owed");
        assert!(b.owed().is_empty());
        assert_eq!(b.lines()[1], (1, 1, 0, 70_000));
    }

    /// AC-T4: not twice. A contact whose latest request from us has the key now is `already`:
    /// nothing sent. One owed whose request did land unanswered is found by the proved read:
    /// nothing sent, no longer owed. Mutant: without the checks — a second send each.
    #[tokio::test]
    async fn a_request_already_there_is_not_sent_again() {
        let b = bench("already", &[]);
        b.fake.world().latest.retain(|(_, t, _)| *t != id(2));
        b.fake.world().latest.push((id(1), id(2), KEY));
        b.debt
            .put(Row {
                identity: base58(&id(1)),
                contact: base58(&id(3)),
                key: KEY,
                state: Owed::Due,
            })
            .unwrap();
        landed(&b.fake.world, id(1), id(3));
        b.fake
            .world()
            .latest
            .retain(|(_, t, k)| !(*t == id(3) && *k == KEY));
        assert!(b.all_three().send(|| {}).await.is_ok());
        assert_eq!(b.fake.world().sends, vec![(id(1), id(4))], "only the 3rd");
        assert!(b.owed().is_empty());
        assert_eq!(b.lines(), vec![(3, 3, 0, 70_000)]);
    }

    /// AC-T5: credits for 1 of 3 — the 2nd is refused for credits, the batch stops, the 2nd
    /// and the 3rd are owed; the line says 1 of 3, 2 left (condition 4: written on this stop).
    #[tokio::test]
    async fn no_credits_stops_the_batch_and_owes_the_rest() {
        let b = bench(
            "credits",
            &[(
                3,
                Answer::Refuse("Current credits balance 1 is not enough to pay 2 fee"),
            )],
        );
        assert_eq!(b.all_three().send(|| {}).await, Err(CoreError::NotEnough));
        assert_eq!(b.sends(), 2, "the 3rd not tried");
        assert_eq!(b.owed().len(), 2);
        assert_eq!(b.lines(), vec![(1, 3, 2, 70_000)]);
    }

    /// AC-T6 and review condition 2: a unique-index refusal at the last version is R-11 —
    /// blocked, not owed for the button; the others go.
    #[tokio::test]
    async fn the_unique_index_at_the_last_version_blocks_one_contact() {
        let b = bench(
            "blocked",
            &[(
                3,
                Answer::Refuse("Document x has duplicate unique properties with other documents"),
            )],
        );
        b.fake.world().docs.insert(
            (id(1), id(3)),
            vec![SentRequest {
                sender_key: 6,
                account_reference: 15 << 28,
            }],
        );
        assert!(b.all_three().send(|| {}).await.is_ok());
        assert_eq!(b.owed(), vec![(base58(&id(3)), Owed::Blocked)]);
        assert!(rows_to_send(&b.debt.load(), &base58(&id(1)), None).is_empty());
        assert_eq!(b.lines(), vec![(2, 3, 0, 140_000)]);
    }

    /// Review answer 1 and condition 3: the 2nd has no answer in time — the batch stops there
    /// (the 3rd owed, not sent), its line has the 1st's fee only; the 2nd's late success
    /// writes its own line with its fee and is no longer owed. While it may land, no other
    /// request of the identity goes (condition 5).
    #[tokio::test(start_paused = true)]
    async fn a_late_request_after_the_batch_keeps_its_fee() {
        let b = bench("late", &[(3, Answer::Late)]);
        assert_eq!(b.all_three().send(|| {}).await, Err(CoreError::Unclear));
        assert_eq!(b.sends(), 2, "stopped at the one not known");
        assert_eq!(b.lines(), vec![(1, 3, 2, 70_000)]);
        assert!(b.unclear.requests_on_their_way(&base58(&id(1))));
        b.fake.gate.notify_one();
        for _ in 0..200 {
            if b.lines().len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(b.lines()[1], (1, 1, 0, 70_000), "its own line, its fee");
        assert_eq!(b.owed(), vec![(base58(&id(4)), Owed::Due)]);
    }

    /// Review condition 4, R-7: the wallet locked while the 1st is sent — the 1st went, the
    /// batch stops before the next; the rest are owed, and the line is written for the one
    /// paid.
    #[tokio::test]
    async fn a_lock_between_contacts_owes_the_rest_and_keeps_the_line() {
        let b = bench("lock", &[(2, Answer::LockMeanwhile)]);
        assert!(b.all_three().send(|| {}).await.is_err());
        assert_eq!(b.sends(), 1);
        assert_eq!(b.owed().len(), 2);
        assert_eq!(b.lines(), vec![(1, 3, 2, 70_000)]);
        // Review r1 S1: after the lock nothing more is asked of the network — one state, for
        // the 1st, no read. Mutant: state() before the stop check — 3 states.
        let w = b.fake.world();
        assert_eq!((w.states, w.reads), (1, 0), "nothing asked after the lock");
        assert!(
            b.debt.of(&base58(&id(1))).iter().all(|r| r.key == KEY),
            "owed with the sender's key as last read"
        );
    }

    /// Review r1 S3: the button on A waits while B's request to A (an own identity, decision
    /// 5) may still land — not only A's own. Mutant: A alone checked.
    #[test]
    fn the_button_waits_for_every_sender_of_its_rows() {
        let unclear = sent::Unclear::default();
        let a = base58(&id(1));
        let b = base58(&id(2));
        let rows = vec![Row {
            identity: b.clone(),
            contact: a.clone(),
            key: KEY,
            state: Owed::Due,
        }];
        assert!(!senders_on_their_way(&unclear, &a, &rows));
        // B's request just ended without an answer: inside its quiet.
        let dir = std::env::temp_dir().join(format!("evolite-resend-s3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let journal = Arc::new(journal::Journal::new(dir.join("platform-history.json")));
        let _ = unclear.not_known(
            sent::Pending::new(b.clone(), sent::Watch::ContactRequest, None, None, None),
            None::<tokio::task::JoinHandle<Result<(), String>>>,
            journal,
            |_, p| async move {
                own_line(
                    journal::Kind::ContactsResent {
                        sent: 1,
                        of: 1,
                        left: 0,
                    },
                    None,
                    None,
                    p.acting,
                )
            },
        );
        assert!(senders_on_their_way(&unclear, &a, &rows), "B's on its way");
        assert!(
            !senders_on_their_way(&unclear, &a, &[]),
            "not when B is not a sender"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC-T7: a contact that is an identity of this wallet — a request each way, each paid by
    /// its own identity and on its own line (review answer 2).
    #[tokio::test]
    async fn an_own_identity_gets_a_request_each_way() {
        let b = bench("own", &[]);
        b.fake.world().credits.insert(id(2), 500_000_000);
        b.fake.world().latest.push((id(2), id(1), 6));
        let pairs = vec![(id(1), id(2)), (id(2), id(1))];
        assert!(b.batch(pairs).send(|| {}).await.is_ok());
        assert_eq!(b.fake.world().sends, vec![(id(1), id(2)), (id(2), id(1))]);
        let acting: Vec<Option<String>> = b
            .journal
            .entries()
            .iter()
            .filter(|e| matches!(e.kind, Some(journal::Kind::ContactsResent { .. })))
            .map(|e| e.identity.clone())
            .collect();
        assert_eq!(acting.len(), 2, "a line for each identity that paid");
        assert_ne!(acting[0], acting[1]);
        assert_eq!(b.lines(), vec![(1, 1, 0, 70_000), (1, 1, 0, 70_000)]);
    }

    /// What the screens are shown: the contacts owed and blocked, both ways for own
    /// identities; nothing at all with nothing to show.
    #[test]
    fn the_shown_identity_sees_whom_it_owes() {
        let row = |identity: u8, contact: u8, state| Row {
            identity: base58(&id(identity)),
            contact: base58(&id(contact)),
            key: KEY,
            state,
        };
        let rows = vec![
            row(1, 2, Owed::Due),
            row(5, 1, Owed::Due),
            row(1, 3, Owed::Blocked),
            row(1, 4, Owed::Waiting),
            row(7, 8, Owed::Due),
        ];
        let shown = shown(&rows, &base58(&id(1)), Shown::default()).unwrap();
        let mut due = vec![base58(&id(2)), base58(&id(5))];
        due.sort();
        assert_eq!(shown.due, due);
        assert_eq!(shown.blocked, vec![base58(&id(3))]);
        assert!(super::shown(&rows, &base58(&id(9)), Shown::default()).is_none());
        let one = rows_to_send(&rows, &base58(&id(1)), Some(&base58(&id(5))));
        assert_eq!(
            one,
            vec![row(5, 1, Owed::Due)],
            "the card's one contact, its way"
        );
    }

    /// FEAT-0020: only a change that disables an acting encryption key and adds the pair
    /// replaces it; disabling alone leaves nothing to sign requests with.
    #[test]
    fn only_a_disabled_encryption_key_calls_for_requests() {
        use evolite_api::{IdentityKey, KeyKind, KeyLevel, KeyOwner, KeyPurpose};
        let key = |id, purpose, disabled_at| IdentityKey {
            id,
            level: KeyLevel::Medium,
            purpose,
            kind: KeyKind::Ecdsa,
            decrypts: false,
            disabled_at,
            bound_to: None,
            owner: KeyOwner::This,
        };
        let keys = vec![
            key(1, KeyPurpose::Identity, None),
            key(6, KeyPurpose::Encryption, None),
            key(4, KeyPurpose::Encryption, Some(1)),
        ];
        use evolite_api::key_plan::{KeyPlan, NewKey, NewPurpose};
        let pair = vec![NewKey {
            id: 8,
            purpose: NewPurpose::Encryption,
            level: KeyLevel::Medium,
            dashpay: true,
        }];
        let plan = |add: Vec<NewKey>, disable: Vec<u32>| KeyPlan { add, disable };
        assert!(replaces_the_pair(&keys, &plan(pair.clone(), vec![6])));
        assert!(
            !replaces_the_pair(&keys, &plan(pair.clone(), vec![1])),
            "not the pair"
        );
        assert!(
            !replaces_the_pair(&keys, &plan(pair, vec![4])),
            "already disabled"
        );
        assert!(
            !replaces_the_pair(&keys, &plan(Vec::new(), vec![6])),
            "only disabled"
        );
    }

    /// Review condition 5 and AC-T2, in the code's order: «Send requests again», a key change
    /// and «Add a contact» ask `requests_on_their_way` before the identity is taken and
    /// anything is signed; the resend never goes through `repeat_request` (BUG-13), which
    /// «Add a contact» still asks. AC-T8: the button needs the keys open first, and nothing
    /// sends what is owed by itself.
    #[test]
    fn the_guards_come_first() {
        fn body<'a>(file: &'a str, fun: &str) -> &'a str {
            let f = &file[file.find(fun).unwrap_or_else(|| panic!("{fun}"))..];
            &f[..f
                .find("\n    }\n")
                .or_else(|| f.find("\r\n    }\r\n"))
                .unwrap_or(f.len())]
        }
        let resend = include_str!("resend.rs");
        let keys = include_str!("identity_keys.rs");
        let dashpay = include_str!("dashpay.rs");
        let order = |f: &str, before: &[&str]| {
            let at: Vec<usize> = before
                .iter()
                .map(|n| f.find(n).unwrap_or_else(|| panic!("no {n}")))
                .collect();
            assert!(at.windows(2).all(|w| w[0] < w[1]), "{before:?}");
        };
        let button = body(resend, "fn resend_requests(");
        order(
            button,
            &[
                "self.dashpay_ready()",
                "senders_on_their_way(",
                "self.identity_busy.take()",
                "self.answer_later(",
            ],
        );
        order(
            body(keys, "fn update_keys("),
            &[
                ".requests_on_their_way(",
                "self.identity_busy.take()",
                "self.answer_later(",
            ],
        );
        order(
            body(dashpay, "fn add_contact("),
            &[
                ".requests_on_their_way(",
                "self.answer_later(",
                "repeat_request(",
            ],
        );
        let code = &resend[..resend.find("#[cfg(test)]").unwrap()];
        assert!(
            !code.contains("repeat_request("),
            "the resend asks BUG-13's check"
        );
        let mut callers = 0;
        for file in [
            include_str!("../dispatch.rs"),
            include_str!("../lifecycle.rs"),
            include_str!("wallet.rs"),
            keys,
            dashpay,
        ] {
            callers += file.matches(".resend_requests(").count();
        }
        assert_eq!(callers, 1, "only the button's Spend sends what is owed");
    }
}
