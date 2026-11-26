//! BUG-265, FEAT-0057 П1: one identity picker on Identity, Tokens, the Workshop and the
//! Collection. Up to four identities, its chips; from five, a button and a list with search
//! (R-1.1). On the hard preset of 10 long-named identities (werner555's wallet, TEAM.md §8d) the
//! button and the list stay within the window, every identity is there to choose, and the
//! Workshop's «Create NFT» stays in reach. Words cut or drawn over others and a page wider than
//! the window: `smallest_window`'s `text_trouble` on the same presets, the list open.

use super::*;

/// The whole names of the preset's identities (the screen reader's, FEAT-0051 R-5).
pub(super) fn names() -> Vec<String> {
    let first = evolite_api::IdentityChoice {
        id: "first".into(),
        name: None,
        slot: 0,
        ..Default::default()
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

/// The list's button: the one named «{label}: {the shown identity}» — or «…: All» on the
/// Collection — in any language.
pub(super) fn list_button(kit: &Kit) -> Option<egui::Rect> {
    let all = format!(": {}", crate::i18n::spoken("All"));
    kit.query_all_by_role(Role::Button)
        .find(|b| {
            b.accesskit_node()
                .label()
                .is_some_and(|l| l.contains(": @werner555-") || l.ends_with(&all))
        })
        .map(|b| b.rect())
}

/// The preset is drawn with its 10 identities: the list's button there.
pub(super) fn all_pills(kit: &Kit) -> bool {
    list_button(kit).is_some()
}

/// The words of `name`'s row in the open list: «{name} · {credits} · {state}».
pub(super) fn row_words(kit: &Kit, name: &str) -> Option<String> {
    let start = format!("{name} · ");
    kit.query_all_by_role(Role::Button)
        .filter_map(|b| b.accesskit_node().label().map(|l| l.to_string()))
        .find(|l| l.starts_with(&start))
}

/// `name`'s row in the open list.
pub(super) fn row_rect(kit: &Kit, name: &str) -> Option<egui::Rect> {
    let start = format!("{name} · ");
    kit.query_all_by_role(Role::Button)
        .find(|b| {
            b.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with(&start))
        })
        .map(|b| b.rect())
}

/// A click on `name`'s row in the open list.
pub(super) fn click_row(kit: &mut Kit, name: &str) {
    let start = format!("{name} · ");
    kit.query_all_by_role(Role::Button)
        .find(|b| {
            b.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with(&start))
        })
        .unwrap_or_else(|| panic!("no row of {name}"))
        .click();
    kit.run_steps(3);
}

/// A click on the list's button: open (or closed again).
pub(super) fn open_list(kit: &mut Kit) {
    let at = list_button(kit).expect("the list's button");
    kit.query_all_by_role(Role::Button)
        .find(|b| b.rect() == at)
        .expect("the button")
        .click();
    kit.run_steps(3);
}

/// The list's search field, when the list is open.
fn search(kit: &Kit) -> Option<egui::Rect> {
    kit.query_all_by_role(Role::TextInput)
        .next()
        .map(|f| f.rect())
}

/// Types `text` into the open list's search, emptied first.
fn search_for(kit: &mut Kit, text: &str) {
    for _ in 0..40 {
        kit.key_press(egui::Key::Backspace);
    }
    kit.query_all_by_role(Role::TextInput)
        .next()
        .expect("the search")
        .type_text(text);
    kit.run_steps(3);
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

/// A1.6, R-1.6: at every size the button and the open list within the window — no wider, its
/// search not past the bottom — and every identity's row there (some scrolled to).
fn list_within(preset: &str, seen: &str) {
    let (_turn, mut kit) = demo_app(preset, "en");
    for size in SIZES {
        kit.set_size(size);
        wait_for(&mut kit, seen, |kit| has_words(kit, seen));
        wait_for(&mut kit, "the list's button", all_pills);
        kit.run_steps(3);
        let window = kit.ctx.content_rect();
        let button = list_button(&kit).expect("button");
        assert!(
            button.left() >= window.left() - 1.0 && button.right() <= window.right() + 1.0,
            "{preset} at {size:?}: the button at {button:?}"
        );
        open_list(&mut kit);
        for name in names() {
            let r = row_rect(&kit, &name).unwrap_or_else(|| panic!("{preset}: no row of {name}"));
            assert!(
                r.left() >= window.left() - 1.0 && r.right() <= window.right() + 1.0,
                "{preset} at {size:?}: {name} at {r:?}"
            );
        }
        let field = search(&kit).expect("the search");
        assert!(
            field.bottom() <= window.bottom(),
            "{preset} at {size:?}: the search at {field:?}"
        );
        kit.key_press(egui::Key::Escape);
        kit.run_steps(2);
    }
}

#[test]
fn the_identity_screen_lists_10_long_names_within_the_window() {
    list_within("identities-long", "Work as");
}

#[test]
fn tokens_list_10_long_names_within_the_window() {
    list_within("tokens-identities-long", "Identity");
}

#[test]
fn the_workshop_lists_10_long_names_and_its_buttons_stay_in_reach() {
    list_within("workshop-identities-long", "NFT collection");
    let (_turn, mut kit) = demo_app("workshop-identities-long", "en");
    for size in SIZES {
        kit.set_size(size);
        wait_for(&mut kit, "W1", |kit| has_text(kit, "NFT collection"));
        wait_for(&mut kit, "the list's button", all_pills);
        kit.run_steps(3);
        let window = kit.ctx.content_rect();
        let create = kit.get_by_role_and_label(Role::Button, "Create NFT").rect();
        assert!(
            create.right() <= window.right() + 1.0 && create.left() >= window.left() - 1.0,
            "«Create NFT» out of reach at {size:?}: {create:?}"
        );
    }
}

/// Only the Identity screen offers «+ New identity» — at the list's end.
#[test]
fn only_the_identity_screen_offers_a_new_identity() {
    for (preset, seen, offered) in [
        ("identities-long", "Work as", true),
        ("tokens-identities-long", "Identity", false),
        ("workshop-identities-long", "NFT collection", false),
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        wait_for(&mut kit, seen, |kit| has_words(kit, seen));
        wait_for(&mut kit, "the list's button", all_pills);
        open_list(&mut kit);
        assert_eq!(
            kit.query_by_label("Create a new identity").is_some(),
            offered,
            "{preset}"
        );
    }
}

/// A1.1: four identities — chips; five — the button and the list, on all four screens.
#[test]
fn four_are_chips_and_five_a_list_on_every_screen() {
    // The demo's own presets (its two identities), given 4 or 5 of the long preset's: the
    // long presets keep 10 by themselves.
    let second = names()[0].clone();
    let button = |kit: &Kit| {
        kit.query_all_by_role(Role::Button).any(|b| {
            b.accesskit_node()
                .label()
                .is_some_and(|l| l.ends_with(": @satoshi") || l.ends_with(": All"))
        })
    };
    for (preset, seen) in [
        ("identity", "Work as"),
        ("tokens", "Identity"),
        ("workshop", "NFT collection"),
        ("collection", "Collection"),
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        wait_for(&mut kit, seen, |kit| has_words(kit, seen));
        for (count, list) in [(4, false), (5, true)] {
            let second = second.clone();
            show_wallet_edited(
                &mut kit,
                3,
                "4 or 5 identities",
                move |w| {
                    if let Some(first) = w.identities.first().cloned() {
                        let mut all = crate::app::dev::long_identities(&first);
                        all[0] = first;
                        all.truncate(count);
                        w.identities = all;
                    }
                },
                move |kit| kit.query_by_label(&second).is_some() != list && button(kit) == list,
            );
        }
    }
}

/// A1.2, R-1.5: the search keeps the rows whose name has it (no «@», any case) or whose id
/// starts with it; nothing found is said in words.
#[test]
fn the_search_keeps_the_names_and_ids_found() {
    let (_turn, mut kit) = demo_app("collection-identities-long", "en");
    wait_for(&mut kit, "the list's button", all_pills);
    open_list(&mut kit);
    search_for(&mut kit, "NUMBER-3");
    assert!(row_words(&kit, "@werner555-identity-number-3-with-a-long-name").is_some());
    assert!(row_words(&kit, "@werner555-identity-number-4-with-a-long-name").is_none());
    search_for(&mut kit, "9Hk2");
    assert!(
        row_words(&kit, "9Hk2Wm4nPq8rSt3uVw5xYz6aBc9dEf2gHj7kLm1n9QwP").is_some(),
        "the nameless one by its id's start"
    );
    search_for(&mut kit, "zzz");
    assert!(has_words(&kit, "No identities for"), "R-1.5");
}

/// A1.3, A1.7: a row's state by R-1.2 — one identity in each — and «checking…» with «—» for
/// the one not read yet, never «0».
#[test]
fn each_row_says_its_identitys_state() {
    let (_turn, mut kit) = demo_app("collection-identities-long", "en");
    wait_for(&mut kit, "the list's button", all_pills);
    open_list(&mut kit);
    let words = |kit: &Kit, n: u32| {
        let name = format!("@werner555-identity-number-{n}-with-a-long-name");
        row_words(kit, &name).unwrap_or_else(|| panic!("no row {n}"))
    };
    for (n, state) in [
        (1, "no keys of this wallet"),
        (2, "needs a sign-in key"),
        (3, "needs a DashPay key"),
        (4, "ready"),
    ] {
        assert!(words(&kit, n).ends_with(state), "{}", words(&kit, n));
    }
    let unread = words(&kit, 5);
    assert!(unread.ends_with(" · — · checking…"), "{unread}");
    let nameless = row_words(&kit, "9Hk2Wm4nPq8rSt3uVw5xYz6aBc9dEf2gHj7kLm1n9QwP").expect("row");
    assert!(nameless.ends_with("no name"), "{nameless}");
}

/// A1.5: under «Hide amounts» the rows' credits are dots.
#[test]
fn hidden_amounts_hide_the_rows_credits() {
    let (_turn, mut kit) = demo_app("collection-identities-long", "en");
    wait_for(&mut kit, "the list's button", all_pills);
    kit.state_mut().settings.set("hide-amounts", true);
    kit.run_steps(2);
    open_list(&mut kit);
    let words = row_words(&kit, "@werner555-identity-number-4-with-a-long-name").expect("row");
    let credits = words.split(" · ").nth(1).unwrap_or_default();
    assert!(
        credits.contains('•') && !credits.chars().any(|c| c.is_ascii_digit()),
        "{words}"
    );
}

/// A1.4, UC-1.3, UC-1.4: Enter on the focused button opens the list, Esc closes it without a
/// choice, the focus back on the button; ↓ goes to the next row and Enter chooses it, the list
/// closed. On the Collection the choice is its filter («All» the first row).
#[test]
fn the_keyboard_opens_moves_and_chooses() {
    let (_turn, mut kit) = demo_app("collection-identities-long", "en");
    wait_for(&mut kit, "the list's button", all_pills);
    tab_to(&mut kit, "Identity: ");
    enter(&mut kit);
    assert!(search(&kit).is_some(), "open");
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    assert!(search(&kit).is_none(), "closed");
    assert!(
        focused(&kit).is_some_and(|f| f.starts_with("Identity: ")),
        "the focus on the button: {:?}",
        focused(&kit)
    );
    assert!(kit.state().collection.only.is_none(), "nothing chosen");
    enter(&mut kit);
    kit.key_press(egui::Key::ArrowDown);
    kit.run_steps(2);
    enter(&mut kit);
    assert!(kit.state().collection.only.is_some(), "an identity chosen");
    assert!(search(&kit).is_none(), "closed");
}

/// FEAT-0057 П1 r1 (Must): the open list asks for every identity's credits once at its opening
/// — not each frame (a storm of reads) — and once more when it is opened again.
#[test]
fn the_open_list_asks_for_the_credits_once() {
    let (_turn, mut kit) = demo_app("collection-identities-long", "en");
    wait_for(&mut kit, "the list's button", all_pills);
    let asked = || crate::components::LIST_ASKED.with(std::cell::Cell::get);
    let before = asked();
    open_list(&mut kit);
    kit.run_steps(60);
    assert_eq!(asked() - before, 1, "60 frames open: one ask");
    kit.key_press(egui::Key::Escape);
    kit.run_steps(3);
    open_list(&mut kit);
    kit.run_steps(10);
    assert_eq!(asked() - before, 2, "opened again: one more");
}
