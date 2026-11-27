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

/// BUG-188 §5: one row — the tab «Collection», then the filters «Received» and «For sale»,
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

/// BUG-282: a group of copies from an issuer nobody trusted — its card (the window at 1024, the
/// panel at 1280) draws no picture and never asks the core for it, and offers «Always show from
/// …» as one copy's card does.
#[test]
fn a_strangers_group_card_draws_no_picture_until_asked() {
    for (w, h) in [(1024.0, 1100.0), (1280.0, 1000.0)] {
        let (_turn, mut kit) = demo_app("collection", "en");
        wait_for(&mut kit, "the tiles", |kit| tiles(kit).len() > 5);
        let art = kit
            .state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .and_then(|w| w.nfts.iter().find(|n| n.name == "Djembe"))
            .map(|n| n.art.clone())
            .expect("Djembe");
        let stranger = |w: &mut evolite_api::Wallet| {
            for n in w.nfts.iter_mut().filter(|n| n.name == "Djembe") {
                n.issuer = "stranger".into();
                n.chosen = false;
            }
        };
        show_wallet_edited(&mut kit, 3, "Djembe from a stranger", stranger, |_| true);
        kit.set_size(egui::vec2(w, h));
        kit.run_steps(2);
        // At 1024 a click opens the card's window; at 1280 the panel shows the first group
        // (Djembe) already — and a click there would land on the tile's own «Show».
        if w < 1280.0 {
            kit.get_by_role_and_label(Role::Button, "Djembe · ×3")
                .click();
        }
        wait_for(&mut kit, "the group's card", |kit| has_text(kit, "COPIES"));
        // In a frame drawn with the stranger's copies (a demo snapshot between frames is
        // jembe's, trusted, and may draw it): «Always show from …», and no picture painted at
        // the card's 72 × 72 — not even marked as one.
        let _ = &art;
        show_wallet_edited(&mut kit, 3, "the stranger's card", stranger, |kit| {
            has_text(kit, "Always show from stranger")
        });
        assert!(
            picture_marks(&kit)
                .iter()
                .all(|r| (r.width() - 72.0).abs() > 1.0),
            "{w}: the group's picture painted before consent: {:?}",
            picture_marks(&kit)
        );
    }
}

/// The pictures painted in the last frame ([`crate::art::PICTURE_MARK`]): their rects.
fn picture_marks(kit: &Kit) -> Vec<egui::Rect> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(r) if r.blur_width == crate::art::PICTURE_MARK => out.push(r.rect),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &kit.output().shapes {
        walk(&c.shape, &mut out);
    }
    out
}

/// QA's kestrel7: a model issued by an identity without a name, its copies listed newest first.
const ISSUER: &str = "6yGSpSLh8tRoffdQFnnKKutfJDYMqonrZRip39Wt9rso";

fn djembe_as_qa_saw_it(kit: &mut Kit) {
    show_wallet_edited(
        kit,
        3,
        "Djembe issued by an id, 77 first",
        |w| {
            let mut djembe: Vec<_> = w
                .nfts
                .iter()
                .filter(|n| n.name == "Djembe")
                .cloned()
                .collect();
            if djembe.first().and_then(|n| n.edition.0) == Some(77) {
                return;
            }
            w.nfts.retain(|n| n.name != "Djembe");
            djembe.reverse();
            for n in &mut djembe {
                n.origin = evolite_api::NftOrigin::Release;
                n.issuer = ISSUER.into();
                // kestrel7's own model: its picture is drawn at once, no «Show».
                n.chosen = true;
                n.price = None;
            }
            w.nfts.splice(0..0, djembe);
        },
        |_| true,
    );
}

/// The copies' rows of the open card: each «№ …» label and the «not for sale» beside it.
fn copy_rows(kit: &Kit) -> Vec<(String, egui::Rect, egui::Rect)> {
    let states: Vec<egui::Rect> = kit
        .query_all_by(|n| n.role() == Role::Label && n.value().as_deref() == Some("not for sale"))
        .map(|n| n.rect())
        .collect();
    kit.query_all_by(|n| n.role() == Role::Label && n.value().is_some_and(|v| v.starts_with("№ ")))
        .filter_map(|n| {
            let rect = n.rect();
            let state = states
                .iter()
                .find(|s| (s.center().y - rect.center().y).abs() < 4.0)?;
            Some((n.value().unwrap_or_default(), rect, *state))
        })
        .collect()
}

/// BUG-309: in the window at 1024 and in the panel at 1280 the copies go by number, the issuer's
/// id is shortened, and the row's words keep 8 or more from «not for sale».
#[test]
fn a_groups_copies_go_by_number_with_the_issuers_id_short() {
    let (_turn, mut kit) = demo_app("collection", "en");
    wait_for(&mut kit, "the tiles", |kit| tiles(kit).len() > 5);
    djembe_as_qa_saw_it(&mut kit);
    // FEAT-0051 R-5: the id whole where it fits, else its ends around «…», at least 4 each.
    let the_id = |shown: &str| {
        shown == ISSUER
            || shown.contains('…')
                && shown.starts_with(&ISSUER[..4])
                && shown.ends_with(&ISSUER[ISSUER.len() - 4..])
    };
    let check = |kit: &Kit, at: &str| {
        let rows = copy_rows(kit);
        let numbers: Vec<&str> = rows
            .iter()
            .map(|(w, _, _)| w.split(" · ").next().unwrap_or_default())
            .collect();
        assert_eq!(numbers, ["№ 12", "№ 40", "№ 77"], "{at}: by number");
        for (w, _, _) in &rows {
            let id = w.split(" · issue ").nth(1).unwrap_or_default();
            assert!(the_id(id), "{at}: «{w}»");
        }
        for (w, row, state) in &rows {
            assert!(
                state.left() - row.right() >= 8.0,
                "{at}: «{w}» {row:?} against {state:?}"
            );
        }
    };
    kit.set_size(egui::vec2(1024.0, 1100.0));
    kit.run_steps(2);
    open_djembe(&mut kit);
    wait_for(&mut kit, "the group's card", |kit| has_text(kit, "COPIES"));
    check(&kit, "1024");
    kit.key_press(egui::Key::Escape);
    kit.set_size(egui::vec2(1280.0, 800.0));
    kit.run_steps(3);
    open_djembe(&mut kit);
    kit.run_steps(3);
    check(&kit, "1280");
}

/// BUG-308: the card's actions on N are off — said so under them, not what they will do once
/// they work; «Delete…» too, with no «Asks “How many of 3”».
#[test]
fn a_groups_actions_say_they_are_not_there_yet() {
    let (_turn, mut kit) = demo_app("collection", "en");
    wait_for(&mut kit, "the tiles", |kit| tiles(kit).len() > 5);
    for (w, h) in [(1024.0, 1100.0), (1280.0, 1000.0)] {
        kit.set_size(egui::vec2(w, h));
        kit.run_steps(2);
        open_djembe(&mut kit);
        wait_for(&mut kit, "the group's card", |kit| has_text(kit, "COPIES"));
        for action in ["Give…", "Put up for sale…", "Take off sale…", "Delete…"] {
            assert!(
                button(&kit, action).accesskit_node().is_disabled(),
                "{w}: {action}"
            );
        }
        assert!(
            has_words(&kit, "Actions on several copies come in the next version"),
            "{w}: said under them"
        );
        assert!(!has_words(&kit, "How many of"), "{w}: no promise");
        kit.key_press(egui::Key::Escape);
        kit.run_steps(2);
    }
}

/// A click on the tile «Djembe · ×3» (the button; at 1280 the panel's title has the same words),
/// then the group's card.
fn open_djembe(kit: &mut Kit) {
    kit.get_by_role_and_label(Role::Button, "Djembe · ×3")
        .click();
    let shown = |kit: &Kit| has_text(kit, "COPIES");
    for _ in 0..200 {
        if shown(kit) {
            return;
        }
        kit.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let buttons: Vec<String> = kit
        .query_all_by_role(Role::Button)
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    panic!("no group card; buttons: {buttons:?}");
}

/// BUG-308 r1 (Should): a group of the shown identity's own model says where one copy can be
/// given now — the model's page in the Workshop; another's model does not.
#[test]
fn an_own_models_card_says_where_one_copy_is_given_now() {
    const NOW: &str = "on the model's page in the Workshop";
    let (_turn, mut kit) = demo_app("collection", "en");
    wait_for(&mut kit, "the tiles", |kit| tiles(kit).len() > 5);
    kit.set_size(egui::vec2(1024.0, 1100.0));
    kit.run_steps(2);
    open_djembe(&mut kit);
    assert!(!has_words(&kit, NOW), "jembe's model, not @satoshi's");
    kit.key_press(egui::Key::Escape);
    kit.run_steps(2);
    let me = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .and_then(|w| w.identity.as_ref())
        .map(|i| i.id.clone())
        .expect("the shown identity");
    show_wallet_edited(
        &mut kit,
        3,
        "Djembe issued by the shown identity",
        move |w| {
            for n in w.nfts.iter_mut().filter(|n| n.name == "Djembe") {
                n.issuer = me.clone();
                n.chosen = true;
            }
        },
        |_| true,
    );
    open_djembe(&mut kit);
    assert!(has_words(&kit, NOW), "its own model");
}
