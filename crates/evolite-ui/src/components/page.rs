//! Page parts: the screen header (title, optional count, actions on the right) and the mono
//! uppercase label above a group or a day.

use super::prelude::*;

/// «‹ {place}» above a screen's title (BUG-169; FEAT-0020 K1, frame Y1): where the screen goes
/// back to, `place` a menu item's name (English, the key). The chevron and the words, 13 Medium
/// `muted`, `text` under the pointer and when focused; 4 above the title. A link for a screen
/// reader, «Back: {place}», and the screen's first Tab stop (it is drawn first). In Arabic the
/// frame's mirror turns it over: at the right, the chevron right of the word, pointing right.
pub fn back_link(ui: &mut Ui, a: &mut Assets, place: &str) -> Response {
    let p = a.p;
    // The colour by the last frame's pointer and focus: the words are drawn before the click
    // that lies over them is registered.
    let lit_id = ui.id().with("back-link-lit");
    let lit = ui.data(|d| d.get_temp::<bool>(lit_id)).unwrap_or(false);
    let color = if lit { p.text } else { p.muted };
    let row = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.style_mut().interaction.selectable_labels = false;
            ui.add(a.icons.icon("chevron-left", color, &p, 14.0));
            ui.label(txt(place, 13.0, Weight::Medium, color));
        })
        .response;
    let resp = crate::a11y::row_click(ui, &row, "back-link");
    named(
        &resp,
        egui::WidgetType::Link,
        true,
        &format!(tr!("Back: {}"), place),
    );
    let now = resp.hovered() || resp.has_focus();
    if now != lit {
        ui.data_mut(|d| d.insert_temp(lit_id, now));
        ui.ctx().request_repaint();
    }
    ui.add_space(4.0);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// BUG-270: the size `text` (translated) is drawn Bold at, up to `size`, so that it fits in `width`
/// on one line — never below `least`. A title is never broken inside a word for want of room.
pub fn fitted_size(ui: &Ui, text: &str, size: f32, least: f32, width: f32) -> f32 {
    let wide = super::text_width(ui, text, sans(size, Weight::Bold));
    if wide <= width {
        size
    } else {
        (size * width / wide).floor().max(least)
    }
}

/// Header: 28 Bold title, an optional mono count next to it, actions on the right (drawn right
/// to left, so the primary one goes first). 41 px, as tall as a button. Where the title and the
/// actions do not fit in one row (a long title in German at 800, BUG-277's scroll bar taking its
/// 10), the actions go to a row of their own under it, at the right as well.
pub fn page_header(
    ui: &mut Ui,
    a: &mut Assets,
    title: &str,
    count: Option<&str>,
    actions: impl FnOnce(&mut Ui, &mut Assets),
) {
    let p = a.p;
    // The actions' width as the last frame drew them: they are drawn after the title.
    let actions_id = ui.id().with("page-header-actions");
    let actions_w = ui.data(|d| d.get_temp::<f32>(actions_id)).unwrap_or(0.0);
    let title_w = ui
        .painter()
        .layout_no_wrap(
            crate::i18n::t(title).into_owned(),
            sans(28.0, Weight::Bold),
            p.text,
        )
        .size()
        .x;
    let count_w = count.map_or(0.0, |count| {
        10.0 + ui
            .painter()
            .layout_no_wrap(
                crate::i18n::t(count).into_owned(),
                mono(16.0, Weight::Medium),
                p.muted,
            )
            .size()
            .x
    });
    let one_row = actions_w == 0.0 || title_w + count_w + 20.0 + actions_w <= ui.available_width();
    let title_row = |ui: &mut Ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        // BUG-299: a title wider than its row (a long @name) is drawn smaller, to 20, and past
        // that cut with «…»; the whole of it under the pointer and for a screen reader.
        let room = ui.available_width() - count_w;
        let size = fitted_size(ui, title, 28.0, 20.0, room);
        ui.add(egui::Label::new(txt(title, size, Weight::Bold, p.text)).truncate());
        if let Some(count) = count {
            ui.label(
                egui::RichText::new(crate::i18n::t(count))
                    .font(mono(16.0, Weight::Medium))
                    .color(p.muted),
            );
        }
    };
    let drawn = |ui: &mut Ui, a: &mut Assets| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            actions(ui, a);
            ui.min_rect().width()
        })
        .inner
    };
    let width = if one_row {
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 41.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                title_row(ui);
                drawn(ui, a)
            },
        )
        .inner
    } else {
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 41.0),
            Layout::left_to_right(Align::Center),
            title_row,
        );
        ui.add_space(8.0);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 41.0),
            Layout::left_to_right(Align::Center),
            |ui| drawn(ui, a),
        )
        .inner
    };
    if (width - actions_w).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(actions_id, width));
        ui.ctx().request_repaint();
    }
}

/// The wheel over the margins beside a screen's content scrolls the screen, as over the content
/// (R2a: wider than 1528 the content is a 1200 column, and the margins were dead). `panel` is the
/// whole of the screen beside the menu, `content` its column. Handed to the first scroll area
/// drawn after this call, so every screen is served by this one place. Under a dialog the pointer
/// is on the dialog's layer, not here: nothing moves.
pub fn wheel_over_margins(ui: &Ui, panel: Rect, content: Rect) {
    let beside = ui.rect_contains_pointer(panel)
        && ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|at| !content.contains(at));
    if !beside {
        return;
    }
    let wheel = ui.input(|i| i.smooth_scroll_delta().y);
    if wheel != 0.0 {
        // Already smoothed by egui: not animated a second time.
        ui.scroll_with_delta_animation(vec2(0.0, wheel), egui::style::ScrollAnimation::none());
    }
}

/// "NETWORK", "TODAY": 11 mono Medium, uppercase, letter spacing 0.88. In the Arabic interface
/// 11 sans Medium as it is: the mono and its spacing set Arabic letters apart and broke words
/// («اللغة  والتنسيق», BUG-118), and Arabic has no capitals.
pub fn section_label(ui: &mut Ui, p: &Palette, text: &str) {
    if crate::i18n::rtl() {
        ui.label(
            egui::RichText::new(crate::i18n::t(text))
                .font(sans(11.0, Weight::Medium))
                .color(p.muted),
        );
        return;
    }
    ui.label(
        egui::RichText::new(crate::i18n::t(text).to_uppercase())
            .font(mono(11.0, Weight::Medium))
            .color(p.muted)
            .extra_letter_spacing(0.88),
    );
}
