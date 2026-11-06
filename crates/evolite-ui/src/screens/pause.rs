//! The pause after too many wrong passwords (FEAT-0002), as the unlock screen and "Open "…""
//! show it: the warning before it, the plate with its countdown, the line after it. The core
//! counts and refuses; this only shows the `wait_until` and `wrong_tries` it sends.

use super::txt;
use crate::components::Assets;
use crate::i18n::{self, own};
use crate::text::plural;
use crate::theme::{Weight, mono, sans};
use egui::accesskit::{Live, Role};
use egui::text::LayoutJob;
use egui::{Color32, CornerRadius, Frame, Margin, Response, TextFormat, Ui};
use evolite_api::{Timestamp, WalletEntry};
use evolite_core::attempts::{FREE_TRIES, pause_after};

/// The end of wallet `w`'s pause: its snapshot's, or while that has none a refusal's for it
/// (the core's answer may come a frame before its snapshot). The snapshot wins: the core may
/// have cut the pause short since the refusal (a clock set back, review r1 should 3).
pub fn until(w: &WalletEntry, refused: Option<Timestamp>) -> Option<Timestamp> {
    w.wait_until.or(refused)
}

/// The end a form counts down to, cut down as the core cuts it (R-31 item 4, AC-T3): a
/// wall clock set back while the screen is open makes the pause no longer than its step from
/// then, and the cut end stays put instead of moving on with the clock. A new end from the
/// core (another snapshot, a refusal) starts afresh, and so does a new count of wrong
/// passwords: a refusal may come a frame before the snapshot with its count (review r3 should 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cut {
    /// The core's end the cut was made from.
    from: Option<Timestamp>,
    /// The wrong passwords in a row it was cut for.
    tries: u32,
    end: Option<Timestamp>,
}

impl Cut {
    /// The end of the pause at `now` for the core's end `core` after `wrong_tries` wrong
    /// passwords in a row.
    pub fn end(
        &mut self,
        core: Option<Timestamp>,
        wrong_tries: u32,
        now: Timestamp,
    ) -> Option<Timestamp> {
        if core != self.from || wrong_tries != self.tries {
            *self = Self {
                from: core,
                tries: wrong_tries,
                end: core,
            };
        }
        let step = Timestamp::try_from(pause_after(wrong_tries)).unwrap_or(Timestamp::MAX);
        self.end = self.end.map(|end| end.min(now.saturating_add(step)));
        self.end
    }
}

/// Whole seconds of the pause left at `now` (Unix seconds); `None`: no pause, the password can
/// go. Over in the second the countdown reaches 0:00 (AC-D5).
pub fn left(until: Option<Timestamp>, now: Timestamp) -> Option<u64> {
    until.filter(|&u| u > now).map(|u| (u - now) as u64)
}

/// The countdown, `m:ss` without a leading zero for the minutes (0:45, 4:12, 14:59): the same
/// digits in every language.
pub fn clock(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// A step of the scale in words: "30 s", "1 min" … "15 min".
pub fn step_text(secs: u64) -> String {
    if secs < 60 {
        format!(tr!("{} s"), secs)
    } else {
        format!(tr!("{} min"), secs / 60)
    }
}

/// "You can try again in 0:45.", the countdown kept left to right in right-to-left text.
fn try_again(secs: u64) -> String {
    format!(tr!("You can try again in {}."), own(&clock(secs)))
}

/// After the 3rd and 4th wrong password in a row: how many are left before the pause, and the
/// usual cause of a wrong one — translated, for a label as it is.
pub fn warning(wrong_tries: u32) -> Option<String> {
    if !(FREE_TRIES - 1..=FREE_TRIES).contains(&wrong_tries) {
        return None;
    }
    let n = FREE_TRIES + 1 - wrong_tries;
    #[rustfmt::skip]
    let s = plural(n, tr!("attempt left before a pause."), tr!("attempts left before a pause."));
    Some(format!(
        "{} {}",
        i18n::t(&s),
        i18n::t(tr!("Check the keyboard layout and Caps Lock."))
    ))
}

/// After a pause (none lasts, but the count is past the free tries): what the next wrong
/// password costs (AC-D5), in English for [`txt`].
pub fn after(wrong_tries: u32) -> Option<String> {
    (wrong_tries > FREE_TRIES).then(|| {
        format!(
            tr!("You can try again. If the password is wrong again, the next pause is {}."),
            step_text(pause_after(wrong_tries.saturating_add(1)))
        )
    })
}

/// "pause 4:12" for a wallet's row in "Which wallet to open?".
pub fn row_text(secs: u64) -> String {
    format!(tr!("pause {}"), own(&clock(secs)))
}

/// The plate in place of "Wrong password" while the pause lasts (AC-D2): why, the
/// countdown, and that nothing is deleted. A screen reader hears why and the time once, when
/// the pause starts — not every second (AC-D8).
pub fn plate(ui: &mut Ui, a: &mut Assets, secs: u64, until: Timestamp) {
    let p = a.p;
    Frame::new()
        .fill(p.raised)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(a.icons.icon("pending", p.muted, &p, 18.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(
                        egui::Label::new(txt(
                            tr!("Too many wrong passwords"),
                            13.0,
                            Weight::SemiBold,
                            p.text,
                        ))
                        .wrap(),
                    );
                    ui.add(egui::Label::new(countdown_line(secs, p.text)).wrap());
                    ui.add(
                        egui::Label::new(
                            txt(
                                tr!("Each new wrong password makes the pause longer. Nothing is deleted."),
                                12.0,
                                Weight::Regular,
                                p.muted,
                            )
                            .line_height(Some(17.0)),
                        )
                        .wrap(),
                    );
                });
            });
        });
    announce_once(ui, secs, until);
}

/// S4 with the countdown in the mono face; in right-to-left text one run, so that the bidi
/// order of the whole line stays right.
fn countdown_line(secs: u64, color: Color32) -> egui::WidgetText {
    let text_font = sans(13.0, Weight::Regular);
    let template = i18n::t(tr!("You can try again in {}."));
    match template.split_once("{}") {
        Some((before, after)) if !i18n::rtl() => {
            let mut job = LayoutJob::default();
            let plain = TextFormat::simple(text_font, color);
            job.append(before, 0.0, plain.clone());
            job.append(
                &clock(secs),
                0.0,
                TextFormat::simple(mono(13.0, Weight::Medium), color),
            );
            job.append(after, 0.0, plain);
            job.into()
        }
        _ => egui::RichText::new(i18n::t(&try_again(secs)))
            .font(text_font)
            .color(color)
            .into(),
    }
}

/// A polite live region whose words are those of the pause's start: a new pause (another
/// `until`) is said again, the countdown's seconds are not.
fn announce_once(ui: &Ui, secs: u64, until: Timestamp) {
    let id = ui.id().with("pause-status");
    let now = format!(
        "{} {}",
        i18n::spoken(tr!("Too many wrong passwords")),
        i18n::spoken(&try_again(secs))
    );
    let words = ui.data_mut(|d| {
        let said = d.get_temp_mut_or_insert_with(id, || (until, now.clone()));
        if said.0 != until {
            *said = (until, now);
        }
        said.1.clone()
    });
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_role(Role::Status);
        node.set_live(Live::Polite);
        node.set_label(words);
    });
}

/// The disabled button's reason, on hover and for a screen reader (AC-D2, AC-D8).
pub fn explain(ui: &Ui, button: Response, secs: u64) -> Response {
    let why = i18n::spoken(&try_again(secs));
    ui.ctx()
        .accesskit_node_builder(button.id, |node| node.set_description(why));
    button.on_hover_text(i18n::t(&try_again(secs)))
}

/// The grey line under the field after a pause, with the next step (AC-D5).
pub fn after_line(ui: &mut Ui, a: &mut Assets, text: &str) {
    let p = a.p;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add(a.icons.icon("info", p.muted, &p, 16.0));
        ui.add(egui::Label::new(txt(text, 12.0, Weight::Regular, p.muted)).wrap());
    });
}

/// The grey warning line under "Wrong password" (AC-D1), already translated.
pub fn warning_line(ui: &mut Ui, a: &Assets, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(sans(12.0, Weight::Regular))
                .color(a.p.muted),
        )
        .wrap(),
    );
}

/// While a pause lasts the countdown changes once a second: one frame at the next second, no
/// frames in between (AC-T5).
pub fn tick(ui: &Ui, now: chrono::DateTime<chrono::Local>) {
    let into = now.timestamp_subsec_millis().min(999) as u64;
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(1000 - into + 5));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn the_countdown_is_minutes_and_seconds_without_a_leading_zero() {
        assert_eq!(clock(45), "0:45");
        assert_eq!(clock(252), "4:12");
        assert_eq!(clock(899), "14:59");
        assert_eq!(clock(900), "15:00");
    }

    #[test]
    fn the_pause_is_over_in_the_second_it_reaches_zero() {
        assert_eq!(left(Some(100), 55), Some(45));
        assert_eq!(left(Some(100), 99), Some(1));
        assert_eq!(left(Some(100), 100), None);
        assert_eq!(left(None, 100), None);
    }

    #[test]
    fn the_warning_comes_after_the_third_and_fourth_wrong_password() {
        assert_eq!(warning(2), None);
        assert!(warning(3).is_some_and(|w| w.starts_with("2 attempts left")));
        assert!(warning(4).is_some_and(|w| w.starts_with("1 attempt left")));
        assert_eq!(warning(5), None, "the pause says it then");
    }

    #[test]
    fn after_a_pause_the_next_step_is_named_up_to_the_ceiling() {
        assert_eq!(after(4), None);
        assert!(after(5).is_some_and(|t| t.ends_with("is 1 min.")));
        assert!(after(9).is_some_and(|t| t.ends_with("is 15 min.")));
        assert!(
            after(40).is_some_and(|t| t.ends_with("is 15 min.")),
            "the ceiling"
        );
    }

    #[test]
    fn russian_says_the_attempts_left_in_the_right_form() {
        let left = |n| {
            crate::i18n::t_in(
                Lang::Ru,
                &crate::text::plural(
                    n,
                    tr!("attempt left before a pause."),
                    tr!("attempts left before a pause."),
                ),
            )
            .into_owned()
        };
        assert_eq!(left(1), "Осталась 1 попытка до паузы.");
        assert_eq!(left(2), "Осталось 2 попытки до паузы.");
        assert_eq!(left(5), "Осталось 5 попыток до паузы.");
        assert_eq!(
            crate::i18n::t_in(Lang::Ru, &step_text(30)),
            "30 с",
            "a step inside a sentence is translated too"
        );
    }

    #[test]
    fn the_snapshot_or_else_the_refusal() {
        let w = WalletEntry {
            id: "w".into(),
            name: "W".into(),
            color: 0,
            origin: evolite_api::WalletOrigin::Created,
            created_at: 0,
            last_opened: None,
            open: true,
            wrong_tries: 5,
            wait_until: Some(130),
        };
        assert_eq!(until(&w, None), Some(130));
        assert_eq!(until(&w, Some(160)), Some(130), "the snapshot wins");
        let none = WalletEntry {
            wait_until: None,
            ..w
        };
        assert_eq!(
            until(&none, Some(160)),
            Some(160),
            "a refusal before its snapshot"
        );
    }

    /// AC-T3 in the form (R-31 item 4): a wall clock set back a day while the screen is open
    /// counts down one step from then, the cut end stays put, and the core's next end wins.
    #[test]
    fn a_clock_set_back_on_the_open_screen_waits_one_step_and_stays_cut() {
        const NOW: Timestamp = 1_800_000_000;
        const DAY: Timestamp = 24 * 60 * 60;
        let mut cut = Cut::default();
        // The 8th wrong password: five minutes.
        let core = Some(NOW + 300);
        assert_eq!(cut.end(core, 8, NOW), Some(NOW + 300), "as the core says");
        assert_eq!(cut.end(core, 8, NOW + 10), Some(NOW + 300));
        let back = NOW - DAY;
        assert_eq!(
            cut.end(core, 8, back),
            Some(back + 300),
            "one step from then"
        );
        assert_eq!(
            cut.end(core, 8, back + 100),
            Some(back + 300),
            "counts down"
        );
        assert_eq!(left(cut.end(core, 8, back + 300), back + 300), None, "over");
        // A new end from the core (the next wrong password): counted from it.
        let next = Some(back + 400 + 600);
        assert_eq!(cut.end(next, 9, back + 400), next);
        assert_eq!(cut.end(None, 0, back + 400), None, "no pause");
    }

    /// Review r3 should 1: the 5th refusal comes a frame before its snapshot, whose count is
    /// still 4 (no step): the end is cut to now. The next frame brings the count, the same end:
    /// the pause is the core's again, not over.
    #[test]
    fn a_new_count_of_wrong_passwords_cuts_afresh() {
        const NOW: Timestamp = 1_800_000_000;
        let mut cut = Cut::default();
        let core = Some(NOW + 30);
        assert_eq!(cut.end(core, 4, NOW), Some(NOW), "an old count: no step");
        assert_eq!(cut.end(core, 5, NOW), Some(NOW + 30), "the count came");
        assert_eq!(
            left(cut.end(core, 5, NOW + 1), NOW + 1),
            Some(29),
            "and counts down"
        );
    }
}
