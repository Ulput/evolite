//! BUG-249: at the smallest window the app allows (800×440), in English, Russian, Japanese,
//! German and Arabic (whose lines wrap differently), every preset fits the window's width, and
//! an open dialog either keeps a scrollable part of at least [`MIN_MIDDLE`] or has its summary
//! and footer in the window.

use super::*;
use crate::cli::PRESETS;
use crate::components::{MIN_MIDDLE, dialog_seen};

const SMALLEST: [f32; 2] = [800.0, 440.0];
const LANGUAGES: [&str; 5] = ["en", "ru", "ja", "de", "ar"];

/// The accessible nodes that stand out of the window to the left or the right — a page wider
/// than the window. A node out at the top or the bottom is only scrolled away.
fn too_wide(kit: &Kit) -> Vec<String> {
    let window = kit.ctx.content_rect();
    let ppp = kit.ctx.pixels_per_point();
    kit.query_all_by(|n| n.bounding_box().is_some())
        .filter_map(|n| {
            let b = n.accesskit_node().bounding_box()?;
            let r = egui::Rect::from_min_max(
                egui::pos2(b.x0 as f32 / ppp, b.y0 as f32 / ppp),
                egui::pos2(b.x1 as f32 / ppp, b.y1 as f32 / ppp),
            );
            let out = r.width() > 0.5
                && r.height() > 0.5
                && (r.left() < window.left() - 1.0 || r.right() > window.right() + 1.0);
            let node = n.accesskit_node();
            out.then(|| {
                format!(
                    "{:?} {:?} at {r:?}",
                    node.role(),
                    node.label().or(node.value()).unwrap_or_default()
                )
            })
        })
        .take(3)
        .collect()
}

/// What a dialog that is up must keep at the smallest window.
fn dialog_kept(kit: &Kit) -> Option<String> {
    let seen = dialog_seen(&kit.ctx)?;
    let window = kit.ctx.content_rect();
    if let Some(view) = seen.scrolled
        && view.height() < MIN_MIDDLE - 1.0
    {
        let pinned = seen.footer.is_some_and(|f| window.contains_rect(f))
            && seen.summary.is_none_or(|(s, _)| window.contains_rect(s));
        if !pinned {
            return Some(format!(
                "{:?}: {:.0} px scroll and its summary or footer out of the window",
                seen.id,
                view.height()
            ));
        }
    }
    None
}

fn sweep(lang: &str) {
    let mut wrong = Vec::new();
    for preset in PRESETS {
        // The staged sync never goes quiet; it has no page of its own to check.
        if matches!(*preset, "sync" | "sync-nodes") {
            continue;
        }
        let (_turn, mut kit) = demo_app(preset, lang);
        if preset.starts_with("connect") {
            wait_for(&mut kit, "the app's request", |kit| {
                dialog_seen(&kit.ctx).is_some()
            });
        }
        kit.set_size(egui::vec2(SMALLEST[0], SMALLEST[1]));
        kit.run_steps(8);
        for w in too_wide(&kit) {
            wrong.push(format!("{preset}: wider than the window: {w}"));
        }
        if let Some(w) = dialog_kept(&kit) {
            wrong.push(format!("{preset}: {w}"));
        }
    }
    assert!(wrong.is_empty(), "{lang}:\n{}", wrong.join("\n"));
}

#[test]
fn every_preset_fits_the_smallest_window_en() {
    sweep(LANGUAGES[0]);
}

#[test]
fn every_preset_fits_the_smallest_window_ru() {
    sweep(LANGUAGES[1]);
}

#[test]
fn every_preset_fits_the_smallest_window_ja() {
    sweep(LANGUAGES[2]);
}

#[test]
fn every_preset_fits_the_smallest_window_de() {
    sweep(LANGUAGES[3]);
}

#[test]
fn every_preset_fits_the_smallest_window_ar() {
    sweep(LANGUAGES[4]);
}
