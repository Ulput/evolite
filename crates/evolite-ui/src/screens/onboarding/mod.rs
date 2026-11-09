//! Onboarding O1–O6: create (phrase → check → password → sync), restore by phrase
//! (phrase → password → sync) and import from DashPay (phrase → what was found → password →
//! sync). The core makes the phrase, checks a typed one, scans DashPay, takes the password
//! and runs the first sync; this screen only shows and asks. What was typed is wiped as soon
//! as it goes to the core.

mod password;
mod phrase;
mod restore;
mod sync;

use password::password;
pub use password::{missing, missing_line, strength};
use phrase::{phrase, verify};
use restore::{found, restore};
use sync::sync;

use super::txt;
use crate::components::{Assets, ButtonKind, button, button_enabled, network_chip};
use crate::context::Ctx;
use crate::icons::Logo;
use crate::theme::{Palette, Weight, mono, sans};
use egui::{Align, Align2, CornerRadius, CursorIcon, Layout, Rect, Sense, Ui, pos2, vec2};
use evolite_api::{Command, CoreError, DashPayScan, DoneWhat, Event, Phrase, VaultState};
use zeroize::Zeroizing;
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Flow {
    Create,
    Restore,
    DashPay,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Step {
    Phrase,
    Verify,
    Restore,
    Found,
    Password,
    Sync,
}

/// What the onboarding asks the app to do after a frame.
pub enum Outcome {
    Stay,
    /// Back from the first step.
    Welcome,
    /// "Open the wallet".
    Wallet,
    /// "About the nodes".
    Nodes,
}

pub struct Onboarding {
    pub flow: Flow,
    pub step: Step,
    long_phrase: bool,
    phrase_hidden: bool,
    written: bool,
    answers: [Option<usize>; 3],
    /// What O2 asks about the current phrase, chosen when O2 first shows it.
    checks: Option<phrase::Checks>,
    /// How many phrases the core has sent: a second one replaces what was shown.
    phrases: u32,
    /// Ask the fixed positions of the Figma frame, not random ones (screenshots).
    fixed_check: bool,
    password: Zeroizing<String>,
    repeat: Zeroizing<String>,
    lock_after: usize,
    /// The auto-lock picked on the password step, for the app to take into Settings once.
    lock_choice: Option<usize>,
    restore_words: Zeroizing<Vec<String>>,
    restore_hidden: bool,
    focused: Option<usize>,
    paste_error: bool,
    coinjoin_ok: bool,
    /// The new phrase from the core, and how long it was asked to be.
    phrase: Option<Phrase>,
    asked_len: Option<usize>,
    /// What the core found on DashPay paths.
    scan: Option<DashPayScan>,
    /// A command is out and the step waits for the core's answer.
    waiting: bool,
    error: Option<String>,
    /// Press the step's main button on the first frame (screenshots of later steps).
    submit_on_open: bool,
}

impl Onboarding {
    /// "Lock the wallet" from the password step, once, after the wallet was asked for:
    /// its position in Settings → Security.
    pub fn take_lock_choice(&mut self) -> Option<usize> {
        self.lock_choice.take()
    }

    pub fn new(flow: Flow) -> Self {
        Self {
            flow,
            step: match flow {
                Flow::Create => Step::Phrase,
                Flow::Restore | Flow::DashPay => Step::Restore,
            },
            long_phrase: false,
            phrase_hidden: false,
            written: false,
            answers: [None; 3],
            checks: None,
            phrases: 0,
            fixed_check: false,
            password: Zeroizing::default(),
            repeat: Zeroizing::default(),
            lock_after: 1,
            lock_choice: None,
            restore_words: Zeroizing::new(vec![String::new(); 12]),
            restore_hidden: false,
            focused: None,
            paste_error: false,
            coinjoin_ok: false,
            phrase: None,
            asked_len: None,
            scan: None,
            waiting: false,
            error: None,
            submit_on_open: false,
        }
    }

    /// A state for screenshots: the step filled in as in Figma.
    #[cfg(feature = "dev-tools")]
    pub fn example(step: Step) -> Self {
        let flow = match step {
            Step::Restore => Flow::Restore,
            Step::Found => Flow::DashPay,
            _ => Flow::Create,
        };
        let mut o = Self::new(flow);
        o.step = match step {
            // These steps need the core: get there by pressing the button on the step before.
            Step::Found => Step::Restore,
            Step::Sync => Step::Password,
            other => other,
        };
        o.submit_on_open = matches!(step, Step::Found | Step::Sync);
        o.written = true;
        o.fixed_check = true;
        o.answers = [Some(1), Some(0), None];
        o.password = Zeroizing::new("Correct-horse-7".into());
        o.repeat = o.password.clone();
        for (i, w) in ["orbit", "canvas", "lunar", "pepper", "alert", "fro"]
            .iter()
            .enumerate()
        {
            o.restore_words[i] = w.to_string();
        }
        if step == Step::Found {
            // A valid test phrase (BIP39 vectors), so the core accepts it and scans.
            o.restore_words = Zeroizing::new(vec!["abandon".to_string(); 11]);
            o.restore_words.push("about".into());
        }
        o.focused = Some(5);
        o.coinjoin_ok = true;
        o
    }

    /// Takes the core's answers that belong to the onboarding.
    pub fn on_event(&mut self, event: &Event) {
        match event {
            Event::NewPhrase(phrase) => {
                // A different phrase (another length) makes the old answers, the check and
                // "I wrote it down" meaningless. The first one keeps what a screenshot set.
                if self.phrases > 0 {
                    self.answers = [None; 3];
                    self.written = false;
                }
                self.phrases += 1;
                self.checks = None;
                self.phrase = Some(phrase.clone());
            }
            Event::PhraseAccepted(result) if self.waiting => {
                self.waiting = false;
                match result {
                    Ok(scan) => {
                        self.scan = scan.clone();
                        self.next();
                    }
                    // The core's own reason, e.g. "This wallet is already on this computer —
                    // “Main”": the phrase is right, and "did not fit" would send the person to
                    // check good words (BUG-27).
                    Err(CoreError::NotAvailable(why)) => self.error = Some(why.clone()),
                    Err(_) => {
                        self.error = Some(
                            tr!("The phrase did not fit: check the words and their order").into(),
                        )
                    }
                }
            }
            Event::Done {
                what: DoneWhat::Create,
                result,
            } if self.waiting => {
                self.waiting = false;
                match result {
                    Ok(()) => self.next(),
                    Err(e) => {
                        self.error = Some(crate::text::failed(tr!("The wallet was not made"), e))
                    }
                }
            }
            _ => {}
        }
    }

    fn back(&mut self) -> Outcome {
        self.step = match (self.flow, self.step) {
            (Flow::Create, Step::Verify) => Step::Phrase,
            (Flow::Create, Step::Password) => Step::Verify,
            (Flow::DashPay, Step::Found) => Step::Restore,
            (Flow::DashPay, Step::Password) if self.scan.is_some() => Step::Found,
            (_, Step::Password) => Step::Restore,
            (_, Step::Sync) => Step::Password,
            _ => return Outcome::Welcome,
        };
        Outcome::Stay
    }

    fn next(&mut self) {
        self.step = match (self.flow, self.step) {
            (_, Step::Phrase) => Step::Verify,
            // The real core does not scan DashPay paths yet: nothing to show, straight on.
            (Flow::DashPay, Step::Restore) if self.scan.is_some() => Step::Found,
            (_, Step::Verify | Step::Restore | Step::Found) => Step::Password,
            _ => Step::Sync,
        };
    }
}

pub fn onboarding(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, o: &mut Onboarding) -> Outcome {
    let p = a.p;
    let mut outcome = Outcome::Stay;
    // The vault got written but the wallet did not start: the wallet exists, so the unlock
    // screen (where the welcome screen leads with a saved vault) starts it again.
    if cx.snap.vault == VaultState::Locked && !o.waiting {
        return Outcome::Welcome;
    }
    // A new wallet starts with a phrase from the core; asked again when the length changes.
    let len = if o.long_phrase { 24 } else { 12 };
    if o.flow == Flow::Create && o.asked_len != Some(len) {
        cx.send(Command::NewPhrase { words: len });
        o.asked_len = Some(len);
        o.phrase = None;
    }
    // The copied phrase is wiped by the app (`clipboard::tick`), wherever the person goes.

    // Top bar: back, logo in the middle, network on the right.
    let bar = ui
        .allocate_exact_size(vec2(ui.available_width(), 68.0), Sense::hover())
        .0;
    let back = Rect::from_min_size(
        pos2(bar.left() + 32.0, bar.center().y - 12.0),
        vec2(80.0, 24.0),
    );
    let back_resp = ui.interact(back, ui.id().with("onboarding-back"), Sense::click());
    crate::components::named(&back_resp, egui::WidgetType::Button, true, tr!("Back"));
    let back_color = if back_resp.hovered() { p.text } else { p.muted };
    a.icons.icon("chevron-left", back_color, &p, 20.0).paint_at(
        ui,
        Rect::from_min_size(pos2(back.left(), back.center().y - 10.0), vec2(20.0, 20.0)),
    );
    ui.painter().text(
        pos2(back.left() + 26.0, back.center().y),
        Align2::LEFT_CENTER,
        crate::i18n::t(tr!("Back")),
        sans(14.0, Weight::Medium),
        back_color,
    );
    if back_resp
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
    {
        outcome = o.back();
    }
    a.icons
        .logo(Logo::Horizontal, &p, 28.0)
        .paint_at(ui, Rect::from_center_size(bar.center(), vec2(106.0, 28.0)));
    let chip_rect = Rect::from_min_size(
        pos2(bar.right() - 32.0 - 82.0, bar.center().y - 12.5),
        vec2(82.0, 25.0),
    );
    let mut chip_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(chip_rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    network_chip(&mut chip_ui, a, cx.snap.network);

    let column = if o.step == Step::Sync { 640.0 } else { 600.0 };
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            let side = ((ui.available_width() - column) / 2.0).max(24.0);
            ui.horizontal_top(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(column.min(ui.available_width() - 24.0));
                    ui.spacing_mut().item_spacing.y = 20.0;
                    ui.add_space(20.0);
                    // The phrase, its words to pick or the phrase being typed: not in captures.
                    if matches!(o.step, Step::Phrase | Step::Verify | Step::Restore) {
                        crate::capture::keep_out();
                    }
                    match o.step {
                        Step::Phrase => phrase(ui, a, o, crate::is_demo(cx.snap)),
                        Step::Verify => verify(ui, a, o),
                        Step::Restore => restore(ui, a, cx, o),
                        Step::Found => found(ui, a, o),
                        Step::Password => password(ui, a, cx, o),
                        Step::Sync => {
                            if let Some(next) = sync(ui, a, &cx.snap.sync, crate::is_demo(cx.snap))
                            {
                                outcome = next;
                            }
                        }
                    }
                    ui.add_space(24.0);
                });
            });
        });
    outcome
}

/// Step dashes for the create flow: the current one long, ink; done ones short, ink.
fn steps(ui: &mut Ui, p: &Palette, current: usize) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 13.0), Sense::hover());
    let mut x = rect.left();
    for i in 1..=3 {
        let w = if i == current { 28.0 } else { 12.0 };
        let color = if i <= current { p.ink } else { p.line };
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(x, rect.center().y - 2.0), vec2(w, 4.0)),
            CornerRadius::same(2),
            color,
        );
        x += w + 6.0;
    }
    ui.painter().text(
        pos2(x + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        crate::i18n::t(&format!(tr!("Step {current} of 3"), current = current)),
        mono(11.0, Weight::Medium),
        p.muted,
    );
}

fn heading(ui: &mut Ui, p: &Palette, title: &str, lead: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.label(txt(title, 28.0, Weight::Bold, p.text));
        ui.add(
            egui::Label::new(txt(lead, 15.0, Weight::Regular, p.muted).line_height(Some(22.5)))
                .wrap(),
        );
    });
}

/// Actions row: optional secondary action on the left, primary on the right.
fn actions(
    ui: &mut Ui,
    a: &mut Assets,
    left: Option<&str>,
    primary: &str,
    enabled: bool,
) -> (bool, bool) {
    let mut out = (false, false);
    ui.horizontal(|ui| {
        if let Some(label) = left {
            out.0 = button(ui, a, label, ButtonKind::Ghost, None).clicked();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            out.1 = button_enabled(ui, a, primary, ButtonKind::Primary, None, enabled).clicked();
        });
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-27: a phrase whose wallet is already here is a right phrase — the core's reason is
    /// said, with the wallet's name, not "the phrase did not fit".
    #[test]
    fn a_phrase_already_on_this_computer_says_so() {
        for flow in [Flow::Restore, Flow::DashPay] {
            let mut o = Onboarding::new(flow);
            o.waiting = true;
            let why = "This wallet is already on this computer — “Kestrel”".to_string();
            o.on_event(&Event::PhraseAccepted(Err(CoreError::NotAvailable(
                why.clone(),
            ))));
            assert_eq!(o.error.as_deref(), Some(why.as_str()));
            crate::i18n::set(crate::i18n::Lang::Ru);
            assert_eq!(
                crate::i18n::t(&why),
                "Этот кошелёк уже есть на компьютере — «Kestrel»"
            );
            crate::i18n::set(crate::i18n::Lang::En);
        }
    }

    #[test]
    fn a_wrong_phrase_still_says_it_did_not_fit() {
        let mut o = Onboarding::new(Flow::Restore);
        o.waiting = true;
        o.on_event(&Event::PhraseAccepted(Err(CoreError::BadPhrase)));
        assert_eq!(
            o.error.as_deref(),
            Some("The phrase did not fit: check the words and their order")
        );
    }
}
