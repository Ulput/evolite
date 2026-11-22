//! BUG-265: one identity picker on Identity, Tokens and the Workshop. On the hard preset of 10
//! long-named identities (werner555's wallet, TEAM.md §8d) every identity is there to choose,
//! its pill whole within the window, and the Workshop's «Create NFT» stays in reach. Words cut
//! or drawn over others and a page wider than the window: `smallest_window`'s `text_trouble`
//! on the same presets.

use super::*;

/// The whole names of the preset's identities (the screen reader's, FEAT-0051 R-5).
pub(super) fn names() -> Vec<String> {
    let first = evolite_api::IdentityChoice {
        id: "first".into(),
        name: None,
        slot: 0,
    };
    crate::app::dev::long_identities(&first)
        .iter()
        .skip(1)
        .map(crate::components::identity_full_name)
        .chain(std::iter::once(
            "@werner555-the-first-identity-with-a-very-long-name".to_string(),
        ))
        .collect()
}

pub(super) fn all_pills(kit: &Kit) -> bool {
    names().iter().all(|n| kit.query_by_label(n).is_some())
}

/// TEAM.md §8d: the window's widths, the smallest one at its height.
pub(super) const SIZES: [egui::Vec2; 6] = [
    egui::vec2(800.0, 440.0),
    egui::vec2(1024.0, 768.0),
    egui::vec2(1150.0, 768.0),
    egui::vec2(1279.0, 800.0),
    egui::vec2(1280.0, 800.0),
    egui::vec2(1920.0, 1080.0),
];

/// Every identity's pill whole within the window at every size.
fn pills_within(preset: &str, seen: &str) {
    let (_turn, mut kit) = demo_app(preset, "en");
    for size in SIZES {
        kit.set_size(size);
        wait_for(&mut kit, seen, |kit| has_words(kit, seen));
        wait_for(&mut kit, "10 identities", all_pills);
        kit.run_steps(3);
        let window = kit.ctx.content_rect();
        for name in names() {
            let pill = kit.get_by_label(&name).rect();
            assert!(
                pill.left() >= window.left() - 1.0 && pill.right() <= window.right() + 1.0,
                "{preset} at {size:?}: {name} at {pill:?}"
            );
        }
    }
}

#[test]
fn the_identity_screen_wraps_10_long_names() {
    pills_within("identities-long", "Work as");
}

#[test]
fn tokens_wrap_10_long_names() {
    pills_within("tokens-identities-long", "Identity");
}

#[test]
fn the_workshop_wraps_10_long_names_and_its_buttons_stay_in_reach() {
    pills_within("workshop-identities-long", "NFT collection");
    let (_turn, mut kit) = demo_app("workshop-identities-long", "en");
    for size in SIZES {
        kit.set_size(size);
        wait_for(&mut kit, "W1", |kit| has_text(kit, "NFT collection"));
        wait_for(&mut kit, "10 identities", all_pills);
        kit.run_steps(3);
        let window = kit.ctx.content_rect();
        let create = kit.get_by_role_and_label(Role::Button, "Create NFT").rect();
        assert!(
            create.right() <= window.right() + 1.0 && create.left() >= window.left() - 1.0,
            "«Create NFT» out of reach at {size:?}: {create:?}"
        );
    }
}

/// Only the Identity screen offers «+ New identity» beside the identities.
#[test]
fn only_the_identity_screen_offers_a_new_identity() {
    for (preset, seen, offered) in [
        ("identities-long", "Work as", true),
        ("tokens-identities-long", "Identity", false),
        ("workshop-identities-long", "NFT collection", false),
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        wait_for(&mut kit, seen, |kit| has_words(kit, seen));
        wait_for(&mut kit, "10 identities", all_pills);
        assert_eq!(
            kit.query_by_label("Create a new identity").is_some(),
            offered,
            "{preset}"
        );
    }
}
