//! The menu on the left: logo, the wallet switcher, sections, the sync widget, network and
//! settings.

use super::NAV;
use super::Screen;
use crate::components::{Assets, NAV_ITEM, nav_item, network_chip};
use crate::context::Ctx;
use crate::icons::Logo;
use crate::text::{self, group_thousands};
use crate::theme::{Weight, sans};
use egui::{Align, Layout, Stroke, Ui, vec2};
use evolite_api::SyncStage;

/// Below this height the menu is low (FEAT-0036): no logo at the top, items of
/// [`NAV_ITEM_LOW`], the sync card of 44 px.
pub const LOW_MENU: f32 = 640.0;
/// The sidebar frame's border (`app.rs`).
const PANEL_BORDER: f32 = 1.0;
/// What a synced block line ends with when its block has a ChainLock. A no-break space after
/// the «·»: a line too long breaks before «· ChainLock», which goes down whole, never leaving
/// the «·» at the end of the first row, away from the word it introduces (BUG-119, design
/// review).
pub(crate) const CHAINLOCKED: &str = " ·\u{a0}ChainLock";
/// A menu item's height in a low menu: 36, so 40 apart.
pub const NAV_ITEM_LOW: f32 = 36.0;

/// Whether a menu `height` px high is low.
pub fn low(height: f32) -> bool {
    height < LOW_MENU
}

/// The fade's height: it reaches up over the last whole item (the designer, FEAT-0036 row 3).
const FADE: f32 = 24.0;

#[cfg(test)]
fn more_id() -> egui::Id {
    egui::Id::new("evolite-menu-more-below")
}

#[cfg(test)]
fn extra_id() -> egui::Id {
    egui::Id::new("evolite-menu-extra-items")
}

/// Whether the menu's fade showed in the last frame (for tests); `None`: no menu drawn yet.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub(crate) fn menu_more_below(ctx: &egui::Context) -> Option<bool> {
    menu_fade(ctx).map(|fade| fade.is_some())
}

/// The rect of the menu's fade in the last frame (for tests): `Some(None)` when none showed.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub(crate) fn menu_fade(ctx: &egui::Context) -> Option<Option<egui::Rect>> {
    ctx.data(|d| d.get_temp::<Option<egui::Rect>>(more_id()))
}

/// Tests: `n` more menu items after [`NAV`], as «Votes» (FEAT-0026) will be, before it is in.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub(crate) fn add_menu_items(ctx: &egui::Context, n: usize) {
    ctx.data_mut(|d| d.insert_temp(extra_id(), n));
}

#[cfg(test)]
fn more_items(ctx: &egui::Context) -> Vec<(Screen, &'static str, &'static str)> {
    let n = ctx.data(|d| d.get_temp::<usize>(extra_id())).unwrap_or(0);
    [(Screen::Collection, "nft", "Votes")]
        .into_iter()
        .take(n)
        .collect()
}

/// Sidebar 248 px (a rail of 76 px below 1280, `theme::Breakpoint`): logo, the wallet switcher
/// (Figma 26), the items of [`NAV`], sync widget, network and settings. Below [`LOW_MENU`] high
/// without the logo and with closer items. `menu`: the wallet menu is open.
pub fn sidebar(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    active: Screen,
    rail: bool,
    menu: &mut bool,
) {
    let p = a.p;
    let snap = cx.snap;
    let status = &snap.sync;
    let network = snap.network;
    ui.spacing_mut().item_spacing.y = 4.0;
    // The menu's own height, bands above it (mainnet, a refused folder) taken away: the whole
    // panel, so it does not change with what is drawn and the menu never flickers at the edge.
    let area = ui.max_rect();
    // The panel's 1 px border, above and below, is not in `area` (`app.rs`, the sidebar's
    // frame): with it, the menu in a window 640 px high is 640.
    let compact = low(area.height() + 2.0 * PANEL_BORDER);
    if compact {
        // FEAT-0036: no logo at the top (the mark stays in the sync card, BUG-75); the switcher
        // 20 px from the top.
        ui.add_space(20.0);
    } else {
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            if rail {
                ui.add_space((ui.available_width() - 36.0) / 2.0);
                ui.add(a.icons.logo(Logo::Mark, &p, 36.0));
            } else {
                ui.add_space(8.0);
                ui.add(a.icons.logo(Logo::Horizontal, &p, 35.0));
            }
        });
    }
    if snap.wallets.iter().any(|w| w.open) {
        // Figma 26: the switcher 87 px from the top, the items 4 px under it.
        if !compact {
            ui.add_space(if rail { 20.0 } else { 24.0 });
        }
        super::wallets::switcher(ui, a, cx, rail, menu);
    } else if !compact {
        ui.add_space(24.0);
    }
    // Footer pinned to the bottom: the sync card (64 px, the rail's 52; 44 in a low window,
    // BUG-155), 12 px, network and settings (40 px), 20 px. The items scroll between, never
    // under it: at 150 % on a laptop the window is about 450 px high.
    let sync_h = if compact {
        SYNC_LOW
    } else if rail {
        SYNC_RAIL
    } else {
        SYNC_WIDE
    };
    let footer = egui::Rect::from_min_max(
        egui::pos2(area.left(), area.bottom() - 60.0),
        egui::pos2(area.right(), area.bottom() - 20.0),
    );
    // FEAT-0037 N3: the rail on mainnet has the network too, over Settings: a globe in a
    // `danger` frame, 40 px and 8 above it (the wide menu has the chip beside Settings).
    let rail_mainnet = rail && network == evolite_api::Network::Mainnet;
    let net = rail_mainnet.then(|| {
        egui::Rect::from_center_size(
            egui::pos2(footer.right() - 20.0, footer.top() - 8.0 - 20.0),
            vec2(40.0, 40.0),
        )
    });
    let above = net.map_or(footer.top(), |r| r.top());
    let sync = egui::Rect::from_min_max(
        egui::pos2(area.left(), above - 12.0 - sync_h),
        egui::pos2(area.right(), above - 12.0),
    );
    let room = (sync.top() - 12.0 - ui.cursor().top()).max(40.0);
    // What this network has (the network matrix, FEAT-0037).
    #[allow(unused_mut)]
    let mut items: Vec<_> = NAV
        .into_iter()
        .filter(|(screen, _, _)| super::shown_on(*screen, network))
        .collect();
    #[cfg(test)]
    items.extend(more_items(ui.ctx()));
    let height = if compact { NAV_ITEM_LOW } else { NAV_ITEM };
    let shown = egui::ScrollArea::vertical()
        .id_salt("nav")
        .max_height(room)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (screen, icon, label) in items {
                let lit = screen == active
                    || (screen == Screen::Identity
                        && matches!(active, Screen::CreateIdentity | Screen::IdentityKeys))
                    || (screen == Screen::Tokens && active == Screen::TokenPage);
                let resp = nav_item(ui, a, icon, label, lit, rail, height);
                // Tab to an item under the edge brings it into view (FEAT-0036 AC-D4).
                if resp.gained_focus() {
                    crate::diagnostics::ask_scroll(ui.ctx());
                    resp.scroll_to_me(None);
                }
                if resp.clicked() {
                    cx.go(screen);
                }
            }
        });
    // More below the edge: a fade into the menu's colour at the bottom of the list, gone once
    // it is scrolled to its end (FEAT-0036 row 3).
    let view = shown.inner_rect;
    let more = crate::components::more_below(&shown);
    let fade = more.then(|| crate::components::fade_up(ui, view, p.surface, FADE));
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(more_id(), fade));
    #[cfg(not(test))]
    let _ = fade;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(footer)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            if !rail {
                network_chip(ui, a, network);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // 40 px square; raised while Settings is open, as in Figma.
                let on = active == Screen::Settings;
                let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), egui::Sense::click());
                // Р1: a newer release, signed — a dot on Settings, and said to a screen reader.
                let newer = cx.snap.update.as_ref().is_some_and(|u| u.newer);
                crate::components::named(&resp, egui::WidgetType::Button, true, tr!("Settings"));
                if newer {
                    let said = crate::i18n::spoken(tr!("A new version of evolite is out"));
                    ui.ctx()
                        .accesskit_node_builder(resp.id, |node| node.set_description(said));
                }
                if on || resp.hovered() {
                    ui.painter().rect_filled(rect, 10, p.raised);
                }
                let color = if on { p.text } else { p.muted };
                a.icons.icon("settings", color, &p, 24.0).paint_at(
                    ui,
                    egui::Rect::from_center_size(rect.center(), vec2(24.0, 24.0)),
                );
                if newer {
                    ui.painter().circle_filled(
                        egui::pos2(rect.right() - 9.0, rect.top() + 9.0),
                        4.0,
                        p.accent,
                    );
                }
                if resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(crate::i18n::t(tr!("Settings")).into_owned())
                    .clicked()
                {
                    cx.go(Screen::Settings);
                }
            });
        },
    );
    if let Some(rect) = net {
        rail_network(ui, a, rect);
    }
    let radius = egui::CornerRadius::same(12);
    ui.painter().rect(
        sync,
        radius,
        p.surface,
        Stroke::new(1.0, p.line),
        egui::StrokeKind::Inside,
    );
    sync_card(ui, a, status, sync, rail, compact);
}

/// The sync card's height in the wide menu (BUG-155, the designer's B).
pub(crate) const SYNC_WIDE: f32 = 64.0;
/// The rail's square.
pub(crate) const SYNC_RAIL: f32 = 52.0;
/// Below [`LOW_MENU`], wide or rail.
pub(crate) const SYNC_LOW: f32 = 44.0;
/// The words' column in the wide menu: from 10 right of the mark to 10 from the card's edge.
pub(crate) const WORDS: f32 = 156.0;

/// What the sync card's bar shows (BUG-155).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Bar {
    /// None: synced.
    None,
    /// The stage's part done, 0..1.
    Part(f32),
    /// A short run along the track: how much is left is not known (nodes, Platform).
    Running,
    /// Stopped where it stood, in `danger`.
    Stopped(f32),
}

/// The sync card's words: its two lines, and the whole of them for the tooltip and the reader.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct CardWords {
    /// The card's first line: a stage's short name, «Stopped», «Synced».
    pub title: String,
    /// The card's second line, cut with «…» when longer than the column.
    pub detail: String,
    /// The second line without «· ChainLock», for a «block {}» too long with it (the designer's
    /// rule: no «…» there).
    pub detail_short: Option<String>,
    /// The title in `danger`.
    pub stopped: bool,
    /// The title in the warning colour (SPV 2a: no new blocks for 30–60 min).
    pub warning: bool,
    /// The tooltip's and the reader's title: a stage's full name with its percent.
    pub tip_title: String,
    /// The tooltip's and the reader's second line, whole.
    pub tip_detail: String,
    pub bar: Bar,
}

/// When the tip came, for «updated {}» (BUG-155 AC-A9): the time today, else the date as History
/// gives it and the time.
pub(crate) fn tip_time(at: evolite_api::Timestamp, now: chrono::DateTime<chrono::Local>) -> String {
    let clock = crate::fiat_lines::clock(at);
    let day = chrono::DateTime::from_timestamp(at, 0)
        .map(|t| t.with_timezone(&chrono::Local).date_naive());
    if day == Some(now.date_naive()) {
        clock
    } else {
        format!("{}, {clock}", text::date_short(at, now))
    }
}

/// `text` in the interface language in reading order: the card's lines are put together from
/// pieces and turned to visual order once, where they are drawn (BUG-153: pieces turned already
/// and turned again swap places in Arabic).
fn reading(text: &str) -> std::borrow::Cow<'_, str> {
    crate::i18n::t_in(crate::i18n::current(), text)
}

/// SPV item 2a, the analyst's strings: no new block for `minutes` (30 or more, by both clocks).
/// Under an hour «No new blocks» in the warning colour, «block {n} · {m} min ago»; over it the
/// same title in `danger`, «block {n} · over {h} hours», the tooltip saying the network may have
/// stopped and what to do. The card's line may drop «block {n} ·» (`detail_short`).
fn stale_words(status: &evolite_api::SyncInfo, minutes: u32) -> CardWords {
    let block = reading(&format!(
        tr!("block {}"),
        group_thousands(status.tip_height as i64)
    ))
    .into_owned();
    if minutes < 60 {
        let ago = reading(&format!(tr!("{} min ago"), minutes)).into_owned();
        return CardWords {
            title: tr!("No new blocks").to_string(),
            detail: format!("{block} · {ago}"),
            detail_short: Some(ago),
            stopped: false,
            warning: true,
            tip_title: tr!("No new blocks").to_string(),
            tip_detail: reading(&text::plural_in(
                minutes,
                tr!(
                    "No new blocks for {} minute. The nodes evolite is connected to may have fallen behind the network: a block usually comes every 2–3 minutes."
                ),
                tr!(
                    "No new blocks for {} minutes. The nodes evolite is connected to may have fallen behind the network: a block usually comes every 2–3 minutes."
                ),
            ))
            .into_owned(),
            bar: Bar::None,
        };
    }
    let hours = minutes / 60;
    let over = reading(&text::plural_in(
        hours,
        tr!("over {} hour"),
        tr!("over {} hours"),
    ))
    .into_owned();
    let body = reading(&text::plural_in(
        hours,
        tr!(
            "No new blocks for over {} hour. Either the nodes evolite is connected to have fallen behind, or the network itself has stopped. The balance and received coins may be out of date."
        ),
        tr!(
            "No new blocks for over {} hours. Either the nodes evolite is connected to have fallen behind, or the network itself has stopped. The balance and received coins may be out of date."
        ),
    ))
    .into_owned();
    let what = reading(tr!(
        "You can restart evolite or set your own node: Settings › Network and nodes"
    ))
    .into_owned();
    CardWords {
        title: tr!("No new blocks").to_string(),
        detail: format!("{block} · {over}"),
        detail_short: Some(over),
        stopped: true,
        warning: false,
        tip_title: tr!("The network is not sending new blocks").to_string(),
        tip_detail: format!("{body}\n{what}"),
        bar: Bar::None,
    }
}

/// The sync card's words (BUG-155, the analyst's AC-A1…A9). `platform`: the mark goes for
/// Platform ([`MarkClock::platform`]).
pub(crate) fn card_words(
    status: &evolite_api::SyncInfo,
    platform: bool,
    now: chrono::DateTime<chrono::Local>,
) -> CardWords {
    let stopped = |detail: String, tip_detail: String, at: f32| CardWords {
        title: tr!("Stopped").to_string(),
        detail,
        detail_short: None,
        stopped: true,
        warning: false,
        tip_title: tr!("The sync stopped").to_string(),
        tip_detail,
        bar: Bar::Stopped(at),
    };
    if let Some(what_to_do) = &status.folder_refused {
        return stopped(
            tr!("a folder needs fixing").to_string(),
            reading(what_to_do).into_owned(),
            0.5,
        );
    }
    if let Some(why) = &status.error {
        let line = format!(
            tr!("{} · resync in Settings › Network and nodes"),
            reading(why)
        );
        return stopped(line.clone(), line, status.progress.unwrap_or(0.5));
    }
    // SPV item 2a: no new block for a while — in place of «Synced» (the analyst's strings).
    if status.stage == SyncStage::Done
        && let Some(minutes) = status.stale_minutes
    {
        return stale_words(status, minutes);
    }
    if status.stage == SyncStage::Done && platform {
        // Core done, Platform catching up (BUG-137): «Platform» and three works at most on the
        // card (the designer, BUG-155 r1), «Sync» with all of them in the tooltip.
        let all: Vec<String> = status
            .platform
            .iter()
            .map(|w| reading(platform_work(*w)).into_owned())
            .collect();
        return CardWords {
            title: "Platform".to_string(),
            detail: platform_words(&status.platform),
            detail_short: None,
            stopped: false,
            warning: false,
            // Whatever goes (contacts, tokens…): the general «Sync» (the designer, BUG-155).
            tip_title: tr!("Sync").to_string(),
            tip_detail: format!(tr!("Platform: {works}"), works = all.join(" · ")),
            bar: Bar::Running,
        };
    }
    if status.stage == SyncStage::Done {
        let block = format!(tr!("block {}"), group_thousands(status.tip_height as i64));
        let mut tip = vec![reading(&block).into_owned()];
        if status.chainlocked {
            tip.push("ChainLock".to_owned());
        }
        if let Some(at) = status.tip_at {
            tip.push(reading(&format!(tr!("updated {}"), tip_time(at, now))).into_owned());
        }
        return CardWords {
            title: tr!("Synced").to_string(),
            detail: block.clone() + if status.chainlocked { CHAINLOCKED } else { "" },
            detail_short: status.chainlocked.then_some(block),
            stopped: false,
            warning: false,
            tip_title: tr!("Synced").to_string(),
            // In Arabic each piece on its own (BUG-166): the time stood with «ChainLock», a Latin run
            // beside it, and read before «updated» (as BUG-118, BUG-119).
            tip_detail: if crate::i18n::rtl() {
                tip.iter()
                    .map(|p| crate::i18n::own(p))
                    .collect::<Vec<_>>()
                    .join(" · ")
            } else {
                tip.join(" · ")
            },
            bar: Bar::None,
        };
    }
    let rest = match status.minutes_left {
        Some(m) => format!(tr!("about {} min left"), m.max(1)),
        None => text::plural(status.core_peers, tr!("Core node"), tr!("Core nodes")),
    };
    let rest = reading(&rest).into_owned();
    let (detail, bar) = match status.progress {
        Some(part) => (
            format!("{} % · {rest}", (part * 100.0).round() as i32),
            Bar::Part(part.clamp(0.0, 1.0)),
        ),
        None => (rest.clone(), Bar::Running),
    };
    CardWords {
        title: text::sync_short(status.stage).to_string(),
        detail,
        detail_short: None,
        stopped: false,
        warning: false,
        tip_title: text::sync_title(status),
        tip_detail: rest,
        bar,
    }
}

/// One line of the card's words at `pos`, cut with «…» at `width` (in Arabic from its end in
/// reading order, BUG-118).
fn card_line(
    ui: &Ui,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    pos: egui::Pos2,
    width: f32,
    row: f32,
) {
    let mut format = egui::TextFormat::simple(font.clone(), color);
    format.line_height = Some(row);
    let mut job = egui::text::LayoutJob::single_section(
        crate::components::cut_to(ui.ctx(), text, &font, width, 1),
        format,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos, galley, color);
}

/// Where the card's parts stand (BUG-155, the designer's table): the mark, the words' column and
/// the bar, for a card `sync` of the wide menu or the rail.
pub(crate) fn card_layout(
    sync: egui::Rect,
    rail: bool,
    compact: bool,
) -> (egui::Rect, egui::Pos2, egui::Rect) {
    let mark = if rail {
        egui::Rect::from_center_size(sync.center(), vec2(28.0, 28.0))
    } else {
        egui::Rect::from_min_size(
            egui::pos2(sync.left() + 12.0, sync.center().y - 14.0),
            vec2(28.0, 28.0),
        )
    };
    let words = egui::pos2(
        mark.right() + 10.0,
        sync.top() + if compact { 13.0 } else { 14.0 },
    );
    let bar = if rail {
        egui::Rect::from_min_size(
            egui::pos2(
                sync.left() + 10.0,
                mark.bottom() + if compact { 3.0 } else { 5.0 },
            ),
            vec2(sync.width() - 20.0, 3.0),
        )
    } else if compact {
        egui::Rect::from_min_size(egui::pos2(words.x, sync.top() + 37.0), vec2(WORDS, 3.0))
    } else {
        egui::Rect::from_min_size(
            egui::pos2(sync.left() + 12.0, sync.bottom() - 7.0),
            vec2(sync.width() - 24.0, 3.0),
        )
    };
    (mark, words, bar)
}

#[cfg(test)]
fn card_id() -> egui::Id {
    egui::Id::new("evolite-sync-card-shown")
}

/// What the sync card drew in the last frame (for tests): its rect, the words' two lines as
/// drawn (before the cut), the bar and its rect.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the app's kittests (dev-tools, the demo core)"
)]
#[derive(Clone, Debug)]
pub(crate) struct CardShown {
    pub card: egui::Rect,
    pub mark: egui::Rect,
    pub title: Option<(String, egui::Rect)>,
    pub detail: Option<(String, egui::Rect)>,
    pub bar: Bar,
    pub bar_rect: egui::Rect,
    pub words: CardWords,
}

/// The sync card in the last frame (for tests); `None`: no menu drawn yet.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub(crate) fn card_shown(ctx: &egui::Context) -> Option<CardShown> {
    ctx.data(|d| d.get_temp::<CardShown>(card_id()))
}

/// The sync card (BUG-155, the designer's B + O7): the mark and the words in the middle of the
/// card's height in every state, a 3 px bar at its foot while sync goes; in the rail the mark
/// alone, centred, the bar under it. The whole words on hover and on keyboard focus in every
/// size, and for the screen reader.
fn sync_card(
    ui: &mut Ui,
    a: &mut Assets,
    status: &evolite_api::SyncInfo,
    sync: egui::Rect,
    rail: bool,
    compact: bool,
) {
    let p = a.p;
    let id = ui.id().with("sync-card");
    let (mark, at, bar_rect) = card_layout(sync, rail, compact);
    // The same mark in every size (BUG-137 AC-D1).
    let clock = sync_mark(ui, a, status, mark);
    let words = card_words(
        &words_status(status, &clock),
        clock.platform(),
        chrono::Local::now(),
    );
    #[cfg(test)]
    let (mut title_shown, mut detail_shown) = (None, None);
    if !rail {
        let title_font = sans(13.0, Weight::SemiBold);
        let color = if words.stopped {
            p.danger
        } else if words.warning {
            p.accent
        } else {
            p.text
        };
        card_line(ui, &words.title, title_font.clone(), color, at, WORDS, 18.0);
        #[cfg(test)]
        {
            let w = crate::components::text_width(ui, &words.title, title_font);
            title_shown = Some((
                words.title.clone(),
                egui::Rect::from_min_size(at, vec2(w, 18.0)),
            ));
        }
        if !compact {
            let font = sans(12.0, Weight::Regular);
            // «block {} · ChainLock» too long for the column: «block {}», never cut (the designer).
            let detail = match &words.detail_short {
                Some(short)
                    if crate::components::text_width(ui, &words.detail, font.clone()) > WORDS =>
                {
                    short
                }
                _ => &words.detail,
            };
            let pos = egui::pos2(at.x, sync.top() + 34.0);
            card_line(ui, detail, font.clone(), p.muted, pos, WORDS, 16.0);
            #[cfg(test)]
            {
                let w = crate::components::text_width(ui, detail, font);
                detail_shown = Some((
                    detail.clone(),
                    egui::Rect::from_min_size(pos, vec2(w, 16.0)),
                ));
            }
        }
    }
    sync_bar(
        ui,
        a,
        id.with("bar"),
        bar_rect,
        words.bar,
        status.stage,
        rail,
    );
    #[cfg(test)]
    ui.data_mut(|d| {
        d.insert_temp(
            card_id(),
            CardShown {
                card: sync,
                mark,
                title: title_shown,
                detail: detail_shown,
                bar: words.bar,
                bar_rect,
                words: words.clone(),
            },
        )
    });
    card_status(ui, a, status, sync, id, &clock, &words, true);
}

/// The bar's way to the end when sync is done, seconds (as the mark's, `TO_DONE`).
const BAR_GONE: f64 = 0.4;
/// Before it goes, the fill runs on from where it was to the end, seconds (the designer's review
/// of BUG-155: it jumped from 45 % to the end).
const BAR_TO_END: f64 = 0.3;
/// A new stage's fill fades in, seconds: no jump back from the last stage's.
const BAR_NEW: f64 = 0.2;
/// One run of the short piece along the track, seconds.
const BAR_RUN: f64 = 1.4;

/// The bar `t` seconds after sync was done, coming from `last`: how far its fill reaches and how
/// opaque it is; `None` once gone. The fill runs on from where it was to the end, then fades
/// with the track (the designer's review of BUG-155: it jumped from 45 % to the end).
pub(crate) fn bar_going(last: Bar, t: f64) -> Option<(f32, f32)> {
    let from = match last {
        Bar::Part(part) | Bar::Stopped(part) => part,
        Bar::Running | Bar::None => 1.0,
    };
    if t < BAR_TO_END {
        let k = (t / BAR_TO_END) as f32;
        Some((from + (1.0 - from) * (1.0 - (1.0 - k).powi(3)), 1.0))
    } else if t < BAR_TO_END + BAR_GONE {
        Some((1.0, 1.0 - ((t - BAR_TO_END) / BAR_GONE) as f32))
    } else {
        None
    }
}

/// The bar's state from frame to frame.
#[derive(Clone, Copy)]
struct BarClock {
    /// The bar last drawn and the stage it was for.
    last: Bar,
    stage: SyncStage,
    /// When the stage began, for the fade of a new stage's fill.
    since: f64,
    /// Since when the bar goes, sync being done, and what it was.
    gone: Option<(f64, Bar)>,
}

/// The sync card's bar (BUG-155): the track in `line`, the fill in `accent` (`danger` at a stop);
/// a short piece running round while how much is left is not known. The frame mirror turns it
/// for Arabic (right to left). With the system's animations off nothing moves.
pub(crate) fn sync_bar(
    ui: &mut Ui,
    a: &Assets,
    id: egui::Id,
    rect: egui::Rect,
    bar: Bar,
    stage: SyncStage,
    rail: bool,
) {
    let p = a.p;
    let now = ui.input(|i| i.time);
    let moving = crate::motion::system_on(ui.ctx());
    let mut clock = ui.data(|d| d.get_temp::<BarClock>(id)).unwrap_or(BarClock {
        last: bar,
        stage,
        since: f64::NEG_INFINITY,
        gone: None,
    });
    if clock.stage != stage {
        clock.stage = stage;
        clock.since = now;
    }
    if bar == Bar::None && clock.last != Bar::None && moving {
        clock.gone = Some((now, clock.last));
    }
    if bar != Bar::None {
        clock.gone = None;
    }
    clock.last = bar;
    ui.data_mut(|d| d.insert_temp(id, clock));
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(2);
    let fill = |painter: &egui::Painter, from: f32, to: f32, color: egui::Color32| {
        let r = egui::Rect::from_min_max(
            egui::pos2(rect.left() + rect.width() * from, rect.top()),
            egui::pos2(rect.left() + rect.width() * to, rect.bottom()),
        );
        painter.rect_filled(r, radius, color);
    };
    let mut again = false;
    match bar {
        Bar::None => {
            // The fill runs on to the end, then goes with the track (as the mark, `TO_DONE`).
            if let Some((at, last)) = clock.gone
                && let Some((to, alpha)) = bar_going(last, now - at)
            {
                let color = if matches!(last, Bar::Stopped(_)) {
                    p.danger
                } else {
                    p.accent
                };
                painter.rect_filled(rect, radius, p.line.gamma_multiply(alpha));
                fill(painter, 0.0, to, color.gamma_multiply(alpha));
                again = true;
            }
        }
        Bar::Part(part) => {
            painter.rect_filled(rect, radius, p.line);
            let fade = if moving && now - clock.since < BAR_NEW {
                again = true;
                ((now - clock.since) / BAR_NEW) as f32
            } else {
                1.0
            };
            fill(painter, 0.0, part, p.accent.gamma_multiply(fade));
        }
        Bar::Stopped(at) => {
            painter.rect_filled(rect, radius, p.line);
            fill(painter, 0.0, at.clamp(0.0, 1.0), p.danger);
        }
        Bar::Running => {
            painter.rect_filled(rect, radius, p.line);
            let piece = (if rail { 10.0 } else { 40.0 }) / rect.width();
            let start = if moving {
                again = true;
                let k = ((now % BAR_RUN) / BAR_RUN) as f32;
                k * (1.0 + piece) - piece
            } else {
                0.0
            };
            fill(painter, start.max(0.0), (start + piece).min(1.0), p.accent);
        }
    }
    if again {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 30.0));
    }
}

/// The rail's network on mainnet (FEAT-0037 N3): a globe in a `danger` frame, «Mainnet · beta»
/// on hover and on keyboard focus, and for the screen reader.
fn rail_network(ui: &mut Ui, a: &mut Assets, rect: egui::Rect) {
    let p = a.p;
    let words = crate::i18n::t(tr!("Mainnet · beta")).into_owned();
    let resp = ui.interact(
        rect,
        ui.id().with("rail-network"),
        egui::Sense::hover() | egui::Sense::FOCUSABLE,
    );
    // Its name, not only its value: a Tab stop says what it is (the Tab sweep).
    crate::components::named(&resp, egui::WidgetType::Label, true, tr!("Mainnet · beta"));
    let spoken = crate::i18n::spoken(tr!("Mainnet · beta"));
    ui.ctx()
        .accesskit_node_builder(resp.id, |node| node.set_label(spoken));
    ui.painter().rect(
        rect,
        10,
        egui::Color32::TRANSPARENT,
        Stroke::new(1.0, p.danger),
        egui::StrokeKind::Inside,
    );
    a.icons.icon("mainnet", p.danger, &p, 20.0).paint_at(
        ui,
        egui::Rect::from_center_size(rect.center(), vec2(20.0, 20.0)),
    );
    if resp.has_focus() {
        resp.show_tooltip_text(words);
    } else {
        resp.on_hover_text(words);
    }
}

/// The block's left edge on the bar's left end and at home, in the logo's 64 × 64 (BUG-75).
const BLOCK_FROM: f32 = 12.9;
const BLOCK_HOME: f32 = 40.15;
/// Where the block stands still when the sync stopped: halfway.
const BLOCK_STOPPED: f32 = 26.525;
/// One cycle of «the block finds its place», seconds.
const CYCLE: f64 = 2.8;
/// From any point of the cycle to the full logo, seconds.
const TO_DONE: f64 = 0.4;

/// Where the moving block is at `t` (0..1) of its cycle: its left edge in the logo's 64 × 64
/// and its opacity. It shows at the bar's left end, rides to its home (ease-out cubic), stays
/// and fades (the designer's curve, BUG-75).
pub(crate) fn block_at(t: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0);
    if t < 0.12 {
        (BLOCK_FROM, t / 0.12)
    } else if t < 0.62 {
        let k = (t - 0.12) / 0.5;
        let eased = 1.0 - (1.0 - k).powi(3);
        (BLOCK_FROM + (BLOCK_HOME - BLOCK_FROM) * eased, 1.0)
    } else if t < 0.86 {
        (BLOCK_HOME, 1.0)
    } else {
        (BLOCK_HOME, 1.0 - (t - 0.86) / 0.14)
    }
}

/// Where the block stands with the system's animations off: as far along the bar as the stage
/// has gone, in steps of 10 % (0…90 %; at the end the full logo); at the start without a count.
pub(crate) fn block_still(progress: Option<f32>) -> f32 {
    let part = progress.map_or(0.0, |p| ((p * 10.0).floor() / 10.0).clamp(0.0, 0.9));
    BLOCK_FROM + (BLOCK_HOME - BLOCK_FROM) * part
}

/// What the rail's mark shows.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum RailMark {
    /// The full logo, still.
    Done,
    /// «e» and bar (`dim`: at 55 %), the block's home at 22 % and the block at `x`, in `danger`
    /// when `stopped`, else in the brand colour, at `alpha`.
    Block {
        dim: bool,
        x: f32,
        alpha: f32,
        stopped: bool,
    },
}

/// Platform work moves the mark once it has gone this long (BUG-137): no flash for a short one.
const PLATFORM_AFTER: f64 = 0.6;
/// A pause shorter than this between two works does not end a cycle.
const PLATFORM_GAP: f64 = 1.0;

/// The sync mark's clock, kept from frame to frame.
#[derive(Clone, Copy, Default)]
pub(crate) struct MarkClock {
    /// When the cycle that runs began; `None`: none runs.
    cycle: Option<f64>,
    /// The way to the full logo: when it began, and where the block was then and how bright.
    to_done: Option<(f64, f32, f32)>,
    /// Platform work (BUG-137): since when it goes without a pause of [`PLATFORM_GAP`], and when
    /// it was last seen.
    platform_since: Option<f64>,
    platform_seen: Option<f64>,
    /// The mark moves (or stands dim) for Platform, with Core done: the words say so.
    for_platform: bool,
    /// The kinds of Platform work last seen (a bit each, [`evolite_api::PlatformWork::ALL`]'s order),
    /// for the words while a cycle finishes after the work.
    last_works: u8,
}

impl MarkClock {
    /// The mark goes for Platform now, Core being done (the words: «Sync · Platform»).
    pub(crate) fn platform(&self) -> bool {
        self.for_platform
    }

    /// The kinds of Platform work last seen.
    pub(crate) fn last_works(&self) -> Vec<evolite_api::PlatformWork> {
        evolite_api::PlatformWork::ALL
            .into_iter()
            .filter(|w| self.last_works & (1 << *w as u8) != 0)
            .collect()
    }

    /// Notes the Platform work of this frame; whether it has gone [`PLATFORM_AFTER`] now.
    fn platform_long(&mut self, working: bool, now: f64) -> bool {
        if working {
            self.platform_since.get_or_insert(now);
            self.platform_seen = Some(now);
        } else if self
            .platform_seen
            .is_some_and(|seen| now - seen >= PLATFORM_GAP)
        {
            self.platform_since = None;
            self.platform_seen = None;
        }
        working
            && self
                .platform_since
                .is_some_and(|since| now - since >= PLATFORM_AFTER - 1e-9)
    }

    /// A cycle begun for Platform is owed its end: the work was seen less than [`PLATFORM_GAP`]
    /// ago, or the cycle it was last seen in has not ended.
    fn platform_owed(&self, now: f64) -> bool {
        let (Some(start), Some(seen)) = (self.cycle, self.platform_seen) else {
            return false;
        };
        if now - seen < PLATFORM_GAP {
            return true;
        }
        let end = start + ((seen - start) / CYCLE).floor() * CYCLE + CYCLE;
        now < end
    }
}

/// What the rail's mark shows at `now` (seconds) and whether it moves, so another frame is
/// wanted soon. `moving`: the system lets things move.
pub(crate) fn rail_mark_now(
    clock: &mut MarkClock,
    status: &evolite_api::SyncInfo,
    moving: bool,
    now: f64,
) -> (RailMark, bool) {
    if status.error.is_some() {
        *clock = MarkClock::default();
        let mark = RailMark::Block {
            dim: false,
            x: BLOCK_STOPPED,
            alpha: 1.0,
            stopped: true,
        };
        return (mark, false);
    }
    let core = status.stage != SyncStage::Done;
    // BUG-137: Platform work too, once it has gone 0.6 s, or while the cycle it began is owed
    // its end (moving only: still, there is no cycle to finish).
    if !status.platform.is_empty() {
        clock.last_works = status
            .platform
            .iter()
            .fold(0, |bits, w| bits | 1 << *w as u8);
    }
    let platform = clock.platform_long(!status.platform.is_empty(), now)
        || (!core && moving && clock.for_platform && clock.platform_owed(now));
    clock.for_platform = !core && platform;
    if !core && !platform {
        // From wherever the cycle was, the block rides home or shows there in 400 ms; a cycle
        // that ran to its end for Platform is at home already, faded.
        if let Some(start) = clock.cycle.take()
            && moving
        {
            let ended = clock.platform_seen.is_some();
            let (x, alpha) = if ended {
                (BLOCK_HOME, 0.0)
            } else {
                block_at((((now - start) % CYCLE) / CYCLE) as f32)
            };
            clock.to_done = Some((now, x, alpha));
        }
        if let Some((at, x, alpha)) = clock.to_done
            && moving
            && now - at < TO_DONE
        {
            let k = ((now - at) / TO_DONE) as f32;
            let eased = 1.0 - (1.0 - k).powi(3);
            let mark = RailMark::Block {
                dim: false,
                x: x + (BLOCK_HOME - x) * eased,
                alpha: alpha + (1.0 - alpha) * k,
                stopped: false,
            };
            return (mark, true);
        }
        clock.to_done = None;
        return (RailMark::Done, false);
    }
    clock.to_done = None;
    if !moving {
        // A new cycle starts from its beginning once things may move again.
        clock.cycle = None;
        let mark = RailMark::Block {
            dim: true,
            // Platform has no percent: the bar's start.
            x: block_still(if core { status.progress } else { None }),
            alpha: 1.0,
            stopped: false,
        };
        return (mark, false);
    }
    let start = *clock.cycle.get_or_insert(now);
    let (x, alpha) = block_at((((now - start) % CYCLE) / CYCLE) as f32);
    let mark = RailMark::Block {
        dim: false,
        x,
        alpha,
        stopped: false,
    };
    (mark, true)
}

/// A kind of Platform work, as the words name it (BUG-137).
fn platform_work(work: evolite_api::PlatformWork) -> &'static str {
    use evolite_api::PlatformWork::*;
    match work {
        Identities => tr!("identities"),
        Names => tr!("names"),
        Credits => tr!("credits"),
        Contacts => tr!("contacts"),
        Tokens => tr!("tokens"),
        Pool => tr!("pool"),
    }
}

/// What Platform catches up on, for the card (BUG-137): three at most, then «and N more».
fn platform_words(works: &[evolite_api::PlatformWork]) -> String {
    let names: Vec<String> = works
        .iter()
        .map(|w| reading(platform_work(*w)).into_owned())
        .collect();
    let mut shown: Vec<String> = names.iter().take(3).cloned().collect();
    if names.len() > 3 {
        shown.push(reading(&format!(tr!("and {} more"), names.len() - 3)).into_owned());
    }
    shown.join(" · ")
}

#[cfg(test)]
fn rail_mark_id() -> egui::Id {
    egui::Id::new("evolite-rail-mark")
}

/// What the rail's mark showed in the last frame and whether it asked for another soon (for
/// tests); `None`: no rail drawn yet.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub(crate) fn rail_mark_shown(ctx: &egui::Context) -> Option<(RailMark, bool)> {
    ctx.data(|d| d.get_temp::<(RailMark, bool)>(rail_mark_id()))
}

/// The one clock of every sync mark (BUG-163 AC-D6): the menu's and the first sync's (O6) go on
/// from the same point of the cycle, from one screen to the other.
fn mark_clock_id() -> egui::Id {
    egui::Id::new("evolite-sync-mark-clock")
}

/// When the running cycle of the sync marks began (for tests); `None`: none runs.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub(crate) fn mark_cycle(ctx: &egui::Context) -> Option<f64> {
    ctx.data(|d| d.get_temp::<MarkClock>(mark_clock_id()))
        .and_then(|c| c.cycle)
}

/// The sync mark at `mark`'s size, in every size of the menu (BUG-137) and on the first sync
/// (O6, 48 px, BUG-163): «the block finds its place» while Core or Platform catches up, the full
/// logo once done, the danger block at a stop. Asks for frames only while it moves. Its clock,
/// for the words: one for every mark.
pub(crate) fn sync_mark(
    ui: &mut Ui,
    a: &mut Assets,
    status: &evolite_api::SyncInfo,
    mark: egui::Rect,
) -> MarkClock {
    let p = a.p;
    let size = mark.width();
    let now = ui.input(|i| i.time);
    let moving = crate::motion::system_on(ui.ctx());
    let id = mark_clock_id();
    let mut clock = ui.data(|d| d.get_temp::<MarkClock>(id)).unwrap_or_default();
    let (shown, again) = rail_mark_now(&mut clock, status, moving, now);
    ui.data_mut(|d| d.insert_temp(id, clock));
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(rail_mark_id(), (shown, again)));
    match shown {
        RailMark::Done => a.icons.sync_mark(3, &p, size).paint_at(ui, mark),
        RailMark::Block {
            dim,
            x,
            alpha,
            stopped,
        } => {
            a.icons.sync_base(dim, &p, size).paint_at(ui, mark);
            let unit = mark.width() / 64.0;
            let block =
                egui::Rect::from_min_size(mark.min + vec2(x, 23.5) * unit, vec2(12.0, 12.0) * unit);
            let color = if stopped { p.danger } else { p.brand };
            ui.painter()
                .rect_filled(block, unit, color.gamma_multiply(alpha));
        }
    }
    // Frames only while it moves: none once done, stopped or still.
    if again {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 30.0));
    }
    clock
}

/// What the words say of Platform: what goes now, or, while a cycle finishes after it, what went
/// last.
pub(crate) fn words_status(
    status: &evolite_api::SyncInfo,
    clock: &MarkClock,
) -> evolite_api::SyncInfo {
    let mut shown = status.clone();
    if clock.platform() && shown.platform.is_empty() {
        shown.platform = clock.last_works();
    }
    shown
}

/// The sync card's whole words (BUG-155 AC-A3, A4): a tooltip on hover and on keyboard focus in
/// every size, and one name for the screen reader — a polite status that changes with the stage,
/// the end and a stop, not with each percent (BUG-137 AC-D7). `tooltip`: off where the words
/// stand whole already (the first sync, BUG-163).
#[allow(
    clippy::too_many_arguments,
    reason = "the card's place, its clock and words, each used once"
)]
pub(crate) fn card_status(
    ui: &mut Ui,
    a: &mut Assets,
    status: &evolite_api::SyncInfo,
    sync: egui::Rect,
    id: egui::Id,
    clock: &MarkClock,
    words: &CardWords,
    tooltip: bool,
) {
    let p = a.p;
    let platform = clock.platform();
    let stopped = words.stopped;
    let title = crate::i18n::t(&words.tip_title).into_owned();
    let detail = crate::i18n::t(&words.tip_detail).into_owned();
    // The reader is told only of a new stage, the end and a stop, and of Platform's turn (BUG-137
    // AC-D7): the status is a polite live region for a second after one, quiet while the percent,
    // the minutes left and the Platform works move on.
    // SPV 2a: entering «No new blocks», or its hour, is told too.
    let stale = status.stale_minutes.map(|m| m >= 60);
    let key = (status.stage, status.error.is_some(), platform, stale);
    let now = ui.input(|i| i.time);
    let changed_id = id.with("changed");
    let changed =
        match ui.data(|d| d.get_temp::<((SyncStage, bool, bool, Option<bool>), f64)>(changed_id)) {
            Some((was, at)) if was == key => at,
            _ => {
                ui.data_mut(|d| d.insert_temp(changed_id, (key, now)));
                now
            }
        };
    let live = if now - changed < 1.0 {
        // One frame when the second is over, for a still mark too: the status goes quiet then,
        // not at the next snapshot (review r1 nit).
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(
                (1.0 - (now - changed)).max(0.0),
            ));
        egui::accesskit::Live::Polite
    } else {
        egui::accesskit::Live::Off
    };
    // Platform's works change often: the reader is told «Sync · Platform», the
    // works are in the tooltip (BUG-137 AC-T7: a second work does not change the name).
    let spoken = if platform {
        format!("{title} · Platform")
    } else {
        format!("{title} · {detail}")
    };
    let resp = ui.interact(sync, id, egui::Sense::hover() | egui::Sense::FOCUSABLE);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.set_role(egui::accesskit::Role::Status);
        node.set_label(spoken.clone());
        node.set_live(live);
    });
    let words = |ui: &mut Ui| {
        ui.set_max_width(280.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        let color = if stopped { p.danger } else { p.text };
        ui.label(
            egui::RichText::new(&title)
                .font(sans(13.0, Weight::SemiBold))
                .color(color),
        );
        ui.label(
            egui::RichText::new(&detail)
                .font(sans(12.0, Weight::Regular))
                .color(p.muted),
        );
    };
    if !tooltip {
        return;
    }
    if resp.has_focus() {
        resp.show_tooltip_ui(words);
    } else {
        resp.on_hover_ui(words);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syncing(stage: SyncStage, progress: Option<f32>) -> evolite_api::SyncInfo {
        evolite_api::SyncInfo {
            stage,
            progress,
            tip_height: 1_284_112,
            headers_height: 0,
            filters_height: 0,
            core_peers: 3,
            platform_nodes: 0,
            minutes_left: None,
            chainlocked: false,
            error: None,
            folder_refused: None,
            disk_bytes: None,
            core_node: None,
            core_node_next: None,
            core_nodes: Vec::new(),
            dapi_nodes: Vec::new(),
            platform: Vec::new(),
            tip_at: None,
            stale_minutes: None,
            contacts_rescan: false,
        }
    }

    /// AC-D1: the designer's curve — shows at the bar's left end, rides home eased, stays, fades.
    #[test]
    fn the_block_follows_the_curve() {
        assert_eq!(block_at(0.0), (BLOCK_FROM, 0.0));
        assert_eq!(block_at(0.06), (BLOCK_FROM, 0.5));
        let (x, alpha) = block_at(0.37);
        assert_eq!(alpha, 1.0);
        // Ease-out: past the middle of the way at the middle of the time.
        assert!(x > (BLOCK_FROM + BLOCK_HOME) / 2.0, "{x}");
        assert_eq!(block_at(0.7), (BLOCK_HOME, 1.0));
        let (x, alpha) = block_at(0.93);
        assert_eq!(x, BLOCK_HOME);
        assert!((alpha - 0.5).abs() < 1e-4, "{alpha}");
        assert!(
            block_at(1.0).1.abs() < 1e-4,
            "gone at the end: no flash at the next start"
        );
    }

    /// AC-D4: still, the block stands by the stage's percent in steps of 10 %, 0…90 %.
    #[test]
    fn still_the_block_stands_by_the_percent() {
        assert_eq!(block_still(None), BLOCK_FROM);
        assert_eq!(block_still(Some(0.0)), BLOCK_FROM);
        let step = (BLOCK_HOME - BLOCK_FROM) / 10.0;
        assert!((block_still(Some(0.64)) - (BLOCK_FROM + 6.0 * step)).abs() < 1e-4);
        assert!((block_still(Some(1.0)) - (BLOCK_FROM + 9.0 * step)).abs() < 1e-4);
    }

    /// AC-D1, AC-D2, AC-D4: moves while syncing and wants frames; once done, 400 ms home, then
    /// the full logo and no frames; still with the system's animations off; a new sync starts
    /// its cycle from the beginning.
    #[test]
    fn the_mark_moves_only_while_it_syncs() {
        let mut clock = MarkClock::default();
        let going = syncing(SyncStage::Filters, Some(0.64));
        let (mark, again) = rail_mark_now(&mut clock, &going, true, 10.0);
        assert!(again, "a frame soon while it moves");
        assert_eq!(
            mark,
            RailMark::Block {
                dim: false,
                x: BLOCK_FROM,
                alpha: 0.0,
                stopped: false
            },
            "the cycle starts at its beginning"
        );
        let (mark, _) = rail_mark_now(&mut clock, &going, true, 10.0 + 0.7 * CYCLE);
        assert!(matches!(mark, RailMark::Block { x, .. } if x == BLOCK_HOME));
        let done = syncing(SyncStage::Done, None);
        let (mark, again) = rail_mark_now(&mut clock, &done, true, 20.0);
        assert!(again, "on its way to the full logo");
        assert!(matches!(mark, RailMark::Block { dim: false, .. }));
        let (mark, again) = rail_mark_now(&mut clock, &done, true, 20.0 + TO_DONE + 0.01);
        assert_eq!(
            (mark, again),
            (RailMark::Done, false),
            "done: still, no frames"
        );
        let (mark, again) = rail_mark_now(&mut clock, &done, true, 30.0);
        assert_eq!((mark, again), (RailMark::Done, false));
        // A resync: from the beginning again, no flash.
        let (mark, _) = rail_mark_now(&mut clock, &going, true, 40.0);
        assert!(matches!(mark, RailMark::Block { alpha, .. } if alpha == 0.0));
        // The system's animations off: still, dim, by the percent; done at once.
        let (mark, again) = rail_mark_now(&mut clock, &going, false, 41.0);
        assert!(!again, "no frames with the animations off");
        assert_eq!(
            mark,
            RailMark::Block {
                dim: true,
                x: block_still(Some(0.64)),
                alpha: 1.0,
                stopped: false
            }
        );
        let (mark, again) = rail_mark_now(&mut clock, &done, false, 42.0);
        assert_eq!(
            (mark, again),
            (RailMark::Done, false),
            "full brightness at once"
        );
    }

    /// AC-D3: a stop — the danger block halfway, still.
    #[test]
    fn a_stop_stands_still_halfway() {
        let mut clock = MarkClock::default();
        let mut stopped = syncing(SyncStage::Filters, Some(0.3));
        stopped.error = Some("no nodes".into());
        for moving in [true, false] {
            let (mark, again) = rail_mark_now(&mut clock, &stopped, moving, 5.0);
            assert!(!again);
            assert_eq!(
                mark,
                RailMark::Block {
                    dim: false,
                    x: BLOCK_STOPPED,
                    alpha: 1.0,
                    stopped: true
                }
            );
        }
    }

    fn on_platform(works: &[evolite_api::PlatformWork]) -> evolite_api::SyncInfo {
        let mut s = syncing(SyncStage::Done, None);
        s.platform = works.to_vec();
        s
    }

    fn moving(shown: (RailMark, bool)) -> bool {
        matches!(
            shown,
            (
                RailMark::Block {
                    dim: false,
                    stopped: false,
                    ..
                },
                true
            )
        )
    }

    /// BUG-137 AC-D2, AC-D3: Platform work moves the mark once it has gone 0.6 s; a shorter one
    /// does not touch it.
    #[test]
    fn platform_work_moves_the_mark_after_six_tenths_of_a_second() {
        use evolite_api::PlatformWork::Tokens;
        let (going, idle) = (on_platform(&[Tokens]), on_platform(&[]));
        let mut clock = MarkClock::default();
        assert_eq!(
            rail_mark_now(&mut clock, &going, true, 10.0),
            (RailMark::Done, false)
        );
        assert_eq!(
            rail_mark_now(&mut clock, &going, true, 10.5),
            (RailMark::Done, false)
        );
        assert!(moving(rail_mark_now(&mut clock, &going, true, 10.6)));
        assert!(clock.platform(), "the words say Platform");
        let mut clock = MarkClock::default();
        rail_mark_now(&mut clock, &going, true, 20.0);
        rail_mark_now(&mut clock, &going, true, 20.3);
        for t in [20.4, 20.6, 21.0, 22.0] {
            assert_eq!(
                rail_mark_now(&mut clock, &idle, true, t),
                (RailMark::Done, false),
                "0.3 s of work: still at {t}"
            );
        }
        assert!(!clock.platform());
    }

    /// BUG-137 AC-D3: a cycle begun runs to its end, then 0.4 s to the full logo; a pause under
    /// a second between two works does not break it.
    #[test]
    fn a_platform_cycle_runs_to_its_end() {
        use evolite_api::PlatformWork::Identities;
        let (going, idle) = (on_platform(&[Identities]), on_platform(&[]));
        let mut clock = MarkClock::default();
        rail_mark_now(&mut clock, &going, true, 0.0);
        assert!(
            moving(rail_mark_now(&mut clock, &going, true, 0.6)),
            "the cycle begins"
        );
        // The work ends at 1.0; the cycle (0.6 + 2.8 = 3.4) goes on.
        assert!(moving(rail_mark_now(&mut clock, &idle, true, 1.0)));
        assert!(moving(rail_mark_now(&mut clock, &idle, true, 3.3)));
        // Its end: from home, 0.4 s to the full logo.
        let (mark, again) = rail_mark_now(&mut clock, &idle, true, 3.45);
        assert!(again, "on its way to the full logo");
        assert!(
            matches!(mark, RailMark::Block { x, .. } if x == BLOCK_HOME),
            "{mark:?}"
        );
        assert_eq!(
            rail_mark_now(&mut clock, &idle, true, 3.45 + TO_DONE + 0.01),
            (RailMark::Done, false)
        );
        // A pause of 0.5 s at the cycle's end: the next cycle, not the full logo.
        let mut clock = MarkClock::default();
        rail_mark_now(&mut clock, &going, true, 0.0);
        rail_mark_now(&mut clock, &going, true, 0.6);
        rail_mark_now(&mut clock, &going, true, 3.0);
        let shown = rail_mark_now(&mut clock, &idle, true, 3.5);
        assert!(moving(shown), "{shown:?}");
        assert!(
            matches!(shown.0, RailMark::Block { x, .. } if x == BLOCK_FROM),
            "a new cycle from the bar's start: {shown:?}"
        );
    }

    /// BUG-137 AC-D5: the system's animations off: no frames, the mark dim with the block at the
    /// bar's start (Platform has no percent); then the full logo at once.
    #[test]
    fn platform_work_with_animations_off_is_dim_and_still() {
        use evolite_api::PlatformWork::Credits;
        let (going, idle) = (on_platform(&[Credits]), on_platform(&[]));
        let mut clock = MarkClock::default();
        rail_mark_now(&mut clock, &going, false, 0.0);
        assert_eq!(
            rail_mark_now(&mut clock, &going, false, 0.7),
            (
                RailMark::Block {
                    dim: true,
                    x: BLOCK_FROM,
                    alpha: 1.0,
                    stopped: false
                },
                false
            )
        );
        assert_eq!(
            rail_mark_now(&mut clock, &idle, false, 0.8),
            (RailMark::Done, false)
        );
    }

    /// A fixed «now»: 14 November 2026, 15:00 here.
    fn at_three() -> chrono::DateTime<chrono::Local> {
        use chrono::TimeZone;
        chrono::Local
            .with_ymd_and_hms(2026, 11, 14, 15, 0, 0)
            .single()
            .expect("a time")
    }

    fn words(status: &evolite_api::SyncInfo, platform: bool) -> CardWords {
        card_words(status, platform, at_three())
    }

    /// SPV item 2a (AC-T2а.2): no new block for 34 minutes — «No new blocks» in the warning
    /// colour, «block {n} · 34 min ago», the reason in the tooltip; for 125 — the same title in
    /// danger, «over 2 hours», the network maybe stopped and what to do. Before Platform's works.
    #[test]
    fn no_new_blocks_says_so_in_place_of_synced() {
        let mut s = on_platform(&[evolite_api::PlatformWork::Tokens]);
        s.tip_height = 1_284_112;
        s.stale_minutes = Some(34);
        let w = words(&s, true);
        assert_eq!(
            (w.title.as_str(), w.warning, w.stopped),
            ("No new blocks", true, false)
        );
        assert_eq!(w.detail, "block 1,284,112 · 34 min ago");
        assert_eq!(w.detail_short.as_deref(), Some("34 min ago"));
        assert!(
            w.tip_detail.starts_with("No new blocks for 34 minutes."),
            "{}",
            w.tip_detail
        );
        s.stale_minutes = Some(125);
        let w = words(&s, false);
        assert_eq!(
            (w.title.as_str(), w.warning, w.stopped),
            ("No new blocks", false, true)
        );
        assert_eq!(w.detail, "block 1,284,112 · over 2 hours");
        assert_eq!(w.tip_title, "The network is not sending new blocks");
        assert!(
            w.tip_detail.contains("No new blocks for over 2 hours."),
            "{}",
            w.tip_detail
        );
        assert!(
            w.tip_detail.ends_with("Settings › Network and nodes"),
            "{}",
            w.tip_detail
        );
        s.stale_minutes = Some(60);
        assert!(words(&s, false).detail.ends_with("over 1 hour"), "one hour");
        s.stale_minutes = None;
        assert_eq!(words(&s, false).title, "Synced");
    }

    /// BUG-137, BUG-155: Core first; with Core done, «Sync» and what goes, three at most on the
    /// card, all of them in the tooltip; the bar runs.
    #[test]
    fn the_words_say_platform_once_core_is_done() {
        use evolite_api::PlatformWork::*;
        let w = words(&on_platform(&[Identities, Tokens]), true);
        assert_eq!((w.title.as_str(), w.stopped), ("Platform", false));
        assert_eq!(w.detail, "identities · tokens");
        assert_eq!(w.tip_title, "Sync");
        assert_eq!(w.bar, Bar::Running);
        let w = words(
            &on_platform(&[Identities, Names, Credits, Contacts, Pool]),
            true,
        );
        assert_eq!(w.detail, "identities · names · credits · and 2 more");
        assert_eq!(
            w.tip_detail,
            "Platform: identities · names · credits · contacts · pool"
        );
        let mut both = on_platform(&[Tokens]);
        both.stage = SyncStage::Filters;
        assert_eq!(words(&both, true).title, "Block filters", "Core first");
        assert_eq!(words(&on_platform(&[]), false).title, "Synced");
    }

    /// BUG-155 AC-A1: the short name on the card, the percent first in the second line, the
    /// full name with it in the tooltip; the bar shows the same part.
    #[test]
    fn a_stage_is_named_short_on_the_card_and_in_full_in_the_tooltip() {
        let mut s = syncing(SyncStage::Quorums, Some(0.45));
        s.minutes_left = Some(3);
        let w = words(&s, false);
        assert_eq!(w.title, "Masternode quorums");
        assert_eq!(w.detail, "45 % · about 3 min left");
        assert_eq!(w.tip_title, "Checking masternode quorums · 45 %");
        assert_eq!(w.tip_detail, "about 3 min left");
        assert_eq!(w.bar, Bar::Part(0.45));
        let w = words(&syncing(SyncStage::Connecting, None), false);
        assert_eq!(
            (w.title.as_str(), w.detail.as_str()),
            ("Network nodes", "3 Core nodes")
        );
        assert_eq!(w.bar, Bar::Running);
    }

    /// BUG-155 AC-A5: a stop says «Stopped» in `danger` and why; the tooltip «The sync stopped»
    /// with the reason whole; the bar stands where it stopped, or halfway.
    #[test]
    fn a_stop_is_said_on_the_card_with_its_reason() {
        let mut s = syncing(SyncStage::Filters, Some(0.3));
        s.error = Some("the node refused us".into());
        let w = words(&s, false);
        assert_eq!((w.title.as_str(), w.stopped), ("Stopped", true));
        assert_eq!(
            w.detail,
            "the node refused us · resync in Settings › Network and nodes"
        );
        assert_eq!(w.tip_title, "The sync stopped");
        assert_eq!(w.bar, Bar::Stopped(0.3));
        let mut s = syncing(SyncStage::Headers, None);
        s.folder_refused = Some("Choose another folder".into());
        let w = words(&s, false);
        assert_eq!(w.detail, "a folder needs fixing");
        assert_eq!(w.tip_detail, "Choose another folder");
        assert_eq!(w.bar, Bar::Stopped(0.5));
    }

    /// BUG-155 AC-A6, A9: synced — no bar; «block {} · ChainLock» on the card, «block {}» when
    /// that is too long; the tooltip adds when the tip came: the time today, the date before.
    #[test]
    fn synced_says_the_block_and_when_it_came() {
        let mut s = on_platform(&[]);
        s.chainlocked = true;
        let today = at_three().timestamp() - 28 * 60;
        s.tip_at = Some(today);
        let w = words(&s, false);
        assert_eq!(w.bar, Bar::None);
        assert_eq!(w.detail, format!("block 1,284,112{CHAINLOCKED}"));
        assert_eq!(w.detail_short.as_deref(), Some("block 1,284,112"));
        assert_eq!(w.tip_detail, "block 1,284,112 · ChainLock · updated 14:32");
        s.tip_at = Some(today - 24 * 60 * 60);
        assert_eq!(
            words(&s, false).tip_detail,
            "block 1,284,112 · ChainLock · updated Nov 13, 14:32"
        );
        s.tip_at = None;
        s.chainlocked = false;
        let w = words(&s, false);
        assert_eq!(
            (w.detail.as_str(), w.detail_short),
            ("block 1,284,112", None)
        );
        assert_eq!(w.tip_detail, "block 1,284,112");
    }

    /// BUG-165: Core's nodes are counted in the language's plural: «1 узел», «3 узла», «5 узлов».
    #[test]
    fn core_nodes_are_counted_in_russian() {
        crate::i18n::set(crate::i18n::Lang::Ru);
        for (n, want) in [(1, "1 узел Core"), (3, "3 узла Core"), (5, "5 узлов Core")]
        {
            let mut s = syncing(SyncStage::Connecting, None);
            s.core_peers = n;
            assert_eq!(words(&s, false).detail, want);
        }
        crate::i18n::set(crate::i18n::Lang::En);
    }

    /// BUG-166: in Arabic the synced tooltip reads «block · ChainLock · updated 14:32» right to left:
    /// the time stays with «updated», not with «ChainLock» (drawn left to right: the time, «updated»,
    /// then «ChainLock»).
    #[test]
    fn in_arabic_the_time_stays_with_updated() {
        crate::i18n::set(crate::i18n::Lang::Ar);
        let mut s = on_platform(&[]);
        s.chainlocked = true;
        s.tip_at = Some(at_three().timestamp() - 28 * 60);
        let shown = crate::i18n::t(&words(&s, false).tip_detail).into_owned();
        crate::i18n::set(crate::i18n::Lang::En);
        let at = |needle: &str| {
            shown
                .find(needle)
                .unwrap_or_else(|| panic!("{needle} in {shown:?}"))
        };
        assert!(
            at("14:32") < at("حُدّثت") && at("حُدّثت") < at("ChainLock"),
            "{shown:?}"
        );
    }

    /// BUG-155, the designer's review: once synced the fill runs on from where it was to the end,
    /// never jumping there, and only then goes with the track.
    #[test]
    fn the_bar_runs_to_its_end_before_it_goes() {
        let (to, alpha) = bar_going(Bar::Part(0.45), 0.0).expect("there");
        assert_eq!((to, alpha), (0.45, 1.0), "from where it was");
        let (to, alpha) = bar_going(Bar::Part(0.45), BAR_TO_END / 2.0).expect("there");
        assert!(to > 0.45 && to < 1.0 && alpha == 1.0, "{to} {alpha}");
        assert_eq!(
            bar_going(Bar::Part(0.45), BAR_TO_END),
            Some((1.0, 1.0)),
            "at the end"
        );
        let (to, alpha) = bar_going(Bar::Part(0.45), BAR_TO_END + BAR_GONE / 2.0).expect("there");
        assert!(to == 1.0 && (alpha - 0.5).abs() < 1e-3, "{to} {alpha}");
        assert_eq!(
            bar_going(Bar::Part(0.45), BAR_TO_END + BAR_GONE),
            None,
            "gone"
        );
        assert_eq!(bar_going(Bar::Running, 0.0), Some((1.0, 1.0)));
    }

    /// BUG-155: the designer's places — the mark in the middle of the card's height in every
    /// size, the words' column 156 from 50, the bar at the foot (wide), under the words (low),
    /// under the mark (rail).
    #[test]
    fn the_card_s_parts_stand_where_the_designer_put_them() {
        let wide = egui::Rect::from_min_size(egui::pos2(16.0, 600.0), vec2(216.0, SYNC_WIDE));
        let (mark, words, bar) = card_layout(wide, false, false);
        assert_eq!(mark.min - wide.min, vec2(12.0, 18.0));
        assert_eq!(words - wide.min, vec2(50.0, 14.0));
        assert_eq!(wide.right() - 10.0 - words.x, WORDS);
        assert_eq!(bar.min - wide.min, vec2(12.0, 57.0));
        assert_eq!(bar.size(), vec2(192.0, 3.0));
        let low = egui::Rect::from_min_size(egui::pos2(16.0, 300.0), vec2(216.0, SYNC_LOW));
        let (mark, words, bar) = card_layout(low, false, true);
        assert_eq!(mark.min - low.min, vec2(12.0, 8.0));
        assert_eq!(words - low.min, vec2(50.0, 13.0));
        assert_eq!(bar.min - low.min, vec2(50.0, 37.0));
        assert_eq!(bar.width(), WORDS);
        let rail = egui::Rect::from_min_size(egui::pos2(12.0, 600.0), vec2(52.0, SYNC_RAIL));
        let (mark, _, bar) = card_layout(rail, true, false);
        assert_eq!(mark.min - rail.min, vec2(12.0, 12.0));
        assert_eq!(bar.min - rail.min, vec2(10.0, 45.0));
        assert_eq!(bar.size(), vec2(32.0, 3.0));
        let rail = egui::Rect::from_min_size(egui::pos2(12.0, 300.0), vec2(52.0, SYNC_LOW));
        let (mark, _, bar) = card_layout(rail, true, true);
        assert_eq!(mark.min - rail.min, vec2(12.0, 8.0));
        assert_eq!(bar.min - rail.min, vec2(10.0, 39.0));
    }
}
