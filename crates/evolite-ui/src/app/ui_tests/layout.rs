//! Responsive spec R2: the window's width class (C/S/W/X) gives the menu or the rail, the
//! screen's margins and the content's width.

use super::*;
use crate::theme::{Breakpoint, CONTENT_MAX_W};

/// Where the screen's title stands: its left edge (its right edge in `ar`, mirrored).
fn title_edge(kit: &Kit, title: &str, rtl: bool, width: f32) -> f32 {
    let r = kit
        .query_all(By::new().role(Role::Label).value(title))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("the title «{title}»"));
    if rtl { width - r.right() } else { r.left() }
}

/// The menu's width: the right edge of its first item (in `ar` from the right). An item is a
/// selectable label named by its words (`components::nav_item`).
fn menu_item_edge(kit: &Kit, name: &str, rtl: bool, width: f32) -> f32 {
    let item = kit
        .query_all_by(|n| n.role() != Role::Label && n.label().as_deref() == Some(name))
        .map(|n| n.rect())
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap_or_else(|| panic!("the menu's «{name}»"));
    if rtl {
        width - item.left()
    } else {
        item.right()
    }
}

/// R2 (the customer, 2026-11-07: дизайнер-27, 28): below 1280 the menu is the rail of 76, wider
/// it is 248 with its words; the margins are 24 in C (800–1023) and 40 wider; from 1600 the
/// content is 1200 wide in the middle of what is left. Mirrored in `ar`. At 1100 the rail too:
/// it was the menu there before.
#[test]
fn the_window_width_gives_the_menu_the_margins_and_the_content_width() {
    for lang in ["en", "ar"] {
        let rtl = lang == "ar";
        for (width, menu, left) in [
            (800.0, 76.0, 76.0 + 24.0),
            (1023.0, 76.0, 76.0 + 24.0),
            (1024.0, 76.0, 76.0 + 40.0),
            (1100.0, 76.0, 76.0 + 40.0),
            (1280.0, 248.0, 248.0 + 40.0),
            (
                1600.0,
                248.0,
                248.0 + 40.0 + (1600.0 - 248.0 - 80.0 - CONTENT_MAX_W) / 2.0,
            ),
            (
                1920.0,
                248.0,
                248.0 + 40.0 + (1920.0 - 248.0 - 80.0 - CONTENT_MAX_W) / 2.0,
            ),
        ] {
            let (_turn, mut kit) = demo_app("overview", lang);
            kit.set_size(egui::vec2(width, 800.0));
            kit.run_steps(3);
            // The menu says its words as written, the title is drawn (shaped in `ar`).
            let name = crate::i18n::spoken("Overview");
            let title = crate::i18n::t("Overview").into_owned();
            let bp = Breakpoint::of(width);
            assert_eq!(bp.rail(), menu < 100.0, "{lang} {width}");
            let item = menu_item_edge(&kit, &name, rtl, width);
            assert!(
                item <= menu && item > menu - 40.0,
                "{lang} {width}: the menu's item ends at {item}, the menu is {menu}"
            );
            let at = title_edge(&kit, &title, rtl, width);
            assert!(
                (at - left).abs() < 1.5,
                "{lang} {width}: the title at {at}, not {left}"
            );
        }
    }
}
