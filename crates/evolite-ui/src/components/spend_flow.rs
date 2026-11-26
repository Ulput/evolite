//! TECH-spend-flow: the one course of a paid window — its password, the wait for the core's
//! answer and the answer itself — written once instead of by hand in each window.
//!
//! - **Idle** → the action can be pressed: [`SpendFlow::command`] gives the `Command::Spend`,
//!   with a fresh request number (`ask`) the core hands back in its `Event::Done`.
//! - **Waiting** → nothing more is sent (a second press pays nothing, the BUG-177/179/186/207
//!   class); only the answer carrying this `ask` is taken, any other — another window's,
//!   another wallet's (BUG-214) — is left.
//! - **Answered** → the figures shown when it was signed and the answer; no action button
//!   (BUG-200): «Close», «View …», or «Again» with an empty form ([`SpendFlow::again`]), not
//!   after an answer that leaves unclear whether it happened.
//!
//! A wrong password goes back to Idle with the form kept, the password field empty and the
//! «wrong password» line (review U3). Lock, a wallet switch and an identity switch forget the
//! flow through the screen's [`crate::screen_state::ScreenState::forget`].

use super::spend_password::SpendPassword;
use evolite_api::{Command, CoreError, Spend};
use std::sync::atomic::{AtomicU64, Ordering};

/// The next request number: unique in the process, so an answer meant for another window,
/// another wallet or another try is never taken.
static NEXT_ASK: AtomicU64 = AtomicU64::new(1);

#[cfg(all(test, demo_core, feature = "dev-tools"))]
thread_local! {
    /// Flows alive on this thread (the guard of review AC-T2: App lists every one).
    static MADE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Flows alive on this thread (tests): made and not dropped yet.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub fn made() -> usize {
    MADE.with(|m| m.get())
}

/// A paid window's flow, whatever it signed (the guard tests of TECH-spend-flow AC-T2).
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub trait AnyFlow {
    /// Nothing sent, nothing waited for.
    fn is_idle(&self) -> bool;
    /// No password typed.
    fn password_empty(&self) -> bool;
    /// As if it was sent and waits, with a password typed.
    fn wait_with_password(&mut self);
}

#[cfg(all(test, demo_core, feature = "dev-tools"))]
impl<S: Default> AnyFlow for SpendFlow<S> {
    fn is_idle(&self) -> bool {
        matches!(self.stage, Stage::Idle)
    }
    fn password_empty(&self) -> bool {
        !self.password.ready(true)
    }
    fn wait_with_password(&mut self) {
        self.password.type_in("typed");
        self.stage = Stage::Waiting {
            ask: 0,
            signed: S::default(),
        };
    }
}

/// Where a paid window is.
#[derive(Debug, PartialEq)]
pub enum Stage<S> {
    Idle,
    Waiting {
        ask: u64,
        signed: S,
    },
    Answered {
        signed: S,
        result: Result<(), CoreError>,
    },
}

/// One paid window's course. `S`: what was shown when it was signed (the figures of
/// BUG-177/179/186), shown again with the answer.
pub struct SpendFlow<S> {
    stage: Stage<S>,
    /// The wallet password typed for it ([`SpendPassword`]).
    pub password: SpendPassword,
}

impl<S> Default for SpendFlow<S> {
    fn default() -> Self {
        #[cfg(all(test, demo_core, feature = "dev-tools"))]
        MADE.with(|m| m.set(m.get() + 1));
        Self {
            stage: Stage::Idle,
            password: SpendPassword::default(),
        }
    }
}

#[cfg(all(test, demo_core, feature = "dev-tools"))]
impl<S> Drop for SpendFlow<S> {
    fn drop(&mut self) {
        MADE.with(|m| m.set(m.get().saturating_sub(1)));
    }
}

impl<S> SpendFlow<S> {
    /// The action may be pressed: nothing sent, or the last one answered «wrong password».
    pub fn can_act(&self) -> bool {
        matches!(self.stage, Stage::Idle)
    }

    /// Sent, its answer awaited.
    pub fn waiting(&self) -> bool {
        matches!(self.stage, Stage::Waiting { .. })
    }

    /// The figures shown when it was signed, while it waits or once answered.
    pub fn signed(&self) -> Option<&S> {
        match &self.stage {
            Stage::Idle => None,
            Stage::Waiting { signed, .. } | Stage::Answered { signed, .. } => Some(signed),
        }
    }

    /// The core's answer, once there.
    pub fn result(&self) -> Option<&Result<(), CoreError>> {
        match &self.stage {
            Stage::Answered { result, .. } => Some(result),
            _ => None,
        }
    }

    /// The command for `op`, with the password when `asked`; `signed` is kept to be shown with
    /// the answer. `None` unless Idle: a second press sends nothing.
    ///
    /// FEAT-0004 FR-7: every spend comes through here, so the tick cannot be forgotten by a
    /// window: one of more than the threshold on mainnet ([`super::large_rule`], set by the app
    /// each frame) that is not ticked — or ticked for another amount — is not sent; the tick
    /// shows above the password, and the button waits for it.
    pub fn command(&mut self, op: Spend, asked: bool, signed: S) -> Option<Command> {
        if !self.can_act() {
            return None;
        }
        self.password.watch(Some(&op));
        if self.password.waits_for_tick() {
            return None;
        }
        let ask = NEXT_ASK.fetch_add(1, Ordering::Relaxed);
        let Command::Spend {
            op,
            password,
            large_confirmed,
            ..
        } = self.password.command(op, asked)
        else {
            unreachable!("a spend's command");
        };
        self.stage = Stage::Waiting { ask, signed };
        Some(Command::Spend {
            op,
            password,
            ask,
            large_confirmed,
        })
    }

    /// The core's answer to a spend: taken only when this flow waits for that very `ask`
    /// (`false`: not this flow's, left for another). A wrong password goes back to Idle with
    /// the field empty and its line shown; anything else is the answer, the password wiped.
    pub fn on_done(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) -> bool {
        let Stage::Waiting { ask: mine, .. } = &self.stage else {
            return false;
        };
        if ask != Some(*mine) {
            return false;
        }
        let Stage::Waiting { signed, .. } = std::mem::replace(&mut self.stage, Stage::Idle) else {
            unreachable!("checked above");
        };
        if matches!(result, Err(CoreError::WrongPassword)) {
            self.password.on_wrong();
        } else if matches!(result, Err(CoreError::LargeSpendNotConfirmed)) {
            // FEAT-0004 review r1 Should 2: nothing was checked or done; the form stays as it
            // is, Idle, with the tick asked for this spend.
            self.password.on_answer(result);
        } else {
            self.password.forget();
            self.stage = Stage::Answered {
                signed,
                result: result.clone(),
            };
        }
        true
    }

    /// The answer, taken out for a window that shows it in a place of its own: the figures
    /// signed and the answer, the flow back to Idle — unless the answer leaves it unclear
    /// whether it happened (BUG-205): then it stays answered, and nothing is sent again until
    /// the window is closed ([`Self::forget`], review U2). `None` unless answered.
    pub fn take_result(&mut self) -> Option<(S, Result<(), CoreError>)>
    where
        S: Clone,
    {
        match std::mem::replace(&mut self.stage, Stage::Idle) {
            Stage::Answered { signed, result } => {
                if !self.can_again_after(&result) {
                    self.stage = Stage::Answered {
                        signed: signed.clone(),
                        result: result.clone(),
                    };
                }
                Some((signed, result))
            }
            other => {
                self.stage = other;
                None
            }
        }
    }

    /// After `result`, the window may send again: the answer says what happened. One that
    /// leaves it unclear (BUG-205) closes only (review U2).
    fn can_again_after(&self, result: &Result<(), CoreError>) -> bool {
        !matches!(result, Err(CoreError::Unclear | CoreError::UnclearWhich))
    }

    /// The number of the request this flow waits for (tests answering it by hand).
    #[cfg(test)]
    pub fn waiting_ask(&self) -> Option<u64> {
        match &self.stage {
            Stage::Waiting { ask, .. } => Some(*ask),
            _ => None,
        }
    }

    /// A stage set directly: the `--screen` frames and tests.
    #[cfg(any(test, feature = "dev-tools"))]
    pub fn set_stage(&mut self, stage: Stage<S>) {
        self.stage = stage;
    }

    /// Lock, wallet or identity switch, the window closed: back to Idle, the password wiped.
    /// An answer still to come is left (its `ask` no longer matches).
    pub fn forget(&mut self) {
        self.stage = Stage::Idle;
        self.password.forget();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FEAT-0004 AC-T7 (the flow every spend goes through): a spend of more than the threshold
    /// is not sent until ticked, and then goes with `large_confirmed`; another amount takes the
    /// tick off; one under it asks none. Mutant: the flow not watching the spend — sent at once.
    #[test]
    fn a_large_spend_waits_for_the_tick() {
        let fees = crate::screens::shielded::tests::fees;
        super::super::set_rule_for_test(Some(super::super::LargeRule {
            threshold: Some(100_000_000),
            fees: fees(),
        }));
        let top = |amount| Spend::TopUpIdentity { amount };
        let mut flow = SpendFlow::<()>::default();
        assert!(
            flow.command(top(150_000_000), false, ()).is_none(),
            "not ticked"
        );
        assert!(flow.password.asks_tick() && !flow.password.ready(false));
        flow.password.tick();
        assert!(flow.password.ready(false));
        // Another amount: the tick is off again.
        assert!(flow.command(top(160_000_000), false, ()).is_none());
        flow.password.tick();
        let Some(Command::Spend {
            large_confirmed, ..
        }) = flow.command(top(160_000_000), false, ())
        else {
            panic!("sent once ticked")
        };
        assert!(large_confirmed);
        // Should 1: the same amount to another recipient takes the tick off.
        let pay = |to: &str| Spend::Send {
            id: 1,
            from: evolite_api::Source::Core,
            to: to.into(),
            amount: 150_000_000,
            note: None,
            rate: None,
            coins: None,
            fee_rate: None,
            fee_shown: None,
        };
        let mut other = SpendFlow::<()>::default();
        assert!(other.command(pay("yAlice"), false, ()).is_none());
        other.password.tick();
        assert!(
            other.command(pay("yBob"), false, ()).is_none(),
            "another recipient"
        );
        assert!(!other.password.ready(false), "unticked");
        // Under the threshold: no tick, sent at once.
        let mut small = SpendFlow::<()>::default();
        let Some(Command::Spend {
            large_confirmed, ..
        }) = small.command(top(100_000_000), false, ())
        else {
            panic!("no tick under the threshold")
        };
        assert!(!large_confirmed && !small.password.asks_tick());
        super::super::set_rule_for_test(None);
    }

    /// FEAT-0004 review r1 Should 2: the core, counting with its own fees, refuses a spend the
    /// form put under the threshold; the tick then shows for that very spend, and ticked it goes
    /// with `large_confirmed`. Another spend does not keep it.
    #[test]
    fn the_cores_refusal_for_want_of_the_tick_shows_the_tick() {
        super::super::set_rule_for_test(Some(super::super::LargeRule {
            threshold: Some(100_000_000),
            fees: crate::screens::shielded::tests::fees(),
        }));
        let near = Spend::TopUpIdentity { amount: 99_000_000 };
        let mut flow = SpendFlow::<()>::default();
        let first = flow
            .command(near.clone(), false, ())
            .expect("under the threshold: sent");
        let ask = ask_of(&first);
        assert!(flow.on_done(Some(ask), &Err(CoreError::LargeSpendNotConfirmed)));
        flow.password.watch(Some(&near));
        assert!(flow.password.asks_tick(), "the tick for the refused spend");
        let _ = flow.take_result();
        assert!(
            flow.command(near.clone(), false, ()).is_none(),
            "not without it"
        );
        flow.password.tick();
        let Some(Command::Spend {
            large_confirmed, ..
        }) = flow.command(near, false, ())
        else {
            panic!("sent ticked")
        };
        assert!(large_confirmed);
        flow.password
            .watch(Some(&Spend::TopUpIdentity { amount: 1 }));
        assert!(!flow.password.asks_tick(), "another spend: no tick");
        super::super::set_rule_for_test(None);
    }

    fn op() -> Spend {
        Spend::FundAddress { amount: 1 }
    }

    fn ask_of(command: &Command) -> u64 {
        match command {
            Command::Spend { ask, .. } => *ask,
            other => panic!("{other:?}"),
        }
    }

    /// AC-T1: the second press while waiting sends nothing; the answer is taken only with its
    /// own number; after it, no action (BUG-200) until «again».
    #[test]
    fn a_second_press_while_waiting_sends_nothing() {
        let mut flow = SpendFlow::<&str>::default();
        assert!(flow.can_act());
        let first = flow.command(op(), false, "0.01").expect("sent");
        assert!(flow.waiting());
        assert_eq!(flow.signed(), Some(&"0.01"));
        assert!(flow.command(op(), false, "0.01").is_none(), "pays once");
        assert!(
            !flow.on_done(Some(ask_of(&first) + 1), &Ok(())),
            "another's answer"
        );
        assert!(!flow.on_done(None, &Ok(())), "an answer to no spend");
        assert!(flow.waiting());
        assert!(flow.on_done(Some(ask_of(&first)), &Ok(())));
        assert_eq!(flow.result(), Some(&Ok(())));
        assert!(!flow.can_act(), "no action after the answer");
        assert!(flow.command(op(), false, "0.01").is_none());
        assert_eq!(flow.take_result(), Some(("0.01", Ok(()))));
        assert!(flow.can_act(), "taken: the form again");
        assert!(flow.signed().is_none());
    }

    /// Two windows: each takes only its own answer, whichever comes first.
    #[test]
    fn each_flow_takes_only_its_own_answer() {
        let (mut a, mut b) = (SpendFlow::<()>::default(), SpendFlow::<()>::default());
        let ask_a = ask_of(&a.command(op(), false, ()).unwrap());
        let ask_b = ask_of(&b.command(op(), false, ()).unwrap());
        assert_ne!(ask_a, ask_b);
        assert!(!a.on_done(Some(ask_b), &Ok(())));
        assert!(b.on_done(Some(ask_b), &Ok(())));
        assert!(a.waiting());
        assert!(a.on_done(Some(ask_a), &Ok(())));
    }

    /// AC-T1, review U3: a wrong password goes back to the form, the field empty and its line
    /// shown; any other answer wipes the password and leaves no action.
    #[test]
    fn a_wrong_password_keeps_the_form_with_the_field_empty() {
        let mut flow = SpendFlow::<()>::default();
        let ask = ask_of(&flow.command(op(), true, ()).unwrap());
        assert!(flow.on_done(Some(ask), &Err(CoreError::WrongPassword)));
        assert!(flow.can_act(), "the form again");
        assert!(flow.password.is_wrong());
        assert!(!flow.password.ready(true), "the field is empty");
        let ask = ask_of(&flow.command(op(), true, ()).unwrap());
        assert!(flow.on_done(Some(ask), &Err(CoreError::NotEnough)));
        assert!(!flow.can_act());
        assert!(!flow.password.is_wrong());
    }

    /// The lock or a switch: whatever the stage, back to the form, and an answer still to come
    /// is not taken.
    #[test]
    fn forgetting_leaves_a_late_answer_untaken() {
        let mut flow = SpendFlow::<()>::default();
        let ask = ask_of(&flow.command(op(), true, ()).unwrap());
        flow.forget();
        assert!(flow.can_act());
        assert!(!flow.on_done(Some(ask), &Ok(())));
        assert!(flow.result().is_none());
    }

    /// Review U2: after an answer that leaves it unclear whether it happened, the window sends
    /// nothing again until it is closed.
    #[test]
    fn an_unclear_answer_allows_no_second_send() {
        for unclear in [CoreError::Unclear, CoreError::UnclearWhich] {
            let mut flow = SpendFlow::<()>::default();
            let ask = ask_of(&flow.command(op(), false, ()).unwrap());
            assert!(flow.on_done(Some(ask), &Err(unclear.clone())));
            assert_eq!(flow.take_result(), Some(((), Err(unclear))));
            assert!(!flow.can_act());
            assert!(flow.command(op(), false, ()).is_none(), "no second send");
            flow.forget();
            assert!(flow.can_act(), "closed: a new window");
        }
    }

    /// The files that may build a `Command::Spend`: the flow, its password, and Send's own
    /// two (answered by `Event::Sent` with its own number).
    const SPEND_BUILDERS: &[&str] = &[
        "components/spend_flow.rs",
        "components/spend_password.rs",
        "screens/send/confirm.rs",
        // FEAT-0056: the last look of a payment to several, the same form's one send.
        "screens/send/many.rs",
        "screens/send/mixed.rs",
    ];

    /// Other files allowed to build one — `(file, why)`. Empty: every paid window goes
    /// through [`SpendFlow`].
    const SPEND_BUILT_ELSEWHERE: &[(&str, &str)] = &[(
        "app/dev.rs",
        "dev-tools presets only (W9 workshop-sell/reprice/unprice): signs the demo sale as a person would, to show its plate; not in a release build",
    )];

    /// TECH-spend-flow review r1, Should: a window that builds its `Command::Spend` itself
    /// goes around [`SpendFlow`] (a second press sends twice, another's answer is taken). Read
    /// from the sources: outside the tests, only the files above build one; a new one fails
    /// here with its file, and a listed exception gone fails too.
    #[test]
    fn only_the_flow_builds_a_spend() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let needle = ["Command::Spend", "{"].concat();
        let mut found = Vec::new();
        let mut dirs = vec![src.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).expect("a source folder").flatten() {
                let path = entry.path();
                let rel = path
                    .strip_prefix(&src)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if path.is_dir() {
                    // The kittests are tests through and through.
                    if rel != "app/ui_tests" {
                        dirs.push(path);
                    }
                    continue;
                }
                if path.extension().is_none_or(|x| x != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("a source file");
                // The code before the file's own test module, without spaces and line breaks.
                let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                let code = compact.split("#[cfg(test)]mod").next().unwrap_or_default();
                if code.contains(&needle) {
                    found.push(rel);
                }
            }
        }
        let allowed: Vec<&str> = SPEND_BUILDERS
            .iter()
            .copied()
            .chain(SPEND_BUILT_ELSEWHERE.iter().map(|(f, _)| *f))
            .collect();
        let new: Vec<_> = found
            .iter()
            .filter(|f| !allowed.contains(&f.as_str()))
            .collect();
        assert!(
            new.is_empty(),
            "a Command::Spend built outside SpendFlow — use the flow, or list it with its reason: {new:?}"
        );
        let gone: Vec<_> = allowed
            .iter()
            .filter(|a| !found.iter().any(|f| f == *a))
            .collect();
        assert!(gone.is_empty(), "listed but builds none: {gone:?}");
    }
}
