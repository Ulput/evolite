//! 05 · History: every operation, filtered and searched, grouped by day.

use super::activity_filters::{self, Filters, History, Period, Status};
use super::txt;
use crate::components::{
    Assets, ButtonKind, TxRow, button, button_enabled, button_row, checkbox, notice, page_header,
    pill, section_label, tx_row_clickable,
};
use crate::context::Ctx;
use crate::csv_export;
use crate::text::{self};
use crate::theme::{Weight, mono, sans};
use chrono::{DateTime, Local};
use egui::{Align, Layout, Margin, Stroke, Ui};
use evolite_api::{Activity, Flow};

// The operation's card lives apart (FEAT-0005); History and the app reach it from here.
#[cfg(all(test, feature = "dev-tools"))]
pub use super::activity_card::note_edit;
pub use super::activity_card::{card_window, forget_note, on_note_done, open_card};
// The presets' notes (`app/dev.rs`).
#[cfg(feature = "dev-tools")]
pub use super::activity_card::{edit_note, note_failed_example};

/// Where the last export's answer is kept: the file written (and how many lines), or why not.
fn saved_id() -> egui::Id {
    egui::Id::new("csv-export")
}

/// Whether the last export left operations out (BUG-100): beside [`saved_id`], so its answer stays
/// one value.
fn left_out_id() -> egui::Id {
    saved_id().with("left-out")
}

/// The last export's answer, when another wallet opens: its file is not this one's (BUG-45).
pub fn forget(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<Result<(std::path::PathBuf, usize), String>>(saved_id());
        d.remove::<bool>(left_out_id());
    });
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
        // The number the file will hold, not the number of rows on screen (BUG-100).
        let own = csv_export::Own::of(wallet);
        let lines = (
            csv_export::lines(shown.iter().map(|i| &wallet.activity[*i]), &own),
            csv_export::lines(&wallet.activity, &own),
        );
        match export_question(ui, a, only_shown, (shown.len(), total), lines, &h.filters) {
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
        let left_out = count < lines.len();
        ui.data_mut(|d| d.insert_temp(left_out_id(), left_out));
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
            if ui.data(|d| d.get_temp::<bool>(left_out_id()).unwrap_or(false)) {
                ui.add(egui::Label::new(txt(LEFT_OUT, 12.0, Weight::Regular, p.muted)).wrap());
            }
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
                    if crate::text::coins_known(cx.snap.sync.stage) && wallet.balances.credits_read
                    {
                        tr!("No operations yet. Receive DASH on the “Receive” screen.")
                    } else {
                        tr!("Still reading the operations — they appear here as soon as they are found")
                    }
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

/// In the export question and under the export's answer, when the file holds fewer lines than
/// there are operations (BUG-100).
pub const LEFT_OUT: &str = tr!(
    "Not every operation becomes a line of the file: those not final yet, the second half of a transfer between your own balances and those with no amount or an unknown fee are left out."
);

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
    (shown_lines, total_lines): (usize, usize),
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
                crate::format::count(shown_lines as u64)
            );
            if crate::components::option_card(ui, a, "check", &only, &what, only_shown).clicked() {
                only_shown = true;
            }
            let all = format!(
                tr!("The whole history · {}"),
                crate::format::count(total_lines as u64)
            );
            if crate::components::option_card(ui, a, "activity", &all, "", !only_shown).clicked() {
                only_shown = false;
            }
            // Rows and lines differ for the choice made: why, in plain words (BUG-100).
            let (rows, lines) = if only_shown {
                (shown, shown_lines)
            } else {
                (total, total_lines)
            };
            if lines != rows {
                ui.add(egui::Label::new(txt(LEFT_OUT, 12.0, Weight::Regular, p.muted)).wrap());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::activity_filters::matches_query;
    use evolite_api::Proof;
    use evolite_api::{ActivityKind, Chain, SavedAddress};

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
            detail: None,
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
