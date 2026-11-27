//! FEAT-0057 П1: «Work as» as a button and a list with search, from five identities on — on
//! Identity, Tokens, the Workshop and the Collection's filter (one picker, BUG-265; BUG-310 is
//! closed by it). Up to four, the chips of [`super::identity_picker`] as before (R-1.1).
//!
//! The button: a 34 px pill as the chips — the shown identity's avatar 24, its name shortened in
//! the middle (FEAT-0051 R-5), «▾»; «{n} identities» muted on its right (R-1.7). The list: the
//! wallet menu's style, 440 wide and never wider than the window: «Name or id» focused, «WALLET
//! IDENTITIES · {n}», rows of 48 — the name, then the credits in mono and the state in a word
//! (R-1.2, R-1.3) — the chosen one raised with an amber ✓, «{n} more · scroll or search» while
//! rows are out of sight (R-1.6), and «+ New identity» on Identity. Enter or Space opens it,
//! the arrows go along the rows, Enter chooses, Esc closes and gives the focus back to the
//! button (UC-1.4).

use super::prelude::*;
use evolite_api::{IdentityChoice, IdentitySearch, Wallet};

/// R-1.1 (the customer, З-1): the list from five identities on; up to four, the chips.
pub const LIST_FROM: usize = 5;

/// The list's width (the designer's frame), never wider than the window less 32.
const LIST_W: f32 = 440.0;

/// A row of the list.
const ROW_H: f32 = 48.0;

/// The pill's widest (the frame at 1024: the name shortened, «10 identities» beside it).
const BUTTON_MAX: f32 = 380.0;

/// The list's own state, kept by the button's id: open, what is searched, the row the arrows
/// are on.
#[derive(Clone, Default)]
struct ListState {
    open: bool,
    query: String,
    at: usize,
    /// When the list last asked for every identity's credits (`ctx` time, s): at its opening,
    /// then once a minute while it stays open (r1 Must).
    asked_at: Option<f64>,
    /// The search field was given the focus at this opening.
    focused: bool,
}

/// What the list was asked this frame — as [`super::Picked`].
#[derive(Default)]
pub(super) struct ListPicked {
    pub chosen: Option<String>,
    pub all: bool,
    pub new_one: bool,
}

/// One entry of the list: «All» (the Collection's filter) or an identity.
#[derive(Clone, Copy)]
enum Entry<'a> {
    All,
    One(&'a IdentityChoice),
}

/// How often an open list asks again for every identity's credits, seconds (А-1).
const ASK_EVERY: f64 = 60.0;

#[cfg(test)]
thread_local! {
    /// How many times this thread's lists asked for every identity's credits (r1 Must's test).
    pub(crate) static ASKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The id App reads to ask the core for every identity's credits (А-1): set when the list
/// opens and once a minute while it stays open; the core reads them at most once a minute.
pub fn read_all_asked_id() -> egui::Id {
    egui::Id::new("identity-list-read-all-credits")
}

/// R-1.2: the state of an identity in a word, by its keys — the same rule as BUG-286/298's.
/// The word and whether it is said in amber (a key the person can add).
pub fn state_words(choice: &IdentityChoice) -> (&'static str, bool) {
    match choice.ready {
        None => (tr!("checking…"), false),
        Some(r) if !r.ours => (tr!("no keys of this wallet"), false),
        Some(r) if !r.sign_in => (tr!("needs a sign-in key"), true),
        Some(r) if !r.dashpay => (tr!("needs a DashPay key"), true),
        Some(_) if choice.name.is_none() => (tr!("no name"), false),
        Some(_) => (tr!("ready"), false),
    }
}

/// R-1.3, A1.7: the credits as on Identity — «—» while not read, dots under «Hide amounts».
pub fn credits_words(choice: &IdentityChoice) -> String {
    choice
        .credits
        .map_or_else(|| "—".to_owned(), crate::format::balance)
}

/// R-1.5: whether `choice` is found by `query` — its name without «@» whatever the case, or the
/// start of its id.
pub fn found(choice: &IdentityChoice, query: &str) -> bool {
    let q = query.trim().trim_start_matches('@').to_lowercase();
    q.is_empty()
        || choice
            .name
            .as_ref()
            .is_some_and(|n| n.to_lowercase().contains(&q))
        || choice.id.to_lowercase().starts_with(&q)
}

/// The button and, while open, the list. `on`: the identity shown (or the Collection's one
/// filtered by); `all: Some(on)`: «All» first (the Collection); `new`: «+ New identity» at the
/// end (Identity); `aside`: muted words on the right after the count.
#[allow(clippy::too_many_arguments)]
pub(super) fn identity_list(
    ui: &mut Ui,
    a: &mut Assets,
    wallet: &Wallet,
    label: &str,
    on: Option<&str>,
    all: Option<bool>,
    new: Option<bool>,
    aside: Option<&str>,
) -> ListPicked {
    let p = a.p;
    let identities = &wallet.identities;
    let mut picked = ListPicked::default();
    let state_id = ui.id().with("identity-list");
    let mut state: ListState = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let shown = on.and_then(|id| identities.iter().find(|c| c.id == id));
    let count = crate::text::plural_in(
        identities.len() as u32,
        tr!("{} identity"),
        tr!("{} identities"),
    );
    let mut button_rect = Rect::NOTHING;
    let mut button_id = None;
    // BUG-340: the words on the right on the row only when they fit beside the smallest button;
    // else on a line of their own under it, wrapping — never over the count (800 × 440, a long
    // name in «Credits @…»).
    let mut aside_below = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let font = crate::theme::style(crate::theme::type_scale::UI.weight(Weight::Medium));
        let label_w = text_width(ui, label, font);
        ui.allocate_ui_with_layout(
            vec2(label_w, PILL_HEIGHT),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(text(
                    label,
                    crate::theme::type_scale::UI.weight(Weight::Medium),
                    p.muted,
                ))
            },
        );
        let count_w = text_width(ui, &count, sans(13.0, Weight::Regular)) + 8.0;
        let aside_w = aside.map_or(0.0, |s| {
            text_width(ui, s, sans(13.0, Weight::Regular)) + 16.0
        });
        let aside_w = if ui.available_width() - count_w - aside_w < 120.0 {
            aside_below = aside;
            0.0
        } else {
            aside_w
        };
        let room = (ui.available_width() - count_w - aside_w).clamp(120.0, BUTTON_MAX);
        let resp = list_button(
            ui,
            a,
            label,
            shown,
            all.is_some() && on.is_none(),
            room,
            state.open,
        );
        button_rect = resp.rect;
        button_id = Some(resp.id);
        let keys = resp.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space));
        if resp.clicked() || (keys && !state.open) {
            state.open = !state.open;
            state.query.clear();
            state.at = entries_of(identities, all, "")
                .iter()
                .position(|e| matches!(e, Entry::One(c) if Some(c.id.as_str()) == on))
                .unwrap_or(0);
            state.focused = false;
        }
        ui.label(txt(&count, 13.0, Weight::Regular, p.muted));
        if let Some(aside) = aside.filter(|_| aside_below.is_none()) {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(txt(aside, 13.0, Weight::Regular, p.muted))
            });
        }
    });
    if let Some(aside) = aside_below {
        ui.add(crate::components::para(
            aside,
            13.0,
            Weight::Regular,
            p.muted,
        ));
    }
    // А-1, r1 (Must): every identity's credits when the list opens, and once a minute while it
    // stays open — not every frame (the core reads them all at most once a minute anyway).
    let now = ui.input(|i| i.time);
    if !state.open {
        state.asked_at = None;
    } else if state.asked_at.is_none_or(|at| now - at >= ASK_EVERY) {
        state.asked_at = Some(now);
        ui.data_mut(|d| d.insert_temp(read_all_asked_id(), true));
        #[cfg(test)]
        ASKED.with(|n| n.set(n.get() + 1));
    }
    if state.open {
        let closed = list(
            ui,
            a,
            wallet,
            (on, all, new),
            button_rect,
            &mut state,
            &mut picked,
        );
        if closed {
            state.open = false;
            if let Some(id) = button_id {
                ui.memory_mut(|m| m.request_focus(id));
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(state_id, state));
    picked
}

/// The entries the list shows for `query`: «All» first where there is one (not searched for),
/// then the identities found, in their order.
fn entries_of<'a>(
    identities: &'a [IdentityChoice],
    all: Option<bool>,
    query: &str,
) -> Vec<Entry<'a>> {
    let mut out = Vec::new();
    if all.is_some() && query.trim().is_empty() {
        out.push(Entry::All);
    }
    out.extend(
        identities
            .iter()
            .filter(|c| found(c, query))
            .map(Entry::One),
    );
    out
}

/// The pill: the avatar, the name (or «All»), «▾»; named «{label}: …» for a screen reader
/// («Work as: @kestrel7»).
#[allow(clippy::too_many_arguments)]
fn list_button(
    ui: &mut Ui,
    a: &mut Assets,
    label: &str,
    shown: Option<&IdentityChoice>,
    all: bool,
    room: f32,
    open: bool,
) -> Response {
    let p = a.p;
    let font = sans(14.0, Weight::SemiBold);
    let full = match shown {
        Some(c) if !all => crate::i18n::own(&super::identity_full_name(c)),
        _ => crate::i18n::t(tr!("All")).into_owned(),
    };
    // 10 + avatar 24 + 8 + name + 8 + ▾ 16 + 12.
    let chrome = 10.0 + if shown.is_some() && !all { 32.0 } else { 0.0 } + 36.0;
    let fits = |s: &str| text_width(ui, s, font.clone()) + chrome <= room;
    let name = if fits(&full) {
        full.clone()
    } else {
        crate::text::middle_fit(&full, &fits, crate::text::NAME_ENDS)
    };
    let width = (text_width(ui, &name, font.clone()) + chrome).min(room);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, PILL_HEIGHT), Sense::click());
    named(
        &resp,
        egui::WidgetType::Button,
        true,
        &format!("{}: {full}", crate::i18n::t(label)),
    );
    let stroke = if open || resp.has_focus() {
        p.brand
    } else {
        p.line
    };
    let fill = if resp.hovered() { p.raised } else { p.surface };
    ui.painter().rect(
        rect,
        CornerRadius::same(17),
        fill,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + 10.0;
    if let (Some(c), false) = (shown, all) {
        let face = Rect::from_min_size(pos2(x, rect.center().y - 12.0), vec2(24.0, 24.0));
        super::avatar(ui, a, face, &c.id, &full, None, &mut |_| {});
        x = face.right() + 8.0;
    }
    paint_text(
        ui,
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        &name,
        font,
        p.text,
    );
    a.icons.icon("chevron-down", p.muted, &p, 16.0).paint_at(
        ui,
        Rect::from_center_size(pos2(rect.right() - 20.0, rect.center().y), vec2(16.0, 16.0)),
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(full)
}

/// The list under the button. True when it closes (a choice, Esc, a click elsewhere).
#[allow(clippy::too_many_arguments)]
fn list(
    ui: &mut Ui,
    a: &mut Assets,
    wallet: &Wallet,
    (on, all, new): (Option<&str>, Option<bool>, Option<bool>),
    button: Rect,
    state: &mut ListState,
    picked: &mut ListPicked,
) -> bool {
    let p = a.p;
    let screen = ui.ctx().content_rect();
    let width = LIST_W.min(screen.width() - 32.0);
    let left = button
        .left()
        .min(screen.right() - 16.0 - width)
        .max(screen.left() + 16.0);
    let top = button.bottom() + 6.0;
    let entries = entries_of(&wallet.identities, all, &state.query);
    // UC-1.4: the arrows along the rows, Enter chooses, Esc closes.
    let (down, up, enter, esc) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if !entries.is_empty() {
        if down {
            state.at = (state.at + 1).min(entries.len() - 1);
        }
        if up {
            state.at = state.at.saturating_sub(1);
        }
    }
    state.at = state.at.min(entries.len().saturating_sub(1));
    let mut choose: Option<Entry> = None;
    if enter
        && state.focused
        && let Some(entry) = entries.get(state.at)
    {
        choose = Some(*entry);
    }
    let mut closed = esc;
    let area = egui::Area::new(egui::Id::new("identity-list"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(left, top))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.line))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(egui::Margin::same(8))
                .shadow(egui::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha(if p.dark { 120 } else { 40 }),
                })
                .show(ui, |ui| {
                    let inner = width - 16.0;
                    ui.set_width(inner);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    // The search, focused at once (UC-1.2).
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut state.query)
                            .hint_text(crate::i18n::t(tr!("Name or id")).into_owned())
                            .desired_width(inner)
                            .margin(egui::Margin::symmetric(10, 8)),
                    );
                    if !state.focused {
                        field.request_focus();
                        state.focused = true;
                    }
                    if field.changed() {
                        state.at = 0;
                    }
                    ui.add_space(4.0);
                    let head = format!(
                        tr!("Wallet identities · {}"),
                        crate::format::count(wallet.identities.len() as u64)
                    );
                    section_label(ui, &p, &head);
                    if entries.is_empty() {
                        // R-1.5.
                        ui.add_space(6.0);
                        ui.add(para(
                            &format!(
                                tr!("No identities for “{}”"),
                                crate::i18n::own(state.query.trim())
                            ),
                            13.0,
                            Weight::Regular,
                            p.muted,
                        ));
                        ui.add_space(6.0);
                    }
                    // R-1.6: no lower than the window; the rows scroll under the search.
                    let room = (screen.bottom() - 16.0 - ui.cursor().top() - 96.0).max(ROW_H * 2.0);
                    let rows = egui::ScrollArea::vertical()
                        .max_height(room.min(ROW_H * 6.0))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (i, entry) in entries.iter().enumerate() {
                                let hit = row(ui, a, *entry, (on, i == state.at), inner);
                                if i == state.at && (down || up) {
                                    hit.scroll_to_me(None);
                                }
                                if hit.clicked() {
                                    choose = Some(*entry);
                                }
                            }
                        });
                    // R-1.6: how many rows are below what is seen.
                    let seen_to = rows.state.offset.y + rows.inner_rect.height();
                    let below = entries
                        .len()
                        .saturating_sub(((seen_to + 1.0) / (ROW_H + 2.0)).floor() as usize);
                    if below > 0 {
                        ui.label(txt(
                            &format!(
                                tr!("{} more · scroll or search"),
                                crate::format::count(below as u64)
                            ),
                            11.0,
                            Weight::Regular,
                            p.muted,
                        ));
                    }
                    // §8d: the search for identities still running.
                    if wallet.identity_search == IdentitySearch::Searching {
                        ui.label(txt(
                            tr!("Searching for your identities…"),
                            11.0,
                            Weight::Regular,
                            p.muted,
                        ));
                    }
                    if let Some(enabled) = new {
                        divider(ui, &p);
                        let resp = add_pill(
                            ui,
                            a,
                            tr!("New identity"),
                            tr!("Create a new identity"),
                            enabled,
                        );
                        if resp.clicked() {
                            picked.new_one = true;
                            closed = true;
                        }
                    }
                });
        });
    // A click outside the list and the button closes it.
    let outside = ui.input(|i| {
        i.pointer.any_click()
            && i.pointer
                .interact_pos()
                .is_some_and(|at| !area.response.rect.contains(at) && !button.contains(at))
    });
    match choose {
        Some(Entry::All) => {
            picked.all = all == Some(false);
            closed = true;
        }
        Some(Entry::One(c)) => {
            if Some(c.id.as_str()) != on || all == Some(true) {
                picked.chosen = Some(c.id.clone());
            }
            closed = true;
        }
        None => {}
    }
    closed || outside
}

/// One row: the avatar, the name (shortened in its middle), the credits and the state in a
/// word; the chosen one raised with an amber ✓; the one the arrows are on with a line.
fn row(
    ui: &mut Ui,
    a: &mut Assets,
    entry: Entry,
    (on, at): (Option<&str>, bool),
    width: f32,
) -> Response {
    let p = a.p;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
    let (full, chosen) = match entry {
        Entry::All => (crate::i18n::t(tr!("All")).into_owned(), on.is_none()),
        Entry::One(c) => (
            crate::i18n::own(&super::identity_full_name(c)),
            Some(c.id.as_str()) == on,
        ),
    };
    if chosen || resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), p.raised);
    }
    if at {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(10),
            Stroke::new(1.0, p.line),
            StrokeKind::Inside,
        );
    }
    let face = Rect::from_min_size(
        pos2(rect.left() + 10.0, rect.center().y - 12.0),
        vec2(24.0, 24.0),
    );
    let x = face.right() + 10.0;
    let room = rect.right() - 36.0 - x;
    let name_font = sans(14.0, Weight::SemiBold);
    let fits = |s: &str| text_width(ui, s, name_font.clone()) <= room;
    let name = if fits(&full) {
        full.clone()
    } else {
        crate::text::middle_fit(&full, &fits, crate::text::NAME_ENDS)
    };
    let mut spoken = full.clone();
    match entry {
        Entry::All => {
            paint_text(
                ui,
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                &name,
                name_font,
                p.text,
            );
        }
        Entry::One(c) => {
            super::avatar(ui, a, face, &c.id, &full, None, &mut |_| {});
            paint_text(
                ui,
                pos2(x, rect.top() + 15.0),
                Align2::LEFT_CENTER,
                &name,
                name_font,
                p.text,
            );
            let credits = credits_words(c);
            let (state, amber) = state_words(c);
            let cr = paint_text(
                ui,
                pos2(x, rect.top() + 34.0),
                Align2::LEFT_CENTER,
                &credits,
                mono(11.0, Weight::Regular),
                p.muted,
            );
            let (font, color) = if amber {
                (sans(11.0, Weight::Medium), p.accent)
            } else {
                (sans(11.0, Weight::Regular), p.muted)
            };
            // The state in what is left of the row, cut with «…» (it never runs under the ✓).
            let mut job = egui::text::LayoutJob::simple_singleline(
                format!("· {}", crate::i18n::t(state)),
                font,
                color,
            );
            job.wrap = egui::text::TextWrapping {
                max_width: (rect.right() - 36.0 - cr.right() - 6.0).max(0.0),
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            let galley = ui.painter().layout_job(job);
            let at = pos2(cr.right() + 6.0, rect.top() + 34.0 - galley.size().y / 2.0);
            ui.painter().galley(at, galley, color);
            spoken = format!(
                "{full} · {} · {}",
                crate::i18n::spoken(&credits),
                crate::i18n::spoken(state)
            );
        }
    }
    if chosen {
        a.icons.icon("check", p.accent, &p, 16.0).paint_at(
            ui,
            Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(16.0, 16.0)),
        );
    }
    named(&resp, egui::WidgetType::Button, true, &spoken);
    let hover = match entry {
        Entry::One(c) if c.name.is_some() => format!("{full}\n{}", c.id),
        _ => full,
    };
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(hover)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::IdentityReady;

    fn choice(name: Option<&str>, ready: Option<IdentityReady>) -> IdentityChoice {
        IdentityChoice {
            id: "8rd3Ux7pQfAbCdEfGhJkLmNoPqRsTuVwXyZkV2mJ38R".into(),
            name: name.map(str::to_owned),
            ready,
            ..Default::default()
        }
    }

    const READY: IdentityReady = IdentityReady {
        ours: true,
        sign_in: true,
        dashpay: true,
        master_ours: true,
    };

    /// R-1.2: one state a row, by seniority; «checking…» while not known (§8d, BUG-243).
    #[test]
    fn the_state_is_the_most_senior_one() {
        let named = Some("kestrel7");
        assert_eq!(state_words(&choice(named, None)).0, "checking…");
        assert_eq!(
            state_words(&choice(named, Some(IdentityReady::default()))).0,
            "no keys of this wallet"
        );
        let no_sign_in = IdentityReady {
            sign_in: false,
            dashpay: false,
            ..READY
        };
        assert_eq!(
            state_words(&choice(named, Some(no_sign_in))),
            ("needs a sign-in key", true)
        );
        let no_dashpay = IdentityReady {
            dashpay: false,
            ..READY
        };
        assert_eq!(
            state_words(&choice(None, Some(no_dashpay))),
            ("needs a DashPay key", true)
        );
        assert_eq!(state_words(&choice(None, Some(READY))).0, "no name");
        assert_eq!(state_words(&choice(named, Some(READY))), ("ready", false));
    }

    /// R-1.5, A1.2: by the name without «@», whatever the case, and by the start of the id.
    #[test]
    fn the_search_takes_the_name_and_the_ids_start() {
        let c = choice(Some("Kestrel7"), None);
        assert!(found(&c, "kes"));
        assert!(found(&c, "@KES"));
        assert!(found(&c, "8rd3"));
        assert!(!found(&c, "rd3"), "the id by its start only");
        assert!(!found(&c, "werner"));
        assert!(found(&c, "  "), "nothing searched: all");
    }

    /// A1.7: credits not read are «—», never «0».
    #[test]
    fn credits_not_read_are_a_dash() {
        assert_eq!(credits_words(&choice(None, None)), "—");
    }
}
