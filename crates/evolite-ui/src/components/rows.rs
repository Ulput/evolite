//! Rows and tiles: sidebar item, transaction row, balance card, icon box.

use super::prelude::*;
use evolite_api::Proof;

/// A menu item's height; the sidebar gives 36 in a low window (FEAT-0036).
pub const NAV_ITEM: f32 = 40.0;

/// nav/default · nav/active: `height` px ([`NAV_ITEM`]), radius 10, icon 20 + label 14. In the
/// rail only the icon.
pub fn nav_item(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    label: &str,
    active: bool,
    rail: bool,
    height: f32,
) -> Response {
    let p = a.p;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    named_choice(&resp, egui::WidgetType::SelectableLabel, active, label);
    let hovered = resp.hovered();
    if active || hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), p.raised);
    }
    let color = if active || hovered { p.text } else { p.muted };
    let icon_x = if rail {
        rect.center().x - 10.0
    } else {
        rect.left() + 12.0
    };
    let icon_rect = Rect::from_min_size(pos2(icon_x, rect.center().y - 10.0), vec2(20.0, 20.0));
    a.icons.icon(icon, color, &p, 20.0).paint_at(ui, icon_rect);
    if !rail {
        let font = sans(
            14.0,
            if active {
                Weight::SemiBold
            } else {
                Weight::Medium
            },
        );
        paint_text(
            ui,
            pos2(icon_rect.right() + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            font,
            color,
        );
    }
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if rail {
        resp.on_hover_text(crate::i18n::t(label).into_owned())
    } else {
        resp
    }
}

/// One operation, already in words. `proof` is `None` where a status means nothing (the
/// owner's own pool operations).
pub struct TxRow<'a> {
    pub icon: &'a str,
    pub title: &'a str,
    pub detail: &'a str,
    pub proof: Option<Proof>,
    pub amount: &'a str,
    /// What went wrong, in red after `detail` (BUG-57: credits not added); `None` mostly.
    pub alert: Option<&'a str>,
    /// `alert` is only said, not something to act on: muted (FEAT-0016, BUG-57).
    pub quiet: bool,
    /// The chip names the Core transaction: the credits are not known to have arrived.
    pub core_chip: bool,
    /// The person's note to the operation (FEAT-0003), quoted after the line and cut with «…»
    /// before the status; its own direction kept.
    pub note: Option<&'a str>,
    /// The amount in the currency of «Fiat rate», grey under it (FEAT-0008, frame 6).
    pub fiat: Option<&'a str>,
}

/// row/transaction: 54 px; icon box 36, title and line under it, status chip, amount in mono.
pub fn tx_row(ui: &mut Ui, a: &mut Assets, row: &TxRow) {
    let _ = tx_row_sensed(ui, a, row, Sense::hover());
}

/// [`tx_row`] that can be clicked (it opens the operation's card).
pub fn tx_row_clickable(ui: &mut Ui, a: &mut Assets, row: &TxRow) -> egui::Response {
    tx_row_sensed(ui, a, row, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn tx_row_sensed(ui: &mut Ui, a: &mut Assets, row: &TxRow, sense: Sense) -> egui::Response {
    let p = a.p;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), sense);
    // "Hide amounts": "••••" under the pointer too (FEAT-0007: no reveal on hover), and for a
    // screen reader, which reads the row's name.
    let hidden = crate::format::display().hide;
    let masked = hidden.then(|| crate::format::mask(row.amount));
    let amount = masked.as_deref().unwrap_or(row.amount);
    let kind = if sense.senses_click() {
        egui::WidgetType::Button
    } else {
        egui::WidgetType::Label
    };
    let mut name = match row.fiat {
        Some(fiat) => format!("{} · {} · {} · {}", row.title, amount, fiat, row.detail),
        None => format!("{} · {} · {}", row.title, amount, row.detail),
    };
    if let Some(alert) = row.alert {
        name = format!("{name} · {alert}");
    }
    match row.note {
        // The note in its quotes is read apart: a « · » of the person's own would split it from
        // its quotes in the line's pieces (FEAT-0003).
        Some(note) => {
            let name = format!(
                "{} · {}",
                crate::i18n::spoken(&name),
                crate::i18n::spoken(&quoted_note(note))
            );
            resp.widget_info(|| egui::WidgetInfo::labeled(kind, true, &name));
        }
        None => named(&resp, kind, true, &name),
    }
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(12), p.surface);
    }
    let cy = rect.center().y;
    let boxr = Rect::from_min_size(pos2(rect.left() + 16.0, cy - 18.0), vec2(36.0, 36.0));
    paint_icon_box(ui, a, boxr, 10, row.icon, 18.0, p.ink);

    let amount_right = rect.right() - 16.0;
    // With the currency under it, the amount goes up as the title does.
    let amount_y = if row.fiat.is_some() { cy - 8.0 } else { cy };
    paint_text(
        ui,
        pos2(amount_right, amount_y),
        Align2::RIGHT_CENTER,
        amount,
        mono(14.0, Weight::Medium),
        p.text,
    );
    if let Some(fiat) = row.fiat {
        paint_text(
            ui,
            pos2(amount_right, cy + 10.0),
            Align2::RIGHT_CENTER,
            fiat,
            mono(12.0, Weight::Regular),
            p.muted,
        );
    }

    // The chip sits left of a 150 px amount column, as in Figma.
    let mut text_right = amount_right - 150.0;
    if let Some(proof) = row.proof {
        let label = status_words(proof, row.core_chip);
        let chip_w = 8.0 + 14.0 + 6.0 + text_width(ui, &label, sans(12.0, Weight::SemiBold)) + 10.0;
        let chip_rect = Rect::from_min_size(
            pos2(amount_right - 150.0 - 14.0 - chip_w, cy - 12.5),
            vec2(chip_w, 25.0),
        );
        let mut chip_ui = ui.new_child(egui::UiBuilder::new().max_rect(chip_rect));
        status_chip_of(&mut chip_ui, a, proof, row.core_chip);
        text_right = chip_rect.left();
    }

    let tx = boxr.right() + 14.0;
    let title_w = text_right - 14.0 - tx;
    // In Arabic broken in reading order first (BUG-119), measured with its mono codes.
    let title_text = lines_measured(row.title, &sans(14.0, Weight::SemiBold), title_w, |line| {
        ui.painter()
            .layout_job(title_job(line, p.text, f32::INFINITY))
            .size()
            .x
    });
    let title = ui
        .painter()
        .layout_job(title_job(&title_text, p.text, title_w));
    ui.painter().galley(pos2(tx, cy - 17.0), title, p.text);
    let line = paint_text(
        ui,
        pos2(tx, cy + 9.0),
        Align2::LEFT_CENTER,
        row.detail,
        sans(12.0, Weight::Regular),
        p.muted,
    );
    let mut line_end = line.right();
    if let Some(alert) = row.alert {
        let dot = paint_text(
            ui,
            pos2(line.right(), cy + 9.0),
            Align2::LEFT_CENTER,
            " · ",
            sans(12.0, Weight::Regular),
            p.muted,
        );
        line_end = paint_text(
            ui,
            pos2(dot.right(), cy + 9.0),
            Align2::LEFT_CENTER,
            alert,
            sans(12.0, Weight::Medium),
            // A state to finish, not an error (calm states CS-1); an answer is only said.
            if row.quiet {
                p.muted
            } else {
                attention_color(&p)
            },
        )
        .right();
    }
    if let Some(note) = row.note {
        let dot = paint_text(
            ui,
            pos2(line_end, cy + 9.0),
            Align2::LEFT_CENTER,
            " · ",
            sans(12.0, Weight::Regular),
            p.muted,
        );
        let room = text_right - 14.0 - dot.right();
        if room > 0.0 {
            let galley = note_galley(ui, note, room, p.muted);
            ui.painter().galley(
                pos2(dot.right(), cy + 9.0 - galley.size().y / 2.0),
                galley,
                p.muted,
            );
        }
    }
    resp
}

/// A note in its quotes (N14: the language's own), the person's words kept as they are.
pub fn quoted_note(note: &str) -> String {
    format!(tr!("“{}”"), crate::i18n::own(note))
}

/// A note as a history line shows it: one row, cut with «…» to `width`.
pub(crate) fn note_galley(
    ui: &Ui,
    note: &str,
    width: f32,
    color: Color32,
) -> std::sync::Arc<egui::Galley> {
    // Arabic is cut by words from its end in reading order (BUG-118): egui's own cut keeps the
    // left of the row, the end of an Arabic phrase. A word too long still meets egui's cut.
    let shown = cut_to(
        ui.ctx(),
        &quoted_note(note),
        &sans(12.0, Weight::Regular),
        width,
        1,
    );
    ui.painter().layout_job(note_job(shown, width, color))
}

/// A note laid out in one row, cut with «…» to `width`. egui keeps the laid-out text from frame
/// to frame, so a long note is not measured anew each frame (AC-T9).
fn note_job(shown: String, width: f32, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::single_section(
        shown,
        egui::TextFormat::simple(sans(12.0, Weight::Regular), color),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: width,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job
}

/// An operation's title as drawn: Inter, with the Dash addresses, short ids and txids in it
/// in mono ("amounts, addresses, txid and hashes — monospaced", design review summary №10).
fn title_job(title: &str, color: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    let words = egui::TextFormat::simple(sans(14.0, Weight::SemiBold), color);
    let code = egui::TextFormat::simple(mono(14.0, Weight::Medium), color);
    for (part, is_code) in code_spans(title) {
        let format = if is_code { &code } else { &words };
        job.append(part, 0.0, format.clone());
    }
    job
}

/// `text` cut into runs of words and of codes (see [`looks_like_code`]), in order; together
/// they are `text` again.
fn code_spans(text: &str) -> Vec<(&str, bool)> {
    let mut out: Vec<(&str, bool)> = Vec::new();
    // Where the words not yet pushed start.
    let mut start = 0;
    // Words are cut at spaces; a code leaves its punctuation outside ("yXk3…Mw2a," → code, ",").
    let mut i = 0;
    loop {
        let end = text[i..].find(' ').map_or(text.len(), |n| i + n);
        let word = &text[i..end];
        let lead = word.len() - word.trim_start_matches(['(', '«', '“', '"']).len();
        let core = word[lead..].trim_end_matches([',', '.', ':', ';', ')', '»', '”', '"']);
        if looks_like_code(core) {
            let at = i + lead;
            if at > start {
                out.push((&text[start..at], false));
            }
            out.push((core, true));
            start = at + core.len();
        }
        if end >= text.len() {
            break;
        }
        i = end + 1;
    }
    if start < text.len() {
        out.push((&text[start..], false));
    }
    out
}

/// A Dash address or txid (26+ letters and digits), or one shortened with "…" ("yXk3…Mw2a",
/// an identity's "6G7V…Qck"); not an @name or a word.
fn looks_like_code(word: &str) -> bool {
    let alnum = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
    match word.split_once('…') {
        Some((head, tail)) => alnum(head) && alnum(tail),
        None => word.len() >= 26 && alnum(word) && word.chars().any(|c| c.is_ascii_digit()),
    }
}

/// An amount as a label. With "Hide amounts" it reads "•••• DASH", under the pointer too:
/// only the eye ([`amount_toggle`]) or Settings show it again (FEAT-0007).
pub fn amount_label(ui: &mut Ui, shown: &str, font: FontId, color: Color32) -> Response {
    let text = if crate::format::display().hide {
        crate::format::mask(shown)
    } else {
        shown.to_owned()
    };
    ui.label(
        egui::RichText::new(crate::i18n::t(&text))
            .font(font)
            .color(color),
    )
}

/// A balance card's amount and how sure it is (BUG-77, the designer's improvement 1).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CardAmount {
    /// Known now: in the text colour.
    Now(i64),
    /// The last known one (the detail says when): muted.
    Earlier(i64),
    /// Not known yet: «—», muted (the detail says so in words).
    Unknown,
}

impl CardAmount {
    /// `duffs` — Core's balance or a sum that counts it — as sure as Core's balance is
    /// ([`evolite_api::Balances::core_as_of`]). Every place that shows Core goes through this
    /// (BUG-80): a 0 before the first check is not an amount.
    pub fn as_core(b: &evolite_api::Balances, duffs: i64) -> Self {
        match b.core_as_of {
            None => Self::Now(duffs),
            Some(evolite_api::CoreAsOf::Checked(_)) => Self::Earlier(duffs),
            Some(evolite_api::CoreAsOf::NotYet) => Self::Unknown,
        }
    }

    /// The Core balance as sure as it is.
    pub fn core(b: &evolite_api::Balances) -> Self {
        Self::as_core(b, b.core)
    }

    /// Everything the wallet holds: as sure as Core, which it counts.
    pub fn total(b: &evolite_api::Balances) -> Self {
        if !b.platform_known() {
            return Self::Unknown;
        }
        Self::as_core(b, b.total())
    }

    /// An amount on Platform: `duffs` while `read`, else «—», never a 0 (BUG-91).
    pub fn platform(read: bool, duffs: i64) -> Self {
        if read {
            Self::Now(duffs)
        } else {
            Self::Unknown
        }
    }

    /// The amount, if there is one to compare with: none while not known.
    pub fn known(self) -> Option<i64> {
        match self {
            Self::Now(duffs) | Self::Earlier(duffs) => Some(duffs),
            Self::Unknown => None,
        }
    }

    /// The amount with its unit (hidden by «Hide amounts»), «—» while not known.
    pub fn balance(self) -> String {
        self.known()
            .map_or_else(|| "—".to_owned(), crate::format::balance)
    }

    /// The text colour when checked now, muted otherwise.
    pub fn color(self, p: &Palette) -> Color32 {
        match self {
            Self::Now(_) => p.text,
            Self::Earlier(_) | Self::Unknown => p.muted,
        }
    }

    /// A tile's amount without its unit, «—» while not known: what the tile shows before «Hide
    /// amounts» masks it.
    pub fn shown(self) -> String {
        self.known()
            .map_or_else(|| "—".to_owned(), |d| crate::format::number(d, false))
    }
}

/// card/balance: icon 18 + title 13 Medium, amount in `font` (24 mono, or 20: one size for the
/// whole row, [`tile_amount_font`]), detail 12. A cell of [`grid`]: one height with the others
/// of its row.
pub fn balance_card_in(
    ui: &mut Ui,
    a: &mut Assets,
    cell: &mut RowCell,
    (icon, title): (&str, &str),
    (amount, font): (CardAmount, FontId),
    detail: &str,
    width: f32,
) {
    balance_card_at_most(
        ui,
        a,
        cell,
        (icon, title),
        (amount, font, None),
        detail,
        width,
    );
}

/// [`balance_card_in`] with an action under its line (FEAT-0015 stage 2, frame 21: «Top up from
/// Core ›», 12 SemiBold, the accent); dim and inactive when `enabled` is false (24). Whether it
/// was clicked.
#[allow(clippy::too_many_arguments)]
pub fn balance_card_action(
    ui: &mut Ui,
    a: &mut Assets,
    cell: &mut RowCell,
    (icon, title): (&str, &str),
    (amount, font): (CardAmount, FontId),
    detail: &str,
    width: f32,
    (action, enabled): (&str, bool),
) -> bool {
    let p = a.p;
    let frame = card(
        &p,
        Margin {
            left: 20,
            right: 20,
            top: 18,
            bottom: 18,
        },
    );
    let mut clicked = false;
    cell.card(ui, frame, |ui| {
        ui.set_width(width - TILE_INSIDE);
        ui.vertical(|ui| {
            balance_card_body(
                ui,
                a,
                icon,
                title,
                (amount, font, None),
                detail,
                width - TILE_INSIDE,
            );
            let color = if enabled {
                p.accent
            } else {
                p.accent.gamma_multiply(0.45)
            };
            let resp = ui.add_enabled(
                enabled,
                egui::Button::new(
                    egui::RichText::new(crate::i18n::t(action))
                        .font(sans(12.0, Weight::SemiBold))
                        .color(color),
                )
                .frame(false),
            );
            named(&resp, egui::WidgetType::Button, enabled, action);
            clicked = resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();
        });
    });
    clicked
}

/// [`balance_card_in`] whose amount shows at most `decimals` decimals, «≈» and the full amount
/// in its tooltip when it has more (R4; the designer, FEAT-0030: denominations give long
/// fractions).
pub fn balance_card_at_most(
    ui: &mut Ui,
    a: &mut Assets,
    cell: &mut RowCell,
    (icon, title): (&str, &str),
    (amount, font, decimals): (CardAmount, FontId, Option<usize>),
    detail: &str,
    width: f32,
) {
    let p = a.p;
    let frame = card(
        &p,
        Margin {
            left: 20,
            right: 20,
            top: 18,
            bottom: 18,
        },
    );
    cell.card(ui, frame, |ui| {
        ui.set_width(width - TILE_INSIDE);
        ui.vertical(|ui| {
            balance_card_body(
                ui,
                a,
                icon,
                title,
                (amount, font, decimals),
                detail,
                width - TILE_INSIDE,
            )
        });
    });
}

/// A tile's margins (20 each side) and its border: what its width leaves for the amount.
const TILE_INSIDE: f32 = 42.0;

/// The amounts of one row of tiles, each `width` wide ([`balance_card_in`]): 24 when every one
/// of them fits at 24, else one step smaller (20) for all of them. An amount is never cut nor
/// wrapped, and the amounts of a row are never of two sizes (responsive spec, «Длинный текст»;
/// the designer, 2026-11-07). Measured as shown, not as the mask of «Hide amounts»: hiding them
/// does not change the size.
pub fn tile_amount_font(ui: &Ui, row: &[CardAmount], width: f32) -> FontId {
    let big = mono(24.0, Weight::Medium);
    let room = width - TILE_INSIDE;
    let fits = row
        .iter()
        .all(|a| super::text_width(ui, &a.shown(), big.clone()) <= room);
    if fits {
        big
    } else {
        mono(20.0, Weight::Medium)
    }
}

fn balance_card_body(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    title: &str,
    (amount, font, decimals): (CardAmount, FontId, Option<usize>),
    detail: &str,
    room: f32,
) {
    let p = a.p;
    {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add(a.icons.icon(icon, p.muted, &p, 18.0));
            ui.label(
                egui::RichText::new(crate::i18n::t(title))
                    .font(sans(13.0, Weight::Medium))
                    .color(p.muted),
            );
        });
        let fitted = fit_tile_amount(amount, |text| {
            super::text_width(ui, text, font.clone()) <= room
                && decimals.is_none_or(|most| decimals_in(text) <= most)
        });
        tile_amount_label(ui, amount, &fitted, font, amount.color(&p));
        ui.label(
            egui::RichText::new(crate::i18n::t(detail))
                .font(sans(12.0, Weight::Regular))
                .color(p.muted),
        );
    }
}

/// The decimals `text`, a number as the interface writes it, shows.
fn decimals_in(text: &str) -> usize {
    let (_, mark) = crate::i18n::separators();
    text.rsplit_once(mark).map_or(0, |(_, after)| {
        after.chars().filter(char::is_ascii_digit).count()
    })
}

/// What a tile shows of its amount ([`fit_tile_amount`]).
#[derive(Clone, PartialEq, Debug)]
pub struct TileText {
    /// The number as drawn (no unit: the tile's detail line names it).
    pub shown: String,
    /// Fewer decimals than the amount has, rounded («≈ 1 234,57» when no longer exact).
    pub shortened: bool,
}

/// Fewest decimals a shortened tile amount keeps (the designer, 2026-11-07).
pub const TILE_MIN_DECIMALS: usize = 2;

/// A tile's amount as it fits (R4a; the designer, 2026-11-07, responsive spec): the whole of it
/// when it `fits`; else as many decimals as fit, never fewer than [`TILE_MIN_DECIMALS`], rounded
/// half up, with «≈» in front while that is not the exact amount. Never wrapped nor cut at the
/// end: with two decimals it is shown even when it still does not fit. «—» stays «—».
pub fn fit_tile_amount(amount: CardAmount, fits: impl Fn(&str) -> bool) -> TileText {
    let whole = amount.shown();
    let Some(duffs) = amount.known() else {
        return TileText {
            shown: whole,
            shortened: false,
        };
    };
    if fits(&whole) {
        return TileText {
            shown: whole,
            shortened: false,
        };
    }
    let written = |decimals: usize| {
        let (number, exact) = crate::format::rounded_number(duffs, decimals);
        if exact {
            number
        } else {
            format!("≈ {number}")
        }
    };
    // From the most decimals any unit has (8, DASH) down: those a unit has not give the whole
    // amount again.
    let shown = (TILE_MIN_DECIMALS..8)
        .rev()
        .map(written)
        .find(|text| *text != whole && fits(text))
        .unwrap_or_else(|| written(TILE_MIN_DECIMALS));
    let shortened = shown != whole;
    TileText { shown, shortened }
}

/// An amount alone on a card's width, on one line with its unit (R4b, the rule of R4a): as many
/// decimals as fit in `room`, the same tooltip and description as a tile's when shortened.
pub fn card_amount_fitted(
    ui: &mut Ui,
    amount: CardAmount,
    font: FontId,
    color: Color32,
    room: f32,
) {
    let unit = crate::format::unit_name();
    let with_unit = |number: &str| format!("{number} {unit}");
    let mut fitted = fit_tile_amount(amount, |text| {
        super::text_width(ui, &with_unit(text), font.clone()) <= room
    });
    fitted.shown = with_unit(&fitted.shown);
    tile_amount_label(ui, amount, &fitted, font, color);
}

/// A tile's amount on one line. Shortened, it says the full amount with its unit on hover, on
/// Tab focus (it takes the focus for that) and to a screen reader as its description. With
/// «Hide amounts» it is «••••» as everywhere and says nothing more.
fn tile_amount_label(
    ui: &mut Ui,
    amount: CardAmount,
    fitted: &TileText,
    font: FontId,
    color: Color32,
) {
    let hidden = crate::format::display().hide;
    let text = if hidden {
        crate::format::mask(&fitted.shown)
    } else {
        fitted.shown.clone()
    };
    let label = egui::Label::new(
        egui::RichText::new(crate::i18n::t(&text))
            .font(font)
            .color(color),
    )
    .wrap_mode(egui::TextWrapMode::Extend);
    let (Some(duffs), true, false) = (amount.known(), fitted.shortened, hidden) else {
        ui.add(label);
        return;
    };
    let resp = ui.add(label.sense(Sense::FOCUSABLE));
    let full = crate::format::amount(duffs);
    // A Tab stop has a name: the amount as shown (a label's words are only its value for egui).
    let name = crate::i18n::spoken(&text);
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.set_label(name.clone());
        node.set_description(full.clone());
    });
    if resp.has_focus() && !resp.hovered() {
        resp.show_tooltip_text(full.clone());
    }
    resp.on_hover_text(full);
}

/// A raised rounded square with an icon in the middle (transaction rows, actions, dialogs).
pub fn paint_icon_box(
    ui: &Ui,
    a: &mut Assets,
    rect: Rect,
    radius: u8,
    icon: &str,
    icon_size: f32,
    color: Color32,
) {
    let p = a.p;
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius), p.raised);
    a.icons.icon(icon, color, &p, icon_size).paint_at(
        ui,
        Rect::from_center_size(rect.center(), vec2(icon_size, icon_size)),
    );
}

/// A wallet's circle (Figma 26 "wallet dot"): its colour and its name's first letter.
pub fn paint_wallet_dot(ui: &Ui, rect: Rect, wallet: &evolite_api::WalletEntry) {
    let color =
        crate::theme::WALLET_COLORS[wallet.color as usize % crate::theme::WALLET_COLORS.len()];
    ui.painter()
        .circle_filled(rect.center(), rect.width() / 2.0, color);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        wallet.letter(),
        sans((rect.width() * 0.46).round(), Weight::SemiBold),
        crate::theme::WALLET_LETTER,
    );
}

/// An app's logo until apps send theirs: its first letter on a raised square with a line.
pub fn paint_letter_box(ui: &Ui, p: &Palette, rect: Rect, name: &str) {
    let radius = (rect.width() * 0.28) as u8;
    ui.painter().rect(
        rect,
        CornerRadius::same(radius),
        p.raised,
        Stroke::new(1.0, p.line),
        StrokeKind::Inside,
    );
    let letter: String = name.chars().next().map(String::from).unwrap_or_default();
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        crate::i18n::t(&letter),
        sans((rect.width() * 0.41).round(), Weight::Bold),
        p.text,
    );
}

#[cfg(test)]
mod tests {
    use super::{CardAmount, TileText, code_spans, fit_tile_amount, looks_like_code};
    use crate::format::{Display, NumberStyle, Unit, set_display};

    fn in_dash() {
        set_display(Display {
            unit: Unit::Dash,
            style: NumberStyle::SpaceComma,
            ..Display::default()
        });
    }

    /// `n` characters of mono fit: what a tile of that width holds.
    fn up_to(n: usize) -> impl Fn(&str) -> bool {
        move |text: &str| text.chars().count() <= n
    }

    fn fitted(shown: &str, shortened: bool) -> TileText {
        TileText {
            shown: shown.into(),
            shortened,
        }
    }

    /// R4a (the designer, 2026-11-07): an amount too wide for its tile keeps as many decimals as
    /// fit, rounded, «≈» in front; never fewer than two, even when two do not fit either.
    #[test]
    fn a_wide_tile_amount_keeps_the_decimals_that_fit() {
        in_dash();
        let amount = CardAmount::Now(123_456_789_012);
        assert_eq!(
            fit_tile_amount(amount, up_to(14)),
            fitted("1\u{a0}234,56789012", false)
        );
        assert_eq!(
            fit_tile_amount(amount, up_to(13)),
            fitted("≈ 1\u{a0}234,56789", true)
        );
        // Never fewer than two decimals, though one would fit.
        assert_eq!(
            fit_tile_amount(amount, up_to(9)),
            fitted("≈ 1\u{a0}234,57", true)
        );
        assert_eq!(
            fit_tile_amount(amount, up_to(10)),
            fitted("≈ 1\u{a0}234,57", true)
        );
        assert_eq!(
            fit_tile_amount(amount, up_to(4)),
            fitted("≈ 1\u{a0}234,57", true)
        );
        // Muted, the last known amount is shortened the same way.
        assert_eq!(
            fit_tile_amount(CardAmount::Earlier(123_456_789_012), up_to(10)),
            fitted("≈ 1\u{a0}234,57", true)
        );
    }

    /// Fewer decimals that are still the exact amount go without «≈»; «—» is never shortened.
    #[test]
    fn an_exact_or_unknown_tile_amount_has_no_approximately() {
        in_dash();
        assert_eq!(
            fit_tile_amount(CardAmount::Now(123_450_000_000), up_to(8)),
            fitted("1\u{a0}234,50", true)
        );
        assert_eq!(
            fit_tile_amount(CardAmount::Unknown, up_to(0)),
            fitted("—", false)
        );
    }

    /// Summary №10: in a row's title the address (whole or short), the short id and the txid are
    /// set apart for mono; the words and @names are not.
    #[test]
    fn addresses_in_a_title_are_set_apart_for_mono() {
        assert!(looks_like_code("yXk3…Mw2a"));
        assert!(looks_like_code("6G7V…Qck"));
        assert!(looks_like_code("yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2"));
        assert!(!looks_like_code("@bob"));
        assert!(!looks_like_code("Transfer"));
        assert!(!looks_like_code("…"));
        assert_eq!(
            code_spans("Sent yXk3…Mw2a"),
            vec![("Sent ", false), ("yXk3…Mw2a", true)]
        );
        assert_eq!(
            code_spans("Перевод кредитов (yXk3…Mw2a), готово"),
            vec![
                ("Перевод кредитов (", false),
                ("yXk3…Mw2a", true),
                ("), готово", false)
            ]
        );
        assert_eq!(
            code_spans("Received from @bob"),
            vec![("Received from @bob", false)]
        );
        assert_eq!(code_spans("yXk3…Mw2a"), vec![("yXk3…Mw2a", true)]);
        assert_eq!(code_spans(""), Vec::<(&str, bool)>::new());
    }
}
