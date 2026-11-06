//! The menu on the left: logo, the wallet switcher, sections, the sync widget, network and
//! settings.

use super::NAV;
use super::Screen;
use crate::components::{Assets, nav_item, network_chip};
use crate::context::Ctx;
use crate::icons::Logo;
use crate::text::{self, group_thousands};
use crate::theme::{Weight, mono, sans};
use egui::{Align, Layout, Stroke, Ui, vec2};
use evolite_api::SyncStage;

/// Sidebar 248 px (rail of 76 px below 1100 px): logo, the wallet switcher (Figma 26),
/// 9 items, sync widget, network and settings. `menu`: the wallet menu is open.
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
    if snap.wallets.iter().any(|w| w.open) {
        // Figma 26: the switcher 87 px from the top, the items 4 px under it.
        ui.add_space(if rail { 20.0 } else { 24.0 });
        super::wallets::switcher(ui, a, cx, rail, menu);
    } else {
        ui.add_space(24.0);
    }
    // Footer pinned to the bottom: sync widget (74 px; 44 in a low window), 12 px, network and
    // settings (40 px), 20 px. The items scroll between, never under it: at 150 % on a laptop
    // the window is about 450 px high.
    let area = ui.max_rect();
    let compact = area.height() < 640.0;
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
    egui::ScrollArea::vertical()
        .id_salt("nav")
        .max_height(room)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (screen, icon, label) in NAV {
                let lit = screen == active
                    || (screen == Screen::Identity && active == Screen::CreateIdentity);
                if nav_item(ui, a, icon, label, lit, rail).clicked() {
                    cx.go(screen);
                }
            }
        });
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
    let done = status.stage == SyncStage::Done;
    a.icons
        .sync_mark(if done { 3 } else { 2 }, &p, 28.0)
        .paint_at(ui, mark);
    let (title, detail) = if done {
        (
            tr!("Synced").to_string(),
            format!(tr!("block {}"), group_thousands(status.tip_height as i64))
                + if status.chainlocked {
                    " · ChainLock"
                } else {
                    ""
                },
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
    // All of it for the screen reader (the words are painted); in the rail or a low window
    // part of it is not drawn: all of it on hover too.
    let both = format!("{} · {}", crate::i18n::t(&title), crate::i18n::t(&detail));
    let resp = ui.interact(sync, ui.id().with("sync-card"), egui::Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &both));
    if rail || compact {
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
        let detail = ui.painter().layout(
            crate::i18n::t(&detail).into_owned(),
            mono(11.0, Weight::Regular),
            p.muted,
            sync.right() - 12.0 - x,
        );
        ui.painter()
            .galley(egui::pos2(x, sync.top() + 31.0), detail, p.muted);
    }
}
