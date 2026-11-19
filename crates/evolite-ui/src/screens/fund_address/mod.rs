//! «Top up the Platform address» from Core (FEAT-0015 stage 2): the form (frame 22), the review
//! with the password (22b), the steps while it runs (23), the stopped one on the Overview and its
//! «Continue» (24, 24b), and what it credited (25). The core does the work
//! (`Spend::FundAddress`, `Spend::ResumeFundAddress`); this keeps what the screens show.

mod dialog;
mod overview;

pub use dialog::fund_dialog;
pub use overview::on_overview;

use crate::components::spend_flow::SpendFlow;
use evolite_api::{CoreError, Duffs};

/// The form (22) and, once «Review» is pressed, the review (22b).
/// It opens empty (BUG-172, the designer: no amount is put into what gets signed); «Max» fills
/// it on a click.
#[derive(Default)]
pub struct FundForm {
    pub amount: String,
    /// An amount put in for the person (`format::fill_suggested`) until they change it: none
    /// when the form opens; a `--screen` frame with an amount sets one.
    pub suggested: Option<Duffs>,
    /// On the review with the password (22b).
    pub review: bool,
}

/// What the funding screens keep between frames.
#[derive(Default)]
pub struct FundAddressState {
    /// The dialog, open.
    pub form: Option<FundForm>,
    /// The top-up of the review (22b): the amount asked for is what it signed (TECH-spend-flow).
    pub flow: SpendFlow<Duffs>,
    /// «Continue the top-up» (24b): what is expected at the address.
    pub resume: SpendFlow<Duffs>,
    /// What the address gained, as the core proved it, for the answer shown (25).
    pub shown_credited: Option<Duffs>,
    /// «Topping up the Platform address» (23) open; «Hide» closes it, the funding goes on.
    pub steps_open: bool,
    /// «Continue the top-up» (24b) open.
    pub resume_open: bool,
    /// What the address gained, as the core proved it (BUG-171 AC-T4), until its «Done».
    pub credited: Option<Duffs>,
}

impl FundAddressState {
    /// `Event::AddressFunded`: what came, ahead of its «Done».
    pub fn on_funded(&mut self, credited: Duffs) {
        self.credited = Some(credited);
    }

    /// The core's answer to `FundAddress` or `ResumeFundAddress`, taken only by the flow that
    /// asked it (`ask`; BUG-214: never another wallet's). «Credited» says what came when the
    /// core proved it (BUG-171: QA's C2 said «+0.0010» for 0.00097064), else what was planned.
    /// A wrong password is said under its field, not as a failed top-up.
    pub fn on_done(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) {
        let credited = self.credited.take();
        if self.flow.on_done(ask, result) || self.resume.on_done(ask, result) {
            self.shown_credited = credited.filter(|_| result.is_ok());
        }
    }

    /// The amount being funded, while the core funds the address (the steps, 23).
    pub fn running(&self) -> Option<Duffs> {
        [&self.flow, &self.resume]
            .into_iter()
            .find(|f| f.waiting())
            .and_then(|f| f.signed().copied())
    }

    /// How the last one ended, with its amount: «Credited» (25), or why not.
    pub fn result(&self) -> Option<(Duffs, &Result<(), CoreError>)> {
        [&self.flow, &self.resume].into_iter().find_map(|f| {
            let result = f.result()?;
            let planned = *f.signed()?;
            Some((self.shown_credited.unwrap_or(planned), result))
        })
    }

    /// The answer shown was closed.
    pub fn close_result(&mut self) {
        for flow in [&mut self.flow, &mut self.resume] {
            if flow.result().is_some() {
                flow.forget();
            }
        }
        self.shown_credited = None;
    }
}

impl crate::screen_state::ScreenState for FundAddressState {
    #[cfg(all(test, demo_core, feature = "dev-tools"))]
    fn flows(&mut self) -> Vec<&mut dyn crate::components::spend_flow::AnyFlow> {
        vec![&mut self.flow, &mut self.resume]
    }

    fn forget(&mut self, _why: crate::screen_state::Why) {
        // BUG-214: a top-up's form, password and answer are the wallet's that asked.
        *self = Self::default();
    }
}
