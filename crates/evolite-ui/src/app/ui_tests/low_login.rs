//! BUG-140, BUG-149: the login screens in a low window. Below 640 px high the logo is compact
//! (the mark 40 with the word beside it), the screen starts 24 from the top, and a fade at its
//! foot says there is more below: «Open» and «Create a wallet» are in sight without scrolling.
//! From 640 px they are as before. The add-wallet welcome starts under its top bar.

use super::*;
use crate::screens::welcome::{ADDING_BAR, COMPACT_LOGO};

/// The logo the login screen drew last frame, and whether it was the compact one.
fn logo(kit: &Kit) -> Option<(egui::Rect, bool)> {
    kit.ctx
        .data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new("evolite-login-logo")))
}

/// The fade at the screen's foot, if it was painted last frame.
fn fade(kit: &Kit) -> Option<egui::Rect> {
    kit.ctx
        .data(|d| d.get_temp::<Option<egui::Rect>>(egui::Id::new("evolite-login-fade")))
        .flatten()
}

/// The rect of the first widget named or worded `name`.
fn rect_of(kit: &Kit, name: &str) -> Option<egui::Rect> {
    kit.query_all_by(|n| n.label().as_deref() == Some(name) || n.value().as_deref() == Some(name))
        .next()
        .map(|n| n.rect())
}

/// Wholly in the window, not only in the tree.
fn in_sight(kit: &Kit, name: &str) -> bool {
    let window = kit.ctx.content_rect();
    rect_of(kit, name).is_some_and(|r| window.contains_rect(r))
}

/// A preset at a window size, settled.
fn at(preset: &str, lang: &str, size: egui::Vec2) -> (MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app(preset, lang);
    kit.set_size(size);
    kit.run_steps(4);
    settle(&mut kit);
    (turn, kit)
}

const LOW: egui::Vec2 = egui::vec2(800.0, 440.0);

/// AC-D: in 800×440 «Open» (three wallets) and «Create a wallet» are in sight without scrolling,
/// under the compact logo, with the fade at the foot (there is more below). In ru and ar too.
#[test]
fn in_the_smallest_window_the_main_button_is_in_sight_under_a_compact_logo() {
    for lang in ["en", "ru", "ar"] {
        for (preset, main) in [("wallet-unlock", "Open"), ("welcome", "Create a wallet")] {
            let (_turn, kit) = at(preset, lang, LOW);
            let main = crate::i18n::spoken(main);
            assert!(
                in_sight(&kit, &main),
                "{lang} {preset}: «{main}» below the edge"
            );
            let (rect, compact) = logo(&kit).expect("a logo");
            assert!(compact, "{lang} {preset}: the big logo");
            assert!(
                (rect.height() - COMPACT_LOGO).abs() < 0.5,
                "{lang} {preset}: the logo is {} high",
                rect.height()
            );
            assert!(
                (rect.top() - kit.ctx.content_rect().top() - 24.0).abs() < 1.0,
                "{lang} {preset}: the logo at {}, not 24 from the top",
                rect.top()
            );
            assert!(
                fade(&kit).is_some(),
                "{lang} {preset}: no fade with more below"
            );
        }
    }
}

/// AC-D: from 640 px high the login screens are as before — the stacked logo, centred, no fade
/// when all of it fits.
#[test]
fn from_640_high_the_login_screens_are_as_before() {
    for size in [egui::vec2(800.0, 640.0), egui::vec2(1024.0, 768.0)] {
        for (preset, height) in [("wallet-unlock", 96.0), ("welcome", 119.0)] {
            let (_turn, kit) = at(preset, "en", size);
            let (rect, compact) = logo(&kit).expect("a logo");
            assert!(!compact, "{size:?} {preset}: the compact logo");
            assert!(
                (rect.height() - height).abs() < 0.5,
                "{size:?} {preset}: the logo is {} high",
                rect.height()
            );
        }
    }
    let (_turn, kit) = at("welcome", "en", egui::vec2(1024.0, 768.0));
    assert!(fade(&kit).is_none(), "a fade over a screen that fits");
}

/// The fade goes once the screen is scrolled to its end: nothing more below.
#[test]
fn the_fade_goes_at_the_end_of_the_screen() {
    let (_turn, mut kit) = at("wallet-unlock", "en", LOW);
    assert!(fade(&kit).is_some(), "no fade at the top");
    let centre = kit.ctx.content_rect().center();
    for _ in 0..30 {
        if fade(&kit).is_none() {
            break;
        }
        kit.hover_at(centre);
        kit.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        kit.run_steps(3);
    }
    assert!(fade(&kit).is_none(), "the fade stays at the end");
}

/// BUG-149: «A new wallet on this computer» in 800×440 starts under the top bar: under «‹ Back
/// to …» and the bar's logo, not drawn over by them. Also in 1024×768.
#[test]
fn the_add_wallet_title_is_under_its_top_bar() {
    for (lang, size) in [("ru", LOW), ("en", LOW), ("ru", egui::vec2(1024.0, 768.0))] {
        let (_turn, kit) = at("wallet-add", lang, size);
        let title = crate::i18n::spoken("A new wallet on this computer");
        let top = kit.ctx.content_rect().top();
        let title = rect_of(&kit, &title).expect("the title");
        assert!(
            title.top() >= top + ADDING_BAR - 0.5,
            "{lang} {size:?}: the title at {}, under the bar's {ADDING_BAR}",
            title.top()
        );
        // The bar's own widgets: «‹ Back to …» (its words) and the network chip, at its top.
        let bar: Vec<egui::Rect> = kit
            .query_all_by(|n| n.label().is_some() || n.value().is_some())
            .map(|n| n.rect())
            .filter(|r| r.top() < top + 40.0)
            .collect();
        assert!(!bar.is_empty(), "{lang} {size:?}: «‹ Back to …» not found");
        for back in bar {
            assert!(
                back.bottom() < title.top(),
                "{lang} {size:?}: the top bar's {back:?} runs into the title {title:?}"
            );
        }
        assert!(
            in_sight(&kit, &crate::i18n::spoken("Create a wallet")),
            "{lang} {size:?}: «Create a wallet» below the edge"
        );
    }
}

/// The designer's BUG-140 review: in 800×440 the wallets are 8 under «Which wallet to open?»,
/// and «Open» is still in sight — the 4 come from the room above it, and the error's line under
/// the password keeps its place.
#[test]
fn in_the_smallest_window_the_wallets_are_8_under_the_title() {
    for lang in ["en", "ru", "ar"] {
        let (_turn, kit) = at("wallet-unlock", lang, LOW);
        let first = kit
            .query_all_by(|n| n.role() == Role::RadioButton)
            .map(|n| n.rect())
            .min_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("a wallet");
        // The title: the label right above the first wallet (in ar a label's value is its
        // words in visual order, not the ones `spoken` gives).
        let title = kit
            .query_all_by(|n| n.role() == Role::Label)
            .map(|n| n.rect())
            .filter(|r| r.bottom() <= first.top())
            .max_by(|a, b| a.bottom().total_cmp(&b.bottom()))
            .unwrap_or_else(|| panic!("{lang}: the title"));
        let gap = first.top() - title.bottom();
        assert!((gap - 8.0).abs() < 1.0, "{lang}: {gap} under the title");
        assert!(
            in_sight(&kit, &crate::i18n::spoken("Open")),
            "{lang}: «Open» below the edge"
        );
    }
}
