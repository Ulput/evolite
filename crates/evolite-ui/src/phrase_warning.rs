//! SEC-7: on Linux no capture of the window can be refused — any X11 program can take the
//! screen, and on Wayland a screenshot, a recording or a screen share the person allowed sees
//! it. So before each showing of the recovery phrase (O1, Settings › Show the phrase) a warning
//! says so, the text for the session ([`crate::capture::Session`]), with what to do first; the
//! phrase is drawn only after «Show the phrase». Windows keeps the window out of captures
//! instead ([`crate::capture`]); macOS has its own (SEC-10).

use crate::capture::Session;
use crate::components::{Assets, ButtonKind, Part, button, button_row, dialog_with_footer, para};
use crate::theme::Weight;
use egui::{CornerRadius, Frame, Margin, Ui};

/// What the person chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Show,
    Cancel,
}

/// What can see the phrase in `session`. Not known: the X11 text, the stricter (R-1).
pub fn text_for(session: Session) -> &'static str {
    match session {
        Session::Wayland => tr!(
            "On Linux with Wayland, a screenshot, recording or screen sharing that you allowed will see the phrase. evolite cannot block it."
        ),
        Session::X11 | Session::Unknown => tr!(
            "On Linux with X11, evolite cannot block screen capture: any program on this computer can capture the screen while the phrase is shown."
        ),
    }
}

/// The warning (R-2), drawn over the screen: «Show the phrase», or «Cancel» (also Escape, the
/// cross and a click outside); `None` while it is open. `told` is the screen's own flag for
/// this showing: the journal hears of the warning once (R-4), with the session, never a word.
pub fn dialog(ui: &mut Ui, a: &mut Assets, session: Session, told: &mut bool) -> Option<Choice> {
    if !std::mem::replace(told, true) {
        evolite_core::diag::record(evolite_core::diag::Record::PhraseWarning {
            session: session.name(),
        });
    }
    let p = a.p;
    let mut choice = None;
    let closed = dialog_with_footer(
        ui,
        a,
        "phrase-warning",
        "warning",
        tr!("The phrase can be captured from the screen"),
        440.0,
        |ui, a, part| {
            if part == Part::Footer {
                button_row(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        choice = Some(Choice::Cancel);
                    }
                    if button(ui, a, tr!("Show the phrase"), ButtonKind::Primary, None).clicked() {
                        choice = Some(Choice::Show);
                    }
                });
                return choice.is_some();
            }
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.add(para(text_for(session), 14.0, Weight::Regular, p.text));
            Frame::new()
                .fill(p.raised)
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(para(
                        tr!(
                            "Before you go on, stop screen sharing and recording. Write the phrase down on paper — do not photograph it or save it to a file."
                        ),
                        13.0,
                        Weight::Regular,
                        p.text,
                    ));
                });
            false
        },
    );
    if closed && choice.is_none() {
        choice = Some(Choice::Cancel);
    }
    choice
}
