//! FEAT-0051 R-4, R-5: identifiers shown whole. [`full_id`] — the block of a card or an object's
//! screen: the id in mono, wrapped at any character, never «…», selectable, and «Copy» under it
//! (the designer, 2026-11-22). [`fit_id`] — an id in a list's line: whole when it fits, else
//! shortened in its middle to the room, not shorter than 4…4 (`text::middle_fit`, the one way to
//! shorten in the middle).

use super::prelude::*;

/// The ends an id keeps when it is shortened (R-5: «not shorter than 4…4»).
pub const ID_ENDS: (usize, usize) = (4, 4);

/// The block: `id` whole in JetBrains Mono Medium 13 / 18 (the text colour), wrapping at any
/// character, selectable as one; with `copy`, «Copy» on the last line, «Copied» in its place
/// for a moment (the clipboard's ordinary copy, not the secret one). UC-4's review before a send
/// and an app's sign-in show it without «Copy».
pub fn full_id(ui: &mut Ui, a: &mut Assets, id: &str, copy: bool) {
    let p = a.p;
    // One id on its own lines: nothing around it to keep apart from (R-6 is the lines'
    // business, where an id stands among words).
    let mut job = egui::text::LayoutJob::single_section(
        id.to_owned(),
        egui::TextFormat {
            font_id: mono(13.0, Weight::Medium),
            color: p.text,
            line_height: Some(18.0),
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width: ui.available_width(),
        max_rows: usize::MAX,
        break_anywhere: true,
        overflow_character: None,
    };
    ui.add(egui::Label::new(job).selectable(true));
    if !copy {
        return;
    }
    ui.add_space(4.0);
    let what = egui::Id::new("full-id-copy").with(id);
    let copied = crate::clipboard::just_copied(ui.ctx(), what);
    let font = sans(13.0, Weight::SemiBold);
    // As wide as the wider of the two, so nothing moves when one stands for the other.
    let width = 16.0
        + 6.0
        + text_width(ui, tr!("Copy"), font.clone()).max(text_width(
            ui,
            tr!("Copied"),
            font.clone(),
        ));
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click());
    let (icon, words, color) = if copied {
        ("check", tr!("Copied"), p.muted)
    } else {
        ("copy", tr!("Copy"), p.accent)
    };
    let icon_rect = Rect::from_min_size(pos2(rect.left(), rect.center().y - 8.0), vec2(16.0, 16.0));
    a.icons.icon(icon, color, &p, 16.0).paint_at(ui, icon_rect);
    paint_text(
        ui,
        pos2(icon_rect.right() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        words,
        font,
        color,
    );
    named(&resp, egui::WidgetType::Button, true, words);
    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        crate::clipboard::copy_answered(ui.ctx(), id.to_owned(), what);
    }
}

/// The designer's §6 (2026-11-22, 0 px): a card's «Copy» of `id` — a text action, copy 16 and
/// Medium 13 muted, 20 high, right-aligned at `right` on the line whose middle is `y` (the
/// value's companion line); «Copied» in its place for a moment, as wide as the wider of the
/// two. `what` names it for a screen reader («Copy: this wallet's address»). Its width, so the
/// line's own words can leave room for it.
pub fn copy_action_at(
    ui: &mut Ui,
    a: &mut Assets,
    id: &str,
    what: &str,
    right: f32,
    y: f32,
) -> f32 {
    let p = a.p;
    let key = egui::Id::new("card-copy").with(id);
    let copied = crate::clipboard::just_copied(ui.ctx(), key);
    let font = sans(13.0, Weight::Medium);
    let width = 16.0
        + 6.0
        + text_width(ui, tr!("Copy"), font.clone()).max(text_width(
            ui,
            tr!("Copied"),
            font.clone(),
        ));
    let rect = Rect::from_min_size(pos2(right - width, y - 10.0), vec2(width, 20.0));
    let resp = ui.interact(rect, key.with("action"), Sense::click());
    let (icon, words) = if copied {
        ("check", tr!("Copied"))
    } else {
        ("copy", tr!("Copy"))
    };
    let color = if resp.hovered() || resp.has_focus() {
        p.text
    } else {
        p.muted
    };
    let icon_rect = Rect::from_min_size(pos2(rect.left(), y - 8.0), vec2(16.0, 16.0));
    a.icons.icon(icon, color, &p, 16.0).paint_at(ui, icon_rect);
    paint_text(
        ui,
        pos2(icon_rect.right() + 6.0, y),
        Align2::LEFT_CENTER,
        words,
        font,
        color,
    );
    let name = if copied {
        crate::i18n::t(tr!("Copied")).into_owned()
    } else {
        format!(tr!("Copy: {}"), crate::i18n::spoken(what))
    };
    named(&resp, egui::WidgetType::Button, true, &name);
    if resp
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(name)
        .clicked()
    {
        crate::clipboard::copy_answered(ui.ctx(), id.to_owned(), key);
    }
    width
}

/// §6 without a companion line: the copy icon alone, 16 in a 24 × 24 area at `at` (the end of
/// the value's last line); «Copy» in its hint and its name.
pub fn copy_icon_at(ui: &mut Ui, a: &mut Assets, id: &str, what: &str, at: Rect) {
    let p = a.p;
    let key = egui::Id::new("card-copy").with(id);
    let copied = crate::clipboard::just_copied(ui.ctx(), key);
    let resp = ui.interact(at, key.with("action"), Sense::click());
    let color = if resp.hovered() || resp.has_focus() {
        p.text
    } else {
        p.muted
    };
    let icon = if copied { "check" } else { "copy" };
    a.icons
        .icon(icon, color, &p, 16.0)
        .paint_at(ui, Rect::from_center_size(at.center(), vec2(16.0, 16.0)));
    let name = if copied {
        crate::i18n::t(tr!("Copied")).into_owned()
    } else {
        format!(tr!("Copy: {}"), crate::i18n::spoken(what))
    };
    named(&resp, egui::WidgetType::Button, true, &name);
    if resp
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(name)
        .clicked()
    {
        crate::clipboard::copy_answered(ui.ctx(), id.to_owned(), key);
    }
}

/// R-5: `id` whole when `fits` says it does, else shortened in its middle — as much of both ends
/// as fits, the two balanced — not shorter than [`ID_ENDS`]. The same id in the same room is
/// always shortened the same.
pub fn fit_id(id: &str, fits: &impl Fn(&str) -> bool) -> String {
    if fits(id) {
        return id.to_owned();
    }
    crate::text::middle_fit(id, fits, ID_ENDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "8rd3Ux7pQfAbCdEfGhJkLmNoPqRsTuVwXyZkV2mJ38R";

    /// AC-A7: how much of an id fits, a pure function — whole when it fits; shortened in the
    /// middle, balanced, never shorter than 4…4; the same input, the same output.
    #[test]
    fn an_id_is_whole_or_shortened_in_its_middle_to_its_room() {
        let room = |n: usize| move |s: &str| s.chars().count() <= n;
        assert_eq!(fit_id(ID, &room(80)), ID, "whole when it fits");
        assert_eq!(fit_id(ID, &room(ID.chars().count())), ID);
        let short = fit_id(ID, &room(20));
        assert!(short.chars().count() <= 20, "{short}");
        assert!(short.contains('…'));
        let (head, tail) = short.split_once('…').unwrap();
        assert!(ID.starts_with(head) && ID.ends_with(tail), "{short}");
        assert!((head.chars().count() as i32 - tail.chars().count() as i32).abs() <= 1);
        // No room at all: still 4…4, the line's «…» is the last resort.
        let least = fit_id(ID, &room(3));
        assert_eq!(least, "8rd3…J38R");
        // From 4…4 up to the whole: each room the same answer twice, never longer than it.
        for n in 9..=ID.chars().count() {
            let once = fit_id(ID, &room(n));
            assert_eq!(once, fit_id(ID, &room(n)));
            assert!(once.chars().count() <= n, "{n}: {once}");
        }
    }

    /// AC-A8 (R-6): an id kept apart in a right-to-left line (its isolation marks) is shortened
    /// by its own characters, the marks kept around it and not counted as its ends.
    #[test]
    fn an_isolated_id_is_shortened_by_its_own_characters() {
        let isolated = format!("\u{2068}{ID}\u{2069}");
        let short =
            crate::text::middle_fit(&isolated, &|s: &str| s.chars().count() <= 14, ID_ENDS);
        assert!(
            short.starts_with('\u{2068}') && short.ends_with('\u{2069}'),
            "{short:?}"
        );
        let bare = short.trim_matches(['\u{2068}', '\u{2069}']);
        let (head, tail) = bare.split_once('…').unwrap();
        assert!(ID.starts_with(head) && ID.ends_with(tail), "{bare}");
        assert!(head.chars().count() >= 4 && tail.chars().count() >= 4, "{bare}");
    }

    /// AC-A8: in an Arabic line a shortened id keeps its order and its brackets as drawn —
    /// left to right, «(» before it and «)» after it, not turned around with the sentence.
    #[test]
    fn an_arabic_line_draws_a_shortened_id_in_its_own_order() {
        crate::i18n::set(crate::i18n::Lang::Ar);
        let short = fit_id(ID, &|s: &str| s.chars().count() <= 12);
        let line = format!("سحب إلى ({short}) مكتمل");
        let shown = crate::i18n::visual(&line).into_owned();
        crate::i18n::set(crate::i18n::Lang::En);
        assert!(shown.contains(&format!("({short})")), "{shown:?}");
    }
}
