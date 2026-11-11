//! Progress and checklists: a bar, a line with a done/no/waiting mark, a numbered step.

use super::prelude::*;

/// A 4 px bar: line track, ink fill.
pub fn progress_bar(ui: &mut Ui, p: &Palette, fraction: f32) {
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    ui.painter().rect_filled(bar, CornerRadius::same(2), p.line);
    let mut done = bar;
    done.set_width(bar.width() * fraction.clamp(0.0, 1.0));
    ui.painter().rect_filled(done, CornerRadius::same(2), p.ink);
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mark {
    Yes,
    /// Running now: sync icon, the text in SemiBold.
    Now,
    /// Not reached yet: everything muted.
    Later,
}

/// A checklist line: a 16 px mark and 13 px text. Returns the row, for a value on the right.
pub fn check_line(ui: &mut Ui, a: &mut Assets, mark: Mark, text: &str) -> Rect {
    let p = a.p;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let cy = rect.center().y;
    let (icon, icon_color, color, weight) = match mark {
        Mark::Yes => ("check", p.text, p.text, Weight::Regular),
        Mark::Now => ("sync", p.text, p.text, Weight::SemiBold),
        Mark::Later => (
            "pending",
            p.muted.gamma_multiply(0.7),
            p.muted,
            Weight::Regular,
        ),
    };
    a.icons.icon(icon, icon_color, &p, 16.0).paint_at(
        ui,
        Rect::from_center_size(pos2(rect.left() + 8.0, cy), vec2(16.0, 16.0)),
    );
    paint_text(
        ui,
        pos2(rect.left() + 26.0, cy),
        Align2::LEFT_CENTER,
        text,
        sans(13.0, weight),
        color,
    );
    rect
}

/// Where a numbered step stands while the core works through them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StepState {
    /// Not reached (or nothing under way): the plain numbered circle.
    Waiting,
    /// Under way: an ink circle.
    Now,
    /// Done: a tick in the accent colour.
    Done,
}

impl StepState {
    /// Step `n` (from 1) while the core is on step `current` (`None`: nothing under way).
    pub fn of(n: usize, current: Option<usize>) -> Self {
        match current {
            Some(c) if n < c => Self::Done,
            Some(c) if n == c => Self::Now,
            _ => Self::Waiting,
        }
    }
}

/// A numbered step: a 22 px circle with the number, a title and a muted line. The circle shows
/// where the step stands: waiting (raised), under way (ink) or done (a tick).
pub fn numbered_step_at(
    ui: &mut Ui,
    a: &mut Assets,
    n: usize,
    title: &str,
    detail: &str,
    state: StepState,
) {
    let p = a.p;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (circle, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        match state {
            StepState::Done => {
                ui.painter().circle_filled(circle.center(), 11.0, p.accent);
                a.icons.icon("check", p.on_ink, &p, 14.0).paint_at(
                    ui,
                    Rect::from_center_size(circle.center(), vec2(14.0, 14.0)),
                );
            }
            StepState::Now | StepState::Waiting => {
                let (fill, color) = if state == StepState::Now {
                    (p.ink, p.on_ink)
                } else {
                    (p.raised, p.muted)
                };
                ui.painter().circle_filled(circle.center(), 11.0, fill);
                paint_text(
                    ui,
                    circle.center(),
                    Align2::CENTER_CENTER,
                    &n.to_string(),
                    sans(12.0, Weight::SemiBold),
                    color,
                );
            }
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(txt(title, 13.0, Weight::SemiBold, p.text));
            ui.add(
                egui::Label::new(
                    txt(detail, 12.0, Weight::Regular, p.muted).line_height(Some(17.0)),
                )
                .wrap(),
            );
        });
    });
}
