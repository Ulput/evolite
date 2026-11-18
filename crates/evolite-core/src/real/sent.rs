//! One rule for a paid Platform write that may have ended after it went to the network
//! (TECH-after-send-outcome; the lesson of BUG-184 and BUG-200).
//!
//! The send runs as a task of its own and is never cut: a deadline only stops the waiting for
//! its answer. Without a proved answer the state is read again (proved): showing the write, it
//! is done; else its outcome is not known, kept in [`Unclear`] for the first read that shows it
//! (or for the send's own late answer) to write its History line, once. Only a refusal that came
//! before the write went out ([`Outcome::Refused`]) is a plain failure.

use crate::real::*;
use evolite_api::key_plan;
use std::future::Future;

/// How long the answer of a send is waited for before the state is read again instead. The send
/// goes on: it is never cut (TECH-after-send-outcome p. 2).
pub(in crate::real) const SEND_WAIT: Duration = Duration::from_secs(120);

/// The answer to a write whose outcome is not known: it may have gone through. Nothing in it
/// says it failed, and the window does not offer to send it again.
pub(in crate::real) const UNCLEAR: &str =
    tr!("Could not find out whether the operation went through — look again before repeating it");

/// How a paid write ended.
pub(in crate::real) enum Outcome<T, S, E> {
    /// The send's own proved answer.
    Done(T),
    /// No answer (an error after the send, or none in time), but the state read again shows it.
    Shown(S),
    /// Neither: it may have gone through. `late` is the send still running past the wait: its
    /// answer, if it comes and is a success, settles the write ([`Unclear::settle_late`]).
    Unclear {
        late: Option<tokio::task::JoinHandle<Result<T, E>>>,
    },
    /// Refused: before the write went out (a check, the signature), or the transition itself
    /// refused by Platform. It did not take effect and may be asked again; a refusal in a block
    /// may still have cost a fee (review r1, S1).
    Refused(E),
}

/// Why a write with no read of its own has no answer.
pub(in crate::real) enum NoAnswer<T, E> {
    Refused(E),
    /// Not known; with the send still running, if it is.
    Unclear(Option<tokio::task::JoinHandle<Result<T, E>>>),
}

impl<T, E> Outcome<T, (), E> {
    /// The answer of a write whose state is not read again: done, refused, or not known.
    pub(in crate::real) fn answer(self) -> Result<T, NoAnswer<T, E>> {
        match self {
            Outcome::Done(done) => Ok(done),
            Outcome::Refused(e) => Err(NoAnswer::Refused(e)),
            Outcome::Shown(()) => Err(NoAnswer::Unclear(None)),
            Outcome::Unclear { late, .. } => Err(NoAnswer::Unclear(late)),
        }
    }
}

/// The History line of a write that went through after all (BUG-145): its cost from the credits
/// before (kept) and after (read now); `kind` from the late answer, else the kept one.
pub(in crate::real) async fn line_after(
    chain: Arc<Chain>,
    wallet: Arc<platform_wallet::PlatformWallet>,
    id: Identifier,
    kind: Option<journal::Kind>,
    pending: Pending,
) -> journal::Entry {
    let after = chain.identity_credits(&wallet, &id, true).await;
    let kind = kind
        .or(pending.kind)
        .expect("a write's kind: kept or from its answer");
    own_line(kind, pending.before, after, pending.acting)
}

/// Runs a paid write by the one rule: `send` as a task of its own, its answer waited for `wait`;
/// an error that `refused` says came before the write went out is [`Outcome::Refused`]; any other
/// error, a panic or no answer in time has the state read again (`reread`), and `shows` says
/// whether it shows the write done.
pub(in crate::real) async fn run<T, S, E, R, F>(
    send: impl Future<Output = Result<T, E>> + Send + 'static,
    wait: Duration,
    refused: impl FnOnce(&E) -> bool,
    reread: R,
    shows: impl FnOnce(&S) -> bool,
) -> Outcome<T, S, E>
where
    T: Send + 'static,
    E: Send + 'static,
    R: FnOnce() -> F,
    F: Future<Output = Option<S>>,
{
    let mut task = tokio::spawn(send);
    // The waiting has a deadline, the send none: past it the task goes on (p. 2).
    match tokio::time::timeout(wait, &mut task).await {
        Ok(Ok(Ok(done))) => return Outcome::Done(done),
        Ok(Ok(Err(e))) if refused(&e) => return Outcome::Refused(e),
        // An error after it may have gone out, or a panic of the send: where it stopped is not
        // known.
        Ok(Ok(Err(_))) | Ok(Err(_)) => {}
        Err(_) => crate::diag::platform_failed("send_wait", "Late"),
    }
    let late = (!task.is_finished()).then_some(task);
    match reread().await {
        Some(state) if shows(&state) => Outcome::Shown(state),
        _ => Outcome::Unclear { late },
    }
}

/// Whether a failed send's error says the write did not take effect: the chain's own checks
/// before it went out (not ready, no such identity), or Platform refusing the transition itself —
/// its keys or signature (BUG-175), the credits (`dashpay_failure`'s words), or a state
/// transition broadcast error (the transition was in a block and failed validation: not done,
/// but a fee may have been charged; review r1, S1). Anything else (a timeout, a DAPI error, a
/// proof that did not check, «already in the cache») may have gone through.
pub(in crate::real) fn refused_before(text: &str) -> bool {
    let lower = text.to_lowercase();
    text == chain::NOT_READY
        || text == tr!("The wallet has no identity")
        || lower.contains("state transition broadcast error")
        || lower.contains("insufficient")
        || lower.contains("is not enough")
        || super::about_keys(text)
}

/// What a read that shows an unclear write done looks for.
#[derive(Clone, Debug)]
pub(in crate::real) enum Watch {
    /// A key change (BUG-184): the identity read again shows the plan.
    Keys(key_plan::KeyPlan),
    /// Only the send's own late answer settles it.
    Answer,
}

/// A paid write sent whose outcome is not known ([`UNCLEAR`]): what its History line needs once a
/// read shows it done. Kept until then, or the lock.
#[derive(Clone, Debug)]
pub(in crate::real) struct Pending {
    pub(in crate::real) identity: String,
    pub(in crate::real) watch: Watch,
    /// The line's kind, when it is known before the answer (a claim's amount is not).
    pub(in crate::real) kind: Option<journal::Kind>,
    pub(in crate::real) before: Option<u64>,
    pub(in crate::real) acting: Option<String>,
    ticket: u64,
}

impl Pending {
    pub(in crate::real) fn new(
        identity: String,
        watch: Watch,
        kind: Option<journal::Kind>,
        before: Option<u64>,
        acting: Option<String>,
    ) -> Self {
        Self {
            identity,
            watch,
            kind,
            before,
            acting,
            ticket: 0,
        }
    }
}

/// The writes whose outcome is not known yet (TECH-after-send-outcome p. 3; BUG-184's slot for
/// key changes, for every kind). Shared by the tasks; the lock forgets them.
#[derive(Clone, Default)]
pub(in crate::real) struct Unclear(Arc<std::sync::Mutex<(u64, Vec<Pending>)>>);

impl Unclear {
    fn held(&self) -> std::sync::MutexGuard<'_, (u64, Vec<Pending>)> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keeps `pending` until a read shows it; its ticket, for its late answer.
    pub(in crate::real) fn keep(&self, mut pending: Pending) -> u64 {
        let mut held = self.held();
        held.0 += 1;
        pending.ticket = held.0;
        held.1.push(pending);
        held.0
    }

    /// Whether any write of `identity` watched as `of` waits.
    pub(in crate::real) fn waits(&self, identity: &str, of: impl Fn(&Watch) -> bool) -> bool {
        self.held()
            .1
            .iter()
            .any(|p| p.identity == identity && of(&p.watch))
    }

    /// The first write of `identity` that `shows` says a read of it shows done, taken out: its
    /// line is then written, once.
    pub(in crate::real) fn take_if(
        &self,
        identity: &str,
        shows: impl Fn(&Watch) -> bool,
    ) -> Option<Pending> {
        let mut held = self.held();
        let at = held
            .1
            .iter()
            .position(|p| p.identity == identity && shows(&p.watch))?;
        Some(held.1.remove(at))
    }

    /// The write of `ticket`, taken out if it still waits.
    pub(in crate::real) fn take(&self, ticket: u64) -> Option<Pending> {
        let mut held = self.held();
        let at = held.1.iter().position(|p| p.ticket == ticket)?;
        Some(held.1.remove(at))
    }

    /// The ending of a write whose outcome is not known: kept as `pending`; the send still
    /// running (`late`), if its answer comes and is a success, writes its line by `line`. The
    /// answer for the window, [`UNCLEAR`].
    pub(in crate::real) fn not_known<T, E, F>(
        &self,
        pending: Pending,
        late: Option<tokio::task::JoinHandle<Result<T, E>>>,
        journal: Arc<journal::Journal>,
        line: impl FnOnce(T, Pending) -> F + Send + 'static,
    ) -> CoreError
    where
        T: Send + 'static,
        E: Send + 'static,
        F: Future<Output = journal::Entry> + Send,
    {
        let ticket = self.keep(pending);
        if let Some(late) = late {
            self.settle_late(ticket, late, journal, line);
        }
        CoreError::Failed(UNCLEAR.into())
    }

    /// The lock: what was not known is forgotten (BUG-184).
    pub(in crate::real) fn forget(&self) {
        self.held().1.clear();
    }

    /// The send of `ticket` still running past the wait: if it ends with a success while its
    /// write still waits, `line` makes its History line from the answer, and it is written once.
    pub(in crate::real) fn settle_late<T, E, F>(
        &self,
        ticket: u64,
        late: tokio::task::JoinHandle<Result<T, E>>,
        journal: Arc<journal::Journal>,
        line: impl FnOnce(T, Pending) -> F + Send + 'static,
    ) where
        T: Send + 'static,
        E: Send + 'static,
        F: Future<Output = journal::Entry> + Send,
    {
        let unclear = self.clone();
        tokio::spawn(async move {
            if let Ok(Ok(answer)) = late.await
                && let Some(pending) = unclear.take(ticket)
            {
                crate::diag::platform_failed("send_late", "Done");
                journal.add(line(answer, pending).await);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind() -> journal::Kind {
        journal::Kind::KeysChanged {
            added: vec![9],
            disabled: vec![],
            bound: 0,
        }
    }

    fn pending(identity: &str) -> Pending {
        Pending::new(identity.into(), Watch::Answer, Some(kind()), Some(10), None)
    }

    type Out = Outcome<u32, u32, String>;

    async fn ran(
        send: impl Future<Output = Result<u32, String>> + Send + 'static,
        wait: Duration,
        state: Option<u32>,
    ) -> Out {
        run(
            send,
            wait,
            |e: &String| e == "refused",
            move || async move { state },
            |s| *s == 7,
        )
        .await
    }

    /// AC-T1: the send's answer is done; an error after it with the state showing it is done
    /// too; not showing it, not known (never «failed»); a refusal before it went out, refused
    /// with no read.
    #[tokio::test]
    async fn a_write_is_done_shown_unclear_or_refused() {
        let wait = Duration::from_secs(5);
        assert!(matches!(
            ran(async { Ok(1) }, wait, None).await,
            Out::Done(1)
        ));
        let lost = || async { Err::<u32, _>("timed out".to_string()) };
        assert!(matches!(ran(lost(), wait, Some(7)).await, Out::Shown(7)));
        let unclear = ran(lost(), wait, Some(6)).await;
        assert!(matches!(unclear, Out::Unclear { late: None }));
        assert!(matches!(ran(lost(), wait, None).await, Out::Unclear { .. }));
        let refused = async { Err::<u32, _>("refused".to_string()) };
        assert!(matches!(ran(refused, wait, Some(7)).await, Out::Refused(_)));
    }

    /// AC-T1, p. 2: past the wait the send is not cut: the state is read, and the send still
    /// running is handed back with the unclear answer; it ends by itself.
    #[tokio::test(start_paused = true)]
    async fn a_late_send_is_not_cut() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let send = async move {
            tokio::time::sleep(Duration::from_secs(300)).await;
            let _ = tx.send(());
            Ok::<u32, String>(3)
        };
        let out = ran(send, Duration::from_secs(120), None).await;
        let Out::Unclear { late: Some(late) } = out else {
            panic!("not unclear with the send running");
        };
        assert_eq!(late.await.unwrap(), Ok(3));
        assert!(rx.await.is_ok(), "the send ran to its end");
    }

    /// AC-T1: the late line is written once — by the read that shows it or by the late answer,
    /// whichever comes first; the lock forgets what waits.
    #[tokio::test]
    async fn a_late_line_is_written_once() {
        let unclear = Unclear::default();
        let a = unclear.keep(pending("A"));
        let _b = unclear.keep(pending("B"));
        assert!(unclear.waits("A", |_| true));
        assert!(
            unclear.take_if("A", |_| true).is_some(),
            "the read shows it"
        );
        assert!(
            unclear.take(a).is_none(),
            "and the late answer finds it written"
        );
        assert!(unclear.take_if("A", |_| true).is_none());
        unclear.forget();
        assert!(!unclear.waits("B", |_| true), "the lock forgets");
    }

    /// AC-T1: a late success settles its write with one line; a second settle of the same
    /// ticket writes nothing.
    #[tokio::test]
    async fn a_late_answer_writes_its_line_once() {
        let unclear = Unclear::default();
        let dir = std::env::temp_dir().join(format!("evolite-sent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let journal = Arc::new(journal::Journal::new(dir.join("history.json")));
        let ticket = unclear.keep(pending("A"));
        for _ in 0..2 {
            let late = tokio::spawn(async { Ok::<u32, String>(1) });
            let done = Arc::new(tokio::sync::Notify::new());
            let ended = done.clone();
            unclear.settle_late(ticket, late, journal.clone(), move |_, p| async move {
                ended.notify_one();
                own_line(p.kind.unwrap(), p.before, Some(0), p.acting)
            });
            let _ = tokio::time::timeout(Duration::from_millis(200), done.notified()).await;
        }
        assert_eq!(journal.entries().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC-T3: no paid send in `ops/*` is cut by a `tokio::time::timeout`: each goes through
    /// [`run`], which waits with a deadline and never cuts.
    #[test]
    fn no_paid_send_is_cut_by_a_timeout() {
        use crate::real::snapshot::pinned_tests::body;
        let paid = [
            (
                include_str!("ops/identity_keys.rs"),
                "update_keys",
                "chain.update_keys(",
            ),
            (include_str!("ops/tokens.rs"), "send_token", ".send_token("),
            (
                include_str!("ops/token_actions.rs"),
                "claim_token",
                ".claim_token(",
            ),
            (
                include_str!("ops/token_actions.rs"),
                "buy_token",
                ".buy_token(",
            ),
            (
                include_str!("ops/token_manage.rs"),
                "issuer_action",
                ".issuer_act(",
            ),
        ];
        for (source, name, call) in paid {
            let f = body(source, name);
            assert!(f.contains(call), "{name}: no send");
            assert!(f.contains("sent::run("), "{name}: not by the one rule");
            assert!(
                !f.contains("tokio::time::timeout"),
                "{name}: a timeout around the send"
            );
        }
    }

    /// Refused before it went out: only what never left or a node refused itself; a timeout, a
    /// transport error or the cache's duplicate may have gone through.
    #[test]
    fn only_a_refusal_before_the_send_is_a_failure() {
        for refused in [
            chain::NOT_READY,
            "The wallet has no identity",
            "SDK error: state transition broadcast error: Identity has insufficient balance",
            "Protocol error: InvalidStateTransitionSignature",
        ] {
            assert!(refused_before(refused), "{refused}");
        }
        for maybe in [
            "SDK operation timeout 30 secs reached: wait for state transition result",
            "Dapi client error: transport error: deadline has elapsed",
            "Proof verification error: invalid proof",
            "tx already exists in cache",
            "timed out",
        ] {
            assert!(!refused_before(maybe), "{maybe}");
        }
    }
}
