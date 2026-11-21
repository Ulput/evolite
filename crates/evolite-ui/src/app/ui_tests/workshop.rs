//! FEAT-0038 W0–W4 on the demo core: the Workshop behind its own switch (AC-A1), the main page
//! (AC-A2), a new collection checked before the password and made with one (AC-A4), and its
//! page (W4).

use super::*;

/// Types `text` into the field named `label`: focused first, as a person clicks into it.
fn type_into(kit: &mut Kit, label: &str, text: &str) {
    kit.get_by_role_and_label(Role::TextInput, label).click();
    kit.run_steps(2);
    kit.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    kit.run_steps(2);
}

fn workshop_in_menu(kit: &Kit) -> bool {
    // By its name: in the rail (below 1280) only its icon is painted.
    kit.query_all_by_label("Workshop").next().is_some()
}

/// AC-A1: no Workshop in the menu by default; «Show the Workshop» in Settings › Advanced shows
/// it after the rest; off again, it goes, and its open page with it.
#[test]
fn the_workshop_is_behind_its_own_switch() {
    let (_turn, mut kit) = demo_app("advanced", "en");
    wait_for(&mut kit, "W0", |kit| has_text(kit, "Show the “Workshop”"));
    assert!(!kit.state().settings.is_on("workshop"), "off by default");
    assert!(!workshop_in_menu(&kit), "no menu item while off");
    kit.get_by_role_and_label(Role::CheckBox, "Show the “Workshop”")
        .click();
    kit.run_steps(2);
    assert!(kit.state().settings.is_on("workshop"), "no password asked");
    assert!(
        !kit.state().settings.is_on("advanced"),
        "not the shared switch"
    );
    wait_for(&mut kit, "the menu item", workshop_in_menu);
    kit.get_by_label("Workshop").click();
    wait_for(&mut kit, "W1", |kit| has_text(kit, "NFT collection"));
    assert_eq!(kit.state().screen, crate::screens::Screen::Workshop);
    kit.state_mut().settings.set("workshop", false);
    wait_for(&mut kit, "back to the Overview", |kit| {
        kit.state().screen == crate::screens::Screen::Overview
    });
    kit.run_steps(2);
    assert!(
        !workshop_in_menu(&kit),
        "the menu item went: {:?}",
        kit.query_all_by_label("Workshop")
            .map(|n| format!("{:?}", n.accesskit_node().role()))
            .collect::<Vec<_>>()
    );
}

/// AC-A2: the cards «Create» on top, the identity's credits, one's collections; the token's
/// card waits for its wizard.
#[test]
fn the_main_page_shows_what_can_be_made_and_ones_collections() {
    let (_turn, mut kit) = demo_app("workshop", "en");
    wait_for(&mut kit, "the collections read", |kit| {
        has_text(kit, "Seasons of the city")
    });
    assert!(has_words(&kit, "Credits @satoshi: "));
    assert!(!button(&kit, "Create NFT").accesskit_node().is_disabled());
    assert!(
        button(&kit, "Create a token")
            .accesskit_node()
            .is_disabled()
    );
    assert!(has_text(&kit, "Creating tokens comes in the next version"));
    assert!(has_words(&kit, "1 model"));
}

/// AC-A4: a link in the name stops «Check» with the reason, before anything is asked.
#[test]
fn a_link_in_the_name_stops_the_check() {
    let (_turn, mut kit) = demo_app("workshop", "en");
    wait_for(&mut kit, "W1", |kit| has_text(kit, "NFT collection"));
    button(&kit, "Create NFT").click();
    wait_for(&mut kit, "W2", |kit| has_text(kit, "New NFT collection"));
    type_into(&mut kit, "Collection name", "Pins at pins.example.com");
    kit.run_steps(3);
    assert!(
        has_text(
            &kit,
            "No links or site addresses: people find the collection by its name."
        ),
        "{:?}",
        painted_texts(&kit)
    );
    assert!(button(&kit, "Check").accesskit_node().is_disabled());
}

/// AC-A4, UC-4: the name and «Belongs to a contract» checked by the core (the fee with the
/// «About» document, the contract's description), W3 with the warning and the password, «Sign
/// and create», then the new collection's page (W4) and it in the list.
#[test]
fn a_collection_is_checked_signed_and_opened() {
    let (_turn, mut kit) = demo_app("workshop", "en");
    wait_for(&mut kit, "W1", |kit| has_text(kit, "Seasons of the city"));
    button(&kit, "Create NFT").click();
    wait_for(&mut kit, "W2", |kit| has_text(kit, "Collection rules"));
    type_into(&mut kit, "Collection name", "Dash Pins");
    type_into(
        &mut kit,
        "Belongs to a contract · optional",
        "7Yowk46VwwHqmD5yZyyygggh937aP6h2UW7aQWBdWpM5",
    );
    wait_for(&mut kit, "the core's check", |kit| {
        !button(kit, "Check").accesskit_node().is_disabled()
    });
    assert!(
        has_text(&kit, "City game: items and rewards"),
        "the contract's description under the field"
    );
    // 0.1 + 0.06 + 0.03 + storage + the «About» document's estimate.
    assert!(has_words(&kit, "≈ 0.1950"), "{:?}", painted_texts(&kit));
    button(&kit, "Check").click();
    wait_for(&mut kit, "W3", |kit| has_text(kit, "Check the collection"));
    assert!(has_text(
        &kit,
        "Once signed, the collection cannot be changed or deleted. The registration is not refunded."
    ));
    assert!(
        button(&kit, "Sign and create")
            .accesskit_node()
            .is_disabled(),
        "not without the password"
    );
    kit.get_by_role(Role::PasswordInput).click();
    kit.run_steps(2);
    kit.get_by_role(Role::PasswordInput).type_text("demo");
    kit.run_steps(2);
    button(&kit, "Sign and create").click();
    wait_for(&mut kit, "W4", |kit| {
        matches!(
            kit.state().workshop.page,
            crate::screens::workshop::Page::Collection(_)
        ) && has_text(kit, "Dash Pins")
    });
    assert!(has_text(&kit, "About the collection"));
    assert!(!button(&kit, "New model").accesskit_node().is_disabled());
    assert!(has_words(&kit, "No models yet."));
    kit.get_by_label("Back: Workshop").click();
    wait_for(&mut kit, "in the list", |kit| {
        has_text(kit, "Seasons of the city") && has_text(kit, "Dash Pins")
    });
}

/// W4: the example collection's model with its numbers from the core — issued of the supply,
/// and how many one has.
#[test]
fn a_collection_page_shows_its_models_numbers() {
    let (_turn, mut kit) = demo_app("workshop-collection", "en");
    wait_for(&mut kit, "the numbers", |kit| {
        painted_has(kit, "issued 13 of 100")
    });
    assert!(painted_has(&kit, "Winter tram"));
    assert!(painted_has(&kit, "you have 12"));
    button(&kit, "About the collection").click();
    wait_for(&mut kit, "About", |kit| has_text(kit, "NFT evolite v1"));
}

/// A noisy PNG of `side` × `side`: too large for a record when the side is big.
fn noisy_png(side: u32) -> Vec<u8> {
    let mut seed = 0x9E37_79B9u32;
    let img = image::RgbaImage::from_fn(side, side, |_, _| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let b = seed.to_le_bytes();
        image::Rgba([b[0], b[1], b[2], 255])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// From W4 to W5 with `file` chosen as `name`.
fn new_model_with(kit: &mut Kit, name: &str, file: Vec<u8>) {
    wait_for(kit, "W4", |kit| painted_has(kit, "Winter tram"));
    button(kit, "New model").click();
    wait_for(kit, "W5", |kit| has_text(kit, "Choose a file"));
    *crate::screens::workshop::new_model::TEST_PICK
        .lock()
        .unwrap() = Some((name.into(), file));
    button(kit, "Choose a file").click();
    wait_for(kit, "the file", |kit| has_words(kit, name));
    type_into(kit, "Model name", "Orange pin");
}

/// UC-5a, AC-A19 (on the demo's quick copy): a PNG over the record's room gets the offer, not
/// the old refusal; the copy shows both sizes; «Use» puts it in the model and «Check» goes on to
/// the review with its password.
#[test]
fn a_picture_too_large_gets_a_smaller_copy() {
    let (_turn, mut kit) = demo_app("workshop-collection", "en");
    new_model_with(&mut kit, "pin.png", noisy_png(120));
    wait_for(&mut kit, "the offer", |kit| {
        has_words(kit, "evolite can make a smaller copy that fits")
    });
    assert!(button(&kit, "Check").accesskit_node().is_disabled());
    button(&kit, "Make a smaller copy").click();
    wait_for(&mut kit, "the copy", |kit| {
        has_words(kit, "Reduced: 512×341")
    });
    assert!(has_words(&kit, "Original file: 1200×800"));
    button(&kit, "Use").click();
    wait_for(&mut kit, "the copy checked", |kit| {
        !button(kit, "Check").accesskit_node().is_disabled()
    });
    button(&kit, "Check").click();
    wait_for(&mut kit, "the review", |kit| {
        has_text(kit, "Check the model")
    });
    assert!(
        button(&kit, "Sign and issue")
            .accesskit_node()
            .is_disabled(),
        "not without the password"
    );
}

/// R-22: a GLB's details stay, and W5 says so; a name that claims another type is refused.
#[test]
fn a_model_file_and_a_wrong_name_are_said() {
    let (_turn, mut kit) = demo_app("workshop-collection", "en");
    let mut glb = b"glTF".to_vec();
    glb.extend_from_slice(&[2, 0, 0, 0, 24, 0, 0, 0]);
    glb.extend_from_slice(b"{\"asset\":\"generator\"}  ");
    new_model_with(&mut kit, "cube.glb", glb);
    wait_for(&mut kit, "the GLB line", |kit| {
        has_words(kit, "Details inside a 3D model")
    });
    *crate::screens::workshop::new_model::TEST_PICK
        .lock()
        .unwrap() = Some(("photo.png".into(), vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 2]));
    button(&kit, "Choose another file").click();
    wait_for(&mut kit, "the mismatch", |kit| {
        has_words(kit, "This is not PNG, though the file is named .png.")
    });
    assert!(button(&kit, "Check").accesskit_node().is_disabled());
}

/// The `tr!` strings of `source`, as written (escapes kept as they are; none here need them).
fn marked(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("tr!(") {
        rest = &rest[at + 4..];
        let open = rest.trim_start();
        if let Some(text) = open.strip_prefix('"')
            && let Some(end) = text.find('"')
        {
            out.push(text[..end].to_owned());
        }
    }
    out
}

/// AC-A13 as the analyst put it (2026-11-21): no Workshop string promises the fee back — in
/// English «will be refunded», «refunded to», «get back», in Russian «вернётся», «вернём»,
/// «возвратим»; «refund» and «возвращ» only right after «not» / «не» («The registration is not
/// refunded» says a fact, not a promise).
#[test]
fn no_workshop_string_promises_a_refund() {
    let sources = [
        include_str!("../../screens/workshop/mod.rs"),
        include_str!("../../screens/workshop/new_collection.rs"),
        include_str!("../../screens/workshop/collection_page.rs"),
        include_str!("../../screens/workshop/new_model.rs"),
    ];
    let keys: Vec<String> = sources.iter().flat_map(|s| marked(s)).collect();
    assert!(keys.len() > 80, "{} strings found", keys.len());
    let only_negated = |text: &str, word: &str, not: &str| {
        text.match_indices(word)
            .all(|(i, _)| text[..i].ends_with(not))
    };
    for key in &keys {
        let en = key.to_lowercase();
        let ru = crate::i18n::t_in(crate::i18n::Lang::Ru, key).to_lowercase();
        for promise in ["will be refunded", "refunded to", "get back"] {
            assert!(!en.contains(promise), "{key}");
        }
        for promise in ["вернётся", "вернём", "возвратим"] {
            assert!(!ru.contains(promise), "{ru}");
        }
        assert!(only_negated(&en, "refund", "not "), "{key}");
        assert!(only_negated(&ru, "возвращ", "не "), "{ru}");
    }
    // The one string that says it, as a fact, is among them.
    assert!(keys.iter().any(|k| k.contains("is not refunded")));
}
