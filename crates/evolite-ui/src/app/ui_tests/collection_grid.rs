//! BUG-188: the Collection's grid of tiles 148–196, its panel only from content of 952 (a window
//! of 1280), and copies of one model as one tile «×N» with their card.

use super::*;
use crate::screens::collection::{TILE_MAX, TILE_MIN, groups};

/// The tiles on screen, each with its rect: one per group of the shown NFTs, named as the
/// screen names them («Djembe · ×3» for a group).
fn tiles(kit: &Kit) -> Vec<(String, egui::Rect)> {
    let nfts = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.nfts.clone())
        .unwrap_or_default();
    let all: Vec<usize> = (0..nfts.len()).collect();
    groups(&nfts, &all)
        .iter()
        .map(|g| {
            let name = &nfts[g[0]].name;
            if g.len() > 1 {
                format!("{name} · ×{}", g.len())
            } else {
                name.clone()
            }
        })
        .filter_map(|label| {
            let rect = kit
                .query_by(|n| {
                    n.role() == Role::Button && n.label().as_deref() == Some(label.as_str())
                })?
                .rect();
            Some((label, rect))
        })
        .collect()
}

/// How many tiles stand in the first row, and their width.
fn first_row(kit: &Kit) -> (usize, f32) {
    let tiles = tiles(kit);
    let top = tiles
        .iter()
        .map(|(_, r)| r.top())
        .fold(f32::INFINITY, f32::min);
    let row: Vec<_> = tiles
        .iter()
        .filter(|(_, r)| (r.top() - top).abs() < 1.0)
        .collect();
    (row.len(), row.first().map_or(0.0, |(_, r)| r.width()))
}

/// AC-D1, AC-D2: at 1024, five tiles in a row, no card beside the grid; at 800, four; at 1280,
/// four beside the panel, which shows the first tile's card without a click.
#[test]
fn the_grid_has_the_designers_columns_and_the_panel_only_from_1280() {
    let (_turn, mut kit) = demo_app("collection", "en");
    wait_for(&mut kit, "the tiles", |kit| tiles(kit).len() > 5);
    let (cols, width) = first_row(&kit);
    assert_eq!(cols, 5, "1024: five columns");
    assert!((TILE_MIN - 4.0..=TILE_MAX).contains(&width), "{width}");
    assert!(!has_text(&kit, "COPIES"), "1024: no card beside the grid");

    kit.set_size(egui::vec2(800.0, 600.0));
    kit.run_steps(3);
    assert_eq!(first_row(&kit).0, 4, "800: four columns");
    assert!(!has_text(&kit, "COPIES"));

    kit.set_size(egui::vec2(1280.0, 800.0));
    kit.run_steps(3);
    assert_eq!(first_row(&kit).0, 4, "1280: four beside the panel");
    assert!(
        has_text(&kit, "COPIES"),
        "the panel shows the first group's card"
    );
    assert!(has_text(&kit, "Djembe · ×3"));
}

/// AC-D4: three copies of one model are one tile «×3» (its painted numbers and «1 for sale of
/// 3» are tested in `screens::collection`); the header still counts copies. Under 1280 a click opens the group's card as a window: each
/// copy, its price or «not for sale», one hash mark, the actions on N.
#[test]
fn copies_of_one_model_are_one_tile_and_open_their_card() {
    let (_turn, mut kit) = demo_app("collection", "en");
    wait_for(&mut kit, "the tiles", |kit| tiles(kit).len() > 5);
    let copies = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map_or(0, |w| w.nfts.len());
    assert!(
        has_text(&kit, &copies.to_string()),
        "the header counts copies"
    );
    let names: Vec<String> = tiles(&kit).into_iter().map(|(name, _)| name).collect();
    assert!(names.len() < copies, "fewer tiles than copies: {names:?}");
    assert!(names.contains(&"Djembe · ×3".to_string()), "{names:?}");
    assert!(names.contains(&"InstantSend · ×4".to_string()), "{names:?}");

    // Tall enough for the whole card, still narrower than 1280: a window.
    kit.set_size(egui::vec2(1024.0, 1100.0));
    kit.run_steps(2);
    kit.get_by_label("Djembe · ×3").click();
    wait_for(&mut kit, "the group's card", |kit| has_text(kit, "COPIES"));
    assert!(has_words(&kit, "You have 3 · 1 for sale at"));
    assert!(has_words(&kit, "2 not for sale"));
    assert!(has_words(&kit, "№ 40 · from @alice"));
    assert!(has_words(&kit, "№ 77 · issue jembe"));
    for action in ["Give…", "Put up for sale…", "Take off sale…", "Delete…"] {
        assert!(has_text(&kit, action), "{action}");
    }
}

/// BUG-188 §5: one row — the tab «Collection», then the filters «Jembe · gifts» and «For sale»,
/// each on and off again; no «All» among them (FEAT-0055's «All» is the identities', in its own
/// row above). «For sale» groups only the copies for sale.
#[test]
fn the_filters_follow_the_collection_tab_and_turn_off_again() {
    let (_turn, mut kit) = demo_app("collection", "en");
    wait_for(&mut kit, "the tiles", |kit| tiles(kit).len() > 5);
    let filters = kit.get_by_label("For sale").rect();
    for all in kit.query_all_by_label("All") {
        assert!(
            all.rect().bottom() <= filters.top(),
            "no «All» in the filters' row: {:?} beside {filters:?}",
            all.rect()
        );
    }
    let all = tiles(&kit).len();

    kit.get_by_label("For sale").click();
    kit.run_steps(3);
    let names: Vec<String> = kit
        .query_all_by_role(Role::Button)
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    assert!(names.contains(&"InstantSend · ×2".to_string()), "{names:?}");
    assert!(
        !names.iter().any(|n| n.starts_with("Coffin Dance")),
        "{names:?}"
    );

    kit.get_by_label("For sale").click();
    kit.run_steps(3);
    assert_eq!(tiles(&kit).len(), all, "off again: all of it");
}
