//! FEAT-0048 step 2: fully mixed coins to Platform — a new identity (path M), the shown
//! identity's top-up (UC-7) and «Shield» (UC-8). One piece for the three screens: the core's plan
//! for an amount (`Command::PreviewMixedLock`), what the review says of it (the coins, the Core
//! fee, «exactly, no excess» or the excess and its yes above 0.01, И7) and «Exactly from
//! denominations: {}» with «Take» (R-M1).

use crate::components::{Assets, ButtonKind, button, kv_row};
use crate::theme::{Weight, mono};
use egui::Ui;
use evolite_api::{Command, CoreError, Duffs, MixedLockQuote, MixedTo};

/// Each screen's asks apart: the one `Event::MixedLockPreview` goes to all three, and each takes
/// only the answer to its own last ask.
pub const CREATE: u64 = 1 << 40;
pub const TOP_UP: u64 = 2 << 40;
pub const POOL: u64 = 3 << 40;

/// The core's plan for a payment from fully mixed coins to Platform, and the person's yes to its
/// excess.
/// How long an amount stays as typed before the core is asked for its plan (r2 Should): not
/// one ask per digit.
pub const QUIET: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Default)]
pub struct MixedLock {
    /// The last ask's number: only its answer is taken.
    ask: u64,
    /// What is to be asked, and since when it is the one wanted.
    pending: Option<((MixedTo, Duffs), std::time::Instant)>,
    asked: Option<(MixedTo, Duffs)>,
    pub quote: Option<Result<MixedLockQuote, CoreError>>,
    /// The excess agreed to (И7: above 0.01 DASH). Another plan's excess takes it off.
    pub agreed: Option<Duffs>,
}

impl MixedLock {
    /// Asks the core for the plan of `amount` to `to` — once for each, and only once it has stayed
    /// so for [`QUIET`] (`now`); a new one takes the yes off. `owner`: the screen's range
    /// ([`CREATE`], [`TOP_UP`], [`POOL`]). While one waits, [`MixedLock::waiting`] says so.
    pub fn ask(
        &mut self,
        owner: u64,
        to: MixedTo,
        amount: Duffs,
        now: std::time::Instant,
    ) -> Option<Command> {
        let want = (to, amount);
        if self.asked == Some(want) {
            self.pending = None;
            return None;
        }
        match self.pending {
            Some((w, since)) if w == want => {
                if now.saturating_duration_since(since) < QUIET {
                    return None;
                }
            }
            _ => {
                self.pending = Some((want, now));
                return None;
            }
        }
        self.pending = None;
        self.ask = owner + (self.ask & ((1 << 40) - 1)) + 1;
        self.asked = Some((to, amount));
        self.quote = None;
        self.agreed = None;
        Some(Command::PreviewMixedLock {
            ask: self.ask,
            to,
            amount,
        })
    }

    /// The core's plan: this screen's last ask only. Another excess than the one agreed takes the
    /// yes off.
    pub fn on_preview(&mut self, ask: u64, result: &Result<MixedLockQuote, CoreError>) {
        if ask != self.ask || self.asked.is_none() {
            return;
        }
        if let Ok(q) = result
            && self.agreed.is_some_and(|agreed| agreed != q.excess)
        {
            self.agreed = None;
        }
        self.quote = Some(result.clone());
    }

    /// The plan for exactly `to` and `amount`, once the core has given it.
    pub fn quote_for(
        &self,
        to: MixedTo,
        amount: Duffs,
    ) -> Option<&Result<MixedLockQuote, CoreError>> {
        (self.asked == Some((to, amount)))
            .then_some(self.quote.as_ref())
            .flatten()
    }

    /// The plan, when the coins make it.
    pub fn plan(&self, to: MixedTo, amount: Duffs) -> Option<&MixedLockQuote> {
        self.quote_for(to, amount).and_then(|r| r.as_ref().ok())
    }

    /// The coins make it and, with an excess above 0.01, it was agreed to.
    pub fn ready(&self, to: MixedTo, amount: Duffs) -> bool {
        self.plan(to, amount)
            .is_some_and(|q| !q.confirm || self.agreed == Some(q.excess))
    }

    /// The spend's `accept_excess`: the excess agreed to, else 0 (И7, the core refuses any other).
    pub fn accept_excess(&self, to: MixedTo, amount: Duffs) -> Duffs {
        self.plan(to, amount)
            .filter(|q| q.confirm && self.agreed == Some(q.excess))
            .map_or(0, |q| q.excess)
    }

    /// An amount waits for [`QUIET`] before it is asked.
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// [`MixedLock::ask`] now, with a frame after [`QUIET`] while an amount waits: what each
    /// screen calls.
    pub fn ask_in(
        &mut self,
        ctx: &egui::Context,
        owner: u64,
        to: MixedTo,
        amount: Duffs,
    ) -> Option<Command> {
        let command = self.ask(owner, to, amount, std::time::Instant::now());
        if self.waiting() {
            ctx.request_repaint_after(QUIET);
        }
        command
    }

    /// Forgotten: the next look asks again.
    pub fn forget(&mut self) {
        self.pending = None;
        self.asked = None;
        self.quote = None;
        self.agreed = None;
    }

    /// A plan put in without asking: the `--screen` frames and tests (the demo answers
    /// «not enough» to every ask).
    #[cfg(feature = "dev-tools")]
    pub fn set(&mut self, to: MixedTo, amount: Duffs, result: Result<MixedLockQuote, CoreError>) {
        self.asked = Some((to, amount));
        self.quote = Some(result);
    }
}

/// Where an excess goes, said in its words (R-M1: the credits; R-M2: the pool).
pub fn excess_words(to: MixedTo, excess: Duffs) -> String {
    let amount = crate::format::amount(excess);
    match to {
        MixedTo::Pool => format!(tr!("Another {} goes into the pool"), amount),
        MixedTo::Identity | MixedTo::TopUp => {
            format!(tr!("Another {} goes to the identity's credits"), amount)
        }
    }
}

/// The review's lines of a plan: the coins, the Core fee, and «exactly, no excess» or the excess
/// — with its tick above 0.01 DASH (И7). Returns whether it is agreed to (or needs no yes).
pub fn plan_lines(
    ui: &mut Ui,
    a: &mut Assets,
    lock: &mut MixedLock,
    to: MixedTo,
    amount: Duffs,
) -> bool {
    let p = a.p;
    let Some(q) = lock.plan(to, amount).cloned() else {
        return false;
    };
    let value = mono(13.0, Weight::Medium);
    kv_row(
        ui,
        &p,
        tr!("From mixed coins"),
        &crate::i18n::t(&crate::text::plural(q.coins, tr!("coin"), tr!("coins"))),
        value.clone(),
    );
    kv_row(
        ui,
        &p,
        tr!("Core fee"),
        &format!("≈ {}", crate::format::amount(q.fee)),
        value,
    );
    if q.exact || q.excess <= 0 {
        ui.add(crate::components::para(
            tr!("exactly, no excess"),
            12.0,
            Weight::Medium,
            p.muted,
        ));
        return true;
    }
    let words = excess_words(to, q.excess);
    if !q.confirm {
        ui.add(crate::components::para(
            &words,
            12.0,
            Weight::Medium,
            p.muted,
        ));
        return true;
    }
    let mut on = lock.agreed == Some(q.excess);
    crate::components::checkbox(ui, a, &mut on, &words);
    lock.agreed = on.then_some(q.excess);
    on
}

/// «Exactly from denominations: {}» with «Take», under an amount the coins cannot make exactly
/// (R-M1). `Some(amount)`: «Take» pressed.
pub fn exactly_hint(ui: &mut Ui, a: &mut Assets, q: &MixedLockQuote) -> Option<Duffs> {
    let exactly = q.exactly.filter(|e| *e > 0 && !q.exact)?;
    let p = a.p;
    let mut take = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add(crate::components::para(
            &format!(
                tr!("Exactly from denominations: {}"),
                crate::format::amount(exactly)
            ),
            12.0,
            Weight::Medium,
            p.text,
        ));
        if button(ui, a, tr!("Take"), ButtonKind::Ghost, None).clicked() {
            take = Some(exactly);
        }
    });
    take
}

/// Why the coins cannot pay it, or why the core did not send it, in words.
pub fn error_text(e: &CoreError) -> String {
    match e {
        CoreError::NotEnough => tr!("Not enough fully mixed coins for this amount.").into(),
        CoreError::MixedExcess { excess, .. } => format!(
            tr!("The excess has changed: {} now. Confirm again."),
            crate::format::amount(*excess)
        ),
        CoreError::NotAvailable(why) => format!("{}.", why.trim_end_matches('.')),
        other => crate::text::failed(tr!("Not sent"), other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(excess: Duffs, confirm: bool) -> MixedLockQuote {
        MixedLockQuote {
            amount: 3_000_000,
            nominal: 3_000_000 + excess,
            exact: excess == 0,
            excess,
            confirm,
            coins: 3,
            fee: 2_260,
            exactly: None,
        }
    }

    /// Each screen takes only its own last answer; another excess takes the yes off; the spend
    /// carries the excess only once it is agreed to (И7).
    #[test]
    fn a_plan_is_its_own_screens_and_its_excess_needs_the_yes() {
        let t0 = std::time::Instant::now();
        let later = t0 + QUIET;
        let mut create = MixedLock::default();
        let mut pool = MixedLock::default();
        assert!(
            create
                .ask(CREATE, MixedTo::Identity, 3_000_000, t0)
                .is_none(),
            "not at once"
        );
        assert!(create.waiting());
        let Some(Command::PreviewMixedLock { ask, .. }) =
            create.ask(CREATE, MixedTo::Identity, 3_000_000, later)
        else {
            panic!("asked once it stayed so");
        };
        assert!(
            create
                .ask(CREATE, MixedTo::Identity, 3_000_000, later)
                .is_none(),
            "once"
        );
        pool.ask(POOL, MixedTo::Pool, 3_000_000, t0);
        pool.ask(POOL, MixedTo::Pool, 3_000_000, later);
        create.on_preview(ask, &Ok(quote(7_000_000, true)));
        pool.on_preview(ask, &Ok(quote(0, false)));
        assert!(
            pool.quote.is_none(),
            "another screen's answer is not the pool's"
        );
        assert!(
            !create.ready(MixedTo::Identity, 3_000_000),
            "an excess > 0.01 needs the yes"
        );
        assert_eq!(create.accept_excess(MixedTo::Identity, 3_000_000), 0);
        create.agreed = Some(7_000_000);
        assert!(create.ready(MixedTo::Identity, 3_000_000));
        assert_eq!(
            create.accept_excess(MixedTo::Identity, 3_000_000),
            7_000_000
        );
        create.on_preview(ask, &Ok(quote(8_000_000, true)));
        assert_eq!(create.agreed, None, "another excess: the yes off");
        assert!(
            create.quote_for(MixedTo::Identity, 2_000_000).is_none(),
            "another amount"
        );
    }

    /// r2 Should: an amount typed digit by digit is asked once, after it stays for [`QUIET`].
    #[test]
    fn an_amount_is_asked_once_it_stays_as_typed() {
        let t0 = std::time::Instant::now();
        let ms = std::time::Duration::from_millis;
        let mut lock = MixedLock::default();
        for (i, amount) in [100_000, 1_000_000, 10_000_000].into_iter().enumerate() {
            assert!(
                lock.ask(POOL, MixedTo::Pool, amount, t0 + ms(100 * i as u64))
                    .is_none(),
                "a digit: nothing asked"
            );
        }
        assert!(
            lock.ask(POOL, MixedTo::Pool, 10_000_000, t0 + ms(400))
                .is_none()
        );
        assert!(matches!(
            lock.ask(POOL, MixedTo::Pool, 10_000_000, t0 + ms(500)),
            Some(Command::PreviewMixedLock {
                amount: 10_000_000,
                ..
            })
        ));
    }
}
