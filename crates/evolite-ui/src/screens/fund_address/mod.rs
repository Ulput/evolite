//! «Top up the Platform address» from Core (FEAT-0015 stage 2): the form (frame 22), the review
//! with the password (22b), the steps while it runs (23), the stopped one on the Overview and its
//! «Continue» (24, 24b), and what it credited (25). The core does the work
//! (`Spend::FundAddress`, `Spend::ResumeFundAddress`); this keeps what the screens show.

mod dialog;
mod overview;

pub use dialog::fund_dialog;
pub use overview::on_overview;

use crate::components::SpendPassword;
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
    /// The amount the person asked for, while the core funds the address (the steps, 23).
    pub running: Option<Duffs>,
    /// How the last one ended, with its amount: «Credited» (25), or why not.
    pub result: Option<(Duffs, Result<(), CoreError>)>,
    /// «Topping up the Platform address» (23) open; «Hide» closes it, the funding goes on.
    pub steps_open: bool,
    /// «Continue the top-up» (24b) open.
    pub resume_open: bool,
    /// The password of the review (22b) and of «Continue the top-up» (24b).
    pub password: SpendPassword,
    pub resume_password: SpendPassword,
    /// What the address gained, as the core proved it (BUG-171 AC-T4), until its «Done».
    pub credited: Option<Duffs>,
}

impl FundAddressState {
    /// `Event::AddressFunded`: what came, ahead of its «Done».
    pub fn on_funded(&mut self, credited: Duffs) {
        self.credited = Some(credited);
    }

    /// The core's answer to `FundAddress` or `ResumeFundAddress`. «Credited» says what came
    /// when the core proved it (BUG-171: QA's C2 said «+0.0010» for 0.00097064), else what
    /// was planned.
    pub fn on_done(&mut self, result: &Result<(), CoreError>) {
        self.password.settle(Some(result));
        self.resume_password.settle(Some(result));
        let credited = self.credited.take();
        let Some(planned) = self.running.take() else {
            return;
        };
        // A wrong password is said under its field, not as a failed top-up.
        if !matches!(result, Err(CoreError::WrongPassword)) {
            let duffs = match result {
                Ok(()) => credited.unwrap_or(planned),
                Err(_) => planned,
            };
            self.result = Some((duffs, result.clone()));
        }
    }

    /// The wallet locked: what was typed for a spend goes.
    pub fn on_lock(&mut self) {
        self.password.forget();
        self.resume_password.forget();
    }
}
