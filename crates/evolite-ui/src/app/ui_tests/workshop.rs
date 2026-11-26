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
        painted_has(kit, "issued 14 of 100")
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

const DEMO_CONTRACT: &str = "FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P";
const DEMO_MODEL: &str = "7Yowk46VwwHqmD5yZyyygggh937aP6h2UW7aQWBdWpM5";

/// UC-6, AC-D12: a tile of W4 opens the model's page — numbers, not a row per copy: «You
/// have», «For sale» with its prices, «Sold», the supply and how many were issued.
#[test]
fn a_model_page_shows_its_numbers() {
    let (_turn, mut kit) = demo_app("workshop-collection", "en");
    wait_for(&mut kit, "W4", |kit| painted_has(kit, "Winter tram"));
    button(&kit, "Winter tram").click();
    wait_for(&mut kit, "W7", |kit| {
        has_text(kit, "You have") && has_text(kit, "12")
    });
    // W7-4 (the customer's mockup of 2026-11-22): numbers, not «issued» twice.
    // BUG-312: 14 issued — 12 held, one given, one sold.
    assert!(has_words(&kit, "supply 100 · numbers no. 1–14"));
    assert!(has_words(&kit, "issued 14 of 100"));
    // W7-3: the collection's rules; it is closed.
    for chip in ["transferable", "for sale", "only the author issues"] {
        assert!(painted_has(&kit, chip), "{chip}");
    }
    assert!(has_text(&kit, "For sale") && has_text(&kit, "Sold") && has_text(&kit, "Given"));
    assert!(
        has_words(&kit, "not for sale 8 · for sale 4"),
        "your copies' parts"
    );
    // «Given» from DocumentHistory (the architect, 2026-11-22): the demo's one gift.
    wait_for(&mut kit, "the gifts", |kit| has_words(kit, ", @alice"));
    assert!(has_words(&kit, "last: "));
    // BUG-314: the counts are read again once the sales answer.
    wait_for(&mut kit, "the price group", |kit| has_words(kit, "— 4"));
    // BUG-276: the copy sold shows its price though History here has no sale (another copy, a
    // cleared History): the sum is the core's, of the same sales as the count.
    show_wallet_edited(
        &mut kit,
        3,
        "History without the sale",
        |w| {
            // The gift's series line (BUG-307) too: «Recent» is empty only without both.
            w.activity.retain(|a| {
                !matches!(
                    a.kind,
                    evolite_api::ActivityKind::NftSold { .. }
                        | evolite_api::ActivityKind::NftSeries { .. }
                )
            })
        },
        |kit| has_text(kit, "No events yet"),
    );
    wait_for(&mut kit, "the sold copy's price", |kit| {
        has_words(kit, "for 1.0000 DASH · credits to")
    });
    assert!(!button(&kit, "Give…").accesskit_node().is_disabled());
    assert!(!button(&kit, "Issue more").accesskit_node().is_disabled());
    // W7-5: ranges, no row a copy; «Prices» by price with its numbers.
    wait_for(&mut kit, "your numbers", |kit| has_words(kit, "no. 1–12"));
    // Those for sale are an ask of their own: it may come after.
    wait_for(&mut kit, "the prices' numbers", |kit| {
        has_words(kit, "4 copies · no. 3, 6, 9, 12")
    });
    // BUG-312: the demo model has its picture.
    assert!(!has_words(&kit, "Without a picture"));
    assert!(has_words(&kit, "evolite picks the copy numbers itself"));
    // W7-7: the contract's id at the advanced level only.
    assert!(!has_text(&kit, "Contract id"));
    // W7-2: back to the collection by its name.
    assert!(has_words(&kit, "Seasons of the city"));
}

/// W7-6: the model's sale in «Recent», with «for … · credits to …» on «Sold»; «All history ›»
/// opens History searching for the model; W7-7 at the advanced level.
#[test]
fn the_model_page_shows_its_recent_and_leads_to_history() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    kit.state_mut().settings.set("advanced", true);
    // The whole page in view: «Recent» is below the fold of a usual window.
    kit.set_size(egui::vec2(1280.0, 1600.0));
    wait_for(&mut kit, "W7", |kit| has_text(kit, "You have"));
    let sale = evolite_api::Activity {
        id: 0x5a1e,
        kind: evolite_api::ActivityKind::NftSold {
            model: Some("Winter tram".into()),
            serial: Some(13),
            buyer: "@alice".into(),
        },
        chain: evolite_api::Chain::Platform,
        at: chrono::Utc::now().timestamp() - 600,
        amount: 100_000_000,
        proof: evolite_api::Proof::Proved,
        txid: None,
        fee: None,
        block: None,
        address: None,
        layers: evolite_api::layer::IDENTITY,
        flow: evolite_api::Flow::In,
        key: String::new(),
        detail: None,
        note: None,
        rate: None,
        tax_transfer: false,
    };
    show_wallet_edited(
        &mut kit,
        3,
        "the sale",
        |w| {
            if !w.activity.iter().any(|a| a.id == 0x5a1e) {
                w.activity.insert(0, sale.clone());
            }
        },
        // The sale in «Recent» and the counts read: on a loaded machine the model's counts come
        // after «Recent» (batch 69's fast run), and «Sold» says «—» until they do.
        |kit| !has_text(kit, "No events yet") && has_words(kit, "credits to"),
    );
    assert!(has_words(&kit, "credits to"), "{:?}", painted_texts(&kit));
    assert!(has_text(&kit, "Contract id"), "the advanced level");
    click_words_of(&mut kit, "All history ›");
    wait_for(&mut kit, "History", |kit| {
        kit.state().history.query == "Winter tram"
    });
}

/// W7 at the medium and the narrow window: two columns from 900 of content, one below; nothing
/// wider than the window.
#[test]
fn the_model_page_fits_at_1024_and_800() {
    for (w, h) in [(1024.0, 768.0), (800.0, 600.0)] {
        let (_turn, mut kit) = demo_app("workshop-model-page", "de");
        kit.set_size(egui::vec2(w, h));
        wait_for(&mut kit, "W7", |kit| painted_has(kit, "Winter tram"));
        kit.run_steps(6);
        let window = kit.ctx.content_rect();
        let ppp = kit.ctx.pixels_per_point();
        let wide: Vec<String> = kit
            .query_all_by(|n| n.bounding_box().is_some())
            .filter_map(|n| {
                let b = n.accesskit_node().bounding_box()?;
                let right = b.x1 as f32 / ppp;
                (right > window.right() + 1.0).then(|| format!("{:?}", n.accesskit_node().label()))
            })
            .take(3)
            .collect();
        assert!(wide.is_empty(), "@{w}: {wide:?}");
    }
}

/// UC-8, W8: «Give…» — how many of the free copies, to whom; the core's plan; the summary
/// pinned over the password with the records and their fee; «Sign and give 2» waits for it.
#[test]
fn giving_copies_is_one_series_with_one_check() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    wait_for(&mut kit, "W7", |kit| has_text(kit, "You have"));
    wait_for(&mut kit, "the numbers", |kit| {
        !button(kit, "Give…").accesskit_node().is_disabled()
    });
    button(&kit, "Give…").click();
    wait_for(&mut kit, "W8", |kit| has_text(kit, "Give “Winter tram”"));
    // Free: 12 held less 4 for sale.
    wait_for(&mut kit, "how many are free", |kit| has_words(kit, "of 8"));
    kit.get_by_role_and_label(Role::TextInput, "How many")
        .click();
    kit.run_steps(2);
    kit.key_press(egui::Key::Backspace);
    kit.get_by_role_and_label(Role::TextInput, "How many")
        .type_text("2");
    type_into(&mut kit, "Recipient", "@alice");
    wait_for(&mut kit, "the plan", |kit| has_words(kit, "2 records"));
    assert!(
        has_words(&kit, "you cannot take a gift back")
            || has_words(&kit, "You cannot take a gift back")
    );
    assert!(
        button(&kit, "Sign and give 2")
            .accesskit_node()
            .is_disabled(),
        "not without the password"
    );
}

/// W12, W12b: a series of this model on its page — its progress; stopped with copies left,
/// «Continue» opens its window with what is left, and «Close» takes the plate away.
#[test]
fn a_stopped_series_can_be_continued_from_the_model_page() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    // What the page asked for when it opened (the sales, BUG-314) has come: a snapshot after the
    // edit would take the plate away before «Continue» is pressed.
    settle(&mut kit);
    show_wallet_edited(
        &mut kit,
        4,
        "the plate",
        |w| {
            w.series.push(evolite_api::series::SeriesState {
                id: 7,
                target: evolite_api::series::SeriesTarget {
                    identity: w.identity.as_ref().unwrap().id.clone(),
                    contract: DEMO_CONTRACT.into(),
                    model: DEMO_MODEL.into(),
                },
                kind: evolite_api::series::SeriesKind::Mint { count: 10 },
                total: 10,
                done: 3,
                refused: 0,
                unclear: 0,
                running: false,
                stopping: false,
                per_write: 87_000,
            })
        },
        |kit| has_text(kit, "Issued 3 of 10"),
    );
    // The plate's own «Continue» (the page has the only series plate; the first is it).
    kit.query_all_by_role_and_label(Role::Button, "Continue")
        .next()
        .expect("«Continue»")
        .click();
    wait_for(&mut kit, "its window", |kit| {
        has_text(kit, "Continue “Winter tram”")
    });
    assert!(has_words(&kit, "Left to write: 7"));
    assert!(has_words(&kit, "7 records"));
}

/// Batch 65: the renderer's time ran out before a copy fitted — W5 says it took too long (not
/// that the picture cannot fit) and «Make a smaller copy» stays for another try.
#[test]
fn a_copy_that_took_too_long_can_be_tried_again() {
    let (_turn, mut kit) = demo_app("workshop-collection", "en");
    new_model_with(&mut kit, "pin.png", noisy_png(120));
    wait_for(&mut kit, "the offer", |kit| {
        has_words(kit, "evolite can make a smaller copy that fits")
    });
    // The renderer's answer to a copy asked for.
    let model = &mut kit.state_mut().workshop.model;
    model.fitting = true;
    model.on_fitted(&Err(evolite_api::CoreError::Failed(
        evolite_api::series::OUT_OF_TIME.into(),
    )));
    wait_for(&mut kit, "the words", |kit| {
        has_words(kit, "Making a smaller copy took too long this time.")
    });
    assert!(!has_words(&kit, "does not fit into the record"));
    button(&kit, "Make a smaller copy").click();
    wait_for(&mut kit, "the copy on another try", |kit| {
        has_words(kit, "Reduced: 512×341")
    });
}

/// BUG-266: the core's first check says «wait for the sync to finish» (the sync ends after the
/// form opened); the form asks again by itself — the price comes and the red words go, with no
/// change to the form.
#[test]
fn a_check_said_not_synced_is_asked_again() {
    let (_turn, mut kit) = demo_app("workshop", "en");
    wait_for(&mut kit, "W1", |kit| has_text(kit, "Seasons of the city"));
    button(&kit, "Create NFT").click();
    wait_for(&mut kit, "W2", |kit| has_text(kit, "Collection rules"));
    // The demo answers this name «not synced» once.
    type_into(&mut kit, "Collection name", "Sync pending");
    wait_for(&mut kit, "the «not synced» answer", |kit| {
        has_words(kit, "wait for the sync to finish")
    });
    assert!(button(&kit, "Check").accesskit_node().is_disabled());
    wait_for(&mut kit, "the check asked again", |kit| {
        !button(kit, "Check").accesskit_node().is_disabled()
    });
    assert!(
        has_words(&kit, "≈ 0.19"),
        "the price: {:?}",
        painted_texts(&kit)
    );
    assert!(
        !has_words(&kit, "wait for the sync to finish"),
        "the stale words are gone"
    );
}

/// BUG-272 (TEAM.md §8d, В3): while the core works the collection's price out, the summary says
/// «Calculating…», not «—»; then the price.
#[test]
fn the_price_says_it_is_being_calculated() {
    let (_turn, mut kit) = demo_app("workshop", "en");
    wait_for(&mut kit, "W1", |kit| has_text(kit, "Seasons of the city"));
    button(&kit, "Create NFT").click();
    wait_for(&mut kit, "W2", |kit| has_text(kit, "Collection rules"));
    type_into(&mut kit, "Collection name", "Dash Pins");
    // The form rests 0.6 s before it asks: the price is coming, and says so.
    assert!(has_words(&kit, "Calculating…"), "{:?}", painted_texts(&kit));
    wait_for(&mut kit, "the price", |kit| has_words(kit, "≈ 0.19"));
    assert!(!has_words(&kit, "Calculating…"));
}

/// BUG-271 r1 (TEAM.md §8d-2): while the search for one's own collections on the network is not
/// done, «My collections» says so; a failed search says so with «Retry», which reads the list
/// again (the core starts the search with it).
#[test]
fn the_search_for_ones_collections_is_said() {
    use evolite_api::workshop::OwnSearch;
    let (_turn, mut kit) = demo_app("workshop", "en");
    wait_for(&mut kit, "W1", |kit| has_text(kit, "Seasons of the city"));
    show_wallet_edited(
        &mut kit,
        3,
        "searching",
        |w| w.own_collections = OwnSearch::Searching,
        |kit| has_words(kit, "Looking for your collections…"),
    );
    show_wallet_edited(
        &mut kit,
        3,
        "failed",
        |w| w.own_collections = OwnSearch::Failed,
        |kit| has_words(kit, "Could not find your collections"),
    );
    click_words_of(&mut kit, "Retry");
    wait_for(&mut kit, "the list read again", |kit| {
        has_text(kit, "Seasons of the city")
    });
}

/// W7 r1 Must (TEAM.md §8d-4): a read of the model's page that failed is said — «Could not read»
/// with «Retry» — not shown as «—» as while reading, and is not asked again every frame: ten
/// frames later it was asked once; «Retry» asks it again at once.
#[test]
fn a_failed_read_of_the_model_page_says_so_and_waits() {
    use evolite_api::workshop::WorkshopAsk;
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    kit.set_size(egui::vec2(1280.0, 1600.0));
    wait_for(&mut kit, "W7", |kit| has_text(kit, "You have"));
    // The demo's collection and model.
    let key = (
        "FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P".to_owned(),
        "7Yowk46VwwHqmD5yZyyygggh937aP6h2UW7aQWBdWpM5".to_owned(),
    );
    let asked = |kit: &Kit| kit.state().workshop.given_asked.get(&key).map(|r| r.asked);
    // Answered, not only asked: the demo's answer coming after the failure below would put the
    // count back over «Could not read» (batch 69's fast run, a loaded machine).
    wait_for(&mut kit, "the gifts answered", |kit| {
        kit.state()
            .workshop
            .given_asked
            .get(&key)
            .is_some_and(|r| r.asked > 0 && !r.asking)
    });
    let ask = WorkshopAsk::Given {
        contract: key.0.clone(),
        model: key.1.clone(),
    };
    kit.state_mut().workshop.on_read(
        &ask,
        &Err(evolite_api::CoreError::Failed("no answer".into())),
    );
    kit.run_steps(10);
    assert!(has_words(&kit, "Could not read"), "said");
    assert_eq!(asked(&kit), Some(1), "asked once in ten frames");
    click_words_of(&mut kit, "Retry");
    kit.run_steps(2);
    assert_eq!(asked(&kit), Some(2), "«Retry» asks at once");
}

/// BUG-312: the example model as the demo shows it — its picture drawn, not «Without a
/// picture»; the sale of no. 14 in «Recent» from History (the gift has no History line yet:
/// FEAT-0038 R-15).
#[test]
fn the_demo_model_has_its_picture_and_its_sale_in_recent() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    kit.set_size(egui::vec2(1280.0, 1600.0));
    wait_for(&mut kit, "W7", |kit| has_text(kit, "You have"));
    // Pictures are off in tests (`ArtOptions::off`): the model has one, it is not drawn here.
    wait_for(&mut kit, "«Recent»", |kit| has_words(kit, "no. 14 · "));
    assert!(!has_words(&kit, "Without a picture"));
    assert!(!has_text(&kit, "No events yet"));
    // Your numbers are an ask of their own: 13 and 14 are not held.
    wait_for(&mut kit, "your numbers", |kit| has_words(kit, "no. 1–12"));
}

/// BUG-314: the collection's page reads the counts without the sales — «Sold» not known
/// (`None`, «—»), never the demo's 1 before anyone asked; the model's page asks the sales, then
/// the counts again: «Sold 1 · for 1.0000 DASH», and the sale's History line in «Recent», written
/// by that read.
#[test]
fn sold_comes_from_the_sales_the_page_asks_for() {
    let (_turn, mut kit) = demo_app("workshop-collection", "en");
    wait_for(&mut kit, "the counts", |kit| {
        painted_has(kit, "issued 14 of 100")
    });
    assert_eq!(
        kit.state().workshop.sold_read(DEMO_CONTRACT, DEMO_MODEL),
        Some(None),
        "no sales asked: «Sold» unknown"
    );
    assert!(
        !kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .is_some_and(|w| w
                .activity
                .iter()
                .any(|a| matches!(a.kind, evolite_api::ActivityKind::NftSold { .. }))),
        "no sale in History before the sales are read"
    );
    button(&kit, "Winter tram").click();
    wait_for(&mut kit, "«Sold» from the sales", |kit| {
        has_words(kit, "for 1.0000 DASH · credits to")
    });
    assert_eq!(
        kit.state().workshop.sold_read(DEMO_CONTRACT, DEMO_MODEL),
        Some(Some(1))
    );
    wait_for(&mut kit, "the sale in «Recent»", |kit| {
        has_words(kit, "no. 14 · ")
    });
}

/// BUG-314 r1 (the coordinator, 2026-11-25): a visit to the model page reads the sales again,
/// at most once a minute — back within the minute, or a minute spent on the page, asks
/// nothing; back after it, they are read again, and a sale that came since has the model
/// counted again («Sold» from the core's sales: two after a second; the core's
/// `a_sale_after_the_first_read_makes_two`).
#[test]
fn a_visit_after_a_minute_reads_the_sales_again() {
    use evolite_api::workshop::{Sale, WorkshopAsk, WorkshopRead};
    let (_turn, mut kit) = demo_app("workshop-collection", "en");
    let open_model = |kit: &mut Kit| {
        wait_for(kit, "W4", |kit| painted_has(kit, "Winter tram"));
        button(kit, "Winter tram").click();
        wait_for(kit, "W7", |kit| has_text(kit, "You have"));
    };
    let back = |kit: &mut Kit| {
        kit.query_all_by(|n| {
            // «‹ Seasons of the city», a link named «Back: …».
            n.role() == Role::Link && n.label().is_some_and(|l| l.starts_with("Back: "))
        })
        .next()
        .expect("back to the collection")
        .click();
        wait_for(kit, "W4", |kit| !has_text(kit, "You have"));
    };
    let read_at = |kit: &Kit| kit.state().workshop.sales_read_at();
    let time = |kit: &Kit| kit.ctx.input(|i| i.time);
    open_model(&mut kit);
    wait_for(&mut kit, "the sales read", |kit| read_at(kit).is_some());
    settle(&mut kit);
    let first = read_at(&kit);
    back(&mut kit);
    open_model(&mut kit);
    settle(&mut kit);
    assert_eq!(
        read_at(&kit),
        first,
        "back within the minute: not read again"
    );
    // A minute on the page: not a visit.
    let since = time(&kit);
    while time(&kit) - since < 61.0 {
        kit.step();
    }
    settle(&mut kit);
    assert_eq!(read_at(&kit), first, "staying on the page reads nothing");
    back(&mut kit);
    open_model(&mut kit);
    wait_for(&mut kit, "the sales read again", |kit| read_at(kit) > first);
    settle(&mut kit);
    assert_eq!(
        kit.state().workshop.sold_read(DEMO_CONTRACT, DEMO_MODEL),
        Some(Some(1)),
        "nothing new since: the counts kept"
    );
    // That read brought a sale of this model: its counts are read again.
    kit.state_mut().workshop.on_read(
        &WorkshopAsk::Sales,
        &Ok(WorkshopRead::Sales(vec![Sale {
            purchase: "demo-purchase-2".into(),
            contract: DEMO_CONTRACT.into(),
            instance: "i".into(),
            model_id: Some(DEMO_MODEL.into()),
            model: Some("Winter tram".into()),
            serial: Some(12),
            buyer: "b".into(),
            buyer_name: None,
            price: 100_000_000,
            at: 1,
        }])),
    );
    assert_eq!(
        kit.state().workshop.sold_read(DEMO_CONTRACT, DEMO_MODEL),
        None,
        "the model is counted again"
    );
    wait_for(&mut kit, "the counts read again", |kit| {
        kit.state().workshop.sold_read(DEMO_CONTRACT, DEMO_MODEL) == Some(Some(1))
    });
}

/// BUG-307 (R-15, AC-A15): a series is one History line — the demo's gift of no. 13 reads
/// «Given 1 “Winter tram” · @alice» with «Platform · series · done 1 of 1» under it, its cost
/// what it took; and the model's «Recent» has it.
#[test]
fn a_gift_series_is_one_line_in_history_and_recent() {
    let (_turn, mut kit) = demo_app("activity", "en");
    wait_for(&mut kit, "the gift's line", |kit| {
        has_words(kit, "Given 1 “Winter tram”")
    });
    assert!(
        has_words(&kit, "series · done 1 of 1"),
        "{:?}",
        painted_texts(&kit)
    );
    let lines = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| {
            w.activity
                .iter()
                .filter(|a| matches!(a.kind, evolite_api::ActivityKind::NftSeries { .. }))
                .count()
        });
    assert_eq!(lines, Some(1), "one line for the series");
    // The first app's turn let go first: a second `demo_app` would wait on it forever.
    drop(kit);
    drop(_turn);
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    wait_for(&mut kit, "«Recent»", |kit| {
        has_words(kit, "Given 1 “Winter tram”")
    });
}

/// Types `text` into the field named `label` in place of what it holds.
fn retype(kit: &mut Kit, label: &str, text: &str) {
    kit.get_by_role_and_label(Role::TextInput, label).click();
    kit.run_steps(2);
    for _ in 0..12 {
        kit.key_press(egui::Key::Backspace);
    }
    kit.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    kit.run_steps(2);
}

/// The password typed and the window's main button pressed.
fn sign(kit: &mut Kit, label: &str) {
    kit.get_by_role(Role::PasswordInput).click();
    kit.run_steps(2);
    kit.get_by_role(Role::PasswordInput).type_text("demo");
    kit.run_steps(2);
    wait_for(kit, "the button", |kit| {
        !button(kit, label).accesskit_node().is_disabled()
    });
    button(kit, label).click();
}

/// The model page with its numbers and its «Prices» read.
fn model_page_read(kit: &mut Kit) {
    wait_for(kit, "W7", |kit| has_text(kit, "You have"));
    wait_for(kit, "the prices", |kit| {
        has_words(kit, "4 copies · no. 3, 6, 9, 12")
            && !button(kit, "Put up for sale…")
                .accesskit_node()
                .is_disabled()
    });
}

/// W9 S1: «Put up for sale…» is the first and the main action, «Give…» and «Issue more» after
/// it; no «Take off sale…» of its own — «Prices» has «Change the price» and «Take off 4».
#[test]
fn the_sale_comes_first_and_prices_have_their_actions() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    model_page_read(&mut kit);
    let at = |label: &str| button(&kit, label).rect().left();
    assert!(at("Put up for sale…") < at("Give…"));
    assert!(at("Give…") < at("Issue more"));
    assert!(kit.query_all_by_label("Take off sale…").next().is_none());
    assert!(
        !button(&kit, "Change the price")
            .accesskit_node()
            .is_disabled()
    );
    assert!(!button(&kit, "Take off 4").accesskit_node().is_disabled());
}

/// UC-7, W9 S2–S3: three free copies put up at 0.5 DASH — the plan's free numbers from the
/// lowest, the fee per record and in all, the credits now and after; one password; the plate
/// «Put up for sale 3 of 3» with what a buyer sees and nothing to sign again; «Prices» read
/// again has the new row beside the old (read back from the core, not kept by the window).
#[test]
fn copies_put_up_for_sale_show_in_prices() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    model_page_read(&mut kit);
    button(&kit, "Put up for sale…").click();
    wait_for(&mut kit, "S2", |kit| has_text(kit, "Put up for sale"));
    assert!(has_words(&kit, "“Winter tram” · free 8"));
    assert!(
        button(&kit, "Sign and put up 0")
            .accesskit_node()
            .is_disabled(),
        "no price: nothing asked"
    );
    retype(&mut kit, "How many", "3");
    retype(&mut kit, "Price per copy", "0.5");
    wait_for(&mut kit, "the plan", |kit| has_words(kit, "3 records"));
    assert!(
        has_words(&kit, "3 at 0.5000 DASH"),
        "{:?}",
        painted_texts(&kit)
    );
    assert!(has_words(&kit, "1.5000 DASH in credits"));
    assert!(has_words(&kit, "× ≈ 0.0000"), "one record's fee");
    assert!(has_words(&kit, "now, ≈"), "the credits now and after");
    assert!(
        has_words(&kit, "here no. 1–2, 4"),
        "the free ones from the lowest"
    );
    assert!(has_words(&kit, "Anyone can buy them"));
    sign(&mut kit, "Sign and put up 3");
    wait_for(&mut kit, "S3", |kit| {
        has_text(kit, "Put up for sale 3 of 3")
    });
    assert!(has_words(
        &kit,
        "They can be bought at 0.5000 DASH per copy."
    ));
    assert!(
        kit.query_all_by_label("Continue").next().is_none(),
        "nothing to sign again"
    );
    wait_for(&mut kit, "«Prices» read again", |kit| {
        has_words(kit, "3 copies · no. 1–2, 4") && has_words(kit, "4 copies · no. 3, 6, 9, 12")
    });
    assert!(has_words(&kit, "not for sale 5 · for sale 7"));
    kit.query_all_by_role_and_label(Role::Button, "Close")
        .next()
        .expect("«Close»")
        .click();
    wait_for(&mut kit, "the plate gone", |kit| {
        !has_text(kit, "Put up for sale 3 of 3")
    });
}

/// UC-7, AC-A11, W9 S4: «Change the price» of the 1 DASH row — the same price holds the button
/// with «The same price»; a new one shows now, after and all bought; signed, the same four
/// copies have the new price.
#[test]
fn a_price_is_changed_on_the_same_copies() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    model_page_read(&mut kit);
    button(&kit, "Change the price").click();
    wait_for(&mut kit, "S4", |kit| {
        has_words(kit, "4 copies at 1.0000 DASH · no. 3, 6, 9, 12")
    });
    retype(&mut kit, "New price per copy", "1");
    kit.run_steps(4);
    assert!(has_text(&kit, "The same price"));
    assert!(
        button(&kit, "Sign and change 0")
            .accesskit_node()
            .is_disabled()
    );
    retype(&mut kit, "New price per copy", "0.7");
    wait_for(&mut kit, "the plan", |kit| has_words(kit, "4 records"));
    assert!(!has_text(&kit, "The same price"));
    assert!(has_words(&kit, "1.0000 DASH per copy"), "now");
    assert!(has_words(&kit, "0.7000 DASH per copy"), "will be");
    assert!(has_words(&kit, "2.8000 DASH in credits"));
    assert!(has_words(&kit, "The price changes on the same 4 copies"));
    sign(&mut kit, "Sign and change 4");
    wait_for(&mut kit, "the plate", |kit| {
        has_text(kit, "Price changed: 4 of 4")
    });
    assert!(has_words(&kit, "bought at 0.7000 DASH per copy"));
    wait_for(&mut kit, "«Prices» read again", |kit| {
        has_words(kit, "4 copies · no. 3, 6, 9, 12") && has_text(kit, "0.7000 DASH")
    });
    assert!(
        !has_text(&kit, "1.0000 DASH"),
        "no copy is left at the old price"
    );
}

/// UC-9, AC-A7, W9 S5: «Take off 4» opens on its price with all four; two taken off go from
/// the highest numbers, and «Prices» keeps the other two.
#[test]
fn copies_are_taken_off_from_the_highest_numbers() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    model_page_read(&mut kit);
    button(&kit, "Take off 4").click();
    wait_for(&mut kit, "S5", |kit| {
        has_words(kit, "“Winter tram” · for sale 4")
    });
    assert!(has_text(&kit, "at 1.0000 DASH · 4") && has_text(&kit, "all 4"));
    wait_for(&mut kit, "the plan", |kit| has_words(kit, "4 records"));
    retype(&mut kit, "How many", "2");
    wait_for(&mut kit, "the plan of two", |kit| {
        has_words(kit, "2 records")
    });
    assert!(has_words(&kit, "from the highest numbers"));
    sign(&mut kit, "Sign and take off 2");
    wait_for(&mut kit, "the plate", |kit| {
        has_text(kit, "Taken off sale 2 of 2")
    });
    wait_for(&mut kit, "«Prices» read again", |kit| {
        has_words(kit, "2 copies · no. 3, 6")
    });
}

/// AC-A8: while a series of this identity runs, the sale and the actions of «Prices» wait.
#[test]
fn a_running_series_holds_the_price_actions() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    model_page_read(&mut kit);
    show_wallet_edited(
        &mut kit,
        4,
        "a running series",
        |w| {
            w.series.push(evolite_api::series::SeriesState {
                id: 9,
                target: evolite_api::series::SeriesTarget {
                    identity: w.identity.as_ref().unwrap().id.clone(),
                    contract: DEMO_CONTRACT.into(),
                    model: DEMO_MODEL.into(),
                },
                kind: evolite_api::series::SeriesKind::Mint { count: 10 },
                total: 10,
                done: 3,
                refused: 0,
                unclear: 0,
                running: true,
                stopping: false,
                per_write: 87_000,
            })
        },
        |kit| has_text(kit, "Issued 3 of 10"),
    );
    for label in ["Put up for sale…", "Change the price", "Take off 4"] {
        assert!(
            button(&kit, label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
}

/// W9 S5: a window opened before the copies for sale are read keeps the price it was opened for
/// and its count — not «all» with 0 (the S5 screenshot of 2026-11-25: «Sign and take off 0», no
/// fee, no credits after).
#[test]
fn a_take_off_opened_before_the_copies_are_read_keeps_its_price() {
    let (_turn, mut kit) = demo_app("workshop-unprice", "en");
    wait_for(&mut kit, "the plan", |kit| has_words(kit, "2 records"));
    settle(&mut kit);
    assert!(
        kit.query_by_role_and_label(Role::Button, "Sign and take off 2")
            .is_some(),
        "{:?}",
        painted_texts(&kit)
    );
}

/// W9 S5 (the designer, 2026-11-25): taking copies off sale is a transfer to oneself, never a
/// gift — History and «Recent» say «Taken off sale 4 “Winter tram”», the gift's line stays the
/// one gift, «Given» does not grow, and the copies keep their numbers; the window says why it
/// costs as a gift.
#[test]
fn a_take_off_is_not_a_gift() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    model_page_read(&mut kit);
    let gifts = |kit: &Kit| {
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .map_or(0, |w| {
                w.activity
                    .iter()
                    .filter(|a| {
                        matches!(
                            &a.kind,
                            evolite_api::ActivityKind::NftSeries {
                                act: evolite_api::series::SeriesAct::Gift { .. },
                                ..
                            }
                        )
                    })
                    .count()
            })
    };
    assert_eq!(gifts(&kit), 1);
    button(&kit, "Take off 4").click();
    wait_for(&mut kit, "the plan", |kit| has_words(kit, "4 records"));
    assert!(has_words(
        &kit,
        "A price can only be removed by passing the item to yourself"
    ));
    sign(&mut kit, "Sign and take off 4");
    wait_for(&mut kit, "the plate", |kit| {
        has_text(kit, "Taken off sale 4 of 4")
    });
    wait_for(&mut kit, "its line in «Recent»", |kit| {
        has_words(kit, "Taken off sale 4 “Winter tram”")
    });
    assert_eq!(gifts(&kit), 1, "no new gift");
    assert!(!has_words(&kit, "Given 4"), "{:?}", painted_texts(&kit));
    wait_for(&mut kit, "the gifts", |kit| has_words(kit, ", @alice"));
    wait_for(&mut kit, "your numbers", |kit| has_words(kit, "no. 1–12"));
}

/// W9 r1 (the architect): «Put up for sale» with its copies not read does not say «free 0»
/// — the model's name alone until they are, then «free 8».
#[test]
fn a_sale_window_opened_before_the_copies_are_read_says_no_zero() {
    let (_turn, mut kit) = demo_app("workshop-sell", "en");
    wait_for(&mut kit, "the free ones", |kit| {
        has_words(kit, "“Winter tram” · free 8")
    });
    // The frame drawn with the counts not read (asked again in it).
    kit.state_mut().workshop.forget_counts();
    kit.step();
    assert!(!has_words(&kit, "· free 0"), "«free 0» before the read");
    assert!(
        has_words(&kit, "“Winter tram”"),
        "{:?}",
        painted_texts(&kit)
    );
    wait_for(&mut kit, "read again", |kit| {
        has_words(kit, "“Winter tram” · free 8")
    });
}
