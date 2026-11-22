//! Suggestions under a field where another person's @name is typed (FEAT-0039, frames N1–N5):
//! up to five names — the exact one, contacts, one's own identities, the network's — in a list
//! laid over what is below the field. Choosing a row only fills the field: the name is then
//! checked as if typed (`ResolveName`), nothing is paid or sent from here.
//!
//! The network is asked 300 ms after the last change of the name's beginning, from 3
//! characters, with a number (`seq`) so a late answer to an older beginning is dropped; after
//! 5 s without an answer the list says so and keeps one's own rows.

mod rank;

pub use rank::{Place, Row, Tag, rank, word_start};

use std::time::{Duration, Instant};

use super::prelude::*;
use evolite_api::{Command, CoreError, NameSuggestion};
use evolite_core::dpns::{SEARCH_FROM, search_key};

/// The pause after the last change before the network is asked (architect §3).
pub const PAUSE: Duration = Duration::from_millis(300);
/// How long the network may take before the list says it did not answer (analyst).
pub const NO_ANSWER: Duration = Duration::from_secs(5);
/// The most names one answer of the network holds: fewer are all there are.
const FULL: usize = 10;

/// The numbers of the asks, one sequence for every field: an answer is told by its number alone.
static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One row's height (frame N2): fixed, a profile's name coming later does not move it.
const ROW: f32 = 48.0;

/// Where the network's answer stands.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Net {
    /// Nothing to ask: fewer than 3 characters, or not a name.
    Idle,
    /// To ask once the pause is over.
    Pausing,
    /// Asked at that moment.
    Waiting(Instant),
    /// Answered (its names in `network`).
    Answered,
    /// No answer in time, an error, or Platform not ready yet.
    Failed,
}

/// The suggestions of one field, kept by its screen's state.
pub struct NameSuggest {
    /// The beginning last seen, in DPNS's form; `None`: not a name.
    key: Option<String>,
    changed_at: Instant,
    /// The last ask: its number and the beginning asked.
    asked: Option<(u64, String)>,
    net: Net,
    /// The network's names for the last answer.
    network: Vec<NameSuggestion>,
    /// The list is shown (closed by Esc, Tab, a choice, a click elsewhere).
    open: bool,
    /// The row chosen by the keyboard or the pointer.
    selected: Option<usize>,
    /// The count read out, once the network has had its say (once per opening).
    announced: Option<usize>,
    /// The list was drawn last frame: Esc is its own.
    shown: bool,
    /// How many `SuggestNames` this field sent (AC-A5).
    pub sent: u32,
}

impl Default for NameSuggest {
    fn default() -> Self {
        Self {
            key: None,
            changed_at: Instant::now(),
            asked: None,
            net: Net::Idle,
            network: Vec::new(),
            open: false,
            selected: None,
            announced: None,
            shown: false,
            sent: 0,
        }
    }
}

impl NameSuggest {
    /// Follows what is typed: a new beginning opens the list and, after [`PAUSE`], asks the
    /// network for it once — unless an answer that was not full already holds every name on a
    /// shorter beginning. A wait past [`NO_ANSWER`] fails. Call every frame, before drawing.
    pub fn follow(&mut self, ctx: &egui::Context, typed: &str, send: &mut impl FnMut(Command)) {
        self.follow_at(ctx, typed, Instant::now(), send);
    }

    fn follow_at(
        &mut self,
        ctx: &egui::Context,
        typed: &str,
        now: Instant,
        send: &mut impl FnMut(Command),
    ) {
        let key = search_key(typed).filter(|k| !k.is_empty());
        if key != self.key {
            self.key = key.clone();
            self.changed_at = now;
            self.open = key.is_some();
            self.selected = None;
            self.announced = None;
            self.net = match &key {
                Some(k) if k.len() >= SEARCH_FROM => {
                    if self.narrows(k) {
                        Net::Answered
                    } else {
                        Net::Pausing
                    }
                }
                _ => Net::Idle,
            };
        }
        match self.net {
            Net::Pausing => {
                let due = self.changed_at + PAUSE;
                if now >= due {
                    let prefix = self.key.clone().unwrap_or_default();
                    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    self.asked = Some((seq, prefix.clone()));
                    self.net = Net::Waiting(now);
                    self.sent += 1;
                    send(Command::SuggestNames { prefix, seq });
                    ctx.request_repaint_after(NO_ANSWER);
                } else {
                    ctx.request_repaint_after(due - now);
                }
            }
            Net::Waiting(since) => {
                if now >= since + NO_ANSWER {
                    self.net = Net::Failed;
                } else {
                    ctx.request_repaint_after(since + NO_ANSWER - now);
                }
            }
            _ => {}
        }
    }

    /// The last answer holds every name on `key`: it was to a beginning of it and not full.
    fn narrows(&self, key: &str) -> bool {
        self.net == Net::Answered
            && self.network.len() < FULL
            && self
                .asked
                .as_ref()
                .is_some_and(|(_, asked)| key.starts_with(asked.as_str()))
    }

    /// The core's answer: taken only for the last ask (AC-T4).
    pub fn on_answer(&mut self, seq: u64, result: &Result<Vec<NameSuggestion>, CoreError>) {
        if self.asked.as_ref().map(|(s, _)| *s) != Some(seq) {
            return;
        }
        match result {
            Ok(names) => {
                self.network = names.clone();
                self.net = Net::Answered;
            }
            Err(_) => {
                self.network.clear();
                self.net = Net::Failed;
            }
        }
    }

    /// The network's names, for [`rank`].
    pub fn network(&self) -> &[NameSuggestion] {
        &self.network
    }

    /// The list is open for the field with `field` id (it has the focus).
    pub fn is_open(&self, ctx: &egui::Context, field: egui::Id) -> bool {
        self.open && ctx.memory(|m| m.has_focus(field))
    }

    /// The keys of an open list, taken before the field sees them: ↓/↑ move along the rows,
    /// Enter chooses the selected one, Esc closes the list (the second Esc is the screen's),
    /// Tab closes it and goes on. Call before drawing the field with id `field`.
    pub fn keys(&mut self, ui: &mut Ui, field: egui::Id, rows: &[Row]) -> Option<Row> {
        if !self.is_open(ui.ctx(), field) {
            return None;
        }
        let n = rows.len();
        let mut picked = None;
        ui.input_mut(|i| {
            if n > 0 && i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                self.selected = Some(self.selected.map_or(0, |s| (s + 1).min(n - 1)));
            }
            if n > 0 && i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                self.selected = self.selected.and_then(|s| s.checked_sub(1));
            }
            if let Some(s) = self.selected.filter(|s| *s < n)
                && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            {
                picked = Some(rows[s].clone());
            }
            if self.shown && i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.open = false;
            }
            if i.key_pressed(egui::Key::Tab) {
                self.open = false;
            }
        });
        picked
    }

    /// Puts the chosen row's name in `value` and closes the list; the field then goes on as if
    /// it was typed.
    pub fn choose(&mut self, value: &mut String, row: &Row) {
        *value = format!("@{}", row.label);
        self.key = search_key(value).filter(|k| !k.is_empty());
        self.open = false;
        self.selected = None;
    }

    /// Draws the open list under the field drawn last in `ui` (`field` its id, `typed` its text):
    /// over what is below, as wide as the field, above it when the window has no room below.
    /// A click on a row chooses it; a click elsewhere closes the list.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        a: &mut Assets,
        field: egui::Id,
        typed: &str,
        rows: &[Row],
        send: &mut impl FnMut(Command),
    ) -> Option<Row> {
        let ctx = ui.ctx().clone();
        self.shown = false;
        if !self.is_open(&ctx, field) {
            // A click on a row takes the focus off the field for that frame: the list waits for it.
            if !self.open || ctx.memory(|m| m.focused().is_some()) {
                return None;
            }
        }
        let footer = self.footer(rows.is_empty());
        if rows.is_empty() && footer.is_none() {
            return None;
        }
        let p = a.p;
        let w = ui.max_rect().width();
        let under = ui.min_rect().bottom() + 4.0;
        let tall = rows.len() as f32 * ROW + 8.0 + if footer.is_some() { 40.0 } else { 0.0 };
        let screen = ctx.content_rect();
        let top = if under + tall > screen.bottom() {
            // Above the field: its top is under the label, above the list's own gap.
            (ui.min_rect().bottom() - 52.0 - tall).max(screen.top())
        } else {
            under
        };
        let count = rows.len();
        // Said once the network has answered or failed: the count it settles on.
        let settled = !matches!(self.net, Net::Pausing | Net::Waiting(_));
        if settled && self.announced.is_none() {
            self.announced = Some(count);
        }
        let announced = self.announced.unwrap_or(count);
        self.shown = true;
        // While the list is shown the field keeps the focus through Esc and ↑/↓: egui would
        // drop it on Esc and move it on the arrows. Set after the field, which sets its own.
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(
                field,
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
        let mut picked = None;
        let spoken = crate::i18n::spoken(&crate::text::plural_in(
            announced as u32,
            tr!("{} suggestion"),
            tr!("{} suggestions"),
        ));
        let area = egui::Area::new(field.with("suggestions"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos2(ui.max_rect().left(), top))
            .show(&ctx, |ui| {
                Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.line))
                    .corner_radius(CornerRadius::same(12))
                    .shadow(egui::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: Color32::from_black_alpha(115),
                    })
                    .inner_margin(Margin::same(4))
                    .show(ui, |ui| {
                        ui.set_width(w - 10.0);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        // The rows are this container's children: a listbox of options, its
                        // count read out once when it opens.
                        ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                            node.set_role(egui::accesskit::Role::ListBox);
                            node.set_label(spoken.clone());
                            node.set_live(egui::accesskit::Live::Polite);
                        });
                        for (i, row) in rows.iter().enumerate() {
                            let resp = self.row(ui, a, field, i, row, send);
                            if resp.hovered() {
                                self.selected = Some(i);
                            }
                            if resp.clicked() {
                                picked = Some(row.clone());
                            }
                        }
                        if let Some((words, spin)) = &footer {
                            self.footer_line(ui, &p, words, *spin, !rows.is_empty(), typed);
                        }
                    })
            });
        let list = area.response;
        if let Some(s) = self.selected.filter(|s| *s < rows.len()) {
            let active = row_id(field, s).accesskit_id();
            ctx.accesskit_node_builder(field, |node| node.set_active_descendant(active));
        }
        // A click elsewhere closes it; a click on a row is a choice.
        if picked.is_none()
            && ctx.input(|i| i.pointer.any_click())
            && !list.contains_pointer()
            && !ctx.memory(|m| m.has_focus(field))
        {
            self.open = false;
        }
        if picked.is_some() {
            ctx.memory_mut(|m| m.request_focus(field));
        }
        picked
    }

    /// The line under the rows: the network still thinking, or not answering; with no row,
    /// that nothing was found.
    fn footer(&self, empty: bool) -> Option<(&'static str, bool)> {
        match self.net {
            Net::Pausing | Net::Waiting(_) => Some((tr!("Searching the network…"), true)),
            Net::Failed => Some((
                tr!("The network did not answer — only your contacts are shown"),
                false,
            )),
            Net::Answered | Net::Idle if empty => Some((tr!("No names starting with “{}”"), false)),
            _ => None,
        }
    }

    fn footer_line(
        &self,
        ui: &mut Ui,
        p: &Palette,
        words: &str,
        spin: bool,
        under_rows: bool,
        typed: &str,
    ) {
        let text = if words.contains("{}") {
            let shown = typed.trim();
            words.replacen("{}", &crate::i18n::own(shown), 1)
        } else {
            words.to_owned()
        };
        if under_rows {
            let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter()
                .hline(line.x_range(), line.center().y, Stroke::new(1.0, p.line));
        }
        let size = if under_rows || spin { 12.0 } else { 13.0 };
        Frame::new()
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if spin {
                        ui.add(egui::Spinner::new().size(12.0).color(p.muted));
                    }
                    ui.label(txt(&text, size, Weight::Regular, p.muted));
                });
            });
    }

    /// One row: the picture, the @name with the matched part SemiBold, the profile's name under
    /// it, the tag on the right. Not a Tab stop: the focus stays in the field.
    fn row(
        &self,
        ui: &mut Ui,
        a: &mut Assets,
        field: egui::Id,
        i: usize,
        row: &Row,
        send: &mut impl FnMut(Command),
    ) -> Response {
        let p = a.p;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
        let resp = ui.interact(rect, row_id(field, i), Sense::CLICK);
        let selected = self.selected == Some(i);
        if selected {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), p.raised);
        }
        let pic = Rect::from_center_size(
            pos2(rect.left() + 10.0 + 14.0, rect.center().y),
            vec2(28.0, 28.0),
        );
        // A picture not drawn yet is asked of the core's renderer (`DrawArt`), as everywhere.
        super::avatar(
            ui,
            a,
            pic,
            &row.identity_id,
            &row.label,
            row.picture.as_deref(),
            send,
        );
        let tag = match row.tag {
            Tag::None => None,
            Tag::Contact => Some(tr!("In contacts")),
            Tag::Own => Some(tr!("Your identity")),
        };
        let mut right = rect.right() - 10.0;
        if let Some(tag) = tag {
            let galley = ui.painter().layout_no_wrap(
                crate::i18n::t(tag).into_owned(),
                sans(11.0, Weight::Medium),
                p.muted,
            );
            let pill = Rect::from_min_size(
                pos2(right - galley.size().x - 16.0, rect.center().y - 10.0),
                vec2(galley.size().x + 16.0, 20.0),
            );
            ui.painter().rect_stroke(
                pill,
                CornerRadius::same(8),
                Stroke::new(1.0, p.line),
                StrokeKind::Inside,
            );
            ui.painter()
                .galley(pill.center() - galley.size() / 2.0, galley, p.muted);
            right = pill.left() - 8.0;
        }
        let left = rect.left() + 48.0;
        let name = name_job(&row.label, row.matched, p.text, right - left);
        let name = ui.painter().layout_job(name);
        let display = row.display.as_deref().filter(|d| !d.trim().is_empty());
        let name_y = if display.is_some() {
            rect.center().y - name.size().y + 1.0
        } else {
            rect.center().y - name.size().y / 2.0
        };
        ui.painter().galley(pos2(left, name_y), name, p.text);
        if let Some(display) = display {
            // BUG-268: the word that matched SemiBold, so it is seen why the row is here.
            let job = display_job(display, row.display_matched, p.muted, right - left);
            let galley = ui.painter().layout_job(job);
            ui.painter()
                .galley(pos2(left, rect.center().y + 2.0), galley, p.muted);
        }
        let mut spoken = format!("@{}", row.label);
        if let Some(d) = display {
            spoken.push_str(", ");
            spoken.push_str(d);
        }
        if let Some(tag) = tag {
            spoken.push_str(", ");
            spoken.push_str(&crate::i18n::spoken(tag));
        }
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, &spoken));
        ui.ctx().accesskit_node_builder(resp.id, |node| {
            node.set_role(egui::accesskit::Role::ListBoxOption);
            node.set_label(spoken.clone());
            node.set_selected(selected);
        });
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }
}

/// The row's widget id: the list's own, under its field.
fn row_id(field: egui::Id, i: usize) -> egui::Id {
    field.with(("suggestion", i))
}

/// The profile's name under the @name, its `matched` characters (from, how many) SemiBold: the
/// start of the word that matched (BUG-268). One line, cut with «…» at `width`.
fn display_job(
    display: &str,
    matched: Option<(usize, usize)>,
    color: Color32,
    width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let format = |weight| egui::TextFormat {
        font_id: sans(12.0, weight),
        color,
        ..Default::default()
    };
    let at = |n: usize| {
        display
            .char_indices()
            .nth(n)
            .map_or(display.len(), |(i, _)| i)
    };
    match matched {
        Some((from, len)) => {
            let (a, b) = (at(from), at(from + len));
            job.append(&display[..a], 0.0, format(Weight::Regular));
            job.append(&display[a..b], 0.0, format(Weight::SemiBold));
            job.append(&display[b..], 0.0, format(Weight::Regular));
        }
        None => job.append(display, 0.0, format(Weight::Regular)),
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(40.0));
    job
}

/// «@alice» with its first `matched` characters SemiBold, one line, cut with «…» at `width`.
fn name_job(label: &str, matched: usize, color: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let format = |weight| egui::TextFormat {
        font_id: sans(14.0, weight),
        color,
        ..Default::default()
    };
    let cut = label
        .char_indices()
        .nth(matched)
        .map_or(label.len(), |(i, _)| i);
    let bold = if matched > 0 {
        Weight::SemiBold
    } else {
        Weight::Regular
    };
    job.append("@", 0.0, format(bold));
    job.append(&label[..cut], 0.0, format(bold));
    job.append(&label[cut..], 0.0, format(Weight::Regular));
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(40.0));
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(s: &mut NameSuggest, ctx: &egui::Context, typed: &str, at: Instant) -> Vec<Command> {
        let mut sent = Vec::new();
        s.follow_at(ctx, typed, at, &mut |c| sent.push(c));
        sent
    }

    fn seq_of(sent: &[Command]) -> u64 {
        match sent {
            [Command::SuggestNames { seq, .. }] => *seq,
            other => panic!("one ask, not {}", other.len()),
        }
    }

    /// AC-A5, AC-T4: nothing under 3 characters; typing «alice» quickly is one ask, after the
    /// pause; a late answer to an older ask is dropped.
    #[test]
    fn one_ask_after_the_pause_and_only_the_last_answer() {
        let ctx = egui::Context::default();
        let mut s = NameSuggest::default();
        let t0 = Instant::now();
        let mut all = Vec::new();
        for (i, typed) in ["@a", "@al", "@ali", "@alic", "@alice"].iter().enumerate() {
            all.extend(asked(
                &mut s,
                &ctx,
                typed,
                t0 + Duration::from_millis(100 * i as u64),
            ));
        }
        assert!(all.is_empty(), "nothing before the pause");
        let sent = asked(
            &mut s,
            &ctx,
            "@alice",
            t0 + Duration::from_millis(400 + 300),
        );
        assert!(matches!(
            sent.as_slice(),
            [Command::SuggestNames { prefix, .. }] if prefix == "a11ce"
        ));
        assert_eq!(s.sent, 1);
        // Nothing is asked under three characters.
        let mut s2 = NameSuggest::default();
        assert!(asked(&mut s2, &ctx, "al", t0).is_empty());
        assert!(asked(&mut s2, &ctx, "al", t0 + Duration::from_secs(2)).is_empty());
        assert_eq!(s2.sent, 0);
        // «alex» asked after «ali»: the late answer to «ali» is not taken.
        let mut s3 = NameSuggest::default();
        asked(&mut s3, &ctx, "ali", t0);
        let first = seq_of(&asked(&mut s3, &ctx, "ali", t0 + PAUSE));
        asked(&mut s3, &ctx, "alex", t0 + PAUSE * 2);
        let second = seq_of(&asked(&mut s3, &ctx, "alex", t0 + PAUSE * 3));
        let late = vec![NameSuggestion {
            label: "alice".into(),
            identity_id: "a".into(),
            display_name: None,
            picture: None,
        }];
        s3.on_answer(first, &Ok(late));
        assert!(
            s3.network().is_empty(),
            "the answer to «ali» came after «alex» was asked"
        );
        s3.on_answer(second, &Ok(Vec::new()));
        assert_eq!(s3.net, Net::Answered);
    }

    /// AC-T3 on the screen's side: an answer that was not full is narrowed without an ask.
    #[test]
    fn a_longer_beginning_of_a_short_answer_is_not_asked() {
        let ctx = egui::Context::default();
        let mut s = NameSuggest::default();
        let t0 = Instant::now();
        asked(&mut s, &ctx, "ali", t0);
        let seq = seq_of(&asked(&mut s, &ctx, "ali", t0 + PAUSE));
        s.on_answer(seq, &Ok(Vec::new()));
        asked(&mut s, &ctx, "alic", t0 + PAUSE * 2);
        assert!(asked(&mut s, &ctx, "alic", t0 + PAUSE * 4).is_empty());
        assert_eq!(s.sent, 1);
    }

    /// AC-A4: no answer in 5 s fails.
    #[test]
    fn five_seconds_without_an_answer_fail() {
        let ctx = egui::Context::default();
        let mut s = NameSuggest::default();
        let t0 = Instant::now();
        asked(&mut s, &ctx, "bob", t0);
        asked(&mut s, &ctx, "bob", t0 + PAUSE);
        asked(
            &mut s,
            &ctx,
            "bob",
            t0 + PAUSE + NO_ANSWER - Duration::from_millis(1),
        );
        assert!(matches!(s.net, Net::Waiting(_)));
        asked(&mut s, &ctx, "bob", t0 + PAUSE + NO_ANSWER);
        assert_eq!(s.net, Net::Failed);
        assert_eq!(
            s.footer(false).map(|f| f.0),
            Some("The network did not answer — only your contacts are shown")
        );
    }
}
