//! 05 · History: every operation, filtered and searched, grouped by day.

use super::activity_filters::{self, Filters, History, Period, Status};
use super::txt;
use crate::components::{
    Assets, ButtonKind, TxRow, button, button_enabled, button_row, checkbox,
    dialog_with_header_held, field_label, kv_row, kv_row_in, notice, page_header, paint_icon_box,
    pill, section_label, status_chip, text_action, tx_row_clickable,
};
use crate::context::Ctx;
use crate::csv_export;
use crate::text::{self};
use crate::theme::{Weight, mono, sans};
use chrono::{DateTime, Local};
use egui::{Align, Layout, Margin, Stroke, Ui};
use evolite_api::{
    Activity, ActivityKind, Chain, Command, CoreError, Duffs, Flow, SavedAddress, Source,
};
use zeroize::{Zeroize, Zeroizing};

/// Where the last export's answer is kept: the file written (and how many lines), or why not.
fn saved_id() -> egui::Id {
    egui::Id::new("csv-export")
}

/// The last export's answer, when another wallet opens: its file is not this one's (BUG-45).
pub fn forget(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Result<(std::path::PathBuf, usize), String>>(saved_id()));
    // And a note half edited in a card of that wallet's operation (FEAT-0003).
    forget_note(ctx);
}

/// The last export's answer still shown, if any.
#[cfg(all(test, feature = "dev-tools"))]
pub fn saved(ctx: &egui::Context) -> Option<Result<(std::path::PathBuf, usize), String>> {
    ctx.data(|d| d.get_temp(saved_id()))
}

/// 05 · History: layer pills, «Filters», search, the filters on as chips, operations grouped by
/// day (FEAT-0009).
pub fn activity(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, h: &mut History) {
    let p = a.p;
    let wallet = cx.wallet();
    ui.spacing_mut().item_spacing.y = 18.0;
    let saved_id = saved_id();
    let mut export = false;
    page_header(ui, a, tr!("History"), None, |ui, a| {
        // «Hide the amounts» before «Export CSV» (BUG-73), and so in the Tab order too: the two
        // are laid out left to right in the place they take at the right end.
        let gap = 10.0;
        let width = crate::components::amount_toggle_width(ui, false)
            + gap
            + crate::components::button_width(ui, tr!("Export CSV"));
        let size = egui::vec2(width, ui.available_height());
        ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            crate::components::amount_toggle(ui, a, false);
            let resp = button_enabled(
                ui,
                a,
                tr!("Export CSV"),
                ButtonKind::Secondary,
                None,
                !crate::is_demo(cx.snap),
            );
            export = resp.clicked();
            #[cfg(demo_core)]
            resp.on_disabled_hover_text(
                crate::i18n::t(tr!(
                    "Nothing to export in the demo: the export takes the real history"
                ))
                .into_owned(),
            );
        });
    });
    let total = wallet.activity.len();
    let shown: Vec<usize> = h
        .shown(&wallet.activity, &wallet.address_book, &cx.now)
        .to_vec();
    // With filters on, «Export CSV» asks which lines (UC-6); with none, it writes them all.
    let mut write: Option<Vec<&Activity>> = None;
    if export {
        if shown.len() < total {
            h.export = Some(true);
        } else {
            write = Some(wallet.activity.iter().collect());
        }
    }
    if let Some(only_shown) = h.export {
        match export_question(ui, a, only_shown, (shown.len(), total), &h.filters) {
            Ask::Open(choice) => h.export = Some(choice),
            Ask::Closed => h.export = None,
            Ask::Export(only_shown) => {
                h.export = None;
                write = Some(if only_shown {
                    shown.iter().map(|i| &wallet.activity[*i]).collect()
                } else {
                    wallet.activity.iter().collect()
                });
            }
        }
    }
    if let Some(lines) = write {
        let lines: Vec<Activity> = lines.into_iter().cloned().collect();
        let csv = csv_export::taxbit(&lines, &csv_export::Own::of(wallet));
        let count = csv.lines().count().saturating_sub(1);
        let day = Local::now().format("%Y-%m-%d").to_string();
        let saved = csv_export::downloads()
            .ok_or_else(|| tr!("the “Downloads” folder was not found").to_string())
            .and_then(|dir| csv_export::save(&dir, &csv, &day).map_err(|e| e.to_string()))
            .map(|path| (path, count));
        ui.data_mut(|d| d.insert_temp(saved_id, saved));
        export = true;
    } else {
        export = false;
    }
    let saved: Option<Result<(std::path::PathBuf, usize), String>> =
        ui.data(|d| d.get_temp(saved_id));
    // The export's answer has just appeared (or a new one came) above the list: the list
    // starts again from its top, the first day and its first row right under the banner
    // (BUG-83).
    let shown_id = saved_id.with("shown");
    let banner_new =
        export || saved.is_some() && !ui.data(|d| d.get_temp(shown_id).unwrap_or(false));
    ui.data_mut(|d| d.insert_temp(shown_id, saved.is_some()));
    match saved {
        Some(Ok((path, lines))) => {
            notice(
                ui,
                a,
                "confirmed",
                p.accent,
                &format!(
                    tr!("Saved {} in DashPay's (TaxBit) format: {}"),
                    text::plural(lines as u32, tr!("operation"), tr!("operations")),
                    path.display()
                ),
            );
            // A move between own balances is no income or expense (BUG-37, rule 6).
            ui.label(txt(
                tr!("Transfers between your own balances are written as their fee only"),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            if button(ui, a, tr!("Show in folder"), ButtonKind::Ghost, None).clicked() {
                // Explorer and Finder select the file; Linux file managers open its folder.
                let mut command = if cfg!(windows) {
                    let mut c = std::process::Command::new("explorer");
                    c.arg(format!("/select,{}", path.display()));
                    c
                } else if cfg!(target_os = "macos") {
                    let mut c = std::process::Command::new("open");
                    c.arg("-R").arg(path);
                    c
                } else {
                    let mut c = std::process::Command::new("xdg-open");
                    c.arg(path.parent().unwrap_or(path.as_path()));
                    c
                };
                let _ = command.spawn();
            }
        }
        Some(Err(why)) => notice(
            ui,
            a,
            "warning",
            p.danger,
            &format!(tr!("The file was not saved: {why}."), why = why),
        ),
        None => {}
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (i, label) in [
            tr!("All"),
            "Core",
            "Platform",
            tr!("Private pool"),
            "Identity",
        ]
        .iter()
        .enumerate()
        {
            if pill(ui, &p, label, h.filters.layer == i).clicked() {
                h.filters.layer = i;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // At 1024 with the rail the search narrows; «Filters» stays (the designer's rule).
            let search_width = if ui.available_width() < 560.0 {
                200.0
            } else {
                272.0
            };
            egui::Frame::new()
                .fill(p.raised)
                .stroke(Stroke::new(1.0, p.field_line))
                .corner_radius(10)
                .inner_margin(Margin::symmetric(14, 11))
                .show(ui, |ui| {
                    // No label above: what it is for is its name (BUG-41).
                    crate::a11y::name_next_field(ui, tr!("Address, name, amount, note"));
                    crate::rtl::text_edit(
                        ui,
                        egui::TextEdit::singleline(&mut h.query)
                            .hint_text(txt(
                                tr!("Address, name, amount, note"),
                                14.0,
                                Weight::Regular,
                                p.muted,
                            ))
                            .font(sans(14.0, Weight::Regular))
                            .frame(egui::Frame::NONE)
                            .desired_width(search_width),
                    );
                });
            let count = h.filters.chips().len();
            let resp = filters_button(ui, a, count, h.open);
            if resp.clicked() {
                h.open = !h.open;
            }
            if h.open {
                filter_panel(ui, a, h, resp.rect, (shown.len(), total), cx.now);
                if !h.open {
                    resp.request_focus();
                }
            }
        });
    });
    // E2 (FEAT-0003, AC-D6): the notes file did not open and was put aside, untouched; or it
    // could not be read now and is left where it is, with no note saved over it (r1 must 1).
    if let Some(error) = wallet.notes_error {
        let why = match error {
            evolite_api::NotesError::PutAside => tr!(
                "The notes file is damaged or cannot be decrypted. It is kept next to the wallet as is; new notes go to a new file."
            ),
            evolite_api::NotesError::NotRead => tr!(
                "The notes file could not be read now. It is left as it is, and notes cannot be changed until the wallet is opened again."
            ),
        };
        crate::components::notice_parts(
            ui,
            a,
            "warning",
            p.danger,
            &[(tr!("Notes could not be read"), why)],
        );
    }
    // The filters on, each with its cross, and how many lines they leave (UC-5).
    let chips = h.filters.chips();
    if !chips.is_empty() || shown.len() < total {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for chip in &chips {
                if filter_chip(ui, a, &activity_filters::chip_label(*chip)).clicked() {
                    h.filters.remove(*chip);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, a, tr!("Reset"), ButtonKind::Ghost, None).clicked() {
                    h.filters.reset();
                    h.filters.layer = 0;
                    h.query.clear();
                }
                ui.label(txt(
                    &shown_of(shown.len(), total),
                    13.0,
                    Weight::Regular,
                    p.muted,
                ));
            });
        });
    }
    let mut list = egui::ScrollArea::vertical().auto_shrink(false);
    if banner_new {
        list = list.vertical_scroll_offset(0.0);
    }
    let filtered = !chips.is_empty();
    list.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        if shown.is_empty() {
            ui.add_space(40.0);
            if filtered {
                // UC-8: the panel's filters leave nothing.
                ui.vertical_centered(|ui| {
                    ui.label(txt(
                        tr!("Nothing matches these filters"),
                        14.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    ui.add_space(12.0);
                    if button(ui, a, tr!("Reset the filters"), ButtonKind::Secondary, None)
                        .clicked()
                    {
                        h.filters.reset();
                    }
                });
            } else {
                // Nothing at all is not "nothing found".
                let empty = if wallet.activity.is_empty() {
                    tr!("No operations yet. Receive DASH on the “Receive” screen.")
                } else {
                    tr!("Nothing found")
                };
                ui.vertical_centered(|ui| ui.label(txt(empty, 14.0, Weight::Regular, p.muted)));
            }
        }
        // A day's heading only above its lines: a day the filters empty has none (AC-D3).
        let mut day = String::new();
        for act in shown.iter().map(|i| &wallet.activity[*i]) {
            let this_day = text::day(act.at, cx.now);
            if this_day != day {
                day = this_day;
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    section_label(ui, &p, &day);
                });
                ui.add_space(4.0);
            }
            let mut detail = format!(
                "{} · {}",
                text::chain(act.chain),
                text::time_of_day(act.at, cx.now)
            );
            // How a withdrawal or a request to us stands, and which identity of ours it
            // came to (BUG-58).
            for part in [
                text::activity_for(act),
                text::activity_status(act).map(str::to_owned),
            ]
            .into_iter()
            .flatten()
            {
                detail.push_str(" · ");
                detail.push_str(crate::i18n::t(&part).as_ref());
            }
            let row = tx_row_clickable(
                ui,
                a,
                &TxRow {
                    icon: text::activity_icon(act),
                    title: &text::activity_title_in(act, &wallet.address_book),
                    detail: &detail,
                    proof: Some(act.proof),
                    amount: &text::signed_amount(act.amount),
                    alert: text::activity_alert(act),
                    note: act.note.as_deref(),
                },
            );
            if row.clicked() {
                open_card(ui.ctx(), act.id);
            }
        }
    });
    card_window(ui, a, cx);
}

/// «Shown N of M», the numbers as the number format writes them.
fn shown_of(n: usize, of: usize) -> String {
    format!(
        tr!("Shown {} of {}"),
        crate::format::count(n as u64),
        crate::format::count(of as u64)
    )
}

/// `history/filters-button`: a pill with the filter icon and how many filters are on; raised
/// while its panel is open. Its name says the number too (AC-T10).
fn filters_button(ui: &mut Ui, a: &mut Assets, on: usize, open: bool) -> egui::Response {
    let p = a.p;
    let words = crate::i18n::t(tr!("Filters")).into_owned();
    let label = if on > 0 {
        format!("{words} {on}")
    } else {
        words
    };
    let font = sans(13.0, Weight::SemiBold);
    let text_w = ui
        .painter()
        .layout_no_wrap(label.clone(), font.clone(), p.text)
        .size()
        .x;
    let size = egui::vec2(16.0 + 16.0 + 6.0 + text_w + 14.0, 33.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let color = if open || on > 0 || resp.hovered() {
        p.text
    } else {
        p.muted
    };
    let fill = if open { p.raised } else { p.surface };
    let stroke = if on > 0 { p.ink } else { p.line };
    ui.painter().rect(
        rect,
        255,
        fill,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    // Laid out left to right; the RTL mirror turns the frame over (rtl.rs).
    let icon_x = rect.left() + 16.0 + 8.0;
    let icon =
        egui::Rect::from_center_size(egui::pos2(icon_x, rect.center().y), egui::vec2(16.0, 16.0));
    a.icons.icon("filter", color, &p, 16.0).paint_at(ui, icon);
    let (text_at, align) = (icon.right() + 6.0, egui::Align2::LEFT_CENTER);
    ui.painter().text(
        egui::pos2(text_at, rect.center().y),
        align,
        &label,
        font,
        color,
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// `history/active-filter-chip`: a raised pill with the filter's words and a cross 12; a click
/// takes the filter off. Named «Remove the filter «…»» (AC-T10).
fn filter_chip(ui: &mut Ui, a: &mut Assets, label: &str) -> egui::Response {
    let p = a.p;
    let words = crate::i18n::t(label).into_owned();
    let font = sans(13.0, Weight::Medium);
    let text_w = ui
        .painter()
        .layout_no_wrap(words.clone(), font.clone(), p.text)
        .size()
        .x;
    let size = egui::vec2(12.0 + text_w + 6.0 + 12.0 + 10.0, 29.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    ui.painter().rect(
        rect,
        255,
        p.raised,
        Stroke::new(1.0, p.line),
        egui::StrokeKind::Inside,
    );
    let (text_at, align, cross_x) = (
        rect.left() + 12.0,
        egui::Align2::LEFT_CENTER,
        rect.right() - 16.0,
    );
    ui.painter().text(
        egui::pos2(text_at, rect.center().y),
        align,
        &words,
        font,
        p.text,
    );
    let cross =
        egui::Rect::from_center_size(egui::pos2(cross_x, rect.center().y), egui::vec2(12.0, 12.0));
    let color = if resp.hovered() { p.text } else { p.muted };
    a.icons.icon("close", color, &p, 12.0).paint_at(ui, cross);
    let name = format!(tr!("Remove the filter “{}”"), words);
    let name = crate::i18n::t(&name).into_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// `history/filter-panel`: under «Filters», 380 wide, not modal. The filters work at once;
/// «Done» and Esc only close it.
fn filter_panel(
    ui: &mut Ui,
    a: &mut Assets,
    h: &mut History,
    button: egui::Rect,
    counts: (usize, usize),
    now: DateTime<Local>,
) {
    let p = a.p;
    const WIDTH: f32 = 380.0;
    // Down from the button, its right edge on the button's, inside the window (the RTL
    // mirror turns it over with the frame).
    let screen = ui.ctx().content_rect();
    let x = (button.right() - WIDTH).clamp(
        screen.left() + 8.0,
        (screen.right() - WIDTH - 8.0).max(screen.left() + 8.0),
    );
    egui::Area::new(egui::Id::new("history-filter-panel"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(x, button.bottom() + 8.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.line))
                .corner_radius(16)
                .shadow(egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(if p.dark { 120 } else { 40 }),
                })
                .inner_margin(Margin::same(20))
                .show(ui, |ui| {
                    ui.set_width(WIDTH - 40.0);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    panel_body(ui, a, h, counts, now);
                });
        });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        h.open = false;
    }
}

fn panel_body(
    ui: &mut Ui,
    a: &mut Assets,
    h: &mut History,
    (shown, total): (usize, usize),
    now: DateTime<Local>,
) {
    let p = a.p;
    let f = &mut h.filters;
    section_label(ui, &p, tr!("Direction"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for flow in [None, Some(Flow::In), Some(Flow::Out), Some(Flow::Between)] {
            let label = activity_filters::flow_label(flow);
            if pill(ui, &p, label, f.flow == flow).clicked() {
                f.flow = flow;
            }
        }
    });
    section_label(ui, &p, tr!("Status"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for status in [
            None,
            Some(Status::Pending),
            Some(Status::Confirmed),
            Some(Status::Final),
        ] {
            let label = activity_filters::status_label(status);
            if pill(ui, &p, label, f.status == status).clicked() {
                f.status = status;
            }
        }
    });
    section_label(ui, &p, tr!("Type"));
    egui::Grid::new("history-filter-types")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            for (n, (bit, label)) in activity_filters::kind::ALL.iter().enumerate() {
                let mut on = f.kinds & bit != 0;
                checkbox(ui, a, &mut on, label);
                if on {
                    f.kinds |= bit;
                } else {
                    f.kinds &= !bit;
                }
                if n % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    section_label(ui, &p, tr!("Period"));
    let custom = matches!(f.period, Period::Custom(..));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for period in [
            Period::All,
            Period::Today,
            Period::Days(7),
            Period::Days(30),
            Period::ThisYear,
        ] {
            let label = activity_filters::period_label(period);
            if pill(ui, &p, &label, f.period == period).clicked() {
                f.period = period;
            }
        }
        if pill(ui, &p, tr!("Custom…"), custom).clicked() && !custom {
            let today = now.date_naive();
            let from = today
                .checked_sub_days(chrono::Days::new(29))
                .unwrap_or(today);
            f.period = Period::Custom(from, today);
            h.from = activity_filters::date_text(from);
            h.to = activity_filters::date_text(today);
        }
    });
    if let Period::Custom(..) = h.filters.period {
        date_fields(ui, a, h);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if button(ui, a, tr!("Reset"), ButtonKind::Ghost, None).clicked() {
            h.filters.reset();
        }
        ui.label(txt(&shown_of(shown, total), 13.0, Weight::Regular, p.muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked() {
                h.open = false;
            }
        });
    });
}

/// «from … to …» of the custom period: Mono, with the calendar icon; a date that does not read
/// says so under the fields. The period follows the fields once both read.
fn date_fields(ui: &mut Ui, a: &mut Assets, h: &mut History) {
    let p = a.p;
    let mut wrong = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (word, from) in [(tr!("from"), true), (tr!("to"), false)] {
            ui.label(txt(word, 13.0, Weight::Regular, p.muted));
            let typed = if from { &mut h.from } else { &mut h.to };
            egui::Frame::new()
                .fill(p.raised)
                .stroke(Stroke::new(1.0, p.field_line))
                .corner_radius(10)
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        crate::a11y::name_next_field(ui, word);
                        crate::rtl::text_edit(
                            ui,
                            egui::TextEdit::singleline(typed)
                                .hint_text(txt("31.10.2026", 13.0, Weight::Regular, p.muted))
                                .font(mono(13.0, Weight::Regular))
                                .frame(egui::Frame::NONE)
                                .desired_width(92.0),
                        );
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        a.icons.icon("calendar", p.muted, &p, 16.0).paint_at(ui, r);
                    });
                });
            wrong |= activity_filters::parse_date(typed).is_none();
        }
    });
    if let (Some(from), Some(to)) = (
        activity_filters::parse_date(&h.from),
        activity_filters::parse_date(&h.to),
    ) {
        h.filters.period = Period::Custom(from, to);
    }
    if wrong {
        ui.label(txt(
            tr!("A date as 31.10.2026"),
            12.0,
            Weight::Regular,
            p.danger,
        ));
    }
}

/// What the export question answered this frame.
enum Ask {
    Open(bool),
    Closed,
    Export(bool),
}

/// `dialog/export-filtered` (UC-6): only the shown lines (the default) or the whole history.
fn export_question(
    ui: &mut Ui,
    a: &mut Assets,
    mut only_shown: bool,
    (shown, total): (usize, usize),
    filters: &Filters,
) -> Ask {
    let mut go = false;
    let closed = crate::components::dialog(
        ui,
        a,
        "history-export-filtered",
        "export-csv",
        tr!("Export CSV"),
        440.0,
        |ui, a| {
            let p = a.p;
            ui.label(txt(
                tr!("Filters are on. What to export?"),
                14.0,
                Weight::Regular,
                p.text,
            ));
            let what = filters
                .chips()
                .into_iter()
                .map(|c| crate::i18n::t(&activity_filters::chip_label(c)).into_owned())
                .collect::<Vec<_>>()
                .join(" · ");
            let only = format!(
                tr!("Only the shown ones · {}"),
                crate::format::count(shown as u64)
            );
            if crate::components::option_card(ui, a, "check", &only, &what, only_shown).clicked() {
                only_shown = true;
            }
            let all = format!(
                tr!("The whole history · {}"),
                crate::format::count(total as u64)
            );
            if crate::components::option_card(ui, a, "activity", &all, "", !only_shown).clicked() {
                only_shown = false;
            }
            let mut close = false;
            button_row(ui, |ui| {
                if button(ui, a, tr!("Export"), ButtonKind::Primary, None).clicked() {
                    go = true;
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close || go
        },
    );
    if go {
        Ask::Export(only_shown)
    } else if closed {
        Ask::Closed
    } else {
        Ask::Open(only_shown)
    }
}

/// The open operation's card, if one is open: History and Overview's latest operations show
/// the same one.
pub fn card_window(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let wallet = cx.wallet();
    let open = ui.data(|d| d.get_temp::<u64>(card_id()));
    if let Some(act) = open.and_then(|id| wallet.activity.iter().find(|a| a.id == id)) {
        let (closed, again, go, note) = card(ui, a, act, cx.now, &wallet.address_book);
        if let Some(command) = note {
            cx.send(command);
        }
        if closed {
            ui.data_mut(|d| d.remove::<u64>(card_id()));
            // A note half edited goes with its card.
            forget_note(ui.ctx());
        }
        // "Open in Contacts": accepting is there, with the password (BUG-42, BUG-58).
        // "Continue on the Identity screen": the cut-short top-up or creation is finished
        // there, with the password (BUG-57).
        if let Some(screen) = go {
            cx.go(screen);
        }
        // "Send again": the Send screen with the same recipient and amount.
        if let Some((to, amount)) = again {
            cx.pay_again(Source::Core, to, amount);
        }
    }
}

/// Where the open card's operation is kept between frames.
fn card_id() -> egui::Id {
    egui::Id::new("activity-card")
}

/// Opens the card of operation `id` (a row clicked; `--screen activity-card`).
pub fn open_card(ctx: &egui::Context, id: u64) {
    ctx.data_mut(|d| d.insert_temp(card_id(), id));
}

/// The note being edited in the open card (FEAT-0003, AC-D4): its operation's key, the text as
/// typed, whether the core is writing it and why the last write failed.
#[derive(Clone, Default)]
pub struct NoteEdit {
    pub key: String,
    pub text: Zeroizing<String>,
    pub waiting: bool,
    /// N10 with the core's reason, and its details.
    pub error: Option<(String, Option<String>)>,
    /// The focus was put at the end of the text.
    placed: bool,
}

/// A note just written, shown until the core's next snapshot carries it.
#[derive(Clone)]
struct NoteSaved {
    key: String,
    note: Option<Zeroizing<String>>,
}

fn note_edit_id() -> egui::Id {
    egui::Id::new("activity-note-edit")
}

fn note_saved_id() -> egui::Id {
    egui::Id::new("activity-note-saved")
}

/// The note being edited, if any.
pub fn note_edit(ctx: &egui::Context) -> Option<NoteEdit> {
    ctx.data(|d| d.get_temp::<NoteEdit>(note_edit_id()))
}

/// Opens the note of operation `key` for editing with `text` in it (a click on «Edit» or «Add a
/// note»; the presets).
pub fn edit_note(ctx: &egui::Context, key: &str, text: &str) {
    let edit = NoteEdit {
        key: key.to_owned(),
        text: Zeroizing::new(text.to_owned()),
        ..Default::default()
    };
    ctx.data_mut(|d| d.insert_temp(note_edit_id(), edit));
}

fn put_note_edit(ctx: &egui::Context, edit: NoteEdit) {
    ctx.data_mut(|d| d.insert_temp(note_edit_id(), edit));
}

/// The core's answer to [`Command::SetNote`] (`Done { what: "note" }`): written — back to the
/// note as shown, with the new text; not — the reason under the field, the text kept for
/// another try (AC-D5).
pub fn on_note_done(ctx: &egui::Context, result: &Result<(), CoreError>) {
    let Some(mut edit) = note_edit(ctx).filter(|e| e.waiting) else {
        return;
    };
    match result {
        Ok(()) => {
            let note = evolite_core::note_text::kept(&edit.text)
                .ok()
                .flatten()
                .map(Zeroizing::new);
            let saved = NoteSaved {
                key: std::mem::take(&mut edit.key),
                note,
            };
            ctx.data_mut(|d| {
                d.remove::<NoteEdit>(note_edit_id());
                d.insert_temp(note_saved_id(), saved);
            });
        }
        Err(e) => {
            edit.waiting = false;
            edit.error = Some((
                text::failed(tr!("The note was not saved"), e),
                text::error_details(e),
            ));
            put_note_edit(ctx, edit);
        }
    }
}

/// `--screen activity-card-note-error`: the note of `key` edited to `text`, and its write failed
/// as on a full disk (the designer's frame 7).
#[cfg(feature = "dev-tools")]
pub fn note_failed_example(ctx: &egui::Context, key: &str, text: &str) {
    edit_note(ctx, key, text);
    if let Some(mut edit) = note_edit(ctx) {
        edit.waiting = true;
        put_note_edit(ctx, edit);
    }
    on_note_done(
        ctx,
        &Err(CoreError::Failed("notes.enc: storage_full".into())),
    );
}
/// Forgets a note being edited or just written: the card closed, the wallet locked or another
/// one opened (AC-T6).
pub fn forget_note(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<NoteEdit>(note_edit_id());
        d.remove::<NoteSaved>(note_saved_id());
    });
}

/// The note the card shows for `act`: one just written until the snapshot has it.
fn shown_note(ctx: &egui::Context, act: &Activity) -> Option<String> {
    let saved = ctx.data(|d| d.get_temp::<NoteSaved>(note_saved_id()));
    match saved {
        Some(saved) if saved.key == act.key => {
            let note = saved.note.as_deref().map(|n| n.to_string());
            if note == act.note {
                ctx.data_mut(|d| d.remove::<NoteSaved>(note_saved_id()));
            }
            note
        }
        _ => act.note.clone(),
    }
}

/// The note's block in the card (FEAT-0003, AC-D4, AC-D5): the note with «Edit», a dashed «+ Add
/// a note» when there is none, or the note edited in place — Enter saves, Escape cancels, an
/// emptied note is deleted. The command to send when saved.
fn note_block(ui: &mut Ui, a: &mut Assets, act: &Activity) -> Option<Command> {
    let p = a.p;
    let ctx = ui.ctx().clone();
    let note = shown_note(&ctx, act);
    let Some(mut edit) = note_edit(&ctx).filter(|e| e.key == act.key) else {
        match &note {
            Some(note) => {
                note_frame(&p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.horizontal(|ui| {
                        note_heading(ui, a);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if text_action(ui, a, "edit", tr!("Edit")).clicked() {
                                edit_note(ui.ctx(), &act.key, note);
                            }
                        });
                    });
                    ui.add(
                        egui::Label::new(txt(
                            &crate::i18n::own(note),
                            14.0,
                            Weight::Regular,
                            p.text,
                        ))
                        .wrap(),
                    );
                });
            }
            None => {
                if add_note_line(ui, a).clicked() {
                    edit_note(ui.ctx(), &act.key, "");
                }
            }
        }
        return None;
    };
    let mut command = None;
    let mut cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
    note_frame(&p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        let full =
            evolite_core::note_text::graphemes(&edit.text) >= evolite_api::NOTE_MAX_GRAPHEMES;
        ui.horizontal(|ui| {
            note_heading(ui, a);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(crate::screens::send::note_counter(&edit.text))
                        .font(mono(12.0, Weight::Regular))
                        .color(if full { p.danger } else { p.muted }),
                );
            });
        });
        let id = ui.make_persistent_id("note-edit-field");
        let focused = ui.memory(|m| m.has_focus(id));
        let field = egui::Frame::new()
            .fill(p.raised)
            .stroke(Stroke::new(
                1.0,
                if focused { p.text } else { p.field_line },
            ))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                crate::a11y::name_next_field(ui, tr!("Note · only you see it"));
                let mut typed = edit.text.to_string();
                let resp = ui.add_enabled(
                    !edit.waiting,
                    egui::TextEdit::multiline(&mut typed)
                        .id(id)
                        .font(sans(14.0, Weight::Regular))
                        .text_color(p.text)
                        .frame(egui::Frame::NONE)
                        // Enter saves: no line breaks in a note.
                        .return_key(None)
                        .desired_rows(1)
                        .desired_width(f32::INFINITY),
                );
                crate::a11y::take_field_name(ui, &resp);
                if resp.changed() {
                    *edit.text = evolite_core::note_text::fit(&typed).0;
                    edit.error = None;
                }
                typed.zeroize();
                resp
            })
            .inner;
        // Opened: the focus in the field, the cursor after the last character.
        if !edit.placed {
            edit.placed = true;
            ui.memory_mut(|m| m.request_focus(id));
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let end = egui::text::CCursor::new(edit.text.chars().count());
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), id);
            } else {
                let mut state = egui::text_edit::TextEditState::default();
                let end = egui::text::CCursor::new(edit.text.chars().count());
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), id);
            }
        }
        let enter = field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if let Some((why, details)) = &edit.error {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(a.icons.icon("warning", p.danger, &p, 14.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.add(egui::Label::new(txt(why, 12.0, Weight::Medium, p.danger)).wrap());
                    if let Some(details) = details {
                        crate::components::details_line(ui, &p, details);
                    }
                });
            });
        } else {
            ui.add(
                egui::Label::new(txt(
                    tr!("Enter saves, Esc cancels. An empty note is deleted."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ))
                .wrap(),
            );
        }
        let kept = evolite_core::note_text::kept(&edit.text).ok().flatten();
        let deletes = kept.is_none() && note.is_some();
        let can_save = !edit.waiting && (kept.is_some() || note.is_some());
        let label = if edit.waiting {
            tr!("Saving…")
        } else if deletes {
            tr!("Delete the note")
        } else {
            tr!("Save")
        };
        let mut save = enter && can_save;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if button_enabled(ui, a, label, ButtonKind::Primary, None, can_save).clicked() {
                save = true;
            }
            if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                cancel = true;
            }
        });
        if save && !cancel {
            if kept == note {
                // Nothing changed: nothing to write.
                cancel = true;
            } else {
                edit.waiting = true;
                edit.error = None;
                command = Some(Command::SetNote {
                    key: act.key.clone(),
                    text: evolite_api::NoteText(kept.unwrap_or_default()),
                });
            }
        }
    });
    if cancel {
        ctx.data_mut(|d| d.remove::<NoteEdit>(note_edit_id()));
    } else {
        put_note_edit(&ctx, edit);
    }
    command
}

/// The note's block: `raised`, r12 (Figma 24).
fn note_frame(p: &crate::theme::Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.raised.gamma_multiply(0.7))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 12))
}

/// N5 with the `document` icon.
fn note_heading(ui: &mut Ui, a: &mut Assets) {
    let p = a.p;
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.add(a.icons.icon("document", p.muted, &p, 14.0));
    ui.label(txt(
        tr!("Note · only you see it"),
        12.0,
        Weight::Medium,
        p.muted,
    ));
}

/// «+ Add a note»: a dashed line, 44 px (Figma, card-add).
fn add_note_line(ui: &mut Ui, a: &mut Assets) -> egui::Response {
    let p = a.p;
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::click());
    crate::components::named(&resp, egui::WidgetType::Button, true, tr!("Add a note"));
    let over = resp.hovered() || resp.has_focus();
    let color = if over { p.text } else { p.field_line };
    let r = rect.shrink(0.5);
    let corners = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    ui.painter().extend(egui::Shape::dashed_line(
        &corners,
        Stroke::new(1.0, color),
        5.0,
        4.0,
    ));
    let text_color = if over { p.text } else { p.muted };
    let icon = a.icons.icon("plus", text_color, &p, 14.0);
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 14.0 + 7.0, rect.center().y),
        egui::vec2(14.0, 14.0),
    );
    icon.paint_at(ui, icon_rect);
    crate::components::paint_text(
        ui,
        egui::pos2(icon_rect.right() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        tr!("Add a note"),
        sans(13.0, Weight::Medium),
        text_color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The block explorer's page of a transaction, on the network of this run.
fn explorer(txid: &str) -> String {
    match evolite_core::network::current() {
        evolite_api::Network::Testnet => {
            format!("https://insight.testnet.networks.dash.org/insight/tx/{txid}")
        }
        evolite_api::Network::Mainnet => format!("https://insight.dash.org/insight/tx/{txid}"),
    }
}

/// Who an operation was with, as its card says it: the label, the value (the whole address
/// when the wallet knows it) and, for a contact paid at an address of its key, a line naming it.
fn counterparty(act: &Activity) -> Option<(&'static str, String, Option<String>)> {
    match &act.kind {
        ActivityKind::Received { from: Some(who) } => Some((tr!("Sender"), who.clone(), None)),
        ActivityKind::Sent { to } => match &act.address {
            // A contact (by name or short id) is paid at a new address of its key.
            Some(address) if to.starts_with('@') || to.contains('…') => Some((
                tr!("To"),
                address.clone(),
                Some(format!(
                    tr!("{to} · a new address from the contact's key"),
                    to = to
                )),
            )),
            Some(address) => Some((tr!("To"), address.clone(), None)),
            None if !to.is_empty() => Some((tr!("To"), to.clone(), None)),
            None => None,
        },
        ActivityKind::CreditTransfer { to } | ActivityKind::Withdrawal { to, .. } => {
            Some((tr!("To"), to.clone(), None))
        }
        // Which identity of ours sent it, when the wallet has several (BUG-69).
        ActivityKind::ContactRequest { to, ours, .. } => Some((
            tr!("To"),
            to.clone(),
            ours.as_ref().map(|o| format!(tr!("from {name}"), name = o)),
        )),
        ActivityKind::ContactRequestIn { from, ours, .. } => Some((
            tr!("Sender"),
            from.clone(),
            ours.as_ref().map(|o| format!(tr!("for {name}"), name = o)),
        )),
        // An asset lock: the identity it registered or tops up, when the core knows it — one of
        // this wallet's (BUG-57).
        ActivityKind::TopUp { identity, .. } if !identity.is_empty() => Some((
            tr!("To"),
            identity.clone(),
            Some(tr!("your identity").into()),
        )),
        // A creation cut short: the identity is not there yet.
        ActivityKind::TopUp { creates: true, .. } => Some((
            tr!("To"),
            "Dash Platform".into(),
            Some(tr!("A new identity of this wallet").into()),
        )),
        // The lock itself does not name it: one of this wallet's identities or its Platform
        // address, and nothing else.
        ActivityKind::TopUp { .. } => Some((
            tr!("To"),
            "Dash Platform".into(),
            Some(tr!("An identity or the Platform address of this wallet").into()),
        )),
        _ => None,
    }
}

/// A label over a value in mono that wraps (an address, a txid), with an optional muted line.
fn mono_block(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    label: &str,
    value: &str,
    note: Option<&str>,
) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, p, label);
        ui.add(
            egui::Label::new(
                egui::RichText::new(crate::i18n::t(value))
                    .font(mono(12.0, Weight::Regular))
                    .color(p.text)
                    .line_height(Some(18.0)),
            )
            .wrap(),
        );
        if let Some(note) = note {
            ui.label(txt(note, 12.0, Weight::Regular, p.muted));
        }
    });
}

/// What the card asks for: closed, the payment to make again, the screen to go to, the note
/// to write (FEAT-0003).
type CardAnswer = (
    bool,
    Option<(String, Duffs)>,
    Option<crate::screens::Screen>,
    Option<Command>,
);

/// 24 · Operation details: what and where, how much and its status, when, the fee and the block,
/// with whom, and the txid to copy or look up; "Send again" for a payment out, "Open in
/// Contacts" for a request to us not answered, "Continue on the Identity screen" for an asset
/// lock whose credits did not come (BUG-57). Whether it closed, the payment to make again when
/// asked, and the screen asked for.
fn card(
    ui: &mut Ui,
    a: &mut Assets,
    act: &Activity,
    now: DateTime<Local>,
    book: &[SavedAddress],
) -> CardAnswer {
    let repeat = again(act);
    let mut note = None;
    // A note edited in place: Escape and a click outside are its own, the card stays (AC-D4).
    let editing = note_edit(ui.ctx()).is_some_and(|e| !act.key.is_empty() && e.key == act.key);
    let mut paid_again = None;
    let mut go = None;
    let answer_there = matches!(
        act.kind,
        ActivityKind::ContactRequestIn {
            state: evolite_api::RequestState::Waiting,
            ..
        }
    );
    let not_added = matches!(
        act.kind,
        ActivityKind::TopUp {
            credits: evolite_api::Credits::NotAdded,
            ..
        }
    );
    let closed = dialog_with_header_held(
        ui,
        a,
        "activity-card",
        464.0,
        editing,
        |ui, a| {
            let p = a.p;
            let (boxr, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
            paint_icon_box(ui, a, boxr, 12, text::activity_icon(act), 22.0, p.ink);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(4.0);
                ui.label(txt(
                    &text::activity_title_in(act, book),
                    20.0,
                    Weight::Bold,
                    p.text,
                ));
                ui.label(txt(
                    &format!("{} · {}", text::chain(act.chain), text::when(act.at, now)),
                    14.0,
                    Weight::Medium,
                    p.muted,
                ));
            });
        },
        |ui, a| {
            let p = a.p;
            ui.horizontal(|ui| {
                crate::components::amount_label(
                    ui,
                    &text::signed_amount(act.amount),
                    mono(28.0, Weight::Medium),
                    p.text,
                );
                // The app's one «Hide amounts», as an icon right after the sum (BUG-73): a
                // click hides the list under the card too.
                ui.add_space(8.0 - ui.spacing().item_spacing.x);
                crate::components::amount_toggle(ui, a, true);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    status_chip(ui, a, act.proof);
                });
            });
            // The person's note, under the amount (FEAT-0003, Figma 24); none where no note can
            // be kept.
            if !act.key.is_empty() {
                note = note_block(ui, a, act);
            }
            // Mono is for amounts, addresses and hashes; the date and the chain are words.
            let words = sans(13.0, Weight::Medium);
            let value = mono(13.0, Weight::Medium);
            kv_row(
                ui,
                &p,
                tr!("When"),
                &text::date_time(act.at, now),
                words.clone(),
            );
            kv_row(ui, &p, tr!("Where"), text::chain(act.chain), words.clone());
            if let Some(status) = text::activity_status(act) {
                kv_row(ui, &p, tr!("Status"), status, words.clone());
            }
            // An asset lock: the chip above is the Core transaction's; whether Platform added
            // the credits is said apart, in red when it did not (BUG-57, customer: red).
            if let Some((credits, bad)) = text::activity_credits(act) {
                let color = if bad { p.danger } else { p.text };
                kv_row_in(ui, &p, tr!("Credits"), credits, words, color);
            }
            if let Some(fee) = act.fee {
                kv_row(
                    ui,
                    &p,
                    tr!("Fee"),
                    // Already inside the amount above: shown as a size, not a second debit
                    //.
                    &crate::format::amount(fee),
                    value.clone(),
                );
            }
            if let Some(block) = act.block {
                kv_row(
                    ui,
                    &p,
                    tr!("Block"),
                    &text::group_thousands(block as i64),
                    value,
                );
            }
            if let Some((label, who, note)) = counterparty(act) {
                // An address the person named says so.
                let note = note.or_else(|| {
                    text::book_label(act, book)
                        .map(|l| format!(tr!("“{l}” · from the address book"), l = l))
                });
                mono_block(ui, &p, label, &who, note.as_deref());
            }
            if not_added {
                let creates = matches!(act.kind, ActivityKind::TopUp { creates: true, .. });
                notice(
                    ui,
                    a,
                    "warning",
                    crate::components::attention_color(&p),
                    if creates {
                        tr!(
                            "The identity's creation was interrupted. You can continue it on the Identity screen; the DASH are not lost"
                        )
                    } else {
                        tr!(
                            "The top-up was interrupted. You can continue it on the Identity screen; the DASH are not lost"
                        )
                    },
                );
            }
            let mut close = false;
            if let Some(txid) = &act.txid {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    mono_block(ui, &p, "txid", txid, None);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        // Two outlined buttons with a gap: a ghost one beside an outlined one read as
                        // one control.
                        ui.spacing_mut().item_spacing.x = 12.0;
                        // "Copied" in its place for a moment, as on "Receive" (BUG-39 №1).
                        let copy_id = egui::Id::new("copy-txid").with(act.id);
                        let label = if crate::clipboard::just_copied(ui.ctx(), copy_id) {
                            tr!("Copied")
                        } else {
                            tr!("Copy txid")
                        };
                        if button(ui, a, label, ButtonKind::Secondary, None).clicked() {
                            crate::clipboard::copy_answered(ui.ctx(), txid.clone(), copy_id);
                        }
                        if button(
                            ui,
                            a,
                            tr!("Open in the explorer"),
                            ButtonKind::Secondary,
                            None,
                        )
                        .on_hover_text(crate::i18n::t(&explorer(txid)).into_owned())
                        .clicked()
                        {
                            // egui's open_url does nothing without eframe's `links`.
                            crate::links::open(&explorer(txid));
                        }
                    });
                });
                notice(
                    ui,
                    a,
                    "shield",
                    p.accent,
                    tr!(
                        "The explorer opens only when you click. The explorer's site learns this computer's address and which operation you look at, so evolite opens it only by the button. You can simply copy the txid."
                    ),
                );
            }
            // The action on the right, «Close» on its left: as in every dialog (designer review №8).
            button_row(ui, |ui| {
                if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                if answer_there
                    && button(ui, a, tr!("Open in Contacts"), ButtonKind::Secondary, None).clicked()
                {
                    go = Some(crate::screens::Screen::Contacts);
                    close = true;
                }
                if not_added
                    && button(
                        ui,
                        a,
                        tr!("Continue on the Identity screen"),
                        ButtonKind::Secondary,
                        None,
                    )
                    .clicked()
                {
                    go = Some(crate::screens::Screen::Identity);
                    close = true;
                }
                if let Some(repeat) = &repeat
                    && button(ui, a, tr!("Send again"), ButtonKind::Secondary, None)
                        .on_hover_text(
                            crate::i18n::t(tr!("The same amount to the same recipient — we show everything again before signing"),)
                        .into_owned())
                        .clicked()
                {
                    paid_again = Some(repeat.clone());
                    close = true;
                }
            });
            close
        },
    );
    (closed, paid_again, go, note)
}

/// What "Send again" pays: a Core payment out, to its @name or address, the amount
/// without the fee. Not for a contact without a name (its short id cannot be paid) or with
/// nothing known of the amount.
fn again(act: &Activity) -> Option<(String, Duffs)> {
    let ActivityKind::Sent { to } = &act.kind else {
        return None;
    };
    let recipient = if to.starts_with('@') {
        to.clone()
    } else {
        act.address.clone().unwrap_or_else(|| to.clone())
    };
    let amount = -act.amount - act.fee.unwrap_or(0);
    (act.chain == Chain::Core && !recipient.is_empty() && !recipient.contains('…') && amount > 0)
        .then_some((recipient, amount))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::activity_filters::matches_query;
    use evolite_api::Proof;

    fn sent(to: &str, amount: i64) -> Activity {
        Activity {
            id: 0x536e_90c0_1af4_19f4,
            kind: ActivityKind::Sent { to: to.into() },
            chain: Chain::Core,
            at: 0,
            amount,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            note: None,
        }
    }

    #[test]
    fn finds_by_address_name_amount_and_txid() {
        let act = sent("yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2", -123_400_000_000);
        // The whole address, though the row shows it short.
        assert!(matches_query(&act, "vve69jqf6", &[]));
        assert!(matches_query(&act, "sent", &[]));
        // Thousands as the interface language writes them.
        assert!(matches_query(&act, "1,234", &[]));
        // A pasted txid, or its start.
        assert!(matches_query(
            &act,
            "536e90c01af419f42cd2b9430f318b26031c9532c86013a409d529b02fc802c4",
            &[]
        ));
        // By the name the person gave the address.
        let book = [SavedAddress {
            address: "yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2".into(),
            label: "Exchange".into(),
        }];
        assert!(matches_query(&act, tr!("exchange"), &book));
        assert!(!matches_query(&act, tr!("exchange"), &[]));
        assert!(matches_query(&act, "536e90c0", &[]));
        assert!(!matches_query(&act, "deadbeef", &[]));
        assert!(!matches_query(&act, "@bob", &[]));
        assert!(matches_query(&sent("@bob", -1), "@bob", &[]));
    }

    /// An address finds the payments it is in: our own address of a payment in, the address
    /// behind a contact's @name, a payment out whose recipient has no name.
    #[test]
    fn finds_payments_by_their_core_address() {
        let ours = "yMXp7gU4uQ4mVQ4V1V3H6cPCXxWfNrsW3d";
        let theirs = "yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2";
        // The screen lowercases what is typed.
        let q = |s: &str| s.to_lowercase();
        let mut received = sent("", 50_000_000);
        received.kind = ActivityKind::Received { from: None };
        received.address = Some(ours.into());
        assert!(matches_query(&received, &q(ours), &[]));
        assert!(
            matches_query(&received, &q("4uQ4mVQ4V1"), &[]),
            "a part of it"
        );
        assert!(!matches_query(&received, &q(theirs), &[]));
        let mut to_contact = sent("@bob", -1);
        to_contact.address = Some(theirs.into());
        assert!(matches_query(&to_contact, &q(theirs), &[]));
        let mut unnamed = sent("", -1);
        unnamed.address = Some(theirs.into());
        assert!(matches_query(&unnamed, &q(theirs), &[]));
        let mut with_txid = sent("", -1);
        with_txid.txid = Some("ab".repeat(32));
        assert!(matches_query(&with_txid, &"ab".repeat(32), &[]));
    }
}

#[cfg(test)]
mod card_tests {
    use super::*;
    use evolite_api::Proof;

    fn sent(to: &str, address: Option<&str>) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Sent { to: to.into() },
            chain: Chain::Core,
            at: 0,
            amount: -1,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: address.map(Into::into),
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            note: None,
        }
    }

    #[test]
    fn says_whom_with_the_address_and_the_contact() {
        // A contact: its new address, and its name under it.
        assert_eq!(
            counterparty(&sent("@bob", Some("yLcBz"))),
            Some((
                tr!("To"),
                "yLcBz".to_string(),
                Some("@bob · a new address from the contact's key".to_string())
            ))
        );
        // An address alone, or a name with no address known.
        assert_eq!(
            counterparty(&sent("yLcBz", Some("yLcBz"))),
            Some((tr!("To"), "yLcBz".to_string(), None))
        );
        assert_eq!(
            counterparty(&sent("@bob", None)),
            Some((tr!("To"), "@bob".to_string(), None))
        );
        assert_eq!(counterparty(&sent("", None)), None);
    }
}

#[cfg(test)]
mod again_tests {
    use super::*;
    use evolite_api::Proof;

    fn out(to: &str, address: Option<&str>, amount: Duffs, fee: Option<Duffs>) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Sent { to: to.into() },
            chain: Chain::Core,
            at: 0,
            amount,
            proof: Proof::ChainLock,
            txid: None,
            fee,
            block: None,
            address: address.map(Into::into),
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            note: None,
        }
    }

    #[test]
    fn pays_the_same_recipient_the_same_amount_again() {
        // To an address: the address, the amount without the fee.
        assert_eq!(
            again(&out("yXk3", Some("yXk3"), -100_000_226, Some(226))),
            Some(("yXk3".to_string(), 100_000_000))
        );
        // To a contact: by its name, so it goes to the next address of its key.
        assert_eq!(
            again(&out("@bob", Some("yLc"), -10_226, Some(226))),
            Some(("@bob".to_string(), 10_000))
        );
        // A contact without a name, or nothing that went out: nothing to repeat.
        assert_eq!(again(&out("6G7V…Qck", None, -10_000, None)), None);
        let mut received = out("yX", None, 5, None);
        received.kind = ActivityKind::Received { from: None };
        assert_eq!(again(&received), None);
    }

    #[test]
    fn offers_saved_addresses_while_typing() {
        let book = [
            SavedAddress {
                address: "yAAA".into(),
                label: "Rent".into(),
            },
            SavedAddress {
                address: "yBBB".into(),
                label: "Exchange".into(),
            },
        ];
        // `book_matches` lives with the Send screen.
        assert_eq!(
            crate::text::book_label(&out("yBBB", None, -1, None), &book),
            Some("Exchange")
        );
        assert_eq!(
            crate::text::activity_title_in(&out("yBBB", Some("yBBB"), -1, None), &book),
            "Sent · Exchange"
        );
    }
}
