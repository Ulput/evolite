//! FEAT-0019 · Tokens (design `docs/design/tokens-2026-11-11.md`, T1–T7; design review
//! `docs/tasks/reviews/FEAT-0019-4-design.md`): the identity's Dash Platform tokens as the core
//! found and proved them, the wallet's own search of the network (first search, node pause, no
//! network, a failed «Find my tokens»), adding a token by hand and sending one. On mainnet the
//! item is there and only explains (R-9, T7). Amounts are the token's own, never DASH (R-4);
//! names are the issuer's words, so the short id and the issuer stand beside them (R-8).

mod dialogs;
pub mod state;

use crate::components::{
    Assets, ButtonKind, button, button_enabled, card, divider, page_header, txt,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::{Weight, mono, sans};
use egui::{
    Align, CornerRadius, Layout, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2,
};
use evolite_api::tokens::{
    SearchState, Sender, TokenInfo, TokenOrigin, TokenRefusal, TokenRow, TokenSearch,
};
use evolite_api::{
    Command, CoreError, KeyKind, KeyLevel, KeyPurpose, Network, ResolvedName, Wallet,
};

pub use dialogs::{AddState, SendState};

/// The widest a paragraph under a centred heading goes (T3, T7).
const NOTE_W: f32 = 460.0;

/// What the screen keeps between frames: its dialogs and the core's answers to them.
#[derive(Default)]
pub struct TokensState {
    /// «Send» of a row: the dialog for that token.
    pub sending: Option<SendState>,
    /// «Add a token».
    pub adding: Option<AddState>,
    /// The first search was asked for when the screen first opened (design §3).
    asked_first: bool,
}

impl TokensState {
    /// [`evolite_api::Event::TokenFound`].
    pub fn on_found(&mut self, query: &str, result: &Result<Vec<TokenInfo>, CoreError>) {
        if let Some(add) = &mut self.adding {
            add.on_found(query, result);
        }
    }

    /// `Done { what: Token }`: a token followed or hidden.
    pub fn on_tracked(&mut self, result: &Result<(), CoreError>) {
        if let Some(add) = &mut self.adding {
            add.on_tracked(result);
        }
    }

    /// `Done { what: SendToken }`.
    pub fn on_sent(&mut self, result: &Result<(), CoreError>) {
        if let Some(send) = &mut self.sending {
            send.on_sent(result);
        }
    }

    /// [`evolite_api::Event::NameResolved`]: the send dialog's recipient, before «Check».
    pub fn on_resolved(&mut self, name: &str, result: &Result<ResolvedName, CoreError>) {
        if let Some(send) = &mut self.sending {
            send.on_resolved(name, result);
        }
    }

    /// Lock, delete or another wallet: the dialogs and the password typed go (BUG-42).
    pub fn forget_wallet(&mut self) {
        if let Some(send) = &mut self.sending {
            send.password.forget();
        }
        *self = TokensState::default();
    }
}

/// What the shown identity has besides the token row, for the checks before the password.
pub(crate) fn sender(wallet: &Wallet) -> Sender {
    let has_key = wallet.identity.as_ref().is_some_and(|i| {
        i.keys.iter().any(|k| {
            k.level == KeyLevel::Critical
                // AUTHENTICATION below MASTER: the real core calls it «signing documents».
                && k.purpose == KeyPurpose::Documents
                && k.kind == KeyKind::Ecdsa
        })
    });
    Sender {
        has_key,
        credits: wallet
            .balances
            .credits_read
            .then_some(wallet.balances.credits.max(0) as u64),
        // The token transfer's own fee is measured live (AC-T9); the core declares this one.
        fee: wallet.fees.document_write.saturating_mul(2).max(0) as u64,
    }
}

/// The shown identity's name, or its short id: «@satoshi».
pub(crate) fn shown_name(wallet: &Wallet) -> String {
    match &wallet.identity {
        Some(i) => match i.names.iter().find(|n| n.primary).or(i.names.first()) {
            Some(n) => format!("@{}", n.label),
            None => state::short(&i.id),
        },
        None => String::new(),
    }
}

/// «@evo-lab», or the issuer's short id.
pub(crate) fn issuer(info: &TokenInfo) -> String {
    match &info.issuer_name {
        Some(name) => format!("@{name}"),
        None => state::short(&info.issuer),
    }
}

/// «id 7Hq2…9fLm · issued by @evo-lab» (+ «· added by hand»).
pub(crate) fn id_line(info: &TokenInfo, by_hand: bool) -> String {
    // The pieces are translated here: the whole line is no catalog key.
    let issued = crate::i18n::t(&format!(
        tr!("issued by {}"),
        crate::i18n::own(&issuer(info))
    ))
    .into_owned();
    // «id 7Hq2…q8Rs» one isolated left-to-right run, as @names are (BUG-119, design r3 R3-3).
    let id = crate::i18n::own(&format!("id {}", state::short(&info.token_id)));
    let mut line = format!("{id} · {issued}");
    if by_hand {
        line.push_str(" · ");
        line.push_str(&crate::i18n::t(tr!("added by hand")));
    }
    line
}

/// A paragraph at FEAT-0033's line height (1.5 of the size), at most `max` wide. Through
/// [`crate::components::para`], so Arabic is broken in reading order (BUG-119; pass-22 design
/// review, «Keep in mind»).
pub(crate) fn para(ui: &mut Ui, text: &str, size: f32, color: egui::Color32, max: Option<f32>) {
    let words = crate::components::para(text, size, Weight::Regular, color)
        .line_height((size * 1.5).round());
    match max {
        Some(max) if ui.available_width() > max => {
            ui.allocate_ui(vec2(max, 0.0), |ui| {
                ui.vertical_centered(|ui| ui.add(words));
            });
        }
        _ => {
            ui.add(words);
        }
    }
}

/// A 40 px `raised` tile with an icon, as the token icon of T1.
fn tile(ui: &mut Ui, a: &mut Assets, icon: &str, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
    crate::components::paint_icon_box(ui, a, rect, 10, icon, 20.0, color);
}

/// The search's bar: 6 high, the track `raised`, the part done in `fill`. `None`: the share is
/// unknown (BUG-132), and a third of the track runs along it. With the system's animations off
/// the track stays empty: a third standing still would read as a share (design review), and
/// the line under it says how many were read.
fn bar(ui: &mut Ui, track: egui::Color32, fill: egui::Color32, fraction: Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(3), track);
    let Some(fraction) = fraction else {
        if !crate::motion::system_on(ui.ctx()) {
            return;
        }
        ui.ctx().request_repaint();
        let width = rect.width() / 3.0;
        // Across in 1.6 s, from just before the track to just past it.
        let phase = (ui.input(|i| i.time) / 1.6).fract() as f32;
        let start = rect.left() - width + phase * (rect.width() + width);
        let run = Rect::from_min_max(
            egui::pos2(start.max(rect.left()), rect.top()),
            egui::pos2((start + width).min(rect.right()), rect.bottom()),
        );
        if run.width() > 0.0 {
            ui.painter().rect_filled(run, CornerRadius::same(3), fill);
        }
        return;
    };
    let mut done = rect;
    done.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    if done.width() > 0.0 {
        ui.painter().rect_filled(done, CornerRadius::same(3), fill);
    }
}

/// A flag beside a token's name: a quiet note, not a button (Inter 12 Regular, `muted`).
fn flag(ui: &mut Ui, a: &mut Assets, icon: &str, label: &str) {
    let p = a.p;
    let font = sans(12.0, Weight::Regular);
    let width = 8.0 + 12.0 + 6.0 + crate::components::text_width(ui, label, font.clone()) + 10.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(12), p.raised);
    let icon_at = Rect::from_center_size(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        vec2(12.0, 12.0),
    );
    a.icons.icon(icon, p.muted, &p, 12.0).paint_at(ui, icon_at);
    crate::components::paint_text(
        ui,
        egui::pos2(rect.left() + 26.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        p.muted,
    );
    crate::components::named(&resp, egui::WidgetType::Label, true, label);
}

/// An identity chip (FEAT-0019 review 4f): the chosen one `raised` with a `line` border and the
/// text colour, the others outlined in `line` with `muted` text; 28 high.
fn identity_chip(ui: &mut Ui, p: &crate::theme::Palette, label: &str, on: bool) -> egui::Response {
    let font = sans(13.0, Weight::Medium);
    let width = 24.0 + crate::components::text_width(ui, label, font.clone());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    if on {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(14), p.raised);
    }
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(14),
        Stroke::new(1.0, p.line),
        StrokeKind::Inside,
    );
    crate::components::paint_text(
        ui,
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        if on { p.text } else { p.muted },
    );
    crate::components::named_choice(&resp, egui::WidgetType::Button, on, label);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn tokens(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut TokensState) {
    let p = a.p;
    ui.spacing_mut().item_spacing.y = 16.0;
    if cx.snap.network == Network::Mainnet {
        page_header(ui, a, tr!("Tokens"), None, |_, _| {});
        mainnet(ui, a);
        return;
    }
    let wallet = cx.wallet();
    let search = wallet.tokens.search.clone();
    let busy = matches!(
        search.state,
        SearchState::Running | SearchState::Paused { .. }
    );
    let mut find = false;
    let mut add = false;
    page_header(ui, a, tr!("Tokens"), None, |ui, a| {
        add = button(ui, a, tr!("Add a token"), ButtonKind::Primary, None).clicked();
        find = button_enabled(
            ui,
            a,
            tr!("Find my tokens"),
            ButtonKind::Secondary,
            None,
            !busy,
        )
        .clicked();
    });
    // The first search starts when the screen first opens on this network (card, АРХ-7).
    if search.state == SearchState::NotYet && !state.asked_first {
        state.asked_first = true;
        find = true;
    }
    para(
        ui,
        tr!(
            "Dash Platform tokens on your identities. evolite finds them itself: it checks the network's contracts with proofs, without third-party services."
        ),
        13.0,
        p.muted,
        None,
    );
    let wallet = cx.wallet();
    let who = sender(wallet);
    let rows: Vec<TokenRow> = state::ordered(&wallet.tokens.rows)
        .into_iter()
        .cloned()
        .collect();
    let name = shown_name(wallet);
    let done = search.done.is_some();
    // T4b: about the whole screen, so above the identities (review 3f).
    if search.failed && done && search.state == SearchState::NoNetwork {
        find |= refresh_failed(ui, a, cx, &search);
    }
    identity_row(ui, a, cx);
    match search.state {
        SearchState::Running | SearchState::Paused { .. } | SearchState::NotYet if !done => {
            searching(ui, a, cx, &search)
        }
        SearchState::NoNetwork if !done => find |= no_network(ui, a, &search),
        _ => {
            if busy {
                searching(ui, a, cx, &search);
            }
            if rows.is_empty() {
                if done {
                    add |= empty(ui, a, &name, &search);
                }
            } else if let Some(token_id) = list(ui, a, &rows, state, who) {
                // Taken out: a found one is hidden, so the search does not bring it back (R-11); one added
                // by hand leaves the list (BUG-131).
                cx.send(Command::TrackToken {
                    token_id,
                    on: false,
                });
            }
        }
    }
    keep_in_mind(ui, a);
    if find {
        cx.send(Command::FindMyTokens);
    }
    if add {
        state.adding = Some(AddState::default());
    }
    if state.sending.is_some() {
        dialogs::send_dialog(ui, a, cx, state);
    }
    if state.adding.is_some() {
        dialogs::add_dialog(ui, a, cx, state);
    }
}

/// T7: tokens on mainnet are not searched or shown yet (R-9). A message, not a warning: the
/// flask muted in a tile (review 3h).
fn mainnet(ui: &mut Ui, a: &mut Assets) {
    let p = a.p;
    let title = tr!("Tokens work only on testnet for now");
    let words = tr!(
        "On mainnet evolite does not search for tokens or show them: first we finish the wallet's main functions. Your mainnet tokens have not gone anywhere — they are on your identities and will appear here when this section is switched on."
    );
    let shown = card(&p, Margin::same(28)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            tile(ui, a, "testnet", p.muted);
            ui.label(txt(title, 18.0, Weight::SemiBold, p.text));
            para(ui, words, 13.0, p.muted, Some(NOTE_W));
        });
    });
    // The screen's one Tab stop, read whole by a screen reader: without it Tab has nowhere to
    // go on this screen (the sweep over the presets).
    let spoken = format!("{} · {}", crate::i18n::t(title), crate::i18n::t(words));
    let resp = ui.interact(
        shown.response.rect,
        ui.id().with("tokens-mainnet"),
        Sense::hover() | Sense::FOCUSABLE,
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
    ui.ctx()
        .accesskit_node_builder(resp.id, |node| node.set_label(spoken.clone()));
    if resp.has_focus() {
        ui.painter().rect_stroke(
            shown.response.rect.expand(2.0),
            16,
            Stroke::new(2.0, p.accent),
            StrokeKind::Outside,
        );
    }
}

/// The identities of the wallet as chips (none with one), and when the last whole pass was;
/// «Identity» and the chips on one middle line (FEAT-0034).
fn identity_row(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let p = a.p;
    let wallet = cx.wallet();
    let shown = wallet.identity.as_ref().map(|i| i.id.clone());
    let checked = wallet
        .tokens
        .search
        .done
        .map(|(at, _)| format!(tr!("Checked {}"), text::when(at, cx.now)));
    let choices: Vec<(String, String)> = wallet
        .identities
        .iter()
        .map(|c| {
            let name = c
                .name
                .as_ref()
                .map(|n| format!("@{n}"))
                .unwrap_or_else(|| state::short(&c.id));
            (c.id.clone(), name)
        })
        .collect();
    if choices.len() < 2 && checked.is_none() {
        return;
    }
    let mut chosen = None;
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 28.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if choices.len() > 1 {
                ui.label(txt(tr!("Identity"), 13.0, Weight::Regular, p.muted));
                for (id, name) in &choices {
                    let on = Some(id) == shown.as_ref();
                    if identity_chip(ui, &p, &crate::i18n::own(name), on).clicked() && !on {
                        chosen = Some(id.clone());
                    }
                }
            }
            if let Some(checked) = &checked {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(txt(checked, 12.0, Weight::Regular, p.muted));
                });
            }
        },
    );
    if let Some(id) = chosen {
        cx.send(Command::ChooseIdentity { id });
    }
}

/// «1 840 of 4 175 contracts · about a minute left», or «1 840 network contracts read» while the
/// network's count is unknown (BUG-132): the time only once the pace is known.
pub(crate) fn progress_text(search: &TokenSearch) -> String {
    let checked = crate::format::count(search.checked as u64);
    let mut line = match search.total {
        Some(total) => crate::i18n::t(&format!(
            tr!("{} of {} contracts"),
            checked,
            crate::format::count(total as u64)
        ))
        .into_owned(),
        // BUG-132: what is counted, while the network's count is unknown.
        None => crate::i18n::t(&grouped_plural(
            search.checked,
            tr!("network contract read"),
            tr!("network contracts read"),
        ))
        .into_owned(),
    };
    if let Some(left) = search.left_secs.filter(|s| *s > 0) {
        line.push_str(" · ");
        if left < 90 {
            line.push_str(&crate::i18n::t(tr!("about a minute left")));
        } else {
            line.push_str(&crate::i18n::t(&format!(
                tr!("about {} min left"),
                left.div_ceil(60)
            )));
        }
    }
    line
}

/// «1,800 network contracts read»: [`crate::text::plural`] with the count grouped, one catalog
/// key in both forms.
fn grouped_plural(n: u32, one: &str, other: &str) -> String {
    let form = if n == 1 { one } else { other };
    format!("{} {form}", crate::format::count(n as u64))
}

/// The share of the network's contracts checked; `None` while their count is unknown.
fn checked_share(search: &TokenSearch) -> Option<f32> {
    match search.total {
        Some(total) if total > 0 => Some((search.checked as f32 / total as f32).clamp(0.0, 1.0)),
        Some(_) => Some(0.0),
        None => None,
    }
}

/// T1 and T5: the search runs, or waits for a node.
fn searching(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, search: &TokenSearch) {
    let p = a.p;
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            tile(ui, a, "token", p.text);
            ui.label(txt(
                tr!("Looking for your tokens on the network…"),
                16.0,
                Weight::SemiBold,
                p.text,
            ));
        });
        para(
            ui,
            tr!("The first search takes about 2 minutes: evolite checks the network's contracts with proofs. You can leave this screen — the search goes on, and what it finds will appear here."),
            12.0,
            p.muted,
            None,
        );
        bar(ui, p.raised, p.accent, checked_share(search));
        match search.state {
            SearchState::Paused { until } => {
                let left = (until - cx.now.timestamp()).max(0);
                let clock = format!("{}:{:02}", left / 60, left % 60);
                ui.horizontal(|ui| {
                    ui.add(a.icons.icon("auto-lock", p.muted, &p, 14.0));
                    ui.label(txt(
                        &format!(
                            tr!("Pause {} — the node asked us to slow down. The search will go on by itself."),
                            clock
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                });
                // The count goes down every second.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs(1));
            }
            _ => {
                // A sentence, not an id: Inter, not Mono (review 3b).
                ui.label(txt(&progress_text(search), 12.0, Weight::Regular, p.muted));
            }
        }
    });
}

/// T4: the first search stopped for want of the network; true when «Retry now» was clicked.
fn no_network(ui: &mut Ui, a: &mut Assets, search: &TokenSearch) -> bool {
    let p = a.p;
    let mut retry = false;
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            tile(ui, a, "warning", p.accent);
            ui.label(txt(
                tr!("Search stopped: no connection to the network"),
                16.0,
                Weight::SemiBold,
                p.text,
            ));
        });
        // The part checked stays shown, muted (review 3c).
        // Stopped: the bar stands, even while the share is unknown.
        bar(ui, p.raised, p.muted, Some(checked_share(search).unwrap_or(0.0)));
        ui.label(txt(&progress_text(&TokenSearch { left_secs: None, ..search.clone() }), 12.0, Weight::Regular, p.muted));
        let total = search.total.unwrap_or(search.checked);
        para(
            ui,
            &format!(
                tr!("Checked {} of {} contracts. Once the connection is back, the search goes on from the same place — no need to start over."),
                crate::format::count(search.checked as u64),
                crate::format::count(total as u64)
            ),
            12.0,
            p.muted,
            None,
        );
        retry = button(ui, a, tr!("Retry now"), ButtonKind::Secondary, None).clicked();
    });
    retry
}

/// T4b: «Find my tokens» failed; the list of the last pass stays. A `raised` banner with an
/// `accent` border, «Retry» inside on the right (review 3f); true when it was clicked.
fn refresh_failed(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, search: &TokenSearch) -> bool {
    let p = a.p;
    let when = search
        .done
        .map(|(at, _)| text::when(at, cx.now))
        .unwrap_or_default();
    let line = format!(
        tr!(
            "Could not find tokens: the network's nodes do not answer. Showing what was found {} — balances may have changed."
        ),
        when
    );
    let mut retry = false;
    egui::Frame::new()
        .fill(p.raised)
        .stroke(Stroke::new(1.0, p.accent))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(a.icons.icon("warning", p.accent, &p, 18.0));
                let width = (ui.available_width()
                    - crate::components::button_width(ui, tr!("Retry"))
                    - 16.0)
                    .max(120.0);
                ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                    para(ui, &line, 13.0, p.text, None);
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    retry = button(ui, a, tr!("Retry"), ButtonKind::Secondary, None).clicked();
                });
            });
        });
    retry
}

/// T3: nothing found on the shown identity; true when «Add a token» was clicked.
fn empty(ui: &mut Ui, a: &mut Assets, name: &str, search: &TokenSearch) -> bool {
    let p = a.p;
    let mut add = false;
    card(&p, Margin::same(32)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            tile(ui, a, "token", p.muted);
            ui.label(txt(
                &format!(tr!("No tokens found for {}"), crate::i18n::own(name)),
                18.0,
                Weight::SemiBold,
                p.text,
            ));
            para(
                ui,
                tr!("Tokens are issued by Dash Platform apps: points, coupons, game money. They sit on an identity, not on an address. If the search did not find your token — for example, one with a zero balance — add it by id."),
                13.0,
                p.muted,
                Some(NOTE_W),
            );
            // The one thing to do here: the primary (review 3g).
            add = button(ui, a, tr!("Add a token"), ButtonKind::Primary, None).clicked();
            if let Some((_, checked)) = search.done {
                ui.label(txt(
                    &format!(
                        tr!("Checked {} contracts on the network"),
                        crate::format::count(checked as u64)
                    ),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
        });
    });
    add
}

/// T2: the rows; the token whose «Hide» or «Remove» was clicked.
fn list(
    ui: &mut Ui,
    a: &mut Assets,
    rows: &[TokenRow],
    state: &mut TokensState,
    who: Sender,
) -> Option<String> {
    let p = a.p;
    let mut send = None;
    let mut hide = None;
    card(&p, Margin::symmetric(20, 0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                divider(ui, &p);
            }
            match token_row(ui, a, row, who) {
                RowAction::Send => send = Some(row.info.token_id.clone()),
                RowAction::Hide => hide = Some(row.info.token_id.clone()),
                RowAction::None => {}
            }
        }
    });
    if let Some(id) = send {
        state.sending = Some(SendState::new(&id));
    }
    hide
}

enum RowAction {
    None,
    Send,
    Hide,
}

/// Whether the row's «Send» is on: only what the row itself says keeps it off, and it says
/// why (R-3а); the credits and the key are said in the dialog.
pub(crate) fn can_send(row: &TokenRow, who: Sender) -> bool {
    state::row_reason(row).is_none()
        && match state::form_problem(row, "", "", who) {
            None
            | Some(state::FormProblem::NoCredits)
            | Some(state::FormProblem::Token(TokenRefusal::NoKey)) => true,
            Some(_) => false,
        }
}

/// One row: 16 above and below, the circle on the middle of the words (review 4a, FEAT-0034).
fn token_row(ui: &mut Ui, a: &mut Assets, row: &TokenRow, who: Sender) -> RowAction {
    let p = a.p;
    let reason = state::row_reason(row);
    let claim = row.claim.then_some(tr!(
        "A distribution interval has passed since the last claim — there may be a payout. evolite does not know the amount yet and will learn to claim it in the next version."
    ));
    let mut action = RowAction::None;
    let balance = match row.balance {
        Some(b) => state::held_amount(b, row.info.decimals),
        // Not read yet: never shown as 0 (AC-T5).
        None => "—".to_owned(),
    };
    let right = 180.0;
    ui.add_space(16.0);
    let words = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (slot, _) = ui.allocate_exact_size(vec2(36.0, 1.0), Sense::hover());
        let block = ui
            .allocate_ui_with_layout(
                vec2((ui.available_width() - right).max(120.0), 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(txt(
                            &crate::i18n::own(state::name(&row.info)),
                            14.0,
                            Weight::SemiBold,
                            p.text,
                        ));
                        // A found token is hidden, so the search does not bring it back; one
                        // added by hand is removed from the list (BUG-131). The core tells
                        // them apart by the origin it keeps.
                        let take_out = if row.origin == TokenOrigin::Found {
                            flag(ui, a, "search", tr!("found automatically"));
                            tr!("Hide")
                        } else {
                            tr!("Remove")
                        };
                        // A label that clicks, not `ui.link`: it is mirrored in Arabic as the
                        // words beside it are.
                        let out = ui
                            .add(
                                egui::Label::new(
                                    txt(take_out, 12.0, Weight::Regular, p.muted).underline(),
                                )
                                .extend()
                                .sense(Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        crate::components::named(&out, egui::WidgetType::Button, true, take_out);
                        if out.clicked() {
                            action = RowAction::Hide;
                        }
                        if row.paused == Some(true) {
                            flag(ui, a, "info", tr!("paused"));
                        }
                        if row.frozen == Some(true) {
                            flag(ui, a, "lock", tr!("frozen"));
                        }
                        if row.claim {
                            flag(ui, a, "plus", tr!("something to claim"));
                        }
                    });
                    ui.label(txt(
                        &id_line(&row.info, row.origin == TokenOrigin::ByHand),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    // Why it cannot be sent, then the claim note: Inter 12 Medium (review 4d).
                    for why in reason.into_iter().chain(claim) {
                        ui.label(txt(why, 12.0, Weight::Medium, p.text).line_height(Some(18.0)));
                    }
                },
            )
            .response
            .rect;
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), block.height()),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                if button_enabled(
                    ui,
                    a,
                    tr!("Send"),
                    ButtonKind::Ghost,
                    None,
                    can_send(row, who),
                )
                .clicked()
                {
                    action = RowAction::Send;
                }
                let shown = ui.label(
                    RichText::new(balance.clone())
                        .font(mono(14.0, Weight::Medium))
                        .color(p.text),
                );
                if row.balance.is_none() {
                    shown.on_hover_text(
                        crate::i18n::t(tr!("The balance is not checked yet")).into_owned(),
                    );
                }
            },
        );
        (slot, block)
    });
    let (slot, block) = words.inner;
    let center = egui::pos2(slot.center().x, block.center().y);
    ui.painter().circle_filled(center, 18.0, p.raised);
    a.icons
        .icon("token", p.text, &p, 18.0)
        .paint_at(ui, Rect::from_center_size(center, vec2(18.0, 18.0)));
    ui.add_space(16.0);
    action
}

/// «Keep in mind» under the list, always (design §5).
fn keep_in_mind(ui: &mut Ui, a: &mut Assets) {
    let p = a.p;
    card(&p, Margin::same(16)).fill(p.raised).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            ui.add(a.icons.icon("info", p.muted, &p, 16.0));
            ui.label(txt(tr!("Keep in mind"), 13.0, Weight::SemiBold, p.text));
        });
        para(
            ui,
            tr!("Tokens with a zero balance do not appear in the list — add such a token by id. Payouts are not in the balance: if a perpetual distribution may have something to claim, the token is marked “something to claim”. Payouts of distributions to evonodes and of pre-programmed distributions are not shown here. Token balances and transfers are visible to everyone on the network."),
            12.0,
            p.muted,
            None,
        );
    });
}

#[cfg(test)]
mod progress_tests {
    use super::*;

    fn search(checked: u32, total: Option<u32>, left_secs: Option<u32>) -> TokenSearch {
        TokenSearch {
            state: SearchState::Running,
            checked,
            total,
            left_secs,
            done: None,
            failed: false,
        }
    }

    /// BUG-132: while the network's count is unknown the line says what is counted; once it is
    /// known, «N of M contracts».
    #[test]
    fn the_progress_says_what_it_counts() {
        let n = crate::format::count(1_800);
        crate::i18n::set(crate::i18n::Lang::En);
        assert_eq!(
            progress_text(&search(1_800, None, None)),
            format!("{n} network contracts read")
        );
        assert_eq!(
            progress_text(&search(1_800, None, Some(60))),
            format!("{n} network contracts read · about a minute left")
        );
        assert_eq!(
            progress_text(&search(1, None, None)),
            "1 network contract read"
        );
        assert_eq!(
            progress_text(&search(1_800, Some(4_190), None)),
            format!("{n} of {} contracts", crate::format::count(4_190))
        );
        crate::i18n::set(crate::i18n::Lang::Ru);
        assert_eq!(
            progress_text(&search(1_800, None, None)),
            "Прочитано 1\u{a0}800 контрактов сети"
        );
        assert_eq!(
            progress_text(&search(21, None, None)),
            "Прочитан 21 контракт сети"
        );
        // Design review: the three Russian forms, not the two English ones.
        assert_eq!(
            progress_text(&search(1_841, None, None)),
            "Прочитан 1\u{a0}841 контракт сети"
        );
        assert_eq!(
            progress_text(&search(1_842, None, None)),
            "Прочитано 1\u{a0}842 контракта сети"
        );
        crate::i18n::set(crate::i18n::Lang::En);
    }
}
