//! The menu on the left: logo, the wallet switcher, sections, the sync widget, network and
//! settings.

use super::NAV;
use super::Screen;
use crate::components::{Assets, NAV_ITEM, nav_item, network_chip};
use crate::context::Ctx;
use crate::icons::Logo;
use crate::text::{self, group_thousands};
use crate::theme::{Weight, mono, sans};
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
    // Footer pinned to the bottom: sync widget (74 px; 44 in a low window), 12 px, network and
    // settings (40 px), 20 px. The items scroll between, never under it: at 150 % on a laptop
    // the window is about 450 px high.
    let sync_h = if compact { 44.0 } else { 74.0 };
    let footer = egui::Rect::from_min_max(
        egui::pos2(area.left(), area.bottom() - 60.0),
        egui::pos2(area.right(), area.bottom() - 20.0),
    );
    let sync = egui::Rect::from_min_max(
        egui::pos2(area.left(), footer.top() - 12.0 - sync_h),
        egui::pos2(area.right(), footer.top() - 12.0),
    );
    let room = (sync.top() - 12.0 - ui.cursor().top()).max(40.0);
    #[allow(unused_mut)]
    let mut items = NAV.to_vec();
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
                    || (screen == Screen::Identity && active == Screen::CreateIdentity);
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
                crate::components::named(&resp, egui::WidgetType::Button, true, tr!("Settings"));
                if on || resp.hovered() {
                    ui.painter().rect_filled(rect, 10, p.raised);
                }
                let color = if on { p.text } else { p.muted };
                a.icons.icon("settings", color, &p, 24.0).paint_at(
                    ui,
                    egui::Rect::from_center_size(rect.center(), vec2(24.0, 24.0)),
                );
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
    let radius = egui::CornerRadius::same(12);
    ui.painter().rect(
        sync,
        radius,
        p.surface,
        Stroke::new(1.0, p.line),
        egui::StrokeKind::Inside,
    );
    let mark = if rail {
        egui::Rect::from_center_size(sync.center(), vec2(28.0, 28.0))
    } else {
        let top = if compact {
            sync.center().y - 14.0
        } else {
            sync.top() + 12.0
        };
        egui::Rect::from_min_size(egui::pos2(sync.left() + 12.0, top), vec2(28.0, 28.0))
    };
    // The rail: the mark alone, moving while sync goes (BUG-75).
    if rail {
        rail_sync(ui, a, status, sync, mark);
        return;
    }
    let done = status.stage == SyncStage::Done;
    a.icons
        .sync_mark(if done { 3 } else { 2 }, &p, 28.0)
        .paint_at(ui, mark);
    let (title, detail) = if status.folder_refused.is_some() {
        (
            tr!("The sync stopped").to_string(),
            tr!("a folder needs fixing").to_string(),
        )
    } else if done {
        (
            tr!("Synced").to_string(),
            format!(tr!("block {}"), group_thousands(status.tip_height as i64))
                + if status.chainlocked { CHAINLOCKED } else { "" },
        )
    } else {
        (
            text::sync_title(status),
            match (&status.error, status.minutes_left) {
                (Some(_), _) => tr!("the sync stopped").to_string(),
                (None, Some(m)) => format!(tr!("about {} min left"), m.max(1)),
                (None, None) => format!(tr!("{} Core nodes"), status.core_peers),
            },
        )
    };
    // All of it for the screen reader (the words are painted); in a low window part of it is
    // not drawn: all of it on hover too.
    let both = format!("{} · {}", crate::i18n::t(&title), crate::i18n::t(&detail));
    let resp = ui.interact(sync, ui.id().with("sync-card"), egui::Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &both));
    if compact {
        resp.on_hover_text(both);
    }
    if !rail {
        let x = mark.right() + 12.0;
        let (y, align) = if compact {
            (sync.center().y, egui::Align2::LEFT_CENTER)
        } else {
            (sync.top() + 12.0, egui::Align2::LEFT_TOP)
        };
        ui.painter().text(
            egui::pos2(x, y),
            align,
            crate::i18n::t(&title),
            sans(13.0, Weight::SemiBold),
            p.text,
        );
        if compact {
            return;
        }
        let font = mono(11.0, Weight::Regular);
        let wrap = sync.right() - 12.0 - x;
        let shown = crate::components::lines(ui.ctx(), &detail, &font, wrap);
        let detail = ui.painter().layout(shown, font, p.muted, wrap);
        ui.painter()
            .galley(egui::pos2(x, sync.top() + 31.0), detail, p.muted);
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

/// The rail mark's clock, kept from frame to frame.
#[derive(Clone, Copy, Default)]
pub(crate) struct MarkClock {
    /// When the cycle that runs began; `None`: none runs.
    cycle: Option<f64>,
    /// The way to the full logo: when it began, and where the block was then and how bright.
    to_done: Option<(f64, f32, f32)>,
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
    if status.stage == SyncStage::Done {
        // From wherever the cycle was, the block rides home or shows there in 400 ms.
        if let Some(start) = clock.cycle.take()
            && moving
        {
            let (x, alpha) = block_at((((now - start) % CYCLE) / CYCLE) as f32);
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
            x: block_still(status.progress),
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

/// The two lines of the rail mark's words, and whether the first is a stop's (`danger`).
fn rail_words(status: &evolite_api::SyncInfo) -> (String, String, bool) {
    if let Some(what_to_do) = &status.folder_refused {
        return (
            tr!("The sync stopped").to_string(),
            crate::i18n::t(what_to_do).to_string(),
            true,
        );
    }
    if let Some(why) = &status.error {
        return (
            tr!("The sync stopped").to_string(),
            format!(
                tr!("{} · resync in Settings → Network and nodes"),
                crate::i18n::t(why)
            ),
            true,
        );
    }
    if status.stage == SyncStage::Done {
        let chainlock = if status.chainlocked { CHAINLOCKED } else { "" };
        return (
            tr!("Synced").to_string(),
            format!(tr!("block {}"), group_thousands(status.tip_height as i64)) + chainlock,
            false,
        );
    }
    let detail = match status.minutes_left {
        Some(m) => format!(tr!("about {} min left"), m.max(1)),
        None => format!(tr!("{} Core nodes"), status.core_peers),
    };
    (
        format!(tr!("Sync · {}"), text::sync_title(status)),
        detail,
        false,
    )
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

/// The sync card of the rail: the mark alone (BUG-75, «the block finds its place»). While sync
/// goes the block rides along the bar to its home, every 2.8 s; once done the full logo, still.
/// With the system's animations off nothing moves: «e» and bar at 55 %, the block as far as the
/// stage has gone. Its words show on hover and on keyboard focus.
fn rail_sync(
    ui: &mut Ui,
    a: &mut Assets,
    status: &evolite_api::SyncInfo,
    sync: egui::Rect,
    mark: egui::Rect,
) {
    let p = a.p;
    let id = ui.id().with("sync-card");
    let now = ui.input(|i| i.time);
    let moving = crate::motion::system_on(ui.ctx());
    let mut clock = ui.data(|d| d.get_temp::<MarkClock>(id)).unwrap_or_default();
    let (shown, again) = rail_mark_now(&mut clock, status, moving, now);
    ui.data_mut(|d| d.insert_temp(id, clock));
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(rail_mark_id(), (shown, again)));
    match shown {
        RailMark::Done => a.icons.sync_mark(3, &p, 28.0).paint_at(ui, mark),
        RailMark::Block {
            dim,
            x,
            alpha,
            stopped,
        } => {
            a.icons.sync_base(dim, &p, 28.0).paint_at(ui, mark);
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
    rail_sync_words(ui, a, status, sync, id);
}

/// The rail mark's words: a tooltip of two lines on hover and on keyboard focus, and one name
/// for the screen reader — a polite status that changes with the stage, the end and a stop,
/// not with each percent.
fn rail_sync_words(
    ui: &mut Ui,
    a: &mut Assets,
    status: &evolite_api::SyncInfo,
    sync: egui::Rect,
    id: egui::Id,
) {
    let p = a.p;
    let (title, detail, stopped) = rail_words(status);
    let title = crate::i18n::t(&title).into_owned();
    let detail = crate::i18n::t(&detail).into_owned();
    // The reader is told only of a new stage, the end and a stop: the status is a polite live
    // region for a second after one, quiet while the percent and the minutes left move on.
    let key = (status.stage, status.error.is_some());
    let now = ui.input(|i| i.time);
    let changed_id = id.with("changed");
    let changed = match ui.data(|d| d.get_temp::<((SyncStage, bool), f64)>(changed_id)) {
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
    let spoken = format!("{title} · {detail}");
    let resp = ui.interact(sync, id, egui::Sense::hover() | egui::Sense::FOCUSABLE);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.set_role(egui::accesskit::Role::Status);
        node.set_label(spoken.clone());
        node.set_live(live);
    });
    let words = |ui: &mut Ui| {
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
}
