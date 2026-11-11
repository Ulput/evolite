//! FEAT-0034: rows whose items stand on one axis — the label and «Hide the amounts», the
//! filter row of History — and room kept for what may appear.

use super::*;

/// The vertical centre of the topmost label whose words are `text`.
fn label_centre(kit: &Kit, text: &str) -> f32 {
    kit.query_all(By::new().role(Role::Label).value(text))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("the label «{text}»"))
        .center()
        .y
}

/// The vertical centre of the topmost button named `name`.
fn button_centre(kit: &Kit, name: &str) -> f32 {
    kit.query_all_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(name))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("the button «{name}»"))
        .center()
        .y
}

/// Item 1, AC-D1: on Overview, Send, Identity and the pool the label on the left and «Hide the
/// amounts» on its right stand on one axis (1 px), at 1024×768.
#[test]
fn the_label_and_hide_the_amounts_share_one_axis() {
    for (preset, label) in [
        ("overview", "Total"),
        ("send", "From"),
        ("identity", "Platform credits"),
        ("shielded", "In the pool"),
    ] {
        let (turn, mut kit) = demo_app(preset, "en");
        wait_for(&mut kit, "the toggle", |kit| {
            kit.query_all_by(|n| n.label().as_deref() == Some("Hide the amounts"))
                .next()
                .is_some()
        });
        kit.run_steps(3);
        let (l, t) = (
            label_centre(&kit, label),
            button_centre(&kit, "Hide the amounts"),
        );
        assert!(
            (l - t).abs() <= 1.0,
            "{preset}: «{label}» at {l}, the toggle at {t}"
        );
        drop(kit);
        drop(turn);
    }
}

/// Item 1 (the coordinator): the toggle's 28 px of clicking stand only around itself — a click
/// in the gap beside it or just under it changes nothing — and its hover background does not
/// reach the label's words, on Overview at 800×600.
#[test]
fn the_toggle_is_clicked_only_where_it_is() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(egui::vec2(800.0, 600.0));
    wait_for(&mut kit, "the toggle", |kit| {
        kit.query_all_by(|n| n.label().as_deref() == Some("Hide the amounts"))
            .next()
            .is_some()
    });
    kit.run_steps(3);
    let toggle = kit
        .query_all_by(|n| {
            n.role() == Role::Button && n.label().as_deref() == Some("Hide the amounts")
        })
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("the toggle");
    let total = kit
        .query_all(By::new().role(Role::Label).value("Total"))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("«Total»");
    assert!(
        !toggle.intersects(total),
        "the hover covers «Total»: {toggle:?} over {total:?}"
    );
    let click = |kit: &mut Kit, at: egui::Pos2| {
        kit.hover_at(at);
        kit.step();
        for pressed in [true, false] {
            kit.input_mut().events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            kit.step();
        }
        kit.run_steps(2);
    };
    let hidden = || crate::format::display().hide;
    let before = hidden();
    // Just past egui's own reach around a widget (`interact_radius`, 5 px): a click that
    // close counts as the widget's by egui's design, for every widget, as before this change.
    let reach = kit.ctx.global_style().interaction.interact_radius + 2.0;
    for at in [
        toggle.left_center() - egui::vec2(reach, 0.0),
        toggle.center_bottom() + egui::vec2(0.0, reach),
    ] {
        click(&mut kit, at);
        assert_eq!(
            hidden(),
            before,
            "a click at {at:?} beside the toggle {toggle:?}"
        );
    }
    // And on it, it does work: once to change, once to put it back.
    click(&mut kit, toggle.center());
    assert_ne!(hidden(), before, "a click on the toggle");
    let on = kit
        .query_all_by(|n| {
            n.role() == Role::Button && n.label().as_deref() == Some("Show the amounts")
        })
        .map(|n| n.rect())
        .next()
        .expect("the toggle, now «Show the amounts»");
    click(&mut kit, on.center());
    assert_eq!(hidden(), before);
}

/// Item 8, AC-D5: «Wrong password» takes a line kept for it from the start, so the button under
/// it does not move when it comes.
#[test]
fn a_wrong_password_does_not_move_the_button() {
    let (_turn, mut kit) = demo_app("wallet-unlock", "en");
    kit.state_mut().unlock.password.clear();
    kit.run_steps(3);
    let button = |kit: &Kit| {
        kit.query_all_by(|n| {
            n.role() == Role::Button
                && n.label()
                    .is_some_and(|l| l == "Open" || l == "Open the wallet")
        })
        .map(|n| n.rect())
        .next()
        .expect("the button that opens")
    };
    let before = button(&kit);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .type_text(evolite_core::DEMO_WRONG_PASSWORD);
    kit.run_steps(2);
    enter(&mut kit);
    wait_for(&mut kit, "«Wrong password»", |kit| {
        has_text(kit, "Wrong password")
    });
    kit.run_steps(2);
    assert_eq!(button(&kit).top(), before.top(), "the button moved");
}

/// Item 7: in Settings → Wallets the open wallet's «identity @name» is one unit: it moves to the
/// next line whole, never leaving «@name» alone there.
#[test]
fn identity_and_its_name_stay_together() {
    let (_turn, mut kit) = demo_app("settings", "en");
    wait_for(&mut kit, "the wallets", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.wallets.iter().any(|w| w.open) && s.wallet.is_some())
    });
    // The line is painted, not a widget: what it says is read from the function that makes it.
    let snap = kit.state().snapshot.clone().expect("a snapshot");
    let open = snap
        .wallets
        .iter()
        .find(|w| w.open)
        .expect("the open wallet");
    let line = crate::screens::wallets::settings_line(open, &snap, chrono::Local::now());
    assert!(line.contains("identity\u{a0}@"), "{line:?}");
    assert!(!line.contains("identity @"), "{line:?}");
}

/// Item 10: at 800×600 a settings row whose control is wide puts the control under its words,
/// as rows of buttons already did: no label is squeezed into a column a few words wide (in ru the
/// number format's line broke before its colon).
#[test]
fn a_narrow_window_does_not_squeeze_a_settings_label() {
    for lang in ["ru", "de"] {
        let (turn, mut kit) = demo_app("appearance", lang);
        kit.set_size(egui::vec2(800.0, 600.0));
        kit.run_steps(6);
        let squeezed: Vec<egui::Rect> = kit
            .query_all(By::new().role(Role::Label))
            .map(|n| n.rect())
            .filter(|r| r.width() < 160.0 && r.height() > 60.0)
            .collect();
        assert!(
            squeezed.is_empty(),
            "{lang}: labels squeezed into {squeezed:?}"
        );
        drop(kit);
        drop(turn);
    }
}

/// r1 Must 1: in the wallet menu the eye stands on the middle of the «WALLETS ON THIS COMPUTER»
/// header, a 28 px band right above the first wallet's row (2 px apart). The header's words are
/// painted, so the band is found from that row.
#[test]
fn the_wallet_menus_eye_is_on_its_headers_middle() {
    let (_turn, mut kit) = demo_app("wallet-menu", "en");
    wait_for(&mut kit, "the eye", |kit| {
        kit.query_all_by(|n| n.label().as_deref() == Some("Hide the amounts"))
            .next()
            .is_some()
    });
    kit.run_steps(3);
    // The menu's compact eye (its hover 24 wide); Overview's own eye is behind the menu.
    let eye = kit
        .query_all_by(|n| {
            n.role() == Role::Button && n.label().as_deref() == Some("Hide the amounts")
        })
        .map(|n| n.rect())
        .find(|r| (r.width() - 24.0).abs() < 1.0)
        .expect("the menu's eye")
        .center()
        .y;
    let first_row = kit
        .query_all_by(|n| n.role() == Role::Button)
        .map(|n| n.rect())
        .filter(|r| (r.width() - 302.0).abs() < 1.0 && (r.height() - 47.0).abs() < 1.0)
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("the first wallet's row");
    // The 28 px header, 2 px of spacing and 2 more kept under the eye's hover, then the row.
    let header = first_row.top() - 2.0 - 2.0 - 14.0;
    assert!(
        (eye - header).abs() <= 1.0,
        "the eye at {eye}, the header's middle at {header}"
    );
}

/// r1 P3 4: the toggle's hover does not reach the row above or under it where they are close —
/// the source cards under «From» on Send.
#[test]
fn the_toggles_hover_stays_off_its_neighbours() {
    let (_turn, mut kit) = demo_app("send", "en");
    wait_for(&mut kit, "the toggle", |kit| {
        kit.query_all_by(|n| n.label().as_deref() == Some("Hide the amounts"))
            .next()
            .is_some()
    });
    kit.run_steps(3);
    let toggle = kit
        .query_all_by(|n| {
            n.role() == Role::Button && n.label().as_deref() == Some("Hide the amounts")
        })
        .map(|n| n.rect())
        .next()
        .expect("the toggle");
    let below = kit
        .query_all_by(|n| n.role() == Role::Button)
        .map(|n| n.rect())
        .filter(|r| r.top() >= toggle.center().y && r.width() > 100.0)
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("what comes under it");
    assert!(
        toggle.bottom() <= below.top() + 0.5,
        "the hover {toggle:?} reaches {below:?}"
    );
}

/// The rect of the topmost node named `name` that is not a plain label.
fn named_rect(kit: &Kit, name: &str) -> egui::Rect {
    kit.query_all_by(|n| n.role() != Role::Label && n.label().as_deref() == Some(name))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("«{name}»"))
}

/// Item 2, AC-D2: in History the layer pills, «Filters» and the search stand on one axis, and
/// the pills are as high as «Filters» (34), at 1024×768.
#[test]
fn historys_pills_filters_and_search_share_one_axis() {
    let (_turn, mut kit) = demo_app("activity", "en");
    kit.run_steps(4);
    let all = named_rect(&kit, "All");
    let filters = named_rect(&kit, "Filters");
    let search = kit
        .query_all_by_role(Role::TextInput)
        .map(|n| n.rect())
        .next()
        .expect("the search");
    for (what, r) in [("«Filters»", filters), ("the search", search)] {
        assert!(
            (all.center().y - r.center().y).abs() <= 1.0,
            "«All» at {}, {what} at {}",
            all.center().y,
            r.center().y
        );
    }
    assert_eq!((all.height(), filters.height()), (34.0, 34.0));
}

/// Item 9, AC-D2: at 800×600 the pills do not run under «Filters» or the search.
#[test]
fn at_800_by_600_the_pills_stay_clear_of_filters_and_search() {
    for lang in ["en", "de"] {
        let (turn, mut kit) = demo_app("activity", lang);
        kit.set_size(egui::vec2(800.0, 600.0));
        kit.run_steps(6);
        let filters = named_rect(&kit, &crate::i18n::spoken(tr!("Filters")));
        let search = kit
            .query_all_by_role(Role::TextInput)
            .map(|n| n.rect())
            .next()
            .expect("the search");
        for pill in ["All", "Core", "Platform", "Private pool", "Identity"] {
            let r = named_rect(&kit, &crate::i18n::spoken(pill));
            assert!(
                !r.intersects(filters) && !r.intersects(search),
                "{lang}: «{pill}» {r:?} under «Filters» {filters:?} or the search {search:?}"
            );
        }
        drop(kit);
        drop(turn);
    }
}

/// Item 4: on Receive the first letters of «Your Core address · testnet» start at the QR card's
/// top edge (1 px), in en, ru and ar.
#[test]
fn the_receive_label_starts_at_the_qrs_top() {
    for lang in ["en", "ru", "ar"] {
        let (_turn, mut kit) = demo_app("receive", lang);
        kit.run_steps(6);
        let text = crate::i18n::t("Your Core address · testnet").into_owned();
        let label = kit
            .query_all(By::new().role(Role::Label).value(&text))
            .map(|n| n.rect())
            .next()
            .unwrap_or_else(|| panic!("{lang}: the label «{text}»"));
        let qr = kit
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(crate::screens::receive::qr_rect_id()))
            .expect("the QR card");
        let galley = kit.ctx.fonts_mut(|f| {
            f.layout_no_wrap(
                text.clone(),
                crate::theme::sans(13.0, crate::theme::Weight::Medium),
                egui::Color32::WHITE,
            )
        });
        let letters = label.top() + crate::screens::receive::ink_top(&galley);
        assert!(
            (letters - qr.top()).abs() <= 1.0,
            "{lang}: the letters start at {letters}, the QR at {}",
            qr.top()
        );
    }
}

/// Item 3: on Receive the title «Earlier addresses», its note on the right and the eye stand on
/// one axis (1 px), in en, ru and ar.
#[test]
fn earlier_addresses_title_note_and_eye_share_one_axis() {
    for lang in ["en", "ru", "ar"] {
        let (_turn, mut kit) = demo_app("receive", lang);
        kit.run_steps(6);
        let title = label_centre(&kit, &crate::i18n::t("Earlier addresses"));
        let note = label_centre(&kit, &crate::i18n::t("still receive payments"));
        let eye = button_centre(&kit, &crate::i18n::spoken("Hide the amounts"));
        for (what, y) in [("the note", note), ("the eye", eye)] {
            assert!(
                (y - title).abs() <= 1.0,
                "{lang}: the title's centre at {title}, {what}'s at {y}"
            );
        }
    }
}

/// Item 5, AC-D4: on the pool the words of each action tile stay 16 px inside its right edge —
/// a long title wraps — and the tiles of a row are of one height, in ru and de, at 1024×768
/// and 800×600.
#[test]
fn the_pools_action_tiles_keep_their_words_inside() {
    for lang in ["ru", "de"] {
        for size in [egui::vec2(1024.0, 768.0), egui::vec2(800.0, 600.0)] {
            let (_turn, mut kit) = demo_app("shielded", lang);
            kit.set_size(size);
            kit.run_steps(6);
            let mut rows: Vec<(f32, f32)> = Vec::new();
            for (title, sub) in crate::screens::shielded::actions() {
                let name = crate::i18n::spoken(title);
                // A button of the same name may stand in the header; the tile is the taller.
                let tile = kit
                    .query_all_by(|n| {
                        n.role() == Role::Button && n.label().as_deref() == Some(name.as_str())
                    })
                    .map(|n| n.rect())
                    .max_by(|a, b| a.height().total_cmp(&b.height()))
                    .unwrap_or_else(|| panic!("the tile «{name}»"));
                let words = crate::screens::shielded::ActionText::new(
                    &kit.ctx,
                    title,
                    sub,
                    tile.width(),
                    (egui::Color32::WHITE, egui::Color32::WHITE),
                );
                for (what, galley) in [("title", &words.title), ("line", &words.sub)] {
                    let right = tile.left() + words.left + galley.size().x;
                    assert!(
                        right <= tile.right() - 16.0 + 0.5,
                        "{lang} {size:?}: «{name}»'s {what} ends at {right}, the tile at {}",
                        tile.right()
                    );
                }
                assert!(
                    words.height() <= tile.height() + 0.5,
                    "{lang} {size:?}: «{name}»'s words are {} high in a tile {} high",
                    words.height(),
                    tile.height()
                );
                rows.push((tile.top(), tile.height()));
            }
            for (top, h) in &rows {
                for (top2, h2) in &rows {
                    if (top - top2).abs() < 1.0 {
                        assert!(
                            (h - h2).abs() < 0.5,
                            "{lang} {size:?}: tiles {h} and {h2} high in one row"
                        );
                    }
                }
            }
        }
    }
}

/// The smallest painted rectangle around `inside`: the card it is drawn on.
fn card_around(kit: &Kit, inside: egui::Rect) -> egui::Rect {
    fn rects(shape: &egui::Shape, found: &mut Vec<egui::Rect>) {
        match shape {
            // Only what is seen: a row lays its cards out first with an unpainted frame.
            egui::Shape::Rect(r) if r.fill.a() > 0 || r.stroke.color.a() > 0 => found.push(r.rect),
            egui::Shape::Vec(v) => v.iter().for_each(|s| rects(s, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &kit.output().shapes {
        rects(&clipped.shape, &mut found);
    }
    found
        .into_iter()
        .filter(|r| r.contains_rect(inside) && r.width() < 900.0)
        .min_by(|a, b| a.area().total_cmp(&b.area()))
        .unwrap_or_else(|| panic!("a card around {inside:?}"))
}

/// Item 6, AC-D3: the cards of one row are one height — Contacts' «How it works» — in en, ru and
/// de at 1024 wide. (Apps' row of three went with FEAT-0023 round 4: the approved L1 has none.)
#[test]
fn the_cards_of_a_row_are_one_height() {
    let rows: [(&str, [&str; 3]); 1] = [(
        "contacts",
        ["Contact request", "Payments to the contact", "Profile"],
    )];
    for lang in ["en", "ru", "de"] {
        for (preset, titles) in rows {
            let (_turn, mut kit) = demo_app(preset, lang);
            // Tall enough for the whole page to be drawn: nothing is scrolled away.
            kit.set_size(egui::vec2(1024.0, 2000.0));
            kit.run_steps(6);
            let cards: Vec<egui::Rect> = titles
                .iter()
                .map(|title| {
                    let text = crate::i18n::t(title).into_owned();
                    let label = kit
                        .query_all(By::new().role(Role::Label).value(&text))
                        .map(|n| n.rect())
                        .max_by(|a, b| a.top().total_cmp(&b.top()))
                        .unwrap_or_else(|| panic!("{lang} {preset}: «{text}»"));
                    card_around(&kit, label)
                })
                .collect();
            assert!(
                cards.iter().all(|c| (c.top() - cards[0].top()).abs() < 1.0),
                "{lang} {preset}: the cards are not in one row: {cards:?}"
            );
            for c in &cards {
                assert!(
                    (c.bottom() - cards[0].bottom()).abs() < 1.0,
                    "{lang} {preset}: the cards end at {:?}",
                    cards.iter().map(|c| c.bottom()).collect::<Vec<_>>()
                );
            }
        }
    }
}

/// Where the words `text` are painted, as one text shape.
fn painted_text(kit: &Kit, text: &str) -> Option<egui::Rect> {
    painted_text_in(kit, text, egui::Rect::EVERYTHING)
}

/// Where the words `text` are painted inside `within`, as one text shape.
fn painted_text_in(kit: &Kit, text: &str, within: egui::Rect) -> Option<egui::Rect> {
    fn texts(shape: &egui::Shape, text: &str, found: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => found.push(t.visual_bounding_rect()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, text, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &kit.output().shapes {
        texts(&clipped.shape, text, &mut found);
    }
    found.into_iter().find(|r| within.contains_rect(*r))
}

/// Item 10, the theme cards: at 800×600 each card's name stays 16 px inside its card — a long
/// one («Как в системе», «Wie im System») takes two lines — and the three cards are one height.
#[test]
fn the_theme_cards_keep_their_names_inside() {
    for lang in ["en", "ru", "de"] {
        for size in [egui::vec2(800.0, 600.0), egui::vec2(1024.0, 768.0)] {
            let (_turn, mut kit) = demo_app("appearance", lang);
            kit.set_size(size);
            kit.run_steps(6);
            let mut heights = Vec::new();
            for label in ["Dark", "Light", "Match the system"] {
                let name = crate::i18n::spoken(label);
                let card = kit
                    .query_all_by(|n| {
                        n.role() == Role::RadioButton && n.label().as_deref() == Some(name.as_str())
                    })
                    .map(|n| n.rect())
                    .next()
                    .unwrap_or_else(|| panic!("{lang}: the theme card «{name}»"));
                let words = painted_text(&kit, &crate::i18n::t(label))
                    .unwrap_or_else(|| panic!("{lang}: the painted «{name}»"));
                assert!(
                    words.right() <= card.right() - 16.0 + 0.5
                        && words.bottom() <= card.bottom() - 12.0,
                    "{lang} {size:?}: «{name}» painted at {words:?} in a card at {card:?}"
                );
                heights.push(card.height());
            }
            assert!(
                heights.iter().all(|h| (h - heights[0]).abs() < 0.5),
                "{lang} {size:?}: the theme cards are {heights:?} high"
            );
        }
    }
}

/// Item 8, designer's r1: in «Open …» the line kept for «Wrong password» is the gap between
/// the password and the catching-up note, about 20–24 px, not added to the usual gap.
#[test]
fn in_open_the_error_line_is_the_gap() {
    for lang in ["en", "ru"] {
        let (_turn, mut kit) = demo_app("wallet-switch", lang);
        kit.run_steps(6);
        let field = kit
            .query_all_by(|n| n.role() == Role::PasswordInput || n.role() == Role::TextInput)
            .map(|n| n.rect())
            .max_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("the password field");
        let text = crate::i18n::t(
            "After opening, about a minute to catch up: new blocks since it was last open.",
        )
        .into_owned();
        let note = kit
            .query_all(By::new().role(Role::Label).value(&text))
            .map(|n| n.rect())
            .next()
            .unwrap_or_else(|| panic!("{lang}: the catching-up note"));
        // Both as painted: the field's box, the note's card.
        let gap = card_around(&kit, note).top() - card_around(&kit, field).bottom();
        assert!(
            (20.0..=24.0).contains(&gap),
            "{lang}: {gap} px between the password and the note"
        );
    }
}

/// Designer's r1 P3 1: every widget near «Hide the amounts» stands at least 6 px from the
/// toggle's hover area, on every screen it is on, at 1024×768 and 800×600.
#[test]
fn the_toggles_hover_area_keeps_6_px_from_its_neighbours() {
    let presets = [
        "overview",
        "send",
        "identity",
        "shielded",
        "receive",
        "activity",
        "wallet-menu",
        "contact-profile",
        "top-up",
        "create-identity",
        "shield-core",
        "unshield",
        "dashpay",
        "activity-card-self",
    ];
    let mut seen = 0;
    let mut close = Vec::new();
    for size in [egui::vec2(1024.0, 768.0), egui::vec2(800.0, 600.0)] {
        for preset in presets {
            let (_turn, mut kit) = demo_app(preset, "ru");
            kit.set_size(size);
            kit.run_steps(6);
            let name = crate::i18n::spoken("Hide the amounts");
            let toggles: Vec<(egui::Rect, String)> = kit
                .query_all_by(|n| {
                    n.role() == Role::Button && n.label().as_deref() == Some(name.as_str())
                })
                .map(|n| (n.rect(), area_of(&n.accesskit_node())))
                .collect();
            for (hover, area) in toggles {
                // Only a toggle the pointer can reach: one under a dialog never shows its hover.
                if !shows_its_hover(&mut kit, hover) {
                    continue;
                }
                seen += 1;
                for n in kit.query_all_by(|n| {
                    matches!(
                        n.role(),
                        Role::Label
                            | Role::Button
                            | Role::TextInput
                            | Role::PasswordInput
                            | Role::CheckBox
                            | Role::RadioButton
                            | Role::Link
                            | Role::Image
                    )
                }) {
                    let r = n.rect();
                    // What stands in another layer (the page under a dialog) is not its neighbour.
                    if r.area() <= 0.0
                        || area_of(&n.accesskit_node()) != area
                        || r.contains_rect(hover)
                        || hover.contains_rect(r)
                    {
                        continue;
                    }
                    let dx = (r.left() - hover.right())
                        .max(hover.left() - r.right())
                        .max(0.0);
                    let dy = (r.top() - hover.bottom())
                        .max(hover.top() - r.bottom())
                        .max(0.0);
                    let away = dx.max(dy);
                    if away < 6.0 {
                        let k = n.accesskit_node();
                        close.push(format!(
                            "{preset} {size:?}: {:?} «{}» {away} px from the hover {hover:?} at {r:?}",
                            k.role(),
                            k.label().or(k.value()).unwrap_or_default()
                        ));
                    }
                }
            }
        }
    }
    assert!(seen >= 20, "the toggle was found only {seen} times");
    assert!(close.is_empty(), "too close:\n{}", close.join("\n"));
}

/// The top-level node `node` is in: one per egui area (the page, a dialog, a menu).
fn area_of(node: &egui_kittest::kittest::AccessKitNode<'_>) -> String {
    let mut at = *node;
    while let Some(up) = at.parent() {
        if up.parent().is_none() {
            return format!("{:?}", at.id());
        }
        at = up;
    }
    format!("{:?}", at.id())
}

/// Whether the pointer on `hover`'s middle paints the toggle's hover background there: the
/// toggle is on top and can be reached.
fn shows_its_hover(kit: &mut Kit, hover: egui::Rect) -> bool {
    fn rects(shape: &egui::Shape, found: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(r) if r.fill.a() > 0 => found.push(r.rect),
            egui::Shape::Vec(v) => v.iter().for_each(|s| rects(s, found)),
            _ => {}
        }
    }
    kit.input_mut()
        .events
        .push(egui::Event::PointerMoved(hover.center()));
    kit.run_steps(2);
    let mut found = Vec::new();
    for clipped in &kit.output().shapes {
        rects(&clipped.shape, &mut found);
    }
    found
        .iter()
        .any(|r| (r.min - hover.min).length() < 0.6 && (r.max - hover.max).length() < 0.6)
}

/// Designer's r1 of part 2: in each pool action tile the title with its line under it stands
/// on the tile's middle, as the icon does (ru, de at 1024×768).
#[test]
fn the_pools_tile_words_stand_on_the_tiles_middle() {
    for lang in ["ru", "de"] {
        let (_turn, mut kit) = demo_app("shielded", lang);
        kit.run_steps(6);
        for (title, sub) in crate::screens::shielded::actions() {
            let name = crate::i18n::spoken(title);
            let tile = kit
                .query_all_by(|n| {
                    n.role() == Role::Button && n.label().as_deref() == Some(name.as_str())
                })
                .map(|n| n.rect())
                .max_by(|a, b| a.height().total_cmp(&b.height()))
                .unwrap_or_else(|| panic!("the tile «{name}»"));
            let top = painted_text_in(&kit, &crate::i18n::t(title), tile)
                .unwrap_or_else(|| panic!("{lang}: the painted «{name}»"));
            let bottom = painted_text_in(&kit, &crate::i18n::t(sub), tile)
                .unwrap_or_else(|| panic!("{lang}: the line under «{name}»"));
            let middle = (top.top() + bottom.bottom()) / 2.0;
            assert!(
                (middle - tile.center().y).abs() <= 2.0,
                "{lang}: «{name}»'s words centred at {middle}, the tile at {}",
                tile.center().y
            );
        }
    }
}

/// Designer's r1 of part 2: in Settings' section list every name ends 12 px inside its item, as
/// it starts 12 px in, and stays inside it top to bottom — a long one takes two lines. The
/// chosen one (Appearance) is drawn bolder; ru, de at 800×600 and 1024×768.
#[test]
fn settings_section_names_keep_12_px_inside() {
    let sections = [
        "Wallets",
        "Network and nodes",
        "Security",
        "Backup",
        "Appearance and language",
        "Notifications",
        "About",
    ];
    for lang in ["ru", "de"] {
        for size in [egui::vec2(800.0, 600.0), egui::vec2(1024.0, 768.0)] {
            let (_turn, mut kit) = demo_app("appearance", lang);
            kit.set_size(size);
            kit.run_steps(6);
            for label in sections {
                let name = crate::i18n::spoken(label);
                let item = kit
                    .query_all_by(|n| {
                        n.label().as_deref() == Some(name.as_str()) && n.role() != Role::Label
                    })
                    .map(|n| n.rect())
                    .find(|r| r.width() > 150.0 && r.width() < 260.0)
                    .unwrap_or_else(|| panic!("{lang}: the section «{name}»"));
                let words = painted_text_in(&kit, &crate::i18n::t(label), item.expand(40.0))
                    .unwrap_or_else(|| panic!("{lang}: the painted «{name}»"));
                assert!(
                    words.right() <= item.right() - 12.0 + 0.5
                        && words.top() >= item.top()
                        && words.bottom() <= item.bottom(),
                    "{lang} {size:?}: «{name}» painted at {words:?} in an item at {item:?}"
                );
            }
        }
    }
}
