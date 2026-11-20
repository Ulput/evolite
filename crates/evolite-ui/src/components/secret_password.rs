//! SEC-11 (TECH-duplicates Д7): the password before a secret — unlocking, opening another
//! wallet, the phrase in Settings (check it, show it) and an identity's private key. The core
//! counts all of them for a wallet as one ([`evolite_api::WalletEntry::wrong_tries`],
//! `wait_until`); each form shows the same thing under its field: the error, the warning before
//! a pause, the pause's plate with its countdown and the line after it; and its button stays
//! disabled while the pause lasts, saying why.

use super::prelude::*;
use crate::screens::pause;
use evolite_api::{CoreError, Timestamp, WalletEntry};

/// The pause of one form's wallet, as it stands this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    /// Whole seconds left; `None`: no pause, the password can go.
    pub left: Option<u64>,
    /// The end counted down to.
    pub end: Option<Timestamp>,
    /// Wrong passwords in a row (for the warning before a pause and the line after one).
    pub wrong_tries: u32,
}

impl Gate {
    /// Wallet `w`'s pause for a form (FEAT-0002): its snapshot's end, else the end a refusal
    /// gave (`refused`, the answer may come a frame before its snapshot); cut as the core cuts
    /// it when the clock is set back (`cut`, kept by the form).
    pub fn of(
        w: Option<&WalletEntry>,
        refused: Option<Timestamp>,
        cut: &mut pause::Cut,
        now: Timestamp,
    ) -> Self {
        let wrong_tries = w.map_or(0, |w| w.wrong_tries);
        let core = w.map_or(refused, |w| pause::until(w, refused));
        // A refusal before its snapshot: the count it was refused at is not known yet, so the
        // cut takes the longest step, not the stale count's (none below the free tries).
        let from_refusal = w.is_none_or(|w| w.wait_until.is_none()) && refused.is_some();
        let counted = if from_refusal { u32::MAX } else { wrong_tries };
        let end = cut.end(core, counted, now);
        Self {
            left: pause::left(end, now),
            end,
            wrong_tries,
        }
    }

    pub fn paused(&self) -> bool {
        self.left.is_some()
    }
}

/// The end a refusal gave, when the core's answer was «too many wrong passwords».
pub fn refused(error: Option<&CoreError>) -> Option<Timestamp> {
    match error {
        Some(CoreError::TooManyTries { until }) => Some(*until),
        _ => None,
    }
}

/// Under the password field: in a pause its plate alone, counting down; else the red `error`,
/// the warning after the 3rd and 4th wrong password, and after a pause what the next one costs.
pub fn lines(
    ui: &mut Ui,
    a: &mut Assets,
    gate: &Gate,
    error: Option<&str>,
    now: chrono::DateTime<chrono::Local>,
) {
    if let Some(secs) = gate.left {
        pause::plate(ui, a, secs, gate.end.unwrap_or_default());
        pause::tick(ui, now);
        return;
    }
    under_field(ui, a, error, gate.wrong_tries);
}

/// [`lines`] without the plate: for a form that puts the plate elsewhere ("Open "…"").
pub fn under_field(ui: &mut Ui, a: &mut Assets, error: Option<&str>, wrong_tries: u32) {
    error_line(ui, a, error);
    tries_lines(ui, a, wrong_tries);
}

/// [`lines`] split around the button, for a dialog whose button must stay put as the lines
/// come (SEC-11 QA: the key window grew under the pointer). Above the button: the error's line,
/// its height kept from the start.
pub fn error_line(ui: &mut Ui, a: &mut Assets, error: Option<&str>) {
    // The error's line is kept from the start, empty, so that «Wrong password» moves nothing
    // under it when it comes (FEAT-0034 item 8).
    match error {
        Some(error) => {
            ui.label(txt(error, 12.0, Weight::Regular, a.p.danger));
        }
        None => {
            // As tall as the error's own label would be.
            let line = egui::WidgetText::from(txt(
                tr!("Wrong password"),
                12.0,
                Weight::Regular,
                a.p.danger,
            ))
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Body,
            )
            .size()
            .y;
            ui.allocate_space(egui::vec2(0.0, line));
        }
    }
}

/// Below the button ([`error_line`] above it): the pause's plate, counting down; else the
/// warning before a pause and the line after one.
pub fn notes(ui: &mut Ui, a: &mut Assets, gate: &Gate, now: chrono::DateTime<chrono::Local>) {
    if let Some(secs) = gate.left {
        pause::plate(ui, a, secs, gate.end.unwrap_or_default());
        pause::tick(ui, now);
        return;
    }
    tries_lines(ui, a, gate.wrong_tries);
}

/// The warning after the 3rd and 4th wrong password, and after a pause what the next one costs.
pub fn tries_lines(ui: &mut Ui, a: &mut Assets, wrong_tries: u32) {
    if let Some(warning) = pause::warning(wrong_tries) {
        pause::warning_line(ui, a, &warning);
    }
    if let Some(after) = pause::after(wrong_tries) {
        pause::after_line(ui, a, &after);
    }
}

/// The button that sends the password: disabled while `ready` is not, and in a pause, saying
/// why on hover and to a screen reader (AC-D2).
pub fn send_button(
    ui: &mut Ui,
    a: &mut Assets,
    label: &str,
    kind: ButtonKind,
    width: Option<f32>,
    ready: bool,
    gate: &Gate,
) -> Response {
    let button = button_enabled(ui, a, label, kind, width, ready && !gate.paused());
    match gate.left {
        Some(secs) => pause::explain(ui, button, secs),
        None => button,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wallet(wrong_tries: u32, wait_until: Option<Timestamp>) -> WalletEntry {
        WalletEntry {
            id: "w1".into(),
            name: "Main".into(),
            color: 0,
            origin: evolite_api::WalletOrigin::Created,
            created_at: 0,
            last_opened: None,
            open: true,
            wrong_tries,
            wait_until,
        }
    }

    /// The wallet's pause from its snapshot, else from a refusal; over when its end has come.
    #[test]
    fn the_gate_is_the_wallets_pause_or_a_refusals() {
        let now = 1_800_000_000;
        let mut cut = pause::Cut::default();
        let open = Gate::of(Some(&wallet(1, None)), None, &mut cut, now);
        assert!(!open.paused());
        assert_eq!(open.wrong_tries, 1);
        let mut cut = pause::Cut::default();
        let paused = Gate::of(Some(&wallet(5, Some(now + 20))), None, &mut cut, now);
        assert_eq!(paused.left, Some(20));
        // A clock set back: no longer than the count's step from now (AC-T3).
        let mut cut = pause::Cut::default();
        let step = evolite_core::attempts::pause_after(5);
        let far = Gate::of(Some(&wallet(5, Some(now + 86_400))), None, &mut cut, now);
        assert_eq!(far.left, Some(step));
        // The refusal before its snapshot, whose count is still the old one.
        let mut cut = pause::Cut::default();
        let refusal = Gate::of(Some(&wallet(2, None)), Some(now + 30), &mut cut, now);
        assert_eq!(refusal.left, Some(30), "the refusal before its snapshot");
        let mut cut = pause::Cut::default();
        let over = Gate::of(Some(&wallet(5, Some(now))), None, &mut cut, now);
        assert!(!over.paused());
        assert_eq!(
            refused(Some(&CoreError::TooManyTries { until: 7 })),
            Some(7)
        );
        assert_eq!(refused(Some(&CoreError::WrongPassword)), None);
    }
}
