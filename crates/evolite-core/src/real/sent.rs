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

/// Why a write has no answer: refused before it went out, or not known.
pub(in crate::real) enum NoAnswer<T, E> {
    Refused(E),
    /// Not known; with the send still running, if it is.
    Unclear(Option<tokio::task::JoinHandle<Result<T, E>>>),
}

/// How a token write landed (BUG-205): its own proved answer, or a proved read showing it — the
/// identity's nonce with the token's contract moved, and the balance or the supply (the value
/// read) past its bound.
pub(in crate::real) enum Landed<T> {
    Answer(T),
    Read(u64),
}

impl<T, E> Outcome<T, TokenState, E> {
    /// The answer of a token write whose token is read again.
    pub(in crate::real) fn landed(self) -> Result<Landed<T>, NoAnswer<T, E>> {
        match self {
            Outcome::Done(done) => Ok(Landed::Answer(done)),
            // default-ok: shown only when the value was read: never none
            Outcome::Shown(state) => Ok(Landed::Read(state.value.unwrap_or_default())),
            Outcome::Refused(e) => Err(NoAnswer::Refused(e)),
            Outcome::Unclear { late } => Err(NoAnswer::Unclear(late)),
        }
    }
}

/// BUG-205 (review r1 S1): a write Platform refused may still have taken a fee. The credits
/// read again: lower than `before`, a «not done» line with what went (`token` names it).
#[allow(clippy::too_many_arguments)]
pub(in crate::real) async fn refused_line(
    chain: &Chain,
    wallet: &platform_wallet::PlatformWallet,
    id: Identifier,
    before: Option<u64>,
    token: String,
    acting: String,
    journal: &journal::Journal,
) {
    let after = chain.identity_credits(wallet, &id, true).await;
    if super::spent_credits(before, after).is_some() {
        crate::diag::platform_failed("refused_fee", "Charged");
        journal.add(own_line(
            journal::Kind::NotDone { token },
            before,
            after,
            Some(acting),
        ));
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
    // TECH-platform-errors: whatever [`super::platform_failure`] reads as a refusal of its own.
    text.to_lowercase()
        .contains("state transition broadcast error")
        || !matches!(
            super::platform_failure(text.to_owned()),
            CoreError::Failed(_)
        )
}

/// What a read that shows an unclear write done looks for.
#[derive(Clone, Debug)]
pub(in crate::real) enum Watch {
    /// A key change (BUG-184): the identity read again shows the plan.
    Keys(key_plan::KeyPlan),
    /// A contact request sent again after the DashPay pair was replaced (FEAT-0020-resend):
    /// known only by its late answer; while it may land, no other request of its identity goes
    /// ([`Unclear::requests_on_their_way`]).
    ContactRequest,
    /// A token write (BUG-205): signed as `nonce + 1` (or later) of the identity with the
    /// token's `contract`; a proved read of that nonce says whether any write of it landed, and
    /// `shows` (when the write moves a value) whether it was this one.
    Token {
        token: String,
        contract: String,
        nonce: u64,
        shows: Option<TokenShows>,
    },
    /// Only the send's own late answer settles it.
    Answer,
    /// BUG-243: a name claimed (or registered) by the identity: a read of the name or its
    /// contest showing the identity settles it.
    Name { label: String },
    /// BUG-243: the identity's «where my identity is confirmed» link of a name.
    Link { label: String },
    /// BUG-243: the identity's DashPay profile.
    Profile,
    /// FEAT-0038 module V1 (r1 Must 1): a contract registered by identity `owner`, its id
    /// made from the identity's nonce after `nonce`; a proved read of those ids settles it.
    Contract { owner: String, nonce: u64 },
}

/// What a proved read of a token shows of a write of it (BUG-205): its balance on the identity,
/// or its supply, past a bound the write reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::real) enum TokenShows {
    /// A send: the balance went down by the amount.
    BalanceAtMost(u64),
    /// A purchase or a claim: the balance went up.
    BalanceAtLeast(u64),
    /// A mint: the supply went up by the amount.
    SupplyAtLeast(u64),
    /// A burn: the supply went down by the amount.
    SupplyAtMost(u64),
}

impl TokenShows {
    /// Whether `read` (the balance or the supply) shows the write.
    pub(in crate::real) fn met(self, read: u64) -> bool {
        match self {
            TokenShows::BalanceAtMost(b) | TokenShows::SupplyAtMost(b) => read <= b,
            TokenShows::BalanceAtLeast(b) | TokenShows::SupplyAtLeast(b) => read >= b,
        }
    }
}

/// How long nothing of the identity with a token's contract must have landed since an
/// unclear write's send ended before it is taken as not landed (BUG-205 r1 M1, p. 3): a
/// transition may still wait in a node's queue, and a repeat signed with the next nonce would
/// let both land.
///
/// The lock keeps a write inside its quiet ([`Unclear::forget`], review r2 M1): the SDK and its
/// nonce cache outlive the lock, so a repeat after unlock would be signed with the next nonce.
/// A restart is safe: the SDK built anew reads the nonce from Platform and signs the same one
/// again, so at most one of the two lands (r2 C1). Hence what waits is not kept on disk «for
/// safety»: kept, it would block a write that cannot land twice.
pub(in crate::real) const QUIET: Duration = Duration::from_secs(120);

/// BUG-243 (7): past the quiet, «nothing landed» needs two reads this far apart: a node behind
/// answers one, and with Platform's blocks empty a few of them may take longer than the quiet.
pub(in crate::real) const NO_GAP: Duration = Duration::from_secs(30);

/// A token write's state read again, proved (BUG-205 r1 M1): the identity's nonce with the
/// token's contract — moved by its own writes of that contract only, from this device or
/// another, never by anyone else's — and, when the write moves one, the balance or the supply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::real) struct TokenState {
    pub(in crate::real) nonce: u64,
    pub(in crate::real) value: Option<u64>,
}

impl TokenState {
    /// Whether it shows the write signed after nonce `n0` landed: a write of the identity with
    /// the contract did (the nonce moved), and the value is past the write's bound. A value
    /// alone is no proof: anyone's transfer of the same token moves a balance.
    pub(in crate::real) fn shows(&self, n0: u64, shows: Option<TokenShows>) -> bool {
        self.nonce > n0 && shows.zip(self.value).is_some_and(|(s, v)| s.met(v))
    }
}

/// [`TokenState`] of `me` (`me_id`) with `token` of `contract`: the nonce, and the value `shows`
/// looks at. `None` when a read failed.
pub(in crate::real) async fn token_state(
    chain: &Chain,
    me_id: &Identifier,
    me: &str,
    contract: &str,
    token: &str,
    shows: Option<TokenShows>,
) -> Option<TokenState> {
    let nonce = chain.contract_nonce(me_id, contract).await.ok()?;
    let value = match shows {
        Some(shows) => Some(token_read(chain, me, token, shows).await?),
        None => None,
    };
    Some(TokenState { nonce, value })
}

/// [`token_state`] when the nonce before the write (`n0`) is known; else no read, and the write
/// is known only by its own answer.
pub(in crate::real) async fn token_state_if(
    chain: &Chain,
    me_id: &Identifier,
    me: &str,
    contract: &str,
    token: &str,
    n0: Option<u64>,
    shows: Option<TokenShows>,
) -> Option<TokenState> {
    n0?;
    token_state(chain, me_id, me, contract, token, shows).await
}

/// What waits for a token write not known to have gone through: by the nonce before it when that
/// was read, else only by its late answer.
pub(in crate::real) fn token_watch(
    token: &str,
    contract: &str,
    n0: Option<u64>,
    shows: Option<TokenShows>,
) -> Watch {
    match n0 {
        Some(nonce) => Watch::Token {
            token: token.to_owned(),
            contract: contract.to_owned(),
            nonce,
            shows,
        },
        None => Watch::Answer,
    }
}

/// A proved read of what `shows` looks at: `identity`'s balance of `token` (none held is 0), or
/// the token's supply. `None` when the read failed.
pub(in crate::real) async fn token_read(
    chain: &Chain,
    identity: &str,
    token: &str,
    shows: TokenShows,
) -> Option<u64> {
    match shows {
        TokenShows::BalanceAtMost(_) | TokenShows::BalanceAtLeast(_) => {
            let facts = chain
                .token_facts(identity, &[token.to_owned()])
                .await
                .ok()?;
            // default-ok: proved facts: a token not held is 0
            Some(facts.balances.get(token).copied().unwrap_or(0))
        }
        TokenShows::SupplyAtLeast(_) | TokenShows::SupplyAtMost(_) => {
            chain.token_supply(token).await.ok()
        }
    }
}

/// A paid write sent whose outcome is not known ([`CoreError::Unclear`]): what its History line needs once a
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
    /// Its send still runs past the wait (BUG-205: nothing of it is repeated meanwhile).
    running: bool,
    /// When its send ended, for [`QUIET`].
    ended: Option<std::time::Instant>,
    /// BUG-243 (7): when a read past its quiet first showed nothing landed.
    no_since: Option<std::time::Instant>,
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
            running: false,
            ended: None,
            no_since: None,
        }
    }

    /// BUG-243 (7): how long since a read past its quiet first showed nothing landed.
    pub(in crate::real) fn no_for(&self) -> Option<Duration> {
        self.no_since.map(|at| at.elapsed())
    }

    pub(in crate::real) fn ticket(&self) -> u64 {
        self.ticket
    }

    pub(in crate::real) fn running(&self) -> bool {
        self.running
    }

    /// How long since its send ended; `None` while it runs.
    pub(in crate::real) fn quiet_for(&self) -> Option<Duration> {
        self.ended.map(|at| at.elapsed())
    }
    /// Tests: as if its send ended `ago`, and a first «no» `no_ago` (if given) ago.
    #[cfg(test)]
    pub(in crate::real) fn aged(&mut self, ago: Duration, no_ago: Option<Duration>) {
        let now = std::time::Instant::now();
        self.running = false;
        self.ended = now.checked_sub(ago);
        self.no_since = no_ago.and_then(|d| now.checked_sub(d));
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

    /// Whether a change of `identity`'s keys not known to have gone through — App Connect's or
    /// the keys screen's — may still land (BUG-208 review r1 S1): its send runs, or ended less
    /// than [`QUIET`] ago. A new one meanwhile would pay again for keys that may be on their way.
    pub(in crate::real) fn keys_on_their_way(&self, identity: &str) -> bool {
        self.on_their_way(identity, |p| {
            matches!(p.watch, Watch::Keys(_))
                || matches!(p.kind, Some(journal::Kind::AppConnect { .. }))
        })
    }

    /// BUG-243: whether a write of `identity` that `of` picks, not known to have gone through,
    /// may still land — its send runs, or ended less than [`QUIET`] ago. «Not seen» by a read
    /// meanwhile is not «not done»: a paid repeat waits ([`Self::keys_on_their_way`] is one).
    pub(in crate::real) fn on_their_way(
        &self,
        identity: &str,
        of: impl Fn(&Pending) -> bool,
    ) -> bool {
        self.held().1.iter().any(|p| {
            p.identity == identity
                && of(p)
                && (p.running || p.quiet_for().is_some_and(|q| q < QUIET))
        })
    }

    /// BUG-243: a write sent and ended with an error that may have come after it went out, and a
    /// read since that does not show it: kept as not known, its quiet from now. No send runs.
    /// A write kept on the disk across a restart (FEAT-0038 W0–W4 r2 Must 1: a registration's
    /// History line «in transit»): its send ended `ago`. Older than this computer has run, it
    /// counts as past its quiet, never as just sent.
    pub(in crate::real) fn restore(&self, mut pending: Pending, ago: Duration) {
        let now = std::time::Instant::now();
        pending.running = false;
        pending.ended = now
            .checked_sub(ago)
            .or_else(|| now.checked_sub(QUIET + NO_GAP))
            .or(Some(now));
        self.keep(pending);
    }

    pub(in crate::real) fn not_seen(&self, pending: Pending) -> CoreError {
        let mut pending = pending;
        pending.running = false;
        pending.ended = Some(std::time::Instant::now());
        self.keep(pending);
        CoreError::Unclear
    }

    /// The first write of `identity` on `token` that waits, as it is (BUG-205).
    /// Whether a contact request of `identity` sent again may still land (FEAT-0020-resend,
    /// review condition 5): its send runs, or ended less than [`QUIET`] ago. Another one now
    /// could be refused for its reference, at a fee.
    pub(in crate::real) fn requests_on_their_way(&self, identity: &str) -> bool {
        self.held().1.iter().any(|p| {
            p.identity == identity
                && matches!(p.watch, Watch::ContactRequest)
                && (p.running || p.quiet_for().is_some_and(|q| q < QUIET))
        })
    }

    pub(in crate::real) fn of_token(&self, identity: &str, token: &str) -> Option<Pending> {
        self.held()
            .1
            .iter()
            .find(|p| {
                p.identity == identity
                    && matches!(&p.watch, Watch::Token { token: t, .. } if t == token)
            })
            .cloned()
    }

    /// BUG-243 (7): a read past the quiet of `ticket` showed nothing landed; the first such
    /// read's time is kept (a second, [`NO_GAP`] later, may decide).
    pub(in crate::real) fn saw_no(&self, ticket: u64) {
        if let Some(p) = self.held().1.iter_mut().find(|p| p.ticket == ticket) {
            p.no_since.get_or_insert_with(std::time::Instant::now);
        }
    }

    /// The contract registration of `identity` not known to have gone through, as it is.
    pub(in crate::real) fn contract_of(&self, identity: &str) -> Option<Pending> {
        self.held()
            .1
            .iter()
            .find(|p| {
                p.identity == identity
                    && matches!(&p.watch, Watch::Contract { owner, .. } if owner == identity)
            })
            .cloned()
    }

    /// The write of `ticket`, taken out if it still waits.
    pub(in crate::real) fn take(&self, ticket: u64) -> Option<Pending> {
        let mut held = self.held();
        let at = held.1.iter().position(|p| p.ticket == ticket)?;
        Some(held.1.remove(at))
    }

    /// The ending of a write whose outcome is not known: kept as `pending`; the send still
    /// running (`late`), if its answer comes and is a success, writes its line by `line`. The
    /// answer for the window, [`CoreError::Unclear`].
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
        let mut pending = pending;
        pending.running = late.is_some();
        pending.ended = late.is_none().then(std::time::Instant::now);
        let ticket = self.keep(pending);
        if let Some(late) = late {
            self.settle_late(ticket, late, journal, line);
        }
        CoreError::Unclear
    }

    /// The lock: what was not known is forgotten (BUG-184) — but a write whose send still runs
    /// is kept: its late success writes its History line even after the lock (BUG-205, review
    /// r1 S2: paid and gone, a line lost for good otherwise), and so is one whose send ended
    /// less than [`QUIET`] ago (review r2 M1: a repeat after unlock is signed with the next nonce,
    /// and both could land).
    pub(in crate::real) fn forget(&self) {
        self.held()
            .1
            .retain(|p| p.running || p.quiet_for().is_some_and(|q| q < QUIET));
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
            let ended = late.await;
            if let Ok(Ok(answer)) = ended {
                if let Some(pending) = unclear.take(ticket) {
                    crate::diag::platform_failed("send_late", "Done");
                    let line = line(answer, pending).await;
                    // Review r1 C2: the wallet deleted meanwhile — its folder is not made again.
                    if journal.folder_there() {
                        journal.add(line);
                    }
                }
            } else if let Some(p) = unclear.held().1.iter_mut().find(|p| p.ticket == ticket) {
                // Ended without an answer: still not known, but no longer on its way.
                p.running = false;
                p.ended = Some(std::time::Instant::now());
            }
        });
    }
}

impl Watch {
    #[cfg(test)]
    fn clone_kind(&self) -> &'static str {
        match self {
            Watch::Keys(_) => "keys",
            Watch::ContactRequest => "request",
            Watch::Token { .. } => "token",
            Watch::Answer => "answer",
            Watch::Name { .. } => "name",
            Watch::Link { .. } => "link",
            Watch::Profile => "profile",
            Watch::Contract { .. } => "contract",
        }
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

    /// BUG-205: what a token read shows of each kind of write.
    #[test]
    fn a_token_read_shows_a_write_past_its_bound() {
        assert!(TokenShows::BalanceAtMost(90).met(90));
        assert!(!TokenShows::BalanceAtMost(90).met(100), "not sent yet");
        assert!(TokenShows::BalanceAtLeast(105).met(110));
        assert!(!TokenShows::BalanceAtLeast(105).met(100), "not bought yet");
        assert!(TokenShows::SupplyAtLeast(1_000).met(1_000));
        assert!(TokenShows::SupplyAtMost(900).met(899));
    }

    /// BUG-205 r1 M1: a token write is shown landed only when the identity's nonce with the
    /// contract moved **and** the value is past its bound. A balance alone — risen by anyone's
    /// transfer of the same token — shows nothing. Mutant: judging by the balance as r1 did.
    #[test]
    fn a_token_write_is_shown_by_its_nonce_and_its_value() {
        let bought = Some(TokenShows::BalanceAtLeast(105));
        let state = |nonce, value| TokenState {
            nonce,
            value: Some(value),
        };
        assert!(
            state(8, 110).shows(7, bought),
            "nonce moved, balance shows it"
        );
        assert!(
            !state(7, 110).shows(7, bought),
            "balance risen by an incoming transfer"
        );
        assert!(
            !state(8, 100).shows(7, bought),
            "another write of ours, not this one"
        );
        let none = TokenState {
            nonce: 8,
            value: None,
        };
        assert!(
            !none.shows(7, None),
            "a write with no value to read: never by its value"
        );
        assert_eq!(token_watch("T", "C", None, bought).clone_kind(), "answer");
    }

    /// BUG-205: a token write that waits is found by its token; its send, running past the
    /// wait, is marked so until it ends; one ended without an answer still waits, not running.
    #[tokio::test]
    async fn a_token_write_waits_marked_while_its_send_runs() {
        let unclear = Unclear::default();
        let journal = {
            let dir = std::env::temp_dir().join(format!("evolite-sent-t-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(journal::Journal::new(dir.join("history.json")))
        };
        let watch = token_watch("T", "C", Some(3), Some(TokenShows::BalanceAtLeast(5)));
        let pending = Pending::new("A".into(), watch, Some(kind()), None, None);
        let (go, gate) = tokio::sync::oneshot::channel::<()>();
        let late = tokio::spawn(async move {
            let _ = gate.await;
            Err::<u32, String>("timed out".into())
        });
        let _ = unclear.not_known(pending, Some(late), journal, |_, p| async move {
            own_line(p.kind.unwrap(), None, None, None)
        });
        assert!(unclear.of_token("B", "T").is_none(), "another identity");
        assert!(unclear.of_token("A", "U").is_none(), "another token");
        assert!(unclear.of_token("A", "T").unwrap().running());
        let _ = go.send(());
        for _ in 0..100 {
            if !unclear.of_token("A", "T").unwrap().running() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("still marked running after its send ended");
    }

    /// BUG-208 review r1 S1: a change of keys (App Connect's or the keys screen's) not known to
    /// have gone through holds a new one of the same identity while its send runs or its quiet
    /// lasts; not another identity's, not a token write's, not one past its quiet.
    /// Mutants: no check (`false`), or `running` alone (the quiet forgotten).
    #[test]
    fn a_key_change_on_its_way_holds_the_next() {
        let connect = || journal::Kind::AppConnect {
            app: "App".into(),
            renewal: false,
        };
        let ended = |identity: &str, watch: Watch, kind, ago: Duration| {
            let mut p = Pending::new(identity.into(), watch, kind, None, None);
            p.ended = Some(std::time::Instant::now().checked_sub(ago).unwrap());
            p
        };
        let unclear = Unclear::default();
        assert!(!unclear.keys_on_their_way("A"), "nothing waits");
        let mut running = Pending::new("A".into(), Watch::Answer, Some(connect()), None, None);
        running.running = true;
        let ticket = unclear.keep(running);
        assert!(unclear.keys_on_their_way("A"), "its send runs");
        assert!(!unclear.keys_on_their_way("B"), "another identity");
        unclear.take(ticket);
        let ticket = unclear.keep(ended(
            "A",
            Watch::Answer,
            Some(connect()),
            Duration::from_secs(30),
        ));
        assert!(unclear.keys_on_their_way("A"), "ended inside its quiet");
        unclear.take(ticket);
        unclear.keep(ended(
            "A",
            Watch::Answer,
            Some(connect()),
            QUIET + Duration::from_secs(1),
        ));
        assert!(!unclear.keys_on_their_way("A"), "past its quiet");
        let token = token_watch("T", "C", Some(3), Some(TokenShows::BalanceAtLeast(5)));
        unclear.keep(ended("C", token, Some(kind()), Duration::from_secs(30)));
        assert!(
            !unclear.keys_on_their_way("C"),
            "a token write is not a key change"
        );
    }

    /// BUG-205 review r2 M1: the lock keeps a token write whose send ended less than [`QUIET`]
    /// ago — still found by its token — and forgets it once the quiet has passed. Mutant: the
    /// old `retain(|p| p.running)` loses it at once.
    #[test]
    fn the_lock_keeps_a_write_inside_its_quiet() {
        let unclear = Unclear::default();
        let watch = || token_watch("T", "C", Some(3), Some(TokenShows::BalanceAtLeast(5)));
        let ended = |ago: Duration| {
            let mut p = Pending::new("A".into(), watch(), Some(kind()), None, None);
            p.ended = Some(std::time::Instant::now().checked_sub(ago).unwrap());
            p
        };
        unclear.keep(ended(Duration::from_secs(30)));
        unclear.forget();
        assert!(
            unclear.of_token("A", "T").is_some(),
            "inside the quiet: kept"
        );
        let ticket = unclear.of_token("A", "T").unwrap().ticket();
        unclear.take(ticket);
        unclear.keep(ended(QUIET + Duration::from_secs(1)));
        unclear.forget();
        assert!(
            unclear.of_token("A", "T").is_none(),
            "past the quiet: forgotten"
        );
    }

    /// BUG-205, review r1 S2: the lock while a send still runs past the wait keeps its write;
    /// its late success, after the lock, writes the one line. A write whose send has ended is
    /// forgotten as before. Mutant: `forget` clearing all, no line.
    #[tokio::test]
    async fn a_late_success_after_the_lock_keeps_its_line() {
        let unclear = Unclear::default();
        let dir = std::env::temp_dir().join(format!("evolite-sent-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let journal = Arc::new(journal::Journal::new(dir.join("history.json")));
        let (go, gate) = tokio::sync::oneshot::channel::<()>();
        let late = tokio::spawn(async move {
            let _ = gate.await;
            Ok::<u32, String>(1)
        });
        let _ = unclear.not_known(
            pending("A"),
            Some(late),
            journal.clone(),
            |_, p| async move { own_line(p.kind.unwrap(), p.before, Some(0), p.acting) },
        );
        unclear.keep(pending("B"));
        unclear.forget();
        assert!(unclear.waits("A", |_| true), "its send runs: kept");
        assert!(!unclear.waits("B", |_| true), "ended: forgotten");
        let _ = go.send(());
        for _ in 0..100 {
            if journal.entries().len() == 1 {
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no line after the lock");
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
            (
                include_str!("ops/app_connect2.rs"),
                "approve_connect2",
                ".send_key_update(",
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

    /// TokenReader R1 (review FEAT-0031-C5 r1, Must): every token write takes its contract and
    /// position from `token_to_write` — proved in this run, the position the contract's own —
    /// never from the wallet's list on the disk (`token.position`, `fresh_contract`). The chain's
    /// writes take a `WriteToken`, which only `token_to_write` makes.
    #[test]
    fn every_token_write_takes_a_proved_contract_and_position() {
        use crate::real::snapshot::pinned_tests::body;
        let writes = [
            (include_str!("ops/tokens.rs"), "send_token"),
            (include_str!("ops/token_actions.rs"), "claim_token"),
            (include_str!("ops/token_actions.rs"), "buy_token"),
            (include_str!("ops/token_manage.rs"), "issuer_action"),
        ];
        for (source, name) in writes {
            let f = body(source, name);
            assert!(
                f.contains(".token_to_write("),
                "{name}: not through token_to_write"
            );
            if name != "claim_token" {
                // Review TokenReader r1, S1: an amount made with the list's decimals.
                assert!(f.contains("same_decimals("), "{name}: decimals not checked");
            }
            for unproved in ["token.position", "fresh_contract("] {
                assert!(!f.contains(unproved), "{name}: {unproved}");
            }
        }
        let chain = [
            include_str!("../../../evolite-chain/src/spv/chain/tokens.rs"),
            include_str!("../../../evolite-chain/src/spv/chain/token_actions.rs"),
            include_str!("../../../evolite-chain/src/spv/chain/token_manage.rs"),
        ]
        .concat();
        // Each chain write's parameters: up to where its return type starts.
        for write in ["send_token", "claim_token", "buy_token", "issuer_act"] {
            let start = chain
                .find(&format!("pub async fn {write}("))
                .unwrap_or_else(|| panic!("no chain {write}"));
            let rest = &chain[start..];
            let params = &rest[..rest.find(") ->").expect("a return type")];
            assert!(
                params.contains("token: &WriteToken"),
                "chain {write}: not a WriteToken"
            );
            assert!(
                !params.contains("position"),
                "chain {write}: a position of its own"
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
