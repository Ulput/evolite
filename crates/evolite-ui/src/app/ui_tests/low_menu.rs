//! FEAT-0036: the menu in a low window. Below 640 px high the top logo goes and the items are
//! 36 px, 40 apart, so all ten (with «Tokens», FEAT-0019) fit in 800×600; when they still do
//! not fit, a fade at the bottom of the list says so, and the wheel and Tab reach the last one.

use super::*;
use crate::screens::sidebar::{add_menu_items, menu_fade, menu_more_below};

/// The footer under the items: 20 + 40 (settings) + 12 + 44 (the low sync card) + 12 px.
const FOOTER_LOW: f32 = 128.0;

/// The rect of the menu item named `name`.
fn item(kit: &Kit, name: &str) -> egui::Rect {
    kit.query_all_by(|n| n.label().as_deref() == Some(name))
        .map(|n| n.rect())
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap_or_else(|| panic!("the menu item «{name}»"))
}

/// The app on Overview at `size`, with `extra` more menu items after the nine of today.
fn menu_at(
    lang: &str,
    size: egui::Vec2,
    extra: usize,
) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app("overview", lang);
    add_menu_items(&kit.ctx, extra);
    kit.set_size(size);
    kit.run_steps(4);
    (turn, kit)
}

/// (height of an item, from one item's top to the next, the first item's top)
fn rhythm(kit: &Kit) -> (f32, f32, f32) {
    let first = item(kit, "Overview");
    let second = item(kit, "Send");
    (first.height(), second.top() - first.top(), first.top())
}

/// AC-T 2, AC-D1, AC-D2: in 800×600 all ten items are in view without scrolling, with no fade;
/// no logo at the top (the items begin under the switcher, 20 px from the top), items 36 px,
/// 40 apart.
#[test]
fn in_800_by_600_all_ten_items_are_in_view() {
    let (_turn, kit) = menu_at("en", egui::vec2(800.0, 600.0), 0);
    let last = item(&kit, "Tokens");
    assert!(
        last.bottom() <= 600.0 - FOOTER_LOW + 0.5,
        "«Tokens» ends at {}, the list at {}",
        last.bottom(),
        600.0 - FOOTER_LOW
    );
    assert_eq!(
        menu_more_below(&kit.ctx),
        Some(false),
        "no fade when all fit"
    );
    let (height, step, top) = rhythm(&kit);
    assert_eq!((height, step), (36.0, 40.0));
    // The border, 20 px, the switcher (44 in the rail), 4 px: no logo above it (with it 129).
    assert_eq!(top, 69.0, "the items begin under the switcher");
}

/// AC-T 3, AC-D4: in 800×440 the last item is past the edge and the fade shows; the wheel brings
/// it into view (and the fade goes at the end of the list), and so does Tab.
#[test]
fn in_800_by_440_the_fade_shows_and_the_last_item_is_reached() {
    let (_turn, mut kit) = menu_at("en", egui::vec2(800.0, 440.0), 0);
    let end = 440.0 - FOOTER_LOW;
    assert!(item(&kit, "Tokens").bottom() > end, "past the edge");
    assert_eq!(menu_more_below(&kit.ctx), Some(true), "the fade");

    // The wheel, over the menu.
    let over = item(&kit, "Overview").center();
    kit.input_mut().events.push(egui::Event::PointerMoved(over));
    kit.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -600.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    kit.run_steps(30);
    assert!(
        item(&kit, "Tokens").bottom() <= end + 0.5,
        "the wheel brings «Tokens» into view: it ends at {}",
        item(&kit, "Tokens").bottom()
    );
    assert_eq!(
        menu_more_below(&kit.ctx),
        Some(false),
        "at the end, no fade"
    );
}

/// AC-D4 by the keyboard: Tab reaches the last item and brings it into view.
#[test]
fn in_800_by_440_tab_brings_the_last_item_into_view() {
    let (_turn, mut kit) = menu_at("en", egui::vec2(800.0, 440.0), 0);
    tab_to(&mut kit, "Tokens");
    kit.run_steps(30);
    let end = 440.0 - FOOTER_LOW;
    assert!(
        item(&kit, "Tokens").bottom() <= end + 0.5,
        "«Tokens» ends at {}, the list at {end}",
        item(&kit, "Tokens").bottom()
    );
}

/// AC-T 1: the threshold is 640, both ways as the window is resized, and steady on each side.
#[test]
fn the_menu_turns_low_below_640_and_back() {
    let (_turn, mut kit) = menu_at("en", egui::vec2(1024.0, 640.0), 0);
    for (height, low) in [(640.0, false), (639.0, true), (640.0, false), (639.0, true)] {
        kit.set_size(egui::vec2(1024.0, height));
        for _ in 0..3 {
            kit.run_steps(1);
            let (item_h, step, _) = rhythm(&kit);
            assert_eq!(
                (item_h, step),
                if low { (36.0, 40.0) } else { (40.0, 44.0) },
                "at {height} px high"
            );
        }
    }
}

/// AC-T 1 (the coordinator's answer 1): it is the menu's own height that counts. With the
/// mainnet band over it, a window 640 px high leaves the menu low.
#[test]
fn the_mainnet_band_takes_its_height_from_the_menu() {
    let (_turn, mut kit) = menu_at("en", egui::vec2(1024.0, 640.0), 0);
    assert_eq!(rhythm(&kit).0, 40.0);
    kit.state_mut().snapshot.as_mut().unwrap().network = evolite_api::Network::Mainnet;
    kit.run_steps(3);
    assert!(has_words(&kit, "Mainnet means real DASH"), "the band");
    assert_eq!(rhythm(&kit).0, 36.0, "the menu under the band is low");
}

/// AC-T 4, AC-D3: from 640 px high the menu is as before: the logo at the top, items 40 px, 44
/// apart — here the menu with its words, 1280×720.
#[test]
fn from_640_high_the_menu_is_as_before() {
    let (_turn, kit) = menu_at("en", egui::vec2(1280.0, 720.0), 0);
    let (height, step, top) = rhythm(&kit);
    assert_eq!((height, step), (40.0, 44.0));
    // The border, 24 px, the logo's row, 24 px, the switcher (50), 4 px.
    assert!(
        top > 130.0,
        "the logo above the switcher: the items at {top}"
    );
}

/// AC-T 6: right to left the menu is mirrored and the fade is still at the bottom of its list.
#[test]
fn right_to_left_the_fade_is_at_the_bottom_too() {
    let (_turn, mut kit) = menu_at("ar", egui::vec2(800.0, 440.0), 1);
    assert_eq!(menu_more_below(&kit.ctx), Some(true));
    tab_to(&mut kit, "Votes");
    kit.run_steps(30);
    assert!(item(&kit, "Votes").bottom() <= 440.0 - FOOTER_LOW + 0.5);
}

/// The designer's row 3: the fade is 24 px, its bottom on the list's bottom, and it reaches up
/// over the last whole item («Contacts» in 800×440), which stays clickable under it.
#[test]
fn the_fade_lies_over_the_last_whole_item_and_lets_it_be_clicked() {
    let (_turn, mut kit) = menu_at("en", egui::vec2(800.0, 440.0), 0);
    let fade = menu_fade(&kit.ctx).flatten().expect("the fade");
    assert_eq!(fade.height(), 24.0);
    // The list ends 12 px over the sync card; the panel's 1 px border lies under the footer.
    assert_eq!(
        fade.bottom(),
        440.0 - 1.0 - FOOTER_LOW,
        "on the list's bottom"
    );
    let contacts = item(&kit, "Contacts");
    assert!(
        contacts.bottom() <= fade.bottom() && contacts.bottom() > fade.top(),
        "over the last whole item: the fade {fade:?}, «Contacts» {contacts:?}"
    );
    // A click under the fade, on the item's lower part.
    let under = egui::pos2(contacts.center().x, contacts.bottom() - 4.0);
    assert!(fade.contains(under));
    for event in [
        egui::Event::PointerMoved(under),
        egui::Event::PointerButton {
            pos: under,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: under,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ] {
        kit.event(event);
        kit.run_steps(1);
    }
    kit.run_steps(3);
    assert_eq!(kit.state().screen, crate::screens::Screen::Contacts);
}
