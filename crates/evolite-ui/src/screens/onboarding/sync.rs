//! O6 · First sync: the core's sync, stage by stage.

use super::txt;
use super::{Outcome, heading};
use crate::components::{
    Assets, ButtonKind, Mark, button, card, check_line, inner, notice, progress_bar, text_action,
};
use crate::text::group_thousands;
use crate::theme::{Weight, mono};
use egui::{Align, Align2, Layout, Margin, Sense, Ui, pos2, vec2};
use evolite_api::{SyncInfo, SyncStage};

/// O6 · First sync: the core's sync, stage by stage.
pub(super) fn sync(ui: &mut Ui, a: &mut Assets, sync: &SyncInfo, demo: bool) -> Option<Outcome> {
    let p = a.p;
    let order = |stage: SyncStage| match stage {
        SyncStage::Connecting | SyncStage::Headers | SyncStage::Masternodes => 0,
        SyncStage::Filters => 1,
        SyncStage::Transactions => 2,
        SyncStage::Platform => 3,
        SyncStage::ChainLock => 4,
        SyncStage::Done => 5,
    };
    let running = order(sync.stage);
    let mut out = None;
    heading(
        ui,
        &p,
        tr!("We verify the network ourselves"),
        tr!(
            "evolite does not take servers at their word: it downloads headers, filters and masternode lists and checks the signatures on this computer."
        ),
    );
    #[cfg(demo_core)]
    if demo {
        crate::components::demo_line(
            ui,
            a,
            tr!("An example of the sync: the real one comes with the wallet core."),
        );
    }
    #[cfg(not(demo_core))]
    let _ = demo;
    let width = ui.available_width();
    let percent = sync.progress;
    let stage = crate::text::sync_stage(sync.stage);
    let filtering = sync.stage == SyncStage::Filters;
    card(&p, Margin::symmetric(32, 28)).show(ui, |ui| {
        ui.set_width(inner(width, 32));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 32.0;
            let (mark, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
            a.icons
                .sync_mark(if running >= 5 { 3 } else { 2 }, &p, 48.0)
                .paint_at(ui, mark);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                ui.label(txt(
                    &format!("{stage} · {} %", (percent * 100.0).round() as i32),
                    15.0,
                    Weight::SemiBold,
                    p.text,
                ));
                progress_bar(ui, &p, percent);
                ui.label(
                    egui::RichText::new(crate::i18n::t(&if filtering {
                        match sync.minutes_left {
                            Some(m) => format!(
                                tr!("About {} min left · {} nodes"),
                                m.max(1),
                                sync.core_peers
                            ),
                            None => format!(tr!("{} nodes"), sync.core_peers),
                        }
                    } else if running < 1 {
                        // Before the filters: nothing is checked yet.
                        format!(tr!("{} nodes"), sync.core_peers)
                    } else {
                        format!(tr!("Filters verified · {} nodes"), sync.core_peers)
                    }))
                    .font(mono(11.0, Weight::Regular))
                    .color(p.muted),
                );
            });
        });
    });
    let fmt = |n: u32| group_thousands(n as i64);
    let tip = sync.tip_height;
    let stages: [(&str, String, usize); 6] = [
        (
            tr!("Block headers"),
            format!(tr!("{} of {}"), fmt(sync.headers_height), fmt(tip)),
            0,
        ),
        (
            tr!("Masternode lists"),
            // The lists come with the headers: done once the filters start.
            if running < 1 {
                tr!("checking quorums").into()
            } else {
                tr!("quorums verified").into()
            },
            0,
        ),
        (
            tr!("Block filters (BIP158)"),
            format!(tr!("{} of {}"), fmt(sync.filters_height), fmt(tip)),
            1,
        ),
        (tr!("Your transactions"), tr!("after the filters").into(), 2),
        (
            tr!("Platform: identities and names"),
            tr!("GroveDB proofs").into(),
            3,
        ),
        (
            tr!("ChainLock of the latest block"),
            tr!("BLS signature").into(),
            4,
        ),
    ];
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (label, detail, order) in stages {
            let mark = if order < running {
                Mark::Yes
            } else if order == running {
                Mark::Now
            } else {
                Mark::Later
            };
            let rect = check_line(ui, a, mark, label);
            ui.add_space(10.0);
            ui.painter().text(
                pos2(rect.right(), rect.center().y),
                Align2::RIGHT_CENTER,
                crate::i18n::t(&detail),
                mono(11.0, Weight::Regular),
                p.muted,
            );
        }
    });
    notice(
        ui,
        a,
        "info",
        p.muted,
        tr!(
            "You can open the wallet now: the balance appears once your transactions are verified. Sending becomes available after a ChainLock."
        ),
    );
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 41.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            if text_action(ui, a, "peers", tr!("About the nodes")).clicked() {
                out = Some(Outcome::Nodes);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, a, tr!("Open the wallet"), ButtonKind::Primary, None).clicked() {
                    out = Some(Outcome::Wallet);
                }
            });
        },
    );
    out
}
